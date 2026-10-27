//! racenav desktop app: wires the Rust crates to the MapLibre front end.
//!
//! Events sent to the front end:
//! - `sk-status`   `{ connected, detail }`
//! - `vessel`      own-vessel state (degrees true, knots, metres)
//! - `chart-ready` `{ ready, tables, dir }` once the chart folder has been opened
//!
//! The `chart` URI scheme serves vector tiles: `chart://localhost/{z}/{x}/{y}.pbf`
//! (`http://chart.localhost/...` on Windows).

mod live;
mod nmea;
mod radar_sim;
mod s63_cmd;
mod settings;
mod stations_cmd;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use tauri::http::{header, Response, StatusCode};
use tauri::{Emitter, Manager};

static CHARTS: std::sync::RwLock<Option<Arc<chart::ChartStore>>> = std::sync::RwLock::new(None);
static CHARTS_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CHART_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// The open chart store (None until the charts are open).
/// The developer's data folder: `C:\dev\tweedplot-data`, or the older
/// `C:\dev\racenav-data` while that is the one there. Without either, data lives in
/// the app data folder (`%APPDATA%\nz.tweedsmarine.tweedplot`).
pub(crate) fn dev_root() -> PathBuf {
    let new = PathBuf::from(r"C:\dev\tweedplot-data");
    let old = PathBuf::from(r"C:\dev\racenav-data");
    // the old folder until the charts have moved (a new folder with only, say, ENC test
    // data in it mustn't hide them)
    if !new.join("linz").is_dir() && old.is_dir() {
        old
    } else {
        new
    }
}

/// The app was called RaceNav (identifier nz.tweedsmarine.racenav): on the first start
/// as TweedPlot, its app data and web view data (settings, layouts, keys) are copied to
/// the new identifier's folders so nothing is lost.
pub fn migrate_from_racenav() {
    fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            let (src, dst) = (e.path(), to.join(e.file_name()));
            if e.file_type()?.is_dir() {
                copy_dir(&src, &dst)?;
            } else if !dst.exists() {
                // a locked or half-written file (a web view cache) is skipped, not fatal
                let _ = std::fs::copy(&src, &dst);
            }
        }
        Ok(())
    }
    for var in ["APPDATA", "LOCALAPPDATA"] {
        let Ok(base) = std::env::var(var) else {
            continue;
        };
        let (old, new) = (
            PathBuf::from(&base).join("nz.tweedsmarine.racenav"),
            PathBuf::from(&base).join("nz.tweedsmarine.tweedplot"),
        );
        if old.is_dir() && !new.exists() {
            let _ = copy_dir(&old, &new);
        }
    }
}

fn charts() -> Option<Arc<chart::ChartStore>> {
    CHARTS.read().unwrap().clone()
}

/// The downloaded high-resolution depths (LINZ coastal elevation tiles), drawn as images.
static HIRES: std::sync::RwLock<Option<Arc<chart::hires::Hires>>> = std::sync::RwLock::new(None);

fn hires() -> Option<Arc<chart::hires::Hires>> {
    HIRES.read().unwrap().clone()
}

/// (Re)open the high-resolution depth tiles (quick: only their headers are read).
fn open_hires(app: &tauri::AppHandle) {
    let h = chart::hires::Hires::open(&hires_dir(app));
    *HIRES.write().unwrap() = Some(Arc::new(h));
}

/// High-resolution depths (LINZ coastal elevation GeoTIFFs): `RACENAV_HIRES_DIR`, else
/// `C:\dev\racenav-data\hires` when racenav-data exists, else `<app data>\hires`.
fn hires_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_HIRES_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("hires");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("hires"))
        .unwrap_or_else(|_| PathBuf::from("hires"))
}

/// A clear 256-pixel PNG, for map tiles without high-resolution depths.
fn clear_png() -> &'static [u8] {
    static PNG: OnceLock<Vec<u8>> = OnceLock::new();
    PNG.get_or_init(|| {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 256, 256);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            if let Ok(mut w) = enc.write_header() {
                let _ = w.write_image_data(&vec![0u8; 256 * 256 * 4]);
            }
        }
        out
    })
}

static DATA_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DATA_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static GRIBS: Mutex<BTreeMap<String, Arc<grib::GribFile>>> = Mutex::new(BTreeMap::new());
/// The library open now (one at a time; File-like New/Open in the Library drawer switch it).
static LIBRARY: std::sync::RwLock<Option<Arc<library::Library>>> = std::sync::RwLock::new(None);
/// Latest Signal K connection state, for a page that loads after the event was sent.
pub(crate) static SK_STATUS: Mutex<Option<(bool, String)>> = Mutex::new(None);

#[tauri::command]
fn sk_status() -> serde_json::Value {
    match SK_STATUS.lock().unwrap().clone() {
        Some((connected, detail)) => {
            serde_json::json!({ "connected": connected, "detail": detail })
        }
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
    v.push(dev_root().join("grib"));
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
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("grib");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("grib"))
        .unwrap_or_else(|_| PathBuf::from("grib"))
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
async fn gribdl_start(
    app: tauri::AppHandle,
    req: gribdl::Request,
) -> Result<serde_json::Value, String> {
    use std::sync::atomic::Ordering;
    if DL_BUSY.swap(true, Ordering::SeqCst) {
        return Err("A download is already running.".into());
    }
    DL_CANCEL.store(false, Ordering::SeqCst);
    let dir = DOWNLOAD_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("grib"));
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
        grib::combine::combine(std::path::Path::new(&a), std::path::Path::new(&b))
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open a mailto: link (a Saildocs request) in the mail program, or a web link from the manual in the browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // mail (Saildocs requests) and web links from the manual; nothing that could run a program
    let ok =
        url.starts_with("mailto:") || url.starts_with("https://") || url.starts_with("http://");
    if !ok
        || url.chars().any(|c| {
            c.is_control() || (c.is_whitespace() && !url.starts_with("mailto:")) || c == '"'
        })
    {
        return Err("only mailto: and web links".into());
    }
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn();
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
async fn grib_files() -> Vec<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(grib_files_blocking)
        .await
        .unwrap_or_default()
}

fn grib_files_blocking() -> Vec<serde_json::Value> {
    let loaded = GRIBS.lock().unwrap();
    let mut out = Vec::new();
    for dir in grib_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(ext.as_str(), "grb" | "grb2" | "grib" | "grib2") {
                continue;
            }
            let md = e.metadata().ok();
            let md2 = md.clone();
            let mtime = md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
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
        run(b)
            .cmp(&run(a))
            .then(b["mtime"].as_u64().cmp(&a["mtime"].as_u64()))
    });
    out
}

/// Header scans are cached by path, modification time and size.
fn scan_cached(p: &std::path::Path, mtime: u64, size: u64) -> Option<grib::FileInfo> {
    static CACHE: Mutex<BTreeMap<String, (u64, u64, Option<grib::FileInfo>)>> =
        Mutex::new(BTreeMap::new());
    let key = p.to_string_lossy().into_owned();
    if let Some((m, s, info)) = CACHE.lock().unwrap().get(&key) {
        if *m == mtime && *s == size {
            return info.clone();
        }
    }
    let info = grib::scan(p).ok();
    CACHE
        .lock()
        .unwrap()
        .insert(key, (mtime, size, info.clone()));
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
                let _ = app.emit(
                    "grib-progress",
                    serde_json::json!({ "path": p, "percent": pc, "stage": stage }),
                );
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
fn grib_wind_grid(
    path: String,
    t: i64,
    bbox: [f64; 4],
    max_points: usize,
) -> Option<serde_json::Value> {
    let f = GRIBS.lock().unwrap().get(&path)?.clone();
    let (g, u) = f.grid_at(grib::Param::WindU, t)?;
    let (_, v) = f.grid_at(grib::Param::WindV, t)?;
    // Grids may run 0..360 (GFS) while the chart asks in -180..180: shift the
    // view by whole turns so it lines up with the grid, and back for the output.
    let turn = ((g.lon0 + g.dlon * (g.ni as f64 - 1.0) / 2.0 - (bbox[0] + bbox[2]) / 2.0) / 360.0)
        .round()
        * 360.0;
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
fn grib_grid(
    path: String,
    layer: String,
    t: i64,
    bbox: [f64; 4],
    max_points: usize,
) -> Option<serde_json::Value> {
    use grib::Param as P;
    let f = GRIBS.lock().unwrap().get(&path)?.clone();
    const KN: f32 = 3600.0 / 1852.0;
    // the grid(s) and how to turn their values into the quantity shown
    let (g, vals): (grib::Grid, Vec<f32>) = match layer.as_str() {
        "wind" => {
            if let (Some((g, u)), Some((_, v))) = (f.grid_at(P::WindU, t), f.grid_at(P::WindV, t)) {
                (
                    g,
                    u.iter()
                        .zip(&v)
                        .map(|(a, b)| (a * a + b * b).sqrt() * KN)
                        .collect(),
                )
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
            (
                g,
                v.iter()
                    .map(|x| if *x > 0.0 { x.log10() } else { f32::NAN })
                    .collect(),
            )
        }
        "current" => {
            let ((g, u), (_, v)) = (f.grid_at(P::CurrentU, t)?, f.grid_at(P::CurrentV, t)?);
            (
                g,
                u.iter()
                    .zip(&v)
                    .map(|(a, b)| (a * a + b * b).sqrt() * KN)
                    .collect(),
            )
        }
        _ => return None,
    };
    let turn = ((g.lon0 + g.dlon * (g.ni as f64 - 1.0) / 2.0 - (bbox[0] + bbox[2]) / 2.0) / 360.0)
        .round()
        * 360.0;
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
fn grib_compare(
    paths: Vec<String>,
    metric: String,
    t: i64,
    bbox: [f64; 4],
    max_points: usize,
) -> Result<serde_json::Value, String> {
    let m = grib::compare::Metric::parse(&metric).ok_or("unknown comparison")?;
    let files: Vec<(String, Arc<grib::GribFile>)> = {
        let g = GRIBS.lock().unwrap();
        paths
            .iter()
            .filter_map(|p| g.get(p).map(|f| (p.clone(), f.clone())))
            .collect()
    };
    let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
    let r = grib::compare::agreement(&refs, m, t, bbox, max_points.clamp(100, 40_000)).ok_or(
        if m == grib::compare::Metric::Trend {
            "Needs two or more runs covering this time."
        } else {
            "Needs two or more GRIBs covering this time and area."
        },
    )?;
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
        paths
            .iter()
            .filter_map(|p| g.get(p).map(|f| (p.clone(), f.clone())))
            .collect()
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

fn score_json(
    files: &[(String, Arc<grib::GribFile>)],
    s: &grib::verify::Score,
) -> serde_json::Value {
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
async fn grib_verify_scores(
    paths: Vec<String>,
    t: i64,
    bbox: [f64; 4],
) -> Result<Vec<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = loaded(&paths);
        let refs: Vec<&grib::GribFile> = files.iter().map(|(_, f)| f.as_ref()).collect();
        let vt = grib::verify::verify_times(&refs)
            .into_iter()
            .find(|v| v.t == t)
            .ok_or("No GRIB has this time as hour 0.")?;
        Ok(grib::verify::scores(&refs, &vt, bbox, 6000)
            .iter()
            .map(|s| score_json(&files, s))
            .collect())
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

/// The error field of the forecast in `forecast` against the later run(s) at time `at` (their hour 0 when `at` is the verify time `t`); `kind` is vector, speed or direction.
#[tauri::command]
fn grib_verify_grid(
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
    let fi = files
        .iter()
        .position(|(p, _)| *p == forecast)
        .ok_or("That GRIB is not loaded.")?;
    let vt = grib::verify::verify_times(&refs)
        .into_iter()
        .find(|v| v.t == t)
        .ok_or("No GRIB has this time as hour 0.")?;
    let r = grib::verify::error_grid(&refs, &vt, fi, k, at, bbox, max_points.clamp(100, 40_000))
        .ok_or("Nothing to compare at this time: the forecast and the later run must both cover it (the later run starts at the verify time).")?;
    Ok(
        serde_json::json!({ "lon0": r.lon0, "lat0": r.lat0, "dlon": r.dlon, "dlat": r.dlat, "ni": r.ni, "nj": r.nj, "values": r.values,
        "used": r.used.iter().map(|&i| file_json(&files, i)).collect::<Vec<_>>() }),
    )
}

/// Every loaded GRIB at one point and time: the quick agreement check.
#[tauri::command]
fn grib_point(lat: f64, lon: f64, t: i64) -> Vec<serde_json::Value> {
    let files: Vec<(String, Arc<grib::GribFile>)> = GRIBS
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
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
fn point_values(f: &grib::GribFile, lat: f64, lon: f64, t: i64) -> serde_json::Value {
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
async fn grib_along(first: Option<String>, pts: Vec<(f64, f64, i64)>) -> Vec<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut files: Vec<(String, Arc<grib::GribFile>)> = GRIBS
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some(p) = &first {
            files.sort_by_key(|(k, _)| k != p);
        }
        pts.iter()
            .map(|&(lat, lon, t)| {
                let mut out = serde_json::Map::new();
                for (_, f) in &files {
                    if let serde_json::Value::Object(m) = point_values(f, lat, lon, t) {
                        for (k, v) in m {
                            if !v.is_null() && !out.contains_key(&k) {
                                out.insert(k, v);
                            }
                        }
                    }
                }
                serde_json::Value::Object(out)
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Signal K URL: `RACENAV_SK_URL` environment variable, else the local server.
pub(crate) fn sk_url() -> String {
    std::env::var("RACENAV_SK_URL").unwrap_or_else(|_| signalk::DEFAULT_URL.to_string())
}

/// Your own polars: `RACENAV_POLAR_DIR`, else `C:\dev\racenav-data\polars` when
/// racenav-data exists, else `<app data>\polars` (made on first use).
fn user_polar_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_POLAR_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    let d = if dev.is_dir() {
        dev.join("polars")
    } else {
        app.path()
            .app_data_dir()
            .map(|p| p.join("polars"))
            .unwrap_or_else(|_| PathBuf::from("polars"))
    };
    let _ = std::fs::create_dir_all(&d);
    d
}

/// The polars shipped with TweedPlot (read only).
fn bundled_polar_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let candidates = [
        app.path()
            .resource_dir()
            .ok()
            .map(|d| d.join("resources").join("polars")),
        app.path().resource_dir().ok().map(|d| d.join("polars")),
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources")
                .join("polars"),
        ),
    ];
    candidates.into_iter().flatten().find(|d| d.is_dir())
}

/// Polar folders, yours first: a polar of yours with the same name as a shipped one is used instead.
fn polar_dirs(app: &tauri::AppHandle) -> Vec<(PathBuf, &'static str)> {
    let mut v = vec![(user_polar_dir(app), "yours")];
    if let Some(b) = bundled_polar_dir(app) {
        v.push((b, "shipped"));
    }
    v.retain(|d| d.0.is_dir());
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
fn polar_files(app: tauri::AppHandle) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for (dir, source) in polar_dirs(&app) {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(ext.as_str(), "csv" | "pol" | "txt") {
                continue;
            }
            // only files that actually parse as polars
            if polar::Polar::from_path(&p).is_err() {
                continue;
            }
            let name = p
                .file_stem()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            if out
                .iter()
                .any(|o| o["name"].as_str() == Some(name.as_str()))
            {
                continue;
            }
            out.push(serde_json::json!({ "path": p.to_string_lossy(), "name": name, "dir": dir, "source": source }));
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

/// Check a polar file's text and copy it into your polar folder. Returns the saved path.
#[tauri::command]
fn polar_import(app: tauri::AppHandle, name: String, text: String) -> Result<String, String> {
    let stem: String = std::path::Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " ._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim().to_string();
    if stem.is_empty() {
        return Err("The file has no usable name.".into());
    }
    polar::Polar::parse(&stem, &text).map_err(|e| {
        format!(
            "Not a polar TweedPlot can read ({e}). See Manual > Reference > Polars for the format."
        )
    })?;
    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("pol")
        .to_ascii_lowercase();
    let ext = if matches!(ext.as_str(), "csv" | "pol" | "txt") {
        ext
    } else {
        "pol".into()
    };
    let dest = user_polar_dir(&app).join(format!("{stem}.{ext}"));
    std::fs::write(&dest, text).map_err(|e| format!("Could not save {}: {e}", dest.display()))?;
    Ok(dest.to_string_lossy().into_owned())
}

/// Open your polar folder in Explorer.
#[tauri::command]
fn polar_open_folder(app: tauri::AppHandle) -> Result<String, String> {
    let d = user_polar_dir(&app);
    #[cfg(windows)]
    std::process::Command::new("explorer")
        .arg(&d)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&d)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(all(unix, not(target_os = "macos")))]
    std::process::Command::new("xdg-open")
        .arg(&d)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(d.to_string_lossy().into_owned())
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
        let marks: Vec<nav_core::LatLon> = req
            .marks
            .iter()
            .map(|m| nav_core::LatLon::new(m[1], m[0]))
            .collect();
        if marks.is_empty() {
            return Err("Add at least one mark.".to_string());
        }
        let specs: Vec<routing::MarkSpec> = marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let o = req.mark_opts.get(i).cloned().unwrap_or_default();
                routing::MarkSpec {
                    pos: *m,
                    rounding: routing::Rounding::parse(&o.rounding),
                    radius_nm: (o.radius_nm > 0.0).then_some(o.radius_nm),
                }
            })
            .collect();
        // barriers: active lines and zones, and barrier marks that aren't on this course
        let mut barriers = routing::Barriers::default();
        let mut barrier_count = 0;
        if req.use_barriers {
            if let Ok(l) = lib() {
                for b in l
                    .barriers()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|b| b.active && b.points.len() >= 2)
                {
                    barrier_count += 1;
                    if b.kind == "zone" && b.points.len() >= 3 {
                        barriers.zones.push(b.points)
                    } else {
                        barriers.lines.push(b.points)
                    }
                }
                let on_course = |m: &library::Mark| {
                    let p = nav_core::LatLon::new(m.lat, m.lon);
                    marks
                        .iter()
                        .chain(std::iter::once(&start))
                        .any(|c| c.distance_to(&p) < 0.01)
                };
                for m in l
                    .marks()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| m.barrier_nm > 0.0 && !on_course(m))
                {
                    barrier_count += 1;
                    barriers.discs.push(([m.lon, m.lat], m.barrier_nm));
                }
            }
        }
        // waves for the wave-height limit: this GRIB, else a loaded wave GRIB covering the start
        let wave_grib: Option<Arc<grib::GribFile>> =
            if req.params.max_hs > 0.0 && grib.times(grib::Param::WaveHeight).is_empty() {
                let g = GRIBS.lock().unwrap();
                g.values()
                    .filter(|f| {
                        f.times(grib::Param::WaveHeight)
                            .first()
                            .is_some_and(|&a| a <= req.t0)
                            && f.times(grib::Param::WaveHeight)
                                .last()
                                .is_some_and(|&b| b >= req.t0)
                    })
                    .max_by_key(|f| f.model().1)
                    .cloned()
            } else {
                None
            };
        let clock = std::time::Instant::now();
        let raster = charts().map(|store| {
            let mut b = [start.lon, start.lat, start.lon, start.lat];
            for m in &marks {
                b = [
                    b[0].min(m.lon),
                    b[1].min(m.lat),
                    b[2].max(m.lon),
                    b[3].max(m.lat),
                ];
            }
            // room to go round headlands: a quarter of the span, at least 0.1 degree
            let pad = ((b[2] - b[0]).max(b[3] - b[1]) * 0.25).clamp(0.1, 2.0);
            routing::Raster::from_chart_with(
                &store,
                [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad],
                req.safety_depth,
                50.0,
                16_000_000,
                req.avoid,
            )
        });
        let raster_ms = clock.elapsed().as_millis();
        let r = match &wave_grib {
            Some(w) => {
                let src = routing::WithWaves {
                    wind: grib.as_ref(),
                    waves: w.as_ref(),
                };
                routing::route_marks(
                    start,
                    req.t0,
                    &specs,
                    &polar,
                    &src,
                    raster.as_ref(),
                    &barriers,
                    &req.params,
                )
            }
            None => routing::route_marks(
                start,
                req.t0,
                &specs,
                &polar,
                grib.as_ref(),
                raster.as_ref(),
                &barriers,
                &req.params,
            ),
        };
        let mut v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
        v["barriers"] = barrier_count.into();
        v["waves_from"] = match (&wave_grib, req.params.max_hs > 0.0) {
            (Some(w), _) => w.model().0.into(),
            (None, true) if !grib.times(grib::Param::WaveHeight).is_empty() => {
                "the same GRIB".into()
            }
            (None, true) => serde_json::Value::String(
                "none: no wave GRIB loaded, the wave limit was not applied".into(),
            ),
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
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("library");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("library"))
        .unwrap_or_else(|_| PathBuf::from("library"))
}

fn lib() -> Result<Arc<library::Library>, String> {
    LIBRARY
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| "The mark library could not be opened.".to_string())
}

// ---------- which library: New, Open, Recent, Save a copy, Merge ----------
// The library in use is remembered in library.json in the app's data folder, with the
// ones used recently. The default is <library folder>\library.gpkg.

fn library_settings_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("library.json"))
}

fn library_settings(app: &tauri::AppHandle) -> serde_json::Value {
    library_settings_file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// The library file to open at start: the one last used, if it's still there.
fn library_start_file(app: &tauri::AppHandle) -> PathBuf {
    let v = library_settings(app);
    v["current"].as_str().map(PathBuf::from).filter(|p| p.is_file()).unwrap_or_else(|| library_dir(app).join("library.gpkg"))
}

fn library_remember(app: &tauri::AppHandle, path: &std::path::Path) {
    let v = library_settings(app);
    let mut recent: Vec<String> = v["recent"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let p = path.to_string_lossy().to_string();
    recent.retain(|x| x != &p);
    recent.insert(0, p.clone());
    recent.truncate(8);
    if let Some(f) = library_settings_file(app) {
        let _ = std::fs::write(f, serde_json::json!({ "current": p, "recent": recent }).to_string());
    }
}

// ---------- one TimeZero symbol table for every library ----------
// What each TimeZero symbol number means is how the skipper (or the fleet) works, not part
// of one library, so the app keeps one table (tz-symbols.json in the app's data folder) and
// every library opened is brought into line with it. The first time, the table is the library
// in use's own; meanings a library has that the table lacks are added to it.

fn symbol_table_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("tz-symbols.json"))
}

fn symbol_table_read(app: &tauri::AppHandle) -> Option<Vec<(String, String, String)>> {
    symbol_table_file(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok())
}

fn symbol_table_write(app: &tauri::AppHandle, t: &[(String, String, String)]) {
    if let (Some(f), Ok(j)) = (symbol_table_file(app), serde_json::to_string_pretty(t)) {
        let _ = std::fs::write(f, j);
    }
}

/// Bring the library in use into line with the app's symbol table; returns marks changed.
fn symbols_sync(app: &tauri::AppHandle) -> Result<usize, String> {
    let l = lib()?;
    let own = l.symbol_table().map_err(es)?;
    let mut table = symbol_table_read(app).unwrap_or_default();
    let before = table.len();
    for e in own {
        if !table.iter().any(|t| t.0 == e.0) {
            table.push(e);
        }
    }
    if table.len() != before || symbol_table_file(app).is_some_and(|f| !f.is_file()) {
        table.sort();
        symbol_table_write(app, &table);
    }
    l.apply_symbol_table(&table).map_err(es)
}

/// Make `path` the library in use (opening, or making, it).
fn library_switch(app: &tauri::AppHandle, path: &std::path::Path) -> Result<(), String> {
    if REC.lock().unwrap().is_some() {
        return Err("The track recorder is on: stop recording before changing library.".into());
    }
    let l = library::Library::open_file(path).map_err(es)?;
    *LIBRARY.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(l));
    library_remember(app, path);
    if let Err(e) = symbols_sync(app) {
        eprintln!("symbol table: {e}");
    }
    library_changed(app);
    Ok(())
}

/// The library in use: its name, file, and the ones used recently.
#[tauri::command]
fn library_current(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let l = lib()?;
    let v = library_settings(&app);
    let recent: Vec<serde_json::Value> = v["recent"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .filter(|p| std::path::Path::new(p).is_file() && std::path::Path::new(p) != l.path())
                .map(|p| serde_json::json!({ "path": p, "name": std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() }))
                .collect()
        })
        .unwrap_or_default();
    Ok(serde_json::json!({ "name": l.name(), "path": l.path(), "dir": l.dir(), "recent": recent }))
}

/// Pick a library file: to open (`save` false) or to make or save a copy to (`save` true).
#[tauri::command]
async fn library_pick_db(app: tauri::AppHandle, save: bool, title: String, suggest: Option<String>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = app.dialog().file().set_title(&title).add_filter("TweedPlot library", &["gpkg"]);
    if let Ok(l) = lib() {
        d = d.set_directory(l.dir());
    }
    if let Some(s) = suggest {
        d = d.set_file_name(&s);
    }
    let picked = if save { d.blocking_save_file() } else { d.blocking_pick_file() };
    let Some(p) = picked else { return Ok(None) };
    let mut p = p.into_path().map_err(|e| e.to_string())?;
    if save && p.extension().is_none_or(|x| !x.eq_ignore_ascii_case("gpkg")) {
        p.set_extension("gpkg");
    }
    Ok(Some(p.to_string_lossy().into_owned()))
}

/// Open a library (or make a new one at `path`) and use it.
#[tauri::command]
async fn library_use(app: tauri::AppHandle, path: String, new: bool) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if new && p.exists() {
        return Err(format!("{path} is already there: open it instead, or choose another name"));
    }
    if !new && !p.is_file() {
        return Err(format!("{path} isn't there"));
    }
    let h = app.clone();
    tauri::async_runtime::spawn_blocking(move || library_switch(&h, &p)).await.map_err(|e| e.to_string())?
}

/// A copy of the whole library in use, at `path` (it stays the one in use).
#[tauri::command]
async fn library_save_copy(path: String) -> Result<(), String> {
    let l = lib()?;
    tauri::async_runtime::spawn_blocking(move || l.backup_to(std::path::Path::new(&path)).map_err(es)).await.map_err(|e| e.to_string())?
}

/// Bring another library's marks, routes, tracks, routings and barriers into the one in use.
#[tauri::command]
async fn library_merge(app: tauri::AppHandle, path: String, job: Option<String>) -> Result<library::ImportReport, String> {
    let job = job.unwrap_or_default();
    let cancel = job_begin(&job);
    let l = lib()?;
    let h = app.clone();
    let j = job.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = job_progress(&h, &j);
        l.merge_from(std::path::Path::new(&path), &mut progress, &cancel).map_err(es)
    })
    .await
    .map_err(|e| e.to_string());
    job_end(&job);
    let r = r??;
    let _ = symbols_sync(&app);
    library_changed(&app);
    Ok(r)
}

// ---------- trips ----------

#[tauri::command]
fn trips_list() -> Result<Vec<library::Trip>, String> {
    rec_flush();
    lib()?.trips().map_err(es)
}
/// Save a trip; `record` starts the track recorder (a new track) if it isn't on.
#[tauri::command]
fn trip_save(app: tauri::AppHandle, trip: library::Trip, record: bool) -> Result<library::Trip, String> {
    let t = lib()?.save_trip(trip).map_err(es)?;
    if record && t.ended.is_none() && REC.lock().unwrap().is_none() {
        rec_flush();
        rec_begin(None)?;
    }
    library_changed(&app);
    Ok(t)
}
/// End a trip now; `stop_recording` stops the track recorder too.
#[tauri::command]
fn trip_end(app: tauri::AppHandle, id: i64, stop_recording: bool) -> Result<library::Trip, String> {
    if stop_recording {
        rec_stop();
    } else {
        rec_flush();
    }
    let t = lib()?.end_trip(id, None).map_err(es)?;
    library_changed(&app);
    Ok(t)
}
#[tauri::command]
fn trips_delete(app: tauri::AppHandle, ids: Vec<i64>) -> Result<usize, String> {
    let n = lib()?.delete_trips(&ids).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
/// The marks and tracks made in a trip: { marks: [ids], tracks: [ids] }.
#[tauri::command]
fn trip_contents(id: i64) -> Result<serde_json::Value, String> {
    let (marks, tracks) = lib()?.trip_contents(id).map_err(es)?;
    Ok(serde_json::json!({ "marks": marks, "tracks": tracks }))
}

// ---------- Trash, undoing an import, a part of the library as a file ----------

#[tauri::command]
fn library_trash() -> Result<Vec<library::TrashItem>, String> {
    lib()?.trash().map_err(es)
}
/// Bring things back from the Trash; `kind` is mark, route, track, routing or barrier.
#[tauri::command]
fn library_restore(app: tauri::AppHandle, kind: String, ids: Vec<String>) -> Result<usize, String> {
    let n = lib()?.restore(&kind, &ids).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
/// Delete things in the Trash for good (no ids: all of that kind).
#[tauri::command]
fn library_purge(kind: String, ids: Vec<String>) -> Result<usize, String> {
    lib()?.purge(&kind, &ids).map_err(es)
}
#[tauri::command]
fn library_imports() -> Result<Vec<library::ImportInfo>, String> {
    lib()?.imports().map_err(es)
}
#[tauri::command]
fn library_undo_import(app: tauri::AppHandle, id: i64) -> Result<library::Counts, String> {
    let n = lib()?.undo_import(id).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
/// The chosen marks, routes and tracks as a library file of their own.
#[tauri::command]
async fn library_export(path: String, marks: Vec<String>, routes: Vec<String>, tracks: Vec<String>) -> Result<library::Counts, String> {
    let l = lib()?;
    tauri::async_runtime::spawn_blocking(move || l.export_library(std::path::Path::new(&path), &marks, &routes, &tracks).map_err(es))
        .await
        .map_err(|e| e.to_string())?
}

/// The library's own backups (newest first): file, date, size.
#[tauri::command]
fn library_backups() -> Result<Vec<serde_json::Value>, String> {
    let l = lib()?;
    let dir = l.dir().join("backups");
    let prefix = format!("{}-", l.name());
    let mut v: Vec<(String, PathBuf, u64)> = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    let day = n.strip_prefix(prefix.as_str())?.strip_suffix(".gpkg")?.to_string();
                    (day.len() == 10).then(|| (day, e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(v.into_iter().map(|(d, p, n)| serde_json::json!({ "day": d, "path": p, "bytes": n })).collect())
}

/// Use a backup: copied (the backup stays as it is) beside the library as
/// "<name> restored <day>.gpkg", which becomes the library in use.
#[tauri::command]
async fn library_use_backup(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let l = lib()?;
    let src = PathBuf::from(&path);
    let day = src.file_stem().map(|s| s.to_string_lossy().chars().rev().take(10).collect::<String>().chars().rev().collect::<String>()).unwrap_or_default();
    let mut to = l.dir().join(format!("{} restored {day}.gpkg", l.name()));
    let mut k = 2;
    while to.exists() {
        to = l.dir().join(format!("{} restored {day} ({k}).gpkg", l.name()));
        k += 1;
    }
    std::fs::copy(&src, &to).map_err(|e| e.to_string())?;
    let h = app.clone();
    let t = to.clone();
    tauri::async_runtime::spawn_blocking(move || library_switch(&h, &t)).await.map_err(|e| e.to_string())??;
    Ok(to.to_string_lossy().into_owned())
}

/// Change many marks at once (group, type, colour; empty or missing: unchanged).
#[tauri::command]
fn marks_update(app: tauri::AppHandle, ids: Vec<String>, group: Option<String>, kind: Option<String>, colour: Option<String>) -> Result<usize, String> {
    let n = lib()?.update_marks(&ids, group.as_deref(), kind.as_deref().filter(|k| !k.is_empty()), colour.as_deref()).map_err(es)?;
    library_changed(&app);
    Ok(n)
}

/// Several tracks in one GPX file in the export folder; returns its path.
#[tauri::command]
fn tracks_export(name: String, ids: Vec<String>) -> Result<String, String> {
    rec_flush();
    let l = lib()?;
    let text = l.export_tracks_gpx(&ids).map_err(es)?;
    Ok(l.write_export(&name, &text).map_err(es)?.to_string_lossy().into_owned())
}

fn es(e: library::Error) -> String {
    e.to_string()
}

#[tauri::command]
fn lib_info() -> Result<serde_json::Value, String> {
    let l = lib()?;
    Ok(serde_json::json!({ "dir": l.dir(), "file": l.path(), "converting": l.converting() }))
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

/// Tell every window the library changed, so each reloads its marks, routes and routings.
fn library_changed(app: &tauri::AppHandle) {
    let _ = app.emit("library-changed", ());
}

#[tauri::command]
fn marks_list() -> Result<Vec<library::Mark>, String> {
    lib()?.marks().map_err(es)
}
#[tauri::command]
fn mark_save(app: tauri::AppHandle, mark: library::Mark) -> Result<library::Mark, String> {
    let m = lib()?.save_mark(mark).map_err(es)?;
    library_changed(&app);
    Ok(m)
}
#[tauri::command]
fn mark_delete(app: tauri::AppHandle, id: String, force: bool) -> Result<(), String> {
    lib()?.delete_mark(&id, force).map_err(es)?;
    library_changed(&app);
    Ok(())
}
/// The TimeZero symbols the marks use, with what each has been given to mean.
#[tauri::command]
fn symbols_list() -> Result<Vec<library::SymbolUse>, String> {
    lib()?.symbols().map_err(es)
}
/// Give a TimeZero symbol a meaning (a mark type and label); returns how many marks changed.
#[tauri::command]
fn symbol_set(app: tauri::AppHandle, symbol: String, kind: String, label: String) -> Result<usize, String> {
    let n = lib()?.set_symbol(&symbol, &kind, &label).map_err(es)?;
    // the one table every library uses
    let mut table = symbol_table_read(&app).unwrap_or_default();
    table.retain(|t| t.0 != symbol);
    table.push((symbol, kind, label.trim().to_string()));
    table.sort();
    symbol_table_write(&app, &table);
    library_changed(&app);
    Ok(n)
}
/// The marks in an area (west, south, east, north), from the spatial index.
#[tauri::command]
fn marks_in(west: f64, south: f64, east: f64, north: f64) -> Result<Vec<library::Mark>, String> {
    lib()?.marks_in(west, south, east, north).map_err(es)
}

#[tauri::command]
fn marks_delete(app: tauri::AppHandle, ids: Vec<String>, force: bool) -> Result<usize, String> {
    let n = lib()?.delete_marks(&ids, force).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
#[tauri::command]
fn routes_list() -> Result<Vec<library::Route>, String> {
    lib()?.routes().map_err(es)
}
#[tauri::command]
fn route_save(app: tauri::AppHandle, route: library::Route) -> Result<library::Route, String> {
    let r = lib()?.save_route(route).map_err(es)?;
    library_changed(&app);
    Ok(r)
}
#[tauri::command]
fn route_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    lib()?.delete_route(&id).map_err(es)?;
    library_changed(&app);
    Ok(())
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
fn routing_save(
    app: tauri::AppHandle,
    routing: library::Routing,
) -> Result<library::RoutingSummary, String> {
    let r = lib()?.save_routing(routing).map_err(es)?;
    library_changed(&app);
    Ok(r)
}
#[tauri::command]
fn routing_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    lib()?.delete_routing(&id).map_err(es)?;
    library_changed(&app);
    Ok(())
}
/// GPX of marks and routes (empty lists = everything) written to the export folder.
#[tauri::command]
fn gpx_export(
    name: String,
    mark_ids: Vec<String>,
    route_ids: Vec<String>,
) -> Result<String, String> {
    let l = lib()?;
    let text = l.export_gpx(&mark_ids, &route_ids).map_err(es)?;
    Ok(l.write_export(&name, &text)
        .map_err(es)?
        .to_string_lossy()
        .into_owned())
}
/// Any GPX text the page made (e.g. a routing with times) written to the export folder.
#[tauri::command]
fn gpx_write(name: String, text: String) -> Result<String, String> {
    Ok(lib()?
        .write_export(&name, &text)
        .map_err(es)?
        .to_string_lossy()
        .into_owned())
}
#[tauri::command]
fn gpx_import(
    app: tauri::AppHandle,
    text: String,
    group: String,
) -> Result<library::ImportReport, String> {
    let r = lib()?.import_gpx(&text, &group).map_err(es)?;
    library_changed(&app);
    Ok(r)
}

/// Import marks, routes and tracks from a file picked in the system file dialog: GPX, or
/// TimeZero's .tzd / .TzBackup (or the WaypointRouteDatabase.xml inside one). The file is read
/// here, not passed through the page: a TimeZero file can be gigabytes.
#[tauri::command]
async fn library_pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("Import marks, routes and tracks")
        .add_filter("GPX and TimeZero", &["gpx", "tzd", "TzBackup", "xml"])
        .add_filter("All files", &["*"])
        .blocking_pick_file();
    let Some(p) = picked else { return Ok(None) };
    Ok(Some(p.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned()))
}

/// Import marks, routes and tracks from a file (picked with library_pick_file): GPX, or
/// TimeZero's .tzd / .TzBackup (or the WaypointRouteDatabase.xml inside one). The file is read
/// here, not passed through the page: a TimeZero file can be gigabytes.
#[tauri::command]
async fn library_import_path(app: tauri::AppHandle, path: String, job: Option<String>) -> Result<library::ImportReport, String> {
    let path = std::path::PathBuf::from(path);
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let job = job.unwrap_or_default();
    let cancel = job_begin(&job);
    let h = app.clone();
    let j = job.clone();
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<library::ImportReport, String> {
        let l = lib()?;
        let mut progress = job_progress(&h, &j);
        if ext == "gpx" {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            l.import_gpx_with(&text, &stem, &mut progress).map_err(es)
        } else {
            l.import_tzd_with(&path, &stem, &mut progress, &cancel).map_err(es)
        }
    })
    .await
    .map_err(|e| e.to_string());
    job_end(&job);
    let r = r??;
    library_changed(&app);
    Ok(r)
}

// ---------- jobs: long tasks the page shows on an activity card ----------
// A job has an id from the page; it reports progress as "job-progress" events
// ({ job, step, done, total }) and can be stopped with job_cancel.

static JOBS: OnceLock<Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>> = OnceLock::new();

fn job_begin(job: &str) -> Arc<std::sync::atomic::AtomicBool> {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if !job.is_empty() {
        JOBS.get_or_init(Default::default).lock().unwrap().insert(job.to_string(), flag.clone());
    }
    flag
}

fn job_end(job: &str) {
    if let Some(m) = JOBS.get() {
        m.lock().unwrap().remove(job);
    }
}

/// A progress callback for a job: sends "job-progress" at most five times a second, and
/// always when the step changes or it finishes.
fn job_progress(app: &tauri::AppHandle, job: &str) -> impl FnMut(&str, usize, usize) {
    let app = app.clone();
    let job = job.to_string();
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let mut last_step = String::new();
    move |step: &str, done: usize, total: usize| {
        if job.is_empty() {
            return;
        }
        if step != last_step || done == total || last.elapsed().as_millis() >= 200 {
            last = std::time::Instant::now();
            last_step = step.to_string();
            let _ = app.emit("job-progress", serde_json::json!({ "job": job, "step": step, "done": done, "total": total }));
        }
    }
}

/// Stop a job (it finishes what it's doing and keeps what's done).
#[tauri::command]
fn job_cancel(job: String) {
    if let Some(f) = JOBS.get().and_then(|m| m.lock().unwrap().get(&job).cloned()) {
        f.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---------- the activity log: every job and result, for support ----------

fn activity_log_path(app: &tauri::AppHandle) -> PathBuf {
    let base = library_dir(app);
    base.parent().map(|p| p.to_path_buf()).unwrap_or(base).join("logs").join("activity.log")
}

/// Add a line to the activity log (`<data folder>\logs\activity.log`; kept under 1 MB).
#[tauri::command]
fn activity_log(app: tauri::AppHandle, line: String) -> Result<(), String> {
    use std::io::Write;
    let p = activity_log_path(&app);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::rename(&p, p.with_extension("old.log"));
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).map_err(|e| e.to_string())?;
    writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), line.replace('\n', " ")).map_err(|e| e.to_string())
}

/// The last `n` lines of the activity log, and where it is.
#[tauri::command]
fn activity_log_read(app: tauri::AppHandle, n: usize) -> serde_json::Value {
    let p = activity_log_path(&app);
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(n)..].to_vec();
    serde_json::json!({ "path": p, "lines": tail })
}

// ---------- tracks and the track recorder ----------

/// The track being recorded from Signal K, when recording is on.
static REC: Mutex<Option<library::Recorder>> = Mutex::new(None);
const REC_FILE: &str = "recorder.json";

fn rec_settings_save(on: bool, id: &str) {
    if let Ok(l) = lib() {
        let _ = std::fs::write(
            l.dir().join(REC_FILE),
            serde_json::json!({ "on": on, "track": id }).to_string(),
        );
    }
}

fn new_log_track() -> library::Track {
    library::Track {
        id: String::new(),
        name: chrono::Local::now()
            .format("Log %d %b %Y %H:%M")
            .to_string(),
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
        None => library::Track {
            id: l.new_track_id(),
            ..new_log_track()
        },
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

/// Carry on recording if it was on when the app closed: the same track if its last fix
/// is under 2 h old, else a new one.
fn rec_resume() {
    let st = lib().ok().and_then(|l| std::fs::read_to_string(l.dir().join(REC_FILE)).ok());
    let Some(v) = st.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) else { return };
    if v["on"].as_bool() != Some(true) {
        return;
    }
    let id = v["track"].as_str().unwrap_or("").to_string();
    let recent = lib()
        .ok()
        .and_then(|l| l.track(&id).ok())
        .and_then(|t| t.stats.end)
        .is_some_and(|e| chrono_now() - e < 2 * 3600);
    if let Err(e) = rec_begin(recent.then_some(id.as_str())) {
        eprintln!("recorder: {e}");
    }
}

/// Offer a Signal K fix to the recorder; saves every 30 s.
pub(crate) fn rec_vessel(app: &tauri::AppHandle, v: &signalk::VesselState) {
    let (Some(lat), Some(lon)) = (v.lat, v.lon) else {
        return;
    };
    let now = chrono_now();
    let t = v
        .datetime
        .as_deref()
        .and_then(library::tracks::parse_iso)
        .unwrap_or(now);
    let (kept, due, stats) = {
        let mut g = REC.lock().unwrap();
        let Some(r) = g.as_mut() else { return };
        let kept = r.push(library::TrackPoint {
            t: Some(t),
            lat,
            lon,
            sog: v.sog,
            cog: v.cog,
            depth: v.depth_surface.or(v.depth),
            temp: v.water_temp,
        });
        (kept, r.due_save(now, 30), r.track.stats.clone())
    };
    if kept {
        let _ = app.emit(
            "rec-point",
            serde_json::json!({ "lon": lon, "lat": lat, "t": t, "stats": stats }),
        );
    }
    if due {
        std::thread::spawn(rec_flush);
    }
}

#[tauri::command]
fn rec_status() -> serde_json::Value {
    let g = REC.lock().unwrap();
    match g.as_ref() {
        Some(r) => {
            serde_json::json!({ "on": true, "id": r.track.id, "name": r.track.name, "stats": r.track.stats })
        }
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
    let id = REC
        .lock()
        .unwrap()
        .take()
        .map(|r| r.track.id)
        .unwrap_or_default();
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
        .map(|s| {
            s.iter()
                .map(|p| serde_json::json!([p.lon, p.lat, p.t, p.sog, p.cog, p.depth, p.temp]))
                .collect()
        })
        .collect();
    Ok(
        serde_json::json!({ "id": t.id, "name": t.name, "colour": t.colour, "stats": t.stats, "segments": segs }),
    )
}

#[tauri::command]
fn track_update(
    id: String,
    name: String,
    notes: String,
    colour: String,
) -> Result<library::TrackSummary, String> {
    rec_flush();
    let s = lib()?
        .update_track_info(&id, &name, &notes, &colour)
        .map_err(es)?;
    if let Some(r) = REC.lock().unwrap().as_mut().filter(|r| r.track.id == id) {
        r.track.name = s.name.clone();
        r.track.notes = s.notes.clone();
        r.track.colour = s.colour.clone();
    }
    Ok(s)
}

#[tauri::command]
fn track_delete(id: String) -> Result<(), String> {
    if REC
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|r| r.track.id == id)
    {
        return Err("This track is being recorded: stop recording first.".into());
    }
    lib()?.delete_track(&id).map_err(es)
}

/// Delete many tracks (not the one being recorded); returns how many went.
#[tauri::command]
fn tracks_delete(ids: Vec<String>) -> Result<usize, String> {
    let recording = REC.lock().unwrap().as_ref().map(|r| r.track.id.clone());
    let l = lib()?;
    let mut n = 0;
    for id in ids.iter().filter(|id| Some(*id) != recording.as_ref()) {
        if l.delete_track(id).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// Write a track as GPX into the library's export folder; returns the path.
#[tauri::command]
fn track_export(id: String) -> Result<String, String> {
    rec_flush();
    let l = lib()?;
    let t = l.track(&id).map_err(es)?;
    Ok(l.write_export(&t.name, &t.to_gpx())
        .map_err(es)?
        .to_string_lossy()
        .into_owned())
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
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("tides");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("tides"))
        .unwrap_or_else(|_| PathBuf::from("tides"))
}

fn tide_db() -> Arc<tides::TideDb> {
    let mut g = TIDES.lock().unwrap();
    if g.is_none() {
        let dir = TIDE_DIR
            .get()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("tides"));
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
        tides::Kind::Secondary {
            reference,
            hw_min,
            lw_min,
            ratio,
        } => format!(
            "Secondary to {reference}: HW {:+} min, LW {:+} min{}",
            hw_min,
            lw_min,
            ratio.map_or(", times only".to_string(), |r| format!(", range × {r:.2}"))
        ),
        tides::Kind::LevelsOnly { reference } => {
            format!("Reference port {reference}: LINZ gives levels only, no times")
        }
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

/// Where the tide is at every station at `t` (the gauges beside the tide diamonds):
/// [{id, f: 0 low .. 1 high, rising}], for stations with predictions.
#[tauri::command]
fn tides_phase(t: i64) -> Vec<serde_json::Value> {
    let db = tide_db();
    db.stations
        .iter()
        .filter_map(|s| {
            let (f, rising) = db.phase_at(&s.id, t)?;
            Some(serde_json::json!({ "id": s.id, "f": (f * 1000.0).round() / 1000.0, "rising": rising }))
        })
        .collect()
}

/// The tidal streams in `bbox` [w, s, e, n] at `t`: each charted tidal diamond with its
/// table and the stream now (from the reference port's high water, the rate scaled from
/// springs by the tide's range), and the charted flood and ebb arrows.
#[tauri::command]
async fn tidal_streams(bbox: [f64; 4], t: i64) -> Result<serde_json::Value, String> {
    let Some(store) = charts() else { return Ok(serde_json::json!({ "panels": [], "arrows": [] })) };
    tauri::async_runtime::spawn_blocking(move || {
        let (panels, arrows) = store.tidal_streams(bbox);
        let db = tide_db();
        let out: Vec<serde_json::Value> = panels
            .iter()
            .map(|p| {
                let mut v = serde_json::to_value(p).unwrap_or_default();
                let refst = db.daily_by_name(&p.ref_name);
                let Some(st) = refst else {
                    v["note"] = format!("No LINZ tide table for {} (download the tides in Weather and tides › Tides)", p.ref_name).into();
                    return v;
                };
                v["ref_id"] = st.id.clone().into();
                let Some((tref, factor)) = db.nearest_reference(&st.id, t, p.rel != "LW") else {
                    v["note"] = format!("No {} prediction for {} near this time", p.rel, p.ref_name).into();
                    return v;
                };
                let h = ((t - tref) as f64 / 3600.0).clamp(-6.0, 6.0);
                let x = h + 6.0;
                let i = (x.floor() as usize).min(11);
                let w = x - i as f64;
                let (d0, r0) = p.hours[i];
                let (d1, r1) = p.hours[i + 1];
                // direction: the short way round; rate: straight line
                let dd = ((d1 - d0 + 540.0) % 360.0) - 180.0;
                let mut dir = (d0 + dd * w + 360.0) % 360.0;
                let spring = r0 + (r1 - r0) * w;
                // a stream turning through slack: take the nearer hour's direction
                if r0 < 0.15 || r1 < 0.15 {
                    dir = if w < 0.5 { d0 } else { d1 };
                }
                let rate = spring * factor.unwrap_or(1.0);
                v["ref_t"] = tref.into();
                v["hour"] = ((h * 10.0).round() / 10.0).into();
                v["dir"] = ((dir * 10.0).round() / 10.0).into();
                v["rate"] = ((rate * 100.0).round() / 100.0).into();
                v["spring"] = ((spring * 100.0).round() / 100.0).into();
                if let Some(f) = factor {
                    v["factor"] = ((f * 100.0).round() / 100.0).into();
                }
                v
            })
            .collect();
        Ok(serde_json::json!({ "panels": out, "arrows": arrows }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download the LINZ secondary ports file and daily tables for `years` (emits "tides-progress").
#[tauri::command]
async fn tides_download(
    app: tauri::AppHandle,
    years: Vec<i32>,
) -> Result<serde_json::Value, String> {
    TIDE_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    let dir = TIDE_DIR.get().cloned().ok_or("no tide folder")?;
    let n = tauri::async_runtime::spawn_blocking(move || {
        tides::download(&dir, &years, &TIDE_CANCEL, |p| {
            let _ = app.emit(
                "tides-progress",
                serde_json::json!({ "stage": p.stage, "done": p.done, "total": p.total }),
            );
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
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("tracking");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("tracking"))
        .unwrap_or_else(|_| PathBuf::from("tracking"))
}

fn with_race<T>(
    key: &str,
    f: impl FnOnce(&mut tracking::Race) -> Result<T, String>,
) -> Result<T, String> {
    let key = tracking::clean_key(key).map_err(|e| e.to_string())?;
    let mut races = RACES.lock().unwrap();
    if !races.contains_key(&key) {
        let dir = TRACKING_DIR.get().ok_or("no tracking folder")?;
        races.insert(
            key.clone(),
            tracking::Race::open(dir, &key).map_err(|e| e.to_string())?,
        );
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
    let Some(dir) = TRACKING_DIR.get() else {
        return Vec::new();
    };
    let mut v: Vec<(u64, serde_json::Value)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let key = e.file_name().to_string_lossy().into_owned();
            let m = std::fs::metadata(e.path().join("fixes.json"))
                .or_else(|_| std::fs::metadata(e.path()));
            let t = m
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let title = std::fs::read_to_string(e.path().join("setup.json"))
                .ok()
                .and_then(|s| tracking::parse_setup(&s).ok())
                .map(|s| s.title)
                .unwrap_or_default();
            (
                t,
                serde_json::json!({ "key": key, "title": title, "modified": t }),
            )
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
    let dev = dev_root().join("linz");
    if dev.is_dir() {
        return dev;
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("charts"))
        .unwrap_or_else(|_| PathBuf::from("charts"))
}

/// World coastline folder (GSHHG zip or GSHHS_shp): `RACENAV_COAST_DIR`, else
/// `C:\dev\racenav-data\coastline` when racenav-data exists, else `<app data>\coastline`.
fn coast_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_COAST_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("coastline");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("coastline"))
        .unwrap_or_else(|_| PathBuf::from("coastline"))
}

/// Downloaded ETOPO areas: `RACENAV_BATHY_DIR`, else `C:\dev\racenav-data\bathy` when
/// racenav-data exists, else `<app data>\bathy`.
fn bathy_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_BATHY_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("bathy");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("bathy"))
        .unwrap_or_else(|_| PathBuf::from("bathy"))
}

/// Maritime boundaries and fishing rules (downloads in `boundaries` and `mpi`):
/// `RACENAV_MARINE_DIR`, else `C:\dev\racenav-data\marine` when racenav-data exists,
/// else `<app data>\marine`.
fn marine_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_MARINE_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("marine");
    }
    app.path()
        .app_data_dir()
        .map(|p| p.join("marine"))
        .unwrap_or_else(|_| PathBuf::from("marine"))
}

/// The folder holding the bundled world depths (`world_depth.gpkg`).
fn world_depth_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let candidates = [
        app.path().resource_dir().ok().map(|d| d.join("resources")),
        app.path().resource_dir().ok(),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|d| d.join("world_depth.gpkg").is_file())
}

/// (Re)open the chart store in the background: LINZ charts, GSHHG coastline,
/// bundled world depths and downloaded ETOPO areas. Emits chart-ready.
pub(crate) fn open_charts(handle: tauri::AppHandle) {
    open_charts_started(handle);
}

fn open_charts_started(handle: tauri::AppHandle) {
    CHARTS_READY.store(false, std::sync::atomic::Ordering::Release);
    *CHART_ERROR.lock().unwrap() = None;
    let dir = chart_dir(&handle);
    let mut dirs = vec![dir.clone(), coast_dir(&handle), bathy_dir(&handle), marine_dir(&handle)];
    if let Some(w) = world_depth_dir(&handle) {
        dirs.push(w);
    }
    // S-57 ENC cells (and the IHO S-63/S-64 test data): enc and s63 in the data folders
    let enc_dirs = s63_cmd::enc_dirs(&handle);
    // the first run with a GSHHG download builds the world coastline file (a minute or two)
    let _ = handle.emit(
        "chart-status",
        serde_json::json!({ "detail": "preparing charts: finding chart files" }),
    );
    log_chart_tile("charts: opening started");
    let sets_off = chart_sets_off(&handle);
    if !sets_off.is_empty() {
        log_chart_tile(&format!("charts: {} chart set(s) not in use", sets_off.len()));
    }
    std::thread::spawn(move || {
        // convert new or changed ENC cells first, saying how far it has got (a big
        // exchange set, such as all of NOAA's, takes a while the first time)
        let t_enc = std::time::Instant::now();
        chart::enc::clear_notes();
        let mut prepared_enc_gpkgs = Vec::new();
        for d in enc_dirs
            .iter()
            .filter(|d| d.ends_with("enc") || d.ends_with("s63"))
        {
            let h = handle.clone();
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            prepared_enc_gpkgs.extend(chart::enc::ensure_gpkgs_sets(d, &sets_off, &mut |done, todo, name| {
                if last.elapsed().as_millis() >= 500 || done == todo || todo == 0 {
                    last = std::time::Instant::now();
                    let detail = if todo == 0 {
                        format!("converting ENC cells: {name}")
                    } else {
                        format!("converting ENC cells: {done} of {todo} {name}")
                    };
                    let _ = h.emit("chart-status", serde_json::json!({ "detail": detail }));
                }
            }));
        }
        log_chart_tile(&format!("charts: ENC cells checked in {:.1} s", t_enc.elapsed().as_secs_f64()));
        open_store(handle, dirs, dir, prepared_enc_gpkgs)
    });
}

fn open_store(
    handle: tauri::AppHandle,
    dirs: Vec<PathBuf>,
    dir: PathBuf,
    prepared_enc_gpkgs: Vec<PathBuf>,
) {
    let h = handle.clone();
    let t0 = std::time::Instant::now();
    let opened =
        chart::ChartStore::open_dirs_with_extra_gpkgs(&dirs, &prepared_enc_gpkgs, &mut |k, n| {
            // only worth saying when there are a lot (thousands of ENC cells)
            if t0.elapsed().as_secs() >= 1 {
                let _ = h.emit(
                    "chart-status",
                    serde_json::json!({
                        "detail": format!("preparing charts: opening {k} of {n} files")
                    }),
                );
            }
        });
    match opened {
        Ok(store) => {
            let t0 = std::time::Instant::now();
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            store.prepare_with_progress(&mut |done, total| {
                if last.elapsed().as_millis() >= 250 || done == total {
                    last = std::time::Instant::now();
                    let _ = handle.emit(
                        "chart-status",
                        serde_json::json!({
                            "detail": format!(
                                "preparing charts: indexing coverage {done} of {total} tables"
                            )
                        }),
                    );
                }
            });
            log_chart_tile(&format!(
                "charts: render indexes prepared in {:.1} s",
                t0.elapsed().as_secs_f64()
            ));
            let n = store.tables.len();
            let world = store.has_world_land();
            *CHARTS.write().unwrap() = Some(Arc::new(store));
            CHARTS_READY.store(true, std::sync::atomic::Ordering::Release);
            let _ = handle.emit(
                "chart-ready",
                serde_json::json!({ "ready": true, "tables": n, "dir": dir, "world_land": world }),
            );
        }
        Err(e) => {
            *CHART_ERROR.lock().unwrap() = Some(e.to_string());
            CHARTS_READY.store(true, std::sync::atomic::Ordering::Release);
            let _ = handle.emit(
                "chart-ready",
                serde_json::json!({ "ready": true, "tables": 0, "dir": dir, "error": e.to_string() }),
            );
        }
    }
}

// ---------- chart sets ----------
// Each top-level folder of an ENC folder (NOAA's ENC_ROOT, the IHO test data …) is a chart
// set that can be left out of use: not checked or opened at start-up, kept converted on
// disk, ready at once when back in use. Kept in chart-sets.json in the app's data folder.

fn chart_sets_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("chart-sets.json"))
}

fn chart_sets_off(app: &tauri::AppHandle) -> Vec<PathBuf> {
    chart_sets_file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["off"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(PathBuf::from)).collect()))
        .unwrap_or_default()
}

/// The chart sets in the ENC folders, with their cells, producers and whether each is in use.
#[tauri::command]
async fn chart_sets_list(app: tauri::AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let off = chart_sets_off(&app);
    let dirs = s63_cmd::enc_dirs(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let mut v = Vec::new();
        for d in dirs.iter().filter(|d| d.is_dir()) {
            for s in chart::enc::chart_sets(d) {
                let on = !off.contains(&s.path);
                v.push(serde_json::json!({ "path": s.path, "name": s.name, "cells": s.cells, "producers": s.producers, "folder": d, "in_use": on }));
            }
        }
        v
    })
    .await
    .map_err(|e| e.to_string())
}

/// Put a chart set in or out of use, and open the charts again.
#[tauri::command]
fn chart_set_use(app: tauri::AppHandle, path: String, on: bool) -> Result<(), String> {
    let mut off = chart_sets_off(&app);
    let p = PathBuf::from(&path);
    off.retain(|x| x != &p);
    if !on {
        off.push(p);
    }
    let file = chart_sets_file(&app).ok_or("no app data folder")?;
    std::fs::write(&file, serde_json::json!({ "off": off }).to_string()).map_err(|e| e.to_string())?;
    open_charts(app);
    Ok(())
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
                    etopo.push(serde_json::json!({ "file": p, "name": p.file_name().map(|n| n.to_string_lossy().into_owned()), "area": area, "source": source, "spacing": spacing, "bytes": size, "old": chart::bathy::file_format(&p) < chart::bathy::FORMAT }));
                }
            }
        }
    }
    let store = charts();
    let linz = store
        .as_ref()
        .map(|s| {
            s.tables
                .iter()
                .filter(|t| {
                    matches!(
                        t.kind,
                        chart::Kind::DepthArea | chart::Kind::Land | chart::Kind::Contour
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let mdir = marine_dir(&app);
    let boundary_zips = std::fs::read_dir(mdir.join("boundaries"))
        .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "zip")).count())
        .unwrap_or(0);
    let mut mpi = Vec::new();
    if let Ok(rd) = std::fs::read_dir(mdir.join("mpi")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                let fetched = std::fs::read(&p)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|v| v["fetched"].as_str().map(str::to_string));
                mpi.push(serde_json::json!({ "name": p.file_stem().map(|n| n.to_string_lossy().into_owned()), "fetched": fetched, "bytes": e.metadata().map(|m| m.len()).unwrap_or(0) }));
            }
        }
    }
    let (n_bound, n_rules) = store
        .as_ref()
        .map(|s| {
            (
                s.tables.iter().filter(|t| t.kind == chart::Kind::Boundary).map(|t| t.rows).sum::<i64>(),
                s.tables.iter().filter(|t| t.kind == chart::Kind::Rule).map(|t| t.rows).sum::<i64>(),
            )
        })
        .unwrap_or((0, 0));
    serde_json::json!({
        "boundaries": { "zips": boundary_zips, "lines": n_bound },
        "mpi": { "files": mpi, "pieces": n_rules },
        "world_depth": world_depth_dir(&app).is_some(),
        "gshhg": gshhg,
        "world_land": store.as_ref().is_some_and(|s| s.has_world_land()),
        "gshhg_bytes": gribdl::data::GSHHG_BYTES,
        "etopo": etopo,
        "linz_tables": linz,
        "dirs": { "charts": chart_dir(&app), "coast": coast, "bathy": bdir, "marine": mdir },
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

/// Download LINZ's maritime boundaries (territorial sea, contiguous zone, EEZ, the
/// outlying islands) and reopen the charts (which builds them). Returns what failed.
#[tauri::command]
async fn data_boundaries(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    data_guard()?;
    let dir = marine_dir(&app);
    let h = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let progress = |done: u64, total: u64| {
            let _ = h.emit("data-progress", serde_json::json!({ "stage": format!("Maritime boundaries (LINZ): file {} of {total}", (done + 1).min(total)), "done": 0, "total": 0 }));
        };
        gribdl::marine::boundaries(&dir, &progress, &DATA_CANCEL).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());
    DATA_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    let (got, failed) = r??;
    open_charts(app);
    Ok(serde_json::json!({ "files": got, "failed": failed }))
}

/// Download MPI's fishing rules (commercial fishing regulations, customary areas,
/// voluntary closures, fisheries management and statistical areas) and reopen the charts.
#[tauri::command]
async fn data_mpi(app: tauri::AppHandle) -> Result<gribdl::marine::MpiSummary, String> {
    data_guard()?;
    let dir = marine_dir(&app);
    let h = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let progress = |stage: &str, done: u64, total: u64| {
            let name = stage.trim_start_matches("MARINE_").replace('_', " ");
            let _ = h.emit("data-progress", serde_json::json!({ "stage": format!("Fishing rules (MPI): {name}, layer {} of {total}", (done + 1).min(total.max(1))), "done": 0, "total": 0 }));
        };
        gribdl::marine::mpi(&dir, &progress, &DATA_CANCEL).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());
    DATA_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    let sum = r??;
    open_charts(app);
    Ok(sum)
}

/// The fishing rules at each of `points` ([lon, lat]): rule id and flags (see
/// `chart::marine`). For the closed-area alarm and the rules at the cursor.
#[tauri::command]
fn rules_at(points: Vec<[f64; 2]>) -> Vec<Vec<chart::marine::RuleHit>> {
    charts().map(|s| s.rules_at(&points)).unwrap_or_else(|| vec![Vec::new(); points.len()])
}

/// One fishing rule's details: the MPI layers it's in and all its attributes.
#[tauri::command]
fn rule_info(rid: i64) -> Option<serde_json::Value> {
    let (layers, attrs) = charts()?.rule_info(rid)?;
    Some(serde_json::json!({ "layers": layers, "attrs": attrs }))
}

/// The high-resolution depth tiles for `area` ([w, s, e, n]): which exist on LINZ's server
/// (that list is fetched once a week), and which are downloaded already.
#[tauri::command]
async fn hires_plan(app: tauri::AppHandle, area: [f64; 4]) -> Result<serde_json::Value, String> {
    let dir = hires_dir(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let names = chart::hires::file_names(area);
        let av = gribdl::coastal::available(&dir).map_err(|e| e.to_string())?;
        let there: Vec<&String> = names.iter().filter(|n| av.contains_key(*n)).collect();
        let have = there.iter().filter(|n| dir.join(format!("{n}.tiff")).exists()).count();
        Ok(serde_json::json!({ "tiles": names.iter().filter(|n| n.len() == 4).count(), "available": there.len(), "have": have }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download the high-resolution depth tiles for `area` that aren't here yet (at most 60).
#[tauri::command]
async fn hires_download(app: tauri::AppHandle, area: [f64; 4]) -> Result<usize, String> {
    data_guard()?;
    let dir = hires_dir(&app);
    let h = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let names = chart::hires::file_names(area);
        let av = gribdl::coastal::available(&dir).map_err(|e| e.to_string())?;
        let want: Vec<(String, String)> = names.into_iter().filter_map(|n| av.get(&n).map(|p| (n, p.clone()))).filter(|(n, _)| !dir.join(format!("{n}.tiff")).exists()).collect();
        if want.len() > 60 {
            return Err(format!("{} tiles: zoom in to a smaller area (60 at a time)", want.len()));
        }
        let progress = |i: usize, n: usize, bytes: u64| {
            let _ = h.emit("data-progress", serde_json::json!({ "stage": format!("High-resolution depths (LINZ): tile {} of {n}", (i + 1).min(n)), "done": bytes, "total": 0 }));
        };
        gribdl::coastal::download(&want, &dir, &progress, &DATA_CANCEL).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());
    DATA_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    let n = r??;
    open_hires(&app);
    Ok(n)
}

/// The high-resolution depth tiles here: name, area, size.
#[tauri::command]
fn hires_status(app: tauri::AppHandle) -> serde_json::Value {
    let dir = hires_dir(&app);
    let tiles: Vec<serde_json::Value> = hires()
        .map(|h| {
            h.coverage()
                .into_iter()
                .map(|(name, b)| {
                    let bytes = std::fs::metadata(dir.join(&name)).map(|m| m.len()).unwrap_or(0);
                    serde_json::json!({ "name": name, "area": b, "bytes": bytes })
                })
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({ "dir": dir, "tiles": tiles })
}

/// Delete downloaded high-resolution depth tiles: one by name, or all with "".
#[tauri::command]
fn hires_delete(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let dir = hires_dir(&app);
    if name.contains(['/', '\\']) || name.contains("..") {
        return Err("bad name".into());
    }
    *HIRES.write().unwrap() = None; // the files are open
    let mut err = None;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let p = e.path();
        let fname = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if p.extension().is_some_and(|x| x == "tiff") && (name.is_empty() || fname == name) {
            if let Err(e) = std::fs::remove_file(&p) {
                err = Some(e.to_string());
            }
        }
    }
    open_hires(&app);
    err.map_or(Ok(()), Err)
}

/// Height (m above NZVD2016) from the high-resolution depths at a place.
#[tauri::command]
fn hires_height(lon: f64, lat: f64) -> Option<f32> {
    hires()?.height_at(lon, lat)
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
    if p.parent().map(|d| d.canonicalize().ok()) != Some(dir.canonicalize().ok())
        || p.extension().is_none_or(|x| x != "gpkg")
    {
        return Err("not a downloaded depth area".into());
    }
    // the store has the file open: reopen without it first, then delete
    *CHARTS.write().unwrap() = None;
    if let Err(e) = std::fs::remove_file(&p) {
        open_charts_started(app);
        return Err(e.to_string());
    }
    open_charts_started(app);
    Ok(())
}

/// How fine the GSHHG coastline is drawn: 0 standard, 1 more, 2 most detail.
#[tauri::command]
fn chart_land_detail(level: u8) {
    chart::coast::LAND_DETAIL.store(level.min(2), std::sync::atomic::Ordering::Relaxed);
    if let Some(s) = charts() {
        s.clear_cache();
    }
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

/// Open a monitor window showing `view` (any number can be open at once): a second screen
/// for instruments now, radar or sounder views later. Returns the new window's label.
/// Async: on Windows a window made from a synchronous command deadlocks and stays blank.
#[tauri::command]
async fn monitor_open(
    app: tauri::AppHandle,
    view: String,
    label: Option<String>,
) -> Result<String, String> {
    if !view.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || view.is_empty() {
        return Err("unknown view".into());
    }
    // a window that was open before comes back under the same label, so it keeps its colours and place
    let wanted = label
        .and_then(|l| l.strip_prefix("monitor-").and_then(|n| n.parse::<u32>().ok()))
        .filter(|n| *n > 0 && app.get_webview_window(&format!("monitor-{n}")).is_none());
    let n = wanted.unwrap_or_else(|| {
        (1u32..)
            .find(|n| app.get_webview_window(&format!("monitor-{n}")).is_none())
            .unwrap_or(1)
    });
    let label = format!("monitor-{n}");
    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(format!("index.html?view={view}").into()),
    )
    .title(format!("TweedPlot monitor {n}"))
    .inner_size(1024.0, 640.0)
    .min_inner_size(320.0, 240.0)
    .build()
    .map(|_| label)
    .map_err(|e| e.to_string())
}

/// The monitor windows that are open, by label, in number order.
#[tauri::command]
fn monitor_list(app: tauri::AppHandle) -> Vec<String> {
    let mut labels: Vec<String> = app
        .webview_windows()
        .into_keys()
        .filter(|l| l.starts_with("monitor-"))
        .collect();
    labels.sort_by_key(|l| l[8..].parse::<u32>().unwrap_or(u32::MAX));
    labels
}

/// Bring a monitor window forward.
#[tauri::command]
async fn monitor_focus(app: tauri::AppHandle, label: String) -> Result<(), String> {
    let w = app
        .get_webview_window(&label)
        .filter(|_| label.starts_with("monitor-"))
        .ok_or("no such window")?;
    let _ = w.unminimize();
    w.set_focus().map_err(|e| e.to_string())
}

/// Close a monitor window.
#[tauri::command]
async fn monitor_close(app: tauri::AppHandle, label: String) -> Result<(), String> {
    let w = app
        .get_webview_window(&label)
        .filter(|_| label.starts_with("monitor-"))
        .ok_or("no such window")?;
    w.close().map_err(|e| e.to_string())
}

/// Charted vertical clearances (bridges, overhead cables) within `radius_m` of a point, nearest first.
#[tauri::command]
fn chart_clearances(lat: f64, lon: f64, radius_m: f64) -> Vec<chart::Clearance> {
    charts()
        .map(|s| s.clearances([lon, lat], radius_m.min(20_000.0)))
        .unwrap_or_default()
}

/// How far ahead (metres) along `cog` (°T), within `dist_m`, the water first gets
/// shallower than `depth` m, or the land starts, from the chart's depth areas (the
/// routing's raster, 20 m cells). None: clear. For the safety contour alarm.
#[tauri::command]
async fn chart_shallow_ahead(lat: f64, lon: f64, cog: f64, dist_m: f64, depth: f64) -> Option<f64> {
    let store = charts()?;
    tauri::async_runtime::spawn_blocking(move || {
        let dist = dist_m.clamp(50.0, 20_000.0);
        let k = lat.to_radians().cos().max(0.01);
        let (s, c) = cog.to_radians().sin_cos();
        let at = |m: f64| [lon + m * s / (111_320.0 * k), lat + m * c / 111_320.0];
        let end = at(dist);
        let pad = 300.0 / 111_320.0;
        let bbox = [lon.min(end[0]) - pad / k, lat.min(end[1]) - pad, lon.max(end[0]) + pad / k, lat.max(end[1]) + pad];
        let r = routing::Raster::from_chart(&store, bbox, depth, 20.0, 2_000_000);
        // start a little ahead: the boat may already be over the contour's edge
        let mut m = 40.0;
        while m <= dist {
            let p = at(m);
            if r.blocked(p[1], p[0]) {
                return Some(m);
            }
            m += 10.0;
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// Summary of the chart folder.
#[tauri::command]
fn chart_info() -> serde_json::Value {
    let ready = CHARTS_READY.load(std::sync::atomic::Ordering::Acquire);
    let error = CHART_ERROR.lock().unwrap().clone();
    match charts() {
        Some(s) => serde_json::json!({
            "ready": ready,
            "tables": s.tables.len(),
            "world_land": s.has_world_land(),
            "error": error,
            "layers": s.tables.iter().map(|t| serde_json::json!({
                "table": t.table, "kind": format!("{:?}", t.kind), "band": t.band, "rows": t.rows
            })).collect::<Vec<_>>(),
        }),
        None => serde_json::json!({ "ready": ready, "tables": 0, "error": error }),
    }
}

fn tile_response(path: &str) -> Response<Vec<u8>> {
    // high-resolution depth images: /hires/<z>/<x>/<y>.png
    if let Some(rest) = path.trim_start_matches('/').strip_prefix("hires/") {
        let p: Vec<u32> = rest.trim_end_matches(".png").split('/').filter_map(|v| v.parse().ok()).collect();
        let png = match (p.as_slice(), hires()) {
            ([z, x, y], Some(h)) => h.tile_png(*z as u8, *x, *y).map(|t| t.as_ref().clone()),
            _ => None,
        };
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::CONTENT_TYPE, "image/png")
            .body(png.unwrap_or_else(|| clear_png().to_vec()))
            .expect("valid response");
    }
    let parts: Vec<&str> = path
        .trim_start_matches('/')
        .trim_end_matches(".pbf")
        .split('/')
        .collect();
    let parsed = match parts.as_slice() {
        [z, x, y] => z
            .parse::<u8>()
            .ok()
            .zip(x.parse::<u32>().ok())
            .zip(y.parse::<u32>().ok()),
        _ => None,
    };
    // never cached by the WebView: a tile drawn by an older build or from older data
    // would otherwise stay on screen (the chart store keeps its own cache in memory)
    let builder = Response::builder()
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "no-store");
    match parsed {
        Some(((z, x), y)) => match charts() {
            Some(store) => match store.try_tile(z, x, y) {
                Some(t) => builder
                    .header(header::CONTENT_TYPE, "application/x-protobuf")
                    .body(t.as_ref().clone()),
                // drawing it failed (reported on the terminal): the chart asks again later
                None => builder
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Vec::new()),
            },
            None => builder
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Vec::new()),
        },
        None => builder.status(StatusCode::NOT_FOUND).body(Vec::new()),
    }
    .expect("valid response")
}

/// Chart tiles are drawn newest-request first. Duplicate requests for a tile share
/// one render while keeping every WebView response alive.
struct TileJobs {
    queued: std::collections::VecDeque<String>,
    queued_at: std::collections::HashMap<String, std::time::Instant>,
    responders: std::collections::HashMap<String, Vec<tauri::UriSchemeResponder>>,
}

static TILE_QUEUE: OnceLock<Arc<(Mutex<TileJobs>, std::sync::Condvar)>> = OnceLock::new();
static TILE_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static TILE_LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn log_chart_tile_line(message: &str) {
    log_chart_tile(message);
}

fn log_chart_tile(message: &str) {
    eprintln!("{message}");
    let Some(path) = TILE_LOG_PATH.get() else {
        return;
    };
    let _guard = TILE_LOG_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let reset = std::fs::metadata(path).is_ok_and(|m| m.len() > 1_000_000);
    let file = if reset {
        std::fs::File::create(path)
    } else {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    };
    let mut file = match file {
        Ok(file) => file,
        Err(e) => {
            eprintln!("chart tile log {}: {e}", path.display());
            return;
        }
    };
    let timestamp = chrono::Local::now().to_rfc3339();
    if let Err(e) = writeln!(file, "{timestamp} {message}") {
        eprintln!("chart tile log {}: {e}", path.display());
    }
}

fn queue_tile(path: String, responder: tauri::UriSchemeResponder) {
    let q = TILE_QUEUE.get_or_init(|| {
        let q = Arc::new((
            Mutex::new(TileJobs {
                queued: std::collections::VecDeque::new(),
                queued_at: std::collections::HashMap::new(),
                responders: std::collections::HashMap::new(),
            }),
            std::sync::Condvar::new(),
        ));
        let n = std::thread::available_parallelism()
            .map_or(2, |n| n.get().saturating_sub(1).clamp(1, 3));
        log_chart_tile(&format!("chart tiles: starting {n} workers"));
        for i in 0..n {
            let q = q.clone();
            std::thread::Builder::new()
                .name(format!("tiles-{i}"))
                .spawn(move || loop {
                    let (job, enqueued_at, queued_at_start) = {
                        let mut jobs = q.0.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(path) = jobs.queued.pop_back() {
                                let enqueued_at = jobs
                                    .queued_at
                                    .remove(&path)
                                    .expect("queued tile has enqueue timestamp");
                                break (path, enqueued_at, jobs.queued.len());
                            }
                            jobs = q.1.wait(jobs).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let wait_ms = enqueued_at.elapsed().as_millis();
                    let started_at = std::time::Instant::now();
                    let r = std::panic::catch_unwind(|| tile_response(&job)).unwrap_or_else(|_| {
                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Vec::new())
                            .expect("valid response")
                    });
                    let render_ms = started_at.elapsed().as_millis();
                    let (responders, queued) = {
                        let mut jobs = q.0.lock().unwrap_or_else(|e| e.into_inner());
                        (jobs.responders.remove(&job).unwrap_or_default(), jobs.queued.len())
                    };
                    let bytes = r.body().len();
                    let zoom = job
                        .trim_start_matches('/')
                        .split('/')
                        .next()
                        .and_then(|z| z.parse::<u8>().ok());
                    let tiny =
                        zoom.is_some_and(|z| z >= 9) && r.status().is_success() && bytes <= 48;
                    if !r.status().is_success()
                        || wait_ms >= 1000
                        || render_ms >= 1500
                        || tiny
                        || std::env::var_os("TWEEDPLOT_TILE_LOG").is_some()
                    {
                        log_chart_tile(&format!(
                            "chart tile {job}: wait={wait_ms}ms render={render_ms}ms status={} bytes={bytes} tiny={tiny} queued={queued_at_start}->{queued} responses={}",
                            r.status().as_u16(),
                            responders.len()
                        ));
                    }
                    for responder in responders {
                        responder.respond(r.clone());
                    }
                })
                .expect("tile thread");
        }
        q
    });
    let mut jobs = q.0.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(responders) = jobs.responders.get_mut(&path) {
        responders.push(responder);
        return;
    }
    jobs.queued.push_back(path.clone());
    jobs.queued_at
        .insert(path.clone(), std::time::Instant::now());
    jobs.responders.insert(path, vec![responder]);
    drop(jobs);
    q.1.notify_one();
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // the main window is the app: closing it closes any monitor windows too
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            chart_info,
            chart_feature,
            chart_clearances,
            chart_shallow_ahead,
            monitor_open,
        monitor_list,
        monitor_focus,
        monitor_close,
            data_boundaries,
            data_mpi,
            hires_plan,
            hires_download,
            hires_status,
            hires_delete,
            hires_height,
            rules_at,
            rule_info,
            tides_info,
            tides_stations,
            tides_curve,
            tides_phase,
            tidal_streams,
            tides_download,
            tides_cancel,
            grib_files,
            grib_load,
            grib_unload,
            grib_loaded,
            grib_wind_grid,
            grib_grid,
            grib_compare,
            grib_meteogram,
            grib_verify_times,
            grib_verify_scores,
            grib_verify_track,
            grib_verify_grid,
            grib_point,
            grib_along,
            radar_sim::radar_sim_start,
            radar_sim::radar_sim_stop,
            radar_sim::radar_sim_stats,
            settings::settings_get,
            settings::settings_save,
            nmea::nmea_ports,
            nmea::nmea_sources_get,
            nmea::nmea_sources_set,
            polar_files,
            polar_import,
            polar_open_folder,
            polar_speed,
            route_compute,
            sk_status,
            stations_cmd::stations_latest,
            stations_cmd::station_history,
            live::ais_targets,
            live::ais_online,
            live::ais_status,
            live::ais_set_own,
            live::ais_tracks,
            live::replay_logs,
            live::replay_start,
            live::replay_control,
            live::replay_stop,
            lib_info,
            marks_list,
            mark_save,
            mark_delete,
            barriers_list,
            barrier_save,
            barrier_delete,
            routes_list,
            route_save,
            route_delete,
            routings_list,
            routing_get,
            routing_save,
            routing_delete,
            gpx_export,
            gpx_write,
            gpx_import,
            marks_delete,
            symbols_list,
            library_trash,
            trips_list,
            trip_save,
            trip_end,
            trips_delete,
            trip_contents,
            library_restore,
            library_purge,
            library_imports,
            library_undo_import,
            library_export,
            symbol_set,
            marks_in,
            library_pick_file,
            library_import_path,
            library_current,
            library_pick_db,
            library_use,
            library_save_copy,
            library_merge,
            library_backups,
            library_use_backup,
            marks_update,
            tracks_export,
            job_cancel,
            activity_log,
            activity_log_read,
            rec_status,
            rec_start,
            rec_stop,
            tracks_list,
            track_get,
            track_update,
            track_delete,
            tracks_delete,
            track_export,
            yb_open,
            yb_refresh,
            yb_races,
            yb_history,
            gribdl_models,
            gribdl_estimate,
            gribdl_start,
            gribdl_cancel,
            open_url,
            grib_combine,
            data_status,
            chart_sets_list,
            chart_set_use,
            data_etopo_plan,
            data_gshhg,
            data_etopo,
            data_etopo_delete,
            data_cancel,
            chart_land_detail,
            s63_cmd::s63_status,
            s63_cmd::s63_activate,
            s63_cmd::s63_deactivate,
            s63_cmd::s63_install_permits,
            s63_cmd::s63_remove_permits,
            s63_cmd::s63_install_key,
            s63_cmd::s63_import_exchange_set,
            s63_cmd::s63_remove_cells
        ])
        .register_asynchronous_uri_scheme_protocol("chart", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            queue_tile(path, responder);
        })
        .setup(|app| {
            match app.path().app_data_dir() {
                Ok(dir) => match std::fs::create_dir_all(&dir) {
                    Ok(()) => {
                        let path = dir.join("chart-tiles.log");
                        if TILE_LOG_PATH.set(path.clone()).is_err() {
                            eprintln!("chart tile log path was already initialized");
                        } else {
                            eprintln!("chart tile diagnostics: {}", path.display());
                            log_chart_tile("chart tile diagnostics started");
                            let _ = chart::LOGGER.set(log_chart_tile_line);
                        }
                    }
                    Err(e) => eprintln!("chart tile log directory {}: {e}", dir.display()),
                },
                Err(e) => eprintln!("chart tile log directory: {e}"),
            }
            let dd = download_dir(app.handle());
            let _ = std::fs::create_dir_all(&dd);
            let _ = DOWNLOAD_DIR.set(dd);
            let td = tracking_dir(app.handle());
            let _ = std::fs::create_dir_all(&td);
            let _ = TRACKING_DIR.set(td);
            let tdir = tide_dir(app.handle());
            let _ = std::fs::create_dir_all(&tdir);
            let _ = TIDE_DIR.set(tdir);
            let start_file = library_start_file(app.handle());
            match library::Library::open_file(&start_file) {
                Ok(l) => {
                    *LIBRARY.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(l));
                    // a library kept as JSON files (before Oct 2026) moves into the database
                    // once, in the background, with progress; the recorder carries on after
                    let h = app.handle().clone();
                    let convert = lib().is_ok_and(|l| l.needs_conversion());
                    std::thread::spawn(move || {
                        if convert {
                            let mut last = std::time::Instant::now();
                            let r = lib().and_then(|l| {
                                l.convert_json(|step, done, total| {
                                    if last.elapsed().as_millis() > 300 || done == total {
                                        last = std::time::Instant::now();
                                        let _ = h.emit("library-converting", serde_json::json!({ "step": step, "done": done, "total": total }));
                                    }
                                })
                                .map_err(es)
                            });
                            let _ = h.emit("library-converted", serde_json::json!({ "ok": r.is_ok(), "text": r.unwrap_or_else(|e| e) }));
                            library_changed(&h);
                        }
                        rec_resume();
                        // the library follows the app's one TimeZero symbol table
                        match symbols_sync(&h) {
                            Ok(n) if n > 0 => library_changed(&h),
                            Ok(_) => {}
                            Err(e) => eprintln!("symbol table: {e}"),
                        }
                        // the day's backup of the library (a copy; nothing waits for it)
                        if let Ok(l) = lib() {
                            if let Err(e) = l.backup_if_due() {
                                eprintln!("library backup: {e}");
                            }
                        }
                    });
                }
                Err(e) => eprintln!("library: {e}"),
            }
            // Charts: open in the background (the first run unzips the LINZ downloads).
            let _ = std::fs::create_dir_all(coast_dir(app.handle()));
            let _ = std::fs::create_dir_all(bathy_dir(app.handle()));
            // the S-63 activation needs this computer's fingerprint (a second or so)
            let h = app.handle().clone();
            std::thread::spawn(move || {
                s63_cmd::load(&h);
                open_hires(&h);
                open_charts(h);
            });

            // Signal K, AIS, replay and native NMEA sources
            live::start(app.handle());
            nmea::start(app.handle());
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
