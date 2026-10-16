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

static CHARTS: OnceLock<Arc<chart::ChartStore>> = OnceLock::new();
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
    serde_json::json!({ "bytes": gribdl::estimate_bytes(&req), "steps": steps.len(), "last": steps.last() })
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
        let path = gribdl::download(&req, &run, &dir, &progress, &DL_CANCEL).map_err(|e| e.to_string())?;
        Ok::<_, String>(serde_json::json!({ "path": path, "run": run, "name": label }))
    })
    .await
    .map_err(|e| e.to_string());
    DL_BUSY.store(false, Ordering::SeqCst);
    res?
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
/// "period" (s), "wavedir" (deg from), "rain" (mm/h), "cape" (J/kg), "mslp" (hPa).
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
        let clock = std::time::Instant::now();
        let raster = CHARTS.get().map(|store| {
            let mut b = [start.lon, start.lat, start.lon, start.lat];
            for m in &marks {
                b = [b[0].min(m.lon), b[1].min(m.lat), b[2].max(m.lon), b[3].max(m.lat)];
            }
            // room to go round headlands: a quarter of the span, at least 0.1 degree
            let pad = ((b[2] - b[0]).max(b[3] - b[1]) * 0.25).clamp(0.1, 2.0);
            routing::Raster::from_chart_with(store, [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad], req.safety_depth, 50.0, 16_000_000, req.avoid)
        });
        let raster_ms = clock.elapsed().as_millis();
        let r = routing::route(start, req.t0, &marks, &polar, grib.as_ref(), raster.as_ref(), &req.params);
        let mut v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
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
    let f = CHARTS.get()?.feature(t, fid)?;
    Some(serde_json::json!({
        "code": f.code, "name": f.class_name, "band": f.band, "position": f.position,
        "attrs": f.attrs,
    }))
}

/// Summary of the chart folder.
#[tauri::command]
fn chart_info() -> serde_json::Value {
    match CHARTS.get() {
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
    match (parsed, CHARTS.get()) {
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
            app_info, chart_info, chart_feature, tides_info, tides_stations, tides_curve, tides_download, tides_cancel, grib_files, grib_load, grib_unload, grib_loaded, grib_wind_grid, grib_grid, grib_point,
            polar_files, route_compute, sk_status,
            lib_info, marks_list, mark_save, mark_delete, routes_list, route_save, route_delete,
            routings_list, routing_get, routing_save, routing_delete, gpx_export, gpx_write, gpx_import,
            yb_open, yb_refresh, yb_races, yb_history,
            gribdl_models, gribdl_estimate, gribdl_start, gribdl_cancel
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
                }
                Err(e) => eprintln!("library: {e}"),
            }
            // Charts: open in the background (the first run unzips the LINZ downloads).
            let handle = app.handle().clone();
            let dir = chart_dir(&handle);
            let coast = coast_dir(&handle);
            let _ = std::fs::create_dir_all(&coast);
            // the first run with a GSHHG download builds the world coastline file (a minute or two)
            let _ = handle.emit("chart-status", serde_json::json!({ "detail": "opening charts" }));
            std::thread::spawn(move || match chart::ChartStore::open_dirs(&[dir.clone(), coast]) {
                Ok(store) => {
                    let n = store.tables.len();
                    let world = store.has_world_land();
                    let _ = CHARTS.set(Arc::new(store));
                    let _ = handle.emit("chart-ready", serde_json::json!({ "tables": n, "dir": dir, "world_land": world }));
                }
                Err(e) => {
                    let _ = handle.emit("chart-ready", serde_json::json!({ "tables": 0, "dir": dir, "error": e.to_string() }));
                }
            });

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
                        signalk::Event::Vessel(v) => handle.emit("vessel", v),
                    };
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running racenav");
}
