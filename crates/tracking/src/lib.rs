//! Race tracking from YB (YellowBrick).
//!
//! Two feeds per race, both public and used by navigation software:
//! - the race setup, `https://cf.yb.tl/JSON/<race>/RaceSetup`: boats (id, name,
//!   sail number, model, colour, classes), classes ("tags") and the course;
//! - the light Expedition position file, `https://yb.tl/<race>-expedition.txt`:
//!   one line per boat with its latest fix, `id,lat,lon,YYMMDDHHMM` (UTC).
//!
//! The position file holds only the latest fix, so tracks build up as the app
//! polls it; every fix is kept on disk per race:
//!
//! ```text
//! <dir>/<race>/setup.json     the race setup as YB sent it
//! <dir>/<race>/fixes.json     { "<boat id>": [[unix s, lat, lon], …] }
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
            Some((id, Fix { t, lat, lon: if lon > 180.0 { lon - 360.0 } else { lon } }))
        })
        .collect()
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
        let fixes: BTreeMap<String, Vec<[f64; 3]>> = read("fixes.json").and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let fixes = fixes
            .into_iter()
            .filter_map(|(k, v)| Some((k.parse().ok()?, v.into_iter().map(|a| Fix { t: a[0] as i64, lat: a[1], lon: a[2] }).collect())))
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

    /// Add fixes; returns how many were new (same boat and time = same fix).
    pub fn merge(&mut self, new: &[(u32, Fix)], now: i64) -> Result<usize> {
        let mut added = 0;
        for &(id, f) in new {
            let v = self.fixes.entry(id).or_default();
            match v.binary_search_by_key(&f.t, |x| x.t) {
                Ok(i) => v[i] = f, // a corrected position replaces the old one
                Err(i) => {
                    v.insert(i, f);
                    added += 1;
                }
            }
        }
        self.meta.positions_fetched = Some(now);
        let out: BTreeMap<String, Vec<[f64; 3]>> =
            self.fixes.iter().map(|(k, v)| (k.to_string(), v.iter().map(|f| [f.t as f64, f.lat, f.lon]).collect())).collect();
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
        .user_agent(concat!("racenav/", env!("CARGO_PKG_VERSION")))
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
        let later = vec![(1, Fix { t: f[0].1.t + 3600, lat: 17.1, lon: -61.7 })];
        assert_eq!(r.merge(&later, 300).unwrap(), 1);
        let r2 = Race::open(&dir, "test1").unwrap();
        assert_eq!(r2.fixes[&1].len(), 2);
        assert_eq!(r2.meta.positions_fetched, Some(300));
        assert_eq!(r2.fixes_since(f[0].1.t).get(&1).map(|v| v.len()), Some(1));
        let _ = std::fs::remove_dir_all(dir);
    }
}
