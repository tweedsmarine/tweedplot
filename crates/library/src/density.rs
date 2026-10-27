//! Where time was spent: years of tracks as an effort map. Each fix's time (to the next
//! fix, at most five minutes) is added to the grid cell it's in, if the boat's speed then
//! was in a band (e.g. 0.5 to 5 kn, fishing rather than steaming). Drawing 13,000 tracks as
//! lines says little; this says where the boat worked, and for how long.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::OptionalExtension;

use crate::{store, Error, Library, Result};

/// Longest gap between fixes still counted (s): beyond it the boat may have been anywhere.
const MAX_GAP: i64 = 300;

impl Library {
    /// The effort map of the tracks `ids`: cells of `cell` degrees, with the minutes spent in
    /// each at speeds from `sog_min` to `sog_max` knots (a fix with no speed logged counts
    /// at the speed worked out from the next fix). [lon, lat, minutes] at each cell's centre.
    pub fn track_density(&self, ids: &[String], cell: f64, sog_min: f64, sog_max: f64, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<Vec<[f64; 3]>> {
        let cell = cell.clamp(0.001, 1.0);
        let mut grid: HashMap<(i64, i64), f64> = HashMap::new();
        for (k, id) in ids.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            if k % 200 == 0 {
                progress("Tracks", k, ids.len());
            }
            let data: Option<Vec<u8>> = self.db().query_row("SELECT data FROM track_data WHERE track_id = ?1", [id], |r| r.get(0)).optional()?;
            let Some(data) = data.filter(|d| !d.is_empty()) else { continue };
            for seg in store::decode_segments(&data)? {
                for w in seg.windows(2) {
                    let (p, q) = (&w[0], &w[1]);
                    let dt = match (p.t, q.t) {
                        (Some(a), Some(b)) if b > a => (b - a).min(MAX_GAP),
                        _ => continue,
                    };
                    let sog = p.sog.unwrap_or_else(|| nav_core::LatLon::new(p.lat, p.lon).distance_to(&nav_core::LatLon::new(q.lat, q.lon)) / (dt as f64 / 3600.0));
                    if sog < sog_min || sog > sog_max {
                        continue;
                    }
                    let key = ((p.lon / cell).floor() as i64, (p.lat / cell).floor() as i64);
                    *grid.entry(key).or_default() += dt as f64 / 60.0;
                }
            }
        }
        progress("Tracks", ids.len(), ids.len());
        Ok(grid.into_iter().map(|((x, y), m)| [(x as f64 + 0.5) * cell, (y as f64 + 0.5) * cell, (m * 10.0).round() / 10.0]).collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Library, Track, TrackPoint};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn effort_in_the_speed_band() {
        let dir = std::env::temp_dir().join(format!("lib-density-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).unwrap();
        // eleven fixes towing at 3 kn, then steaming at 10 kn
        let mut seg = Vec::new();
        for i in 0..11 {
            seg.push(TrackPoint { t: Some(i * 60), lat: -41.0 + i as f64 * 0.0001, lon: 174.0, sog: Some(3.0), cog: None, depth: None, temp: None });
        }
        for i in 11..20 {
            seg.push(TrackPoint { t: Some(i * 60), lat: -40.9 + i as f64 * 0.01, lon: 174.0, sog: Some(10.0), cog: None, depth: None, temp: None });
        }
        let t = lib.save_track(Track { name: "tow".into(), segments: vec![seg], ..Default::default() }).unwrap();
        let cells = lib.track_density(&[t.id], 0.01, 0.5, 5.0, &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        let total: f64 = cells.iter().map(|c| c[2]).sum();
        assert!((total - 11.0).abs() < 0.01, "{total}"); // the last towing fix counts to the next one
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod real {
    /// RN_LIB=<a real library.gpkg> cargo test --release --lib real_density -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_density() {
        let Ok(src) = std::env::var("RN_LIB") else { return };
        let dir = std::env::temp_dir().join("rn-real");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&src, dir.join("library.gpkg")).unwrap();
        let t0 = std::time::Instant::now();
        let lib = crate::Library::open(&dir).unwrap();
        println!("open + migrate {:?}", t0.elapsed());
        let ts = lib.tracks().unwrap();
        let ids: Vec<String> = ts.iter().map(|t| t.id.clone()).collect();
        let t0 = std::time::Instant::now();
        let cells = lib.track_density(&ids, 0.01, 0.5, 5.0, &mut |_, _, _| {}, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let h: f64 = cells.iter().map(|c| c[2]).sum::<f64>() / 60.0;
        println!("{} tracks → {} cells, {:.0} h, {:?}", ids.len(), cells.len(), h, t0.elapsed());
        println!("trips {} events {}", lib.trips().unwrap().len(), lib.events().unwrap().len());
        let t0 = std::time::Instant::now();
        let o = crate::Overlay::open(&dir.join("library.gpkg")).unwrap();
        println!("overlay marks {} in {:?}", o.marks().unwrap().len(), t0.elapsed());
    }
}
