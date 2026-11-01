//! Weather observations from coastal stations and buoys, for comparing what
//! the wind is actually doing with the forecast.
//!
//! | Source | What | Licence |
//! | --- | --- | --- |
//! | MetService WIS2 node (`wis2.metservice.com`) | NZ synoptic stations (many coastal and island automatic stations), hourly or 3-hourly | MetService open access data, free for personal and commercial use |
//! | aviationweather.gov METAR | Airports worldwide, half-hourly; some buoy and C-MAN reports | US government, public domain |
//! | NOAA NDBC | Moored and drifting buoys, C-MAN coastal stations (mostly US, some worldwide) | US government, public domain |
//!
//! Units: knots, hPa, °C, metres, degrees true (direction the wind comes from).

pub mod metar;
pub mod ndbc;
pub mod wis2;

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    Format(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One observation at one station.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Obs {
    /// "<source>:<station id>", unique across sources.
    pub id: String,
    pub source: &'static str,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Observation time, Unix s.
    pub t: i64,
    pub wind_dir: Option<f64>,
    pub wind_kn: Option<f64>,
    pub gust_kn: Option<f64>,
    pub pressure_hpa: Option<f64>,
    pub temp_c: Option<f64>,
    pub dewpoint_c: Option<f64>,
    pub vis_km: Option<f64>,
    pub wave_m: Option<f64>,
    pub wave_period_s: Option<f64>,
    pub wave_dir: Option<f64>,
    pub water_temp_c: Option<f64>,
    /// The report as sent (METAR text), when there is one.
    pub raw: Option<String>,
    /// "station" (land/coast), "buoy", "airport", "ship".
    pub kind: &'static str,
}

pub(crate) const MS_TO_KN: f64 = 3600.0 / 1852.0;

pub(crate) fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent("TweedPlot (marine chart plotter; weather observations; +https://tweedsmarine.nz)")
        .timeout(std::time::Duration::from_secs(40))
        .build()
        .map_err(|e| Error::Http(e.to_string()))
}

pub(crate) fn get_text(url: &str) -> Result<String> {
    let r = client()?.get(url).send().map_err(|e| Error::Http(format!("{url}: {e}")))?;
    if r.status().as_u16() == 204 {
        return Ok(String::new());
    }
    if !r.status().is_success() {
        return Err(Error::Http(format!("{url}: HTTP {}", r.status())));
    }
    r.text().map_err(|e| Error::Http(e.to_string()))
}

/// Whether a point is in [w, s, e, n] (east may pass 180).
pub fn in_bbox(lat: f64, lon: f64, b: [f64; 4]) -> bool {
    if lat < b[1] || lat > b[3] {
        return false;
    }
    let w = b[0];
    let span = b[2] - b[0];
    if span >= 360.0 {
        return true;
    }
    (lon - w).rem_euclid(360.0) <= span
}

/// The latest observation of each station in `obs`.
pub fn latest(mut obs: Vec<Obs>) -> Vec<Obs> {
    obs.sort_by(|a, b| a.id.cmp(&b.id).then(b.t.cmp(&a.t)));
    obs.dedup_by(|b, a| a.id == b.id);
    obs
}
