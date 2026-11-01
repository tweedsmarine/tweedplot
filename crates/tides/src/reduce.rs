//! The height of tide above chart datum at any place and time, for reducing soundings to
//! chart datum: the LINZ ports near the place (daily tables, and secondary ports with heights),
//! each predicted with LINZ's cosine curve, blended by distance.
//!
//! - Within [`ONE_PORT_KM`] of a port, that port alone.
//! - Else up to [`MAX_PORTS`] ports within `max_km`, weighted by the inverse square of the
//!   distance (so the nearest dominates, and the blend changes smoothly between ports).
//! - A port without a table for the time is left out of the blend; with none left, or no port
//!   within `max_km`, there is no height (the sounding is kept as logged).
//!
//! Built for millions of soundings: each port's high and low waters are read once, and the
//! ports for a place are found once per ~1 km square.

use std::collections::HashMap;

use crate::{Extreme, Kind, TideDb};

/// Within this distance of a port, that port alone is used (km).
pub const ONE_PORT_KM: f64 = 2.0;
/// At most this many ports in a blend.
pub const MAX_PORTS: usize = 3;
/// The default reach of the ports (km): beyond it a sounding is kept as logged.
pub const DEFAULT_MAX_KM: f64 = 30.0;

/// The height of tide at a place and time, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct TideAt {
    /// Metres above chart datum.
    pub h: f64,
    /// The ports used: (station index in [`TideDb::stations`], weight 0..1).
    pub ports: Vec<(usize, f64)>,
}

/// Heights of tide from one [`TideDb`], with its caches.
pub struct Reducer<'a> {
    db: &'a TideDb,
    max_km: f64,
    /// Each usable station's high and low waters (with heights), by station index.
    series: HashMap<usize, Vec<Extreme>>,
    /// The stations near each ~1 km square (by 0.01° lat/lon), with their distance weights.
    near: HashMap<(i32, i32), Vec<(usize, f64)>>,
}

fn dist_km(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> f64 {
    let dy = (lat0 - lat1) * 111.2;
    let dx = (lon0 - lon1) * 111.2 * ((lat0 + lat1) / 2.0).to_radians().cos();
    (dx * dx + dy * dy).sqrt()
}

impl<'a> Reducer<'a> {
    /// `max_km`: how far a port reaches (see [`DEFAULT_MAX_KM`]).
    pub fn new(db: &'a TideDb, max_km: f64) -> Self {
        let mut series = HashMap::new();
        for (i, s) in db.stations.iter().enumerate() {
            // daily ports, and secondary ports whose heights LINZ gives (a range ratio and levels)
            let usable = match &s.kind {
                Kind::Daily => true,
                Kind::Secondary { ratio, .. } => ratio.is_some() && s.levels.msl.is_some(),
                Kind::LevelsOnly { .. } => false,
            };
            if !usable {
                continue;
            }
            if let Some(ext) = db.extremes(&s.id, i64::MIN / 4, i64::MAX / 4) {
                if ext.iter().any(|e| e.h.is_some()) {
                    series.insert(i, ext);
                }
            }
        }
        Reducer { db, max_km, series, near: HashMap::new() }
    }

    /// Whether any port has heights (no tide tables: nothing can be reduced).
    pub fn has_ports(&self) -> bool {
        !self.series.is_empty()
    }

    /// The first and last time any port has a table for.
    pub fn span(&self) -> Option<(i64, i64)> {
        let first = self.series.values().filter_map(|v| v.first().map(|e| e.t)).min()?;
        let last = self.series.values().filter_map(|v| v.last().map(|e| e.t)).max()?;
        Some((first, last))
    }

    fn ports_near(&mut self, lat: f64, lon: f64) -> &[(usize, f64)] {
        let k = ((lat * 100.0).floor() as i32, (lon * 100.0).floor() as i32);
        if !self.near.contains_key(&k) {
            // the centre of the square: close enough for choosing ports and weights
            let (clat, clon) = ((k.0 as f64 + 0.5) / 100.0, (k.1 as f64 + 0.5) / 100.0);
            let mut d: Vec<(usize, f64)> = self
                .series
                .keys()
                .map(|&i| {
                    let s = &self.db.stations[i];
                    (i, dist_km(clat, clon, s.lat, s.lon))
                })
                .filter(|(_, km)| *km <= self.max_km)
                .collect();
            d.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let pick: Vec<(usize, f64)> = match d.first() {
                Some(&(i, km)) if km <= ONE_PORT_KM => vec![(i, 1.0)],
                _ => d.into_iter().take(MAX_PORTS).map(|(i, km)| (i, 1.0 / (km * km).max(1e-6))).collect(),
            };
            self.near.insert(k, pick);
        }
        &self.near[&k]
    }

    /// The height of tide above chart datum at a place and time (Unix seconds, UTC).
    pub fn at(&mut self, lat: f64, lon: f64, t: i64) -> Option<TideAt> {
        let ports: Vec<(usize, f64)> = self.ports_near(lat, lon).to_vec();
        let mut sum = 0.0;
        let mut wsum = 0.0;
        let mut used = Vec::with_capacity(ports.len());
        for (i, w) in ports {
            if let Some(h) = TideDb::height_at(&self.series[&i], t) {
                sum += w * h;
                wsum += w;
                used.push((i, w));
            }
        }
        if wsum <= 0.0 {
            return None;
        }
        for u in &mut used {
            u.1 /= wsum;
        }
        Some(TideAt { h: sum / wsum, ports: used })
    }

    /// The name of a station used in a [`TideAt`].
    pub fn port_name(&self, i: usize) -> &str {
        &self.db.stations[i].name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Levels, Station};

    fn port(db: &mut TideDb, name: &str, lat: f64, lon: f64, ext: Vec<Extreme>) {
        db.series.insert(crate::key(name), ext);
        db.stations.push(Station { id: format!("d:{name}"), name: name.into(), lat, lon, kind: Kind::Daily, levels: Levels::default(), approx: false });
    }

    fn tide(t0: i64, low: f64, high: f64) -> Vec<Extreme> {
        // half-day tides: low at t0, high 6 h 12 min later, and so on for two days
        (0..8).map(|k| Extreme { t: t0 + k * 22_320, h: Some(if k % 2 == 0 { low } else { high }), high: k % 2 == 1 }).collect()
    }

    #[test]
    fn one_port_close_by_and_a_blend_between() {
        let t0 = 1_800_000_000;
        let mut db = TideDb::default();
        port(&mut db, "A", -41.0, 173.0, tide(t0, 0.5, 3.5));
        port(&mut db, "B", -41.0, 173.2, tide(t0, 1.0, 2.0));
        let mut r = Reducer::new(&db, DEFAULT_MAX_KM);
        assert!(r.has_ports());
        // at low water, next to A: A alone
        let a = r.at(-41.0, 173.001, t0).unwrap();
        assert!((a.h - 0.5).abs() < 1e-9 && a.ports.len() == 1);
        // half-way between: both, equally
        let m = r.at(-41.0, 173.1, t0).unwrap();
        assert_eq!(m.ports.len(), 2);
        assert!((m.h - 0.75).abs() < 0.05, "{}", m.h);
        // at half tide at A (cosine curve: the middle of the range)
        let h = r.at(-41.0, 173.0, t0 + 11_160).unwrap().h;
        assert!((h - 2.0).abs() < 1e-6, "{h}");
        // far from both, or outside the tables: no height
        assert!(r.at(-42.0, 175.0, t0).is_none());
        assert!(r.at(-41.0, 173.0, t0 - 86_400).is_none());
        assert_eq!(r.port_name(a.ports[0].0), "A");
    }

    #[test]
    fn a_port_without_a_table_drops_out_of_the_blend() {
        let t0 = 1_800_000_000;
        let mut db = TideDb::default();
        port(&mut db, "A", -41.0, 173.0, tide(t0, 0.5, 3.5));
        // B's table is for another time
        port(&mut db, "B", -41.0, 173.2, tide(t0 + 10 * 86_400, 1.0, 2.0));
        let mut r = Reducer::new(&db, DEFAULT_MAX_KM);
        let m = r.at(-41.0, 173.1, t0).unwrap();
        assert_eq!(m.ports.len(), 1);
        assert!((m.h - 0.5).abs() < 1e-9);
    }
}
