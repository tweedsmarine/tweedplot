//! Telling apart two (or more) GNSS receivers that arrive mixed on one NMEA input.
//!
//! A PC multiplexing several streams it has no control over can get position sentences from two
//! receivers with the same talker (`GP`), so the position jumps between the two antennas each time
//! one or the other is heard. Each receiver formats its sentences in its own way, though, and that
//! doesn't change from one fix to the next: how many decimals of a minute it gives, whether the UTC
//! time has decimals, and its talker. That fingerprint sorts every RMC, GGA, GLL and GNS into a
//! *stream*; one stream is used for the position and the others are ignored.
//!
//! The stream used is the best one (RTK, then DGPS, then autonomous; then the lower HDOP; then the
//! faster rate), chosen when the input starts. It stays in use while it is heard: another stream
//! takes over only when the one in use goes quiet, or has been worse (lost its DGPS, say) for
//! [`FixStreams::BETTER_AFTER`] seconds. A stream can also be pinned by the user.
//!
//! Within a stream, a fix whose UTC time goes backwards is dropped: that is another receiver, a
//! second or so behind, that happens to format its sentences the same way.

use crate::Sentence;
use serde::Serialize;

/// One receiver's fixes, as far as can be told from their format.
#[derive(Debug, Clone)]
struct Stream {
    key: String,
    letter: char,
    talker: String,
    lat_decimals: u8,
    time_decimals: u8,
    first_seen: f64,
    last_seen: f64,
    /// Distinct fix times heard (several sentences carry the same fix).
    fixes: u64,
    last_time: Option<f64>,
    quality: Option<u8>,
    hdop: Option<f64>,
    satellites: Option<u32>,
    lat: Option<f64>,
    lon: Option<f64>,
    /// Since when this stream has been better than the one in use (auto mode).
    better_since: Option<f64>,
    backwards: u64,
}

impl Stream {
    /// Fixes a second, to 0.1 (so two 1 Hz receivers tie).
    fn rate(&self) -> f64 {
        let span = self.last_seen - self.first_seen;
        if span < 1.0 {
            0.0
        } else {
            ((self.fixes.saturating_sub(1)) as f64 / span * 10.0).round() / 10.0
        }
    }
}

/// A stream, for the Connections settings.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamSummary {
    pub key: String,
    /// "A", "B" … in the order they were first heard.
    pub letter: String,
    pub talker: String,
    pub lat_decimals: u8,
    pub time_decimals: u8,
    /// 0 none or estimated … 5 RTK fixed (see [`quality_name`]).
    pub quality: Option<u8>,
    pub quality_name: String,
    pub hdop: Option<f64>,
    pub satellites: Option<u32>,
    /// Fixes a second.
    pub rate: f64,
    pub fresh: bool,
    pub in_use: bool,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Fixes dropped because their time went backwards.
    pub backwards: u64,
}

/// What [`FixStreams::accept`] decided about a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not a position sentence, or from the stream in use: apply it.
    Use,
    /// A fix from a stream not in use, or out of time order: leave it out.
    Ignore,
}

#[derive(Debug, Clone)]
pub struct FixStreams {
    streams: Vec<Stream>,
    in_use: Option<String>,
    pinned: Option<String>,
    /// Seconds without a fix before a stream counts as gone.
    pub stale_after: f64,
    started: Option<f64>,
    switch: Option<(String, String)>,
    /// The receiver the last sentence was put down to (for the NMEA monitor).
    last: Option<char>,
}

impl Default for FixStreams {
    fn default() -> Self {
        Self { streams: Vec::new(), in_use: None, pinned: None, stale_after: 10.0, started: None, switch: None, last: None }
    }
}

/// Quality rank from a GGA fix quality.
fn rank_from_gga(q: &str) -> Option<u8> {
    Some(match q.trim().parse::<u8>().ok()? {
        4 => 5,     // RTK fixed
        5 => 4,     // RTK float
        2 | 9 => 3, // DGPS, SBAS
        3 => 3,     // PPS
        1 => 2,     // autonomous
        6 => 1,     // dead reckoning
        _ => 0,
    })
}

/// Quality rank from an NMEA 2.3+ mode indicator (RMC, GLL, VTG; the first letter of GNS).
fn rank_from_mode(m: &str) -> Option<u8> {
    Some(match m.trim().chars().next()? {
        'R' => 5,
        'F' => 4,
        'D' | 'P' => 3,
        'A' => 2,
        'E' => 1,
        'N' | 'V' | 'S' | 'M' => 0,
        _ => return None,
    })
}

pub fn quality_name(rank: Option<u8>) -> &'static str {
    match rank {
        Some(5) => "RTK",
        Some(4) => "RTK float",
        Some(3) => "DGPS",
        Some(2) => "GPS",
        Some(1) => "estimated",
        Some(_) => "no fix",
        None => "",
    }
}

fn decimals(s: &str) -> u8 {
    s.split_once('.').map_or(0, |(_, d)| d.len().min(9) as u8)
}

fn seconds_of_day(t: &str) -> Option<f64> {
    if t.len() < 6 || !t.is_char_boundary(6) {
        return None;
    }
    let h: f64 = t[0..2].parse().ok()?;
    let m: f64 = t[2..4].parse().ok()?;
    let s: f64 = t[4..].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

fn coord(v: &str, hemi: &str, lat: bool) -> Option<f64> {
    let deg_len = if lat { 2 } else { 3 };
    if v.len() < deg_len + 1 || !v.is_char_boundary(deg_len) {
        return None;
    }
    let d: f64 = v[..deg_len].parse().ok()?;
    let m: f64 = v[deg_len..].parse().ok()?;
    let x = d + m / 60.0;
    Some(if hemi == "S" || hemi == "W" { -x } else { x })
}

/// What a position sentence says about its fix: (time, lat, lat hemisphere, lon, lon hemisphere,
/// quality rank, HDOP, satellites). None for anything else, or a fix marked invalid.
#[allow(clippy::type_complexity)]
fn fix_of(s: &Sentence) -> Option<(&str, &str, &str, &str, &str, Option<u8>, Option<f64>, Option<u32>)> {
    let f = &s.fields;
    let g = |i: usize| f.get(i).map_or("", String::as_str);
    match s.formatter.as_str() {
        "RMC" if g(1) == "A" => Some((g(0), g(2), g(3), g(4), g(5), rank_from_mode(g(11)), None, None)),
        "GGA" if rank_from_gga(g(5)).is_some_and(|r| r > 0) => Some((g(0), g(1), g(2), g(3), g(4), rank_from_gga(g(5)), g(7).parse().ok(), g(6).parse().ok())),
        "GLL" if g(5) == "A" => Some((g(4), g(0), g(1), g(2), g(3), rank_from_mode(g(6)), None, None)),
        "GNS" if rank_from_mode(g(5)).is_some_and(|r| r > 0) => {
            Some((g(0), g(1), g(2), g(3), g(4), rank_from_mode(g(5)), g(7).parse().ok(), g(6).parse().ok()))
        }
        _ => None,
    }
}

impl FixStreams {
    /// How long a stream must be better than the one in use before it takes over (seconds).
    pub const BETTER_AFTER: f64 = 30.0;
    /// For this long after the first fix, the best stream is taken at once (seconds).
    pub const SETTLE: f64 = 5.0;
    /// A stream not heard for this long is forgotten (seconds).
    pub const FORGET_AFTER: f64 = 600.0;

    /// Use this stream whenever it is heard (None: automatic).
    pub fn set_pinned(&mut self, key: Option<String>) {
        self.pinned = key.filter(|k| !k.is_empty());
    }

    /// The stream in use, if any.
    pub fn in_use(&self) -> Option<&str> {
        self.in_use.as_deref()
    }

    /// How many streams have been heard recently.
    pub fn count(&self) -> usize {
        self.streams.len()
    }

    /// The last change of stream ("A (GP, 5 decimals)", "B (…)"), once.
    pub fn take_switch(&mut self) -> Option<(String, String)> {
        self.switch.take()
    }

    pub fn label(&self, key: &str) -> String {
        self.streams.iter().find(|s| s.key == key).map_or_else(
            || key.to_owned(),
            |s| {
                format!(
                    "GPS {} ({}, {} decimals{})",
                    s.letter,
                    s.talker,
                    s.lat_decimals,
                    if s.quality.is_some() { format!(", {}", quality_name(s.quality)) } else { String::new() }
                )
            },
        )
    }

    /// Decide about one sentence heard at `now` (seconds, any steady clock).
    pub fn accept(&mut self, s: &Sentence, now: f64) -> Verdict {
        self.last = None;
        if s.formatter == "VTG" {
            return self.accept_vtg(s, now);
        }
        let Some((time, lat, ns, lon, ew, rank, hdop, sats)) = fix_of(s) else {
            return Verdict::Use;
        };
        let key = format!("{}-{}-{}", s.talker, decimals(lat), decimals(time));
        self.streams.retain(|st| now - st.last_seen <= Self::FORGET_AFTER);
        let index = match self.streams.iter().position(|st| st.key == key) {
            Some(i) => i,
            None => {
                let letter = (b'A'..=b'Z').map(char::from).find(|c| !self.streams.iter().any(|st| st.letter == *c)).unwrap_or('?');
                self.streams.push(Stream {
                    key: key.clone(),
                    letter,
                    talker: s.talker.clone(),
                    lat_decimals: decimals(lat),
                    time_decimals: decimals(time),
                    first_seen: now,
                    last_seen: now,
                    fixes: 0,
                    last_time: None,
                    quality: None,
                    hdop: None,
                    satellites: None,
                    lat: None,
                    lon: None,
                    better_since: None,
                    backwards: 0,
                });
                self.streams.len() - 1
            }
        };
        self.started.get_or_insert(now);
        let st = &mut self.streams[index];
        self.last = Some(st.letter);
        let t = seconds_of_day(time);
        if let (Some(t), Some(last)) = (t, st.last_time) {
            let back = last - t;
            if back > 0.001 && back < 43_200.0 {
                st.backwards += 1;
                return Verdict::Ignore;
            }
        }
        if t.is_some() && t != st.last_time {
            st.fixes += 1;
            st.last_time = t;
        }
        st.last_seen = now;
        if rank.is_some() {
            st.quality = rank;
        }
        if hdop.is_some() {
            st.hdop = hdop;
        }
        if sats.is_some() {
            st.satellites = sats;
        }
        if let (Some(a), Some(b)) = (coord(lat, ns, true), coord(lon, ew, false)) {
            st.lat = Some(a);
            st.lon = Some(b);
        }
        self.choose(now);
        if self.in_use.as_deref() == Some(key.as_str()) {
            Verdict::Use
        } else {
            Verdict::Ignore
        }
    }

    /// A VTG has no position to fingerprint, but its mode letter usually says which receiver sent
    /// it: left out when only streams not in use (with that talker) have that quality.
    fn accept_vtg(&mut self, s: &Sentence, now: f64) -> Verdict {
        let Some(rank) = s.fields.get(8).and_then(|m| rank_from_mode(m)) else {
            return Verdict::Use;
        };
        let same: Vec<&Stream> = self.streams.iter().filter(|st| st.talker == s.talker && now - st.last_seen <= self.stale_after).collect();
        if same.len() < 2 {
            return Verdict::Use;
        }
        let matching: Vec<&&Stream> = same.iter().filter(|st| st.quality == Some(rank)).collect();
        let last = (matching.len() == 1).then(|| matching[0].letter);
        let verdict =
            if !matching.is_empty() && matching.iter().all(|st| Some(st.key.as_str()) != self.in_use.as_deref()) { Verdict::Ignore } else { Verdict::Use };
        self.last = last;
        verdict
    }

    /// The receiver ("A", "B" …) the last sentence was put down to, if it could be told.
    pub fn last_letter(&self) -> Option<char> {
        self.last
    }

    fn fresh(&self, st: &Stream, now: f64) -> bool {
        now - st.last_seen <= self.stale_after
    }

    /// Order: better quality, lower HDOP, faster, heard first.
    fn better(a: &Stream, b: &Stream) -> std::cmp::Ordering {
        b.quality
            .unwrap_or(0)
            .cmp(&a.quality.unwrap_or(0))
            .then(a.hdop.unwrap_or(99.0).total_cmp(&b.hdop.unwrap_or(99.0)))
            .then(b.rate().total_cmp(&a.rate()))
            .then(a.first_seen.total_cmp(&b.first_seen))
    }

    fn choose(&mut self, now: f64) {
        let before = self.in_use.clone();
        let fresh: Vec<&Stream> = self.streams.iter().filter(|st| self.fresh(st, now)).collect();
        let best = fresh.iter().copied().min_by(|a, b| Self::better(a, b)).map(|st| st.key.clone());
        let current = self.in_use.as_ref().and_then(|k| fresh.iter().find(|st| &st.key == k)).copied();
        let pinned = self.pinned.as_ref().and_then(|k| fresh.iter().find(|st| &st.key == k)).map(|st| st.key.clone());
        let settling = self.started.is_some_and(|t| now - t < Self::SETTLE);
        let next = if pinned.is_some() {
            pinned
        } else if settling || current.is_none() {
            best
        } else {
            // auto, the current one still heard: another takes over only after being better a while
            let cur = current.unwrap().clone();
            let mut take = None;
            for st in &mut self.streams {
                if st.key == cur.key || now - st.last_seen > self.stale_after {
                    st.better_since = None;
                    continue;
                }
                if st.quality.unwrap_or(0) > cur.quality.unwrap_or(0) {
                    let since = *st.better_since.get_or_insert(now);
                    if now - since >= Self::BETTER_AFTER {
                        take = Some(st.key.clone());
                    }
                } else {
                    st.better_since = None;
                }
            }
            take.or(Some(cur.key))
        };
        if next != before {
            for st in &mut self.streams {
                st.better_since = None;
            }
            // a switch worth saying: from one stream that was in use to another (not the first pick,
            // nor the settling at start)
            if let (Some(from), Some(to), false) = (&before, &next, settling) {
                self.switch = Some((self.label(from), self.label(to)));
            }
            self.in_use = next;
        }
    }

    /// The streams heard, for the settings.
    pub fn summary(&self, now: f64) -> Vec<StreamSummary> {
        self.streams
            .iter()
            .map(|st| StreamSummary {
                key: st.key.clone(),
                letter: st.letter.to_string(),
                talker: st.talker.clone(),
                lat_decimals: st.lat_decimals,
                time_decimals: st.time_decimals,
                quality: st.quality,
                quality_name: quality_name(st.quality).to_owned(),
                hdop: st.hdop,
                satellites: st.satellites,
                rate: st.rate(),
                fresh: self.fresh(st, now),
                in_use: self.in_use.as_deref() == Some(st.key.as_str()),
                lat: st.lat,
                lon: st.lon,
                backwards: st.backwards,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Framer, Observation};

    /// The test capture, one sentence a line, fed at about 26 sentences a second as it was heard.
    fn capture() -> Vec<Sentence> {
        let text = include_str!("../tests/data/two-gps.nmea");
        let mut framer = Framer::default();
        let mut out = Vec::new();
        for line in text.lines().filter(|l| l.starts_with('$')) {
            for s in framer.push(format!("{line}\r\n").as_bytes()) {
                out.push(s.expect("valid sentence"));
            }
        }
        out
    }

    fn sentence(body: &str) -> Sentence {
        let sum = body.bytes().fold(0u8, |a, b| a ^ b);
        crate::parse_sentence(format!("${body}*{sum:02X}").as_bytes(), 1024).unwrap()
    }

    #[test]
    fn two_receivers_on_one_input_are_told_apart_and_the_dgps_one_is_used() {
        let mut fs = FixStreams::default();
        let mut obs = Observation::default();
        let mut lats = Vec::new();
        for (i, s) in capture().iter().enumerate() {
            let now = i as f64 / 6.0;
            if fs.accept(s, now) == Verdict::Use {
                obs.apply(s);
                if matches!(s.formatter.as_str(), "RMC" | "GGA" | "GLL") {
                    lats.push(obs.lat.unwrap());
                }
            }
        }
        let sum = fs.summary(80.0 / 6.0);
        assert_eq!(sum.len(), 2, "{sum:?}");
        let a = sum.iter().find(|s| s.lat_decimals == 5).unwrap();
        let b = sum.iter().find(|s| s.lat_decimals == 4).unwrap();
        assert!(a.in_use && !b.in_use);
        assert_eq!((a.quality_name.as_str(), b.quality_name.as_str()), ("DGPS", "GPS"));
        assert_eq!(a.hdop, Some(1.10));
        // the position no longer jumps between the antennas (2.7 m apart): GPS 1 alone wanders
        // well under a metre
        let (lo, hi) = lats.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        assert!((hi - lo) * 60.0 * 1852.0 < 0.2, "spread {} m", (hi - lo) * 60.0 * 1852.0);
        assert!(fs.take_switch().is_none());
    }

    #[test]
    fn the_vtg_of_the_stream_not_in_use_is_left_out() {
        let mut fs = FixStreams::default();
        fs.accept(&sentence("GPGGA,105018.00,4152.53048,S,07331.44018,W,2,07,1.10,0.9,M,15.8,M,,"), 0.0);
        fs.accept(&sentence("GPGGA,105017,4152.5317,S,07331.4405,W,1,8,1.00,3.30,M,,M"), 0.5);
        assert_eq!(fs.accept(&sentence("GPVTG,,T,,M,0.087,N,0.161,K,D"), 1.0), Verdict::Use);
        assert_eq!(fs.last_letter(), Some('A'));
        assert_eq!(fs.accept(&sentence("GPVTG,309.5,T,,M,0.1,N,0.2,K,A"), 1.0), Verdict::Ignore);
        assert_eq!(fs.last_letter(), Some('B'));
        // another talker's VTG (the instruments) is not ours to judge
        assert_eq!(fs.accept(&sentence("IIVTG,309.5,T,,M,0.0,N,0.1,K,A"), 1.0), Verdict::Use);
    }

    #[test]
    fn a_pinned_stream_is_used_and_a_quiet_one_hands_over() {
        let a = |t: &str| sentence(&format!("GPGGA,{t}.00,4152.53048,S,07331.44018,W,2,07,1.10,0.9,M,15.8,M,,"));
        let b = |t: &str| sentence(&format!("GPGGA,{t},4152.5317,S,07331.4405,W,1,8,1.00,3.30,M,,M"));
        let mut fs = FixStreams::default();
        fs.accept(&a("105000"), 0.0);
        fs.accept(&b("105000"), 0.5);
        assert_eq!(fs.in_use(), Some("GP-5-2"));
        fs.set_pinned(Some("GP-4-0".into()));
        assert_eq!(fs.accept(&b("105001"), 1.0), Verdict::Use);
        assert_eq!(fs.accept(&a("105002"), 2.0), Verdict::Ignore);
        // automatic again; A stays in use while heard, B is ignored …
        fs.set_pinned(None);
        for t in 3..20 {
            fs.accept(&a(&format!("1050{t:02}")), t as f64);
            assert_eq!(fs.accept(&b(&format!("1050{t:02}")), t as f64 + 0.5), Verdict::Ignore);
        }
        // … until A goes quiet: B takes over after the stale time, and the switch is reported
        assert_eq!(fs.accept(&b("105025"), 25.0), Verdict::Ignore);
        assert_eq!(fs.accept(&b("105031"), 31.0), Verdict::Use);
        let (from, to) = fs.take_switch().unwrap();
        assert!(from.starts_with("GPS A") && to.starts_with("GPS B"), "{from} → {to}");
    }

    #[test]
    fn a_better_stream_takes_over_only_after_a_while() {
        let a = |t: u32, q: u8| sentence(&format!("GPGGA,1050{t:02}.00,4152.53048,S,07331.44018,W,{q},07,1.10,0.9,M,15.8,M,,"));
        let b = |t: u32| sentence(&format!("GPGGA,1050{t:02},4152.5317,S,07331.4405,W,2,8,1.10,3.30,M,,M"));
        let mut fs = FixStreams::default();
        for t in 0..10 {
            fs.accept(&a(t, 2), t as f64);
            fs.accept(&b(t), t as f64 + 0.5);
        }
        assert_eq!(fs.in_use(), Some("GP-5-2")); // same quality: the first heard stays
                                                 // A loses its DGPS: B (still DGPS) takes over 30 s later, not at once
        for t in 10..39 {
            fs.accept(&a(t, 1), t as f64);
            assert_eq!(fs.accept(&b(t), t as f64 + 0.5), Verdict::Ignore, "at {t}");
        }
        fs.accept(&a(41, 1), 41.0);
        assert_eq!(fs.accept(&b(41), 41.5), Verdict::Use);
    }

    #[test]
    fn a_fix_going_back_in_time_is_dropped() {
        let mut fs = FixStreams::default();
        let g = |t: &str| sentence(&format!("GPGGA,{t},4152.5317,S,07331.4405,W,1,8,1.00,3.30,M,,M"));
        assert_eq!(fs.accept(&g("235959"), 0.0), Verdict::Use);
        assert_eq!(fs.accept(&g("235958"), 0.1), Verdict::Ignore);
        assert_eq!(fs.accept(&g("235959"), 0.2), Verdict::Use); // the same fix in another sentence
        assert_eq!(fs.accept(&g("000001"), 0.3), Verdict::Use); // past midnight
        assert_eq!(fs.summary(0.3)[0].backwards, 1);
    }

    #[test]
    fn one_receiver_is_never_held_back() {
        let mut fs = FixStreams::default();
        for (i, s) in capture().iter().filter(|s| !s.fields.iter().any(|f| f.len() == 10 && f.contains("4152."))).enumerate() {
            assert_eq!(fs.accept(s, i as f64), Verdict::Use, "{}", s.raw);
        }
        assert_eq!(fs.count(), 1);
    }
}
