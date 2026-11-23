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
    serde_json::json!({ "bytes": gribdl::estimate_bytes(&req), "download": gribdl::download_bytes(&req), "steps": steps.len(), "last": steps.last(), "step": gribdl::ocean_step(&req).map(|(res, k)| serde_json::json!({ "res": res, "k": k })) })
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

/// Pick GRIB files anywhere on the PC and copy them into the first GRIB folder. Returns the new paths
/// (empty when the dialog is cancelled).
#[tauri::command]
pub(crate) async fn grib_add(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("Add GRIB files")
        .add_filter("GRIB files", &["grb", "grb2", "grib", "grib2"])
        .add_filter("All files", &["*"])
        .blocking_pick_files();
    let Some(list) = picked else { return Ok(Vec::new()) };
    let Some(dir) = grib_dirs().into_iter().next() else { return Err("There is no GRIB folder to copy into.".to_string()) };
    let mut out = Vec::new();
    for f in list {
        let src = f.into_path().map_err(|e| e.to_string())?;
        let Some(name) = src.file_name() else { continue };
        let dst = dir.join(name);
        if dst != src {
            std::fs::copy(&src, &dst).map_err(|e| format!("Could not copy {}: {e}", src.display()))?;
        }
        out.push(dst.to_string_lossy().into_owned());
    }
    Ok(out)
}

/// Delete GRIB files from the GRIB folders: never a loaded file, and never from qtVlm's own folder.
/// Returns how many were deleted.
#[tauri::command]
pub(crate) fn grib_delete(paths: Vec<String>) -> Result<usize, String> {
    let dirs = grib_dirs();
    let loaded = GRIBS.lock().unwrap();
    let mut n = 0;
    for p in paths {
        let path = std::path::PathBuf::from(&p);
        let in_folder = path.parent().is_some_and(|d| dirs.iter().any(|g| g.as_path() == d));
        if loaded.contains_key(&p) || !in_folder || p.to_ascii_lowercase().contains("qtvlm") {
            continue;
        }
        std::fs::remove_file(&path).map_err(|e| format!("Could not delete {p}: {e}"))?;
        n += 1;
    }
    Ok(n)
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

fn finite_range(values: &[f32]) -> Option<(f32, f32)> {
    values.iter().copied().filter(|v| v.is_finite()).fold(None, |range, v| {
        Some(match range {
            Some((lo, hi)) => (f32::min(lo, v), f32::max(hi, v)),
            None => (v, v),
        })
    })
}

fn shading_range(values: &[f32], precision: f32) -> Option<(f32, f32)> {
    finite_range(values).map(|(lo, hi)| (lo.min((lo * precision).round() / precision), hi.max((hi * precision).round() / precision)))
}

/// One quantity on a thinned grid inside `bbox` at time `t`, for shading and
/// contours: "wind" and "gust" (kn), "waves", "swell", "windwaves" (m),
/// "period" (s), "wavedir" (deg from), "rain" (mm/h), "cape" (J/kg), "mslp" (hPa), "temp" (°C).
#[tauri::command]
pub(crate) fn grib_grid(path: String, layer: String, t: i64, bbox: [f64; 4], max_points: usize, depth: Option<f64>) -> Option<serde_json::Value> {
    use grib::stack::Derived;
    use grib::Param as P;
    let f = GRIBS.lock().unwrap().get(&path)?.clone();
    // a layer at depth: the chosen depth when the file has several, else its one depth
    let at_depth = |p: P| match depth {
        Some(d) if f.depths(p).len() > 1 => f.grid_at_depth(p, d, t),
        _ => f.grid_at(p, t),
    };
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
        "sst_err" => f.grid_at(P::SeaTempError, t)?,
        "sss" => f.grid_at(P::Salinity, t)?,
        "subt" => {
            let (g, v) = at_depth(P::SubTemp)?;
            (g, v.iter().map(|x| x - 273.15).collect())
        }
        "subs" => at_depth(P::SubSalinity)?,
        "subcur" => {
            let ((g, u), (_, v)) = (at_depth(P::SubCurrentU)?, at_depth(P::SubCurrentV)?);
            (g, u.iter().zip(&v).map(|(a, b)| (a * a + b * b).sqrt() * KN).collect())
        }
        "thermo_depth" => grib::stack::derived_grid(&f, Derived::ThermoDepth, t)?,
        "thermo_str" => grib::stack::derived_grid(&f, Derived::ThermoStrength, t)?,
        "shear_depth" => grib::stack::derived_grid(&f, Derived::ShearDepth, t)?,
        "shear_str" => grib::stack::derived_grid(&f, Derived::ShearStrength, t)?,
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
        "mld" => f.grid_at(P::MixedLayer, t)?,
        // plankton and production span decades too: log10, like chlorophyll
        "phyc" | "nppv" => {
            let (g, v) = f.grid_at(if layer == "phyc" { P::Phytoplankton } else { P::PrimaryProduction }, t)?;
            (g, v.iter().map(|x| if *x > 0.0 { x.log10() } else { f32::NAN }).collect())
        }
        "o2" => f.grid_at(P::Oxygen, t)?,
        "ph" => f.grid_at(P::Ph, t)?,
        // sea ice: per cent of the sea covered
        "ice" => {
            let (g, v) = f.grid_at(P::SeaIce, t)?;
            (g, v.iter().map(|x| x * 100.0).collect())
        }
        // the way the water flows, degrees true
        "currentdir" => {
            let ((g, u), (_, v)) = (f.grid_at(P::CurrentU, t)?, f.grid_at(P::CurrentV, t)?);
            (g, u.iter().zip(&v).map(|(a, b)| (a.atan2(*b).to_degrees() + 360.0) % 360.0).collect())
        }
        _ => return None,
    };
    // Include both the original values and the rounded values returned for shading.
    let precision = match layer.as_str() {
        "sst" | "sst_err" | "subt" | "sss" | "subs" | "skin" | "thermo_str" | "shear_str" | "subcur" => 100.0,
        "chl" | "phyc" | "nppv" | "ph" => 1000.0,
        "ssh" => 100.0,
        _ => 10.0,
    };
    let range = shading_range(&vals, precision);
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
            out.push(x.is_finite().then(|| (x * precision).round() / precision));
        }
    }
    Some(serde_json::json!({
        "lon0": g.lon0 + i_lo as f64 * g.dlon - turn,
        "lat0": g.lat0 + j_lo as f64 * g.dlat,
        "dlon": g.dlon * stride as f64,
        "dlat": g.dlat * stride as f64,
        "ni": ni, "nj": nj, "values": out, "range": range,
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
pub(crate) fn grib_point(lat: f64, lon: f64, t: i64, depth: Option<f64>) -> Vec<serde_json::Value> {
    let files: Vec<(String, Arc<grib::GribFile>)> = GRIBS.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    files
        .into_iter()
        .map(|(k, f)| {
            let mut j = point_values(&f, lat, lon, t, depth);
            j["path"] = k.clone().into();
            j["name"] = f.summary().name.into();
            j
        })
        .collect()
}

/// The water column at a point and time from a file with a depth stack (temperature, salinity,
/// current at each depth, with the thermocline and shear), or null.
#[tauri::command]
pub(crate) fn grib_profile(path: String, lat: f64, lon: f64, t: i64) -> Option<grib::stack::Profile> {
    let f = GRIBS.lock().unwrap().get(&path)?.clone();
    grib::stack::profile(&f, lat, lon, t)
}

/// A section through the depth stack along a line ([lon, lat] corners) at time `t`, `n` points.
#[tauri::command]
pub(crate) async fn grib_section(path: String, line: Vec<[f64; 2]>, t: i64, n: usize) -> Option<grib::stack::Section> {
    let f = GRIBS.lock().unwrap().get(&path)?.clone();
    tauri::async_runtime::spawn_blocking(move || grib::stack::section(&f, &line, n, t)).await.ok().flatten()
}

/// Every value a GRIB has at a point and time, in display units (kn, hPa, m, s, °C…).
/// With a depth stack, the values "at depth" are at the depth in the file nearest `depth` (50 m if none given).
pub(crate) fn point_values(f: &grib::GribFile, lat: f64, lon: f64, t: i64, depth: Option<f64>) -> serde_json::Value {
    let w = f.wind(lat, lon, t);
    let kn = |x: Option<f64>| x.map(|v| (v * 3600.0 / 1852.0 * 10.0).round() / 10.0);
    // a file with a depth stack: its values at the depth chosen (the profile shows the rest)
    let stack_depth = f.nearest_depth(grib::Param::SubTemp, depth.unwrap_or(50.0)).filter(|_| f.depths(grib::Param::SubTemp).len() > 1);
    let sub = |p: grib::Param| match stack_depth {
        Some(d) => f.value_at_depth(p, d, lat, lon, t),
        None => f.value(p, lat, lon, t),
    };
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
                    "sst_err": f.value(grib::Param::SeaTempError, lat, lon, t).map(|e| (e * 100.0).round() / 100.0),
                    "sss": f.value(grib::Param::Salinity, lat, lon, t).map(|s| (s * 100.0).round() / 100.0),
                    "subt": sub(grib::Param::SubTemp).map(|k| ((k - 273.15) * 100.0).round() / 100.0),
                    "subs": sub(grib::Param::SubSalinity).map(|s| (s * 100.0).round() / 100.0),
                    "skin": f.value(grib::Param::SkinTemp, lat, lon, t).map(|k| ((k - 273.15) * 100.0).round() / 100.0),
                    "chl": f.value(grib::Param::Chlorophyll, lat, lon, t).map(|c| (c * 1000.0).round() / 1000.0),
                    "ssh": f.value(grib::Param::SeaLevel, lat, lon, t).map(|h| (h * 100.0).round() / 100.0),
                    "mld": f.value(grib::Param::MixedLayer, lat, lon, t).map(|d| (d * 10.0).round() / 10.0),
                    "phyc": f.value(grib::Param::Phytoplankton, lat, lon, t).map(|c| (c * 1000.0).round() / 1000.0),
                    "nppv": f.value(grib::Param::PrimaryProduction, lat, lon, t).map(|c| (c * 100.0).round() / 100.0),
                    "o2": f.value(grib::Param::Oxygen, lat, lon, t).map(|c| (c * 10.0).round() / 10.0),
                    "ph": f.value(grib::Param::Ph, lat, lon, t).map(|c| (c * 1000.0).round() / 1000.0),
                    "ice": f.value(grib::Param::SeaIce, lat, lon, t).map(|c| (c * 100.0).round()),
                    "depth": stack_depth.or_else(|| f.series(grib::Param::SubTemp).first().or(f.series(grib::Param::SubSalinity).first()).map(|x| x.surface_value)),
                    "subcur": sub(grib::Param::SubCurrentU).zip(sub(grib::Param::SubCurrentV)).map(|(u, v)| serde_json::json!({
                        "kn": ((u * u + v * v).sqrt() * 3600.0 / 1852.0 * 100.0).round() / 100.0,
                        "set": nav_core::norm360(u.atan2(v).to_degrees()).round(),
                    })),
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
                    if let serde_json::Value::Object(m) = point_values(f, lat, lon, t, None) {
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

    let suggest = request.headers().get("x-suggest").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut d = app.dialog().file().set_title("Save the weather view").add_filter("PNG image", &["png"]);
        if let Some(s) = suggest {
            d = d.set_file_name(&s);
        }
        d.blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(p) = picked else { return Ok(None) };
    let mut p = p.into_path().map_err(|e| e.to_string())?;
    if p.extension().is_none_or(|x| !x.eq_ignore_ascii_case("png")) {
        p.set_extension("png");
    }
    std::fs::write(&p, data).map_err(|e| e.to_string())?;
    Ok(Some(p.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::{finite_range, shading_range};

    #[test]
    fn full_grid_range_includes_extremes_and_ignores_missing_values() {
        assert_eq!(finite_range(&[f32::NAN, 1.0, 23.2, f32::INFINITY, -2.0]), Some((-2.0, 23.2)));
        assert_eq!(finite_range(&[f32::NAN]), None);
        assert_eq!(finite_range(&[4.0]), Some((4.0, 4.0)));
        assert_eq!(shading_range(&[1.03, 1.06], 10.0), Some((1.0, 1.1)));
    }
}

// ---------- sea temperature products against the boat (P1.4) ----------

/// The boat's sea temperature readings from the library's tracks (`track_ids`, or every
/// track that logged a temperature), only inside `bbox` and after `since` when given,
/// thinned to one every 10 minutes or 2 km.
fn boat_sst_obs(track_ids: &[String], bbox: Option<[f64; 4]>, since: Option<i64>) -> Result<(Vec<grib::sstverify::Obs>, usize), String> {
    let lib = crate::library_cmd::lib()?;
    let inside = |lat: f64, lon: f64| {
        bbox.is_none_or(|b| {
            let lo = grib::sstverify::lon_in(b, lon);
            lat >= b[1] && lat <= b[3] && lo >= b[0] && lo <= b[2]
        })
    };
    let ids: Vec<String> = if track_ids.is_empty() {
        lib.tracks()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|t| t.has_temp)
            .filter(|t| since.is_none_or(|s| t.stats.end.is_none_or(|e| e >= s)))
            .filter(|t| match (bbox, t.stats.bbox) {
                (Some(b), Some(tb)) => {
                    let w = grib::sstverify::lon_in(b, tb[0]);
                    tb[1] <= b[3] && tb[3] >= b[1] && (w <= b[2] || grib::sstverify::lon_in(b, tb[2]) >= b[0] || tb[2] - tb[0] >= 360.0)
                }
                _ => true,
            })
            .map(|t| t.id)
            .collect()
    } else {
        track_ids.to_vec()
    };
    let mut obs = Vec::new();
    let mut tracks = 0;
    for id in &ids {
        let Ok(t) = lib.track(id) else { continue };
        let before = obs.len();
        for p in t.segments.iter().flatten() {
            if let (Some(tt), Some(temp)) = (p.t, p.temp) {
                if since.is_some_and(|s| tt < s) || !inside(p.lat, p.lon) {
                    continue;
                }
                obs.push(grib::sstverify::Obs { t: tt, lat: p.lat, lon: p.lon, temp });
            }
        }
        if obs.len() > before {
            tracks += 1;
        }
    }
    Ok((grib::sstverify::thin(obs, 600, 2000.0), tracks))
}

/// Sea temperature files in the weather folders that are not loaded but cover the readings'
/// times (± `max_age_s`) and area, newest first, at most `limit`.
fn disk_sst_files(obs: &[grib::sstverify::Obs], max_age_s: i64, limit: usize) -> Vec<String> {
    let (Some(t0), Some(t1)) = (obs.iter().map(|o| o.t).min(), obs.iter().map(|o| o.t).max()) else { return Vec::new() };
    let lat = (obs.iter().map(|o| o.lat).fold(f64::INFINITY, f64::min), obs.iter().map(|o| o.lat).fold(f64::NEG_INFINITY, f64::max));
    let loaded = GRIBS.lock().unwrap().keys().cloned().collect::<std::collections::BTreeSet<String>>();
    let mut out = Vec::new();
    for f in grib_files_blocking() {
        let Some(path) = f["path"].as_str() else { continue };
        if loaded.contains(path) {
            continue;
        }
        let Some(info) = f.get("info").filter(|i| !i.is_null()) else { continue };
        let runs = info["runs"].as_array().cloned().unwrap_or_default();
        let hit = runs.iter().any(|r| {
            let sea = r["contents"].as_array().is_some_and(|c| c.iter().any(|x| x.as_str() == Some("sea temperature")));
            let (first, last) = (r["first"].as_i64().unwrap_or(i64::MAX), r["last"].as_i64().unwrap_or(i64::MIN));
            let b: Vec<f64> = r["bbox"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
            let in_time = first - max_age_s <= t1 && last + max_age_s >= t0;
            let in_lat = b.len() == 4 && b[1] <= lat.1 && b[3] >= lat.0;
            let in_lon = b.len() == 4
                && obs.iter().any(|o| {
                    let lo = grib::sstverify::lon_in([b[0], b[1], b[2], b[3]], o.lon);
                    lo >= b[0] && lo <= b[2]
                });
            sea && in_time && in_lat && in_lon
        });
        if hit {
            out.push(path.to_string());
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Score every sea temperature product against the boat's own water temperature along its
/// tracks: the loaded files, plus (with `from_disk`) the sea temperature files in the weather
/// folders that cover the tracks. `max_age_h`: the oldest field matched to a reading.
#[tauri::command]
pub(crate) async fn sst_verify(
    track_ids: Vec<String>,
    bbox: Option<[f64; 4]>,
    since: Option<i64>,
    from_disk: bool,
    max_age_h: Option<f64>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let max_age_s = (max_age_h.unwrap_or(36.0).clamp(1.0, 240.0) * 3600.0) as i64;
        let (obs, tracks) = boat_sst_obs(&track_ids, bbox, since)?;
        let mut files: Vec<(String, Arc<grib::GribFile>)> = loaded(&[]);
        files.retain(|(_, f)| !f.times(grib::Param::SeaTemp).is_empty());
        let mut skipped = 0;
        if from_disk && !obs.is_empty() {
            for p in disk_sst_files(&obs, max_age_s, 120) {
                match grib::GribFile::open(&p) {
                    Ok(f) => files.push((p, Arc::new(f))),
                    Err(_) => skipped += 1,
                }
            }
        }
        let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
        let scores = grib::sstverify::score(&refs, &obs, max_age_s);
        let r2 = |x: f64| (x * 100.0).round() / 100.0;
        let products: Vec<serde_json::Value> = scores
            .iter()
            .map(|s| {
                // at most 1,500 pairs for the plot, evenly spaced
                let step = s.pairs.len().div_ceil(1500).max(1);
                let pairs: Vec<serde_json::Value> =
                    s.pairs.iter().step_by(step).map(|p| serde_json::json!([p.t, r2(p.boat), r2(p.product), p.lat, p.lon])).collect();
                serde_json::json!({
                    "product": s.product, "n": s.n, "covered": s.covered, "coverage": r2(s.coverage()),
                    "bias": r2(s.bias), "mae": r2(s.mae), "rms": r2(s.rms), "sd": r2(s.sd), "age_h": (s.age_h * 10.0).round() / 10.0,
                    "files": s.files.iter().map(|&i| files[i].0.clone()).collect::<Vec<_>>(),
                    "pairs": pairs,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "readings": obs.len(), "tracks": tracks, "files": files.len(), "skipped": skipped,
            "first": obs.first().map(|o| o.t), "last": obs.last().map(|o| o.t),
            "products": products,
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}
