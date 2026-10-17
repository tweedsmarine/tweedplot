//! ECMWF open data (CC BY 4.0): IFS HRES, AIFS and the IFS wave model at
//! 0.25°, from data.ecmwf.int (or its AWS mirror when that fails).
//!
//! Each forecast step is one global file with a JSON-lines `.index` beside it
//! giving every field's byte offset and length:
//!
//! ```text
//! https://data.ecmwf.int/forecasts/20261017/00z/ifs/0p25/oper/20261017000000-24h-oper-fc.grib2
//! https://data.ecmwf.int/forecasts/20261017/00z/ifs/0p25/oper/20261017000000-24h-oper-fc.index
//!   {"param": "10u", "levtype": "sfc", "step": "24", "_offset": 123, "_length": 609069, ...}
//! ```
//!
//! Only the wanted fields are fetched (HTTP range requests); they are global
//! and CCSDS-packed, so each is decoded, cut to the area and written again
//! with simple packing. Four steps download at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::{client, Error, Model, Progress, Request, Result, Run};

const DEFAULT_SOURCES: [&str; 2] = ["https://data.ecmwf.int/forecasts", "https://ecmwf-forecasts.s3.eu-central-1.amazonaws.com"];

/// Where to download from: ECMWF, then its AWS mirror (`RACENAV_ECMWF_URL` replaces both, for testing).
fn sources() -> Vec<String> {
    match std::env::var("RACENAV_ECMWF_URL") {
        Ok(u) if !u.is_empty() => vec![u.trim_end_matches('/').to_string()],
        _ => DEFAULT_SOURCES.iter().map(|s| s.to_string()).collect(),
    }
}
const WORKERS: usize = 4;

/// ECMWF short names for a racenav parameter id, in order of preference
/// (the first that the step's index lists is taken; e.g. gust names vary).
fn short_names(model: Model, param: &str) -> Vec<Vec<&'static str>> {
    match (model, param) {
        (Model::EcmwfWave0p25, "waves") => vec![vec!["swh"]],
        (Model::EcmwfWave0p25, "period") => vec![vec!["pp1d"], vec!["mwd"]],
        (Model::EcmwfWave0p25, _) => vec![],
        (_, "wind") => vec![vec!["10u"], vec!["10v"]],
        (_, "mslp") => vec![vec!["msl"]],
        (_, "temp") => vec![vec!["2t"]],
        (Model::EcmwfIfs0p25, "gust") => vec![vec!["10fg", "i10fg", "fg10"]],
        _ => vec![],
    }
}

fn dir(model: Model, run: &Run) -> String {
    let (m, stream) = match model {
        Model::EcmwfAifs0p25 => ("aifs-single", "oper"),
        Model::EcmwfWave0p25 => ("ifs", if run.hh == "00" || run.hh == "12" { "wave" } else { "scwv" }),
        _ => ("ifs", if run.hh == "00" || run.hh == "12" { "oper" } else { "scda" }),
    };
    format!("{}/{}z/{m}/0p25/{stream}", run.date, run.hh)
}

fn file_stem(model: Model, run: &Run, step: u32) -> String {
    let stream = dir(model, run).rsplit('/').next().unwrap_or("oper").to_string();
    format!("{}{}0000-{step}h-{stream}-fc", run.date, run.hh)
}

/// Longest forecast of a cycle: the 06Z and 18Z IFS runs are short.
pub fn cycle_hours(model: Model, hh: &str) -> u32 {
    match model {
        Model::EcmwfAifs0p25 => 360,
        _ if hh == "06" || hh == "18" => 144,
        _ => 360,
    }
}

/// URL of a step's index (`ext` "index") or data ("grib2") at source `k`.
pub fn url(model: Model, run: &Run, step: u32, ext: &str, k: usize) -> String {
    let src = sources();
    format!("{}/{}/{}.{ext}", src[k % src.len()], dir(model, run), file_stem(model, run, step))
}

/// The newest run whose last wanted step is published.
pub fn latest_run(model: Model, last_hour: u32, now: i64) -> Result<Run> {
    let c = client()?;
    let newest = now - now.rem_euclid(6 * 3600);
    let mut last_err = None;
    for k in 0..8 {
        let run = Run::at(newest - k * 6 * 3600);
        if last_hour > cycle_hours(model, &run.hh) {
            continue;
        }
        for s in 0..sources().len() {
            match c.head(url(model, &run, last_hour, "index", s)).send() {
                Ok(r) if r.status().is_success() => return Ok(run),
                Ok(_) => break, // not there yet: same answer from the mirror
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(match last_err {
        Some(e) => Error::Http(format!("ECMWF open data: {e}")),
        None => Error::NoRun(format!("no complete {} run found in the last two days", model.info().name)),
    })
}

/// Byte ranges (offset, length) of the wanted fields in one step, merged where adjacent.
fn ranges(index: &str, wanted: &[Vec<&str>]) -> Result<Vec<(u64, u64)>> {
    let lines: Vec<serde_json::Value> = index.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let mut picks: Vec<(u64, u64)> = Vec::new();
    for alts in wanted {
        let hit = alts.iter().find_map(|name| {
            lines.iter().find(|v| v["param"].as_str() == Some(name) && v["levtype"].as_str().is_none_or(|l| l == "sfc"))
        });
        if let Some(v) = hit {
            let (Some(o), Some(n)) = (v["_offset"].as_u64(), v["_length"].as_u64()) else { continue };
            picks.push((o, n));
        }
    }
    picks.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (o, n) in picks {
        match merged.last_mut() {
            Some(m) if m.0 + m.1 == o => m.1 += n,
            _ => merged.push((o, n)),
        }
    }
    Ok(merged)
}

fn get(c: &reqwest::blocking::Client, url: &str, range: Option<(u64, u64)>, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut wait = 2;
    for attempt in 0..4 {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let mut rq = c.get(url);
        if let Some((o, n)) = range {
            rq = rq.header("Range", format!("bytes={}-{}", o, o + n - 1));
        }
        match rq.send() {
            Ok(r) if r.status().is_success() => return r.bytes().map(|b| b.to_vec()).map_err(|e| Error::Http(e.to_string())),
            Ok(r) if r.status().as_u16() == 404 => return Err(Error::Http(format!("not published yet ({url})"))),
            Ok(r) if attempt == 3 => return Err(Error::Http(format!("ECMWF answered HTTP {}", r.status().as_u16()))),
            Err(e) if attempt == 3 => return Err(Error::Http(e.to_string())),
            _ => {}
        }
        std::thread::sleep(Duration::from_secs(wait));
        wait *= 3;
    }
    unreachable!()
}

/// One step: fetch the wanted fields, cut them to the area, re-encode.
fn step(c: &reqwest::blocking::Client, req: &Request, run: &Run, f: u32, source: &AtomicUsize, process: u8, cancel: &AtomicBool) -> Result<(Vec<u8>, u64)> {
    let wanted: Vec<Vec<&str>> = req.params.iter().flat_map(|p| short_names(req.model, p)).collect();
    let mut last = None;
    let n_src = sources().len();
    for _ in 0..n_src {
        let s = source.load(Ordering::Relaxed);
        let attempt = (|| -> Result<(Vec<u8>, u64)> {
            let index = String::from_utf8_lossy(&get(c, &url(req.model, run, f, "index", s), None, cancel)?).to_string();
            let rs = ranges(&index, &wanted)?;
            if rs.is_empty() {
                return Err(Error::Bad(format!("step {f} h has none of the fields asked for")));
            }
            let mut out = Vec::new();
            let mut got = 0u64;
            for r in rs {
                let mut data = get(c, &url(req.model, run, f, "grib2", s), Some(r), cancel)?;
                if data.len() as u64 > r.1 && data.len() as u64 >= r.0 + r.1 {
                    // the server ignored the range and sent the whole file
                    data = data[r.0 as usize..(r.0 + r.1) as usize].to_vec();
                }
                got += data.len() as u64;
                let (fields, skipped) = grib::GribFile::decode_fields(&data);
                if fields.is_empty() {
                    return Err(Error::Bad(format!("step {f} h: nothing decoded ({})", skipped.first().cloned().unwrap_or_default())));
                }
                for fld in &fields {
                    if let Some(cut) = grib::write::crop(fld, req.area) {
                        out.extend(grib::write::encode(&cut, process));
                    }
                }
            }
            Ok((out, got))
        })();
        match attempt {
            Ok(x) => return Ok(x),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e @ Error::Http(_)) if !e.to_string().starts_with("not published") => {
                // try the mirror for this and later steps
                let _ = source.compare_exchange(s, (s + 1) % n_src, Ordering::Relaxed, Ordering::Relaxed);
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| Error::Http("no source answered".into())))
}

/// The process code written into each field so the file reads back as this model.
pub fn process(model: Model) -> u8 {
    match model {
        Model::EcmwfAifs0p25 => grib::write::PROCESS_ECMWF_AIFS,
        Model::EcmwfWave0p25 => grib::write::PROCESS_ECMWF_WAM,
        _ => grib::write::PROCESS_ECMWF_IFS,
    }
}

pub fn download(req: &Request, run: &Run, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<PathBuf> {
    let max = cycle_hours(req.model, &run.hh);
    let hours: Vec<u32> = req.model.hours(req.hours, req.every).into_iter().filter(|&h| h <= max).collect();
    if hours.is_empty() {
        return Err(Error::Bad("no forecast times".into()));
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(crate::file_name(req, run, *hours.last().unwrap()));
    let c = client()?;
    let process = process(req.model);
    let source = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let results: Mutex<Vec<Option<Result<Vec<u8>>>>> = Mutex::new((0..hours.len()).map(|_| None).collect());
    let stop = AtomicBool::new(false);
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
                let r = step(&c, req, run, hours[i], &source, process, cancel);
                match &r {
                    Ok((_, got)) => {
                        let b = bytes.fetch_add(*got, Ordering::Relaxed) + got;
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        progress(d, hours.len(), b);
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
    let results = results.into_inner().unwrap();
    // keep the steps up to the first failure (a run can be cut short on the server)
    let mut out = Vec::new();
    let mut kept = 0;
    for r in results {
        match r {
            Some(Ok(b)) => {
                out.extend(b);
                kept += 1;
            }
            Some(Err(e)) if kept == 0 => return Err(e),
            _ => break,
        }
    }
    let path = if kept < hours.len() { dir.join(crate::file_name(req, run, hours[kept - 1])) } else { path };
    let part = path.with_extension("grb2.part");
    std::fs::write(&part, &out)?;
    std::fs::rename(&part, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_streams() {
        let run = Run { date: "20261017".into(), hh: "00".into(), t: 0 };
        assert_eq!(url(Model::EcmwfIfs0p25, &run, 24, "index", 0), "https://data.ecmwf.int/forecasts/20261017/00z/ifs/0p25/oper/20261017000000-24h-oper-fc.index");
        assert_eq!(url(Model::EcmwfWave0p25, &run, 0, "grib2", 1), "https://ecmwf-forecasts.s3.eu-central-1.amazonaws.com/20261017/00z/ifs/0p25/wave/20261017000000-0h-wave-fc.grib2");
        let r18 = Run { date: "20261017".into(), hh: "18".into(), t: 0 };
        assert_eq!(url(Model::EcmwfIfs0p25, &r18, 6, "grib2", 0), "https://data.ecmwf.int/forecasts/20261017/18z/ifs/0p25/scda/20261017180000-6h-scda-fc.grib2");
        assert_eq!(url(Model::EcmwfWave0p25, &r18, 6, "grib2", 0), "https://data.ecmwf.int/forecasts/20261017/18z/ifs/0p25/scwv/20261017180000-6h-scwv-fc.grib2");
        assert_eq!(url(Model::EcmwfAifs0p25, &r18, 6, "grib2", 0), "https://data.ecmwf.int/forecasts/20261017/18z/aifs-single/0p25/oper/20261017180000-6h-oper-fc.grib2");
        assert_eq!(cycle_hours(Model::EcmwfIfs0p25, "18"), 144);
    }

    #[test]
    fn index_ranges_pick_and_merge() {
        let idx = r#"{"domain": "g", "date": "20261017", "time": "0000", "type": "fc", "stream": "oper", "step": "24", "levtype": "sfc", "param": "msl", "_offset": 0, "_length": 500}
{"domain": "g", "type": "fc", "step": "24", "levtype": "pl", "levelist": "1000", "param": "u", "_offset": 500, "_length": 700}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "10u", "_offset": 1200, "_length": 600}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "10v", "_offset": 1800, "_length": 600}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "10fg", "_offset": 9000, "_length": 300}"#;
        let r = ranges(idx, &short_names(Model::EcmwfIfs0p25, "wind")).unwrap();
        assert_eq!(r, vec![(1200, 1200)]);
        let mut w = short_names(Model::EcmwfIfs0p25, "wind");
        w.extend(short_names(Model::EcmwfIfs0p25, "mslp"));
        w.extend(short_names(Model::EcmwfIfs0p25, "gust"));
        assert_eq!(ranges(idx, &w).unwrap(), vec![(0, 500), (1200, 1200), (9000, 300)]);
        // AIFS has no gust option; a missing field is simply left out
        assert!(short_names(Model::EcmwfAifs0p25, "gust").is_empty());
    }
}
