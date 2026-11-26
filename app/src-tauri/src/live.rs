//! Live data in the app: the Signal K client, AIS targets (Signal K and the
//! aisstream.io feed, merged), and replaying a session log in place of the
//! live stream.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

static TARGETS: Mutex<Option<ais::Targets>> = Mutex::new(None);
static REPLAY: Mutex<Option<(Arc<signalk::replay::Control>, String)>> = Mutex::new(None);
static AIS_REQ: std::sync::LazyLock<Mutex<std::collections::HashMap<&'static str, tokio::sync::watch::Sender<ais::stream::Request>>>> =
    std::sync::LazyLock::new(Default::default);
static AIS_STATUS: std::sync::LazyLock<Mutex<std::collections::HashMap<&'static str, (bool, String)>>> = std::sync::LazyLock::new(Default::default);

fn now_s() -> i64 {
    chrono::Utc::now().timestamp()
}

fn with_targets<R>(f: impl FnOnce(&mut ais::Targets) -> R) -> R {
    let mut g = TARGETS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(ais::Targets::default))
}

/// An AIS target heard over NMEA 0183 (an AIS receiver on a serial or network port).
pub(crate) fn merge_ais(u: ais::Update) {
    if !is_replaying() {
        with_targets(|t| t.merge(u, now_s()));
    }
}

pub(crate) fn is_replaying() -> bool {
    REPLAY.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// The folder Signal K sessions are logged to.
pub fn log_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|p| p.join("logs"))
}

/// Handle one Signal K event, live or replayed.
fn on_event(app: &tauri::AppHandle, ev: signalk::Event, replayed: bool) {
    match ev {
        signalk::Event::Status { connected, detail } => {
            *crate::SK_STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some((connected, detail.clone()));
            let _ = app.emit("sk-status", serde_json::json!({ "connected": connected, "detail": detail }));
        }
        signalk::Event::Vessel(mut v) => {
            if replayed != is_replaying() {
                return; // live data waits while a log plays
            }
            if !replayed && crate::nmea::navigation_source_id().is_some() {
                return;
            }
            if !replayed {
                v.position_age_ms = v.position_received_at_instant.map(|t| t.elapsed().as_millis() as u64);
                v.depth_age_ms = v.depth_received_at_instant.iter().map(|(field, at)| (field.clone(), at.elapsed().as_millis() as u64)).collect();
                crate::rec_vessel(app, &v);
            }
            let _ = app.emit("vessel", serde_json::json!({ "replay": replayed, "v": v }));
        }
        signalk::Event::Other { context, values } => {
            if replayed != is_replaying() {
                return;
            }
            // log time when replayed would be better; the receive time keeps expiry simple
            with_targets(|t| t.apply_signalk(&context, &values, now_s()));
        }
    }
}

/// Start Signal K, the online AIS feed and the target emitter.
pub fn start(app: &tauri::AppHandle) {
    // Signal K
    let handle = app.clone();
    let opts = signalk::Options { url: crate::sk_url(), log_dir: log_dir(app), min_interval: Duration::from_millis(250) };
    let (tx, mut rx) = tokio::sync::mpsc::channel(512);
    tauri::async_runtime::spawn(signalk::run(opts, tx));
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            on_event(&handle, ev, false);
        }
    });

    // Online AIS: one feed per provider (aisstream.io, Open Waters), each idle until the page sends a key and an area
    let (atx, mut arx) = tokio::sync::mpsc::channel::<(&'static str, ais::stream::Event)>(1024);
    for provider in ["aisstream", "openwaters"] {
        let (req_tx, req_rx) = tokio::sync::watch::channel(ais::stream::Request::default());
        AIS_REQ.lock().unwrap_or_else(|e| e.into_inner()).insert(provider, req_tx);
        let (ptx, mut prx) = tokio::sync::mpsc::channel(1024);
        tauri::async_runtime::spawn(ais::stream::run(req_rx, ptx));
        let atx = atx.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(ev) = prx.recv().await {
                if atx.send((provider, ev)).await.is_err() {
                    return;
                }
            }
        });
    }
    let h2 = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some((provider, ev)) = arx.recv().await {
            match ev {
                ais::stream::Event::Status { connected, detail } => {
                    AIS_STATUS.lock().unwrap_or_else(|e| e.into_inner()).insert(provider, (connected, detail.clone()));
                    let _ = h2.emit("ais-status", serde_json::json!({ "provider": provider, "connected": connected, "detail": detail }));
                }
                ais::stream::Event::Update(u) => {
                    if !is_replaying() {
                        with_targets(|t| t.merge(u, now_s()));
                    }
                }
            }
        }
    });

    // changed targets to the page every 2 s
    let h3 = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let (changed, gone) = with_targets(|t| t.take_changes(now_s()));
            if !changed.is_empty() || !gone.is_empty() {
                let _ = h3.emit("ais", serde_json::json!({ "targets": changed, "gone": gone }));
            }
        }
    });

    let h4 = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let r = REPLAY.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some((c, name)) = r {
                let (s, e) = *c.span.lock().unwrap_or_else(|e| e.into_inner());
                let _ = h4.emit(
                    "replay",
                    serde_json::json!({
                        "name": name, "at": c.at.load(Ordering::Relaxed), "start": s, "end": e,
                        "paused": c.paused.load(Ordering::Relaxed), "speed": c.speed.load(Ordering::Relaxed) as f64 / 100.0,
                    }),
                );
            }
        }
    });
}

// ---------- AIS commands ----------------------------------------------------------

/// Every AIS target known now (for a page that has just loaded).
#[tauri::command]
pub fn ais_targets() -> Vec<ais::Target> {
    with_targets(|t| t.all())
}

/// Set one online provider ("aisstream" or "openwaters"), its key and the areas ([[lat, lon], [lat, lon]] each); an empty key stops that feed.
#[tauri::command]
pub fn ais_online(provider: Option<String>, key: String, boxes: Vec<[[f64; 2]; 2]>) {
    let provider = if provider.as_deref() == Some("openwaters") { "openwaters" } else { "aisstream" };
    if key.trim().is_empty() {
        AIS_STATUS.lock().unwrap_or_else(|e| e.into_inner()).insert(provider, (false, "off".into()));
    }
    if let Some(tx) = AIS_REQ.lock().unwrap_or_else(|e| e.into_inner()).get(provider) {
        let _ = tx.send(ais::stream::Request { provider: provider.into(), key, boxes });
    }
}

/// Get an Open Waters token. With the same `seed` as before it is the same station again;
/// without one a new private key is made. Returns { seed, token } for the page to keep.
#[tauri::command]
pub async fn ow_register(seed: Option<String>, name: Option<String>) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let seed = match seed.filter(|s| !s.trim().is_empty()) {
            Some(s) => s,
            None => ais::openwaters::new_seed()?,
        };
        let body = ais::openwaters::key_request(&seed, now_s(), name.as_deref().unwrap_or(""))?;
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
        let text = client
            .post(ais::openwaters::KEYS_URL)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .and_then(|r| r.text())
            .map_err(|e| format!("Open Waters: {e}"))?;
        let token = ais::openwaters::token_of(&text)?;
        Ok(serde_json::json!({ "seed": seed, "token": token }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Our own MMSI (from the Boat panel): never shown as a target.
#[tauri::command]
pub fn ais_set_own(mmsi: Option<u32>) {
    with_targets(|t| t.set_own(mmsi.filter(|m| *m > 0)));
}

/// Recent tracks of the given targets: MMSI → [[t, lon, lat], …].
#[tauri::command]
pub fn ais_tracks(mmsis: Vec<u32>) -> std::collections::HashMap<u32, Vec<[f64; 3]>> {
    with_targets(|t| mmsis.iter().filter_map(|m| t.tracks.get(m).map(|tr| (*m, tr.clone()))).collect())
}

// ---------- replay commands ----------------------------------------------------------

/// The Signal K session logs, newest first.
#[tauri::command]
pub fn replay_logs(app: tauri::AppHandle) -> serde_json::Value {
    let dir = log_dir(&app);
    let logs = dir.as_deref().map(signalk::replay::list).unwrap_or_default();
    serde_json::json!({ "dir": dir, "logs": logs })
}

/// Play a log in place of the live stream (live Signal K is ignored until it stops).
#[tauri::command]
pub fn replay_start(app: tauri::AppHandle, path: String, speed: Option<f64>) -> Result<(), String> {
    replay_stop(app.clone());
    let p = std::path::PathBuf::from(&path);
    if !p.is_file() {
        return Err(format!("{path}: not found"));
    }
    let ctl = Arc::new(signalk::replay::Control::default());
    ctl.speed.store((speed.unwrap_or(1.0).clamp(0.1, 1000.0) * 100.0) as u32, Ordering::Relaxed);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    *REPLAY.lock().unwrap_or_else(|e| e.into_inner()) = Some((ctl.clone(), name));
    with_targets(|t| t.clear());
    let _ = app.emit("ais", serde_json::json!({ "reset": true, "targets": [], "gone": [] }));
    let (tx, mut rx) = tokio::sync::mpsc::channel(512);
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            on_event(&h, ev, true);
        }
    });
    let h2 = app.clone();
    std::thread::spawn(move || {
        let r = signalk::replay::run(&p, ctl.clone(), tx, Duration::from_millis(250));
        // ended by itself (not replaced by another replay): back to live
        let mut g = REPLAY.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_ref().is_some_and(|(c, _)| Arc::ptr_eq(c, &ctl)) {
            *g = None;
            drop(g);
            with_targets(|t| t.clear());
            let _ = h2.emit("ais", serde_json::json!({ "reset": true, "targets": [], "gone": [] }));
            let _ = h2.emit("replay-end", serde_json::json!({ "error": r.err().map(|e| e.to_string()) }));
        }
    });
    Ok(())
}

/// Pause or play, change speed (1 = real time), or go to a time (Unix ms).
#[tauri::command]
pub fn replay_control(paused: Option<bool>, speed: Option<f64>, seek: Option<i64>) {
    if let Some((c, _)) = REPLAY.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if let Some(p) = paused {
            c.paused.store(p, Ordering::Relaxed);
        }
        if let Some(s) = speed {
            c.speed.store((s.clamp(0.1, 1000.0) * 100.0) as u32, Ordering::Relaxed);
        }
        if let Some(t) = seek {
            c.seek.store(t, Ordering::Relaxed);
        }
    }
}

/// Stop the replay and go back to live data.
#[tauri::command]
pub fn replay_stop(app: tauri::AppHandle) {
    let r = REPLAY.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((c, _)) = r {
        c.stop.store(true, Ordering::Relaxed);
        with_targets(|t| t.clear());
        let _ = app.emit("ais", serde_json::json!({ "reset": true, "targets": [], "gone": [] }));
        let _ = app.emit("replay-end", serde_json::json!({ "error": null }));
    }
}
