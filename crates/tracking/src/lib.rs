//! Race tracking from YB (YellowBrick).
//!
//! Two feeds per race, both public and used by navigation software:
//! - the race setup, `https://cf.yb.tl/JSON/<race>/RaceSetup`: boats (id, name,
//!   sail number, model, colour, classes), classes ("tags") and the course;
//! - the light Expedition position file, `https://yb.tl/<race>-expedition.txt`:
//!   one line per boat with its latest fix, `id,lat,lon,YYMMDDHHMM` (UTC);
//! - the full history, `https://cf.yb.tl/BIN/<race>/AllPositions3`: every
//!   published fix of every boat, with YB's distance to finish (binary, see
//!   [`parse_all_positions`]). This is what YB's own race viewer loads.
//!
//! Every fix is kept on disk per race:
//!
//! ```text
//! <dir>/<race>/setup.json     the race setup as YB sent it
//! <dir>/<race>/fixes.json     { "<boat id>": [[unix s, lat, lon, dtf NM or -1], …] }
//! <dir>/<race>/meta.json      when each feed was last fetched
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    Format(String),
}
pub type Result<T> = std::result::Result<T, Error>;

pub const SETUP_URL: &str = "https://cf.yb.tl/JSON/{race}/RaceSetup";
pub const POSITIONS_URL: &str = "https://yb.tl/{race}-expedition.txt";
pub const HISTORY_URL: &str = "https://cf.yb.tl/BIN/{race}/AllPositions3";

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Team {
    pub id: u32,
    pub name: String,
    pub sail: String,
    pub model: String,
    /// "#rrggbb"
    pub colour: String,
    pub skipper: String,
    pub country: String,
    pub status: String,
    /// Class (tag) ids.
    pub tags: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Tag {
    pub id: u32,
    pub name: String,
    pub sort: i64,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct CoursePoint {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Setup {
    pub title: String,
    /// Race start, unix seconds, if YB gives one.
    pub start: Option<i64>,
    pub teams: Vec<Team>,
    pub tags: Vec<Tag>,
    pub course: Vec<CoursePoint>,
    /// Course length as YB gives it (NM when the race uses nautical units).
    pub course_nm: Option<f64>,
}

/// One position report.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Fix {
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
    /// YB's distance to finish, NM (from the history file only).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dtf: Option<f64>,
}

// ---------- parsing ----------

/// Read a RaceSetup document. Fields YB doesn't send are left empty.
pub fn parse_setup(text: &str) -> Result<Setup> {
    let v: Value = serde_json::from_str(text)?;
    if !v.is_object() {
        return Err(Error::Format("race setup is not a JSON object".into()));
    }
    let s = |x: &Value, k: &str| x.get(k).and_then(|y| y.as_str().map(str::to_string).or_else(|| y.as_i64().map(|n| n.to_string()))).unwrap_or_default();
    let num = |x: &Value, k: &str| x.get(k).and_then(|y| y.as_f64().or_else(|| y.as_str().and_then(|t| t.parse().ok())));
    let teams = v
        .get("teams")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let id = num(t, "id")? as u32;
                    Some(Team {
                        id,
                        name: s(t, "name"),
                        sail: s(t, "sail"),
                        model: s(t, "model"),
                        colour: colour(&s(t, "colour")),
                        skipper: [s(t, "captain"), s(t, "owner")].into_iter().find(|x| !x.is_empty()).unwrap_or_default(),
                        country: s(t, "country"),
                        status: s(t, "status"),
                        tags: t.get("tags").and_then(Value::as_array).map(|x| x.iter().filter_map(|y| y.as_u64().map(|n| n as u32)).collect()).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let tags = v
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| Some(Tag { id: num(t, "id")? as u32, name: s(t, "name"), sort: num(t, "sort").unwrap_or(0.0) as i64 }))
                .collect()
        })
        .unwrap_or_default();
    let course_v = v.get("course");
    let course = course_v
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|n| Some(CoursePoint { name: s(n, "name"), lat: num(n, "lat")?, lon: num(n, "lon")? })).collect())
        .unwrap_or_default();
    Ok(Setup {
        title: s(&v, "title"),
        start: num(&v, "start").map(|t| if t > 1e11 { (t / 1000.0) as i64 } else { t as i64 }).filter(|&t| t > 0),
        teams,
        tags,
        course,
        course_nm: course_v.and_then(|c| num(c, "distance")),
    })
}

fn colour(c: &str) -> String {
    let c = c.trim().trim_start_matches('#');
    if c.len() == 6 && c.chars().all(|x| x.is_ascii_hexdigit()) {
        format!("#{}", c.to_ascii_lowercase())
    } else {
        String::new()
    }
}

/// Read an Expedition position file: `id,lat,lon,YYMMDDHHMM` per line (UTC).
/// Lines that don't parse are skipped.
pub fn parse_expedition(text: &str) -> Vec<(u32, Fix)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim().split(',').map(str::trim).collect();
            if f.len() < 4 {
                return None;
            }
            let id: u32 = f[0].parse().ok()?;
            let lat: f64 = f[1].parse().ok()?;
            let lon: f64 = f[2].parse().ok()?;
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=360.0).contains(&lon) {
                return None;
            }
            let t = yymmddhhmm(f[3])?;
            Some((id, Fix { t, lat, lon: if lon > 180.0 { lon - 360.0 } else { lon }, dtf: None }))
        })
        .collect()
}

/// Read YB's AllPositions3 history.
///
/// Layout (big-endian): flags u8 (1 altitude, 2 DTF, 4 laps, 8 percentage),
/// base time u32, then per boat: id u16, count u16 and that many moments,
/// newest first. A moment whose first byte has the top bit clear is absolute:
/// time u32 (from base), lat i32, lon i32 (1e-5 degrees), [alt i16],
/// [dtf i32 metres, [lap u8]], [pc i32]. One with the top bit set is a delta
/// from the moment before it: seconds earlier u16 (top bit masked), dlat i16,
/// dlon i16, [alt i16], [ddtf i16, [lap u8]], [pc i16].
pub fn parse_all_positions(b: &[u8]) -> Result<BTreeMap<u32, Vec<Fix>>> {
    let bad = |what: &str| Error::Format(format!("YB history file: {what}"));
    if b.len() < 5 {
        return Err(bad("too short"));
    }
    let flags = b[0];
    let (alt, dtf, laps, pc) = (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0, flags & 8 != 0);
    let u16_ = |o: usize| b.get(o..o + 2).map(|x| u16::from_be_bytes([x[0], x[1]]));
    let i16_ = |o: usize| u16_(o).map(|v| v as i16);
    let u32_ = |o: usize| b.get(o..o + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    let i32_ = |o: usize| u32_(o).map(|v| v as i32);
    let base = u32_(1).unwrap() as i64;
    let mut o = 5;
    let mut out = BTreeMap::new();
    while o < b.len() {
        let id = u16_(o).ok_or_else(|| bad("cut short"))? as u32;
        let n = u16_(o + 2).ok_or_else(|| bad("cut short"))? as usize;
        o += 4;
        let mut v: Vec<Fix> = Vec::with_capacity(n);
        // working values in file units: lat/lon 1e-5 degrees, dtf metres
        let (mut t, mut la, mut lo, mut d) = (0i64, 0i64, 0i64, 0i64);
        for i in 0..n {
            let first = *b.get(o).ok_or_else(|| bad("cut short"))?;
            if first & 0x80 != 0 {
                if i == 0 {
                    return Err(bad("first moment is a delta"));
                }
                let w = (u16_(o).ok_or_else(|| bad("cut short"))? & 0x7fff) as i64;
                la += i16_(o + 2).ok_or_else(|| bad("cut short"))? as i64;
                lo += i16_(o + 4).ok_or_else(|| bad("cut short"))? as i64;
                o += 6;
                if alt { o += 2; }
                if dtf {
                    d += i16_(o).ok_or_else(|| bad("cut short"))? as i64;
                    o += 2;
                    if laps { o += 1; }
                }
                if pc { o += 2; }
                t -= w;
            } else {
                t = base + u32_(o).ok_or_else(|| bad("cut short"))? as i64;
                la = i32_(o + 4).ok_or_else(|| bad("cut short"))? as i64;
                lo = i32_(o + 8).ok_or_else(|| bad("cut short"))? as i64;
                o += 12;
                if alt { o += 2; }
                if dtf {
                    d = i32_(o).ok_or_else(|| bad("cut short"))? as i64;
                    o += 4;
                    if laps { o += 1; }
                }
                if pc { o += 4; }
            }
            let (lat, lon) = (la as f64 / 1e5, lo as f64 / 1e5);
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                return Err(bad("position out of range (format not as expected)"));
            }
            v.push(Fix { t, lat, lon, dtf: dtf.then_some(d as f64 / 1852.0) });
        }
        if o > b.len() {
            return Err(bad("cut short"));
        }
        v.sort_by_key(|f| f.t);
        out.insert(id, v);
    }
    Ok(out)
}

/// "2002280511" → 2020-02-28 05:11 UTC as unix seconds.
fn yymmddhhmm(s: &str) -> Option<i64> {
    if s.len() != 10 || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n = |a: usize| s[a..a + 2].parse::<u32>().ok();
    let d = chrono::NaiveDate::from_ymd_opt(2000 + n(0)? as i32, n(2)?, n(4)?)?;
    let dt = d.and_hms_opt(n(6)?, n(8)?, 0)?;
    Some(dt.and_utc().timestamp())
}

// ---------- the race store ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Meta {
    pub setup_fetched: Option<i64>,
    pub positions_fetched: Option<i64>,
    #[serde(default)]
    pub history_fetched: Option<i64>,
}

pub struct Race {
    pub key: String,
    dir: PathBuf,
    pub fixes: BTreeMap<u32, Vec<Fix>>,
    pub meta: Meta,
}

/// Race keys are YB's short names, e.g. "ggr2026" or "rolexs2h2025".
pub fn clean_key(k: &str) -> Result<String> {
    let k = k.trim().trim_start_matches("https://").trim_start_matches("yb.tl/").trim_matches('/').to_ascii_lowercase();
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(Error::Format(format!("“{k}” is not a YB race name (letters and digits, e.g. ggr2026)")));
    }
    Ok(k)
}

impl Race {
    pub fn open(root: &Path, key: &str) -> Result<Self> {
        let key = clean_key(key)?;
        let dir = root.join(&key);
        std::fs::create_dir_all(&dir)?;
        let read = |f: &str| std::fs::read(dir.join(f)).ok();
        let fixes: BTreeMap<String, Vec<Vec<f64>>> = read("fixes.json").and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let fixes = fixes
            .into_iter()
            .filter_map(|(k, v)| {
                Some((
                    k.parse().ok()?,
                    v.into_iter()
                        .filter(|a| a.len() >= 3)
                        .map(|a| Fix { t: a[0] as i64, lat: a[1], lon: a[2], dtf: a.get(3).copied().filter(|d| *d >= 0.0) })
                        .collect(),
                ))
            })
            .collect();
        let meta = read("meta.json").and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Ok(Race { key, dir, fixes, meta })
    }

    /// The cached setup, if any.
    pub fn setup(&self) -> Option<Setup> {
        std::fs::read_to_string(self.dir.join("setup.json")).ok().and_then(|t| parse_setup(&t).ok())
    }

    pub fn save_setup_text(&mut self, text: &str, now: i64) -> Result<Setup> {
        let s = parse_setup(text)?;
        write_atomic(&self.dir.join("setup.json"), text.as_bytes())?;
        self.meta.setup_fetched = Some(now);
        self.save_meta()?;
        Ok(s)
    }

    /// Add fixes; returns how many were new. Fixes of one boat less than a
    /// minute apart are the same report (the Expedition file rounds times to
    /// the minute, the history file doesn't); the one with YB's DTF wins.
    pub fn merge(&mut self, new: &[(u32, Fix)], now: i64) -> Result<usize> {
        let mut added = 0;
        for &(id, f) in new {
            let v = self.fixes.entry(id).or_default();
            let i = v.partition_point(|x| x.t < f.t - 59);
            match v.get(i).filter(|x| (x.t - f.t).abs() < 60) {
                Some(old) => {
                    if f.dtf.is_some() || old.dtf.is_none() {
                        v[i] = f; // a corrected or fuller report replaces the old one
                    }
                }
                None => {
                    v.insert(i, f);
                    added += 1;
                }
            }
        }
        self.meta.positions_fetched = Some(now);
        let out: BTreeMap<String, Vec<[f64; 4]>> = self
            .fixes
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|f| [f.t as f64, f.lat, f.lon, f.dtf.unwrap_or(-1.0)]).collect()))
            .collect();
        write_atomic(&self.dir.join("fixes.json"), &serde_json::to_vec(&out)?)?;
        self.save_meta()?;
        Ok(added)
    }

    /// Fixes newer than `since` (unix s), per boat.
    pub fn fixes_since(&self, since: i64) -> BTreeMap<u32, Vec<Fix>> {
        self.fixes
            .iter()
            .map(|(k, v)| (*k, v.iter().filter(|f| f.t > since).copied().collect::<Vec<_>>()))
            .filter(|(_, v)| !v.is_empty())
            .collect()
    }

    /// Merge a whole history download.
    pub fn merge_history(&mut self, h: &BTreeMap<u32, Vec<Fix>>, now: i64) -> Result<usize> {
        let all: Vec<(u32, Fix)> = h.iter().flat_map(|(id, v)| v.iter().map(move |f| (*id, *f))).collect();
        self.meta.history_fetched = Some(now);
        self.merge(&all, now)
    }

    fn save_meta(&self) -> Result<()> {
        write_atomic(&self.dir.join("meta.json"), &serde_json::to_vec(&self.meta)?)
    }
}

fn write_atomic(p: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

// ---------- downloading ----------

fn get(url: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("TweedPlot/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(40))
        .build()
        .map_err(|e| Error::Http(e.to_string()))?;
    let r = client.get(url).send().map_err(|e| Error::Http(format!("{url}: {e}")))?;
    let status = r.status();
    if !status.is_success() {
        return Err(Error::Http(format!("{url}: HTTP {}", status.as_u16())));
    }
    r.text().map_err(|e| Error::Http(e.to_string()))
}

pub fn fetch_setup(key: &str) -> Result<String> {
    get(&SETUP_URL.replace("{race}", key))
}

fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("TweedPlot/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| Error::Http(e.to_string()))?;
    let r = client.get(url).send().map_err(|e| Error::Http(format!("{url}: {e}")))?;
    let status = r.status();
    if !status.is_success() {
        return Err(Error::Http(format!("{url}: HTTP {}", status.as_u16())));
    }
    Ok(r.bytes().map_err(|e| Error::Http(e.to_string()))?.to_vec())
}

/// Every published fix of every boat.
pub fn fetch_history(key: &str) -> Result<BTreeMap<u32, Vec<Fix>>> {
    parse_all_positions(&get_bytes(&HISTORY_URL.replace("{race}", key))?)
}

pub fn fetch_positions(key: &str) -> Result<Vec<(u32, Fix)>> {
    let text = get(&POSITIONS_URL.replace("{race}", key))?;
    let fixes = parse_expedition(&text);
    if fixes.is_empty() && !text.trim().is_empty() {
        let head: String = text.chars().take(80).collect();
        return Err(Error::Format(format!("the position file for “{key}” was not understood (starts “{head}”)")));
    }
    Ok(fixes)
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPEDITION: &str = "1,17.01112,-61.77076,2002280511\n2,17.01113,-61.77075,2002270751\n\nbad line\n3,17.37460,-61.65150,2002281545\n";

    #[test]
    fn reads_expedition_positions() {
        let f = parse_expedition(EXPEDITION);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].0, 1);
        assert_eq!(f[0].1.lat, 17.01112);
        // 2020-02-28 05:11 UTC
        assert_eq!(f[0].1.t, 1582866660);
    }

    #[test]
    fn reads_race_setup() {
        let t = r##"{"title":"WARC 2024","start":1715904000000,
          "teams":[{"id":25,"name":"Fatjax","sail":"GBR63X","model":"Shipman 63","colour":"FF0000","owner":"I & J Kirkpatrick",
                    "country":"United Kingdom","status":"RACING","tags":[69933,2]},
                   {"id":"26","name":"Second","tags":[]}],
          "tags":[{"id":69933,"name":"All Boats","sort":1}],
          "course":{"nodes":[{"name":"Grenada","lon":-61.758,"lat":12.044},{"lon":-61.77,"lat":12.1}],"distance":244.04}}"##;
        let s = parse_setup(t).unwrap();
        assert_eq!(s.title, "WARC 2024");
        assert_eq!(s.start, Some(1715904000));
        assert_eq!(s.teams.len(), 2);
        assert_eq!(s.teams[0].colour, "#ff0000");
        assert_eq!(s.teams[0].skipper, "I & J Kirkpatrick");
        assert_eq!(s.teams[1].id, 26);
        assert_eq!(s.tags[0].name, "All Boats");
        assert_eq!(s.course.len(), 2);
        assert_eq!(s.course[0].name, "Grenada");
        assert_eq!(s.course_nm, Some(244.04));
    }

    /// Two boats cut from the GGR 2026 history file (17 Oct 2026): Knox-Johnston
    /// (id 957, 7 fixes) and Moitessier (id 985, 30 fixes), the 1968 ghosts.
    const GGR_HISTORY_B64: &str = include_str!("../tests/ggr2026_history_sample.b64");

    #[test]
    fn reads_yb_history() {
        let bytes = b64(GGR_HISTORY_B64.trim());
        assert_eq!(bytes.len(), 605);
        let h = parse_all_positions(&bytes).unwrap();
        assert_eq!(h.keys().copied().collect::<Vec<_>>(), vec![957, 985]);
        assert_eq!(h[&957].len(), 7);
        assert_eq!(h[&985].len(), 30);
        for v in h.values() {
            assert!(v.windows(2).all(|w| w[0].t < w[1].t), "oldest first");
            assert!(v.iter().all(|f| f.dtf.is_some()));
            // all inside the race's plausible window (2026) and on Earth
            assert!(v.iter().all(|f| f.t > 1_780_000_000 && f.t < 1_830_000_000));
        }
        // Moitessier's ghost starts from Plymouth, as in 1968
        let first = h[&985][0];
        assert!((first.lat - 50.35).abs() < 0.05 && (first.lon + 4.14).abs() < 0.05, "{first:?}");
    }

    #[test]
    fn history_and_expedition_fixes_merge() {
        let dir = std::env::temp_dir().join(format!("racenav-trk-m-{}", std::process::id()));
        let mut r = Race::open(&dir, "test2").unwrap();
        let h = vec![(5, Fix { t: 1_792_094_429, lat: -5.7, lon: -27.2, dtf: Some(23000.0) })];
        assert_eq!(r.merge(&h, 1).unwrap(), 1);
        // the same report from the Expedition file, rounded to the minute: not a new fix, DTF kept
        let e = vec![(5, Fix { t: 1_792_094_400, lat: -5.7, lon: -27.2, dtf: None })];
        assert_eq!(r.merge(&e, 2).unwrap(), 0);
        assert_eq!(r.fixes[&5][0].dtf, Some(23000.0));
        let r2 = Race::open(&dir, "test2").unwrap();
        assert_eq!(r2.fixes[&5][0].dtf, Some(23000.0));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn b64(s: &str) -> Vec<u8> {
        let val = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        let mut out = Vec::new();
        for ch in s.as_bytes().chunks(4) {
            let n = ch.iter().filter(|&&c| c != b'=').count();
            let v: Vec<u32> = ch.iter().map(|&c| if c == b'=' { 0 } else { val(c) as u32 }).collect();
            let w = (v[0] << 18) | (v[1] << 12) | (v.get(2).copied().unwrap_or(0) << 6) | v.get(3).copied().unwrap_or(0);
            out.extend_from_slice(&[(w >> 16) as u8, (w >> 8) as u8, w as u8][..n - 1]);
        }
        out
    }

    #[test]
    fn keys_are_cleaned() {
        assert_eq!(clean_key(" https://yb.tl/GGR2026/ ").unwrap(), "ggr2026");
        assert!(clean_key("a b").is_err());
    }

    #[test]
    fn merges_and_keeps_fixes_on_disk() {
        let dir = std::env::temp_dir().join(format!("racenav-trk-{}", std::process::id()));
        let mut r = Race::open(&dir, "test1").unwrap();
        let f = parse_expedition(EXPEDITION);
        assert_eq!(r.merge(&f, 100).unwrap(), 3);
        assert_eq!(r.merge(&f, 200).unwrap(), 0); // same fixes again
        let later = vec![(1, Fix { t: f[0].1.t + 3600, lat: 17.1, lon: -61.7, dtf: None })];
        assert_eq!(r.merge(&later, 300).unwrap(), 1);
        let r2 = Race::open(&dir, "test1").unwrap();
        assert_eq!(r2.fixes[&1].len(), 2);
        assert_eq!(r2.meta.positions_fetched, Some(300));
        assert_eq!(r2.fixes_since(f[0].1.t).get(&1).map(|v| v.len()), Some(1));
        let _ = std::fs::remove_dir_all(dir);
    }
}
