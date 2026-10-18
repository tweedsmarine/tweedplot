//! racenav desktop app: wires the Rust crates to the MapLibre front end.
//!
//! Events sent to the front end:
//! - `sk-status`   `{ connected, detail }`
//! - `vessel`      own-vessel state (degrees true, knots, metres)
//! - `chart-ready` `{ tables, dir }` once the chart folder has been opened
//!
//! The `chart` URI scheme serves vector tiles: `chart://localhost/{z}/{x}/{y}.pbf`
//! (`http://chart.localhost/...` on Windows).

use std::path::PathBuf;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tauri::http::{header, Response, StatusCode};
use tauri::{Emitter, Manager};

static CHARTS: std::sync::RwLock<Option<Arc<chart::ChartStore>>> = std::sync::RwLock::new(None);
/// The open chart store (None until the charts are open).
fn charts() -> Option<Arc<chart::ChartStore>> {
    CHARTS.read().unwrap().clone()
}
static DATA_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DATA_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static GRIBS: Mutex<BTreeMap<String, Arc<grib::GribFile>>> = Mutex::new(BTreeMap::new());
static LIBRARY: OnceLock<library::Library> = OnceLock::new();
/// Latest Signal K connection state, for a page that loads after the event was sent.
static SK_STATUS: Mutex<Option<(bool, String)>> = Mutex::new(None);

#[tauri::command]
fn sk_status() -> serde_json::Value {
    match SK_STATUS.lock().unwrap().clone() {
        Some((connected, detail)) => serde_json::json!({ "connected": connected, "detail": detail }),
        None => serde_json::json!({ "connected": false, "detail": "connecting" }),
    }
}

/// GRIB folders scanned for files: `RACENAV_GRIB_DIR`, `C:\dev\racenav-data\grib`
/// and qtVlm's own GRIB folder (Meltemus downloads land there).
fn grib_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("RACENAV_GRIB_DIR") {
        v.push(d.into());
    }
    if let Some(d) = DOWNLOAD_DIR.get() {
        v.push(d.clone());
    }
    v.push(r"C:\dev\racenav-data\grib".into());
    v.push(r"C:\Program Files (x86)\qtVlm\grib".into());
    v.retain(|d| d.is_dir());
    v.dedup();
    v
}

// ---------- GRIB downloads (NOMADS GFS) ----------

/// Downloaded GRIBs go here: `RACENAV_GRIB_DIR`, else `C:\dev\racenav-data\grib`
/// when racenav-data exists, else `<app data>\grib`.
static DOWNLOAD_DIR: OnceLock<PathBuf> = OnceLock::new();
static DL_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DL_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn download_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_GRIB_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data");
    if dev.is_dir() {
        return dev.join("grib");
    }
    app.path().app_data_dir().map(|p| p.join("grib")).unwrap_or_else(|_| PathBuf::from("grib"))
}

#[tauri::command]
fn gribdl_models() -> Vec<gribdl::ModelInfo> {
    gribdl::Model::all().iter().map(|m| m.info()).collect()
}

#[tauri::command]
fn gribdl_estimate(req: gribdl::Request) -> serde_json::Value {
    let steps = req.model.hours(req.hours, req.every);
    serde_json::json!({ "bytes": gribdl::estimate_bytes(&req), "download": gribdl::download_bytes(&req), "steps": steps.len(), "last": steps.last() })
}

/// Find the newest complete run and download it, cut to the area. Progress
/// comes as `gribdl-progress` events; returns the file written.
#[tauri::command]
async fn gribdl_start(app: tauri::AppHandle, req: gribdl::Request) -> Result<serde_json::Value, String> {
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
async fn grib_combine(a: String, b: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        grib::combine::combine(std::path::Path::new(&a), std::path::Path::new(&b)).map(|p| p.to_string_lossy().into_owned()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open a mailto: link (a Saildocs request) in the mail program, or a web link from the manual in the browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // mail (Saildocs requests) and web links from the manual; nothing that could run a program
    let ok = url.starts_with("mailto:") || url.starts_with("https://") || url.starts_with("http://");
    if !ok || url.chars().any(|c| c.is_control() || (c.is_whitespace() && !url.starts_with("mailto:")) || c == '"') {
        return Err("only mailto: and web links".into());
    }
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &url]).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn gribdl_cancel() {
    DL_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[tauri::command]
async fn grib_files() -> Vec<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(grib_files_blocking).await.unwrap_or_default()
}

fn grib_files_blocking() -> Vec<serde_json::Value> {
    let loaded = GRIBS.lock().unwrap();
    let mut out = Vec::new();
    for dir in grib_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
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
fn scan_cached(p: &std::path::Path, mtime: u64, size: u64) -> Option<grib::FileInfo> {
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
async fn grib_load(app: tauri::AppHandle, path: String) -> Result<grib::Summary, String> {
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
fn grib_unload(path: String) {
    GRIBS.lock().unwrap().remove(&path);
}

#[tauri::command]
fn grib_loaded() -> Vec<grib::Summary> {
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
fn grib_wind_grid(path: String, t: i64, bbox: [f64; 4], max_points: usize) -> Option<serde_json::Value> {
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
fn grib_grid(path: String, layer: String, t: i64, bbox: [f64; 4], max_points: usize) -> Option<serde_json::Value> {
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
        "gust" => { let (g, v) = f.grid_at(P::Gust, t)?; (g, v.iter().map(|x| x * KN).collect()) }
        "waves" => f.grid_at(P::WaveHeight, t)?,
        "swell" => f.grid_at(P::SwellHeight, t)?,
        "windwaves" => f.grid_at(P::WindWaveHeight, t)?,
        "period" => f.grid_at(P::WavePeriod, t)?,
        "wavedir" => f.grid_at(P::WaveDir, t)?,
        "rain" => { let (g, v) = f.grid_at(P::Rain, t)?; (g, v.iter().map(|x| x * 3600.0).collect()) }
        "cape" => f.grid_at(P::Cape, t)?,
        "mslp" => { let (g, v) = f.grid_at(P::Mslp, t)?; (g, v.iter().map(|x| x / 100.0).collect()) }
        "temp" => { let (g, v) = f.grid_at(P::AirTemp, t)?; (g, v.iter().map(|x| x - 273.15).collect()) }
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
            out.push(x.is_finite().then(|| (x * 10.0).round() / 10.0));
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
fn grib_compare(paths: Vec<String>, metric: String, t: i64, bbox: [f64; 4], max_points: usize) -> Result<serde_json::Value, String> {
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

fn loaded(paths: &[String]) -> Vec<(String, Arc<grib::GribFile>)> {
    let g = GRIBS.lock().unwrap();
    if paths.is_empty() {
        let mut v: Vec<_> = g.iter().map(|(k, f)| (k.clone(), f.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    } else {
        paths.iter().filter_map(|p| g.get(p).map(|f| (p.clone(), f.clone()))).collect()
    }
}

fn file_json(files: &[(String, Arc<grib::GribFile>)], i: usize) -> serde_json::Value {
    let (name, run) = files[i].1.model();
    serde_json::json!({ "path": files[i].0, "name": name, "run": run, "members": files[i].1.members().len() })
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Every loaded GRIB (or those in `paths`) at one point over its whole span.
#[tauri::command]
async fn grib_meteogram(paths: Vec<String>, lat: f64, lon: f64) -> Vec<serde_json::Value> {
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

fn score_json(files: &[(String, Arc<grib::GribFile>)], s: &grib::verify::Score) -> serde_json::Value {
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
fn grib_verify_times(paths: Vec<String>) -> Vec<serde_json::Value> {
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
async fn grib_verify_scores(paths: Vec<String>, t: i64, bbox: [f64; 4]) -> Result<Vec<serde_json::Value>, String> {
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
async fn grib_verify_track(paths: Vec<String>, bbox: [f64; 4]) -> Vec<serde_json::Value> {
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

/// The error field of the forecast in `forecast` against the reference at verify time `t`; `kind` is vector, speed or direction.
#[tauri::command]
fn grib_verify_grid(paths: Vec<String>, t: i64, forecast: String, kind: String, bbox: [f64; 4], max_points: usize) -> Result<serde_json::Value, String> {
    let files = loaded(&paths);
    let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
    let k = match kind.as_str() {
        "speed" => grib::verify::ErrorKind::Speed,
        "direction" => grib::verify::ErrorKind::Direction,
        _ => grib::verify::ErrorKind::Vector,
    };
    let fi = files.iter().position(|(p, _)| *p == forecast).ok_or("That GRIB is not loaded.")?;
    let vt = grib::verify::verify_times(&refs).into_iter().find(|v| v.t == t).ok_or("No GRIB has this time as hour 0.")?;
    let r = grib::verify::error_grid(&refs, &vt, fi, k, bbox, max_points.clamp(100, 40_000)).ok_or("This GRIB does not forecast that time.")?;
    Ok(serde_json::json!({ "lon0": r.lon0, "lat0": r.lat0, "dlon": r.dlon, "dlat": r.dlat, "ni": r.ni, "nj": r.nj, "values": r.values,
        "used": r.used.iter().map(|&i| file_json(&files, i)).collect::<Vec<_>>() }))
}

/// Every loaded GRIB at one point and time: the quick agreement check.
#[tauri::command]
fn grib_point(lat: f64, lon: f64, t: i64) -> Vec<serde_json::Value> {
    let files: Vec<(String, Arc<grib::GribFile>)> = GRIBS.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    files
        .into_iter()
        .map(|(k, f)| {
            let w = f.wind(lat, lon, t);
            let kn = |x: Option<f64>| x.map(|v| (v * 3600.0 / 1852.0 * 10.0).round() / 10.0);
            serde_json::json!({
                "path": k,
                "name": f.summary().name,
                "tws": w.map(|w| (w.tws * 10.0).round() / 10.0),
                "twd": w.map(|w| w.twd.round()),
                "gust": kn(f.value(grib::Param::Gust, lat, lon, t)),
                "mslp": f.value(grib::Param::Mslp, lat, lon, t).map(|p| (p / 10.0).round() / 10.0),
                "wave": f.value(grib::Param::WaveHeight, lat, lon, t).map(|h| (h * 10.0).round() / 10.0),
                "period": f.value(grib::Param::WavePeriod, lat, lon, t).map(|h| h.round()),
                "wavedir": f.value(grib::Param::WaveDir, lat, lon, t).map(|d| d.round()),
                "temp": f.value(grib::Param::AirTemp, lat, lon, t).map(|k| ((k - 273.15) * 10.0).round() / 10.0),
                "rain": f.value(grib::Param::Rain, lat, lon, t).map(|r| (r * 36000.0).round() / 10.0),
            })
        })
        .collect()
}

/// Signal K URL: `RACENAV_SK_URL` environment variable, else the local server.
fn sk_url() -> String {
    std::env::var("RACENAV_SK_URL").unwrap_or_else(|_| signalk::DEFAULT_URL.to_string())
}

/// Polar folders: `RACENAV_POLAR_DIR`, `C:\dev\racenav-data\polars`, qtVlm's polar folder.
fn polar_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("RACENAV_POLAR_DIR") {
        v.push(d.into());
    }
    v.push(r"C:\dev\racenav-data\polars".into());
    v.push(r"C:\Program Files (x86)\qtVlm\polar".into());
    v.retain(|d| d.is_dir());
    v
}

/// Boat speed from a polar file (cached) at a TWA and TWS, kn.
#[tauri::command]
fn polar_speed(path: String, twa: f64, tws: f64) -> Option<f64> {
    static CACHE: Mutex<Option<(String, Arc<polar::Polar>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    if c.as_ref().map(|x| x.0 != path).unwrap_or(true) {
        *c = Some((path.clone(), Arc::new(polar::Polar::from_path(&path).ok()?)));
    }
    let p = c.as_ref()?.1.clone();
    let b = p.bsp(twa.abs(), tws);
    b.is_finite().then_some((b * 100.0).round() / 100.0)
}

#[tauri::command]
fn polar_files() -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for dir in polar_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "csv" | "pol" | "txt") {
                continue;
            }
            // only files that actually parse as polars
            if polar::Polar::from_path(&p).is_err() {
                continue;
            }
            out.push(serde_json::json!({
                "path": p.to_string_lossy(),
                "name": p.file_stem().map(|f| f.to_string_lossy().into_owned()),
                "dir": dir,
            }));
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

#[derive(serde::Deserialize)]
struct RouteRequest {
    /// [lon, lat]
    start: [f64; 2],
    marks: Vec<[f64; 2]>,
    /// Unix seconds UTC
    t0: i64,
    grib_path: String,
    polar_path: String,
    safety_depth: f64,
    /// Rocks, wrecks, obstructions and marine farms to keep off.
    #[serde(default)]
    avoid: routing::Avoid,
    #[serde(default)]
    params: routing::Params,
    /// Per mark (same order as `marks`): rounding "port" | "stbd" | "close" | "" and radius NM (0 = default).
    #[serde(default)]
    mark_opts: Vec<MarkOpt>,
    /// Keep off the library's active barriers and barrier marks.
    #[serde(default = "yes")]
    use_barriers: bool,
}

fn yes() -> bool {
    true
}

#[derive(serde::Deserialize, Default, Clone)]
#[serde(default)]
struct MarkOpt {
    rounding: String,
    radius_nm: f64,
}

/// Route from start through the marks with one loaded GRIB, a polar and the chart.
#[tauri::command]
async fn route_compute(req: RouteRequest) -> Result<serde_json::Value, String> {
    let grib = GRIBS
        .lock()
        .unwrap()
        .get(&req.grib_path)
        .cloned()
        .ok_or("Load the GRIB first (Weather panel).")?;
    tauri::async_runtime::spawn_blocking(move || {
        let polar = polar::Polar::from_path(&req.polar_path).map_err(|e| format!("Polar: {e}"))?;
        let start = nav_core::LatLon::new(req.start[1], req.start[0]);
        let marks: Vec<nav_core::LatLon> = req.marks.iter().map(|m| nav_core::LatLon::new(m[1], m[0])).collect();
        if marks.is_empty() {
            return Err("Add at least one mark.".to_string());
        }
        let specs: Vec<routing::MarkSpec> = marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let o = req.mark_opts.get(i).cloned().unwrap_or_default();
                routing::MarkSpec { pos: *m, rounding: routing::Rounding::parse(&o.rounding), radius_nm: (o.radius_nm > 0.0).then_some(o.radius_nm) }
            })
            .collect();
        // barriers: active lines and zones, and barrier marks that aren't on this course
        let mut barriers = routing::Barriers::default();
        let mut barrier_count = 0;
        if req.use_barriers {
            if let Ok(l) = lib() {
                for b in l.barriers().unwrap_or_default().into_iter().filter(|b| b.active && b.points.len() >= 2) {
                    barrier_count += 1;
                    if b.kind == "zone" && b.points.len() >= 3 { barriers.zones.push(b.points) } else { barriers.lines.push(b.points) }
                }
                let on_course = |m: &library::Mark| {
                    let p = nav_core::LatLon::new(m.lat, m.lon);
                    marks.iter().chain(std::iter::once(&start)).any(|c| c.distance_to(&p) < 0.01)
                };
                for m in l.marks().unwrap_or_default().into_iter().filter(|m| m.barrier_nm > 0.0 && !on_course(m)) {
                    barrier_count += 1;
                    barriers.discs.push(([m.lon, m.lat], m.barrier_nm));
                }
            }
        }
        // waves for the wave-height limit: this GRIB, else a loaded wave GRIB covering the start
        let wave_grib: Option<Arc<grib::GribFile>> = if req.params.max_hs > 0.0 && grib.times(grib::Param::WaveHeight).is_empty() {
            let g = GRIBS.lock().unwrap();
            g.values()
                .filter(|f| f.times(grib::Param::WaveHeight).first().is_some_and(|&a| a <= req.t0) && f.times(grib::Param::WaveHeight).last().is_some_and(|&b| b >= req.t0))
                .max_by_key(|f| f.model().1)
                .cloned()
        } else {
            None
        };
        let clock = std::time::Instant::now();
        let raster = charts().map(|store| {
            let mut b = [start.lon, start.lat, start.lon, start.lat];
            for m in &marks {
                b = [b[0].min(m.lon), b[1].min(m.lat), b[2].max(m.lon), b[3].max(m.lat)];
            }
            // room to go round headlands: a quarter of the span, at least 0.1 degree
            let pad = ((b[2] - b[0]).max(b[3] - b[1]) * 0.25).clamp(0.1, 2.0);
            routing::Raster::from_chart_with(&store, [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad], req.safety_depth, 50.0, 16_000_000, req.avoid)
        });
        let raster_ms = clock.elapsed().as_millis();
        let r = match &wave_grib {
            Some(w) => {
                let src = routing::WithWaves { wind: grib.as_ref(), waves: w.as_ref() };
                routing::route_marks(start, req.t0, &specs, &polar, &src, raster.as_ref(), &barriers, &req.params)
            }
            None => routing::route_marks(start, req.t0, &specs, &polar, grib.as_ref(), raster.as_ref(), &barriers, &req.params),
        };
        let mut v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
        v["barriers"] = barrier_count.into();
        v["waves_from"] = match (&wave_grib, req.params.max_hs > 0.0) {
            (Some(w), _) => w.model().0.into(),
            (None, true) if !grib.times(grib::Param::WaveHeight).is_empty() => "the same GRIB".into(),
            (None, true) => serde_json::Value::String("none: no wave GRIB loaded, the wave limit was not applied".into()),
            _ => serde_json::Value::Null,
        };
        v["obstacles"] = serde_json::json!({
            "cell_m": raster.as_ref().map(|r| r.cell_size_m().round()),
            "build_ms": raster_ms,
            "charts": raster.as_ref().is_some_and(|r| r.any_blocked()),
        });
        Ok(v)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- mark and route library ----------

/// Library folder: `RACENAV_LIBRARY_DIR`, else `C:\dev\racenav-data\library` when
/// racenav-data exists, else `<app data>\library`.
fn library_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_LIBRARY_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data");
    if dev.is_dir() {
        return dev.join("library");
    }
    app.path().app_data_dir().map(|p| p.join("library")).unwrap_or_else(|_| PathBuf::from("library"))
}

fn lib() -> Result<&'static library::Library, String> {
    LIBRARY.get().ok_or_else(|| "The mark library could not be opened.".to_string())
}

fn es(e: library::Error) -> String {
    e.to_string()
}

#[tauri::command]
fn lib_info() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "dir": lib()?.dir() }))
}
#[tauri::command]
fn barriers_list() -> Result<Vec<library::Barrier>, String> {
    lib()?.barriers().map_err(es)
}

#[tauri::command]
fn barrier_save(barrier: library::Barrier) -> Result<library::Barrier, String> {
    lib()?.save_barrier(barrier).map_err(es)
}

#[tauri::command]
fn barrier_delete(id: String) -> Result<(), String> {
    lib()?.delete_barrier(&id).map_err(es)
}

#[tauri::command]
fn marks_list() -> Result<Vec<library::Mark>, String> {
    lib()?.marks().map_err(es)
}
#[tauri::command]
fn mark_save(mark: library::Mark) -> Result<library::Mark, String> {
    lib()?.save_mark(mark).map_err(es)
}
#[tauri::command]
fn mark_delete(id: String, force: bool) -> Result<(), String> {
    lib()?.delete_mark(&id, force).map_err(es)
}
#[tauri::command]
fn routes_list() -> Result<Vec<library::Route>, String> {
    lib()?.routes().map_err(es)
}
#[tauri::command]
fn route_save(route: library::Route) -> Result<library::Route, String> {
    lib()?.save_route(route).map_err(es)
}
#[tauri::command]
fn route_delete(id: String) -> Result<(), String> {
    lib()?.delete_route(&id).map_err(es)
}
#[tauri::command]
fn routings_list() -> Result<Vec<library::RoutingSummary>, String> {
    lib()?.routings().map_err(es)
}
#[tauri::command]
fn routing_get(id: String) -> Result<library::Routing, String> {
    lib()?.routing(&id).map_err(es)
}
#[tauri::command]
fn routing_save(routing: library::Routing) -> Result<library::RoutingSummary, String> {
    lib()?.save_routing(routing).map_err(es)
}
#[tauri::command]
fn routing_delete(id: String) -> Result<(), String> {
    lib()?.delete_routing(&id).map_err(es)
}
/// GPX of marks and routes (empty lists = everything) written to the export folder.
#[tauri::command]
fn gpx_export(name: String, mark_ids: Vec<String>, route_ids: Vec<String>) -> Result<String, String> {
    let l = lib()?;
    let text = l.export_gpx(&mark_ids, &route_ids).map_err(es)?;
    Ok(l.write_export(&name, &text).map_err(es)?.to_string_lossy().into_owned())
}
/// Any GPX text the page made (e.g. a routing with times) written to the export folder.
#[tauri::command]
fn gpx_write(name: String, text: String) -> Result<String, String> {
    Ok(lib()?.write_export(&name, &text).map_err(es)?.to_string_lossy().into_owned())
}
#[tauri::command]
fn gpx_import(text: String, group: String) -> Result<library::ImportReport, String> {
    lib()?.import_gpx(&text, &group).map_err(es)
}

// ---------- tracks and the track recorder ----------

/// The track being recorded from Signal K, when recording is on.
static REC: Mutex<Option<library::Recorder>> = Mutex::new(None);
const REC_FILE: &str = "recorder.json";

fn rec_settings_save(on: bool, id: &str) {
    if let Ok(l) = lib() {
        let _ = std::fs::write(l.dir().join(REC_FILE), serde_json::json!({ "on": on, "track": id }).to_string());
    }
}

fn new_log_track() -> library::Track {
    library::Track {
        id: String::new(),
        name: chrono::Local::now().format("Log %d %b %Y %H:%M").to_string(),
        source: "log".into(),
        ..Default::default()
    }
}

/// Start recording: carry on with `resume` if given (and it exists), else a new track.
fn rec_begin(resume: Option<&str>) -> Result<library::TrackSummary, String> {
    let l = lib()?;
    let mut t = match resume.and_then(|id| l.track(id).ok()) {
        Some(t) => t,
        // a new track is written with its first fix, so a session without Signal K leaves no empty track
        None => library::Track { id: l.new_track_id(), ..new_log_track() },
    };
    t.source = "log".into();
    let sum = t.summary();
    rec_settings_save(true, &t.id);
    *REC.lock().unwrap() = Some(library::Recorder::new(t));
    Ok(sum)
}

/// Save the recording track now (if it has new fixes).
fn rec_flush() {
    let snapshot = {
        let mut g = REC.lock().unwrap();
        match g.as_mut() {
            Some(r) if r.is_dirty() => {
                r.mark_saved(chrono_now());
                Some(r.track.clone())
            }
            _ => None,
        }
    };
    if let (Some(t), Ok(l)) = (snapshot, lib()) {
        if let Err(e) = l.store_track(&t) {
            eprintln!("track save: {e}");
        }
    }
}

/// Offer a Signal K fix to the recorder; saves every 30 s.
fn rec_vessel(app: &tauri::AppHandle, v: &signalk::VesselState) {
    let (Some(lat), Some(lon)) = (v.lat, v.lon) else { return };
    let now = chrono_now();
    let t = v.datetime.as_deref().and_then(library::tracks::parse_iso).unwrap_or(now);
    let (kept, due, stats) = {
        let mut g = REC.lock().unwrap();
        let Some(r) = g.as_mut() else { return };
        let kept = r.push(library::TrackPoint { t: Some(t), lat, lon, sog: v.sog, cog: v.cog });
        (kept, r.due_save(now, 30), r.track.stats.clone())
    };
    if kept {
        let _ = app.emit("rec-point", serde_json::json!({ "lon": lon, "lat": lat, "t": t, "stats": stats }));
    }
    if due {
        std::thread::spawn(rec_flush);
    }
}

#[tauri::command]
fn rec_status() -> serde_json::Value {
    let g = REC.lock().unwrap();
    match g.as_ref() {
        Some(r) => serde_json::json!({ "on": true, "id": r.track.id, "name": r.track.name, "stats": r.track.stats }),
        None => serde_json::json!({ "on": false }),
    }
}

/// Start recording; `fresh` starts a new track, else the last one carries on.
#[tauri::command]
fn rec_start(fresh: bool) -> Result<serde_json::Value, String> {
    rec_flush();
    let last = if fresh {
        None
    } else {
        std::fs::read_to_string(lib()?.dir().join(REC_FILE))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["track"].as_str().map(str::to_string))
    };
    rec_begin(last.as_deref())?;
    Ok(rec_status())
}

#[tauri::command]
fn rec_stop() -> serde_json::Value {
    rec_flush();
    let id = REC.lock().unwrap().take().map(|r| r.track.id).unwrap_or_default();
    rec_settings_save(false, &id);
    rec_status()
}

#[tauri::command]
fn tracks_list() -> Result<Vec<library::TrackSummary>, String> {
    rec_flush();
    lib()?.tracks().map_err(es)
}

/// A track's points for drawing: segments of [lon, lat, t, sog, cog].
#[tauri::command]
fn track_get(id: String) -> Result<serde_json::Value, String> {
    let t = {
        let g = REC.lock().unwrap();
        match g.as_ref().filter(|r| r.track.id == id) {
            Some(r) => r.track.clone(),
            None => {
                drop(g);
                lib()?.track(&id).map_err(es)?
            }
        }
    };
    let segs: Vec<Vec<serde_json::Value>> = t
        .segments
        .iter()
        .map(|s| s.iter().map(|p| serde_json::json!([p.lon, p.lat, p.t, p.sog, p.cog])).collect())
        .collect();
    Ok(serde_json::json!({ "id": t.id, "name": t.name, "colour": t.colour, "stats": t.stats, "segments": segs }))
}

#[tauri::command]
fn track_update(id: String, name: String, notes: String, colour: String) -> Result<library::TrackSummary, String> {
    rec_flush();
    let s = lib()?.update_track_info(&id, &name, &notes, &colour).map_err(es)?;
    if let Some(r) = REC.lock().unwrap().as_mut().filter(|r| r.track.id == id) {
        r.track.name = s.name.clone();
        r.track.notes = s.notes.clone();
        r.track.colour = s.colour.clone();
    }
    Ok(s)
}

#[tauri::command]
fn track_delete(id: String) -> Result<(), String> {
    if REC.lock().unwrap().as_ref().is_some_and(|r| r.track.id == id) {
        return Err("This track is being recorded: stop recording first.".into());
    }
    lib()?.delete_track(&id).map_err(es)
}

/// Write a track as GPX into the library's export folder; returns the path.
#[tauri::command]
fn track_export(id: String) -> Result<String, String> {
    rec_flush();
    let l = lib()?;
    let t = l.track(&id).map_err(es)?;
    Ok(l.write_export(&t.name, &t.to_gpx()).map_err(es)?.to_string_lossy().into_owned())
}

// ---------- tides (LINZ) ----------

static TIDE_DIR: OnceLock<PathBuf> = OnceLock::new();
static TIDES: Mutex<Option<Arc<tides::TideDb>>> = Mutex::new(None);
static TIDE_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tide folder: `RACENAV_TIDE_DIR`, else `C:\dev\racenav-data\tides` when
/// racenav-data exists, else `<app data>\tides`.
fn tide_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_TIDE_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data");
    if dev.is_dir() {
        return dev.join("tides");
    }
    app.path().app_data_dir().map(|p| p.join("tides")).unwrap_or_else(|_| PathBuf::from("tides"))
}

fn tide_db() -> Arc<tides::TideDb> {
    let mut g = TIDES.lock().unwrap();
    if g.is_none() {
        let dir = TIDE_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("tides"));
        *g = Some(Arc::new(tides::TideDb::open(&dir)));
    }
    g.as_ref().unwrap().clone()
}

/// What tide data is on this PC: stations, years with daily tables, folder.
#[tauri::command]
fn tides_info() -> serde_json::Value {
    let db = tide_db();
    serde_json::json!({
        "stations": db.stations.len(),
        "daily": db.stations.iter().filter(|s| s.kind == tides::Kind::Daily).count(),
        "years": db.years,
        "dir": TIDE_DIR.get(),
    })
}

/// Every tide station as GeoJSON points (`kind`: daily, secondary, levels).
#[tauri::command]
fn tides_stations() -> serde_json::Value {
    let db = tide_db();
    let features: Vec<serde_json::Value> = db
        .stations
        .iter()
        .map(|s| {
            let (kind, reference) = match &s.kind {
                tides::Kind::Daily => ("daily", None),
                tides::Kind::Secondary { reference, .. } => ("secondary", Some(reference.clone())),
                tides::Kind::LevelsOnly { reference } => ("levels", Some(reference.clone())),
            };
            serde_json::json!({ "type": "Feature",
                "properties": { "id": s.id, "name": s.name, "kind": kind, "ref": reference, "approx": s.approx },
                "geometry": { "type": "Point", "coordinates": [s.lon, s.lat] } })
        })
        .collect();
    serde_json::json!({ "type": "FeatureCollection", "features": features })
}

/// Tides at a station from `t0` to `t1` (Unix seconds): highs and lows, the curve
/// every 10 minutes, mean levels, and how the prediction is made.
#[tauri::command]
fn tides_curve(id: String, t0: i64, t1: i64) -> Result<serde_json::Value, String> {
    let db = tide_db();
    let s = db.station(&id).ok_or("No such tide station.")?;
    let ext = db.extremes(&id, t0, t1).unwrap_or_default();
    let curve = db.curve(&id, t0, t1, 600);
    let lv = s.levels;
    let method = match &s.kind {
        tides::Kind::Daily => "LINZ daily table".to_string(),
        tides::Kind::Secondary { reference, hw_min, lw_min, ratio } => format!(
            "Secondary to {reference}: HW {:+} min, LW {:+} min{}",
            hw_min,
            lw_min,
            ratio.map_or(", times only".to_string(), |r| format!(", range × {r:.2}"))
        ),
        tides::Kind::LevelsOnly { reference } => format!("Reference port {reference}: LINZ gives levels only, no times"),
    };
    Ok(serde_json::json!({
        "id": s.id, "name": s.name, "approx": s.approx, "method": method,
        "chatham": s.lon < -175.0 && s.lat < -43.0 && s.lat > -45.0,
        "extremes": ext.iter().filter(|e| e.t >= t0 - 3 * 3600 && e.t <= t1 + 3 * 3600).map(|e| serde_json::json!({ "t": e.t, "h": e.h, "high": e.high })).collect::<Vec<_>>(),
        "curve": curve,
        "levels": { "mhws": lv.mhws, "mhwn": lv.mhwn, "mlwn": lv.mlwn, "mlws": lv.mlws, "msl": lv.msl },
        "years": db.years,
    }))
}

/// Download the LINZ secondary ports file and daily tables for `years` (emits "tides-progress").
#[tauri::command]
async fn tides_download(app: tauri::AppHandle, years: Vec<i32>) -> Result<serde_json::Value, String> {
    TIDE_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    let dir = TIDE_DIR.get().cloned().ok_or("no tide folder")?;
    let n = tauri::async_runtime::spawn_blocking(move || {
        tides::download(&dir, &years, &TIDE_CANCEL, |p| {
            let _ = app.emit("tides-progress", serde_json::json!({ "stage": p.stage, "done": p.done, "total": p.total }));
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    *TIDES.lock().unwrap() = None; // re-read on next use
    Ok(serde_json::json!({ "written": n, "info": tides_info() }))
}

#[tauri::command]
fn tides_cancel() {
    TIDE_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

// ---------- race tracking (YB) ----------

static TRACKING_DIR: OnceLock<PathBuf> = OnceLock::new();
static RACES: Mutex<BTreeMap<String, tracking::Race>> = Mutex::new(BTreeMap::new());

/// Tracking folder: `RACENAV_TRACKING_DIR`, else `C:\dev\racenav-data\tracking`
/// when racenav-data exists, else `<app data>\tracking`.
fn tracking_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_TRACKING_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data");
    if dev.is_dir() {
        return dev.join("tracking");
    }
    app.path().app_data_dir().map(|p| p.join("tracking")).unwrap_or_else(|_| PathBuf::from("tracking"))
}

fn with_race<T>(key: &str, f: impl FnOnce(&mut tracking::Race) -> Result<T, String>) -> Result<T, String> {
    let key = tracking::clean_key(key).map_err(|e| e.to_string())?;
    let mut races = RACES.lock().unwrap();
    if !races.contains_key(&key) {
        let dir = TRACKING_DIR.get().ok_or("no tracking folder")?;
        races.insert(key.clone(), tracking::Race::open(dir, &key).map_err(|e| e.to_string())?);
    }
    f(races.get_mut(&key).unwrap())
}

/// Open a race: the cached setup and every fix kept so far; the setup is
/// (re)fetched when missing or more than 6 hours old. Network errors are
/// reported in `error` alongside whatever is cached.
#[tauri::command]
async fn yb_open(race: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = tracking::clean_key(&race).map_err(|e| e.to_string())?;
        let (cached, fetched) = with_race(&key, |r| Ok((r.setup(), r.meta.setup_fetched)))?;
        let now = tracking::now();
        let mut error = None;
        let mut setup = cached;
        if setup.is_none() || fetched.is_none_or(|t| now - t > 6 * 3600) {
            match tracking::fetch_setup(&key) {
                Ok(text) => match with_race(&key, |r| r.save_setup_text(&text, now).map_err(|e| e.to_string())) {
                    Ok(s) => setup = Some(s),
                    Err(e) => error = Some(format!("Race setup not understood: {e}")),
                },
                Err(e) => error = Some(if setup.is_some() { format!("Using the saved race setup ({e})") } else { e.to_string() }),
            }
        }
        with_race(&key, |r| {
            Ok(serde_json::json!({ "key": key, "setup": setup, "fixes": r.fixes, "meta": r.meta, "error": error }))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download the latest positions and keep them; returns the fixes newer than `since`.
#[tauri::command]
async fn yb_refresh(race: String, since: i64) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = tracking::clean_key(&race).map_err(|e| e.to_string())?;
        let now = tracking::now();
        match tracking::fetch_positions(&key) {
            Ok(fixes) => with_race(&key, |r| {
                let added = r.merge(&fixes, now).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "key": key, "added": added, "boats": fixes.len(), "fixes": r.fixes_since(since), "meta": r.meta, "source": "expedition" }))
            }),
            // no Expedition file for this race: the history file has the latest fixes too
            Err(e1) => {
                let h = tracking::fetch_history(&key).map_err(|e2| format!("{e1}; history: {e2}"))?;
                with_race(&key, |r| {
                    let added = r.merge_history(&h, now).map_err(|e| e.to_string())?;
                    Ok(serde_json::json!({ "key": key, "added": added, "boats": h.len(), "fixes": r.fixes_since(since), "meta": r.meta, "source": "history" }))
                })
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download the race's full history (every published fix of every boat) and keep it.
#[tauri::command]
async fn yb_history(race: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = tracking::clean_key(&race).map_err(|e| e.to_string())?;
        let h = tracking::fetch_history(&key).map_err(|e| e.to_string())?;
        let n: usize = h.values().map(Vec::len).sum();
        let now = tracking::now();
        with_race(&key, |r| {
            let added = r.merge_history(&h, now).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "key": key, "added": added, "received": n, "boats": h.len(), "fixes": r.fixes, "meta": r.meta }))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Races with something saved, newest first.
#[tauri::command]
fn yb_races() -> Vec<serde_json::Value> {
    let Some(dir) = TRACKING_DIR.get() else { return Vec::new() };
    let mut v: Vec<(u64, serde_json::Value)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let key = e.file_name().to_string_lossy().into_owned();
            let m = std::fs::metadata(e.path().join("fixes.json")).or_else(|_| std::fs::metadata(e.path()));
            let t = m.ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            let title = std::fs::read_to_string(e.path().join("setup.json")).ok().and_then(|s| tracking::parse_setup(&s).ok()).map(|s| s.title).unwrap_or_default();
            (t, serde_json::json!({ "key": key, "title": title, "modified": t }))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|x| x.1).collect()
}

/// Chart folder: `RACENAV_CHART_DIR`, else `C:\dev\racenav-data\linz` if present,
/// else `<app data>\charts`.
fn chart_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_CHART_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data\linz");
    if dev.is_dir() {
        return dev;
    }
    app.path().app_data_dir().map(|p| p.join("charts")).unwrap_or_else(|_| PathBuf::from("charts"))
}

/// World coastline folder (GSHHG zip or GSHHS_shp): `RACENAV_COAST_DIR`, else
/// `C:\dev\racenav-data\coastline` when racenav-data exists, else `<app data>\coastline`.
fn coast_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_COAST_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data");
    if dev.is_dir() {
        return dev.join("coastline");
    }
    app.path().app_data_dir().map(|p| p.join("coastline")).unwrap_or_else(|_| PathBuf::from("coastline"))
}

/// Downloaded ETOPO areas: `RACENAV_BATHY_DIR`, else `C:\dev\racenav-data\bathy` when
/// racenav-data exists, else `<app data>\bathy`.
fn bathy_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_BATHY_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data");
    if dev.is_dir() {
        return dev.join("bathy");
    }
    app.path().app_data_dir().map(|p| p.join("bathy")).unwrap_or_else(|_| PathBuf::from("bathy"))
}

/// The folder holding the bundled world depths (`world_depth.gpkg`).
fn world_depth_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let candidates = [
        app.path().resource_dir().ok().map(|d| d.join("resources")),
        app.path().resource_dir().ok(),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")),
    ];
    candidates.into_iter().flatten().find(|d| d.join("world_depth.gpkg").is_file())
}

/// (Re)open the chart store in the background: LINZ charts, GSHHG coastline,
/// bundled world depths and downloaded ETOPO areas. Emits chart-ready.
fn open_charts(handle: tauri::AppHandle) {
    let dir = chart_dir(&handle);
    let mut dirs = vec![dir.clone(), coast_dir(&handle), bathy_dir(&handle)];
    if let Some(w) = world_depth_dir(&handle) {
        dirs.push(w);
    }
    // the first run with a GSHHG download builds the world coastline file (a minute or two)
    let _ = handle.emit("chart-status", serde_json::json!({ "detail": "opening charts" }));
    std::thread::spawn(move || match chart::ChartStore::open_dirs(&dirs) {
        Ok(store) => {
            let n = store.tables.len();
            let world = store.has_world_land();
            *CHARTS.write().unwrap() = Some(Arc::new(store));
            let _ = handle.emit("chart-ready", serde_json::json!({ "tables": n, "dir": dir, "world_land": world }));
        }
        Err(e) => {
            let _ = handle.emit("chart-ready", serde_json::json!({ "tables": 0, "dir": dir, "error": e.to_string() }));
        }
    });
}

/// What chart data is installed, for Chart › Chart data.
#[tauri::command]
fn data_status(app: tauri::AppHandle) -> serde_json::Value {
    let coast = coast_dir(&app);
    let gshhg = coast.join(gribdl::data::GSHHG_FILE).is_file() || coast.join("GSHHS_shp").is_dir();
    let bdir = bathy_dir(&app);
    let mut etopo = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&bdir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "gpkg") {
                if let Some((area, source, spacing)) = chart::bathy::area_info(&p) {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    etopo.push(serde_json::json!({ "file": p, "name": p.file_name().map(|n| n.to_string_lossy().into_owned()), "area": area, "source": source, "spacing": spacing, "bytes": size }));
                }
            }
        }
    }
    let store = charts();
    let linz = store.as_ref().map(|s| s.tables.iter().filter(|t| matches!(t.kind, chart::Kind::DepthArea | chart::Kind::Land | chart::Kind::Contour)).count()).unwrap_or(0);
    serde_json::json!({
        "world_depth": world_depth_dir(&app).is_some(),
        "gshhg": gshhg,
        "world_land": store.as_ref().is_some_and(|s| s.has_world_land()),
        "gshhg_bytes": gribdl::data::GSHHG_BYTES,
        "etopo": etopo,
        "linz_tables": linz,
        "dirs": { "charts": chart_dir(&app), "coast": coast, "bathy": bdir },
        "busy": DATA_BUSY.load(std::sync::atomic::Ordering::SeqCst),
    })
}

/// Points and stride an ETOPO download of `area` at `arcsec` would fetch.
#[tauri::command]
fn data_etopo_plan(area: [f64; 4], arcsec: u32) -> serde_json::Value {
    let (points, stride) = gribdl::data::etopo_plan(area, arcsec);
    serde_json::json!({ "points": points, "spacing": 15 * stride, "bytes": points * 4 })
}

fn data_guard() -> Result<(), String> {
    if DATA_BUSY.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("A chart data download is already running.".into());
    }
    DATA_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// Download the GSHHG world coastline (about 150 MB) and reopen the charts (which builds it).
#[tauri::command]
async fn data_gshhg(app: tauri::AppHandle) -> Result<String, String> {
    data_guard()?;
    let dir = coast_dir(&app);
    let h = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let progress = |done: u64, total: u64| {
            let _ = h.emit("data-progress", serde_json::json!({ "stage": "World coastline (GSHHG)", "done": done, "total": total }));
        };
        gribdl::data::gshhg(&dir, &progress, &DATA_CANCEL).map(|p| p.to_string_lossy().into_owned()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());
    DATA_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    let path = r??;
    open_charts(app);
    Ok(path)
}

/// Download ETOPO 2022 depths for `area` at `arcsec` (15, 30, 60), write depth
/// bands and contours to the bathy folder, and reopen the charts.
#[tauri::command]
async fn data_etopo(app: tauri::AppHandle, area: [f64; 4], arcsec: u32) -> Result<String, String> {
    data_guard()?;
    let dir = bathy_dir(&app);
    let h = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let _ = h.emit("data-progress", serde_json::json!({ "stage": "Depths (ETOPO 2022): downloading", "done": 0, "total": 0 }));
        let progress = |done: u64, _total: u64| {
            let _ = h.emit("data-progress", serde_json::json!({ "stage": "Depths (ETOPO 2022): downloading", "done": done, "total": 0 }));
        };
        let g = gribdl::data::etopo(area, arcsec, &progress, &DATA_CANCEL).map_err(|e| e.to_string())?;
        let _ = h.emit("data-progress", serde_json::json!({ "stage": "Depths (ETOPO 2022): drawing depth bands and contours", "done": 0, "total": 0 }));
        let spacing = (g.dlat * 3600.0).round() as u32;
        let grid = chart::bathy::Grid { west: g.west, south: g.south, dlon: g.dlon, dlat: g.dlat, nx: g.nx, ny: g.ny, z: g.z };
        let f = |v: f64, pos: char, neg: char| format!("{:.1}{}", v.abs(), if v < 0.0 { neg } else { pos });
        let name = format!("etopo_{}_{}_{}_{}_{spacing}s.gpkg", f(area[1], 'N', 'S'), f(area[3], 'N', 'S'), f(area[0], 'E', 'W'), f(area[2], 'E', 'W'));
        chart::bathy::write_grid(&grid, &dir.join(name), &format!("ETOPO 2022 {spacing}\u{2033} (NOAA NCEI)"))
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());
    DATA_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    let path = r??;
    open_charts(app);
    Ok(path)
}

/// Delete a downloaded ETOPO area (only files in the bathy folder) and reopen the charts.
#[tauri::command]
fn data_etopo_delete(app: tauri::AppHandle, file: String) -> Result<(), String> {
    let p = PathBuf::from(&file);
    let dir = bathy_dir(&app);
    if p.parent().map(|d| d.canonicalize().ok()) != Some(dir.canonicalize().ok()) || p.extension().is_none_or(|x| x != "gpkg") {
        return Err("not a downloaded depth area".into());
    }
    // the store has the file open: reopen without it first, then delete
    *CHARTS.write().unwrap() = None;
    std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    open_charts(app);
    Ok(())
}

#[tauri::command]
fn data_cancel() {
    DATA_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> serde_json::Value {
    let log_dir = app.path().app_data_dir().ok().map(|p| p.join("logs"));
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "signalk_url": sk_url(),
        "log_dir": log_dir,
        "chart_dir": chart_dir(&app),
    })
}

/// All attributes of a chart feature (table index and fid from the tiles), in words.
#[tauri::command]
fn chart_feature(t: usize, fid: i64) -> Option<serde_json::Value> {
    let f = charts()?.feature(t, fid)?;
    Some(serde_json::json!({
        "code": f.code, "name": f.class_name, "band": f.band, "position": f.position,
        "attrs": f.attrs,
    }))
}

/// Summary of the chart folder.
#[tauri::command]
fn chart_info() -> serde_json::Value {
    match charts() {
        Some(s) => serde_json::json!({
            "tables": s.tables.len(),
            "world_land": s.has_world_land(),
            "layers": s.tables.iter().map(|t| serde_json::json!({
                "table": t.table, "kind": format!("{:?}", t.kind), "band": t.band, "rows": t.rows
            })).collect::<Vec<_>>(),
        }),
        None => serde_json::json!({ "tables": 0 }),
    }
}

fn tile_response(path: &str) -> Response<Vec<u8>> {
    let parts: Vec<&str> = path.trim_start_matches('/').trim_end_matches(".pbf").split('/').collect();
    let parsed = match parts.as_slice() {
        [z, x, y] => z.parse::<u8>().ok().zip(x.parse::<u32>().ok()).zip(y.parse::<u32>().ok()),
        _ => None,
    };
    let builder = Response::builder().header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    match (parsed, charts()) {
        (Some(((z, x), y)), Some(store)) => builder
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .body(store.tile(z, x, y).as_ref().clone()),
        (Some(_), None) => builder.status(StatusCode::SERVICE_UNAVAILABLE).body(Vec::new()),
        _ => builder.status(StatusCode::NOT_FOUND).body(Vec::new()),
    }
    .expect("valid response")
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_info, chart_info, chart_feature, tides_info, tides_stations, tides_curve, tides_download, tides_cancel, grib_files, grib_load, grib_unload, grib_loaded, grib_wind_grid, grib_grid, grib_compare, grib_meteogram, grib_verify_times, grib_verify_scores, grib_verify_track, grib_verify_grid, grib_point,
            polar_files, polar_speed, route_compute, sk_status,
            lib_info, marks_list, mark_save, mark_delete, barriers_list, barrier_save, barrier_delete, routes_list, route_save, route_delete,
            routings_list, routing_get, routing_save, routing_delete, gpx_export, gpx_write, gpx_import,
            rec_status, rec_start, rec_stop, tracks_list, track_get, track_update, track_delete, track_export,
            yb_open, yb_refresh, yb_races, yb_history,
            gribdl_models, gribdl_estimate, gribdl_start, gribdl_cancel, open_url, grib_combine,
            data_status, data_etopo_plan, data_gshhg, data_etopo, data_etopo_delete, data_cancel
        ])
        .register_asynchronous_uri_scheme_protocol("chart", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(tile_response(&path)));
        })
        .setup(|app| {
            let dd = download_dir(app.handle());
            let _ = std::fs::create_dir_all(&dd);
            let _ = DOWNLOAD_DIR.set(dd);
            let td = tracking_dir(app.handle());
            let _ = std::fs::create_dir_all(&td);
            let _ = TRACKING_DIR.set(td);
            let tdir = tide_dir(app.handle());
            let _ = std::fs::create_dir_all(&tdir);
            let _ = TIDE_DIR.set(tdir);
            match library::Library::open(library_dir(app.handle())) {
                Ok(l) => {
                    let _ = LIBRARY.set(l);
                    // carry on recording if it was on: the same track if its last fix is under 2 h old
                    let st = lib().ok().and_then(|l| std::fs::read_to_string(l.dir().join(REC_FILE)).ok());
                    if let Some(v) = st.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
                        if v["on"].as_bool() == Some(true) {
                            let id = v["track"].as_str().unwrap_or("").to_string();
                            let recent = lib().ok().and_then(|l| l.track(&id).ok()).and_then(|t| t.stats.end).is_some_and(|e| chrono_now() - e < 2 * 3600);
                            if let Err(e) = rec_begin(recent.then_some(id.as_str())) {
                                eprintln!("recorder: {e}");
                            }
                        }
                    }
                }
                Err(e) => eprintln!("library: {e}"),
            }
            // Charts: open in the background (the first run unzips the LINZ downloads).
            let _ = std::fs::create_dir_all(coast_dir(app.handle()));
            let _ = std::fs::create_dir_all(bathy_dir(app.handle()));
            open_charts(app.handle().clone());

            // Signal K
            let handle = app.handle().clone();
            let log_dir = app.path().app_data_dir().ok().map(|p| p.join("logs"));
            let opts = signalk::Options { url: sk_url(), log_dir, min_interval: Duration::from_millis(250) };
            let (tx, mut rx) = tokio::sync::mpsc::channel(256);
            tauri::async_runtime::spawn(signalk::run(opts, tx));
            tauri::async_runtime::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    let _ = match ev {
                        signalk::Event::Status { connected, detail } => {
                            *SK_STATUS.lock().unwrap() = Some((connected, detail.clone()));
                            handle.emit("sk-status", serde_json::json!({ "connected": connected, "detail": detail }))
                        }
                        signalk::Event::Vessel(v) => {
                            rec_vessel(&handle, &v);
                            handle.emit("vessel", v)
                        }
                    };
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building racenav")
        .run(|_app, event| {
            // save the track being recorded on the way out
            if let tauri::RunEvent::Exit = event {
                rec_flush();
            }
        });
}
