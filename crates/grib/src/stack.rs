//! The ocean below the surface, from a file with temperature, salinity and current at many
//! depths (a "depth stack", e.g. Copernicus Marine's model to 500 m): the profile at a point,
//! a section along a line, and maps worked out from the profiles — the thermocline (where
//! the water cools fastest with depth) and the shear (where the current changes fastest).
//!
//! What is worked out, and why:
//! * **Thermocline**: the depth between 10 and 300 m (the seasonal thermocline; the top few
//!   metres warm and cool with the day) where temperature falls fastest, and how fast, in
//!   °C per 10 m. A strong, shallow thermocline holds surface fish near the top; longlines are
//!   set above or below it for different fish.
//! * **Shear**: the depth between 5 and 300 m where the current changes fastest from one
//!   level to the next, and how much, in knots per 10 m. Strong shear across a net's
//!   height bends it and can damage it; a longline drifts unevenly through it.

use crate::{GribFile, Grid, Param};

/// m/s to knots
const KN: f64 = 3600.0 / 1852.0;

/// Thermocline and shear are looked for between these depths (m).
pub const THERMO_RANGE: (f64, f64) = (10.0, 300.0);
pub const SHEAR_RANGE: (f64, f64) = (5.0, 300.0);
/// Below this (°C per 10 m) there is no thermocline to speak of: no depth is given.
pub const THERMO_MIN: f64 = 0.1;
/// Below this (knots per 10 m) the shear is not worth a depth.
pub const SHEAR_MIN: f64 = 0.05;

/// The depth (mid-way between two levels) and strength of the steepest step in a profile,
/// within `range`. `step(a, b)` is the change from level a to the next deeper level b,
/// per metre (`None` if either level has no value). Depths shallowest first.
fn steepest(depths: &[f64], range: (f64, f64), step: impl Fn(usize, usize) -> Option<f64>) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for a in 0..depths.len().saturating_sub(1) {
        let b = a + 1;
        let dz = depths[b] - depths[a];
        let mid = (depths[a] + depths[b]) / 2.0;
        if dz <= 0.0 || mid < range.0 || mid > range.1 {
            continue;
        }
        let Some(g) = step(a, b) else { continue };
        let g = g / dz * 10.0;
        if best.is_none_or(|(_, s)| g > s) {
            best = Some((mid, g));
        }
    }
    best
}

/// Thermocline (depth m, strength °C per 10 m) from temperatures (°C) at depths (m, shallowest first).
pub fn thermocline(depths: &[f64], temp: &[f64]) -> Option<(f64, f64)> {
    steepest(depths, THERMO_RANGE, |a, b| (temp[a].is_finite() && temp[b].is_finite()).then(|| (temp[a] - temp[b]).max(0.0)))
}

/// Shear (depth m, strength knots per 10 m) from current components (m/s) at depths.
pub fn shear(depths: &[f64], u: &[f64], v: &[f64]) -> Option<(f64, f64)> {
    steepest(depths, SHEAR_RANGE, |a, b| {
        let ok = [u[a], u[b], v[a], v[b]].iter().all(|x| x.is_finite());
        ok.then(|| ((u[b] - u[a]).hypot(v[b] - v[a])) * KN)
    })
}

/// What a derived map shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Derived {
    ThermoDepth,
    ThermoStrength,
    ShearDepth,
    ShearStrength,
}

/// Grids of a parameter at every depth at time `t` (all on one grid), shallowest first.
fn levels(f: &GribFile, p: Param, t: i64) -> Option<(Grid, Vec<f64>, Vec<Vec<f32>>)> {
    let depths = f.depths(p);
    let mut grid = None;
    let mut out = Vec::with_capacity(depths.len());
    for &d in &depths {
        let (g, v) = f.grid_at_depth(p, d, t)?;
        if grid.is_some_and(|x| x != g) {
            return None;
        }
        grid = Some(g);
        out.push(v);
    }
    Some((grid?, depths, out))
}

/// A map worked out from the depth stack at time `t`: depth (m) or strength. Cells with no
/// thermocline (or no shear) worth a depth are empty in the depth maps; strength is always given.
pub fn derived_grid(f: &GribFile, kind: Derived, t: i64) -> Option<(Grid, Vec<f32>)> {
    let thermo = matches!(kind, Derived::ThermoDepth | Derived::ThermoStrength);
    let depth_map = matches!(kind, Derived::ThermoDepth | Derived::ShearDepth);
    let (g, depths, a) = levels(f, if thermo { Param::SubTemp } else { Param::SubCurrentU }, t)?;
    if depths.len() < 3 {
        return None;
    }
    let b = if thermo { None } else { Some(levels(f, Param::SubCurrentV, t)?.2) };
    let n = g.ni * g.nj;
    let mut out = vec![f32::NAN; n];
    let mut col_a = vec![0.0; depths.len()];
    let mut col_b = vec![0.0; depths.len()];
    for (k, o) in out.iter_mut().enumerate() {
        for (z, lev) in a.iter().enumerate() {
            col_a[z] = lev[k] as f64;
        }
        if !col_a[0].is_finite() {
            continue; // land
        }
        let r = match &b {
            None => thermocline(&depths, &col_a),
            Some(b) => {
                for (z, lev) in b.iter().enumerate() {
                    col_b[z] = lev[k] as f64;
                }
                shear(&depths, &col_a, &col_b)
            }
        };
        let min = if thermo { THERMO_MIN } else { SHEAR_MIN };
        *o = match r {
            Some((d, s)) if depth_map && s >= min => d as f32,
            Some(_) if depth_map => f32::NAN,
            Some((_, s)) => s as f32,
            None => f32::NAN,
        };
    }
    Some((g, out))
}

/// The water at one point and time, level by level.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Profile {
    pub depths: Vec<f64>,
    /// °C, PSU, knots, degrees true (the way it flows): `None` where there is no value.
    pub temp: Vec<Option<f64>>,
    pub sal: Vec<Option<f64>>,
    pub kn: Vec<Option<f64>>,
    pub set: Vec<Option<f64>>,
    /// The thermocline and shear worked out from it: [depth, strength].
    pub thermocline: Option<[f64; 2]>,
    pub shear: Option<[f64; 2]>,
}

fn round(x: f64, k: f64) -> f64 {
    (x * k).round() / k
}

/// The profile at a point and time, or `None` if the file has no depth stack there.
pub fn profile(f: &GribFile, lat: f64, lon: f64, t: i64) -> Option<Profile> {
    let mut depths = f.depths(Param::SubTemp);
    for p in [Param::SubSalinity, Param::SubCurrentU] {
        for d in f.depths(p) {
            if !depths.iter().any(|x| (x - d).abs() < 0.05) {
                depths.push(d);
            }
        }
    }
    depths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if depths.len() < 2 {
        return None;
    }
    let at = |p: Param, d: f64| if f.depths(p).iter().any(|x| (x - d).abs() < 0.05) { f.value_at_depth(p, d, lat, lon, t) } else { None };
    let temp: Vec<Option<f64>> = depths.iter().map(|&d| at(Param::SubTemp, d).map(|k| k - 273.15)).collect();
    let sal: Vec<Option<f64>> = depths.iter().map(|&d| at(Param::SubSalinity, d)).collect();
    let uv: Vec<Option<(f64, f64)>> = depths.iter().map(|&d| Some((at(Param::SubCurrentU, d)?, at(Param::SubCurrentV, d)?))).collect();
    if temp.iter().chain(&sal).all(Option::is_none) && uv.iter().all(Option::is_none) {
        return None;
    }
    let nan = |v: &[Option<f64>]| v.iter().map(|x| x.unwrap_or(f64::NAN)).collect::<Vec<_>>();
    let thermo = thermocline(&depths, &nan(&temp)).filter(|r| r.1 >= THERMO_MIN);
    let u = nan(&uv.iter().map(|x| x.map(|x| x.0)).collect::<Vec<_>>());
    let v = nan(&uv.iter().map(|x| x.map(|x| x.1)).collect::<Vec<_>>());
    let sh = shear(&depths, &u, &v).filter(|r| r.1 >= SHEAR_MIN);
    Some(Profile {
        temp: temp.iter().map(|x| x.map(|x| round(x, 100.0))).collect(),
        sal: sal.iter().map(|x| x.map(|x| round(x, 1000.0))).collect(),
        kn: uv.iter().map(|x| x.map(|(u, v)| round(u.hypot(v) * KN, 100.0))).collect(),
        set: uv.iter().map(|x| x.map(|(u, v)| (u.atan2(v).to_degrees() + 360.0).rem_euclid(360.0).round())).collect(),
        thermocline: thermo.map(|(d, s)| [round(d, 10.0), round(s, 100.0)]),
        shear: sh.map(|(d, s)| [round(d, 10.0), round(s, 100.0)]),
        depths,
    })
}

/// A section along a line: values at `n` points spaced evenly along it, at every depth.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Section {
    /// Distance of each point from the start (nautical miles), and the point [lon, lat].
    pub dist: Vec<f64>,
    pub pts: Vec<[f64; 2]>,
    pub depths: Vec<f64>,
    /// [depth][point]: °C, PSU, knots (`None`: no value, e.g. below the sea floor).
    pub temp: Vec<Vec<Option<f64>>>,
    pub sal: Vec<Vec<Option<f64>>>,
    pub kn: Vec<Vec<Option<f64>>>,
    /// [depth][point]: the way the current flows, degrees true.
    pub set: Vec<Vec<Option<f64>>>,
    /// The thermocline depth along the line (m), where there is one.
    pub thermocline: Vec<Option<f64>>,
}

/// Great-circle distance, nautical miles.
fn nm(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (la1, la2) = (a[1].to_radians(), b[1].to_radians());
    let dla = la2 - la1;
    let dlo = (b[0] - a[0]).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * h.sqrt().asin() * 3440.065
}

/// Points every equal distance along a line of [lon, lat] (straight between its corners).
pub fn along(line: &[[f64; 2]], n: usize) -> Vec<([f64; 2], f64)> {
    let legs: Vec<f64> = line.windows(2).map(|w| nm(w[0], w[1])).collect();
    let total: f64 = legs.iter().sum();
    if line.len() < 2 || total <= 0.0 || n < 2 {
        return line.first().map(|p| vec![(*p, 0.0)]).unwrap_or_default();
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let s = total * k as f64 / (n - 1) as f64;
        let (mut acc, mut i) = (0.0, 0);
        while i + 1 < legs.len() && acc + legs[i] < s {
            acc += legs[i];
            i += 1;
        }
        let f = if legs[i] > 0.0 { ((s - acc) / legs[i]).clamp(0.0, 1.0) } else { 0.0 };
        let (a, b) = (line[i], line[i + 1]);
        out.push(([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f], s));
    }
    out
}

/// A section along `line` ([lon, lat] corners) at time `t`, `n` points.
pub fn section(f: &GribFile, line: &[[f64; 2]], n: usize, t: i64) -> Option<Section> {
    let pts = along(line, n.clamp(2, 400));
    let depths = f.depths(Param::SubTemp);
    if depths.len() < 2 {
        return None;
    }
    let grid_rows = |p: Param, conv: &dyn Fn(f64) -> f64| -> Vec<Vec<Option<f64>>> {
        depths
            .iter()
            .map(|&d| match f.grid_at_depth(p, d, t) {
                Some((g, v)) => pts.iter().map(|(q, _)| g.sample(&v, q[1], q[0]).map(conv)).collect(),
                None => vec![None; pts.len()],
            })
            .collect()
    };
    let temp = grid_rows(Param::SubTemp, &|k| round(k - 273.15, 100.0));
    let sal = grid_rows(Param::SubSalinity, &|s| round(s, 1000.0));
    let u = grid_rows(Param::SubCurrentU, &|x| x);
    let v = grid_rows(Param::SubCurrentV, &|x| x);
    let kn: Vec<Vec<Option<f64>>> =
        u.iter().zip(&v).map(|(ur, vr)| ur.iter().zip(vr).map(|(a, b)| Some(round(a.as_ref()?.hypot(*b.as_ref()?) * KN, 100.0))).collect()).collect();
    let set: Vec<Vec<Option<f64>>> = u
        .iter()
        .zip(&v)
        .map(|(ur, vr)| ur.iter().zip(vr).map(|(a, b)| Some((a.as_ref()?.atan2(*b.as_ref()?).to_degrees() + 360.0).rem_euclid(360.0).round())).collect())
        .collect();
    let thermo = (0..pts.len())
        .map(|x| {
            let col: Vec<f64> = temp.iter().map(|r| r[x].unwrap_or(f64::NAN)).collect();
            thermocline(&depths, &col).filter(|r| r.1 >= THERMO_MIN).map(|r| round(r.0, 10.0))
        })
        .collect();
    Some(Section { dist: pts.iter().map(|p| round(p.1, 100.0)).collect(), pts: pts.iter().map(|p| p.0).collect(), depths, temp, sal, kn, set, thermocline: thermo })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Field;

    #[test]
    fn steepest_steps() {
        let d = [0.5, 10.0, 20.0, 30.0, 50.0, 80.0, 120.0];
        // mixed to 20 m, then a sharp drop between 20 and 30 m, then slow cooling
        let t = [18.0, 18.0, 17.9, 15.9, 15.0, 14.5, 14.0];
        let (z, s) = thermocline(&d, &t).unwrap();
        assert_eq!(z, 25.0);
        assert!((s - 2.0).abs() < 1e-9); // 2 °C over 10 m
                                         // the top step (0.5 to 10 m, mid 5 m) is above the range looked at
        let t2 = [25.0, 18.0, 17.95, 17.9, 17.85, 17.8, 17.75];
        assert!(thermocline(&d, &t2).unwrap().1 < THERMO_MIN);
        // no values below the sea floor
        let t3 = [18.0, 18.0, 17.0, f64::NAN, f64::NAN, f64::NAN, f64::NAN];
        assert_eq!(thermocline(&d, &t3).unwrap().0, 15.0);
        // shear: 0.5 m/s eastward over the top 30 m, still below
        let u = [0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0];
        let v = [0.0; 7];
        let (z, s) = shear(&d, &u, &v).unwrap();
        assert_eq!(z, 25.0);
        assert!((s - 0.5 * KN).abs() < 1e-9);
    }

    fn field(p: Param, number: u8, depth: f64, t: i64, v: f32) -> Field {
        Field {
            param: p,
            discipline: 10,
            category: 3,
            number,
            surface_type: 160,
            surface_value: depth,
            centre: 255,
            ref_time: 0,
            valid_time: t,
            member: None,
            grid: Grid::regular(2, 2, -41.0, 174.0, 1.0, 1.0),
            values: vec![v; 4],
            local: None,
        }
    }

    #[test]
    fn depths_in_a_file() {
        let mut fs = Vec::new();
        for (d, temp) in [(0.5, 18.0), (25.0, 17.0), (50.0, 13.0), (100.0, 12.0)] {
            for t in [0, 21600] {
                fs.push(field(Param::SubTemp, 0, d, t, temp + 273.15 + if t > 0 { 0.5 } else { 0.0 }));
            }
        }
        let f = GribFile::from_fields(fs);
        assert_eq!(f.depths(Param::SubTemp), vec![0.5, 25.0, 50.0, 100.0]);
        // the plain series is the shallowest depth
        assert_eq!(f.series(Param::SubTemp).len(), 2);
        assert!(f.series(Param::SubTemp).iter().all(|x| x.surface_value == 0.5));
        // the nearest depth, between times
        let (_, v) = f.grid_at_depth(Param::SubTemp, 45.0, 10800).unwrap();
        assert!((v[0] as f64 - (13.25 + 273.15)).abs() < 1e-3);
        assert!((f.value_at_depth(Param::SubTemp, 30.0, -40.5, 174.5, 0).unwrap() - (17.0 + 273.15)).abs() < 1e-3);
        // thermocline map: between 25 and 50 m (mid 37.5), 4 °C over 25 m
        let (_, m) = derived_grid(&f, Derived::ThermoDepth, 0).unwrap();
        assert_eq!(m[0], 37.5);
        let (_, s) = derived_grid(&f, Derived::ThermoStrength, 0).unwrap();
        assert!((s[0] - 1.6).abs() < 1e-4);
        // the profile and a section
        let p = profile(&f, -40.5, 174.5, 0).unwrap();
        assert_eq!(p.depths.len(), 4);
        assert_eq!(p.temp[2], Some(13.0));
        assert_eq!(p.thermocline, Some([37.5, 1.6]));
        let sct = section(&f, &[[174.2, -40.8], [174.8, -40.2]], 5, 0).unwrap();
        assert_eq!(sct.dist.len(), 5);
        assert_eq!(sct.temp[1][2], Some(17.0));
        assert_eq!(sct.thermocline[0], Some(37.5));
    }

    #[test]
    fn spacing_along_a_line() {
        let p = along(&[[174.0, -41.0], [174.0, -40.0], [175.0, -40.0]], 3);
        assert_eq!(p.len(), 3);
        assert!((p[0].1).abs() < 1e-9);
        // 60 NM north, then about 46 NM east: the middle point is on the first leg
        assert!((p[1].0[0] - 174.0).abs() < 1e-9 && p[1].0[1] > -41.0 && p[1].0[1] < -40.0);
        assert!((p[2].0[0] - 175.0).abs() < 1e-9);
    }
}
