//! Tidal streams from the LINZ charts: the tidal diamonds (S-57 TS_PAD, "tidal stream
//! panel data"), each a table of the stream's direction and rate for every hour from
//! six hours before to six hours after high water at a reference port; and the
//! charted flood and ebb arrows (TS_FEB). The app works out the stream now from the
//! reference port's tide predictions.
//!
//! A panel's `ts_tsp` reads like
//! `6366,Port Taranaki,HW,158,0.7,161,1.3,...` : the reference port's LINZ number and
//! name, HW (or LW) as the reference, then 13 direction (°T) and rate (kn) pairs.

use crate::geom::Geom;
use crate::Kind;

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct TidalPanel {
    pub lon: f64,
    pub lat: f64,
    /// The diamond's letter on the chart.
    pub name: String,
    pub ref_no: String,
    pub ref_name: String,
    /// "HW" or "LW": what the hours count from.
    pub rel: String,
    /// Direction (°T, the way the stream runs) and rate (kn) for each hour, -6 to +6.
    pub hours: Vec<(f64, f64)>,
    /// Scale band of the chart it came from (bigger: larger scale).
    pub band: u8,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct TidalArrow {
    pub lon: f64,
    pub lat: f64,
    /// 1 flood, 2 ebb, 3 other (S-57 CAT_TS).
    pub cat: u8,
    /// Direction, °T.
    pub dir: f64,
    /// Rate, kn (when charted).
    pub rate: Option<f64>,
    pub band: u8,
}

/// Parse a TS_TSP value; None when it doesn't hold 13 hours.
pub fn parse_tsp(v: &str) -> Option<(String, String, String, Vec<(f64, f64)>)> {
    let parts: Vec<&str> = v.split(',').map(str::trim).collect();
    let i = parts.iter().position(|p| p.eq_ignore_ascii_case("HW") || p.eq_ignore_ascii_case("LW"))?;
    let (no, name) = match i {
        0 => (String::new(), String::new()),
        1 => (String::new(), parts[0].to_string()),
        _ => (parts[0].to_string(), parts[1..i].join(", ")),
    };
    let nums: Vec<f64> = parts[i + 1..].iter().filter_map(|p| p.parse().ok()).collect();
    if nums.len() < 26 {
        return None;
    }
    let hours = nums.chunks(2).take(13).map(|c| (c[0], c[1])).collect();
    Some((no, name, parts[i].to_ascii_uppercase(), hours))
}

impl crate::ChartStore {
    /// The tidal diamonds and flood/ebb arrows in `bbox` [w, s, e, n]. A diamond charted
    /// in several scale bands comes once, from the largest-scale chart.
    pub fn tidal_streams(&self, bbox: [f64; 4]) -> (Vec<TidalPanel>, Vec<TidalArrow>) {
        let mut panels: Vec<TidalPanel> = Vec::new();
        let mut arrows: Vec<TidalArrow> = Vec::new();
        for t in self.tables.iter().filter(|t| t.kind == Kind::Feature) {
            let code = t.class.map(|c| c.code).unwrap_or("");
            match code {
                "TS_PAD" => self.query_text(t, bbox, &["objnam", "ts_tsp"], |row| {
                    let Geom::Points(pts) = &row.geom else { return };
                    let Some(p) = pts.first() else { return };
                    let Some((ref_no, ref_name, rel, hours)) = row.vals[1].as_deref().and_then(parse_tsp) else { return };
                    panels.push(TidalPanel {
                        lon: p[0], lat: p[1],
                        name: row.vals[0].clone().unwrap_or_default().trim().to_string(),
                        ref_no, ref_name, rel, hours, band: t.band,
                    });
                }),
                "TS_FEB" => self.query_text(t, bbox, &["cat_ts", "orient", "curvel"], |row| {
                    let Geom::Points(pts) = &row.geom else { return };
                    let Some(p) = pts.first() else { return };
                    let num = |i: usize| row.vals[i].as_deref().and_then(|s| s.trim().parse::<f64>().ok());
                    let Some(dir) = num(1) else { return };
                    arrows.push(TidalArrow { lon: p[0], lat: p[1], cat: num(0).unwrap_or(3.0) as u8, dir, rate: num(2), band: t.band });
                }),
                _ => {}
            }
        }
        // the same diamond in several bands: keep the largest scale
        panels.sort_by_key(|p| std::cmp::Reverse(p.band));
        let mut kept: Vec<TidalPanel> = Vec::new();
        for p in panels {
            let near = |q: &TidalPanel| {
                let k = q.lat.to_radians().cos();
                (((q.lon - p.lon) * k).powi(2) + (q.lat - p.lat).powi(2)).sqrt() * 111_320.0 < 800.0
            };
            if !kept.iter().any(|q| near(q) && q.name == p.name) {
                kept.push(p);
            }
        }
        arrows.sort_by_key(|a| std::cmp::Reverse(a.band));
        let mut karrows: Vec<TidalArrow> = Vec::new();
        for a in arrows {
            let k = a.lat.to_radians().cos();
            if !karrows.iter().any(|q| (((q.lon - a.lon) * k).powi(2) + (q.lat - a.lat).powi(2)).sqrt() * 111_320.0 < 300.0 && (q.dir - a.dir).abs() < 20.0) {
                karrows.push(a);
            }
        }
        (kept, karrows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linz_panel() {
        let v = "6366,Port Taranaki,HW,158,0.7,161,1.3,167,2.1,174,2.4,181,2.3,193,1.5,351,0.5,000,1.4,353,2.2,351,2.2,351,1.9,007,1.2,021,0.7";
        let (no, name, rel, h) = parse_tsp(v).unwrap();
        assert_eq!((no.as_str(), name.as_str(), rel.as_str()), ("6366", "Port Taranaki", "HW"));
        assert_eq!(h.len(), 13);
        assert_eq!(h[0], (158.0, 0.7));
        assert_eq!(h[7], (0.0, 1.4));
        assert!(parse_tsp("6366,Port Taranaki,HW,158,0.7").is_none());
    }
}

#[cfg(test)]
mod real {
    /// `RACENAV_TEST_TSD=<folder with a LINZ tidal stream gpkg> cargo test -p chart real_panels -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_panels() {
        let dir = std::env::var("RACENAV_TEST_TSD").unwrap();
        let s = crate::ChartStore::open_dir(&dir).unwrap();
        let (p, a) = s.tidal_streams([160.0, -53.0, 190.0, -28.0]);
        println!("{} panels, {} arrows; first {:?}", p.len(), a.len(), p.first());
        assert!(p.len() > 10);
    }
}
