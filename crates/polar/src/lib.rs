//! Boat polars.
//!
//! Reads the common table format used by qtVlm, OpenCPN and others:
//!
//! ```text
//! TWA\TWS;4;6;8;...      (qtVlm .csv: ';' separated)
//! TWA\TWS<TAB>4<TAB>6... (.pol: tab or space separated)
//! 40;1.82;2.74;...
//! ```
//!
//! Boat speed is found by bilinear interpolation in TWA and TWS. Below the
//! smallest TWS column speed scales linearly to zero at 0 kn; above the
//! largest it is held at the last column (no extrapolation upwards).

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum PolarError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("line {line}: {msg}")]
    Parse { line: usize, msg: String },
    #[error("polar is empty")]
    Empty,
}

#[derive(Debug, Clone)]
pub struct Polar {
    pub name: String,
    /// True wind speeds (kn), ascending.
    pub tws: Vec<f64>,
    /// True wind angles (deg, 0..180), ascending.
    pub twa: Vec<f64>,
    /// speeds[i][j] = boat speed at twa[i], tws[j].
    pub speeds: Vec<Vec<f64>>,
}

/// Best VMG result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vmg {
    pub twa: f64,
    pub bsp: f64,
    pub vmg: f64,
}

impl Polar {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, PolarError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Self::parse(&name, &text)
    }

    /// Parse a polar table; the separator (';', tab, ',' or spaces) is detected per file.
    pub fn parse(name: &str, text: &str) -> Result<Self, PolarError> {
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l.trim())).filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));

        let (hline, header) = lines.next().ok_or(PolarError::Empty)?;
        let sep = detect_sep(header);
        let tws: Vec<f64> = split(header, sep).skip(1).map(|s| num(s, hline)).collect::<Result<_, _>>()?;
        if tws.is_empty() {
            return Err(PolarError::Empty);
        }
        if tws.windows(2).any(|w| w[1] <= w[0]) {
            return Err(PolarError::Parse { line: hline, msg: "wind speeds must go up from left to right".into() });
        }

        let mut rows: Vec<(f64, Vec<f64>)> = Vec::new();
        for (ln, l) in lines {
            let mut it = split(l, sep);
            let a = num(it.next().unwrap_or(""), ln)?;
            let v: Vec<f64> = it.map(|s| num(s, ln)).collect::<Result<_, _>>()?;
            if v.len() != tws.len() {
                return Err(PolarError::Parse { line: ln, msg: format!("expected {} speeds, found {}", tws.len(), v.len()) });
            }
            rows.push((a, v));
        }
        if rows.is_empty() {
            return Err(PolarError::Empty);
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (twa, speeds) = rows.into_iter().unzip();
        Ok(Polar { name: name.to_string(), tws, twa, speeds })
    }

    /// Boat speed (kn) at a true wind angle (deg, sign ignored) and speed (kn).
    pub fn bsp(&self, twa: f64, tws: f64) -> f64 {
        let a = nav_core::norm180(twa).abs();
        // a table with a 0 kn column (a power boat's) gives its speed in a calm; otherwise no wind, no speed
        if tws <= 0.0 && self.tws[0] > 0.0 {
            return 0.0;
        }
        let tws = tws.max(0.0);
        let (i0, i1, fa) = bracket(&self.twa, a);
        let row = |i: usize| -> f64 {
            let r = &self.speeds[i];
            if tws < self.tws[0] && self.tws[0] > 0.0 {
                r[0] * tws / self.tws[0]
            } else {
                let (j0, j1, fs) = bracket(&self.tws, tws);
                lerp(r[j0], r[j1], fs)
            }
        };
        lerp(row(i0), row(i1), fa)
    }

    /// Best upwind VMG at a TWS, searched at 0.5 deg steps from `min_twa` to 90.
    pub fn best_vmg_up(&self, tws: f64, min_twa: f64) -> Vmg {
        self.search(tws, min_twa, 90.0, 1.0)
    }

    /// Best downwind VMG (returned as a positive number) at a TWS.
    pub fn best_vmg_down(&self, tws: f64) -> Vmg {
        self.search(tws, 90.0, 180.0, -1.0)
    }

    fn search(&self, tws: f64, from: f64, to: f64, sign: f64) -> Vmg {
        let mut best = Vmg { twa: from, bsp: 0.0, vmg: f64::MIN };
        let mut a = from;
        while a <= to + 1e-9 {
            let b = self.bsp(a, tws);
            let v = sign * b * a.to_radians().cos();
            if v > best.vmg {
                best = Vmg { twa: a, bsp: b, vmg: v };
            }
            a += 0.5;
        }
        best
    }

    /// A polar from a table (the polar editor's): wind speeds strictly going up, wind angles from 0
    /// to 180 strictly going up, one row of speeds per angle with one speed (0 or more) per wind speed.
    pub fn from_table(name: &str, tws: Vec<f64>, twa: Vec<f64>, speeds: Vec<Vec<f64>>) -> Result<Self, String> {
        if tws.is_empty() || twa.is_empty() {
            return Err("the table needs at least one wind speed and one wind angle".into());
        }
        if tws.iter().any(|v| !v.is_finite() || *v < 0.0) || tws.windows(2).any(|w| w[1] <= w[0]) {
            return Err("wind speeds must be 0 or more and go up from left to right".into());
        }
        if twa.iter().any(|v| !v.is_finite() || !(0.0..=180.0).contains(v)) || twa.windows(2).any(|w| w[1] <= w[0]) {
            return Err("wind angles must be 0 to 180 and go up from top to bottom".into());
        }
        if speeds.len() != twa.len() || speeds.iter().any(|r| r.len() != tws.len()) {
            return Err("every wind angle needs a speed for every wind speed".into());
        }
        if speeds.iter().flatten().any(|v| !v.is_finite() || *v < 0.0) {
            return Err("boat speeds must be numbers, 0 or more".into());
        }
        Ok(Polar { name: name.to_string(), tws, twa, speeds })
    }

    /// The table as qtVlm's .csv (`TWA\TWS;4;6;…`), which OpenCPN, Expedition imports and TweedPlot read.
    pub fn to_csv(&self) -> String {
        let f = |v: f64| {
            let s = format!("{v:.3}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            if s.is_empty() || s == "-0" {
                "0".to_string()
            } else {
                s.to_string()
            }
        };
        let mut out = String::from("TWA\\TWS");
        for t in &self.tws {
            out.push(';');
            out.push_str(&f(*t));
        }
        out.push('\n');
        for (a, row) in self.twa.iter().zip(&self.speeds) {
            out.push_str(&f(*a));
            for v in row {
                out.push(';');
                out.push_str(&f(*v));
            }
            out.push('\n');
        }
        out
    }

    /// Scale every speed by `factor` (e.g. 0.9 for 90 % efficiency).
    pub fn scaled(&self, factor: f64) -> Polar {
        let mut p = self.clone();
        for r in &mut p.speeds {
            for v in r {
                *v *= factor;
            }
        }
        p
    }
}

fn detect_sep(header: &str) -> Option<char> {
    if header.contains(';') {
        Some(';')
    } else if header.contains('\t') {
        Some('\t')
    } else if header.contains(',') {
        Some(',')
    } else {
        None // whitespace
    }
}

fn split(l: &str, sep: Option<char>) -> Box<dyn Iterator<Item = &str> + '_> {
    match sep {
        Some(c) => Box::new(l.split(c).map(str::trim).filter(|s| !s.is_empty())),
        None => Box::new(l.split_whitespace()),
    }
}

fn num(s: &str, line: usize) -> Result<f64, PolarError> {
    s.replace(',', ".").parse::<f64>().map_err(|_| PolarError::Parse { line, msg: format!("not a number: {s:?}") })
}

/// Indices either side of x in an ascending vector and the fraction between them.
fn bracket(v: &[f64], x: f64) -> (usize, usize, f64) {
    if x <= v[0] {
        return (0, 0, 0.0);
    }
    let last = v.len() - 1;
    if x >= v[last] {
        return (last, last, 0.0);
    }
    let i = v.partition_point(|&e| e <= x) - 1;
    let f = (x - v[i]) / (v[i + 1] - v[i]);
    (i, i + 1, f)
}

fn lerp(a: f64, b: f64, f: f64) -> f64 {
    a + (b - a) * f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_polar_moves_in_a_calm() {
        let p = Polar::parse(
            "power",
            "TWA\\TWS;0;10;15;20;25;30\n0;22;22;18;12;8;5\n45;22;22;19;14;9;6\n90;22;22;21;17;12;8\n135;22;22;22;19;15;10\n180;22;22;22;20;16;12\n",
        )
        .unwrap();
        assert_eq!(p.bsp(30.0, 0.0), 22.0);
        assert_eq!(p.bsp(0.0, 20.0), 12.0);
        assert!((p.bsp(90.0, 22.5) - 14.5).abs() < 1e-9);
    }

    // Morgane median polar (Chris, 2026), first rows trimmed for the test.
    const MORGANE_CSV: &str = "TWA\\TWS;4;6;8;10;12;14;16;20;25;30
0;0.00;0.00;0.00;0.00;0.00;0.00;0.00;0.00;0.00;0.00
30;0.00;0.00;0.00;0.00;0.00;0.00;0.00;0.00;0.00;0.00
40;1.82;2.74;3.47;4.03;4.03;4.03;4.03;4.03;4.18;2.20
45;1.84;2.78;3.51;4.09;4.09;4.09;4.09;4.09;4.37;2.22
50;1.86;2.81;3.56;4.14;4.23;4.23;4.23;4.23;4.72;2.39
60;1.90;2.88;3.64;4.23;4.30;4.39;4.42;4.58;5.01;2.93
90;1.99;3.00;3.80;4.42;4.70;5.40;5.85;5.97;5.25;5.56
135;1.97;2.97;3.76;4.37;4.71;5.13;5.46;5.58;5.63;5.76
180;2.05;3.10;3.92;4.56;4.67;4.87;5.19;5.47;5.83;6.27
";

    #[test]
    fn a_table_saves_and_reads_back() {
        let p = Polar::parse("m", MORGANE_CSV).unwrap();
        let again = Polar::parse("m", &p.to_csv()).unwrap();
        assert_eq!(again.tws, p.tws);
        assert_eq!(again.twa, p.twa);
        assert_eq!(again.speeds, p.speeds);
        assert!(p.to_csv().starts_with("TWA\\TWS;4;6;8;"));
        assert!(Polar::from_table("x", vec![6.0, 4.0], vec![40.0], vec![vec![1.0, 2.0]]).is_err());
        assert!(Polar::from_table("x", vec![4.0], vec![40.0, 40.0], vec![vec![1.0], vec![2.0]]).is_err());
        assert!(Polar::from_table("x", vec![4.0], vec![40.0], vec![vec![-1.0]]).is_err());
        assert!(Polar::from_table("x", vec![4.0, 6.0], vec![40.0, 90.0], vec![vec![1.0, 2.0], vec![1.5, 2.5]]).is_ok());
    }

    #[test]
    fn parses_qtvlm_csv() {
        let p = Polar::parse("Morgane_median", MORGANE_CSV).unwrap();
        assert_eq!(p.tws.len(), 10);
        assert_eq!(p.twa.len(), 9);
        assert_eq!(p.bsp(90.0, 16.0), 5.85);
    }

    #[test]
    fn parses_tab_pol() {
        let pol = MORGANE_CSV.replace(';', "\t").replace('\n', "\r\n");
        let p = Polar::parse("x", &pol).unwrap();
        assert_eq!(p.bsp(135.0, 20.0), 5.58);
    }

    #[test]
    fn interpolates_both_ways() {
        let p = Polar::parse("m", MORGANE_CSV).unwrap();
        // halfway between 10 and 12 kn at 90 deg
        assert!((p.bsp(90.0, 11.0) - 4.56).abs() < 1e-9);
        // halfway between 45 and 50 deg at 10 kn
        assert!((p.bsp(47.5, 10.0) - 4.115).abs() < 1e-9);
        // negative angle (port tack) is the same as starboard
        assert_eq!(p.bsp(-90.0, 16.0), p.bsp(90.0, 16.0));
    }

    #[test]
    fn light_and_strong_wind_edges() {
        let p = Polar::parse("m", MORGANE_CSV).unwrap();
        assert!((p.bsp(90.0, 2.0) - 0.995).abs() < 1e-9);
        assert_eq!(p.bsp(90.0, 40.0), 5.56);
        assert_eq!(p.bsp(90.0, 0.0), 0.0);
    }

    #[test]
    fn best_vmg() {
        let p = Polar::parse("m", MORGANE_CSV).unwrap();
        let up = p.best_vmg_up(12.0, 40.0);
        assert!(up.twa >= 40.0 && up.twa <= 60.0, "{up:?}");
        let down = p.best_vmg_down(12.0);
        assert!(down.twa >= 135.0, "{down:?}");
        assert!(down.vmg > 0.0);
    }

    #[test]
    fn rejects_ragged_rows() {
        let bad = "TWA;4;6\n40;1.0\n";
        assert!(matches!(Polar::parse("b", bad), Err(PolarError::Parse { line: 2, .. })));
    }
}
