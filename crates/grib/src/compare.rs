//! Agreement between GRIBs: how much several forecasts for the same time
//! disagree, point by point. The same displays as the earlier stand-alone
//! "GRIB agreement viewer":
//!
//! - **vector**: RMS distance of each wind from the mean wind (kn), speed and direction together;
//! - **speed**: spread of wind speed (kn);
//! - **direction**: spread of wind direction (°), circular, left blank where the mean wind is under 6 kn;
//! - **pressure**: spread of MSL pressure (hPa);
//! - **trend**: newest run minus oldest run, wind speed (kn; + = the newer run is windier).
//!
//! Spread is max − min for up to six forecasts, P90 − P10 above that (so one
//! outlier in a big set doesn't decide the picture). Each forecast is sampled
//! bilinearly at the slider time (linear in time between its fields) on a common
//! grid as coarse as the coarsest file. Ensemble files count as their control run.

use crate::{GribFile, Param};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Vector,
    Speed,
    Direction,
    Pressure,
    Trend,
}

impl Metric {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "vector" => Metric::Vector,
            "speed" => Metric::Speed,
            "direction" => Metric::Direction,
            "pressure" => Metric::Pressure,
            "trend" => Metric::Trend,
            _ => return None,
        })
    }
}

/// Direction spread is left out where the mean wind is lighter than this (kn).
pub const DIR_MIN_KN: f64 = 6.0;
const KN: f64 = 3600.0 / 1852.0;

/// Result on a regular grid (row by row from the south-west corner).
#[derive(Debug, Clone)]
pub struct AgreementGrid {
    pub lon0: f64,
    pub lat0: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub ni: usize,
    pub nj: usize,
    pub values: Vec<Option<f32>>,
    /// Mean wind (u, v in m/s) where at least two forecasts have wind, for barbs.
    pub mean_wind: Vec<Option<(f32, f32)>>,
    /// Indexes into the input of the files that cover the time and area.
    pub used: Vec<usize>,
}

/// u, v (m/s) at a point: from U/V, or from speed and direction (UKMO style files).
pub fn wind_uv(f: &GribFile, lat: f64, lon: f64, t: i64) -> Option<(f64, f64)> {
    if let (Some(u), Some(v)) = (f.value(Param::WindU, lat, lon, t), f.value(Param::WindV, lat, lon, t)) {
        return Some((u, v));
    }
    let s = f.value(Param::WindSpeed, lat, lon, t)?;
    let d = f.value(Param::WindDir, lat, lon, t)?.to_radians();
    Some((-s * d.sin(), -s * d.cos()))
}

/// max − min for up to six values, P90 − P10 above that.
pub fn spread(v: &mut [f64]) -> Option<f64> {
    if v.len() < 2 {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.len() <= 6 {
        return Some(v[v.len() - 1] - v[0]);
    }
    let q = |p: f64| {
        let x = p * (v.len() - 1) as f64;
        let (i, f) = (x.floor() as usize, x.fract());
        v[i] + (v[(i + 1).min(v.len() - 1)] - v[i]) * f
    };
    Some(q(0.9) - q(0.1))
}

/// Signed difference a − b in degrees, in -180..180.
fn ang_diff(a: f64, b: f64) -> f64 {
    (a - b + 540.0).rem_euclid(360.0) - 180.0
}

/// The grid of a file's first field of `p` (or of wind speed, for speed/direction files).
fn first_grid(f: &GribFile, p: Param) -> Option<crate::Grid> {
    f.series(p).first().map(|x| x.grid).or_else(|| f.series(Param::WindSpeed).first().map(|x| x.grid))
}

fn covers(f: &GribFile, p: Param, t: i64) -> bool {
    let ts = f.times(p);
    matches!((ts.first(), ts.last()), (Some(a), Some(b)) if *a <= t && t <= *b)
}

fn has_wind(f: &GribFile, t: i64) -> bool {
    covers(f, Param::WindU, t) || covers(f, Param::WindSpeed, t)
}

/// Compare `files` at time `t` over `bbox` [w, s, e, n] (longitudes as the chart
/// gives them; files running 0..360 are handled), with at most `max_points` points.
pub fn agreement(files: &[&GribFile], metric: Metric, t: i64, bbox: [f64; 4], max_points: usize) -> Option<AgreementGrid> {
    let needs_pressure = metric == Metric::Pressure;
    let used: Vec<usize> = (0..files.len())
        .filter(|&i| if needs_pressure { covers(files[i], Param::Mslp, t) } else { has_wind(files[i], t) })
        .collect();
    if used.len() < 2 {
        return None;
    }
    // the trend needs two different runs: oldest and newest
    let runs: Vec<i64> = used.iter().map(|&i| files[i].model().1).collect();
    let (old_i, new_i) = {
        let mut o = 0;
        let mut n = 0;
        for k in 0..used.len() {
            if runs[k] < runs[o] {
                o = k;
            }
            if runs[k] > runs[n] {
                n = k;
            }
        }
        (used[o], used[n])
    };
    if metric == Metric::Trend && runs.iter().min() == runs.iter().max() {
        return None;
    }
    // grid: as coarse as the coarsest file, coarser still if the view needs too many points
    let p = if needs_pressure { Param::Mslp } else { Param::WindU };
    let res = used.iter().filter_map(|&i| first_grid(files[i], p)).map(|g| g.dlat.abs().max(g.dlon.abs())).fold(0.0f64, f64::max).max(0.01);
    // each file's own longitude range: a file stored 0..360 is asked in its own longitudes
    let boxes: Vec<Option<[f64; 4]>> = files.iter().map(|f| first_grid(f, p).map(|g| g.bbox())).collect();
    let (w, s, e, n) = (bbox[0], bbox[1].max(-89.9), bbox[2], bbox[3].min(89.9));
    let mut step = res;
    while ((e - w) / step).ceil() * ((n - s) / step).ceil() > max_points.max(4) as f64 {
        step *= 1.25;
    }
    let lon0 = (w / step).floor() * step;
    let lat0 = (s / step).floor() * step;
    let ni = (((e - lon0) / step).ceil() as usize + 1).max(1);
    let nj = (((n - lat0) / step).ceil() as usize + 1).max(1);
    let mut values = Vec::with_capacity(ni * nj);
    let mut mean_wind = Vec::with_capacity(ni * nj);
    let mut buf: Vec<f64> = Vec::with_capacity(used.len());
    let mut uv: Vec<(f64, f64)> = Vec::with_capacity(used.len());
    for j in 0..nj {
        let lat = lat0 + j as f64 * step;
        for i in 0..ni {
            let lon = lon0 + i as f64 * step;
            let sample_lon = |k: usize| match boxes[k] {
                Some(b) if lon < b[0] => lon + 360.0,
                Some(b) if lon > b[2] => lon - 360.0,
                _ => lon,
            };
            let mut val = None;
            let mut mw = None;
            if needs_pressure {
                buf.clear();
                for &k in &used {
                    if let Some(x) = files[k].value(Param::Mslp, lat, sample_lon(k), t) {
                        buf.push(x / 100.0);
                    }
                }
                val = spread(&mut buf);
            } else {
                uv.clear();
                for &k in &used {
                    if let Some(w) = wind_uv(files[k], lat, sample_lon(k), t) {
                        uv.push(w);
                    }
                }
                if uv.len() >= 2 {
                    let m = uv.len() as f64;
                    let (mu, mv) = (uv.iter().map(|x| x.0).sum::<f64>() / m, uv.iter().map(|x| x.1).sum::<f64>() / m);
                    mw = Some((mu as f32, mv as f32));
                    val = match metric {
                        Metric::Vector => Some((uv.iter().map(|x| (x.0 - mu).powi(2) + (x.1 - mv).powi(2)).sum::<f64>() / m).sqrt() * KN),
                        Metric::Speed => {
                            buf.clear();
                            buf.extend(uv.iter().map(|x| x.0.hypot(x.1) * KN));
                            spread(&mut buf)
                        }
                        Metric::Direction => {
                            // spread of each direction about the mean direction (unit vectors), blank in light airs
                            let mean_kn = uv.iter().map(|x| x.0.hypot(x.1)).sum::<f64>() / m * KN;
                            if mean_kn < DIR_MIN_KN {
                                None
                            } else {
                                let (su, sv) = uv.iter().fold((0.0, 0.0), |a, x| {
                                    let h = x.0.hypot(x.1).max(1e-9);
                                    (a.0 + x.0 / h, a.1 + x.1 / h)
                                });
                                let mean_dir = su.atan2(sv).to_degrees();
                                buf.clear();
                                buf.extend(uv.iter().map(|x| ang_diff(x.0.atan2(x.1).to_degrees(), mean_dir)));
                                spread(&mut buf)
                            }
                        }
                        Metric::Trend => {
                            let a = wind_uv(files[new_i], lat, sample_lon(new_i), t);
                            let b = wind_uv(files[old_i], lat, sample_lon(old_i), t);
                            a.zip(b).map(|(a, b)| (a.0.hypot(a.1) - b.0.hypot(b.1)) * KN)
                        }
                        Metric::Pressure => None,
                    };
                }
            }
            values.push(val.filter(|v| v.is_finite()).map(|v| ((v * 10.0).round() / 10.0) as f32));
            mean_wind.push(mw);
        }
    }
    Some(AgreementGrid { lon0, lat0, dlon: step, dlat: step, ni, nj, values, mean_wind, used })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{Field, Grid};

    /// A file with one time of uniform wind (u, v m/s) and pressure (hPa), run at `run`.
    fn file(u: f32, v: f32, hpa: f32, run: i64, t: i64) -> GribFile {
        let g = Grid::regular(5, 5, -42.0, 172.0, 0.5, 0.5);
        let f = |param, val: f32| Field {
            param, discipline: 0, category: 0, number: 0, surface_type: 103, surface_value: 10.0, centre: 98,
            ref_time: run, valid_time: t, member: None, grid: g, values: vec![val; 25], local: None,
        };
        GribFile::from_fields(vec![f(Param::WindU, u), f(Param::WindV, v), f(Param::Mslp, hpa * 100.0)])
    }

    #[test]
    fn two_forecasts() {
        let t = 1_792_000_000;
        // A: 5 m/s from the west; B: 5 m/s from the south, a later run and windier in pressure terms
        let a = file(5.0, 0.0, 1010.0, t - 86400, t);
        let b = file(0.0, 5.0, 1013.0, t - 43200, t);
        let bbox = [172.5, -41.5, 173.5, -40.5];
        let at = |m| agreement(&[&a, &b], m, t, bbox, 1000).unwrap();
        let first = |g: &AgreementGrid| g.values.iter().flatten().next().copied().unwrap();
        // vector: each 3.54 m/s from the mean (2.5, 2.5) = 6.87 kn
        assert!((first(&at(Metric::Vector)) - 6.9).abs() < 0.11);
        assert_eq!(first(&at(Metric::Speed)), 0.0);
        assert!((first(&at(Metric::Direction)) - 90.0).abs() < 0.11);
        assert!((first(&at(Metric::Pressure)) - 3.0).abs() < 0.11);
        assert_eq!(first(&at(Metric::Trend)), 0.0);
        // one forecast only, or none covering the time: nothing to compare
        assert!(agreement(&[&a], Metric::Vector, t, bbox, 1000).is_none());
        assert!(agreement(&[&a, &b], Metric::Vector, t + 3600, bbox, 1000).is_none());
        // light airs: no direction spread
        let c = file(1.0, 0.0, 1010.0, t - 86400, t);
        let d = file(0.0, 1.0, 1010.0, t - 43200, t);
        assert!(agreement(&[&c, &d], Metric::Direction, t, bbox, 1000).unwrap().values.iter().all(|v| v.is_none()));
        // trend: newer run minus older; B windier by 5 m/s = +9.7 kn
        let e = file(10.0, 0.0, 1010.0, t - 43200, t);
        assert!((first(&agreement(&[&a, &e], Metric::Trend, t, bbox, 1000).unwrap()) - 9.7).abs() < 0.11);
        // same run twice: no trend
        assert!(agreement(&[&a, &a], Metric::Trend, t, bbox, 1000).is_none());
    }

    #[test]
    fn spread_rule() {
        assert_eq!(spread(&mut [3.0]), None);
        assert_eq!(spread(&mut [3.0, 7.0, 5.0]), Some(4.0));
        // seven or more: P90 - P10, one outlier counts less
        let mut v = vec![10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 30.0];
        let s = spread(&mut v).unwrap();
        assert!(s > 0.0 && s < 20.0, "{s}");
    }

    #[test]
    fn angle_differences_wrap() {
        assert_eq!(ang_diff(10.0, 350.0), 20.0);
        assert_eq!(ang_diff(350.0, 10.0), -20.0);
        assert_eq!(ang_diff(180.0, 0.0), -180.0);
    }
}
