//! New Zealand tide predictions from LINZ (Toitū Te Whenua), offline.
//!
//! Two kinds of LINZ file, both downloaded once into a folder by [`download`]:
//!
//! - **Daily tables** for about 90 places, one CSV per place and year
//!   (`static.charts.linz.govt.nz/tide-tables/maj-ports/csv/<Name> <year>.csv`): the
//!   times and heights of every high and low water, in local time (NZST/NZDT, or
//!   Chatham time for the Chatham Islands).
//! - **Secondary ports** (`NZNA_Secondary-ports-<years>.csv`): about 330 more places,
//!   each with a reference (standard) port, mean time differences for high and low
//!   water, mean levels and a range ratio.
//!
//! A secondary port's tides are its reference port's, shifted by the time
//! differences, with heights `MSL_sec + (h_ref - MSL_ref) × ratio`, the method LINZ
//! gives. Between a high and a low, the height follows LINZ's cosine curve.
//!
//! LINZ predictions are Crown copyright; they are not official tide tables under
//! Maritime Rules Part 25. Not for navigation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, NaiveDate, Weekday};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("download: {0}")]
    Http(String),
    #[error("{0}")]
    Format(String),
    #[error("cancelled")]
    Cancelled,
}

/// A high or low water.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extreme {
    /// Unix time, seconds (UTC).
    pub t: i64,
    /// Height above chart datum, metres (None when LINZ gives no heights for a secondary port).
    pub h: Option<f64>,
    pub high: bool,
}

/// Mean tide levels above chart datum, metres.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Levels {
    pub mhws: Option<f64>,
    pub mhwn: Option<f64>,
    pub mlwn: Option<f64>,
    pub mlws: Option<f64>,
    pub msl: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// LINZ publishes a daily table for this place.
    Daily,
    /// Predicted from a reference port by LINZ's time and height differences.
    Secondary {
        reference: String,
        /// Mean high water time difference, minutes.
        hw_min: i32,
        /// Mean low water time difference, minutes (the high water one when LINZ gives none).
        lw_min: i32,
        /// Range ratio (None: times only).
        ratio: Option<f64>,
    },
    /// Listed by LINZ but without time differences: levels only.
    LevelsOnly { reference: String },
}

#[derive(Debug, Clone)]
pub struct Station {
    /// "d:<name>" for a daily port, "s:<LINZ number>" for a secondary port.
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub kind: Kind,
    pub levels: Levels,
    /// LINZ marks the data as approximate.
    pub approx: bool,
}

// ---------- time ----------------------------------------------------------

fn last_sunday(year: i32, month: u32) -> NaiveDate {
    let mut d = NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap().pred_opt().unwrap();
    while d.weekday() != Weekday::Sun {
        d = d.pred_opt().unwrap();
    }
    d
}

fn first_sunday(year: i32, month: u32) -> NaiveDate {
    let mut d = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    while d.weekday() != Weekday::Sun {
        d = d.succ_opt().unwrap();
    }
    d
}

fn ts(d: NaiveDate, h: u32, m: u32) -> i64 {
    d.and_hms_opt(h, m, 0).unwrap().and_utc().timestamp()
}

/// True if NZ daylight time applies at Unix time `t`. NZDT runs from 02:00 NZST on
/// the last Sunday in September to 03:00 NZDT on the first Sunday in April (both
/// 14:00 UTC the day before); the Chathams change at the same instant.
pub fn nz_daylight(t: i64) -> bool {
    let year = chrono::DateTime::from_timestamp(t, 0).map_or(2000, |d| d.year());
    let end = ts(first_sunday(year, 4), 3, 0) - 13 * 3600;
    let start = ts(last_sunday(year, 9), 2, 0) - 12 * 3600;
    t < end || t >= start
}

/// Offset of local time from UTC at `t`, minutes: NZ (+12:00/+13:00) or Chatham (+12:45/+13:45).
pub fn local_offset_min(t: i64, chatham: bool) -> i32 {
    (if nz_daylight(t) { 780 } else { 720 }) + if chatham { 45 } else { 0 }
}

/// Unix time of a local wall-clock time. In the hour repeated when daylight time
/// ends, `after` (the previous event) picks the reading that keeps times in order.
fn local_to_utc(d: NaiveDate, h: u32, m: u32, chatham: bool, after: Option<i64>) -> i64 {
    let wall = ts(d, h, m);
    let extra = if chatham { 45 * 60 } else { 0 };
    let daylight = wall - 13 * 3600 - extra;
    let standard = wall - 12 * 3600 - extra;
    let d_ok = nz_daylight(daylight);
    let s_ok = !nz_daylight(standard);
    match (d_ok, s_ok) {
        (true, true) => {
            if after.is_some_and(|a| daylight <= a) {
                standard
            } else {
                daylight
            }
        }
        (true, false) => daylight,
        (false, true) => standard,
        // in the hour skipped when daylight time starts: read it as standard time
        (false, false) => standard,
    }
}

// ---------- LINZ files ----------------------------------------------------------

/// The first line of a daily table: number, name and position.
#[derive(Debug, Clone, PartialEq)]
pub struct DailyHeader {
    pub number: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// "41°16'S" or "176°34'W" to signed degrees.
fn parse_dm(s: &str) -> Option<f64> {
    let s = s.trim();
    let hemi = s.chars().last()?;
    let body = &s[..s.len() - hemi.len_utf8()];
    let (d, m) = body.split_once('°')?;
    let m = m.trim_end_matches(['\'', '´', '’']);
    let v = d.trim().parse::<f64>().ok()? + m.trim().parse::<f64>().ok()? / 60.0;
    Some(if matches!(hemi, 'S' | 'W' | 's' | 'w') { -v } else { v })
}

fn is_chatham(name: &str) -> bool {
    name.to_ascii_lowercase().contains("chatham")
}

/// A LINZ daily table: header and every high and low water, in time order.
pub fn parse_daily(text: &str) -> Result<(DailyHeader, Vec<Extreme>), Error> {
    let mut lines = text.lines();
    let first = lines.next().ok_or_else(|| Error::Format("empty tide table".into()))?;
    let f: Vec<&str> = first.split(',').map(str::trim).collect();
    if f.len() < 4 {
        return Err(Error::Format(format!("not a LINZ tide table: {first}")));
    }
    let header = DailyHeader {
        number: f[0].to_string(),
        name: f[1].to_string(),
        lat: parse_dm(f[2]).ok_or_else(|| Error::Format(format!("latitude {}", f[2])))?,
        lon: parse_dm(f[3]).ok_or_else(|| Error::Format(format!("longitude {}", f[3])))?,
    };
    let chatham = is_chatham(&header.name);
    let mut raw: Vec<(i64, f64)> = Vec::new();
    for line in lines {
        let c: Vec<&str> = line.split(',').map(str::trim).collect();
        if c.len() < 6 {
            continue;
        }
        let (Ok(day), Ok(month), Ok(year)) = (c[0].parse::<u32>(), c[2].parse::<u32>(), c[3].parse::<i32>()) else { continue };
        let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else { continue };
        for pair in c[4..].chunks(2) {
            let [tm, h] = pair else { continue };
            let Some((hh, mm)) = tm.split_once(':') else { continue };
            let (Ok(hh), Ok(mm), Ok(h)) = (hh.parse::<u32>(), mm.parse::<u32>(), h.parse::<f64>()) else { continue };
            let t = local_to_utc(date, hh, mm, chatham, raw.last().map(|x| x.0));
            raw.push((t, h));
        }
    }
    Ok((header, mark_highs(raw)))
}

/// Times and heights to extremes: a high is higher than the one before it (or after it, for the first).
fn mark_highs(mut raw: Vec<(i64, f64)>) -> Vec<Extreme> {
    raw.sort_by_key(|x| x.0);
    raw.dedup_by_key(|x| x.0);
    (0..raw.len())
        .map(|i| {
            let (t, h) = raw[i];
            let other = if i > 0 { raw[i - 1].1 } else { raw.get(1).map_or(h, |x| x.1) };
            Extreme { t, h: Some(h), high: h > other }
        })
        .collect()
}

/// One place in the LINZ secondary ports file.
#[derive(Debug, Clone, PartialEq)]
pub struct SecondaryRow {
    pub number: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// The standard port the group belongs to (for a standard port, itself).
    pub reference: String,
    /// True for the standard port at the head of a group ("hhmm" time differences).
    pub standard: bool,
    pub hw_min: Option<i32>,
    pub lw_min: Option<i32>,
    pub levels: Levels,
    pub ratio: Option<f64>,
    pub approx: bool,
}

/// "+0022", "-0104", "0000" to minutes.
fn parse_hhmm(s: &str) -> Option<i32> {
    let s = s.trim().replace(' ', "");
    let (sign, digits) = match s.strip_prefix('-') {
        Some(r) => (-1, r.to_string()),
        None => (1, s.trim_start_matches('+').to_string()),
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: i32 = digits.parse().ok()?;
    Some(sign * (v / 100 * 60 + v % 100))
}

fn num(s: &str) -> Option<f64> {
    s.trim().parse().ok()
}

/// The LINZ secondary ports file (NZ Nautical Almanac, "NZNA_Secondary-ports-*.csv").
pub fn parse_secondary(text: &str) -> Vec<SecondaryRow> {
    let mut out = Vec::new();
    let mut reference = String::new();
    for line in text.lines() {
        let c: Vec<&str> = line.split(',').map(str::trim).collect();
        if c.len() < 15 || c[0].is_empty() || !c[0].starts_with(|ch: char| ch.is_ascii_digit()) {
            continue;
        }
        let (Some(lat_d), Some(lat_m), Some(lon_d)) = (num(c[2]), num(c[3]), num(c[4])) else { continue };
        let west = c[5].ends_with(['W', 'w']);
        let Some(lon_m) = num(c[5].trim_end_matches(['W', 'w', 'E', 'e'])) else { continue };
        let lat = -(lat_d + lat_m / 60.0);
        let lon = (lon_d + lon_m / 60.0) * if west { -1.0 } else { 1.0 };
        let standard = c[6].eq_ignore_ascii_case("hhmm");
        if standard {
            reference = c[1].to_string();
        }
        out.push(SecondaryRow {
            number: c[0].to_string(),
            name: c[1].to_string(),
            lat,
            lon,
            reference: reference.clone(),
            standard,
            hw_min: if standard { Some(0) } else { parse_hhmm(c[6]) },
            lw_min: if standard { Some(0) } else { parse_hhmm(c[8]) },
            levels: Levels { mhws: num(c[10]), mhwn: num(c[11]), mlwn: num(c[12]), mlws: num(c[13]), msl: num(c[14]) },
            ratio: if standard { Some(1.0) } else { c.get(15).and_then(|s| num(s)) },
            approx: c.get(16).is_some_and(|s| s.contains('*')),
        });
    }
    out
}

// ---------- the database ----------------------------------------------------------

fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn dist_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dy = (a.0 - b.0) * 111.2;
    let dx = (a.1 - b.1) * 111.2 * a.0.to_radians().cos();
    (dx * dx + dy * dy).sqrt()
}

/// Every station and the daily tables, from a folder written by [`download`].
#[derive(Debug, Default)]
pub struct TideDb {
    pub stations: Vec<Station>,
    /// Extremes of each daily port, by name key.
    series: HashMap<String, Vec<Extreme>>,
    /// Mean sea level of each standard port, by name key.
    ref_msl: HashMap<String, f64>,
    /// Years with daily tables.
    pub years: Vec<i32>,
}

/// File name of the secondary ports table in the tide folder.
pub const SECONDARY_FILE: &str = "secondary-ports.csv";

impl TideDb {
    /// Read the tide folder (missing or empty: no stations).
    pub fn open(dir: &Path) -> Self {
        let mut db = TideDb::default();
        let mut headers: HashMap<String, DailyHeader> = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
            files.sort();
            for p in files {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if !name.ends_with(".csv") || name == SECONDARY_FILE {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&p) else { continue };
                let Ok((h, ex)) = parse_daily(&text) else { continue };
                if let Some(y) = name.trim_end_matches(".csv").rsplit(' ').next().and_then(|y| y.parse().ok()) {
                    if !db.years.contains(&y) {
                        db.years.push(y);
                    }
                }
                let k = key(&h.name);
                let s = db.series.entry(k.clone()).or_default();
                s.extend(ex);
                s.sort_by_key(|e| e.t);
                s.dedup_by_key(|e| e.t);
                headers.insert(k, h);
            }
        }
        db.years.sort();
        let rows = std::fs::read_to_string(dir.join(SECONDARY_FILE)).map(|t| parse_secondary(&t)).unwrap_or_default();
        db.build(headers, rows);
        db
    }

    fn build(&mut self, headers: HashMap<String, DailyHeader>, rows: Vec<SecondaryRow>) {
        for r in rows.iter().filter(|r| r.standard) {
            if let Some(m) = r.levels.msl {
                self.ref_msl.insert(key(&r.name), m);
            }
        }
        let mut daily: Vec<&DailyHeader> = headers.values().collect();
        daily.sort_by(|a, b| a.name.cmp(&b.name));
        for h in &daily {
            // levels from the secondary file, for the same place
            let same = rows.iter().find(|r| key(&r.name) == key(&h.name) || dist_km((r.lat, r.lon), (h.lat, h.lon)) < 3.0);
            self.stations.push(Station {
                id: format!("d:{}", h.name),
                name: h.name.clone(),
                lat: h.lat,
                lon: h.lon,
                kind: Kind::Daily,
                levels: same.map(|r| r.levels).unwrap_or_default(),
                approx: false,
            });
        }
        for r in rows.iter().filter(|r| !r.standard) {
            // a place LINZ also publishes a daily table for is shown once, as daily
            if daily.iter().any(|h| dist_km((r.lat, r.lon), (h.lat, h.lon)) < 3.0) {
                continue;
            }
            let kind = match r.hw_min {
                Some(hw) => Kind::Secondary { reference: r.reference.clone(), hw_min: hw, lw_min: r.lw_min.unwrap_or(hw), ratio: r.ratio },
                None => Kind::LevelsOnly { reference: r.reference.clone() },
            };
            self.stations.push(Station { id: format!("s:{}", r.number), name: r.name.clone(), lat: r.lat, lon: r.lon, kind, levels: r.levels, approx: r.approx });
        }
    }

    pub fn station(&self, id: &str) -> Option<&Station> {
        self.stations.iter().find(|s| s.id == id)
    }

    /// High and low waters at a station between `t0` and `t1` (plus one either side, for curves).
    pub fn extremes(&self, id: &str, t0: i64, t1: i64) -> Option<Vec<Extreme>> {
        let s = self.station(id)?;
        let window = |v: &[Extreme], shift: i64| -> Vec<Extreme> {
            let i0 = v.partition_point(|e| e.t + shift < t0).saturating_sub(1);
            let i1 = (v.partition_point(|e| e.t + shift <= t1) + 1).min(v.len());
            v[i0..i1].to_vec()
        };
        match &s.kind {
            Kind::Daily => Some(window(self.series.get(&key(&s.name))?, 0)),
            Kind::Secondary { reference, hw_min, lw_min, ratio } => {
                let base = self.series.get(&key(reference))?;
                let msl_ref = self.ref_msl.get(&key(reference)).copied();
                let shift = (*hw_min).min(*lw_min) as i64 * 60;
                let mut out: Vec<Extreme> = window(base, shift.min(0))
                    .into_iter()
                    .map(|e| {
                        let dt = if e.high { *hw_min } else { *lw_min } as i64 * 60;
                        let h = match (e.h, s.levels.msl, msl_ref, ratio) {
                            (Some(h), Some(ms), Some(mr), Some(r)) => Some(ms + (h - mr) * r),
                            _ => None,
                        };
                        Extreme { t: e.t + dt, h, high: e.high }
                    })
                    .collect();
                out.sort_by_key(|e| e.t);
                Some(out)
            }
            Kind::LevelsOnly { .. } => None,
        }
    }

    /// A standard port with a daily table, by name (spaces, case and punctuation ignored).
    pub fn daily_by_name(&self, name: &str) -> Option<&Station> {
        let k = key(name);
        self.stations.iter().find(|s| s.kind == Kind::Daily && key(&s.name) == k)
    }

    /// Where the tide is at `t` between the low and high water either side: 0 at low
    /// water, 1 at high (LINZ's cosine curve), and whether it is rising.
    pub fn phase_at(&self, id: &str, t: i64) -> Option<(f64, bool)> {
        let ext = self.extremes(id, t - 14 * 3600, t + 14 * 3600)?;
        let i = ext.partition_point(|e| e.t <= t);
        let (a, b) = (ext.get(i.checked_sub(1)?)?, ext.get(i)?);
        if b.t - a.t > 14 * 3600 || b.t <= a.t || a.high == b.high {
            return None;
        }
        let x = (t - a.t) as f64 / (b.t - a.t) as f64;
        let up = (1.0 - (std::f64::consts::PI * x).cos()) / 2.0;
        Some(if b.high { (up, true) } else { (1.0 - up, false) })
    }

    /// The high (or low, `high` false) water nearest `t`, within 7 hours, and that tide's
    /// range as a fraction of the mean spring range (for scaling tidal stream rates).
    pub fn nearest_reference(&self, id: &str, t: i64, high: bool) -> Option<(i64, Option<f64>)> {
        let s = self.station(id)?;
        let ext = self.extremes(id, t - 14 * 3600, t + 14 * 3600)?;
        let (i, e) = ext.iter().enumerate().filter(|(_, e)| e.high == high).min_by_key(|(_, e)| (e.t - t).abs())?;
        if (e.t - t).abs() > 7 * 3600 {
            return None;
        }
        let other: Vec<f64> = [i.checked_sub(1).and_then(|j| ext.get(j)), ext.get(i + 1)]
            .into_iter()
            .flatten()
            .filter(|x| x.high != high)
            .filter_map(|x| x.h)
            .collect();
        let spring = match (s.levels.mhws, s.levels.mlws) {
            (Some(a), Some(b)) if a > b => Some(a - b),
            _ => None,
        };
        let factor = match (e.h, spring) {
            (Some(h), Some(sp)) if !other.is_empty() => {
                let o = other.iter().sum::<f64>() / other.len() as f64;
                Some(((h - o).abs() / sp).clamp(0.2, 1.5))
            }
            _ => None,
        };
        Some((e.t, factor))
    }

    /// Height at `t` (LINZ cosine curve between the extremes either side).
    pub fn height_at(ext: &[Extreme], t: i64) -> Option<f64> {
        let i = ext.partition_point(|e| e.t <= t);
        let (a, b) = (ext.get(i.checked_sub(1)?)?, ext.get(i)?);
        let (ha, hb) = (a.h?, b.h?);
        // a gap in the data (more than 14 h between tides): no curve across it
        if b.t - a.t > 14 * 3600 || b.t == a.t {
            return None;
        }
        let x = (t - a.t) as f64 / (b.t - a.t) as f64;
        Some(ha + (hb - ha) * (1.0 - (std::f64::consts::PI * x).cos()) / 2.0)
    }

    /// The tide curve from `t0` to `t1` every `step` seconds: (time, height).
    pub fn curve(&self, id: &str, t0: i64, t1: i64, step: i64) -> Vec<(i64, f64)> {
        let Some(ext) = self.extremes(id, t0, t1) else { return Vec::new() };
        let mut out = Vec::new();
        let mut t = t0;
        while t <= t1 {
            if let Some(h) = Self::height_at(&ext, t) {
                out.push((t, h));
            }
            t += step.max(60);
        }
        out
    }
}

// ---------- download ----------------------------------------------------------

const LIST_PAGE: &str = "https://www.linz.govt.nz/products-services/tides-and-tidal-streams/tide-predictions/tide-predictions-list-view";
const MAIN_PAGE: &str = "https://www.linz.govt.nz/products-services/tides-and-tidal-streams/tide-predictions";

/// Progress of a download: what is happening, files done, files in all.
#[derive(Debug, Clone)]
pub struct Progress {
    pub stage: String,
    pub done: usize,
    pub total: usize,
}

fn client() -> Result<reqwest::blocking::Client, Error> {
    reqwest::blocking::Client::builder()
        .user_agent("racenav (tide predictions; chris@tweedsmarine.nz)")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Http(e.to_string()))
}

fn get_text(c: &reqwest::blocking::Client, url: &str) -> Result<String, Error> {
    let r = c.get(url).send().map_err(|e| Error::Http(format!("{url}: {e}")))?;
    if !r.status().is_success() {
        return Err(Error::Http(format!("{url}: HTTP {}", r.status())));
    }
    r.text().map_err(|e| Error::Http(e.to_string()))
}

fn decode_url(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).replace("&amp;", "&")
}

/// Every href in an HTML page.
fn hrefs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("href=\"") {
        rest = &rest[i + 6..];
        if let Some(j) = rest.find('"') {
            out.push(rest[..j].to_string());
            rest = &rest[j..];
        }
    }
    out
}

/// Daily-table links on the LINZ list page: (place, year, url).
pub fn daily_links(html: &str) -> Vec<(String, i32, String)> {
    let mut out: Vec<(String, i32, String)> = Vec::new();
    for h in hrefs(html) {
        let Some(i) = h.find("/maj-ports/csv/") else { continue };
        let file = decode_url(&h[i + 15..]);
        let Some(stem) = file.strip_suffix(".csv") else { continue };
        let Some((name, year)) = stem.rsplit_once(' ') else { continue };
        let Ok(year) = year.parse() else { continue };
        let url = if h.starts_with("http") { h.clone() } else { format!("https://static.charts.linz.govt.nz{}", &h[h.find("/tide-tables").unwrap_or(0)..]) };
        if !out.iter().any(|x| x.0 == name && x.1 == year) {
            out.push((name.to_string(), year, url));
        }
    }
    out
}

/// Download the LINZ secondary ports file and the daily tables for `years` into
/// `dir` (files already there are kept). Returns the number of files written.
pub fn download(dir: &Path, years: &[i32], cancel: &AtomicBool, mut progress: impl FnMut(Progress)) -> Result<usize, Error> {
    std::fs::create_dir_all(dir)?;
    let c = client()?;
    progress(Progress { stage: "Reading the LINZ tide pages".into(), done: 0, total: 0 });
    let main = get_text(&c, MAIN_PAGE)?;
    let sec = hrefs(&main)
        .into_iter()
        .find(|h| h.to_ascii_lowercase().contains("secondary") && h.ends_with(".csv"))
        .ok_or_else(|| Error::Http("no secondary ports file on the LINZ tide page".into()))?;
    let sec_url = if sec.starts_with("http") { sec } else { format!("https://www.linz.govt.nz{sec}") };
    let list = get_text(&c, LIST_PAGE)?;
    let links: Vec<_> = daily_links(&list).into_iter().filter(|l| years.contains(&l.1)).collect();
    if links.is_empty() {
        return Err(Error::Http(format!("LINZ lists no daily tide tables for {years:?}")));
    }
    let total = links.len() + 1;
    let write = |name: &str, body: &str| -> Result<(), Error> {
        let tmp = dir.join(format!("{name}.part"));
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, dir.join(name))?;
        Ok(())
    };
    let text = get_text(&c, &sec_url)?;
    if parse_secondary(&text).is_empty() {
        return Err(Error::Format("the secondary ports file has no ports".into()));
    }
    write(SECONDARY_FILE, &text)?;
    let mut written = 1;
    for (i, (name, year, url)) in links.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        progress(Progress { stage: format!("{name} {year}"), done: i + 1, total });
        let file = format!("{name} {year}.csv").replace(['/', '\\', ':'], "-");
        if dir.join(&file).exists() {
            continue;
        }
        let body = get_text(&c, url)?;
        parse_daily(&body)?;
        write(&file, &body)?;
        written += 1;
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    progress(Progress { stage: "done".into(), done: total, total });
    Ok(written)
}

#[cfg(test)]
mod tests {
    #[test]
    fn phase_and_reference() {
        let mut db = TideDb::default();
        let hw = 1_800_000_000i64;
        let ext = vec![
            Extreme { t: hw - 6 * 3600 - 1200, h: Some(0.5), high: false },
            Extreme { t: hw, h: Some(3.5), high: true },
            Extreme { t: hw + 6 * 3600 + 1200, h: Some(0.7), high: false },
        ];
        db.series.insert(key("Nelson"), ext);
        db.stations.push(Station { id: "d:Nelson".into(), name: "Nelson".into(), lat: -41.26, lon: 173.28, kind: Kind::Daily,
            levels: Levels { mhws: Some(4.0), mlws: Some(0.4), ..Default::default() }, approx: false });
        assert_eq!(db.daily_by_name("NELSON").unwrap().id, "d:Nelson");
        let (f, up) = db.phase_at("d:Nelson", hw - 3 * 3600 - 600).unwrap();
        assert!(up && (f - 0.5).abs() < 0.02, "{f}");
        let (f, up) = db.phase_at("d:Nelson", hw + 600).unwrap();
        assert!(!up && f > 0.95);
        let (t, k) = db.nearest_reference("d:Nelson", hw + 2 * 3600, true).unwrap();
        assert_eq!(t, hw);
        assert!((k.unwrap() - 2.9 / 3.6).abs() < 0.01);
        assert!(db.nearest_reference("d:Nelson", hw + 8 * 3600, true).is_none());
    }

    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        ts(NaiveDate::from_ymd_opt(y, mo, d).unwrap(), h, mi)
    }

    #[test]
    fn daylight_rule() {
        // 2026: NZDT ends Sun 5 Apr 03:00 (Sat 4 Apr 14:00 UTC), starts Sun 27 Sep 02:00 (Sat 26 Sep 14:00 UTC)
        assert!(nz_daylight(utc(2026, 4, 4, 13, 59)));
        assert!(!nz_daylight(utc(2026, 4, 4, 14, 0)));
        assert!(!nz_daylight(utc(2026, 9, 26, 13, 59)));
        assert!(nz_daylight(utc(2026, 9, 26, 14, 0)));
        assert!(nz_daylight(utc(2026, 1, 1, 0, 0)));
        assert!(!nz_daylight(utc(2026, 7, 1, 0, 0)));
    }

    #[test]
    fn reads_a_daily_table() {
        let text = std::fs::read_to_string(fixtures().join("Nelson 2026.csv")).unwrap();
        let (h, ex) = parse_daily(&text).unwrap();
        assert_eq!(h.name, "Nelson");
        assert!((h.lat + 41.2667).abs() < 0.001 && (h.lon - 173.2667).abs() < 0.001);
        // 1 Jan 2026 01:51 NZDT = 31 Dec 12:51 UTC, low 1.1 m; then high 3.7 m at 08:37
        assert_eq!(ex[0], Extreme { t: utc(2025, 12, 31, 12, 51), h: Some(1.1), high: false });
        assert_eq!(ex[1], Extreme { t: utc(2025, 12, 31, 19, 37), h: Some(3.7), high: true });
        // highs and lows alternate everywhere, through both daylight changes
        for w in ex.windows(2) {
            assert!(w[1].t > w[0].t);
            if w[1].t - w[0].t < 14 * 3600 {
                assert_ne!(w[0].high, w[1].high, "{:?}", w);
            }
        }
        // 5 Apr (25 h day): 00:25 NZDT, then 05:16 NZST
        let a = ex.iter().find(|e| e.t == utc(2026, 4, 4, 11, 25)).unwrap();
        assert!(a.high);
        assert!(ex.iter().any(|e| e.t == utc(2026, 4, 4, 17, 16) && !e.high));
        // 27 Sep (23 h day): 04:34 NZDT = 26 Sep 15:34 UTC
        assert!(ex.iter().any(|e| e.t == utc(2026, 9, 26, 15, 34)));
    }

    #[test]
    fn chatham_tables_are_in_chatham_time() {
        let text = std::fs::read_to_string(fixtures().join("Waitangi - Chatham Island 2026.csv")).unwrap();
        let (h, ex) = parse_daily(&text).unwrap();
        assert!(h.lon < -176.0);
        // 03:43 CHADT (+13:45) = 31 Dec 13:58 UTC
        assert_eq!(ex[0].t, utc(2025, 12, 31, 13, 58));
        // LINZ's Lyttelton + 22 min offset for Waitangi lands within the published range
        let ly = parse_daily(&std::fs::read_to_string(fixtures().join("Lyttelton 2026.csv")).unwrap()).unwrap().1;
        let d = (ex[0].t - ly[0].t) / 60;
        assert!((-4..=52).contains(&d), "{d} min");
    }

    #[test]
    fn reads_secondary_ports() {
        let rows = parse_secondary(&std::fs::read_to_string(fixtures().join(SECONDARY_FILE)).unwrap());
        let n = |s: &str| rows.iter().find(|r| r.name == s).unwrap();
        assert!(n("Nelson").standard);
        let m = n("Māpua");
        assert_eq!((m.reference.as_str(), m.hw_min, m.lw_min, m.ratio), ("Nelson", Some(19), Some(19), Some(0.97)));
        assert!((m.lat + 41.25).abs() < 1e-9 && (m.lon - 173.1).abs() < 1e-9);
        let w = n("Waitangi");
        assert_eq!(w.reference, "Lyttelton");
        assert!(w.lon < -176.5);
        assert_eq!(n("Rakaia River Mouth").lw_min, None);
        assert!(n("Astrolabe Roadstead").approx && n("Astrolabe Roadstead").hw_min.is_none());
        assert_eq!(n("Stephens Island / Takapourewa").hw_min, Some(-30));
    }

    #[test]
    fn database_and_curves() {
        let db = TideDb::open(&fixtures());
        assert_eq!(db.years, vec![2026]);
        // Waitangi secondary is left out: LINZ has a daily table there
        assert!(db.stations.iter().all(|s| s.id != "s:6536"));
        let nelson = db.station("d:Nelson").unwrap();
        assert_eq!(nelson.levels.msl, Some(2.4));
        // Māpua: Nelson + 19 min, heights 2.4 + (h - 2.40) × 0.97
        let t0 = utc(2025, 12, 31, 12, 0);
        let t1 = utc(2026, 1, 1, 12, 0);
        let ex = db.extremes("s:6455b", t0, t1).unwrap();
        let first = ex.iter().find(|e| e.t >= utc(2025, 12, 31, 13, 0)).unwrap();
        assert_eq!(first.t, utc(2025, 12, 31, 13, 10));
        assert!((first.h.unwrap() - (2.4 + (1.1 - 2.4) * 0.97)).abs() < 1e-9);
        // the curve passes through the extremes and stays between them
        let c = db.curve("d:Nelson", utc(2025, 12, 31, 12, 51), utc(2025, 12, 31, 19, 37), 600);
        assert!((c[0].1 - 1.1).abs() < 1e-6);
        assert!(c.iter().all(|p| p.1 >= 1.1 - 1e-9 && p.1 <= 3.7 + 1e-9));
        let mid = TideDb::height_at(&db.extremes("d:Nelson", t0, t1).unwrap(), (utc(2025, 12, 31, 12, 51) + utc(2025, 12, 31, 19, 37)) / 2).unwrap();
        assert!((mid - 2.4).abs() < 1e-6);
        // no curve across a gap in the tables
        assert!(TideDb::height_at(&db.extremes("d:Nelson", utc(2026, 1, 6, 0, 0), utc(2026, 4, 4, 0, 0)).unwrap(), utc(2026, 2, 1, 0, 0)).is_none());
        // a place with levels only gives no tides
        assert!(db.extremes("s:6455", t0, t1).is_none());
    }

    #[test]
    fn finds_daily_links() {
        let html = r#"<a href="https://static.charts.linz.govt.nz/tide-tables/maj-ports/csv/Nelson%202026.csv">2026.csv</a>
            <a href="https://static.charts.linz.govt.nz/tide-tables/maj-ports/pdf/Nelson%202026.pdf">pdf</a>
            <a href="https://static.charts.linz.govt.nz/tide-tables/maj-ports/csv/Waitangi%20-%20Chatham%20Island%202027.csv">x</a>"#;
        let l = daily_links(html);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].0.as_str(), l[0].1), ("Nelson", 2026));
        assert_eq!((l[1].0.as_str(), l[1].1), ("Waitangi - Chatham Island", 2027));
    }
}
