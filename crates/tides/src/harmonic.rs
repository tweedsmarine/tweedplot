//! Tide predictions from LINZ's harmonic constituents ("NZ Tide Constituents", one file per
//! gauge, Foreman's analysis output): the tide at any time, past or future, so soundings from
//! years without LINZ daily tables can still be reduced to chart datum.
//!
//! h(t) = Z0 + Σ f·A·cos(V + u − G), with Foreman's nodal corrections (f, u from the
//! satellites, latitude dependent) and astronomical arguments (V), as in t_tide and UTide.
//! LINZ's phases are referred to the file's time zone (180° = NZST, UTC+12), so the
//! arguments are taken at the time in that zone. Z0 is the mean level above chart datum:
//! LINZ's daily tables are made from these same constituents ("Based on constituent set with
//! reference date ..."), and the tests check them against each other.

use crate::foreman::{CONSTITUENTS, SATELLITES, SHALLOW};
use crate::Extreme;

/// One gauge's constituents.
#[derive(Debug, Clone)]
pub struct Harmonics {
    /// LINZ's station number ("077").
    pub number: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Hours east of UTC that the phases are referred to (12 for NZST).
    pub zone_h: f64,
    /// Mean level above chart datum (m).
    pub z0: f64,
    /// (index in the constituent table, amplitude m, phase lag degrees).
    pub cons: Vec<(usize, f64, f64)>,
    /// Constituents in the file that aren't in the table (left out).
    pub unknown: Vec<String>,
}

/// Degrees and minutes as LINZ writes them (1416 = 41°16'), south and east for New Zealand;
/// longitudes past 180° (written as 18xxx) are west.
fn dm(v: &str) -> Option<f64> {
    let n: u32 = v.trim().trim_end_matches(',').parse().ok()?;
    Some((n / 100) as f64 + (n % 100) as f64 / 60.0)
}

/// Read one LINZ constituents file.
pub fn parse(text: &str) -> Option<Harmonics> {
    let mut lines = text.lines();
    let first = lines.next()?;
    // "NELSON           Units in CM"
    let (name, units) = match first.find("Units in") {
        Some(i) => (first[..i].trim().to_string(), first[i + 8..].trim().to_ascii_uppercase()),
        None => (first.trim().to_string(), "CM".to_string()),
    };
    let scale = if units.starts_with('M') && !units.starts_with("MM") {
        1.0
    } else if units.starts_with("MM") {
        0.001
    } else {
        0.01
    };
    let mut h = Harmonics { number: String::new(), name, lat: 0.0, lon: 0.0, zone_h: 12.0, z0: 0.0, cons: Vec::new(), unknown: Vec::new() };
    let mut have_pos = false;
    for line in lines {
        let l = line.trim();
        if l.starts_with("STATION") {
            // "STATION 077 TIME ZONE = 180, LATITUDE = 1416, LONGITUDE = 17316"
            let flat = l.replace([',', '='], " ");
            let w: Vec<&str> = flat.split_whitespace().collect();
            h.number = w.get(1).unwrap_or(&"").to_string();
            let after = |key: &str| w.iter().position(|x| *x == key).and_then(|i| w.get(i + 1)).copied();
            if let Some(z) = w.iter().position(|x| *x == "ZONE").and_then(|i| w.get(i + 1)).and_then(|z| z.parse::<f64>().ok()) {
                h.zone_h = z / 15.0;
            }
            if let (Some(la), Some(lo)) = (after("LATITUDE").and_then(dm), after("LONGITUDE").and_then(dm)) {
                h.lat = -la;
                h.lon = if lo > 180.0 { lo - 360.0 } else { lo };
                have_pos = true;
            }
            continue;
        }
        // "  14 M2      .08051140   077 0105/1225 133.5880 274.97"; the columns are fixed width,
        // so a large amplitude can run into the months ("0106/1219134.2423"): A and G are read
        // from after the second MMYY
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() < 5 || w[0].parse::<u32>().is_err() {
            continue;
        }
        let Some(slash) = l.find('/') else { continue };
        let rest: Vec<&str> = l.get(slash + 5..).unwrap_or("").split_whitespace().collect();
        let [a, g] = rest[..] else { continue };
        let (Ok(a), Ok(g)) = (a.parse::<f64>(), g.parse::<f64>()) else { continue };
        let name = w[1];
        if name.eq_ignore_ascii_case("Z0") {
            h.z0 = a * scale;
            continue;
        }
        match CONSTITUENTS.iter().position(|c| c.name.eq_ignore_ascii_case(name)) {
            Some(i) => h.cons.push((i, a * scale, g)),
            None => h.unknown.push(name.to_string()),
        }
    }
    (have_pos && !h.cons.is_empty()).then_some(h)
}

/// Days on the proleptic Gregorian calendar (1 Jan of year 1 = day 1), as UTide counts them.
fn ordinal_days(unix: f64) -> f64 {
    719_163.0 + unix / 86_400.0
}

/// tau, s, h, p, N', p' (cycles) at `jd` (see [`ordinal_days`]): t_tide's `t_astron`.
fn astro(jd: f64) -> [f64; 6] {
    const C: [[f64; 4]; 5] = [
        [270.434164, 13.1763965268, -0.0000850, 0.000000039],
        [279.696678, 0.9856473354, 0.00002267, 0.000000000],
        [334.329556, 0.1114040803, -0.0007739, -0.00000026],
        [-259.183275, 0.0529539222, -0.0001557, -0.000000050],
        [281.220844, 0.0000470684, 0.0000339, 0.000000070],
    ];
    let d = jd - 693_595.5;
    let dd = d / 10_000.0;
    let args = [1.0, d, dd * dd, dd * dd * dd];
    let mut a = [0.0; 5];
    for (k, c) in C.iter().enumerate() {
        a[k] = ((c[0] * args[0] + c[1] * args[1] + c[2] * args[2] + c[3] * args[3]) / 360.0) % 1.0;
    }
    let tau = jd.rem_euclid(1.0) + a[1] - a[0];
    [tau, a[0], a[1], a[2], a[3], a[4]]
}

/// Nodal amplitude factors and phase corrections (cycles) for every constituent at `jd`.
fn nodal(jd: f64, lat: f64) -> (Vec<f64>, Vec<f64>) {
    let ast = astro(jd);
    let lat = if lat.abs() < 5.0 { 5f64.copysign(lat) } else { lat };
    let slat = lat.to_radians().sin();
    let n = CONSTITUENTS.len();
    let mut re = vec![1.0; n];
    let mut im = vec![0.0; n];
    for &(ic, dd, ph, amp, latfac) in SATELLITES.iter() {
        let r = match latfac {
            1 => amp * 0.36309 * (1.0 - 5.0 * slat * slat) / slat,
            2 => amp * 2.59808 * slat,
            _ => amp,
        };
        let uu = ((dd[0] * ast[3] + dd[1] * ast[4] + dd[2] * ast[5] + ph) % 1.0) * std::f64::consts::TAU;
        re[ic as usize] += r * uu.cos();
        im[ic as usize] += r * uu.sin();
    }
    let mut f: Vec<f64> = (0..n).map(|i| re[i].hypot(im[i])).collect();
    let mut u: Vec<f64> = (0..n).map(|i| im[i].atan2(re[i]) / std::f64::consts::TAU).collect();
    for (k, c) in CONSTITUENTS.iter().enumerate() {
        if let Some((start, count)) = c.shallow {
            let terms = &SHALLOW[start as usize..start as usize + count as usize];
            f[k] = terms.iter().map(|&(j, coef)| f[j as usize].powf(coef.abs())).product();
            u[k] = terms.iter().map(|&(j, coef)| u[j as usize] * coef).sum();
        }
    }
    (f, u)
}

/// Astronomical arguments (cycles) for every constituent at `jd`.
fn arguments(jd: f64) -> Vec<f64> {
    let ast = astro(jd);
    let mut v: Vec<f64> = CONSTITUENTS.iter().map(|c| (c.doodson.iter().zip(ast.iter()).map(|(d, a)| d * a).sum::<f64>() + c.semi) % 1.0).collect();
    for (k, c) in CONSTITUENTS.iter().enumerate() {
        if let Some((start, count)) = c.shallow {
            v[k] = SHALLOW[start as usize..start as usize + count as usize].iter().map(|&(j, coef)| v[j as usize] * coef).sum();
        }
    }
    v
}

impl Harmonics {
    /// The arguments are taken at the time in the phases' zone.
    fn jd(&self, unix: f64) -> f64 {
        ordinal_days(unix + self.zone_h * 3600.0)
    }

    /// Height above chart datum (m) at Unix time `t` (seconds, UTC).
    pub fn height(&self, t: i64) -> f64 {
        let jd = self.jd(t as f64);
        let (f, u) = nodal(jd, self.lat);
        let v = arguments(jd);
        self.sum(&f, &u, &v)
    }

    fn sum(&self, f: &[f64], u: &[f64], v: &[f64]) -> f64 {
        self.z0 + self.cons.iter().map(|&(i, a, g)| f[i] * a * ((v[i] + u[i] - g / 360.0) * std::f64::consts::TAU).cos()).sum::<f64>()
    }

    /// High and low waters from `t0` to `t1`: the curve every 6 minutes, each turn refined
    /// with a parabola (the nodal corrections, which change over years, once a day).
    pub fn extremes(&self, t0: i64, t1: i64) -> Vec<Extreme> {
        const STEP: i64 = 360;
        let mut out = Vec::new();
        let (mut f, mut u) = (Vec::new(), Vec::new());
        let mut nodal_day = i64::MIN;
        let mut h = |t: i64| -> f64 {
            let day = t.div_euclid(86_400);
            if day != nodal_day {
                (f, u) = nodal(self.jd((day * 86_400 + 43_200) as f64), self.lat);
                nodal_day = day;
            }
            self.sum(&f, &u, &arguments(self.jd(t as f64)))
        };
        let (mut a, mut b) = (h(t0 - STEP), h(t0));
        let mut t = t0;
        while t <= t1 {
            let c = h(t + STEP);
            let high = b > a && b >= c;
            if high || (b < a && b <= c) {
                let den = a - 2.0 * b + c;
                let off = if den != 0.0 { 0.5 * (a - c) / den } else { 0.0 };
                out.push(Extreme { t: t + (off * STEP as f64).round() as i64, h: Some(b - 0.25 * (a - c) * off), high });
            }
            (a, b) = (b, c);
            t += STEP;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)).unwrap()
    }

    #[test]
    fn reads_a_constituents_file() {
        let h = parse(&fixture("077NELSON_Harm_2005-2025.txt")).unwrap();
        assert_eq!((h.number.as_str(), h.name.as_str()), ("077", "NELSON"));
        // LINZ's Nelson file says 1416 (14°16'S) for 41°16'S: the station list's position is used
        // instead when the gauge is matched to a station (TideDb)
        assert!((h.lat + 14.2667).abs() < 0.001 && (h.lon - 173.2667).abs() < 0.001);
        assert_eq!(h.zone_h, 12.0);
        assert!((h.z0 - 2.401055).abs() < 1e-9);
        assert_eq!(h.cons.len(), 30);
        assert!(h.unknown.is_empty(), "{:?}", h.unknown);
        // each constituent's frequency in the file is the table's
        let text = fixture("077NELSON_Harm_2005-2025.txt");
        for l in text.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            if w.len() > 4 && w[0].parse::<u32>().is_ok() && w[1] != "Z0" {
                let c = CONSTITUENTS.iter().find(|c| c.name == w[1]).unwrap();
                assert!((c.freq - w[2].parse::<f64>().unwrap()).abs() < 1e-7, "{}", w[1]);
            }
        }
    }

    /// Nodal factors and arguments as UTide gives them (UTide 0.4.0, lat -41.27).
    #[test]
    fn reads_an_amplitude_run_into_the_dates() {
        let text = " Manawatu River        Units in CM\n STATION  148   TIME ZONE =  180, LATITUDE =  4028, LONGITUDE = 17514\n\n  NO NAME    FREQUENCY   STN  M-Y/ M-Y      A       G\n   1 Z0      .00000000   148 0106/1219134.2423    .00\n   2 O1      .03873065   148 0106/1219  1.3720  41.60\n";
        let h = parse(text).unwrap();
        assert!((h.z0 - 1.342423).abs() < 1e-9);
        assert_eq!(h.cons.len(), 1);
        assert!((h.cons[0].1 - 0.01372).abs() < 1e-9 && (h.cons[0].2 - 41.6).abs() < 1e-9);
    }

    #[test]
    fn matches_utide() {
        let jd = 739_677.0; // UTide's day number for 2026-03-02 00:00 (in the phases' zone)
        let (f, u) = nodal(jd, -41.27);
        let v = arguments(jd);
        let idx = |n: &str| CONSTITUENTS.iter().position(|c| c.name == n).unwrap();
        for (n, ef, eu, ev) in [
            ("M2", 0.9642008139724291, 0.002354063312345526, 0.11808851701161416),
            ("K1", 1.1072685736556, 0.007736421212204985, 0.19390234898223468),
            ("O1", 1.1806694754931777, -0.009305077328991093, -0.07581383197062053),
            ("N2", 0.9688801018775306, 0.00208825539180721, 0.922064379590493),
            ("M4", 0.9296832096650948, 0.004708126624691052, 0.23617703402322832),
            ("2MK5", 1.0294090015174298, 0.012444547836896037, 0.430079383005463),
            ("L2", 1.2767601075113648, -0.025086492092179537, 0.8141126544327353),
        ] {
            let i = idx(n);
            assert!((f[i] - ef).abs() < 1e-9, "{n} f {} {ef}", f[i]);
            assert!((u[i] - eu).abs() < 1e-9, "{n} u {} {eu}", u[i]);
            assert!(((v[i] - ev + 0.5).rem_euclid(1.0) - 0.5).abs() < 1e-9, "{n} v {} {ev}", v[i]);
        }
    }

    /// LINZ's own 2026 tables for Nelson and Lyttelton come from these constituents: every high
    /// and low water within a few minutes, and the heights within LINZ's rounding (0.1 m).
    #[test]
    fn agrees_with_the_linz_daily_tables() {
        for (harm, table) in [("077NELSON_Harm_2005-2025.txt", "Nelson 2026.csv"), ("107LYTTELTON_Harm_2006-2025.txt", "Lyttelton 2026.csv")] {
            let mut h = parse(&fixture(harm)).unwrap();
            let (head, linz) = crate::parse_daily(&fixture(table)).unwrap();
            h.lat = head.lat;
            let (t0, t1) = (linz[1].t, linz[linz.len() - 2].t);
            let ours = h.extremes(t0 - 86_400, t1 + 86_400);
            let (mut worst_t, mut worst_h, mut n) = (0i64, 0f64, 0);
            for e in &linz[1..linz.len() - 1] {
                let o = ours.iter().filter(|o| o.high == e.high).min_by_key(|o| (o.t - e.t).abs()).unwrap();
                worst_t = worst_t.max((o.t - e.t).abs());
                worst_h = worst_h.max((o.h.unwrap() - e.h.unwrap()).abs());
                n += 1;
            }
            assert!(n >= 2, "{table}: {n}");
            assert!(worst_t <= 5 * 60, "{table}: times out by up to {worst_t} s");
            assert!(worst_h <= 0.11, "{table}: heights out by up to {worst_h} m");
        }
    }
}
