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
    v.push(r"C:\dev\racenav-data\grib".into());
    v.push(r"C:\Program Files (x86)\qtVlm\grib".into());
    v.retain(|d| d.is_dir());
    v
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
async fn grib_load(path: String) -> Result<grib::Summary, String> {
    let p = path.clone();
    let f = tauri::async_runtime::spawn_blocking(move || grib::GribFile::open(&p))
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
        "lon0": g.lon0 + i_lo as f64 * g.dlon,
        "lat0": g.lat0 + j_lo as f64 * g.dlat,
        "dlon": g.dlon * stride as f64,
        "dlat": g.dlat * stride as f64,
        "ni": ni, "nj": nj, "tws": tws, "twd": twd,
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
            routing::Raster::from_chart(store, [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad], req.safety_depth, 50.0, 16_000_000)
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

#[tauri::command]
fn chart_info() -> serde_json::Value {
    match CHARTS.get() {
        Some(s) => serde_json::json!({
            "tables": s.tables.len(),
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
            app_info, chart_info, grib_files, grib_load, grib_unload, grib_loaded, grib_wind_grid, grib_point,
            polar_files, route_compute, sk_status
        ])
        .register_asynchronous_uri_scheme_protocol("chart", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(tile_response(&path)));
        })
        .setup(|app| {
            // Charts: open in the background (the first run unzips the LINZ downloads).
            let handle = app.handle().clone();
            let dir = chart_dir(&handle);
            std::thread::spawn(move || match chart::ChartStore::open_dir(&dir) {
                Ok(store) => {
                    let n = store.tables.len();
                    let _ = CHARTS.set(Arc::new(store));
                    let _ = handle.emit("chart-ready", serde_json::json!({ "tables": n, "dir": dir }));
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
