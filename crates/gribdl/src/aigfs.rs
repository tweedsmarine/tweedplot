//! NOAA AIGFS (the AI version of GFS, operational on NOMADS since 17 Dec 2025): 0.25°,
//! 6-hourly to 384 h, four cycles a day. Public domain (US Government work).
//!
//! NOMADS has no GRIB filter for it, only the global files, each with a wgrib2 `.idx`:
//!
//! ```text
//! https://nomads.ncep.noaa.gov/pub/data/nccf/com/aigfs/prod/aigfs.20261124/00/model/atmos/grib2/aigfs.t00z.sfc.f006.grib2
//!   .idx:  1:0:d=2026112400:UGRD:10 m above ground:6 hour fcst:
//!          2:1043712:d=2026112400:VGRD:10 m above ground:6 hour fcst:
//! ```
//!
//! The surface file holds 10 m wind, 2 m temperature, MSL pressure and precipitation. Only the
//! wanted fields are fetched (HTTP range requests), decoded, cut to the area and written again
//! with simple packing, marked as AIGFS so it never reads as GFS.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::{client, Error, Model, Progress, Request, Result, Run};

const BASE: &str = "https://nomads.ncep.noaa.gov/pub/data/nccf/com/aigfs/prod";
const WORKERS: usize = 3;

fn base() -> String {
    match std::env::var("RACENAV_AIGFS_URL") {
        Ok(u) if !u.is_empty() => u.trim_end_matches('/').to_string(),
        _ => BASE.to_string(),
    }
}

/// URL of a step's GRIB file (`idx` false) or its index.
pub fn url(run: &Run, step: u32, idx: bool) -> String {
    format!("{}/aigfs.{}/{}/model/atmos/grib2/aigfs.t{}z.sfc.f{step:03}.grib2{}", base(), run.date, run.hh, run.hh, if idx { ".idx" } else { "" })
}

/// (variable, level) pairs in the index for a racenav parameter id.
fn wanted(param: &str) -> &'static [(&'static str, &'static str)] {
    match param {
        "wind" => &[("UGRD", "10 m above ground"), ("VGRD", "10 m above ground")],
        "mslp" => &[("PRMSL", "mean sea level")],
        "temp" => &[("TMP", "2 m above ground")],
        _ => &[],
    }
}

/// Byte ranges (offset, length; None = to the end of the file) of the wanted fields, merged where adjacent.
pub fn ranges(index: &str, params: &[String]) -> Vec<(u64, Option<u64>)> {
    let rows: Vec<(u64, &str, &str)> = index
        .lines()
        .filter_map(|l| {
            let mut it = l.split(':');
            let _n = it.next()?;
            let off = it.next()?.trim().parse().ok()?;
            let _d = it.next()?;
            Some((off, it.next()?, it.next()?))
        })
        .collect();
    let mut picks: Vec<(u64, Option<u64>)> = Vec::new();
    for p in params {
        for (var, lev) in wanted(p) {
            if let Some(i) = rows.iter().position(|r| r.1 == *var && r.2 == *lev) {
                let len = rows.get(i + 1).map(|n| n.0 - rows[i].0);
                picks.push((rows[i].0, len));
            }
        }
    }
    picks.sort_unstable_by_key(|p| p.0);
    picks.dedup();
    let mut merged: Vec<(u64, Option<u64>)> = Vec::new();
    for (o, n) in picks {
        match merged.last_mut() {
            Some((mo, Some(mn))) if *mo + *mn == o => *mn = n.map_or(u64::MAX, |n| *mn + n),
            _ => merged.push((o, n)),
        }
    }
    // a merged range that reached the last field runs to the end
    merged.into_iter().map(|(o, n)| (o, n.filter(|&n| n != u64::MAX))).collect()
}

fn get(c: &reqwest::blocking::Client, url: &str, range: Option<(u64, Option<u64>)>, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut wait = 2;
    for attempt in 0..4 {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let mut rq = c.get(url);
        if let Some((o, n)) = range {
            rq = rq.header(
                "Range",
                match n {
                    Some(n) => format!("bytes={}-{}", o, o + n - 1),
                    None => format!("bytes={o}-"),
                },
            );
        }
        match rq.send() {
            Ok(r) if r.status().is_success() => return r.bytes().map(|b| b.to_vec()).map_err(|e| Error::Http(e.to_string())),
            Ok(r) if r.status().as_u16() == 404 => return Err(Error::Http(format!("not on the server yet ({url})"))),
            Ok(r) if attempt == 3 => return Err(Error::Http(format!("NOMADS answered HTTP {}", r.status().as_u16()))),
            Err(e) if attempt == 3 => return Err(Error::Http(e.to_string())),
            _ => {}
        }
        std::thread::sleep(Duration::from_secs(wait));
        wait *= 3;
    }
    unreachable!()
}

/// The newest run whose last wanted step is published.
pub fn latest_run(last_hour: u32, now: i64) -> Result<Run> {
    let c = client()?;
    let newest = now - now.rem_euclid(6 * 3600);
    for k in 0..8 {
        let run = Run::at(newest - k * 6 * 3600);
        match c.head(url(&run, last_hour, true)).send() {
            Ok(r) if r.status().is_success() => return Ok(run),
            Ok(_) => {}
            Err(e) => return Err(Error::Http(format!("NOMADS: {e}"))),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(Error::NoRun("no complete AIGFS run found on NOMADS in the last two days".into()))
}

/// One step: the wanted fields, cut to the area and re-encoded.
fn step(c: &reqwest::blocking::Client, req: &Request, run: &Run, f: u32, cancel: &AtomicBool) -> Result<(Vec<u8>, u64)> {
    let index = String::from_utf8_lossy(&get(c, &url(run, f, true), None, cancel)?).to_string();
    let rs = ranges(&index, &req.params);
    if rs.is_empty() {
        return Err(Error::Bad(format!("step {f} h has none of the fields asked for")));
    }
    let mut out = Vec::new();
    let mut got = 0;
    for r in rs {
        let mut data = get(c, &url(run, f, false), Some(r), cancel)?;
        if let Some(n) = r.1 {
            if data.len() as u64 > n && data.len() as u64 >= r.0 + n {
                data = data[r.0 as usize..(r.0 + n) as usize].to_vec(); // the server sent the whole file
            }
        }
        got += data.len() as u64;
        let (fields, skipped) = grib::GribFile::decode_fields(&data);
        if fields.is_empty() {
            return Err(Error::Bad(format!("step {f} h: nothing decoded ({})", skipped.first().cloned().unwrap_or_default())));
        }
        for fld in &fields {
            if let Some(mut cut) = grib::write::crop(fld, req.area) {
                cut.centre = 7; // NCEP, so the process code reads back as AIGFS
                out.extend(grib::write::encode(&cut, grib::write::PROCESS_NOAA_AIGFS));
            }
        }
    }
    Ok((out, got))
}

pub fn download(req: &Request, run: &Run, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<PathBuf> {
    let hours = Model::Aigfs0p25.hours(req.hours, req.every);
    if hours.is_empty() {
        return Err(Error::Bad("no forecast times".into()));
    }
    std::fs::create_dir_all(dir)?;
    let c = client()?;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let results: Mutex<Vec<Option<Result<Vec<u8>>>>> = Mutex::new((0..hours.len()).map(|_| None).collect());
    progress(0, hours.len(), 0);
    std::thread::scope(|sc| {
        for _ in 0..WORKERS.min(hours.len()) {
            sc.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= hours.len() {
                    return;
                }
                let r = step(&c, req, run, hours[i], cancel);
                match &r {
                    Ok((_, got)) => {
                        let b = bytes.fetch_add(*got, Ordering::Relaxed) + got;
                        progress(done.fetch_add(1, Ordering::Relaxed) + 1, hours.len(), b);
                    }
                    Err(_) => stop.store(true, Ordering::Relaxed),
                }
                results.lock().unwrap()[i] = Some(r.map(|x| x.0));
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    // keep the steps up to the first failure (the end of a run may not be published yet)
    let mut out = Vec::new();
    let mut kept = 0;
    for r in results.into_inner().unwrap() {
        match r {
            Some(Ok(b)) => {
                out.extend(b);
                kept += 1;
            }
            Some(Err(e)) if kept == 0 => return Err(e),
            _ => break,
        }
    }
    let path = dir.join(crate::file_name(req, run, hours[kept - 1]));
    let part = path.with_extension("grb2.part");
    std::fs::write(&part, &out)?;
    std::fs::rename(&part, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_ranges() {
        let run = Run { date: "20261124".into(), hh: "06".into(), t: 0 };
        assert_eq!(
            url(&run, 12, true),
            "https://nomads.ncep.noaa.gov/pub/data/nccf/com/aigfs/prod/aigfs.20261124/06/model/atmos/grib2/aigfs.t06z.sfc.f012.grib2.idx"
        );
        let idx = "1:0:d=2026112406:UGRD:10 m above ground:12 hour fcst:\n\
                   2:1000:d=2026112406:VGRD:10 m above ground:12 hour fcst:\n\
                   3:2100:d=2026112406:TMP:2 m above ground:12 hour fcst:\n\
                   4:3000:d=2026112406:PRMSL:mean sea level:12 hour fcst:\n\
                   5:4200:d=2026112406:APCP:surface:6-12 hour acc fcst:\n";
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(ranges(idx, &p(&["wind"])), vec![(0, Some(2100))]);
        assert_eq!(ranges(idx, &p(&["wind", "mslp"])), vec![(0, Some(2100)), (3000, Some(1200))]);
        assert_eq!(ranges(idx, &p(&["wind", "temp", "mslp"])), vec![(0, Some(4200))]);
        // the last field in the file runs to the end
        let idx2 = "1:0:d=2026112406:UGRD:10 m above ground:12 hour fcst:\n2:1000:d=2026112406:PRMSL:mean sea level:12 hour fcst:\n";
        assert_eq!(ranges(idx2, &p(&["mslp"])), vec![(1000, None)]);
        assert_eq!(ranges(idx2, &p(&["wind", "mslp"])), vec![(0, None)]);
        assert!(ranges(idx, &p(&["gust"])).is_empty());
    }

    #[test]
    fn cut_fields_read_back_as_aigfs() {
        // a global 1° wind field (0..359°E), cut across 180° and written as we do
        let g = grib::Grid::regular(360, 181, -90.0, 0.0, 1.0, 1.0);
        let fld = grib::Field {
            param: grib::Param::WindU,
            discipline: 0,
            category: 2,
            number: 2,
            surface_type: 103,
            surface_value: 10.0,
            centre: 7,
            ref_time: 1_795_000_000,
            valid_time: 1_795_000_000 + 6 * 3600,
            member: None,
            grid: g,
            values: (0..360 * 181).map(|i| (i % 360) as f32).collect(),
            local: None,
        };
        let mut cut = grib::write::crop(&fld, [175.0, -45.0, -175.0, -35.0]).unwrap();
        cut.centre = 7;
        let f = grib::GribFile::from_bytes(&grib::write::encode(&cut, grib::write::PROCESS_NOAA_AIGFS));
        assert_eq!(f.model().0, "NOAA AIGFS");
        // the value is the column's longitude: 179 °E and 181 °E (= 179 °W) either side of the line
        assert!((f.value(grib::Param::WindU, -40.0, 179.0, fld.valid_time).unwrap() - 179.0).abs() < 0.01);
        assert!((f.value(grib::Param::WindU, -40.0, 181.0, fld.valid_time).unwrap() - 181.0).abs() < 0.01);
    }
}
