//! Replay a session log written by [`crate::run`]: each line is the receive
//! time (RFC 3339), a tab, and the Signal K message. Messages are sent at their
//! recorded pace times a speed factor, and the replay can be paused, sped up
//! and moved to any time.

use crate::{Event, Router, VesselState};
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// A log file's span and size.
#[derive(Debug, Clone, Serialize)]
pub struct LogInfo {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    /// First and last message time, Unix ms.
    pub start: i64,
    pub end: i64,
    pub lines: usize,
}

fn line_time(line: &str) -> Option<i64> {
    let ts = line.split('\t').next()?;
    chrono::DateTime::parse_from_rfc3339(ts.trim()).ok().map(|t| t.timestamp_millis())
}

/// (time, byte offset) of every line with a time.
fn index(path: &Path) -> std::io::Result<Vec<(i64, u64)>> {
    let mut r = BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
    let mut out = Vec::new();
    let mut off = 0u64;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        // only the time prefix is parsed here
        let head = &buf[..buf.len().min(40)];
        if let Some(t) = std::str::from_utf8(head).ok().and_then(|h| h.split('\t').next()).and_then(line_time) {
            out.push((t, off));
        }
        off += n as u64;
    }
    Ok(out)
}

/// Describe a log: first and last time, lines, size.
pub fn info(path: &Path) -> Option<LogInfo> {
    let bytes = std::fs::metadata(path).ok()?.len();
    let mut f = std::fs::File::open(path).ok()?;
    let mut first = String::new();
    BufReader::new(&mut f).read_line(&mut first).ok()?;
    let start = line_time(&first)?;
    // last line: read the tail
    let tail = 64 * 1024u64;
    f.seek(SeekFrom::Start(bytes.saturating_sub(tail))).ok()?;
    // the tail can start in the middle of a multi-byte character, so it is not read as strict UTF-8
    let mut raw = Vec::new();
    f.take(tail).read_to_end(&mut raw).ok()?;
    let s = String::from_utf8_lossy(&raw);
    let end = s.lines().rev().find_map(line_time).unwrap_or(start);
    let name = path.file_name()?.to_string_lossy().into_owned();
    // lines: estimate from the size of the first line (counting all would read the file)
    let lines = (bytes / first.len().max(1) as u64) as usize;
    Some(LogInfo { path: path.to_path_buf(), name, bytes, start, end, lines })
}

/// The logs in a folder, newest first.
pub fn list(dir: &Path) -> Vec<LogInfo> {
    let mut v: Vec<LogInfo> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .filter_map(|p| info(&p))
        .collect();
    v.sort_by_key(|l| std::cmp::Reverse(l.start));
    v
}

/// Shared controls for a running replay.
#[derive(Debug)]
pub struct Control {
    pub paused: AtomicBool,
    /// Speed × 100 (100 = real time).
    pub speed: AtomicU32,
    /// Seek request, Unix ms (i64::MIN = none).
    pub seek: AtomicI64,
    pub stop: AtomicBool,
    /// Where the replay is, Unix ms, and the log's span.
    pub at: AtomicI64,
    pub span: Mutex<(i64, i64)>,
}

impl Default for Control {
    fn default() -> Self {
        Control {
            paused: AtomicBool::new(false),
            speed: AtomicU32::new(100),
            seek: AtomicI64::new(i64::MIN),
            stop: AtomicBool::new(false),
            at: AtomicI64::new(0),
            span: Mutex::new((0, 0)),
        }
    }
}

/// Replay `path` until it ends or is stopped; events go to `tx` (blocking sends:
/// run it on its own thread). Own-vessel events are sent at most every
/// `min_interval` of log time. Returns when done.
pub fn run(path: &Path, ctl: Arc<Control>, tx: mpsc::Sender<Event>, min_interval: Duration) -> std::io::Result<()> {
    let idx = index(path)?;
    if idx.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "no timed lines in this log"));
    }
    *ctl.span.lock().unwrap() = (idx[0].0, idx[idx.len() - 1].0);
    let mut file = BufReader::new(std::fs::File::open(path)?);
    let read_at = |i: usize, file: &mut BufReader<std::fs::File>| -> Option<Value> {
        file.seek(SeekFrom::Start(idx[i].1)).ok()?;
        let mut line = String::new();
        file.read_line(&mut line).ok()?;
        serde_json::from_str(line.split_once('\t')?.1).ok()
    };
    let mut state = VesselState::default();
    let mut router = Router::default();
    let mut i = 0usize;
    // wall clock and log time that match, for pacing
    let mut anchor = (Instant::now(), idx[0].0);
    let mut anchor_speed = ctl.speed.load(Ordering::Relaxed).max(1) as f64 / 100.0;
    ctl.at.store(idx[0].0, Ordering::Relaxed);
    let mut last_sent: i64 = i64::MIN / 2;
    let min_ms = min_interval.as_millis() as i64;
    while i < idx.len() {
        if ctl.stop.load(Ordering::Relaxed) || tx.is_closed() {
            return Ok(());
        }
        let seek = ctl.seek.swap(i64::MIN, Ordering::Relaxed);
        if seek != i64::MIN {
            // rebuild the state up to the new time: from the start when going back
            let target = idx.partition_point(|(t, _)| *t < seek);
            if target < i {
                state = VesselState::default();
                router = Router::default();
                i = 0;
            }
            while i < target {
                if let Some(v) = read_at(i, &mut file) {
                    let (_, others) = router.route_at(&mut state, &v, idx[i].0);
                    for ev in others {
                        let _ = tx.blocking_send(ev);
                    }
                }
                i += 1;
            }
            let _ = tx.blocking_send(Event::Vessel(replay_state(&state, seek)));
            last_sent = idx[i.min(idx.len() - 1)].0;
            anchor = (Instant::now(), seek);
            ctl.at.store(seek, Ordering::Relaxed);
            continue;
        }
        if ctl.paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
            anchor = (Instant::now(), ctl.at.load(Ordering::Relaxed));
            continue;
        }
        let speed = ctl.speed.load(Ordering::Relaxed).max(1) as f64 / 100.0;
        if (speed - anchor_speed).abs() > 1e-9 {
            anchor = (Instant::now(), ctl.at.load(Ordering::Relaxed));
            anchor_speed = speed;
        }
        let (t, _) = idx[i];
        let due = anchor.0 + Duration::from_millis(((t - anchor.1).max(0) as f64 / speed) as u64);
        let now = Instant::now();
        if due > now {
            let elapsed = (now.duration_since(anchor.0).as_millis() as f64 * speed) as i64;
            ctl.at.store(anchor.1.saturating_add(elapsed).min(t), Ordering::Relaxed);
            std::thread::sleep((due - now).min(Duration::from_millis(50)));
            continue; // check the controls again
        }
        if let Some(v) = read_at(i, &mut file) {
            let (own, others) = router.route_at(&mut state, &v, t);
            for ev in others {
                if tx.blocking_send(ev).is_err() {
                    return Ok(());
                }
            }
            if own && t - last_sent >= min_ms {
                last_sent = t;
                if tx.blocking_send(Event::Vessel(replay_state(&state, t))).is_err() {
                    return Ok(());
                }
            }
        }
        ctl.at.store(t, Ordering::Relaxed);
        i += 1;
    }
    let _ = tx.blocking_send(Event::Vessel(replay_state(&state, idx[idx.len() - 1].0)));
    Ok(())
}

fn replay_state(state: &VesselState, at: i64) -> VesselState {
    let mut result = state.clone();
    result.position_age_ms = state.position_received_at_ms.map(|t| at.saturating_sub(t).max(0) as u64);
    result.depth_age_ms = state.depth_received_at_ms.iter().map(|(field, t)| (field.clone(), at.saturating_sub(*t).max(0) as u64)).collect();
    result.replay_time_ms = Some(at);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_log(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join("sk-20261019-000000Z.log");
        let mut s = String::new();
        s.push_str(&format!("2026-10-19T00:00:00+00:00\t{}\n", json!({"name": "signalk-server", "self": "vessels.urn:mrn:imo:mmsi:512999999"})));
        for k in 0..20 {
            let t = format!("2026-10-19T00:00:{:02}.000+00:00", k);
            let own = json!({"context": "vessels.urn:mrn:imo:mmsi:512999999", "updates": [{"values": [
                {"path": "navigation.position", "value": {"latitude": -41.0 - k as f64 * 0.001, "longitude": 173.0}},
                {"path": "navigation.speedOverGround", "value": 3.0}]}]});
            s.push_str(&format!("{t}\t{own}\n"));
            let other = json!({"context": "vessels.urn:mrn:imo:mmsi:512000123", "updates": [{"values": [
                {"path": "navigation.position", "value": {"latitude": -41.2, "longitude": 173.2 + k as f64 * 0.001}}]}]});
            s.push_str(&format!("{t}\t{other}\n"));
        }
        std::fs::write(&p, s).unwrap();
        p
    }

    #[test]
    fn replays_fast_with_targets_and_seeks() {
        let dir = std::env::temp_dir().join(format!("racenav-replay-{}", std::process::id()));
        let p = write_log(&dir);
        let li = info(&p).unwrap();
        assert_eq!(li.end - li.start, 19_000);
        assert_eq!(list(&dir).len(), 1);
        let ctl = Arc::new(Control::default());
        ctl.speed.store(100 * 1000, Ordering::Relaxed); // 1000× : 19 s of log in ~20 ms
        let (tx, mut rx) = mpsc::channel(1000);
        let c2 = ctl.clone();
        let p2 = p.clone();
        let h = std::thread::spawn(move || run(&p2, c2, tx, Duration::from_millis(250)));
        let mut own = 0;
        let mut other = 0;
        let mut last_lat = 0.0;
        while let Some(ev) = rx.blocking_recv() {
            match ev {
                Event::Vessel(v) => {
                    own += 1;
                    last_lat = v.lat.unwrap_or(0.0);
                }
                Event::Other { context, .. } => {
                    assert!(context.ends_with("512000123"));
                    other += 1;
                }
                _ => {}
            }
        }
        h.join().unwrap().unwrap();
        assert_eq!(other, 20, "every target message");
        assert!((19..=21).contains(&own), "own vessel at most every 250 ms of log time: {own}");
        assert!((last_lat - (-41.019)).abs() < 1e-9);

        // seeking back while paused rebuilds the state at that time
        let ctl = Arc::new(Control::default());
        ctl.paused.store(true, Ordering::Relaxed);
        ctl.seek.store(li.start + 5_500, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel(1000);
        let c2 = ctl.clone();
        let h = std::thread::spawn(move || run(&p, c2, tx, Duration::from_millis(250)));
        let mut lat = None;
        let t0 = Instant::now();
        while lat.is_none() && t0.elapsed() < Duration::from_secs(5) {
            if let Ok(Event::Vessel(v)) = rx.try_recv() {
                lat = v.lat;
            }
        }
        assert!((lat.unwrap() - (-41.005)).abs() < 1e-9, "{lat:?}");
        ctl.stop.store(true, Ordering::Relaxed);
        h.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn routes_own_and_other_contexts() {
        let mut r = Router::default();
        let mut s = VesselState::default();
        r.route(&mut s, &json!({"self": "vessels.urn:mrn:imo:mmsi:512999999"}));
        let own = json!({"context": "vessels.urn:mrn:imo:mmsi:512999999", "updates": [{"values": [{"path": "navigation.speedOverGround", "value": 1.0}]}]});
        assert!(r.route(&mut s, &own).0);
        let other = json!({"context": "atons.urn:mrn:imo:mmsi:995121001", "updates": [{"values": [{"path": "navigation.position", "value": {"latitude": -41.0, "longitude": 173.0}}]}]});
        let (o, evs) = r.route(&mut s, &other);
        assert!(!o);
        assert!(matches!(&evs[0], Event::Other { context, values } if context.starts_with("atons.") && values.len() == 1));
        // no context = self (subscribe=self servers)
        let bare = json!({"updates": [{"values": [{"path": "navigation.speedOverGround", "value": 2.0}]}]});
        assert!(r.route(&mut s, &bare).0);
    }

    #[test]
    fn replay_position_age_uses_recorded_fix_time() {
        let mut router = Router::default();
        let mut state = VesselState::default();
        let fix = json!({"updates": [{"values": [{"path": "navigation.position", "value": {"latitude": -41, "longitude": 173}}]}]});
        let wind = json!({"updates": [{"values": [{"path": "environment.wind.speedApparent", "value": 4.0}]}]});
        router.route_at(&mut state, &fix, 1000);
        router.route_at(&mut state, &json!({"updates":[{"values":[{"path":"environment.depth.belowTransducer","value":5.0}]}]}), 2000);
        router.route_at(&mut state, &wind, 32000);
        assert_eq!(replay_state(&state, 32000).position_age_ms, Some(31000));
        assert_eq!(replay_state(&state, 32000).depth_age_ms["depth"], 30000);
        assert_eq!(state.position_observation, 1);
    }

    #[test]
    fn replay_clock_advances_at_speed_and_stops_while_paused() {
        let dir = std::env::temp_dir().join(format!("racenav-replay-clock-{}", std::process::id()));
        let p = write_log(&dir);
        let start = info(&p).unwrap().start;
        let ctl = Arc::new(Control::default());
        ctl.speed.store(1000, Ordering::Relaxed);
        let (tx, _rx) = mpsc::channel(1000);
        let c2 = ctl.clone();
        let p2 = p.clone();
        let h = std::thread::spawn(move || run(&p2, c2, tx, Duration::from_millis(250)));
        let deadline = Instant::now() + Duration::from_secs(3);
        while ctl.at.load(Ordering::Relaxed) < start + 500 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ctl.at.load(Ordering::Relaxed) >= start + 500, "the replay clock did not advance");
        ctl.paused.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(120));
        let paused_at = ctl.at.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(ctl.at.load(Ordering::Relaxed), paused_at, "the replay clock moved while paused");
        ctl.seek.store(start + 5500, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(3);
        while ctl.at.load(Ordering::Relaxed) != start + 5500 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(ctl.at.load(Ordering::Relaxed), start + 5500);
        ctl.stop.store(true, Ordering::Relaxed);
        h.join().unwrap().unwrap();
        std::fs::remove_file(p).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
