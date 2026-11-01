//! Tracks: where a boat went, with times. Logged from the own boat's Signal K
//! fixes by a [`Recorder`], or imported from GPX (`<trk>`).
//!
//! Stored one file per track, `<dir>/tracks/<id>.json`. Points are written
//! compactly as `[t, lat, lon, sog, cog]` arrays (t in Unix seconds, SOG in knots,
//! COG in degrees true; t, sog and cog may be null).

use serde::{Deserialize, Serialize};

/// One fix.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(from = "PointRow", into = "PointRow")]
pub struct TrackPoint {
    pub t: Option<i64>,
    pub lat: f64,
    pub lon: f64,
    pub sog: Option<f64>,
    pub cog: Option<f64>,
    /// Depth of water (m, below the surface or transducer), when logged.
    pub depth: Option<f64>,
    /// Sea temperature (°C), when logged.
    pub temp: Option<f64>,
}

/// A fix as stored: [t, lat, lon, sog, cog, depth, temp], the trailing nulls left off
/// (so tracks without depth or temperature are no bigger than before).
#[derive(Deserialize)]
struct PointRow(
    Option<i64>,
    f64,
    f64,
    #[serde(default)] Option<f64>,
    #[serde(default)] Option<f64>,
    #[serde(default)] Option<f64>,
    #[serde(default)] Option<f64>,
);

impl Serialize for PointRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let rest = [self.3, self.4, self.5, self.6];
        let n = 3 + rest.iter().rposition(Option::is_some).map_or(0, |i| i + 1);
        let mut q = s.serialize_seq(Some(n))?;
        q.serialize_element(&self.0)?;
        q.serialize_element(&self.1)?;
        q.serialize_element(&self.2)?;
        for v in &rest[..n - 3] {
            q.serialize_element(v)?;
        }
        q.end()
    }
}

impl From<PointRow> for TrackPoint {
    fn from(r: PointRow) -> Self {
        TrackPoint { t: r.0, lat: r.1, lon: r.2, sog: r.3, cog: r.4, depth: r.5, temp: r.6 }
    }
}
impl From<TrackPoint> for PointRow {
    fn from(p: TrackPoint) -> Self {
        let r = |v: Option<f64>, k: f64| v.map(|x| (x * k).round() / k);
        PointRow(p.t, (p.lat * 1e6).round() / 1e6, (p.lon * 1e6).round() / 1e6, r(p.sog, 100.0), r(p.cog, 10.0), r(p.depth, 10.0), r(p.temp, 100.0))
    }
}

/// Totals for a track, kept with it so the list needn't read every point.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackStats {
    pub points: usize,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub distance_nm: f64,
    /// Highest SOG recorded or worked out between fixes, knots.
    pub max_sog: Option<f64>,
    /// [west, south, east, north]
    pub bbox: Option<[f64; 4]>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub notes: String,
    /// "#rrggbb"; empty: the app picks one.
    pub colour: String,
    /// "log" (recorded by this app) or "gpx" (imported).
    pub source: String,
    pub modified: i64,
    pub stats: TrackStats,
    /// Pieces of the track: a new segment starts after a gap in recording.
    pub segments: Vec<Vec<TrackPoint>>,
}

/// A track without its points, for lists.
#[derive(Debug, Clone, Serialize)]
pub struct TrackSummary {
    pub id: String,
    pub name: String,
    pub notes: String,
    pub colour: String,
    pub source: String,
    pub modified: i64,
    pub stats: TrackStats,
    /// Depth and sea temperature were logged along it (it can be coloured by them).
    #[serde(default)]
    pub has_depth: bool,
    #[serde(default)]
    pub has_temp: bool,
}

impl Track {
    pub fn summary(&self) -> TrackSummary {
        TrackSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            notes: self.notes.clone(),
            colour: self.colour.clone(),
            source: self.source.clone(),
            modified: self.modified,
            stats: self.stats.clone(),
            has_depth: self.segments.iter().flatten().any(|p| p.depth.is_some()),
            has_temp: self.segments.iter().flatten().any(|p| p.temp.is_some()),
        }
    }

    /// Work out [`TrackStats`] from the points.
    pub fn update_stats(&mut self) {
        let mut s = TrackStats::default();
        let mut bb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for seg in &self.segments {
            for (i, p) in seg.iter().enumerate() {
                s.points += 1;
                bb = [bb[0].min(p.lon), bb[1].min(p.lat), bb[2].max(p.lon), bb[3].max(p.lat)];
                if let Some(t) = p.t {
                    s.start = Some(s.start.map_or(t, |x| x.min(t)));
                    s.end = Some(s.end.map_or(t, |x| x.max(t)));
                }
                if let Some(v) = p.sog {
                    s.max_sog = Some(s.max_sog.map_or(v, |m: f64| m.max(v)));
                }
                if i > 0 {
                    let q = seg[i - 1];
                    let d = dist_nm(q.lat, q.lon, p.lat, p.lon);
                    s.distance_nm += d;
                    // speed between fixes counts only over a minute or more (GPS jitter)
                    if let (Some(a), Some(b)) = (q.t, p.t) {
                        if b - a >= 60 && p.sog.is_none() {
                            let v = d / ((b - a) as f64 / 3600.0);
                            s.max_sog = Some(s.max_sog.map_or(v, |m: f64| m.max(v)));
                        }
                    }
                }
            }
        }
        if s.points > 0 {
            s.bbox = Some(bb);
        }
        s.distance_nm = (s.distance_nm * 100.0).round() / 100.0;
        self.stats = s;
    }

    /// The track as GPX 1.1.
    pub fn to_gpx(&self) -> String {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"racenav\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
        );
        out.push_str(&self.trk_xml());
        out.push_str("</gpx>\n");
        out
    }

    /// The track as a GPX `<trk>` element.
    pub fn trk_xml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("  <trk>\n    <name>{}</name>\n", crate::gpx::esc(&self.name)));
        if !self.notes.is_empty() {
            out.push_str(&format!("    <desc>{}</desc>\n", crate::gpx::esc(&self.notes)));
        }
        for seg in &self.segments {
            out.push_str("    <trkseg>\n");
            for p in seg {
                out.push_str(&format!("      <trkpt lat=\"{:.6}\" lon=\"{:.6}\">", p.lat, p.lon));
                if let Some(t) = p.t {
                    out.push_str(&format!("<time>{}</time>", iso_time(t)));
                }
                // course and speed (GPX 1.0 elements, read by OpenCPN, qtVlm and most plotters) as extensions in 1.1
                if p.sog.is_some() || p.cog.is_some() {
                    out.push_str("<extensions>");
                    if let Some(c) = p.cog {
                        out.push_str(&format!("<course>{c:.1}</course>"));
                    }
                    if let Some(v) = p.sog {
                        out.push_str(&format!("<speed>{:.2}</speed>", v * 1852.0 / 3600.0));
                    }
                    out.push_str("</extensions>");
                }
                out.push_str("</trkpt>\n");
            }
            out.push_str("    </trkseg>\n");
        }
        out.push_str("  </trk>\n");
        out
    }
}

pub fn dist_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    nav_core::LatLon::new(lat1, lon1).distance_to(&nav_core::LatLon::new(lat2, lon2))
}

// ---------- ISO 8601 times (no date library needed) ----------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unix seconds to "2026-10-17T09:30:05Z".
pub fn iso_time(t: i64) -> String {
    let (d, s) = (t.div_euclid(86400), t.rem_euclid(86400));
    let (y, m, dd) = civil_from_days(d);
    format!("{y:04}-{m:02}-{dd:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

/// "2026-10-17T09:30:05Z", "…T09:30:05.123Z", "…T22:30:05+13:00" or a space for the T, to Unix seconds.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let off = match rest {
        "" | "Z" | "z" => 0,
        r if (r.starts_with('+') || r.starts_with('-')) && r.len() >= 3 => {
            let sign = if r.starts_with('-') { -1 } else { 1 };
            let r = &r[1..].replace(':', "");
            let hh: i64 = r.get(0..2)?.parse().ok()?;
            let mm: i64 = r.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(0);
            sign * (hh * 3600 + mm * 60)
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se - off)
}

// ---------- recording ----------

/// Rules for keeping fixes: enough to draw the track and see speed and turns,
/// not every fix the GPS sends.
#[derive(Debug, Clone, Copy)]
pub struct RecordRules {
    /// Keep a fix this long after the last one (seconds) if the boat has moved at least `min_move_m`.
    pub interval_s: i64,
    pub min_move_m: f64,
    /// A change of course this big (degrees) keeps a fix sooner, after `turn_s`.
    pub turn_deg: f64,
    pub turn_s: i64,
    /// Keep a fix at least this often even when not moving.
    pub idle_s: i64,
    /// A gap this long starts a new segment.
    pub gap_s: i64,
}

impl Default for RecordRules {
    fn default() -> Self {
        RecordRules { interval_s: 10, min_move_m: 5.0, turn_deg: 15.0, turn_s: 2, idle_s: 600, gap_s: 600 }
    }
}

/// Logs fixes into a track. The owner saves it from time to time ([`Recorder::due_save`]).
#[derive(Debug, Clone)]
pub struct Recorder {
    pub track: Track,
    pub rules: RecordRules,
    dirty: bool,
    saved_at: i64,
}

fn angle_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

impl Recorder {
    pub fn new(track: Track) -> Self {
        Recorder { track, rules: RecordRules::default(), dirty: false, saved_at: 0 }
    }

    /// Offer a fix; returns true if it was kept.
    pub fn push(&mut self, p: TrackPoint) -> bool {
        if !(p.lat.is_finite() && p.lon.is_finite()) || (p.lat == 0.0 && p.lon == 0.0) || p.lat.abs() > 90.0 {
            return false;
        }
        let Some(t) = p.t else { return false };
        let r = self.rules;
        let last = self.track.segments.last().and_then(|s| s.last()).copied();
        let new_segment = match last {
            None => true,
            Some(q) => {
                let qt = q.t.unwrap_or(t);
                if t <= qt && t >= qt - 1 {
                    return false; // same second
                }
                // a long gap, or time going backwards (a log played again): new segment
                if t - qt > r.gap_s || t < qt {
                    true
                } else {
                    let dt = t - qt;
                    let moved = dist_nm(q.lat, q.lon, p.lat, p.lon) * 1852.0;
                    let turned = match (q.cog, p.cog, p.sog) {
                        (Some(a), Some(b), Some(v)) if v > 1.0 => angle_diff(a, b) >= r.turn_deg,
                        _ => false,
                    };
                    let keep = (dt >= r.interval_s && moved >= r.min_move_m) || (turned && dt >= r.turn_s && moved >= 1.0) || dt >= r.idle_s;
                    if !keep {
                        return false;
                    }
                    false
                }
            }
        };
        if new_segment || self.track.segments.is_empty() {
            self.track.segments.push(Vec::new());
        }
        self.track.segments.last_mut().unwrap().push(p);
        // keep the totals current without walking the whole track
        let s = &mut self.track.stats;
        if let Some(q) = last.filter(|_| !new_segment) {
            s.distance_nm = ((s.distance_nm + dist_nm(q.lat, q.lon, p.lat, p.lon)) * 100.0).round() / 100.0;
        }
        s.points += 1;
        s.start = Some(s.start.map_or(t, |x| x.min(t)));
        s.end = Some(s.end.map_or(t, |x| x.max(t)));
        if let Some(v) = p.sog {
            s.max_sog = Some(s.max_sog.map_or(v, |m| m.max(v)));
        }
        s.bbox = Some(match s.bbox {
            None => [p.lon, p.lat, p.lon, p.lat],
            Some(b) => [b[0].min(p.lon), b[1].min(p.lat), b[2].max(p.lon), b[3].max(p.lat)],
        });
        self.dirty = true;
        true
    }

    /// True when there are unsaved fixes and the last save is `every_s` old (wall clock `now`).
    pub fn due_save(&self, now: i64, every_s: i64) -> bool {
        self.dirty && now - self.saved_at >= every_s
    }

    pub fn mark_saved(&mut self, now: i64) {
        self.dirty = false;
        self.saved_at = now;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(t: i64, lat: f64, lon: f64) -> TrackPoint {
        TrackPoint { t: Some(t), lat, lon, sog: Some(6.0), cog: Some(90.0), ..Default::default() }
    }

    #[test]
    fn iso_times() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        let t = parse_iso("2026-10-17T09:30:05Z").unwrap();
        assert_eq!(iso_time(t), "2026-10-17T09:30:05Z");
        assert_eq!(parse_iso("2026-10-17T22:30:05+13:00"), Some(t));
        assert_eq!(parse_iso("2026-10-17T09:30:05.250Z"), Some(t));
        assert_eq!(parse_iso("2026-10-17 09:30:05"), Some(t));
        assert_eq!(parse_iso("2024-02-29T00:00:00Z").map(iso_time).as_deref(), Some("2024-02-29T00:00:00Z"));
        assert_eq!(parse_iso("garbage"), None);
    }

    #[test]
    fn compact_points_round_trip() {
        let p = TrackPoint { t: Some(1_790_000_000), lat: -41.2612345678, lon: 173.28, sog: Some(6.234), cog: None, ..Default::default() };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(j, "[1790000000,-41.261235,173.28,6.23]"); // trailing nulls left off
        let d = TrackPoint { depth: Some(121.53), temp: Some(16.8), ..p };
        let j2 = serde_json::to_string(&d).unwrap();
        assert_eq!(j2, "[1790000000,-41.261235,173.28,6.23,null,121.5,16.8]");
        let d2: TrackPoint = serde_json::from_str(&j2).unwrap();
        assert_eq!((d2.depth, d2.temp, d2.cog), (Some(121.5), Some(16.8), None));
        let q: TrackPoint = serde_json::from_str(&j).unwrap();
        assert_eq!(q.t, p.t);
        assert!((q.lat - p.lat).abs() < 1e-6);
        let r: TrackPoint = serde_json::from_str("[null,-41.0,173.0]").unwrap();
        assert_eq!((r.t, r.sog), (None, None));
    }

    #[test]
    fn recorder_thins_and_splits() {
        let mut r = Recorder::new(Track::default());
        let t0 = 1_790_000_000;
        // a fix every second, sailing east at about 6 kn (3 m/s): kept every 10 s
        let mut kept = 0;
        for i in 0..60 {
            if r.push(pt(t0 + i, -41.0, 173.0 + i as f64 * 3.0 / 84_000.0)) {
                kept += 1;
            }
        }
        assert_eq!(kept, 6, "first fix plus one every 10 s");
        // a tack: course change keeps a fix within seconds
        let mut tack = pt(t0 + 53, -41.0, 173.0 + 53.0 * 3.0 / 84_000.0);
        tack.cog = Some(150.0);
        assert!(r.push(tack));
        // standing still: nothing kept until the idle interval
        let here = *r.track.segments[0].last().unwrap();
        assert!(!r.push(TrackPoint { t: Some(t0 + 100), ..here }));
        // a gap of an hour starts a new segment
        assert!(r.push(pt(t0 + 3700, -41.1, 173.1)));
        assert_eq!(r.track.segments.len(), 2);
        // time going backwards (a log replayed): new segment too
        assert!(r.push(pt(t0 + 50, -41.0, 173.0)));
        assert_eq!(r.track.segments.len(), 3);
        // running totals match a full recount
        let running = r.track.stats.clone();
        r.track.update_stats();
        assert_eq!(running.points, r.track.stats.points);
        assert!((running.distance_nm - r.track.stats.distance_nm).abs() < 0.02);
        // bad fixes are ignored
        assert!(!r.push(TrackPoint { t: Some(t0 + 9999), lat: 0.0, lon: 0.0, ..Default::default() }));
        assert!(!r.push(TrackPoint { t: None, lat: -41.0, lon: 173.0, ..Default::default() }));
    }

    #[test]
    fn writes_gpx() {
        let mut tr = Track { name: "Log & test".into(), ..Default::default() };
        tr.segments = vec![vec![pt(1_790_000_000, -41.0, 173.0), pt(1_790_000_060, -41.001, 173.002)]];
        let g = tr.to_gpx();
        assert!(g.contains("<name>Log &amp; test</name>"));
        assert!(g.contains("<trkpt lat=\"-41.000000\" lon=\"173.000000\"><time>2026-09-21T14:13:20Z</time>"));
        assert!(g.contains("<speed>3.09</speed>"));
        let back = crate::parse_gpx(&g).unwrap();
        assert_eq!(back.tracks.len(), 1);
        let t = &back.tracks[0];
        assert_eq!(t.name, "Log & test");
        assert_eq!(t.segments[0].len(), 2);
        assert_eq!(t.segments[0][1].t, Some(1_790_000_060));
        assert!((t.segments[0][0].sog.unwrap() - 6.0).abs() < 0.01);
        assert_eq!(t.segments[0][0].cog, Some(90.0));
    }
}
