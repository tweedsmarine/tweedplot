//! GRIB weather data for racenav.
//!
//! Reads GRIB2 files (Meltemus, ECMWF, GFS, ICON and others), keeps every
//! field on a canonical grid (rows south to north, columns west to east), and
//! answers point and grid queries interpolated in space (bilinear) and time
//! (linear). Times are Unix seconds UTC. Outside a file's time span a query
//! returns `None`: a truncated GRIB must never look like calm weather.

mod decode;
pub mod compare;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use decode::{unix, FieldMeta};

#[derive(Debug, thiserror::Error)]
pub enum GribError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad GRIB: {0}")]
    Format(String),
    #[error("not supported: {0}")]
    Unsupported(String),
}

/// The quantities the app understands. Everything else is kept as `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
pub enum Param {
    /// 10 m wind, eastward component (m/s).
    WindU,
    /// 10 m wind, northward component (m/s).
    WindV,
    /// Wind gust (m/s).
    Gust,
    /// 10 m wind speed (m/s), when a file gives speed and direction (UKMO via Meltemus).
    WindSpeed,
    /// 10 m wind direction, degrees, the direction it blows FROM.
    WindDir,
    /// Mean sea-level pressure (Pa).
    Mslp,
    /// Significant height of combined wind waves and swell (m).
    WaveHeight,
    /// Peak (primary) wave period (s) and direction waves come FROM (deg).
    WavePeriod,
    WaveDir,
    /// Significant height of swell and of wind waves (m).
    SwellHeight,
    WindWaveHeight,
    /// Precipitation rate (kg m-2 s-1 = mm/s).
    Rain,
    /// Convective available potential energy (J/kg).
    Cape,
    /// Ocean current, eastward (m/s).
    CurrentU,
    /// Ocean current, northward (m/s).
    CurrentV,
    Other(u8, u8, u8),
}

impl Param {
    pub fn from_codes(discipline: u8, category: u8, number: u8, surface: u8) -> Self {
        match (discipline, category, number) {
            (0, 2, 2) if surface == 103 => Param::WindU,
            (0, 2, 3) if surface == 103 => Param::WindV,
            (0, 2, 22) => Param::Gust,
            (0, 2, 1) if surface == 103 => Param::WindSpeed,
            (0, 2, 0) if surface == 103 => Param::WindDir,
            (0, 3, 0) if surface == 101 => Param::Mslp,
            (0, 3, 1) => Param::Mslp,
            (10, 0, 3) => Param::WaveHeight,
            (10, 0, 5) => Param::WindWaveHeight,
            (10, 0, 8) => Param::SwellHeight,
            (10, 0, 10) => Param::WaveDir,
            (10, 0, 11) => Param::WavePeriod,
            (0, 1, 7) => Param::Rain,
            (0, 7, 6) => Param::Cape,
            (10, 1, 2) => Param::CurrentU,
            (10, 1, 3) => Param::CurrentV,
            (d, c, n) => Param::Other(d, c, n),
        }
    }
}

/// Regular lat/lon grid in canonical order: `values[j * ni + i]`, j = 0 the
/// southern row, i = 0 the western column.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Grid {
    pub ni: usize,
    pub nj: usize,
    /// Latitude of the southern row (deg).
    pub lat0: f64,
    /// Longitude of the western column (deg, -180..180).
    pub lon0: f64,
    pub dlat: f64,
    pub dlon: f64,
    #[serde(skip)]
    scan: u8,
}

impl Grid {
    #[allow(clippy::too_many_arguments)]
    fn from_grib(ni: usize, nj: usize, la1: f64, lo1: f64, la2: f64, _lo2: f64, di: f64, dj: f64, scan: u8) -> Self {
        let dlat = if dj > 0.0 && nj > 1 { dj } else if nj > 1 { (la2 - la1).abs() / (nj - 1) as f64 } else { 1.0 };
        let dlon = if di > 0.0 { di } else { 1.0 };
        let lat0 = la1.min(la2);
        let west = if scan & 0x80 != 0 { lo1 - (ni as f64 - 1.0) * dlon } else { lo1 };
        Self { ni, nj, lat0, lon0: nav_core::wrap_lon(west), dlat, dlon, scan }
    }

    /// A regular grid from its south-west corner and spacing (canonical order: rows south to north).
    pub fn regular(ni: usize, nj: usize, lat0: f64, lon0: f64, dlat: f64, dlon: f64) -> Self {
        Grid { ni, nj, lat0, lon0, dlat, dlon, scan: 0x40 }
    }

    fn canonical(&self) -> Grid {
        Grid { scan: 0x40, ..*self }
    }

    /// Reorder raw GRIB values into canonical order.
    fn normalise(&self, raw: Vec<f32>) -> Vec<f32> {
        let (ni, nj, s) = (self.ni, self.nj, self.scan);
        if s & 0xE0 == 0x40 {
            return raw; // already south-to-north, west-to-east, row by row
        }
        let mut out = vec![f32::NAN; ni * nj];
        for (k, v) in raw.into_iter().enumerate().take(ni * nj) {
            let (i, j) = if s & 0x20 != 0 { (k / nj, k % nj) } else { (k % ni, k / ni) };
            let ci = if s & 0x80 != 0 { ni - 1 - i } else { i };
            let cj = if s & 0x40 != 0 { j } else { nj - 1 - j };
            out[cj * ni + ci] = v;
        }
        out
    }

    fn is_global(&self) -> bool {
        (self.ni as f64 * self.dlon - 360.0).abs() < self.dlon * 0.5
    }

    /// [west, south, east, north].
    pub fn bbox(&self) -> [f64; 4] {
        [
            self.lon0,
            self.lat0,
            self.lon0 + (self.ni as f64 - 1.0) * self.dlon,
            self.lat0 + (self.nj as f64 - 1.0) * self.dlat,
        ]
    }

    /// Fractional grid index of a position, if it is inside the grid.
    fn index(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        let fy = (lat - self.lat0) / self.dlat;
        let mut dl = lon - self.lon0;
        dl = dl.rem_euclid(360.0);
        let fx = dl / self.dlon;
        let eps = 1e-9;
        let max_x = if self.is_global() { self.ni as f64 } else { self.ni as f64 - 1.0 };
        (fy >= -eps && fy <= self.nj as f64 - 1.0 + eps && fx <= max_x + eps).then_some((fx, fy))
    }

    /// Bilinear sample; near missing points it falls back to the nearest valid corner.
    pub fn sample(&self, values: &[f32], lat: f64, lon: f64) -> Option<f64> {
        let (fx, fy) = self.index(lat, lon)?;
        let i0 = fx.floor() as usize;
        let j0 = fy.floor() as usize;
        let (tx, ty) = (fx - i0 as f64, fy - j0 as f64);
        let wrap = |i: usize| if self.is_global() { i % self.ni } else { i.min(self.ni - 1) };
        let (i0, i1) = (wrap(i0), wrap(i0 + 1));
        let (j0c, j1) = (j0.min(self.nj - 1), (j0 + 1).min(self.nj - 1));
        let v = |i: usize, j: usize| values[j * self.ni + i] as f64;
        let c = [v(i0, j0c), v(i1, j0c), v(i0, j1), v(i1, j1)];
        if c.iter().all(|x| x.is_finite()) {
            let a = c[0] + (c[1] - c[0]) * tx;
            let b = c[2] + (c[3] - c[2]) * tx;
            return Some(a + (b - a) * ty);
        }
        let near = match (tx < 0.5, ty < 0.5) {
            (true, true) => c[0],
            (false, true) => c[1],
            (true, false) => c[2],
            (false, false) => c[3],
        };
        near.is_finite().then_some(near)
    }
}

/// One decoded GRIB field.
#[derive(Debug, Clone)]
pub struct Field {
    pub param: Param,
    pub discipline: u8,
    pub category: u8,
    pub number: u8,
    pub surface_type: u8,
    pub surface_value: f64,
    pub centre: u16,
    pub ref_time: i64,
    pub valid_time: i64,
    /// Ensemble member, if any.
    pub member: Option<u16>,
    pub grid: Grid,
    /// Canonical order, NaN = missing.
    pub values: Vec<f32>,
    /// Local-use section as JSON (Meltemus: model, run, cycle, resolution).
    pub local: Option<serde_json::Value>,
}

/// True wind from a GRIB: speed in knots and direction it blows FROM (deg true).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Wind {
    pub u: f64,
    pub v: f64,
    pub tws: f64,
    pub twd: f64,
}

impl Wind {
    pub fn from_uv(u: f64, v: f64) -> Self {
        let tws = (u * u + v * v).sqrt() * 3600.0 / 1852.0;
        let twd = nav_core::norm360((-u).atan2(-v).to_degrees());
        Wind { u, v, tws, twd }
    }
}

/// A loaded GRIB file.
#[derive(Debug, Clone)]
pub struct GribFile {
    pub path: PathBuf,
    pub fields: Vec<Field>,
    pub skipped: Vec<String>,
    /// Sorted field indices per parameter, built once (queries are hot in routing).
    index: std::collections::BTreeMap<Param, Vec<usize>>,
}

/// Summary for the user interface.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub file: String,
    /// Display name, e.g. "ECMWF 0p25 2026-10-11 12Z".
    pub name: String,
    pub model: String,
    pub ref_time: i64,
    pub params: Vec<ParamSummary>,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParamSummary {
    pub param: Param,
    pub times: Vec<i64>,
    pub bbox: [f64; 4],
    pub dlat: f64,
    pub dlon: f64,
}

impl GribFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GribError> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path)?;
        let d = decode::decode(&bytes);
        if d.fields.is_empty() && !d.skipped.is_empty() {
            return Err(GribError::Unsupported(d.skipped.join("; ")));
        }
        Ok(GribFile::build(path, d.fields, d.skipped))
    }

    /// As [`open`](Self::open), reporting `progress(fraction 0..1, stage)` while it reads and decodes.
    pub fn open_with_progress(path: impl AsRef<Path>, progress: &(dyn Fn(f64, &str) + Sync)) -> Result<Self, GribError> {
        let path = path.as_ref().to_path_buf();
        progress(0.0, "reading");
        let bytes = std::fs::read(&path)?;
        progress(0.05, "decoding");
        let d = decode::decode_with_progress(&bytes, &|done, total| progress(0.05 + 0.9 * done as f64 / total.max(1) as f64, "decoding"));
        if d.fields.is_empty() && !d.skipped.is_empty() {
            return Err(GribError::Unsupported(d.skipped.join("; ")));
        }
        progress(0.97, "indexing");
        Ok(GribFile::build(path, d.fields, d.skipped))
    }

    /// A file from fields already decoded (tests, and fields made by the app).
    pub fn from_fields(fields: Vec<Field>) -> Self {
        GribFile::build(PathBuf::new(), fields, Vec::new())
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let d = decode::decode(bytes);
        GribFile::build(PathBuf::new(), d.fields, d.skipped)
    }

    /// Fields of one parameter (deterministic or control member), sorted by valid
    /// time; for duplicate times the latest run wins.
    fn build(path: PathBuf, mut fields: Vec<Field>, skipped: Vec<String>) -> Self {
        fields.extend(uv_from_speed_dir(&fields));
        let mut index: std::collections::BTreeMap<Param, Vec<usize>> = std::collections::BTreeMap::new();
        for (i, f) in fields.iter().enumerate() {
            if f.member.unwrap_or(0) == 0 {
                index.entry(f.param).or_default().push(i);
            }
        }
        for v in index.values_mut() {
            v.sort_by_key(|&i| (fields[i].valid_time, std::cmp::Reverse(fields[i].ref_time)));
            v.dedup_by_key(|i| fields[*i].valid_time);
        }
        GribFile { path, fields, skipped, index }
    }

    pub fn series(&self, p: Param) -> Vec<&Field> {
        self.index.get(&p).map(|v| v.iter().map(|&i| &self.fields[i]).collect()).unwrap_or_default()
    }

    /// Bracketing fields for time `t` without allocating.
    fn bracket_at(&self, p: Param, t: i64) -> Option<(&Field, &Field, f64)> {
        let idx = self.index.get(&p)?;
        let (first, last) = (&self.fields[*idx.first()?], &self.fields[*idx.last()?]);
        if t < first.valid_time || t > last.valid_time {
            return None;
        }
        let i = idx.partition_point(|&k| self.fields[k].valid_time <= t);
        let a = &self.fields[idx[i - 1]];
        if a.valid_time == t || i == idx.len() {
            return Some((a, a, 0.0));
        }
        let b = &self.fields[idx[i]];
        Some((a, b, (t - a.valid_time) as f64 / (b.valid_time - a.valid_time) as f64))
    }

    pub fn times(&self, p: Param) -> Vec<i64> {
        self.series(p).iter().map(|f| f.valid_time).collect()
    }

    /// One parameter at a point and time, linear in time between fields.
    pub fn value(&self, p: Param, lat: f64, lon: f64, t: i64) -> Option<f64> {
        let (a, b, w) = self.bracket_at(p, t)?;
        let va = a.grid.sample(&a.values, lat, lon)?;
        if w == 0.0 {
            return Some(va);
        }
        let vb = b.grid.sample(&b.values, lat, lon)?;
        Some(va + (vb - va) * w)
    }

    pub fn wind(&self, lat: f64, lon: f64, t: i64) -> Option<Wind> {
        Some(Wind::from_uv(self.value(Param::WindU, lat, lon, t)?, self.value(Param::WindV, lat, lon, t)?))
    }

    pub fn current(&self, lat: f64, lon: f64, t: i64) -> Option<(f64, f64)> {
        Some((self.value(Param::CurrentU, lat, lon, t)?, self.value(Param::CurrentV, lat, lon, t)?))
    }

    /// A whole grid at time `t` (linear in time), for display.
    pub fn grid_at(&self, p: Param, t: i64) -> Option<(Grid, Vec<f32>)> {
        let s = self.series(p);
        let (a, b, w) = bracket(&s, t)?;
        if w == 0.0 || a.grid != b.grid {
            return Some((a.grid, a.values.clone()));
        }
        let w = w as f32;
        Some((a.grid, a.values.iter().zip(&b.values).map(|(x, y)| x + (y - x) * w).collect()))
    }

    /// Model name and run, from Meltemus' local section when present.
    pub fn model(&self) -> (String, i64) {
        let wind = self.fields.iter().find(|f| f.param == Param::WindU).or(self.fields.first());
        let Some(f) = wind else { return ("empty".into(), 0) };
        let mut name = model_name(f.local.as_ref(), f.centre);
        // NOAA wave files carry no model name: say so when there is no wind in them
        if f.centre == 7 && f.local.is_none() && f.param != Param::WindU && f.discipline == 10 {
            name = "NCEP GFS-Wave".into();
        }
        (name, f.ref_time)
    }

    pub fn summary(&self) -> Summary {
        let (model, ref_time) = self.model();
        let mut params: Vec<Param> = self.fields.iter().map(|f| f.param).collect();
        params.sort();
        params.dedup();
        let params = params
            .into_iter()
            .filter_map(|p| {
                let s = self.series(p);
                let g = s.first()?.grid;
                Some(ParamSummary { param: p, times: s.iter().map(|f| f.valid_time).collect(), bbox: g.bbox(), dlat: g.dlat, dlon: g.dlon })
            })
            .collect();
        Summary {
            file: self.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            name: format!("{model} {}", fmt_short(ref_time)),
            model,
            ref_time,
            params,
            skipped: self.skipped.len(),
        }
    }
}

/// Some files (UKMO through Meltemus) give wind as speed and direction rather
/// than U/V components. Build U/V fields from them so everything downstream
/// (barbs, routing, comparisons) works the same.
fn uv_from_speed_dir(fields: &[Field]) -> Vec<Field> {
    if fields.iter().any(|f| f.param == Param::WindU) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for sp in fields.iter().filter(|f| f.param == Param::WindSpeed) {
        let Some(dir) = fields.iter().find(|f| {
            f.param == Param::WindDir && f.valid_time == sp.valid_time && f.ref_time == sp.ref_time && f.member == sp.member && f.grid == sp.grid
        }) else {
            continue;
        };
        let (mut u, mut v) = (Vec::with_capacity(sp.values.len()), Vec::with_capacity(sp.values.len()));
        for (&ws, &wd) in sp.values.iter().zip(&dir.values) {
            let r = (wd as f64).to_radians();
            // direction is where the wind comes FROM, so the air moves the other way
            u.push((-(ws as f64) * r.sin()) as f32);
            v.push((-(ws as f64) * r.cos()) as f32);
        }
        out.push(Field { param: Param::WindU, number: 2, values: u, ..sp.clone() });
        out.push(Field { param: Param::WindV, number: 3, values: v, ..sp.clone() });
    }
    out
}

/// One model run inside a file, from a header scan.
#[derive(Debug, Clone, Serialize)]
pub struct RunInfo {
    /// e.g. "ECMWF 0p25"; from Meltemus' local section or the issuing centre.
    pub model: String,
    pub ref_time: i64,
    /// "wind", "gust", "pressure", "waves", "currents"...
    pub contents: Vec<String>,
    pub first: i64,
    pub last: i64,
    /// [west, south, east, north]
    pub bbox: [f64; 4],
    pub dlat: f64,
    pub dlon: f64,
    /// Number of ensemble members (0 = deterministic).
    pub members: usize,
}

/// What a file holds, without decoding the data (fast, for the file chooser).
#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    /// Runs in the file, the wind run first.
    pub runs: Vec<RunInfo>,
    /// Short description: "ECMWF 0p25 · run 11 Oct 12Z · 37.3–34.5°S 173.8–176.0°E · to 26 Oct 12Z · +waves +currents".
    pub label: String,
}

pub fn scan(path: impl AsRef<Path>) -> Result<FileInfo, GribError> {
    let metas = decode::scan_file(path.as_ref())?;
    let mut runs: Vec<(String, i64, Vec<&FieldMeta>)> = Vec::new();
    for m in &metas {
        let name = model_name(m.local.as_ref(), m.centre);
        match runs.iter_mut().find(|r| r.0 == name && r.1 == m.ref_time) {
            Some(r) => r.2.push(m),
            None => runs.push((name, m.ref_time, vec![m])),
        }
    }
    let mut infos: Vec<RunInfo> = runs
        .into_iter()
        .map(|(model, ref_time, fs)| {
            let mut contents: Vec<String> = Vec::new();
            for f in &fs {
                let c = match f.param {
                    Param::WindU | Param::WindV | Param::WindSpeed | Param::WindDir => "wind",
                    Param::Gust => "gust",
                    Param::Mslp => "pressure",
                    Param::WaveHeight | Param::WavePeriod | Param::WaveDir | Param::SwellHeight | Param::WindWaveHeight => "waves",
                    Param::Rain => "rain",
                    Param::Cape => "CAPE",
                    Param::CurrentU | Param::CurrentV => "currents",
                    Param::Other(..) => "other",
                };
                if !contents.iter().any(|x| x == c) {
                    contents.push(c.to_string());
                }
            }
            let g = fs[0].grid;
            let mut members: Vec<u16> = fs.iter().filter_map(|f| f.member).collect();
            members.sort();
            members.dedup();
            RunInfo {
                model,
                ref_time,
                contents,
                first: fs.iter().map(|f| f.valid_time).min().unwrap_or(0),
                last: fs.iter().map(|f| f.valid_time).max().unwrap_or(0),
                bbox: g.bbox(),
                dlat: g.dlat,
                dlon: g.dlon,
                members: if members.len() > 1 { members.len() } else { 0 },
            }
        })
        .collect();
    infos.sort_by_key(|r| !r.contents.iter().any(|c| c == "wind"));
    let label = match infos.first() {
        None => "no readable fields".to_string(),
        Some(r) => {
            let mut extras: Vec<String> = Vec::new();
            for c in infos.iter().flat_map(|o| o.contents.iter()) {
                if (c == "waves" || c == "currents") && !extras.contains(c) {
                    extras.push(c.clone());
                }
            }
            let mut s = format!(
                "{} · run {} · {} · to {}",
                r.model,
                fmt_short(r.ref_time),
                fmt_area(&r.bbox),
                fmt_short(r.last)
            );
            if r.members > 0 {
                s.push_str(&format!(" · {} members", r.members));
            }
            for e in extras {
                s.push_str(&format!(" +{e}"));
            }
            s
        }
    };
    Ok(FileInfo { runs: infos, label })
}

fn model_name(local: Option<&serde_json::Value>, centre: u16) -> String {
    local
        .and_then(|l| {
            let disp = l.get("dispName").or(l.get("model"))?.as_str()?;
            let res = l.get("res").and_then(|r| r.as_str()).unwrap_or("");
            Some(if res.is_empty() { disp.to_string() } else { format!("{disp} {res}") })
        })
        .unwrap_or_else(|| centre_name(centre).to_string())
}

/// "11 Oct 12Z"
pub fn fmt_short(t: i64) -> String {
    let full = fmt_run(t); // "2026-10-11 12Z"
    let m: usize = full[5..7].parse().unwrap_or(1);
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][m.clamp(1, 12) - 1];
    let d: u32 = full[8..10].parse().unwrap_or(0);
    format!("{d} {mon} {}", &full[11..])
}

/// "37.3–34.5°S 173.8–176.0°E"
pub fn fmt_area(b: &[f64; 4]) -> String {
    // round half away from zero (37.25 -> 37.3)
    let r = |x: f64| (x * 10.0).round() / 10.0;
    let b = &[r(b[0]), r(b[1]), r(b[2]), r(b[3])];
    let lat = |a: f64, c: f64| {
        let (h, x, y) = if a <= 0.0 && c <= 0.0 { ("S", -a, -c) } else if a >= 0.0 { ("N", a, c) } else { ("", a, c) };
        if h.is_empty() { format!("{x:.1}–{y:.1}°") } else { format!("{:.1}–{:.1}°{h}", x.max(y), x.min(y)) }
    };
    let lon = |a: f64, c: f64| {
        if a >= 0.0 && c >= 0.0 {
            format!("{a:.1}–{c:.1}°E")
        } else if a <= 0.0 && c <= 0.0 {
            format!("{:.1}–{:.1}°W", -a, -c)
        } else {
            format!("{:.1}°W–{c:.1}°E", -a)
        }
    };
    let east = nav_core::wrap_lon(b[2]);
    let west = nav_core::wrap_lon(b[0]);
    format!("{} {}", lat(b[1], b[3]), lon(west, east))
}

fn bracket<'a>(s: &[&'a Field], t: i64) -> Option<(&'a Field, &'a Field, f64)> {
    let first = s.first()?;
    let last = s.last()?;
    if t < first.valid_time || t > last.valid_time {
        return None;
    }
    let i = s.partition_point(|f| f.valid_time <= t);
    let a = s[i - 1];
    if a.valid_time == t || i == s.len() {
        return Some((a, a, 0.0));
    }
    let b = s[i];
    Some((a, b, (t - a.valid_time) as f64 / (b.valid_time - a.valid_time) as f64))
}

fn centre_name(c: u16) -> &'static str {
    match c {
        7 => "NCEP GFS",
        74 => "UK Met Office",
        78 => "DWD",
        85 => "Météo-France",
        98 => "ECMWF",
        _ => "GRIB",
    }
}

/// "2026-10-11 12Z"
pub fn fmt_run(t: i64) -> String {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil_from_days
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}Z", secs / 3600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_direction_is_from() {
        // southerly: air moving north (v > 0) comes FROM 180
        let w = Wind::from_uv(0.0, 5.0);
        assert!((w.twd - 180.0).abs() < 1e-9);
        // westerly: air moving east (u > 0) comes FROM 270
        let w = Wind::from_uv(5.0, 0.0);
        assert!((w.twd - 270.0).abs() < 1e-9);
        assert!((w.tws - 9.719).abs() < 1e-3);
    }

    #[test]
    fn speed_and_direction_become_uv() {
        let g = Grid::from_grib(1, 1, -40.0, 170.0, -40.0, 170.0, 1.0, 1.0, 0x40).canonical();
        let mk = |param, number, v: f32| Field {
            param, discipline: 0, category: 2, number, surface_type: 103, surface_value: 10.0, centre: 0,
            ref_time: 0, valid_time: 0, member: None, grid: g, values: vec![v], local: None,
        };
        // 10 m/s from 270 (a westerly)
        let f = GribFile::build(PathBuf::new(), vec![mk(Param::WindSpeed, 1, 10.0), mk(Param::WindDir, 0, 270.0)], vec![]);
        let w = f.wind(-40.0, 170.0, 0).unwrap();
        assert!((w.twd - 270.0).abs() < 1e-4, "{w:?}");
        assert!((w.tws - 10.0 * 3600.0 / 1852.0).abs() < 1e-4);
        assert!(w.u > 9.99);
    }

    #[test]
    fn run_format() {
        assert_eq!(fmt_run(unix(2026, 10, 11, 12, 0, 0)), "2026-10-11 12Z");
    }

    #[test]
    fn grid_scan_north_to_south() {
        // 2 x 2 grid, first row north (scan 0x00), values 1 2 / 3 4
        let g = Grid::from_grib(2, 2, -40.0, 170.0, -41.0, 171.0, 1.0, 1.0, 0x00);
        let v = g.normalise(vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(v, vec![3.0, 4.0, 1.0, 2.0]);
        let c = g.canonical();
        assert_eq!(c.lat0, -41.0);
        assert_eq!(c.sample(&v, -40.0, 171.0), Some(2.0));
        assert_eq!(c.sample(&v, -40.5, 170.5), Some(2.5));
        assert_eq!(c.sample(&v, -39.0, 170.5), None);
    }
}
