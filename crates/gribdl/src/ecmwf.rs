//! ECMWF open data (CC BY 4.0): IFS HRES and its wave model at 0.1° and 0.25°,
//! plus AIFS at 0.25°, from data.ecmwf.int (or its AWS mirror when that fails).
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
use std::time::SystemTime;

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
const GLOBAL_CACHE_LIMIT: u64 = 5 * 1024 * 1024 * 1024;
static CACHE_LOCK: Mutex<()> = Mutex::new(());
static CACHE_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// ECMWF short names for a racenav parameter id, in order of preference
/// (the first that the step's index lists is taken; e.g. gust names vary).
fn short_names(model: Model, param: &str) -> Vec<Vec<&'static str>> {
    match (model, param) {
        (Model::EcmwfWave0p25 | Model::EcmwfWave0p1, "waves") => vec![vec!["swh"]],
        (Model::EcmwfWave0p25 | Model::EcmwfWave0p1, "period") => vec![vec!["pp1d"], vec!["mwd"]],
        (Model::EcmwfWave0p25 | Model::EcmwfWave0p1, _) => vec![],
        (_, "wind") => vec![vec!["10u"], vec!["10v"]],
        (_, "mslp") => vec![vec!["msl"]],
        (_, "temp") => vec![vec!["2t"]],
        (Model::EcmwfIfs0p1, "current") => vec![vec!["sve"], vec!["svn"]],
        // total precipitation, accumulated from the start of the run: turned into a rate on loading (grib crate)
        (Model::EcmwfIfs0p25 | Model::EcmwfIfs0p1 | Model::EcmwfAifs0p25, "rain") => vec![vec!["tp"]],
        (Model::EcmwfIfs0p25 | Model::EcmwfIfs0p1, "gust") => vec![vec!["10fg", "i10fg", "fg10"]],
        _ => vec![],
    }
}

fn dir(model: Model, run: &Run) -> String {
    let (m, resolution, stream) = match model {
        Model::EcmwfAifs0p25 => ("aifs-single", "0p25", "oper"),
        Model::EcmwfWave0p25 => ("ifs", "0p25", "wave"),
        Model::EcmwfWave0p1 => ("ifs", "0p1", "wave"),
        Model::EcmwfIfs0p1 => ("ifs", "0p1", "oper"),
        _ => ("ifs", "0p25", "oper"),
    };
    format!("{}/{}z/{m}/{resolution}/{stream}", run.date, run.hh)
}

fn file_stem(model: Model, run: &Run, step: u32) -> String {
    let stream = dir(model, run).rsplit('/').next().unwrap_or("oper").to_string();
    let level = if matches!(model, Model::EcmwfIfs0p1 | Model::EcmwfWave0p1) { "-sfc" } else { "" };
    format!("{}{}0000-{step}h-{stream}-fc{level}", run.date, run.hh)
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
        let hit = alts.iter().find_map(|name| lines.iter().find(|v| v["param"].as_str() == Some(name) && v["levtype"].as_str().is_none_or(|l| l == "sfc")));
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

fn cache_path(cache_dir: &Path, url: &str, range: (u64, u64)) -> PathBuf {
    let path = url.split_once("://").map(|(_, rest)| rest.split_once('/').map_or(rest, |(_, path)| path));
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.unwrap_or(url).bytes().chain(range.0.to_le_bytes()).chain(range.1.to_le_bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    cache_dir.join(format!("{hash:016x}.grb2"))
}

fn read_cached_range(path: &Path, expected_len: u64) -> Result<Option<Vec<u8>>> {
    let _guard = CACHE_LOCK.lock().map_err(|_| Error::Bad("ECMWF cache lock poisoned".into()))?;
    match std::fs::read(path) {
        Ok(data) if data.len() as u64 == expected_len => Ok(Some(data)),
        Ok(_) => {
            std::fs::remove_file(path)?;
            Ok(None)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn cache_range(cache_dir: &Path, path: &Path, data: &[u8]) -> Result<()> {
    if data.len() as u64 > GLOBAL_CACHE_LIMIT {
        return Ok(());
    }
    let _guard = CACHE_LOCK.lock().map_err(|_| Error::Bad("ECMWF cache lock poisoned".into()))?;
    std::fs::create_dir_all(cache_dir)?;
    if path.exists() {
        return Ok(());
    }

    let mut entries = Vec::new();
    let mut used = 0u64;
    for entry in std::fs::read_dir(cache_dir)? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|ext| ext == "grb2") {
            let metadata = entry.metadata()?;
            used = used.saturating_add(metadata.len());
            entries.push((metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), metadata.len(), entry.path()));
        }
    }
    entries.sort_by_key(|entry| entry.0);
    for (_, len, old) in entries {
        if used.saturating_add(data.len() as u64) <= GLOBAL_CACHE_LIMIT {
            break;
        }
        std::fs::remove_file(old)?;
        used = used.saturating_sub(len);
    }

    let temp = cache_dir.join(format!(".{}.{}.part", std::process::id(), CACHE_TEMP_ID.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&temp, data)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

fn remove_cached_range(path: &Path) -> Result<()> {
    let _guard = CACHE_LOCK.lock().map_err(|_| Error::Bad("ECMWF cache lock poisoned".into()))?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// One step: fetch the wanted fields, cut them to the area, re-encode.
fn step(
    c: &reqwest::blocking::Client,
    req: &Request,
    run: &Run,
    f: u32,
    source: &AtomicUsize,
    process: u8,
    cancel: &AtomicBool,
    cache_dir: Option<&Path>,
) -> Result<(Vec<u8>, u64, bool)> {
    let wanted: Vec<Vec<&str>> = req.params.iter().flat_map(|p| short_names(req.model, p)).collect();
    let mut last = None;
    let n_src = sources().len();
    for _ in 0..n_src {
        let s = source.load(Ordering::Relaxed);
        let attempt = (|| -> Result<(Vec<u8>, u64, bool)> {
            let index = String::from_utf8_lossy(&get(c, &url(req.model, run, f, "index", s), None, cancel)?).to_string();
            let rs = ranges(&index, &wanted)?;
            if rs.is_empty() {
                return Err(Error::Bad(format!("step {f} h has none of the fields asked for")));
            }
            let mut out = Vec::new();
            let mut got = 0u64;
            let currents_requested = req.model == Model::EcmwfIfs0p1 && req.params.iter().any(|p| p == "current");
            let mut decoded_current_u = false;
            let mut decoded_current_v = false;
            let data_url = url(req.model, run, f, "grib2", s);
            for r in rs {
                let cached_path = cache_dir.map(|dir| cache_path(dir, &data_url, r));
                let mut downloaded = false;
                let mut data = if let Some(path) = &cached_path {
                    match read_cached_range(path, r.1)? {
                        Some(data) => data,
                        None => {
                            downloaded = true;
                            get(c, &data_url, Some(r), cancel)?
                        }
                    }
                } else {
                    downloaded = true;
                    get(c, &data_url, Some(r), cancel)?
                };
                if data.len() as u64 > r.1 && data.len() as u64 >= r.0 + r.1 {
                    // the server ignored the range and sent the whole file
                    data = data[r.0 as usize..(r.0 + r.1) as usize].to_vec();
                }
                let (mut fields, mut skipped) = grib::GribFile::decode_fields(&data);
                if fields.is_empty() && !downloaded {
                    if let Some(path) = &cached_path {
                        remove_cached_range(path)?;
                        data = get(c, &data_url, Some(r), cancel)?;
                        if data.len() as u64 > r.1 && data.len() as u64 >= r.0 + r.1 {
                            data = data[r.0 as usize..(r.0 + r.1) as usize].to_vec();
                        }
                        (fields, skipped) = grib::GribFile::decode_fields(&data);
                        downloaded = true;
                    }
                }
                if fields.is_empty() {
                    return Err(Error::Bad(format!("step {f} h: nothing decoded ({})", skipped.first().cloned().unwrap_or_default())));
                }
                decoded_current_u |= fields.iter().any(|field| field.param == grib::Param::CurrentU);
                decoded_current_v |= fields.iter().any(|field| field.param == grib::Param::CurrentV);
                if downloaded {
                    if let (Some(dir), Some(path)) = (cache_dir, &cached_path) {
                        cache_range(dir, path, &data)?;
                    }
                    got += data.len() as u64;
                }
                for fld in &fields {
                    // IFS total precipitation (tp, 0/1/52, metres): written as 0/1/8 in kg m-2 = mm, as AIFS has
                    // it, since the file keeps only template 4.0 (the grib crate turns it into a rain rate)
                    let tp;
                    let fld = if fld.centre == 98 && fld.discipline == 0 && fld.category == 1 && fld.number == 52 {
                        tp = grib::Field { number: 8, param: grib::Param::Other(0, 1, 8), values: fld.values.iter().map(|v| v * 1000.0).collect(), ..fld.clone() };
                        &tp
                    } else {
                        fld
                    };
                    if let Some(cut) = grib::write::crop(fld, req.area) {
                        out.extend(grib::write::encode(&cut, process));
                    }
                }
            }
            Ok((out, got, currents_requested && !(decoded_current_u && decoded_current_v)))
        })();
        match attempt {
            Ok((bytes, total, missing_currents)) => return Ok((bytes, total, missing_currents)),
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

fn current_warning(missing_steps: usize, first_missing_hour: Option<u32>) -> Option<String> {
    if missing_steps == 0 {
        return None;
    }
    let first = first_missing_hour.map_or(String::new(), |hour| format!(", starting at step {hour} h"));
    Some(format!(
        "Warning: could not decode both requested surface current fields (sve/svn) at {missing_steps} forecast step(s){first}. Other selected fields were downloaded; Current may not plot at those times."
    ))
}

pub fn download(req: &Request, run: &Run, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<(PathBuf, Option<String>)> {
    let max = cycle_hours(req.model, &run.hh);
    let hours: Vec<u32> = req.model.hours(req.hours, req.every).into_iter().filter(|&h| h <= max).collect();
    if hours.is_empty() {
        return Err(Error::Bad("no forecast times".into()));
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(crate::file_name(req, run, *hours.last().unwrap()));
    let c = client()?;
    let process = process(req.model);
    let cache_dir = matches!(req.model, Model::EcmwfIfs0p1 | Model::EcmwfWave0p1).then(|| dir.join(".ecmwf-cache"));
    let source = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let results: Mutex<Vec<Option<Result<(Vec<u8>, bool)>>>> = Mutex::new((0..hours.len()).map(|_| None).collect());
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
                let r = step(&c, req, run, hours[i], &source, process, cancel, cache_dir.as_deref());
                match &r {
                    Ok((_, got, _)) => {
                        let b = bytes.fetch_add(*got, Ordering::Relaxed) + got;
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        progress(d, hours.len(), b);
                    }
                    Err(_) => stop.store(true, Ordering::Relaxed),
                }
                results.lock().unwrap()[i] = Some(r.map(|x| (x.0, x.2)));
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
    let mut missing_current_steps = 0;
    let mut first_missing_current_hour = None;
    for r in results {
        match r {
            Some(Ok((b, incomplete_currents))) => {
                out.extend(b);
                if incomplete_currents {
                    missing_current_steps += 1;
                    first_missing_current_hour.get_or_insert(hours[kept]);
                }
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
    Ok((path, current_warning(missing_current_steps, first_missing_current_hour)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_streams() {
        let run = Run { date: "20261017".into(), hh: "00".into(), t: 0 };
        assert_eq!(
            url(Model::EcmwfIfs0p25, &run, 24, "index", 0),
            "https://data.ecmwf.int/forecasts/20261017/00z/ifs/0p25/oper/20261017000000-24h-oper-fc.index"
        );
        assert_eq!(
            url(Model::EcmwfWave0p25, &run, 0, "grib2", 1),
            "https://ecmwf-forecasts.s3.eu-central-1.amazonaws.com/20261017/00z/ifs/0p25/wave/20261017000000-0h-wave-fc.grib2"
        );
        let r18 = Run { date: "20261017".into(), hh: "18".into(), t: 0 };
        assert_eq!(
            url(Model::EcmwfIfs0p25, &r18, 6, "grib2", 0),
            "https://data.ecmwf.int/forecasts/20261017/18z/ifs/0p25/oper/20261017180000-6h-oper-fc.grib2"
        );
        assert_eq!(
            url(Model::EcmwfWave0p25, &r18, 6, "grib2", 0),
            "https://data.ecmwf.int/forecasts/20261017/18z/ifs/0p25/wave/20261017180000-6h-wave-fc.grib2"
        );
        assert_eq!(
            url(Model::EcmwfAifs0p25, &r18, 6, "grib2", 0),
            "https://data.ecmwf.int/forecasts/20261017/18z/aifs-single/0p25/oper/20261017180000-6h-oper-fc.grib2"
        );
        assert_eq!(
            url(Model::EcmwfIfs0p1, &run, 24, "index", 0),
            "https://data.ecmwf.int/forecasts/20261017/00z/ifs/0p1/oper/20261017000000-24h-oper-fc-sfc.index"
        );
        assert_eq!(
            url(Model::EcmwfWave0p1, &run, 24, "grib2", 0),
            "https://data.ecmwf.int/forecasts/20261017/00z/ifs/0p1/wave/20261017000000-24h-wave-fc-sfc.grib2"
        );
        assert_eq!(
            url(Model::EcmwfIfs0p1, &r18, 6, "grib2", 0),
            "https://data.ecmwf.int/forecasts/20261017/18z/ifs/0p1/oper/20261017180000-6h-oper-fc-sfc.grib2"
        );
        assert_eq!(cycle_hours(Model::EcmwfIfs0p25, "18"), 144);
    }

    #[test]
    fn index_ranges_pick_and_merge() {
        let idx = r#"{"domain": "g", "date": "20261017", "time": "0000", "type": "fc", "stream": "oper", "step": "24", "levtype": "sfc", "param": "msl", "_offset": 0, "_length": 500}
{"domain": "g", "type": "fc", "step": "24", "levtype": "pl", "levelist": "1000", "param": "u", "_offset": 500, "_length": 700}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "10u", "_offset": 1200, "_length": 600}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "10v", "_offset": 1800, "_length": 600}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "10fg", "_offset": 9000, "_length": 300}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "sve", "_offset": 10000, "_length": 200}
{"domain": "g", "type": "fc", "step": "24", "levtype": "sfc", "param": "svn", "_offset": 10200, "_length": 200}"#;
        let r = ranges(idx, &short_names(Model::EcmwfIfs0p25, "wind")).unwrap();
        assert_eq!(r, vec![(1200, 1200)]);
        let mut w = short_names(Model::EcmwfIfs0p25, "wind");
        w.extend(short_names(Model::EcmwfIfs0p25, "mslp"));
        w.extend(short_names(Model::EcmwfIfs0p25, "gust"));
        assert_eq!(ranges(idx, &w).unwrap(), vec![(0, 500), (1200, 1200), (9000, 300)]);
        // AIFS has no gust option; a missing field is simply left out
        assert!(short_names(Model::EcmwfAifs0p25, "gust").is_empty());
        assert_eq!(short_names(Model::EcmwfIfs0p1, "gust"), vec![vec!["10fg", "i10fg", "fg10"]]);
        assert_eq!(short_names(Model::EcmwfIfs0p1, "current"), vec![vec!["sve"], vec!["svn"]]);
        assert!(short_names(Model::EcmwfIfs0p25, "current").is_empty());
        let mut available_params = short_names(Model::EcmwfIfs0p1, "wind");
        available_params.extend(short_names(Model::EcmwfIfs0p1, "current"));
        let missing_north_current = idx.replace(", \"param\": \"svn\"", ", \"param\": \"unknown\"");
        assert_eq!(ranges(&missing_north_current, &available_params).unwrap(), vec![(1200, 1200), (10000, 200)]);
        assert_eq!(grib::Param::from_codes(10, 1, 2, 1), grib::Param::CurrentU);
        assert_eq!(grib::Param::from_codes(10, 1, 3, 1), grib::Param::CurrentV);
        assert_eq!(short_names(Model::EcmwfWave0p1, "waves"), vec![vec!["swh"]]);
        assert_eq!(ranges(idx, &short_names(Model::EcmwfIfs0p1, "current")).unwrap(), vec![(10000, 400)]);
    }

    #[test]
    fn incomplete_currents_warn_without_blocking_download() {
        assert!(current_warning(0, None).is_none());
        let note = current_warning(1, Some(0)).unwrap();
        assert!(note.contains("could not decode both requested surface current fields (sve/svn)"));
        assert!(note.contains("Other selected fields were downloaded"));
    }

    #[test]
    fn global_range_cache_round_trips() {
        let root = std::env::temp_dir().join(format!("racenav-ecmwf-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = cache_path(&root, "https://data.ecmwf.int/forecasts/20261023/00z/ifs/0p1/oper/20261023000000-24h-oper-fc-sfc.grib2", (123, 4));
        assert_ne!(path, cache_path(&root, "https://data.ecmwf.int/forecasts/20261023/00z/ifs/0p1/oper/20261023000000-24h-oper-fc-sfc.grib2", (124, 4),));
        assert_eq!(read_cached_range(&path, 4).unwrap(), None);
        cache_range(&root, &path, b"data").unwrap();
        assert_eq!(read_cached_range(&path, 4).unwrap(), Some(b"data".to_vec()));
        assert_eq!(read_cached_range(&path, 5).unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
