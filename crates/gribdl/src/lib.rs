//! GRIB downloads from free sources, without email.
//!
//! NOAA NOMADS "grib filter" cuts each forecast file to the area and the
//! fields asked for on the server, so only a few kilobytes per step come down:
//!
//! ```text
//! https://nomads.ncep.noaa.gov/cgi-bin/filter_gfs_0p25.pl?dir=%2Fgfs.YYYYMMDD%2FHH%2Fatmos
//!     &file=gfs.tHHz.pgrb2.0p25.fFFF&var_UGRD=on&lev_10_m_above_ground=on…
//!     &subregion=&leftlon=…&rightlon=…&toplat=…&bottomlat=…
//! ```
//!
//! One request per forecast step, one at a time with a short pause (NOMADS
//! blocks clients that hammer it). The steps are appended to one GRIB2 file.
//!
//! ECMWF open data (IFS, AIFS, IFS wave) comes as global files; see `ecmwf`.

pub mod coastal;
pub mod data;
pub mod ecmwf;
pub mod ice;
pub mod marine;
pub mod ocean;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    NoRun(String),
    #[error("{0}")]
    Bad(String),
    #[error("cancelled")]
    Cancelled,
}
pub type Result<T> = std::result::Result<T, Error>;

const NOMADS: &str = "https://nomads.ncep.noaa.gov";

/// A downloadable model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Gfs0p25,
    Gfs0p50,
    Gfs1p00,
    GfsWave0p25,
    EcmwfIfs0p25,
    EcmwfIfs0p1,
    EcmwfAifs0p25,
    EcmwfWave0p25,
    EcmwfWave0p1,
    /// Ocean (see `ocean`): MUR sea surface temperature analyses, and the HYCOM ocean model.
    MurSst,
    HycomEspc,
    /// Himawari-9 hourly clear-sky sea temperature; daily chlorophyll (VIIRS).
    Himawari,
    ChlViirs,
    /// Sea ice concentration, daily (AMSR2, University of Bremen): see `ice`.
    IceAsi,
}

/// What a model offers, for the download dialog.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: Model,
    pub name: &'static str,
    pub res_deg: f64,
    /// Longest forecast, hours.
    pub max_hours: u32,
    /// (up to hour, step hours): e.g. hourly to 120 h, then 3-hourly.
    pub steps: Vec<(u32, u32)>,
    pub params: Vec<ParamInfo>,
    /// Hours after the run time before the whole run is usually on the server.
    pub delay_h: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParamInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// Fields per step (wind = 2: U and V).
    pub fields: u32,
    pub default: bool,
}

impl Model {
    pub fn all() -> [Model; 14] {
        [
            Model::Gfs0p25,
            Model::Gfs0p50,
            Model::Gfs1p00,
            Model::GfsWave0p25,
            Model::EcmwfIfs0p25,
            Model::EcmwfIfs0p1,
            Model::EcmwfAifs0p25,
            Model::EcmwfWave0p25,
            Model::EcmwfWave0p1,
            Model::MurSst,
            Model::Himawari,
            Model::HycomEspc,
            Model::ChlViirs,
            Model::IceAsi,
        ]
    }

    pub fn is_ocean(self) -> bool {
        matches!(self, Model::MurSst | Model::HycomEspc | Model::Himawari | Model::ChlViirs | Model::IceAsi)
    }

    /// The wave model whose fields can go in the same file as this model's wind.
    pub fn wave_companion(self) -> Option<Model> {
        match self {
            Model::Gfs0p25 | Model::Gfs0p50 | Model::Gfs1p00 => Some(Model::GfsWave0p25),
            Model::EcmwfIfs0p25 | Model::EcmwfAifs0p25 => Some(Model::EcmwfWave0p25),
            Model::EcmwfIfs0p1 => Some(Model::EcmwfWave0p1),
            _ => None,
        }
    }

    pub fn is_ecmwf(self) -> bool {
        matches!(self, Model::EcmwfIfs0p25 | Model::EcmwfIfs0p1 | Model::EcmwfAifs0p25 | Model::EcmwfWave0p25 | Model::EcmwfWave0p1)
    }

    pub fn info(self) -> ModelInfo {
        let atmos = || {
            vec![
                ParamInfo { id: "wind", name: "Wind at 10 m", fields: 2, default: true },
                ParamInfo { id: "gust", name: "Gusts", fields: 1, default: true },
                ParamInfo { id: "mslp", name: "Pressure (MSL)", fields: 1, default: true },
                ParamInfo { id: "rain", name: "Rain rate", fields: 1, default: false },
                ParamInfo { id: "cape", name: "CAPE (squall risk)", fields: 1, default: false },
                ParamInfo { id: "temp", name: "Air temperature (2 m)", fields: 1, default: false },
                ParamInfo { id: "waves", name: "Waves from GFS-Wave in the same file (height, period, direction)", fields: 3, default: false },
            ]
        };
        match self {
            Model::Gfs0p25 => {
                ModelInfo { id: self, name: "GFS 0.25°", res_deg: 0.25, max_hours: 384, steps: vec![(120, 1), (384, 3)], params: atmos(), delay_h: 5 }
            }
            Model::Gfs0p50 => ModelInfo { id: self, name: "GFS 0.5°", res_deg: 0.5, max_hours: 384, steps: vec![(384, 3)], params: atmos(), delay_h: 5 },
            Model::Gfs1p00 => ModelInfo { id: self, name: "GFS 1°", res_deg: 1.0, max_hours: 384, steps: vec![(384, 3)], params: atmos(), delay_h: 5 },
            Model::GfsWave0p25 => ModelInfo {
                id: self,
                name: "GFS-Wave 0.25°",
                res_deg: 0.25,
                max_hours: 384,
                steps: vec![(120, 1), (384, 3)],
                params: vec![
                    ParamInfo { id: "waves", name: "Significant wave height", fields: 1, default: true },
                    ParamInfo { id: "period", name: "Peak period and direction", fields: 2, default: true },
                    ParamInfo { id: "swell", name: "Swell and wind-wave height", fields: 2, default: false },
                ],
                delay_h: 6,
            },
            Model::EcmwfIfs0p25 => ModelInfo {
                id: self,
                name: "ECMWF IFS 0.25°",
                res_deg: 0.25,
                max_hours: 360,
                steps: vec![(144, 3), (360, 6)],
                params: vec![
                    ParamInfo { id: "wind", name: "Wind at 10 m", fields: 2, default: true },
                    ParamInfo { id: "gust", name: "Gusts (where published)", fields: 1, default: false },
                    ParamInfo { id: "mslp", name: "Pressure (MSL)", fields: 1, default: true },
                    ParamInfo { id: "temp", name: "Air temperature (2 m)", fields: 1, default: false },
                    ParamInfo { id: "waves", name: "Waves from the ECMWF wave model in the same file (height, period, direction)", fields: 3, default: false },
                ],
                delay_h: 8,
            },
            Model::EcmwfIfs0p1 => ModelInfo {
                id: self,
                name: "ECMWF IFS 0.1° (~9 km)",
                res_deg: 0.1,
                max_hours: 360,
                steps: vec![(144, 3), (360, 6)],
                params: vec![
                    ParamInfo { id: "wind", name: "Wind at 10 m", fields: 2, default: true },
                    ParamInfo { id: "gust", name: "Gusts", fields: 1, default: false },
                    ParamInfo { id: "mslp", name: "Pressure (MSL)", fields: 1, default: true },
                    ParamInfo { id: "temp", name: "Air temperature (2 m)", fields: 1, default: false },
                    ParamInfo { id: "current", name: "Surface currents", fields: 2, default: false },
                    ParamInfo { id: "waves", name: "Waves from the ECMWF wave model in the same file (height, period, direction)", fields: 3, default: false },
                ],
                delay_h: 10,
            },
            Model::EcmwfAifs0p25 => ModelInfo {
                id: self,
                name: "ECMWF AIFS 0.25° (AI model)",
                res_deg: 0.25,
                max_hours: 360,
                steps: vec![(360, 6)],
                params: vec![
                    ParamInfo { id: "wind", name: "Wind at 10 m", fields: 2, default: true },
                    ParamInfo { id: "mslp", name: "Pressure (MSL)", fields: 1, default: true },
                    ParamInfo { id: "temp", name: "Air temperature (2 m)", fields: 1, default: false },
                    ParamInfo { id: "waves", name: "Waves from the ECMWF wave model in the same file (height, period, direction)", fields: 3, default: false },
                ],
                delay_h: 7,
            },
            // MUR: `hours` / 24 = how many of the latest daily analyses
            Model::MurSst => ModelInfo {
                id: self,
                name: "MUR sea temperature 1 km (satellite analysis)",
                res_deg: 0.01,
                max_hours: 14 * 24,
                steps: vec![(14 * 24, 24)],
                params: vec![ParamInfo { id: "sst", name: "Sea surface temperature", fields: 1, default: true }],
                delay_h: 0,
            },
            // Himawari: `hours` = how many hours back from now, `every` = 1, 2, 3 or 6
            Model::Himawari => ModelInfo {
                id: self,
                name: "Himawari-9 sea temperature 2 km, hourly (satellite, clear sky)",
                res_deg: 0.02,
                max_hours: 72,
                steps: vec![(72, 1)],
                params: vec![ParamInfo { id: "skin", name: "Sea surface temperature (clear sky)", fields: 1, default: true }],
                delay_h: 0,
            },
            // chlorophyll: `hours` / 24 = how many of the latest daily analyses
            Model::ChlViirs => ModelInfo {
                id: self,
                name: "Chlorophyll-a 9 km, daily (VIIRS satellites, gap-filled)",
                res_deg: 0.0833,
                max_hours: 14 * 24,
                steps: vec![(14 * 24, 24)],
                params: vec![ParamInfo { id: "chl", name: "Chlorophyll-a", fields: 1, default: true }],
                delay_h: 0,
            },
            // sea ice: `hours` / 24 = how many of the latest daily grids
            Model::IceAsi => ModelInfo {
                id: self,
                name: "Sea ice concentration 3 km, daily (AMSR2 satellite, University of Bremen; polar seas)",
                res_deg: 0.025,
                max_hours: 14 * 24,
                steps: vec![(14 * 24, 24)],
                params: vec![ParamInfo { id: "ice", name: "Sea ice concentration", fields: 1, default: true }],
                delay_h: 0,
            },
            Model::HycomEspc => ModelInfo {
                id: self,
                name: "HYCOM ocean model 1/12° (temperature, salinity, currents, sea surface height)",
                res_deg: 0.08,
                max_hours: 192,
                // the surface fields are hourly, but the server is very slow on hourly
                // steps (minutes a day, against seconds at 3-hourly): 3 h is the finest offered
                steps: vec![(192, 3)],
                params: vec![
                    ParamInfo { id: "sst", name: "Sea surface temperature", fields: 1, default: true },
                    ParamInfo { id: "sss", name: "Sea surface salinity", fields: 1, default: true },
                    ParamInfo { id: "current", name: "Surface currents", fields: 2, default: true },
                    ParamInfo { id: "ssh", name: "Sea surface height (non-tidal analysis and forecast)", fields: 1, default: false },
                    ParamInfo { id: "depth", name: "Temperature and salinity at a depth", fields: 2, default: false },
                ],
                delay_h: 0,
            },
            Model::EcmwfWave0p25 => ModelInfo {
                id: self,
                name: "ECMWF wave 0.25°",
                res_deg: 0.25,
                max_hours: 360,
                steps: vec![(144, 3), (360, 6)],
                params: vec![
                    ParamInfo { id: "waves", name: "Significant wave height", fields: 1, default: true },
                    ParamInfo { id: "period", name: "Peak period and mean direction", fields: 2, default: true },
                ],
                delay_h: 8,
            },
            Model::EcmwfWave0p1 => ModelInfo {
                id: self,
                name: "ECMWF wave 0.1° (~9 km)",
                res_deg: 0.1,
                max_hours: 360,
                steps: vec![(144, 3), (360, 6)],
                params: vec![
                    ParamInfo { id: "waves", name: "Significant wave height", fields: 1, default: true },
                    ParamInfo { id: "period", name: "Peak period and mean direction", fields: 2, default: true },
                ],
                delay_h: 10,
            },
        }
    }

    fn script(self) -> &'static str {
        match self {
            Model::Gfs0p25 => "filter_gfs_0p25.pl",
            Model::Gfs0p50 => "filter_gfs_0p50.pl",
            Model::Gfs1p00 => "filter_gfs_1p00.pl",
            Model::GfsWave0p25 => "filter_gfswave.pl",
            _ => "",
        }
    }

    fn subdir(self, date: &str, hh: &str) -> String {
        match self {
            Model::GfsWave0p25 => format!("/gfs.{date}/{hh}/wave/gridded"),
            _ => format!("/gfs.{date}/{hh}/atmos"),
        }
    }

    fn file(self, hh: &str, f: u32) -> String {
        match self {
            Model::Gfs0p25 => format!("gfs.t{hh}z.pgrb2.0p25.f{f:03}"),
            Model::Gfs0p50 => format!("gfs.t{hh}z.pgrb2full.0p50.f{f:03}"),
            Model::Gfs1p00 => format!("gfs.t{hh}z.pgrb2.1p00.f{f:03}"),
            Model::GfsWave0p25 => format!("gfswave.t{hh}z.global.0p25.f{f:03}.grib2"),
            _ => String::new(),
        }
    }

    /// Filter variables and levels for the chosen parameters.
    fn vars(self, params: &[String]) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        let has = |p: &str| params.iter().any(|x| x == p);
        if self.is_ecmwf() {
            let info = self.info();
            return params.iter().filter_map(|p| info.params.iter().find(|x| x.id == p).map(|x| x.id)).collect();
        }
        if self == Model::GfsWave0p25 {
            if has("waves") {
                v.extend(["var_HTSGW=on"]);
            }
            if has("period") {
                v.extend(["var_PERPW=on", "var_DIRPW=on"]);
            }
            if has("swell") {
                v.extend(["var_SWELL=on", "var_WVHGT=on"]);
            }
            if !v.is_empty() {
                v.push("lev_surface=on");
                if has("swell") {
                    v.push("lev_1_in_sequence=on");
                }
            }
        } else {
            if has("wind") {
                v.extend(["var_UGRD=on", "var_VGRD=on", "lev_10_m_above_ground=on"]);
            }
            if has("gust") {
                v.extend(["var_GUST=on", "lev_surface=on"]);
            }
            if has("mslp") {
                v.extend(["var_PRMSL=on", "lev_mean_sea_level=on"]);
            }
            if has("rain") {
                v.extend(["var_PRATE=on", "lev_surface=on"]);
            }
            if has("cape") {
                v.extend(["var_CAPE=on", "lev_surface=on"]);
            }
            if has("temp") {
                v.extend(["var_TMP=on", "lev_2_m_above_ground=on"]);
            }
            v.sort();
            v.dedup();
        }
        v
    }

    /// Forecast hours from 0 to `hours`: the model's own steps (e.g. hourly to
    /// 120 h, then 3-hourly) thinned to multiples of `every` hours.
    pub fn hours(self, hours: u32, every: u32) -> Vec<u32> {
        let info = self.info();
        let hours = hours.min(info.max_hours);
        let every = every.max(1);
        let mut native = Vec::new();
        let mut h = 0;
        while h <= hours {
            native.push(h);
            h += info.steps.iter().find(|(upto, _)| h < *upto).map(|s| s.1).unwrap_or(3);
        }
        native.into_iter().filter(|h| h % every == 0).collect()
    }
}

/// A download request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub model: Model,
    /// [west, south, east, north], degrees; east may be beyond 180 to cross the date line.
    pub area: [f64; 4],
    pub params: Vec<String>,
    /// Forecast length, hours.
    pub hours: u32,
    /// Step between forecast times, hours (1, 3, 6, 12).
    pub every: u32,
    /// HYCOM: the depth (m) for the "depth" parameter.
    #[serde(default)]
    pub depth: f64,
}

/// A model run: date "YYYYMMDD" and cycle "00"/"06"/"12"/"18".
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Run {
    pub date: String,
    pub hh: String,
    /// Run time, unix seconds.
    pub t: i64,
}

impl Run {
    pub(crate) fn at(t: i64) -> Run {
        let d = chrono::DateTime::from_timestamp(t, 0).unwrap_or_default();
        Run { date: d.format("%Y%m%d").to_string(), hh: d.format("%H").to_string(), t }
    }
}

/// The filter's longitude window: 0..360 with left < right (right may pass 360).
fn lon_window(w: f64, e: f64) -> (f64, f64) {
    let l = w.rem_euclid(360.0);
    let mut r = e.rem_euclid(360.0);
    if r <= l {
        r += 360.0;
    }
    (l, r)
}

/// The filter URL for one step.
pub fn step_url(req: &Request, run: &Run, f: u32) -> String {
    let m = req.model;
    let (l, r) = lon_window(req.area[0], req.area[2]);
    let dir = m.subdir(&run.date, &run.hh).replace('/', "%2F");
    format!(
        "{NOMADS}/cgi-bin/{}?dir={dir}&file={}&{}&subregion=&leftlon={l}&rightlon={r}&toplat={}&bottomlat={}",
        m.script(),
        m.file(&run.hh, f),
        m.vars(&req.params).join("&"),
        req.area[3],
        req.area[1]
    )
}

/// Rough amount downloaded, bytes: the file for NOMADS (cut on the server);
/// for ECMWF whole global fields of about 600 kB (wave 300 kB) each.
pub fn download_bytes(req: &Request) -> u64 {
    if !req.model.is_ecmwf() {
        return estimate_bytes(req);
    }
    let info = req.model.info();
    let wave_extra = req.model.wave_companion().is_some() && req.params.iter().any(|p| p == "waves");
    let fields: u32 = info.params.iter().filter(|p| p.id != "waves" || !wave_extra).filter(|p| req.params.iter().any(|x| x == p.id)).map(|p| p.fields).sum();
    let per = match req.model {
        Model::EcmwfWave0p25 => 300_000,
        Model::EcmwfIfs0p1 => 3_000_000,
        Model::EcmwfWave0p1 => 2_000_000,
        _ => 600_000,
    };
    let wave_per = match req.model.wave_companion() {
        Some(Model::EcmwfWave0p1) => 2_000_000,
        Some(_) => 300_000,
        None => 0,
    };
    let waves = if wave_extra { 3 * wave_per * Model::EcmwfWave0p25.hours(req.hours, req.every).len() as u64 } else { 0 };
    fields as u64 * req.model.hours(req.hours, req.every).len() as u64 * per + waves
}

/// Satellite sea temperature: (source spacing in degrees, the step taken), so
/// the Get data line can say whether a file comes at full resolution.
pub fn ocean_step(req: &Request) -> Option<(f64, usize)> {
    match req.model {
        Model::MurSst => Some((0.01, ocean::mur_stride(req.area, (req.hours / 24).clamp(1, 14)))),
        Model::Himawari => Some((0.02, ocean::him_stride(req.area, (req.hours / req.every.max(1)).max(1) as usize))),
        _ => None,
    }
}

/// Rough size of the file written, bytes: grid points × fields × steps at about
/// 1.4 bytes a point after packing, plus headers.
pub fn estimate_bytes(req: &Request) -> u64 {
    let info = req.model.info();
    if req.model == Model::MurSst {
        // 0.01°, full resolution unless over the budget, one field a day
        let days = (req.hours / 24).clamp(1, 14);
        let k = ocean::mur_stride(req.area, days) as f64;
        let nx = (ocean::width(req.area) / 0.01 / k).floor() + 1.0;
        let ny = ((req.area[3] - req.area[1]).abs() / 0.01 / k).floor() + 1.0;
        return ((nx * ny * 1.6 + 200.0) * days as f64) as u64;
    }
    if req.model == Model::Himawari {
        let fields = (req.hours / req.every.max(1)).max(1);
        let k = ocean::him_stride(req.area, fields as usize) as f64;
        let nx = (ocean::width(req.area) / 0.02 / k).floor() + 1.0;
        let ny = ((req.area[3] - req.area[1]).abs() / 0.02 / k).floor() + 1.0;
        return ((nx * ny * 1.6 + 200.0) * fields as f64) as u64;
    }
    if req.model == Model::ChlViirs {
        let nx = (ocean::width(req.area) / 0.0833).floor() + 1.0;
        let ny = ((req.area[3] - req.area[1]).abs() / 0.0833).floor() + 1.0;
        return ((nx * ny * 2.0 + 200.0) * (req.hours / 24).clamp(1, 14) as f64) as u64;
    }
    if req.model == Model::IceAsi {
        // about a byte a cell (much of it open water or land, packed small), a day each
        let mid = ((req.area[1] + req.area[3]) / 2.0).to_radians().cos().max(0.05);
        let nx = ((req.area[2] - req.area[0]).abs() / (0.025 / mid)).floor().min(1600.0) + 1.0;
        let ny = ((req.area[3] - req.area[1]).abs() / 0.025).floor().min(1600.0) + 1.0;
        return ((nx * ny * 1.0 + 200.0) * (req.hours / 24).clamp(1, 14) as f64) as u64;
    }
    let (l, r) = lon_window(req.area[0], req.area[2]);
    let nx = ((r - l) / info.res_deg).floor() + 1.0;
    let ny = ((req.area[3] - req.area[1]).abs() / info.res_deg).floor() + 1.0;
    let fields: u32 = info.params.iter().filter(|p| req.params.iter().any(|x| x == p.id)).map(|p| p.fields).sum();
    let steps = req.model.hours(req.hours, req.every).len() as f64;
    ((nx * ny * 1.4 + 200.0) * fields as f64 * steps) as u64
}

pub(crate) fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("TweedPlot/", env!("CARGO_PKG_VERSION"), " (marine chart plotter; +https://tweedsmarine.nz)"))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Http(e.to_string()))
}

/// The newest run whose last wanted step is on the server, checked from the
/// newest possible cycle backwards (the index file of that step exists).
pub fn latest_run(model: Model, last_hour: u32, now: i64) -> Result<Run> {
    if model.is_ecmwf() {
        return ecmwf::latest_run(model, last_hour, now);
    }
    if model.is_ocean() {
        // the servers serve the newest data themselves
        return Ok(Run::at(now - now.rem_euclid(3600)));
    }
    let c = client()?;
    let newest = now - now.rem_euclid(6 * 3600);
    for k in 0..8 {
        let run = Run::at(newest - k * 6 * 3600);
        let idx = match model {
            Model::GfsWave0p25 => format!("{NOMADS}/pub/data/nccf/com/gfs/prod{}/{}.idx", model.subdir(&run.date, &run.hh), model.file(&run.hh, last_hour)),
            _ => format!("{NOMADS}/pub/data/nccf/com/gfs/prod{}/{}.idx", model.subdir(&run.date, &run.hh), model.file(&run.hh, last_hour)),
        };
        match c.head(&idx).send() {
            Ok(r) if r.status().is_success() => return Ok(run),
            Ok(_) => {}
            Err(e) => return Err(Error::Http(format!("NOMADS: {e}"))),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(Error::NoRun(format!("no complete {} run found on NOMADS in the last two days", model.info().name)))
}

/// Progress: steps done, steps in all, bytes so far.
pub type Progress<'a> = &'a (dyn Fn(usize, usize, u64) + Sync);

/// Download every step into `dir`; returns the file written.
pub fn download(req: &Request, run: &Run, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<PathBuf> {
    download_ex(req, run, dir, progress, cancel).map(|d| d.path)
}

/// A finished download: the file, and a note when requested data was incomplete.
#[derive(Debug, Clone, Serialize)]
pub struct Downloaded {
    pub path: PathBuf,
    pub note: Option<String>,
}

/// Download; with "waves" ticked for a wind model, also download its wave model
/// (GFS-Wave, ECMWF wave) for the same run, area and steps, and put both in one
/// file ("…+waves.grb2"). If the waves fail, the wind file is kept and `note` says why.
pub fn download_ex(req: &Request, run: &Run, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<Downloaded> {
    if req.model.is_ocean() {
        return ocean::download(req, run.t, dir, progress, cancel).map(|path| Downloaded { path, note: None });
    }
    let wave_model = req.model.wave_companion().filter(|_| req.params.iter().any(|p| p == "waves"));
    let Some(wm) = wave_model else {
        return download_one(req, run, dir, progress, cancel);
    };
    let mut atm = req.clone();
    atm.params.retain(|p| p != "waves");
    let wave = Request { model: wm, params: vec!["waves".into(), "period".into()], ..req.clone() };
    if atm.params.is_empty() {
        return download_one(&wave, run, dir, progress, cancel);
    }
    let wind = download_one(&atm, run, dir, progress, cancel)?;
    match download_one(&wave, run, dir, progress, cancel) {
        Ok(wv) => {
            let out = grib::combine::combined_path(&wind.path);
            grib::combine::write(&wind.path, &wv.path, &out).map_err(|e| Error::Bad(e.to_string()))?;
            let _ = std::fs::remove_file(&wind.path);
            let _ = std::fs::remove_file(&wv.path);
            Ok(Downloaded { path: out, note: merge_notes(wind.note, wv.note) })
        }
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Err(e) => Ok(Downloaded { path: wind.path, note: merge_notes(wind.note, Some(format!("Wind downloaded; the waves could not be: {e}"))) }),
    }
}

fn merge_notes(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (Some(note), None) | (None, Some(note)) => Some(note),
        (None, None) => None,
    }
}

fn download_one(req: &Request, run: &Run, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<Downloaded> {
    if req.model.vars(&req.params).is_empty() {
        return Err(Error::Bad("choose at least one parameter".into()));
    }
    if !(req.area[3] > req.area[1]) || !(-90.0..=90.0).contains(&req.area[1]) || !(-90.0..=90.0).contains(&req.area[3]) {
        return Err(Error::Bad("the area is not valid".into()));
    }
    if req.model.is_ecmwf() {
        let (path, note) = ecmwf::download(req, run, dir, progress, cancel)?;
        return Ok(Downloaded { path, note });
    }
    std::fs::create_dir_all(dir)?;
    let hours = req.model.hours(req.hours, req.every);
    let name = file_name(req, run, *hours.last().unwrap_or(&0));
    let path = dir.join(&name);
    let part = path.with_extension("grb2.part");
    let mut out = std::fs::File::create(&part)?;
    let c = client()?;
    let mut bytes = 0u64;
    progress(0, hours.len(), 0);
    for (i, &f) in hours.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err(Error::Cancelled);
        }
        let url = step_url(req, run, f);
        let data = get_retry(&c, &url, cancel)?;
        if data.len() < 8 || &data[..4] != b"GRIB" {
            drop(out);
            let _ = std::fs::remove_file(&part);
            let head = String::from_utf8_lossy(&data[..data.len().min(200)]).replace('\n', " ");
            return Err(Error::Http(format!("step f{f:03}: not GRIB data ({head})")));
        }
        out.write_all(&data)?;
        bytes += data.len() as u64;
        progress(i + 1, hours.len(), bytes);
        std::thread::sleep(Duration::from_millis(250)); // be polite
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&part, &path)?;
    Ok(Downloaded { path, note: None })
}

pub(crate) fn get_retry(c: &reqwest::blocking::Client, url: &str, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut wait = 2;
    for attempt in 0..4 {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match c.get(url).send() {
            Ok(r) if r.status().is_success() => return r.bytes().map(|b| b.to_vec()).map_err(|e| Error::Http(e.to_string())),
            Ok(r) if r.status().as_u16() == 404 => return Err(Error::Http(format!("not on the server yet ({url})"))),
            Ok(r) if attempt == 3 || r.status().as_u16() == 400 => {
                let code = r.status().as_u16();
                let host = url.split('/').nth(2).unwrap_or("the server").to_string();
                let body = r.text().unwrap_or_default();
                let why = body.lines().find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('<')).unwrap_or("").chars().take(200).collect::<String>();
                return Err(Error::Http(format!("{host} answered HTTP {code}{}", if why.is_empty() { String::new() } else { format!(": {why}") })));
            }
            Err(e) if attempt == 3 => return Err(Error::Http(e.to_string())),
            _ => {}
        }
        std::thread::sleep(Duration::from_secs(wait));
        wait *= 3;
    }
    unreachable!()
}

/// e.g. "GFS_0p25_20261015_12z_f000-f240_41S-34S_172E-178E.grb2"
pub(crate) fn file_name(req: &Request, run: &Run, last: u32) -> String {
    let m = match req.model {
        Model::Gfs0p25 => "GFS_0p25",
        Model::Gfs0p50 => "GFS_0p50",
        Model::Gfs1p00 => "GFS_1p00",
        Model::GfsWave0p25 => "GFSwave_0p25",
        Model::EcmwfIfs0p25 => "ECMWF_IFS_0p25",
        Model::EcmwfIfs0p1 => "ECMWF_IFS_0p1",
        Model::EcmwfAifs0p25 => "ECMWF_AIFS_0p25",
        Model::EcmwfWave0p25 => "ECMWF_wave_0p25",
        Model::EcmwfWave0p1 => "ECMWF_wave_0p1",
        Model::MurSst => "MUR_SST",
        Model::HycomEspc => "HYCOM_ESPC",
        Model::Himawari => "HIMAWARI_SST",
        Model::ChlViirs => "CHL_VIIRS",
        Model::IceAsi => "SEAICE_AMSR2",
    };
    let lat = |v: f64| format!("{}{}", v.abs().round(), if v < 0.0 { "S" } else { "N" });
    let lon = |v: f64| {
        let v = (v + 180.0).rem_euclid(360.0) - 180.0;
        format!("{}{}", v.abs().round(), if v < 0.0 { "W" } else { "E" })
    };
    format!("{m}_{}_{}z_f000-f{last:03}_{}-{}_{}-{}.grb2", run.date, run.hh, lat(req.area[1]), lat(req.area[3]), lon(req.area[0]), lon(req.area[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves_in_the_same_file() {
        assert_eq!(Model::EcmwfIfs0p25.wave_companion(), Some(Model::EcmwfWave0p25));
        assert_eq!(Model::EcmwfIfs0p1.wave_companion(), Some(Model::EcmwfWave0p1));
        assert_eq!(Model::Gfs0p50.wave_companion(), Some(Model::GfsWave0p25));
        assert_eq!(Model::EcmwfWave0p25.wave_companion(), None);
        assert!(Model::all().contains(&Model::EcmwfIfs0p1));
        assert!(Model::all().contains(&Model::EcmwfWave0p1));
        assert!(Model::EcmwfIfs0p1.info().params.iter().any(|p| p.id == "current" && p.fields == 2));
        for m in [Model::Gfs0p25, Model::EcmwfIfs0p25, Model::EcmwfIfs0p1, Model::EcmwfAifs0p25] {
            assert!(m.info().params.iter().any(|p| p.id == "waves"), "{m:?}");
        }
        // wind-model filter variables ignore the waves tick (they come from the wave model)
        let r =
            Request { model: Model::Gfs0p25, area: [172.0, -41.5, 178.5, -34.0], params: vec!["wind".into(), "waves".into()], hours: 48, every: 3, depth: 0.0 };
        assert!(!r.model.vars(&r.params).iter().any(|v| v.contains("HTSGW")));
        // the ECMWF amount counts wave fields at the wave model's size
        let e = Request { model: Model::EcmwfIfs0p25, ..r.clone() };
        let plain = Request { params: vec!["wind".into()], ..e.clone() };
        assert!(download_bytes(&e) > download_bytes(&plain));
        let hres = Request { model: Model::EcmwfIfs0p1, ..e.clone() };
        assert!(download_bytes(&hres) > download_bytes(&e));
        assert_eq!(
            file_name(&hres, &Run { date: "20261015".into(), hh: "12".into(), t: 0 }, 240),
            "ECMWF_IFS_0p1_20261015_12z_f000-f240_42S-34S_172E-179E.grb2"
        );
    }

    fn req(model: Model) -> Request {
        Request { model, area: [172.0, -41.5, 178.5, -34.0], params: vec!["wind".into(), "mslp".into()], hours: 240, every: 3, depth: 0.0 }
    }

    #[test]
    fn steps_follow_the_model() {
        let h = Model::Gfs0p25.hours(240, 1);
        assert_eq!(&h[..3], &[0, 1, 2]);
        assert!(h.contains(&120) && h.contains(&123) && !h.contains(&121));
        assert_eq!(*h.last().unwrap(), 240);
        let h3 = Model::Gfs0p25.hours(240, 3);
        assert_eq!(h3.len(), 81);
        let h6 = Model::Gfs1p00.hours(384, 6);
        assert_eq!(h6.len(), 65);
        assert!(h6.iter().all(|x| x % 6 == 0));
        let h2 = Model::Gfs0p50.hours(24, 1); // 3-hourly only
        assert_eq!(h2, vec![0, 3, 6, 9, 12, 15, 18, 21, 24]);
    }

    #[test]
    fn urls_match_nomads() {
        let run = Run { date: "20261015".into(), hh: "12".into(), t: 0 };
        let u = step_url(&req(Model::Gfs0p25), &run, 3);
        assert!(
            u.starts_with("https://nomads.ncep.noaa.gov/cgi-bin/filter_gfs_0p25.pl?dir=%2Fgfs.20261015%2F12%2Fatmos&file=gfs.t12z.pgrb2.0p25.f003&"),
            "{u}"
        );
        assert!(u.contains("var_UGRD=on") && u.contains("lev_10_m_above_ground=on") && u.contains("var_PRMSL=on"));
        assert!(u.ends_with("&subregion=&leftlon=172&rightlon=178.5&toplat=-34&bottomlat=-41.5"), "{u}");
        let w = Request { params: vec!["waves".into()], ..req(Model::GfsWave0p25) };
        let u = step_url(&w, &run, 3);
        assert!(
            u.contains("filter_gfswave.pl?dir=%2Fgfs.20261015%2F12%2Fwave%2Fgridded&file=gfswave.t12z.global.0p25.f003.grib2&var_HTSGW=on&lev_surface=on"),
            "{u}"
        );
        // across the date line: 170E to 170W
        let d = Request { area: [170.0, -45.0, -170.0, -30.0], ..req(Model::Gfs1p00) };
        assert!(step_url(&d, &run, 0).contains("leftlon=170&rightlon=190"));
        assert_eq!(file_name(&d, &run, 384), "GFS_1p00_20261015_12z_f000-f384_45S-30S_170E-170W.grb2");
    }

    #[test]
    fn size_estimate_is_sane() {
        // 27 x 31 points, 3 fields, 81 steps: around 300 kB
        let b = estimate_bytes(&req(Model::Gfs0p25));
        assert!(b > 150_000 && b < 600_000, "{b}");
    }

    pub(crate) fn b64(s: &str) -> Vec<u8> {
        let val = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        let mut out = Vec::new();
        for ch in s.trim().as_bytes().chunks(4) {
            let n = ch.iter().filter(|&&c| c != b'=').count();
            let v: Vec<u32> = ch.iter().map(|&c| if c == b'=' { 0 } else { val(c) as u32 }).collect();
            let w = (v[0] << 18) | (v[1] << 12) | (v.get(2).copied().unwrap_or(0) << 6) | v.get(3).copied().unwrap_or(0);
            out.extend_from_slice(&[(w >> 16) as u8, (w >> 8) as u8, w as u8][..n - 1]);
        }
        out
    }

    /// Real NOMADS filter output (GFS 0.25°, run 15 Oct 2026 12Z, f003, 41–40°S 173–174°E):
    /// pressure, gust, U and V decode and give wind at Nelson.
    #[test]
    fn decodes_real_gfs_filter_output() {
        let g = grib::GribFile::from_bytes(&b64(include_str!("../tests/gfs_0p25_nelson_f003.b64")));
        let w = g.wind(-40.5, 173.5, g.times(grib::Param::WindU)[0]).expect("wind");
        assert!(w.tws >= 0.0 && w.tws < 80.0 && (0.0..=360.0).contains(&w.twd), "{w:?}");
        assert_eq!(g.times(grib::Param::Mslp).len(), 1);
        assert_eq!(g.times(grib::Param::Gust).len(), 1);
        let p = g.value(grib::Param::Mslp, -40.5, 173.5, g.times(grib::Param::Mslp)[0]).unwrap();
        assert!(p > 95_000.0 && p < 105_000.0, "{p}");
    }

    /// Real GFS and GFS-Wave output of one run, written one after the other: one
    /// file with wind and waves, named as such.
    #[test]
    fn gfs_and_gfswave_in_one_file() {
        let dir = std::env::temp_dir().join(format!("racenav-gfsw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("GFS_0p25_t.grb2"), dir.join("GFSwave_0p25_t.grb2"));
        std::fs::write(&a, b64(include_str!("../tests/gfs_0p25_nelson_f003.b64"))).unwrap();
        std::fs::write(&b, b64(include_str!("../tests/gfswave_0p25_nelson_f003.b64"))).unwrap();
        let out = grib::combine::combine(&b, &a).unwrap();
        let g = grib::GribFile::open(&out).unwrap();
        let t = g.times(grib::Param::WaveHeight)[0];
        assert!(g.wind(-40.5, 173.5, g.times(grib::Param::WindU)[0]).is_some());
        assert!(g.value(grib::Param::WaveHeight, -40.5, 173.5, t).is_some() || g.value(grib::Param::WaveHeight, -40.0, 174.5, t).is_some());
        assert_eq!(g.model().0, "NCEP GFS + waves");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real GFS-Wave filter output (same run and area): wave height decodes.
    #[test]
    fn decodes_real_gfswave_filter_output() {
        let g = grib::GribFile::from_bytes(&b64(include_str!("../tests/gfswave_0p25_nelson_f003.b64")));
        let t = g.times(grib::Param::WaveHeight);
        assert_eq!(t.len(), 1);
        // the area is partly land (Nelson): find a sea point with a value
        let any = (0..5)
            .flat_map(|i| (0..5).map(move |j| (-41.0 + 0.25 * i as f64, 173.0 + 0.25 * j as f64)))
            .filter_map(|(la, lo)| g.value(grib::Param::WaveHeight, la, lo, t[0]))
            .count();
        assert!(any > 0);
    }
}
