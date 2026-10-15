//! GRIB weather data for racenav.
//!
//! Reads GRIB2 files (Meltemus, ECMWF, GFS, ICON and others), keeps every
//! field on a canonical grid (rows south to north, columns west to east), and
//! answers point and grid queries interpolated in space (bilinear) and time
//! (linear). Times are Unix seconds UTC. Outside a file's time span a query
//! returns `None`: a truncated GRIB must never look like calm weather.

mod decode;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use decode::unix;

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
    /// Mean sea-level pressure (Pa).
    Mslp,
    /// Significant height of combined wind waves and swell (m).
    WaveHeight,
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
            (0, 3, 0) if surface == 101 => Param::Mslp,
            (0, 3, 1) => Param::Mslp,
            (10, 0, 3) => Param::WaveHeight,
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
        Ok(GribFile { path, fields: d.fields, skipped: d.skipped })
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let d = decode::decode(bytes);
        GribFile { path: PathBuf::new(), fields: d.fields, skipped: d.skipped }
    }

    /// Fields of one parameter (deterministic or control member), sorted by valid
    /// time; for duplicate times the latest run wins.
    pub fn series(&self, p: Param) -> Vec<&Field> {
        let mut v: Vec<&Field> = self.fields.iter().filter(|f| f.param == p && f.member.unwrap_or(0) == 0).collect();
        v.sort_by_key(|f| (f.valid_time, std::cmp::Reverse(f.ref_time)));
        v.dedup_by_key(|f| f.valid_time);
        v
    }

    pub fn times(&self, p: Param) -> Vec<i64> {
        self.series(p).iter().map(|f| f.valid_time).collect()
    }

    /// One parameter at a point and time, linear in time between fields.
    pub fn value(&self, p: Param, lat: f64, lon: f64, t: i64) -> Option<f64> {
        let s = self.series(p);
        let (a, b, w) = bracket(&s, t)?;
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
        let name = f
            .local
            .as_ref()
            .and_then(|l| {
                let disp = l.get("dispName").or(l.get("model"))?.as_str()?;
                let res = l.get("res").and_then(|r| r.as_str()).unwrap_or("");
                Some(if res.is_empty() { disp.to_string() } else { format!("{disp} {res}") })
            })
            .unwrap_or_else(|| centre_name(f.centre).to_string());
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
            name: format!("{model} {}", fmt_run(ref_time)),
            model,
            ref_time,
            params,
            skipped: self.skipped.len(),
        }
    }
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
        7 => "NCEP",
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
