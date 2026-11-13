//! GRIB weather: downloads, loading and the grids, points and comparisons the page draws.

use super::*;

// ---------- GRIB downloads (NOMADS GFS) ----------

/// Downloaded GRIBs go here: `RACENAV_GRIB_DIR`, else `C:\dev\racenav-data\grib`
/// when racenav-data exists, else `<app data>\grib`.
pub(crate) static DOWNLOAD_DIR: OnceLock<PathBuf> = OnceLock::new();
pub(crate) static DL_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) static DL_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn download_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_GRIB_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("grib");
    }
    app.path().app_data_dir().map(|p| p.join("grib")).unwrap_or_else(|_| PathBuf::from("grib"))
}

#[tauri::command]
pub(crate) fn gribdl_models() -> Vec<gribdl::ModelInfo> {
    gribdl::Model::all().iter().map(|m| m.info()).collect()
}

#[tauri::command]
pub(crate) fn gribdl_estimate(req: gribdl::Request) -> serde_json::Value {
    let steps = req.model.hours(req.hours, req.every);
    serde_json::json!({ "bytes": gribdl::estimate_bytes(&req), "download": gribdl::download_bytes(&req), "steps": steps.len(), "last": steps.last() })
}

/// Find the newest complete run and download it, cut to the area. Progress
/// comes as `gribdl-progress` events; returns the file written.
#[tauri::command]
pub(crate) async fn gribdl_start(app: tauri::AppHandle, req: gribdl::Request) -> Result<serde_json::Value, String> {
    use std::sync::atomic::Ordering;
    if DL_BUSY.swap(true, Ordering::SeqCst) {
        return Err("A download is already running.".into());
    }
    DL_CANCEL.store(false, Ordering::SeqCst);
    let dir = DOWNLOAD_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("grib"));
    let res = tauri::async_runtime::spawn_blocking(move || {
        let last = *req.model.hours(req.hours, req.every).last().unwrap_or(&0);
        let _ = app.emit("gribdl-progress", serde_json::json!({ "stage": "Finding the latest run", "done": 0, "total": 0, "bytes": 0 }));
        let run = gribdl::latest_run(req.model, last, chrono_now()).map_err(|e| e.to_string())?;
        let label = format!("{} run {}", req.model.info().name, grib::fmt_run(run.t));
        let progress = |done: usize, total: usize, bytes: u64| {
            let _ = app.emit("gribdl-progress", serde_json::json!({ "stage": label, "done": done, "total": total, "bytes": bytes }));
        };
        let d = gribdl::download_ex(&req, &run, &dir, &progress, &DL_CANCEL).map_err(|e| e.to_string())?;
        Ok::<_, String>(serde_json::json!({ "path": d.path, "run": run, "name": label, "note": d.note }))
    })
    .await
    .map_err(|e| e.to_string());
    DL_BUSY.store(false, Ordering::SeqCst);
    res?
}

/// Put a wave GRIB into the same file as the wind GRIB of the same run
/// (`<wind file>+waves.grb2` beside the wind file). Returns the new file's path.
#[tauri::command]
pub(crate) async fn grib_combine(a: String, b: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        grib::combine::combine(std::path::Path::new(&a), std::path::Path::new(&b)).map(|p| p.to_string_lossy().into_owned()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn gribdl_cancel() {
    DL_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub(crate) fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[tauri::command]
pub(crate) async fn grib_files() -> Vec<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(grib_files_blocking).await.unwrap_or_default()
}

pub(crate) fn grib_files_blocking() -> Vec<serde_json::Value> {
    let loaded = GRIBS.lock().unwrap();
    let mut out = Vec::new();
    for dir in grib_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "grb" | "grb2" | "grib" | "grib2") {
                continue;
            }
            let md = e.metadata().ok();
            let md2 = md.clone();
            let mtime = md.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
            let key = p.to_string_lossy().into_owned();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            let info = scan_cached(&p, mtime.unwrap_or(0), size);
            out.push(serde_json::json!({
                "info": info,
                "path": key,
                "file": p.file_name().map(|f| f.to_string_lossy().into_owned()),
                "dir": dir,
                "size": md2.map(|m| m.len()),
                "mtime": mtime,
                "loaded": loaded.contains_key(&key),
            }));
        }
    }
    // newest model run first, then newest download
    out.sort_by(|a, b| {
        let run = |x: &serde_json::Value| x["info"]["runs"][0]["ref_time"].as_i64().unwrap_or(0);
        run(b).cmp(&run(a)).then(b["mtime"].as_u64().cmp(&a["mtime"].as_u64()))
    });
    out
}

/// Header scans are cached by path, modification time and size.
pub(crate) fn scan_cached(p: &std::path::Path, mtime: u64, size: u64) -> Option<grib::FileInfo> {
    static CACHE: Mutex<BTreeMap<String, (u64, u64, Option<grib::FileInfo>)>> = Mutex::new(BTreeMap::new());
    let key = p.to_string_lossy().into_owned();
    if let Some((m, s, info)) = CACHE.lock().unwrap().get(&key) {
        if *m == mtime && *s == size {
            return info.clone();
        }
    }
    let info = grib::scan(p).ok();
    CACHE.lock().unwrap().insert(key, (mtime, size, info.clone()));
    info
}

#[tauri::command]
pub(crate) async fn grib_load(app: tauri::AppHandle, path: String) -> Result<grib::Summary, String> {
    let p = path.clone();
    let f = tauri::async_runtime::spawn_blocking(move || {
        // progress events, at most one per 2 %
        let last = std::sync::atomic::AtomicU32::new(0);
        let progress = |frac: f64, stage: &str| {
            let pc = (frac * 100.0).round() as u32;
            let prev = last.load(std::sync::atomic::Ordering::Relaxed);
            if pc >= prev + 2 || pc == 0 || pc >= 97 {
                last.store(pc, std::sync::atomic::Ordering::Relaxed);
                let _ = app.emit("grib-progress", serde_json::json!({ "path": p, "percent": pc, "stage": stage }));
            }
        };
        grib::GribFile::open_with_progress(&p, &progress)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let mut s = f.summary();
    s.file = path.clone();
    GRIBS.lock().unwrap().insert(path, Arc::new(f));
    Ok(s)
}

#[tauri::command]
pub(crate) fn grib_unload(path: String) {
    GRIBS.lock().unwrap().remove(&path);
}

#[tauri::command]
pub(crate) fn grib_loaded() -> Vec<grib::Summary> {
    GRIBS
        .lock()
        .unwrap()
        .iter()
        .map(|(k, f)| {
            let mut s = f.summary();
            s.file = k.clone();
            s
        })
        .collect()
}

/// Wind on a thinned grid inside `bbox` [w, s, e, n] at time `t`, at most about `max_points` points.
#[tauri::command]
pub(crate) fn grib_wind_grid(path: String, t: i64, bbox: [f64; 4], max_points: usize) -> Option<serde_json::Value> {
    let f = GRIBS.lock().unwrap().get(&path)?.clone();
    let (g, u) = f.grid_at(grib::Param::WindU, t)?;
    let (_, v) = f.grid_at(grib::Param::WindV, t)?;
    // Grids may run 0..360 (GFS) while the chart asks in -180..180: shift the
    // view by whole turns so it lines up with the grid, and back for the output.
    let turn = ((g.lon0 + g.dlon * (g.ni as f64 - 1.0) / 2.0 - (bbox[0] + bbox[2]) / 2.0) / 360.0).round() * 360.0;
    let bbox = [bbox[0] + turn, bbox[1], bbox[2] + turn, bbox[3]];
    // grid index window covering the bbox
    let i_lo = (((bbox[0] - g.lon0) / g.dlon).floor().max(0.0) as usize).min(g.ni - 1);
    let i_hi = (((bbox[2] - g.lon0) / g.dlon).ceil().max(0.0) as usize).min(g.ni - 1);
    let j_lo = (((bbox[1] - g.lat0) / g.dlat).floor().max(0.0) as usize).min(g.nj - 1);
    let j_hi = (((bbox[3] - g.lat0) / g.dlat).ceil().max(0.0) as usize).min(g.nj - 1);
    let (wi, wj) = (i_hi - i_lo + 1, j_hi - j_lo + 1);
    let stride = (((wi * wj) as f64 / max_points.max(1) as f64).sqrt().ceil() as usize).max(1);
    let mut tws = Vec::new();
    let mut twd = Vec::new();
    let (mut ni, mut nj) = (0, 0);
    for j in (j_lo..=j_hi).step_by(stride) {
        nj += 1;
        ni = 0;
        for i in (i_lo..=i_hi).step_by(stride) {
            ni += 1;
            let k = j * g.ni + i;
            let (uu, vv) = (u[k] as f64, v[k] as f64);
            if uu.is_finite() && vv.is_finite() {
                let w = grib::Wind::from_uv(uu, vv);
                tws.push(Some((w.tws * 10.0).round() / 10.0));
                twd.push(Some(w.twd.round()));
            } else {
                tws.push(None);
                twd.push(None);
            }
        }
    }
    Some(serde_json::json!({
        "lon0": g.lon0 + i_lo as f64 * g.dlon - turn,
        "lat0": g.lat0 + j_lo as f64 * g.dlat,
        "dlon": g.dlon * stride as f64,
        "dlat": g.dlat * stride as f64,
        "ni": ni, "nj": nj, "tws": tws, "twd": twd,
    }))
}

/// One quantity on a thinned grid inside `bbox` at time `t`, for shading and
/// contours: "wind" and "gust" (kn), "waves", "swell", "windwaves" (m),
/// "period" (s), "wavedir" (deg from), "rain" (mm/h), "cape" (J/kg), "mslp" (hPa), "temp" (°C).
#[tauri::command]
pub(crate) fn grib_grid(path: String, layer: String, t: i64, bbox: [f64; 4], max_points: usize) -> Option<serde_json::Value> {
    use grib::Param as P;
    let f = GRIBS.lock().unwrap().get(&path)?.clone();
    const KN: f32 = 3600.0 / 1852.0;
    // the grid(s) and how to turn their values into the quantity shown
    let (g, vals): (grib::Grid, Vec<f32>) = match layer.as_str() {
        "wind" => {
            if let (Some((g, u)), Some((_, v))) = (f.grid_at(P::WindU, t), f.grid_at(P::WindV, t)) {
                (g, u.iter().zip(&v).map(|(a, b)| (a * a + b * b).sqrt() * KN).collect())
            } else {
                let (g, w) = f.grid_at(P::WindSpeed, t)?;
                (g, w.iter().map(|x| x * KN).collect())
            }
        }
        "gust" => {
            let (g, v) = f.grid_at(P::Gust, t)?;
            (g, v.iter().map(|x| x * KN).collect())
        }
        "waves" => f.grid_at(P::WaveHeight, t)?,
        "swell" => f.grid_at(P::SwellHeight, t)?,
        "windwaves" => f.grid_at(P::WindWaveHeight, t)?,
        "period" => f.grid_at(P::WavePeriod, t)?,
        "wavedir" => f.grid_at(P::WaveDir, t)?,
        "rain" => {
            let (g, v) = f.grid_at(P::Rain, t)?;
            (g, v.iter().map(|x| x * 3600.0).collect())
        }
        "cape" => f.grid_at(P::Cape, t)?,
        "mslp" => {
            let (g, v) = f.grid_at(P::Mslp, t)?;
            (g, v.iter().map(|x| x / 100.0).collect())
        }
        "temp" => {
            let (g, v) = f.grid_at(P::AirTemp, t)?;
            (g, v.iter().map(|x| x - 273.15).collect())
        }
        // ocean: °C, PSU, current in knots
        "sst" => {
            let (g, v) = f.grid_at(P::SeaTemp, t)?;
            (g, v.iter().map(|x| x - 273.15).collect())
        }
        "sss" => f.grid_at(P::Salinity, t)?,
        "subt" => {
            let (g, v) = f.grid_at(P::SubTemp, t)?;
            (g, v.iter().map(|x| x - 273.15).collect())
        }
        "subs" => f.grid_at(P::SubSalinity, t)?,
        "skin" => {
            let (g, v) = f.grid_at(P::SkinTemp, t)?;
            (g, v.iter().map(|x| x - 273.15).collect())
        }
        // chlorophyll as log10 mg/m³: shaded, filtered and contoured on a log scale
        "chl" => {
            let (g, v) = f.grid_at(P::Chlorophyll, t)?;
            (g, v.iter().map(|x| if *x > 0.0 { x.log10() } else { f32::NAN }).collect())
        }
        "current" => {
            let ((g, u), (_, v)) = (f.grid_at(P::CurrentU, t)?, f.grid_at(P::CurrentV, t)?);
            (g, u.iter().zip(&v).map(|(a, b)| (a * a + b * b).sqrt() * KN).collect())
        }
        "ssh" => f.grid_at(P::SeaLevel, t)?,
        // the way the water flows, degrees true
        "currentdir" => {
            let ((g, u), (_, v)) = (f.grid_at(P::CurrentU, t)?, f.grid_at(P::CurrentV, t)?);
            (g, u.iter().zip(&v).map(|(a, b)| (a.atan2(*b).to_degrees() + 360.0) % 360.0).collect())
        }
        _ => return None,
    };
    let turn = ((g.lon0 + g.dlon * (g.ni as f64 - 1.0) / 2.0 - (bbox[0] + bbox[2]) / 2.0) / 360.0).round() * 360.0;
    let bbox = [bbox[0] + turn, bbox[1], bbox[2] + turn, bbox[3]];
    let i_lo = (((bbox[0] - g.lon0) / g.dlon).floor().max(0.0) as usize).min(g.ni - 1);
    let i_hi = (((bbox[2] - g.lon0) / g.dlon).ceil().max(0.0) as usize).min(g.ni - 1);
    let j_lo = (((bbox[1] - g.lat0) / g.dlat).floor().max(0.0) as usize).min(g.nj - 1);
    let j_hi = (((bbox[3] - g.lat0) / g.dlat).ceil().max(0.0) as usize).min(g.nj - 1);
    let (wi, wj) = (i_hi - i_lo + 1, j_hi - j_lo + 1);
    let stride = (((wi * wj) as f64 / max_points.max(1) as f64).sqrt().ceil() as usize).max(1);
    let (mut ni, mut nj) = (0, 0);
    let mut out: Vec<Option<f32>> = Vec::new();
    for j in (j_lo..=j_hi).step_by(stride) {
        nj += 1;
        ni = 0;
        for i in (i_lo..=i_hi).step_by(stride) {
            ni += 1;
            let x = vals[j * g.ni + i];
            // a tenth is plenty for weather; sea temperature fronts and salinity need finer
            let k = match layer.as_str() {
                "sst" | "subt" | "sss" | "subs" | "skin" => 100.0,
                "chl" => 1000.0,
                "ssh" => 100.0,
                _ => 10.0,
            };
            out.push(x.is_finite().then(|| (x * k).round() / k));
        }
    }
    Some(serde_json::json!({
        "lon0": g.lon0 + i_lo as f64 * g.dlon - turn,
        "lat0": g.lat0 + j_lo as f64 * g.dlat,
        "dlon": g.dlon * stride as f64,
        "dlat": g.dlat * stride as f64,
        "ni": ni, "nj": nj, "values": out,
    }))
}

/// How much the loaded GRIBs in `paths` disagree at time `t` over `bbox` (see
/// `grib::compare`): `metric` is vector, speed, direction, pressure or trend.
#[tauri::command]
pub(crate) fn grib_compare(paths: Vec<String>, metric: String, t: i64, bbox: [f64; 4], max_points: usize) -> Result<serde_json::Value, String> {
    let m = grib::compare::Metric::parse(&metric).ok_or("unknown comparison")?;
    let files: Vec<(String, Arc<grib::GribFile>)> = {
        let g = GRIBS.lock().unwrap();
        paths.iter().filter_map(|p| g.get(p).map(|f| (p.clone(), f.clone()))).collect()
    };
    let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
    let r = grib::compare::agreement(&refs, m, t, bbox, max_points.clamp(100, 40_000)).ok_or(if m == grib::compare::Metric::Trend {
        "Needs two or more runs covering this time."
    } else {
        "Needs two or more GRIBs covering this time and area."
    })?;
    let used: Vec<serde_json::Value> = r
        .used
        .iter()
        .map(|&i| {
            let (name, run) = files[i].1.model();
            serde_json::json!({ "path": files[i].0, "name": name, "run": run })
        })
        .collect();
    Ok(serde_json::json!({
        "lon0": r.lon0, "lat0": r.lat0, "dlon": r.dlon, "dlat": r.dlat, "ni": r.ni, "nj": r.nj,
        "values": r.values, "used": used, "layers": r.layers, "members": r.members,
    }))
}

pub(crate) fn loaded(paths: &[String]) -> Vec<(String, Arc<grib::GribFile>)> {
    let g = GRIBS.lock().unwrap();
    if paths.is_empty() {
        let mut v: Vec<_> = g.iter().map(|(k, f)| (k.clone(), f.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    } else {
        paths.iter().filter_map(|p| g.get(p).map(|f| (p.clone(), f.clone()))).collect()
    }
}

pub(crate) fn file_json(files: &[(String, Arc<grib::GribFile>)], i: usize) -> serde_json::Value {
    let (name, run) = files[i].1.model();
    serde_json::json!({ "path": files[i].0, "name": name, "run": run, "members": files[i].1.members().len() })
}

pub(crate) fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Every loaded GRIB (or those in `paths`) at one point over its whole span.
#[tauri::command]
pub(crate) async fn grib_meteogram(paths: Vec<String>, lat: f64, lon: f64) -> Vec<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = loaded(&paths);
        let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
        grib::compare::meteogram(&refs, lat, lon)
            .into_iter()
            .map(|s| {
                let pts: Vec<serde_json::Value> = s
                    .points
                    .iter()
                    .zip(s.tws_range.iter())
                    .map(|(&(t, tws, twd, gust, msl), rng)| {
                        serde_json::json!({ "t": t, "tws": tws.map(r1), "twd": twd.map(f64::round), "gust": gust.map(r1), "mslp": msl.map(r1),
                            "lo": rng.map(|r| r1(r.0)), "hi": rng.map(|r| r1(r.1)) })
                    })
                    .collect();
                let mut j = file_json(&files, s.file);
                j["points"] = serde_json::Value::Array(pts);
                j
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

pub(crate) fn score_json(files: &[(String, Arc<grib::GribFile>)], s: &grib::verify::Score) -> serde_json::Value {
    let mut j = file_json(files, s.file);
    j["lead_h"] = s.lead_h.into();
    j["n"] = s.n.into();
    j["rms"] = r1(s.rms_kn).into();
    j["bias"] = r1(s.speed_bias_kn).into();
    j["dir"] = s.dir_abs_deg.map(f64::round).into();
    j["veer"] = s.veer_deg.map(f64::round).into();
    j["msl"] = s.msl_bias_hpa.map(r1).into();
    j
}

/// Verify times among the loaded GRIBs: a run's hour 0 that earlier runs forecast.
#[tauri::command]
pub(crate) fn grib_verify_times(paths: Vec<String>) -> Vec<serde_json::Value> {
    let files = loaded(&paths);
    let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
    grib::verify::verify_times(&refs)
        .into_iter()
        .map(|vt| {
            serde_json::json!({
                "t": vt.t,
                "refs": vt.refs.iter().map(|&i| file_json(&files, i)).collect::<Vec<_>>(),
                "forecasts": vt.forecasts.iter().map(|f| { let mut j = file_json(&files, f.file); j["lead_h"] = f.lead_h.into(); j }).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// Scores of every earlier forecast against the hour-0 reference at verify time `t`.
#[tauri::command]
pub(crate) async fn grib_verify_scores(paths: Vec<String>, t: i64, bbox: [f64; 4]) -> Result<Vec<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = loaded(&paths);
        let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
        let vt = grib::verify::verify_times(&refs).into_iter().find(|v| v.t == t).ok_or("No GRIB has this time as hour 0.")?;
        Ok(grib::verify::scores(&refs, &vt, bbox, 6000).iter().map(|s| score_json(&files, s)).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Scores averaged over every verify time by model and lead band.
#[tauri::command]
pub(crate) async fn grib_verify_track(paths: Vec<String>, bbox: [f64; 4]) -> Vec<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = loaded(&paths);
        let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
        grib::verify::track_record(&refs, bbox, 4000)
            .into_iter()
            .map(|r| {
                serde_json::json!({ "model": r.model, "band": r.band, "count": r.count, "rms": r1(r.rms_kn), "bias": r1(r.speed_bias_kn),
                    "dir": r.dir_abs_deg.map(f64::round), "veer": r.veer_deg.map(f64::round), "msl": r.msl_bias_hpa.map(r1) })
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// The error field of the forecast in `forecast` against the later run(s) at time `at` (their hour 0 when `at` is the verify time `t`); `kind` is vector, speed or direction.
#[tauri::command]
pub(crate) fn grib_verify_grid(
    paths: Vec<String>,
    t: i64,
    at: i64,
    forecast: String,
    kind: String,
    bbox: [f64; 4],
    max_points: usize,
) -> Result<serde_json::Value, String> {
    let files = loaded(&paths);
    let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
    let k = match kind.as_str() {
        "speed" => grib::verify::ErrorKind::Speed,
        "direction" => grib::verify::ErrorKind::Direction,
        _ => grib::verify::ErrorKind::Vector,
    };
    let fi = files.iter().position(|(p, _)| *p == forecast).ok_or("That GRIB is not loaded.")?;
    let vt = grib::verify::verify_times(&refs).into_iter().find(|v| v.t == t).ok_or("No GRIB has this time as hour 0.")?;
    let r = grib::verify::error_grid(&refs, &vt, fi, k, at, bbox, max_points.clamp(100, 40_000))
        .ok_or("Nothing to compare at this time: the forecast and the later run must both cover it (the later run starts at the verify time).")?;
    Ok(serde_json::json!({ "lon0": r.lon0, "lat0": r.lat0, "dlon": r.dlon, "dlat": r.dlat, "ni": r.ni, "nj": r.nj, "values": r.values,
        "used": r.used.iter().map(|&i| file_json(&files, i)).collect::<Vec<_>>() }))
}

/// Every loaded GRIB at one point and time: the quick agreement check.
#[tauri::command]
pub(crate) fn grib_point(lat: f64, lon: f64, t: i64) -> Vec<serde_json::Value> {
    let files: Vec<(String, Arc<grib::GribFile>)> = GRIBS.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    files
        .into_iter()
        .map(|(k, f)| {
            let mut j = point_values(&f, lat, lon, t);
            j["path"] = k.clone().into();
            j["name"] = f.summary().name.into();
            j
        })
        .collect()
}

/// Every value a GRIB has at a point and time, in display units (kn, hPa, m, s, °C…).
pub(crate) fn point_values(f: &grib::GribFile, lat: f64, lon: f64, t: i64) -> serde_json::Value {
    let w = f.wind(lat, lon, t);
    let kn = |x: Option<f64>| x.map(|v| (v * 3600.0 / 1852.0 * 10.0).round() / 10.0);
    serde_json::json!({
                    "tws": w.map(|w| (w.tws * 10.0).round() / 10.0),
                    "twd": w.map(|w| w.twd.round()),
                    "gust": kn(f.value(grib::Param::Gust, lat, lon, t)),
                    "mslp": f.value(grib::Param::Mslp, lat, lon, t).map(|p| (p / 10.0).round() / 10.0),
                    "wave": f.value(grib::Param::WaveHeight, lat, lon, t).map(|h| (h * 10.0).round() / 10.0),
                    "period": f.value(grib::Param::WavePeriod, lat, lon, t).map(|h| h.round()),
                    "wavedir": f.value(grib::Param::WaveDir, lat, lon, t).map(|d| d.round()),
                    "temp": f.value(grib::Param::AirTemp, lat, lon, t).map(|k| ((k - 273.15) * 10.0).round() / 10.0),
                    "rain": f.value(grib::Param::Rain, lat, lon, t).map(|r| (r * 36000.0).round() / 10.0),
                    "sst": f.value(grib::Param::SeaTemp, lat, lon, t).map(|k| ((k - 273.15) * 100.0).round() / 100.0),
                    "sss": f.value(grib::Param::Salinity, lat, lon, t).map(|s| (s * 100.0).round() / 100.0),
                    "subt": f.value(grib::Param::SubTemp, lat, lon, t).map(|k| ((k - 273.15) * 100.0).round() / 100.0),
                    "subs": f.value(grib::Param::SubSalinity, lat, lon, t).map(|s| (s * 100.0).round() / 100.0),
                    "skin": f.value(grib::Param::SkinTemp, lat, lon, t).map(|k| ((k - 273.15) * 100.0).round() / 100.0),
                    "chl": f.value(grib::Param::Chlorophyll, lat, lon, t).map(|c| (c * 1000.0).round() / 1000.0),
                    "ssh": f.value(grib::Param::SeaLevel, lat, lon, t).map(|h| (h * 100.0).round() / 100.0),
                    "depth": f.series(grib::Param::SubTemp).first().or(f.series(grib::Param::SubSalinity).first()).map(|x| x.surface_value),
                    "cur": f.current(lat, lon, t).map(|(u, v)| serde_json::json!({
                        "kn": ((u * u + v * v).sqrt() * 3600.0 / 1852.0 * 100.0).round() / 100.0,
                        "set": nav_core::norm360(u.atan2(v).to_degrees()).round(),
                    })),
    })
}

/// The weather along a route: at each (lat, lon, t), each value from the first loaded
/// GRIB that has it, the shown one first (so its wind wins; waves or sea data can come
/// from another file).
#[tauri::command]
pub(crate) async fn grib_along(first: Option<String>, pts: Vec<(f64, f64, i64)>) -> Result<Vec<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut files: Vec<(String, Arc<grib::GribFile>)> = GRIBS.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        files.sort_by(|(a, _), (b, _)| (Some(a) != first.as_ref(), a).cmp(&(Some(b) != first.as_ref(), b)));
        pts.iter()
            .map(|&(lat, lon, t)| {
                let mut out = serde_json::Map::new();
                let mut sources = serde_json::Map::new();
                for (path, f) in &files {
                    if let serde_json::Value::Object(m) = point_values(f, lat, lon, t) {
                        for (k, v) in m {
                            if !v.is_null() && !out.contains_key(&k) {
                                sources.insert(k.clone(), path.clone().into());
                                out.insert(k, v);
                            }
                        }
                    }
                }
                out.insert("_sources".into(), serde_json::Value::Object(sources));
                serde_json::Value::Object(out)
            })
            .collect()
    })
    .await
    .map_err(|e| format!("Could not read forecast along route: {e}"))
}

/// Ask where to save a weather snapshot (PNG) and write it; returns the path, or None if cancelled.
/// The picture comes as the raw request body (not a JSON list of numbers), the suggested file
/// name in the `x-suggest` header.
#[tauri::command]
pub(crate) async fn weather_save_png(app: tauri::AppHandle, request: tauri::ipc::Request<'_>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("expected the picture as raw bytes".into());
    };
    if !data.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err("not a PNG picture".into());
    }
    let mut d = app.dialog().file().set_title("Save the weather view").add_filter("PNG image", &["png"]);
    if let Some(s) = request.headers().get("x-suggest").and_then(|v| v.to_str().ok()) {
        d = d.set_file_name(s);
    }
    let Some(p) = d.blocking_save_file() else { return Ok(None) };
    let mut p = p.into_path().map_err(|e| e.to_string())?;
    if p.extension().is_none_or(|x| !x.eq_ignore_ascii_case("png")) {
        p.set_extension("png");
    }
    std::fs::write(&p, data).map_err(|e| e.to_string())?;
    Ok(Some(p.to_string_lossy().into_owned()))
}
