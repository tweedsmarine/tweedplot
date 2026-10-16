//! Verify: how well earlier runs forecast the wind that the later analysis
//! (hour 0) shows — the "Verify" display of the earlier GRIB agreement viewer.
//!
//! - **Verify times** are the hour 0 of a loaded run that an earlier loaded run also forecast.
//! - **Reference**: the average of the deterministic hour-0 fields valid then
//!   (ensembles, as their mean, only if there is nothing else). Hour 0 is the
//!   model's analysis, not an observation: good offshore, weaker near coasts and in
//!   light airs, and it favours its own model; averaging several analyses helps.
//! - **Scores** for each earlier forecast over the sea points in view: RMS vector
//!   error (kn), speed bias (forecast − reference, kn), mean absolute direction
//!   error and veer bias (°, + = forecast veered clockwise; where the reference is
//!   6 kn or more), MSL pressure bias (hPa). Land is left out using any loaded wave
//!   field (missing over land); without one, every point counts.
//! - **Track record**: the scores averaged over every verify time, by model and lead (≤ 24 h, ≤ 48 h, > 48 h).

use crate::compare::{common_grid, covers, has_wind, layer_value, layer_wind, own_lon, Source};
use crate::{GribFile, Param};

const KN: f64 = 3600.0 / 1852.0;
const DIR_MIN_KN: f64 = 6.0;

/// One forecast being verified at a verify time.
#[derive(Debug, Clone)]
pub struct Forecast {
    pub file: usize,
    pub lead_h: f64,
}

#[derive(Debug, Clone)]
pub struct VerifyTime {
    pub t: i64,
    /// Files whose hour 0 is the reference.
    pub refs: Vec<usize>,
    /// Earlier runs that forecast this time.
    pub forecasts: Vec<Forecast>,
}

fn src(f: &GribFile) -> Source {
    if f.members().len() > 1 { Source::Mean } else { Source::Control }
}

fn has_hour0(f: &GribFile) -> Option<i64> {
    let run = f.model().1;
    let mut ts = f.any_times(Param::WindU);
    if ts.is_empty() {
        ts = f.any_times(Param::WindSpeed);
    }
    ts.contains(&run).then_some(run)
}

/// Every verify time among these files, oldest first.
pub fn verify_times(files: &[&GribFile]) -> Vec<VerifyTime> {
    let mut ts: Vec<i64> = files.iter().filter_map(|f| has_hour0(f)).collect();
    ts.sort_unstable();
    ts.dedup();
    let mut out = Vec::new();
    for t in ts {
        let at_t: Vec<usize> = (0..files.len()).filter(|&i| has_hour0(files[i]) == Some(t)).collect();
        let det: Vec<usize> = at_t.iter().copied().filter(|&i| files[i].members().len() <= 1).collect();
        let refs = if det.is_empty() { at_t } else { det };
        let mut forecasts: Vec<Forecast> = (0..files.len())
            .filter(|&i| files[i].model().1 < t && has_wind(files[i], t))
            .map(|i| Forecast { file: i, lead_h: (t - files[i].model().1) as f64 / 3600.0 })
            .collect();
        forecasts.sort_by(|a, b| a.lead_h.partial_cmp(&b.lead_h).unwrap());
        if !forecasts.is_empty() && !refs.is_empty() {
            out.push(VerifyTime { t, refs, forecasts });
        }
    }
    out
}

/// Scores of one forecast at one verify time.
#[derive(Debug, Clone, Default)]
pub struct Score {
    pub file: usize,
    pub lead_h: f64,
    /// Sea points scored.
    pub n: usize,
    pub rms_kn: f64,
    pub speed_bias_kn: f64,
    pub dir_abs_deg: Option<f64>,
    pub veer_deg: Option<f64>,
    pub msl_bias_hpa: Option<f64>,
}

/// Signed a − b in degrees, -180..180.
fn ang(a: f64, b: f64) -> f64 {
    (a - b + 540.0).rem_euclid(360.0) - 180.0
}
/// Direction the wind blows from, degrees.
fn from_dir(u: f64, v: f64) -> f64 {
    ((-u).atan2(-v).to_degrees() + 360.0) % 360.0
}

/// A sea test from any loaded wave field: true where waves are given (or everywhere if no file has waves).
fn sea_mask<'a>(files: &'a [&'a GribFile], t: i64) -> impl Fn(f64, f64) -> bool + 'a {
    let wave = files.iter().position(|f| !f.any_times(Param::WaveHeight).is_empty());
    move |lat: f64, lon: f64| match wave {
        None => true,
        Some(k) => {
            let f = files[k];
            let ts = f.any_times(Param::WaveHeight);
            // the wave field nearest in time: land is missing in every one of them
            let near = ts.iter().copied().min_by_key(|x| (x - t).abs()).unwrap();
            let b = f.any_grid(Param::WaveHeight).map(|g| g.bbox());
            let inside = b.is_some_and(|b| lat >= b[1] && lat <= b[3]);
            !inside || layer_value(f, src(f), Param::WaveHeight, lat, own_lon(b, lon), near).is_some_and(f64::is_finite)
        }
    }
}

/// Reference wind (u, v m/s) and MSL (hPa) at a point: the mean of the reference files.
fn reference(files: &[&GribFile], refs: &[usize], boxes: &[Option<[f64; 4]>], lat: f64, lon: f64, t: i64) -> (Option<(f64, f64)>, Option<f64>) {
    let (mut su, mut sv, mut nw, mut sp, mut np) = (0.0, 0.0, 0, 0.0, 0);
    for &r in refs {
        let f = files[r];
        let lo = own_lon(boxes[r], lon);
        if let Some((u, v)) = layer_wind(f, src(f), lat, lo, t) {
            su += u;
            sv += v;
            nw += 1;
        }
        if let Some(p) = layer_value(f, src(f), Param::Mslp, lat, lo, t) {
            sp += p / 100.0;
            np += 1;
        }
    }
    ((nw > 0).then(|| (su / nw as f64, sv / nw as f64)), (np > 0).then(|| sp / np as f64))
}

/// Score every forecast of a verify time over `bbox`.
pub fn scores(files: &[&GribFile], vt: &VerifyTime, bbox: [f64; 4], max_points: usize) -> Vec<Score> {
    let mut used: Vec<usize> = vt.refs.clone();
    used.extend(vt.forecasts.iter().map(|f| f.file));
    let g = common_grid(files, &used, Param::WindU, bbox, max_points);
    let sea = sea_mask(files, vt.t);
    #[derive(Default)]
    struct Acc { n: usize, se: f64, sb: f64, nd: usize, sd: f64, sv: f64, np: usize, sp: f64 }
    let mut acc: Vec<Acc> = vt.forecasts.iter().map(|_| Acc::default()).collect();
    for j in 0..g.nj {
        let lat = g.lat0 + j as f64 * g.step;
        for i in 0..g.ni {
            let lon = g.lon0 + i as f64 * g.step;
            if !sea(lat, lon) {
                continue;
            }
            let (rw, rp) = reference(files, &vt.refs, &g.boxes, lat, lon, vt.t);
            let Some((ru, rv)) = rw else { continue };
            let rs = ru.hypot(rv) * KN;
            for (k, fc) in vt.forecasts.iter().enumerate() {
                let f = files[fc.file];
                let lo = own_lon(g.boxes[fc.file], lon);
                let Some((u, v)) = layer_wind(f, src(f), lat, lo, vt.t) else { continue };
                let a = &mut acc[k];
                a.n += 1;
                a.se += ((u - ru).powi(2) + (v - rv).powi(2)) * KN * KN;
                a.sb += u.hypot(v) * KN - rs;
                if rs >= DIR_MIN_KN {
                    let d = ang(from_dir(u, v), from_dir(ru, rv));
                    a.nd += 1;
                    a.sd += d.abs();
                    a.sv += d;
                }
                if let (Some(rp), Some(p)) = (rp, layer_value(f, src(f), Param::Mslp, lat, lo, vt.t)) {
                    a.np += 1;
                    a.sp += p / 100.0 - rp;
                }
            }
        }
    }
    vt.forecasts
        .iter()
        .zip(acc)
        .filter(|(_, a)| a.n > 0)
        .map(|(fc, a)| Score {
            file: fc.file,
            lead_h: fc.lead_h,
            n: a.n,
            rms_kn: (a.se / a.n as f64).sqrt(),
            speed_bias_kn: a.sb / a.n as f64,
            dir_abs_deg: (a.nd > 0).then(|| a.sd / a.nd as f64),
            veer_deg: (a.nd > 0).then(|| a.sv / a.nd as f64),
            msl_bias_hpa: (a.np > 0).then(|| a.sp / a.np as f64),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Vector error, kn.
    Vector,
    /// Speed error, forecast − reference, kn.
    Speed,
    /// Direction error, forecast − reference, ° (blank under 6 kn).
    Direction,
}

/// The error field of one forecast against the reference at a verify time, on a grid like `compare::agreement`'s.
pub fn error_grid(files: &[&GribFile], vt: &VerifyTime, forecast: usize, kind: ErrorKind, bbox: [f64; 4], max_points: usize) -> Option<crate::compare::AgreementGrid> {
    if !vt.forecasts.iter().any(|f| f.file == forecast) || !covers(files[forecast], Param::WindU, vt.t) && !covers(files[forecast], Param::WindSpeed, vt.t) {
        return None;
    }
    let mut used = vt.refs.clone();
    used.push(forecast);
    let g = common_grid(files, &used, Param::WindU, bbox, max_points);
    let sea = sea_mask(files, vt.t);
    let mut values = Vec::with_capacity(g.ni * g.nj);
    let mut mean_wind = Vec::with_capacity(g.ni * g.nj);
    let f = files[forecast];
    for j in 0..g.nj {
        let lat = g.lat0 + j as f64 * g.step;
        for i in 0..g.ni {
            let lon = g.lon0 + i as f64 * g.step;
            let (rw, _) = if sea(lat, lon) { reference(files, &vt.refs, &g.boxes, lat, lon, vt.t) } else { (None, None) };
            let fw = layer_wind(f, src(f), lat, own_lon(g.boxes[forecast], lon), vt.t);
            let v = match (rw, fw) {
                (Some((ru, rv)), Some((u, v))) => match kind {
                    ErrorKind::Vector => Some(((u - ru).powi(2) + (v - rv).powi(2)).sqrt() * KN),
                    ErrorKind::Speed => Some((u.hypot(v) - ru.hypot(rv)) * KN),
                    ErrorKind::Direction => (ru.hypot(rv) * KN >= DIR_MIN_KN).then(|| ang(from_dir(u, v), from_dir(ru, rv))),
                },
                _ => None,
            };
            values.push(v.map(|x| ((x * 10.0).round() / 10.0) as f32));
            mean_wind.push(rw.map(|(u, v)| (u as f32, v as f32)));
        }
    }
    Some(crate::compare::AgreementGrid { lon0: g.lon0, lat0: g.lat0, dlon: g.step, dlat: g.step, ni: g.ni, nj: g.nj, values, mean_wind, used, layers: 2, members: false })
}

/// One line of the track record.
#[derive(Debug, Clone)]
pub struct TrackRow {
    pub model: String,
    /// "≤ 24 h", "≤ 48 h" or "> 48 h"
    pub band: &'static str,
    /// Verify times averaged.
    pub count: usize,
    pub rms_kn: f64,
    pub speed_bias_kn: f64,
    pub dir_abs_deg: Option<f64>,
    pub veer_deg: Option<f64>,
    pub msl_bias_hpa: Option<f64>,
}

pub fn band(lead_h: f64) -> &'static str {
    if lead_h <= 24.0 { "≤ 24 h" } else if lead_h <= 48.0 { "≤ 48 h" } else { "> 48 h" }
}

/// Scores at every verify time averaged by model and lead band.
pub fn track_record(files: &[&GribFile], bbox: [f64; 4], max_points: usize) -> Vec<TrackRow> {
    let mut rows: Vec<(String, &'static str, Vec<Score>)> = Vec::new();
    for vt in verify_times(files) {
        for s in scores(files, &vt, bbox, max_points) {
            let model = files[s.file].model().0;
            let b = band(s.lead_h);
            match rows.iter_mut().find(|r| r.0 == model && r.1 == b) {
                Some(r) => r.2.push(s),
                None => rows.push((model, b, vec![s])),
            }
        }
    }
    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    let mut out: Vec<TrackRow> = rows
        .into_iter()
        .map(|(model, band, s)| TrackRow {
            count: s.len(),
            rms_kn: mean(&s.iter().map(|x| x.rms_kn).collect::<Vec<_>>()).unwrap_or(0.0),
            speed_bias_kn: mean(&s.iter().map(|x| x.speed_bias_kn).collect::<Vec<_>>()).unwrap_or(0.0),
            dir_abs_deg: mean(&s.iter().filter_map(|x| x.dir_abs_deg).collect::<Vec<_>>()),
            veer_deg: mean(&s.iter().filter_map(|x| x.veer_deg).collect::<Vec<_>>()),
            msl_bias_hpa: mean(&s.iter().filter_map(|x| x.msl_bias_hpa).collect::<Vec<_>>()),
            model,
            band,
        })
        .collect();
    out.sort_by(|a, b| a.model.cmp(&b.model).then(a.band.cmp(b.band)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, Grid};

    /// A file run at `run` with times run, run+12 h, run+24 h; wind (u, 0) m/s; MSL hPa.
    fn run_file(run: i64, u: f32, hpa: f32) -> GribFile {
        let g = Grid::regular(5, 5, -42.0, 172.0, 0.5, 0.5);
        let mut fields = Vec::new();
        for h in [0, 12, 24] {
            for (param, val) in [(Param::WindU, u), (Param::WindV, 0.0), (Param::Mslp, hpa * 100.0)] {
                fields.push(Field {
                    param, discipline: 0, category: 0, number: 0, surface_type: 103, surface_value: 10.0, centre: 98,
                    ref_time: run, valid_time: run + h * 3600, member: None, grid: g, values: vec![val; 25], local: None,
                });
            }
        }
        GribFile::from_fields(fields)
    }

    #[test]
    fn verify_two_runs() {
        let t0 = 1_792_000_000;
        // older run: 8 m/s westerly, 1012 hPa; newer run 12 h later: 10 m/s, 1010 hPa
        let old = run_file(t0, 8.0, 1012.0);
        let new = run_file(t0 + 43200, 10.0, 1010.0);
        let files = [&old, &new];
        let vts = verify_times(&files);
        assert_eq!(vts.len(), 1);
        let vt = &vts[0];
        assert_eq!((vt.t, vt.refs.clone()), (t0 + 43200, vec![1]));
        assert_eq!(vt.forecasts.len(), 1);
        assert_eq!((vt.forecasts[0].file, vt.forecasts[0].lead_h), (0, 12.0));
        let bbox = [172.5, -41.5, 173.5, -40.5];
        let s = &scores(&files, vt, bbox, 1000)[0];
        // 2 m/s too light = 3.89 kn vector error and -3.89 kn speed bias; direction right; MSL 2 hPa high
        assert!((s.rms_kn - 2.0 * KN).abs() < 1e-6);
        assert!((s.speed_bias_kn + 2.0 * KN).abs() < 1e-6);
        assert!(s.dir_abs_deg.unwrap().abs() < 1e-6);
        assert!((s.msl_bias_hpa.unwrap() - 2.0).abs() < 1e-6);
        let g = error_grid(&files, vt, 0, ErrorKind::Speed, bbox, 1000).unwrap();
        assert_eq!(g.values.iter().flatten().next().copied(), Some(-3.9));
        let tr = track_record(&files, bbox, 1000);
        assert_eq!(tr.len(), 1);
        assert_eq!((tr[0].band, tr[0].count), ("≤ 24 h", 1));
    }
}
