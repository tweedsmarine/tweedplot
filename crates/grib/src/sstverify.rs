//! Sea temperature products scored against the boat's own water temperature (P1.4).
//!
//! The boat's sensor is the one observation a skipper can trust where they fish, so every
//! sea temperature file (MUR, ACSPO, Geo-Polar, HYCOM, Copernicus…) is read at the boat's
//! logged positions and times and compared, like the wind forecast verification.
//!
//! - **Observations** come from the library's tracks (fixes with a sea temperature), thinned
//!   to one every 10 minutes or 2 km so a slow drift doesn't outweigh a passage.
//! - **Matching:** a file's sea temperature field nearest in time, if it is within
//!   `max_age_s` of the observation (no interpolation across days; analyses are daily).
//!   Where several files of the same product cover an observation, the nearest in time wins.
//! - **Scores per product** (the model name): bias (product − boat, °C), mean absolute error,
//!   RMS, the spread left once the bias is taken off (SD), coverage (share of the
//!   observations in the product's area and time that got a value; clouds and land
//!   leave gaps), and the mean age of the data used.
//!
//! A hull sensor reads the water at 0.5–2 m. Satellite products are the skin or the
//! foundation temperature, so a steady offset on calm sunny days is expected: the SD shows
//! how well a product follows the changes, which is what finds temperature breaks.

use crate::compare::own_lon;
use crate::{GribFile, Param};

const K: f64 = 273.15;

/// One of the boat's sea temperature readings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obs {
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
    /// °C
    pub temp: f64,
}

/// Plausible water temperatures only; drops readings outside them (a dry sensor, a bad sentence).
pub fn plausible(o: &Obs) -> bool {
    o.temp.is_finite() && (-2.5..=36.0).contains(&o.temp) && o.lat.abs() <= 90.0 && o.lon.is_finite()
}

/// Metres between two points (haversine).
fn dist_m(a: &Obs, b: &Obs) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = p2 - p1;
    let dl = (b.lon - a.lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371_000.0 * h.sqrt().asin()
}

/// Keep one reading every `min_s` seconds or `min_m` metres, whichever comes first.
/// The input is sorted by time first; implausible readings are dropped.
pub fn thin(mut obs: Vec<Obs>, min_s: i64, min_m: f64) -> Vec<Obs> {
    obs.retain(plausible);
    obs.sort_by_key(|o| o.t);
    let mut out: Vec<Obs> = Vec::with_capacity(obs.len() / 4 + 1);
    for o in obs {
        match out.last() {
            Some(l) if o.t - l.t < min_s && dist_m(l, &o) < min_m => {}
            _ => out.push(o),
        }
    }
    out
}

/// `lon` moved by whole turns into the box `b` ([west, south, east, north]) where it can be.
pub fn lon_in(b: [f64; 4], lon: f64) -> f64 {
    own_lon(Some(b), lon)
}

/// The product's value (°C) and the age of its field (observation time − field time, s).
pub fn sea_temp_at(f: &GribFile, lat: f64, lon: f64, t: i64, max_age_s: i64) -> Option<(f64, i64)> {
    let grid = f.any_grid(Param::SeaTemp)?;
    let b = grid.bbox();
    let lo = own_lon(Some(b), lon);
    if lat < b[1] || lat > b[3] || lo < b[0] || lo > b[2] {
        return None;
    }
    let near = f.times(Param::SeaTemp).into_iter().min_by_key(|x| (x - t).abs())?;
    if (near - t).abs() > max_age_s {
        return None;
    }
    let v = f.value(Param::SeaTemp, lat, lo, near)?;
    // files keep sea temperature in kelvin; a °C value (never above 60) is left as it is
    let c = if v > 100.0 { v - K } else { v };
    c.is_finite().then_some((c, t - near))
}

/// Whether the file covers the observation's place and time at all (with or without a value).
fn covers(f: &GribFile, o: &Obs, max_age_s: i64) -> bool {
    let Some(grid) = f.any_grid(Param::SeaTemp) else { return false };
    let b = grid.bbox();
    let lo = own_lon(Some(b), o.lon);
    if o.lat < b[1] || o.lat > b[3] || lo < b[0] || lo > b[2] {
        return false;
    }
    f.times(Param::SeaTemp).into_iter().any(|x| (x - o.t).abs() <= max_age_s)
}

/// One product's matched readings.
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
    pub boat: f64,
    pub product: f64,
    /// Index of the file used.
    pub file: usize,
    /// Observation time − field time, s.
    pub age_s: i64,
}

/// A product's score against the boat.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SstScore {
    /// The model name, e.g. "MUR SST 1 km".
    pub product: String,
    /// Files of this product used.
    pub files: Vec<usize>,
    /// Readings matched.
    pub n: usize,
    /// Readings in the product's area and time (matched or a gap).
    pub covered: usize,
    /// Product − boat, °C.
    pub bias: f64,
    pub mae: f64,
    pub rms: f64,
    /// RMS once the bias is taken off, °C.
    pub sd: f64,
    /// Mean |age| of the fields used, hours.
    pub age_h: f64,
    pub pairs: Vec<Pair>,
}

impl SstScore {
    /// Share of the covered readings that got a value, 0–1.
    pub fn coverage(&self) -> f64 {
        if self.covered == 0 {
            0.0
        } else {
            self.n as f64 / self.covered as f64
        }
    }
}

/// Files that hold a sea temperature.
pub fn sst_files(files: &[&GribFile]) -> Vec<usize> {
    (0..files.len()).filter(|&i| !files[i].times(Param::SeaTemp).is_empty()).collect()
}

/// Score every sea temperature product in `files` against the boat's readings.
/// Products with no matched reading are left out; the best (lowest SD, then RMS) first.
pub fn score(files: &[&GribFile], obs: &[Obs], max_age_s: i64) -> Vec<SstScore> {
    let idx = sst_files(files);
    // group the files by product name, keeping the input order
    let mut products: Vec<(String, Vec<usize>)> = Vec::new();
    for &i in &idx {
        let name = files[i].model().0;
        match products.iter_mut().find(|p| p.0 == name) {
            Some(p) => p.1.push(i),
            None => products.push((name, vec![i])),
        }
    }
    let mut out = Vec::new();
    for (product, fs) in products {
        let mut s = SstScore { product, ..Default::default() };
        let mut used: Vec<usize> = Vec::new();
        for o in obs.iter().filter(|o| plausible(o)) {
            let mut best: Option<(f64, i64, usize)> = None;
            let mut covered = false;
            for &i in &fs {
                if !covers(files[i], o, max_age_s) {
                    continue;
                }
                covered = true;
                if let Some((v, age)) = sea_temp_at(files[i], o.lat, o.lon, o.t, max_age_s) {
                    if best.is_none_or(|b| age.abs() < b.1.abs()) {
                        best = Some((v, age, i));
                    }
                }
            }
            if covered {
                s.covered += 1;
            }
            if let Some((v, age, i)) = best {
                if !used.contains(&i) {
                    used.push(i);
                }
                s.pairs.push(Pair { t: o.t, lat: o.lat, lon: o.lon, boat: o.temp, product: v, file: i, age_s: age });
            }
        }
        s.n = s.pairs.len();
        if s.n == 0 {
            continue;
        }
        let n = s.n as f64;
        let d: Vec<f64> = s.pairs.iter().map(|p| p.product - p.boat).collect();
        s.bias = d.iter().sum::<f64>() / n;
        s.mae = d.iter().map(|x| x.abs()).sum::<f64>() / n;
        s.rms = (d.iter().map(|x| x * x).sum::<f64>() / n).sqrt();
        s.sd = (d.iter().map(|x| (x - s.bias).powi(2)).sum::<f64>() / n).sqrt();
        s.age_h = s.pairs.iter().map(|p| p.age_s.abs() as f64).sum::<f64>() / n / 3600.0;
        used.sort_unstable();
        s.files = used;
        out.push(s);
    }
    out.sort_by(|a, b| a.sd.total_cmp(&b.sd).then(a.rms.total_cmp(&b.rms)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, Grid};

    /// A daily SST file: `days` fields from `t0`, each `c0 + slope·(lon − 172)` °C (in kelvin), NaN west of `land_w`.
    fn sst_file(name_centre: u16, t0: i64, days: i64, c0: f32, slope: f32, land_w: f64) -> GribFile {
        let g = Grid::regular(11, 11, -42.0, 172.0, 0.1, 0.1);
        let mut fields = Vec::new();
        for d in 0..days {
            let mut values = Vec::with_capacity(121);
            for _j in 0..11 {
                for i in 0..11 {
                    let lon = 172.0 + i as f64 * 0.1;
                    values.push(if lon < land_w { f32::NAN } else { c0 + slope * (lon - 172.0) as f32 + 273.15 });
                }
            }
            fields.push(Field {
                param: Param::SeaTemp,
                discipline: 10,
                category: 3,
                number: 0,
                surface_type: 1,
                surface_value: 0.0,
                centre: name_centre,
                ref_time: t0,
                valid_time: t0 + d * 86400,
                member: None,
                grid: g,
                values,
                local: None,
            });
        }
        GribFile::from_fields(fields)
    }

    #[test]
    fn thinning_keeps_time_or_distance() {
        let o = |t, lon, temp| Obs { t, lat: -41.5, lon, temp };
        let v = vec![o(0, 172.5, 18.0), o(60, 172.5, 18.0), o(700, 172.5, 18.1), o(760, 172.53, 18.2), o(800, 172.5, 99.0)];
        let k = thin(v, 600, 2000.0);
        // 60 s on and still: dropped; 700 s: kept; 760 s and 2.5 km on: kept; 99 °C: dropped
        assert_eq!(k.iter().map(|x| x.t).collect::<Vec<_>>(), vec![0, 700, 760]);
    }

    #[test]
    fn scores_bias_spread_and_gaps() {
        let t0 = 1_792_000_000;
        // product A: 0.5 °C warm everywhere; product B: right on average but a slope across the area, and land west of 172.3
        let a = sst_file(7, t0, 3, 18.5, 0.0, 0.0);
        let b = sst_file(98, t0, 3, 17.75, 1.0, 172.3);
        let files = [&a, &b];
        // the boat reads 18.0 °C at four places a day apart (day 0 and 1), within the area
        let obs: Vec<Obs> = [(172.1, 0), (172.4, 3600), (172.5, 86400), (172.6, 86400 + 7200)]
            .iter()
            .map(|&(lon, dt)| Obs { t: t0 + dt, lat: -41.5, lon, temp: 18.0 })
            .collect();
        let s = score(&files, &obs, 36 * 3600);
        assert_eq!(s.len(), 2);
        let sa = s.iter().find(|x| x.files == vec![0]).unwrap();
        assert_eq!((sa.n, sa.covered), (4, 4));
        assert!((sa.bias - 0.5).abs() < 1e-4 && sa.sd < 1e-4 && (sa.rms - 0.5).abs() < 1e-4);
        let sb = s.iter().find(|x| x.files == vec![1]).unwrap();
        // 172.1 is on B's land: a gap (covered, no value)
        assert_eq!((sb.n, sb.covered), (3, 4));
        assert!((sb.coverage() - 0.75).abs() < 1e-9);
        // B at 172.4, 172.5, 172.6 = 18.15, 18.25, 18.35 → bias +0.25, spread 0.0816
        assert!((sb.bias - 0.25).abs() < 1e-3, "{}", sb.bias);
        assert!((sb.sd - 0.0816).abs() < 1e-3, "{}", sb.sd);
        // A has no spread, so it ranks first
        assert_eq!(s[0].files, vec![0]);
        // a reading 3 days after the last field is too old for either
        let late = [Obs { t: t0 + 5 * 86400, lat: -41.5, lon: 172.5, temp: 18.0 }];
        assert!(score(&files, &late, 36 * 3600).is_empty());
        // the age of the field used: 2 h after day 1's field
        assert_eq!(sa.pairs[3].age_s, 7200);
    }

    #[test]
    fn nearest_file_of_a_product_wins() {
        let t0 = 1_792_000_000;
        let d0 = sst_file(7, t0, 1, 18.0, 0.0, 0.0);
        let d1 = sst_file(7, t0 + 86400, 1, 19.0, 0.0, 0.0);
        let files = [&d0, &d1];
        let obs = [Obs { t: t0 + 80000, lat: -41.5, lon: 172.5, temp: 19.0 }];
        let s = score(&files, &obs, 36 * 3600);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pairs[0].file, 1);
        assert!(s[0].bias.abs() < 1e-4, "{}", s[0].bias);
    }
}
