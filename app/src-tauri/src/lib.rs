//! racenav desktop app: wires the Rust crates to the MapLibre front end.
//!
//! Events sent to the front end:
//! - `sk-status`   `{ connected, detail }`
//! - `vessel`      own-vessel state (degrees true, knots, metres)
//! - `chart-ready` `{ ready, tables, dir }` once the chart folder has been opened
//!
//! The `chart` URI scheme serves vector tiles: `chart://localhost/{z}/{x}/{y}.pbf`
//! (`http://chart.localhost/...` on Windows).

mod chart_texts;
mod depthpacks;
mod grib_cmd;
mod imagery_cmd;
mod library_cmd;
mod live;
mod nmea;
mod pbathy;
mod plugin;
mod publications;
mod radar_sim;
mod routing_cmd;
mod s63_cmd;
mod screenshot;
mod settings;
mod stations_cmd;
mod webmap;
use grib_cmd::*;
use library_cmd::*;
use routing_cmd::*;
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
        let (old, new) = (PathBuf::from(&base).join("nz.tweedsmarine.racenav"), PathBuf::from(&base).join("nz.tweedsmarine.tweedplot"));
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
    app.path().app_data_dir().map(|p| p.join("hires")).unwrap_or_else(|_| PathBuf::from("hires"))
}

/// The NIWA 2016 bathymetry grid (250 m), imported from the zip NIWA supplies.
static NZBATHY: std::sync::RwLock<Option<Arc<chart::nzbathy::NzBathy>>> = std::sync::RwLock::new(None);

fn nzbathy() -> Option<Arc<chart::nzbathy::NzBathy>> {
    NZBATHY.read().unwrap().clone()
}

fn open_nzbathy(app: &tauri::AppHandle) {
    *NZBATHY.write().unwrap() = chart::nzbathy::NzBathy::open(&nzbathy_dir(app)).map(Arc::new);
}

/// NIWA bathymetry store: `RACENAV_NZBATHY_DIR`, else `<dev data>\nzbathy`, else `<app data>\nzbathy`.
fn nzbathy_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_NZBATHY_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("nzbathy");
    }
    app.path().app_data_dir().map(|p| p.join("nzbathy")).unwrap_or_else(|_| PathBuf::from("nzbathy"))
}

/// Ask for the NIWA bathymetry zip (or the GeoTIFF inside it).
#[tauri::command]
async fn nzbathy_pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("Import NIWA bathymetry grid")
        .add_filter("NIWA bathymetry (zip or GeoTIFF)", &["zip", "tif", "tiff"])
        .add_filter("All files", &["*"])
        .blocking_pick_file();
    let Some(p) = picked else { return Ok(None) };
    Ok(Some(p.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned()))
}

/// Convert the NIWA grid into the store and open it.
#[tauri::command]
async fn nzbathy_import(app: tauri::AppHandle, path: String) -> Result<serde_json::Value, String> {
    data_guard()?;
    let dir = nzbathy_dir(&app);
    let h = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        *NZBATHY.write().unwrap() = None; // the old store is replaced
        let progress = |stage: &str, done: u64, total: u64| {
            let _ = h.emit("data-progress", serde_json::json!({ "stage": stage, "done": done, "total": total }));
        };
        chart::nzbathy::import(std::path::Path::new(&path), &dir, &progress, &DATA_CANCEL).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());
    DATA_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
    open_nzbathy(&app);
    let s = r??;
    Ok(serde_json::json!({ "area": s.area, "bytes": s.bytes }))
}

/// Whether the NIWA grid is imported: its folder, size and area.
#[tauri::command]
fn nzbathy_status(app: tauri::AppHandle) -> serde_json::Value {
    let dir = nzbathy_dir(&app);
    match nzbathy() {
        Some(n) => serde_json::json!({ "dir": dir, "have": true, "bytes": n.bytes(), "area": n.area() }),
        None => serde_json::json!({ "dir": dir, "have": false }),
    }
}

#[tauri::command]
fn nzbathy_delete(app: tauri::AppHandle) -> Result<(), String> {
    *NZBATHY.write().unwrap() = None; // the file is open
    let p = nzbathy_dir(&app).join(chart::nzbathy::FILE);
    let r = if p.exists() { std::fs::remove_file(&p).map_err(|e| e.to_string()) } else { Ok(()) };
    open_nzbathy(&app);
    r
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

/// Signal K URL: `RACENAV_SK_URL` environment variable, else the local server.
pub(crate) fn sk_url() -> String {
    std::env::var("RACENAV_SK_URL").unwrap_or_else(|_| signalk::DEFAULT_URL.to_string())
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
        let _ = std::fs::write(l.dir().join(REC_FILE), serde_json::json!({ "on": on, "track": id }).to_string());
    }
}

fn new_log_track() -> library::Track {
    library::Track { id: String::new(), name: chrono::Local::now().format("Log %d %b %Y %H:%M").to_string(), source: "log".into(), ..Default::default() }
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

/// Carry on recording if it was on when the app closed: the same track if its last fix
/// is under 2 h old, else a new one.
fn rec_resume() {
    let st = lib().ok().and_then(|l| std::fs::read_to_string(l.dir().join(REC_FILE)).ok());
    let Some(v) = st.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) else { return };
    if v["on"].as_bool() != Some(true) {
        return;
    }
    let id = v["track"].as_str().unwrap_or("").to_string();
    let recent = lib().ok().and_then(|l| l.track(&id).ok()).and_then(|t| t.stats.end).is_some_and(|e| chrono_now() - e < 2 * 3600);
    if let Err(e) = rec_begin(recent.then_some(id.as_str())) {
        eprintln!("recorder: {e}");
    }
}

/// Offer a Signal K fix to the recorder; saves every 30 s.
/// The track being recorded now, if any.
pub(crate) fn rec_track_id() -> Option<String> {
    REC.lock().unwrap().as_ref().map(|r| r.track.id.clone())
}

pub(crate) fn rec_vessel(app: &tauri::AppHandle, v: &signalk::VesselState) {
    // own soundings, live (when on): every depth update, not only the track's fixes
    pbathy::live_vessel(app, v);
    let (Some(lat), Some(lon)) = (v.lat, v.lon) else {
        return;
    };
    let now = chrono_now();
    let t = v.datetime.as_deref().and_then(library::tracks::parse_iso).unwrap_or(now);
    let (kept, due, stats) = {
        let mut g = REC.lock().unwrap();
        let Some(r) = g.as_mut() else { return };
        // the track says what its depths are measured below: the transducer when the source
        // gives that (with the transducer's depth from Settings › Vessel), else the surface
        if r.track.depth_ref.is_empty() {
            if v.depth.is_some() {
                r.track.depth_ref = "transducer".into();
                r.track.depth_offset = crate::settings::settings_get(app.clone()).ok().and_then(|s| s.vessel.transducer_depth_m);
            } else if v.depth_surface.is_some() {
                r.track.depth_ref = "surface".into();
            }
        }
        let depth = match r.track.depth_ref.as_str() {
            "transducer" => v.depth,
            "surface" => v.depth_surface,
            _ => None,
        };
        let kept = r.push(library::TrackPoint { t: Some(t), lat, lon, sog: v.sog, cog: v.cog, depth, temp: v.water_temp });
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
    let segs: Vec<Vec<serde_json::Value>> =
        t.segments.iter().map(|s| s.iter().map(|p| serde_json::json!([p.lon, p.lat, p.t, p.sog, p.cog, p.depth, p.temp])).collect()).collect();
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
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("tides");
    }
    app.path().app_data_dir().map(|p| p.join("tides")).unwrap_or_else(|_| PathBuf::from("tides"))
}

pub(crate) fn tide_db() -> Arc<tides::TideDb> {
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
        "gauges": db.harmonics.len(),
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
                tides::Kind::Gauge => ("gauge", None),
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
    let harm = db.harmonics_of(&id).is_some();
    let method = match &s.kind {
        tides::Kind::Daily if harm => "LINZ daily table; other years from LINZ's harmonic constituents".to_string(),
        tides::Kind::Daily => "LINZ daily table".to_string(),
        tides::Kind::Gauge => "LINZ harmonic constituents for this gauge".to_string(),
        _ if harm => "LINZ harmonic constituents for this place".to_string(),
        tides::Kind::Secondary { reference, hw_min, lw_min, ratio } => format!(
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
    let dev = dev_root();
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
        with_race(&key, |r| Ok(serde_json::json!({ "key": key, "setup": setup, "fixes": r.fixes, "meta": r.meta, "error": error })))
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
            let m = std::fs::metadata(e.path().join("fixes.json")).or_else(|_| std::fs::metadata(e.path()));
            let t = m.ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            let title =
                std::fs::read_to_string(e.path().join("setup.json")).ok().and_then(|s| tracking::parse_setup(&s).ok()).map(|s| s.title).unwrap_or_default();
            (t, serde_json::json!({ "key": key, "title": title, "modified": t }))
        })
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.0));
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
    app.path().app_data_dir().map(|p| p.join("charts")).unwrap_or_else(|_| PathBuf::from("charts"))
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
    app.path().app_data_dir().map(|p| p.join("coastline")).unwrap_or_else(|_| PathBuf::from("coastline"))
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
    app.path().app_data_dir().map(|p| p.join("bathy")).unwrap_or_else(|_| PathBuf::from("bathy"))
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
    app.path().app_data_dir().map(|p| p.join("marine")).unwrap_or_else(|_| PathBuf::from("marine"))
}

/// The bundled IHO S-101 catalogues: the folder holding `s101-pc` and `s101-fc`.
fn s101_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let candidates = [
        app.path().resource_dir().ok().map(|d| d.join("resources")),
        app.path().resource_dir().ok(),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")),
    ];
    candidates.into_iter().flatten().find(|d| d.join("s101-pc").is_dir() && d.join("s101-fc").is_dir())
}

const S101_FC: &str = "101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml";

/// Portrayal context values set by the page (safety contour, ...), kept for a re-opened store.
static S52_CONTEXT: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Turn on the IHO portrayal in a newly opened store.
fn s52_attach(app: &tauri::AppHandle, store: &chart::ChartStore) {
    let Some(dir) = s101_dir(app) else {
        log_chart_tile("charts: the S-101 catalogues are not installed: no IHO symbols");
        return;
    };
    match chart::s52::S52::open(&dir.join("s101-pc"), &dir.join("s101-fc").join(S101_FC)) {
        Ok(s) => {
            store.set_s52(Some(s));
            for (k, v) in S52_CONTEXT.lock().unwrap().iter() {
                store.set_s52_context(k, v);
            }
        }
        Err(e) => log_chart_tile(&format!("charts: IHO portrayal not available: {e}")),
    }
}

/// Portray the chart's features in the background; `chart-s52` when done (the page then
/// asks for the tiles again).
fn s52_warm(app: tauri::AppHandle, store: Arc<chart::ChartStore>) {
    if !store.s52_on() {
        return;
    }
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let n = store.s52_warm(&|_, _| {});
        if n > 0 {
            log_chart_tile(&format!("charts: IHO portrayal of {n} tables in {:.1} s", t0.elapsed().as_secs_f64()));
            let _ = app.emit("chart-s52", serde_json::json!({ "tables": n }));
        }
    });
}

/// Set a portrayal context parameter (SafetyContour, SafetyDepth, ...): the features are
/// portrayed again when it changes anything.
#[tauri::command]
fn chart_s52_context(app: tauri::AppHandle, id: String, value: String) {
    {
        let mut c = S52_CONTEXT.lock().unwrap();
        c.retain(|(k, _)| *k != id);
        c.push((id.clone(), value.clone()));
    }
    let store = CHARTS.read().unwrap().clone();
    if let Some(store) = store {
        if store.set_s52_context(&id, &value) {
            s52_warm(app, store);
        }
    }
}

/// The catalogue's symbols for a palette ("Day", "Dusk", "Night"): id, standalone SVG and
/// view box in mm (the symbol's pivot is 0,0).
#[tauri::command]
async fn s52_symbols(app: tauri::AppHandle, palette: String) -> Result<serde_json::Value, String> {
    s52_cached(app, format!("symbols:{palette}"), move |cat| {
        serde_json::Value::Array(
            cat.symbols.keys().filter_map(|id| cat.symbol_svg(id, &palette).map(|(svg, vb)| serde_json::json!({ "id": id, "svg": svg, "vb": vb }))).collect(),
        )
    })
    .await
}

/// The portrayal catalogue's answers, worked out once per run (the catalogue is read once).
static S52_CACHE: std::sync::LazyLock<Mutex<(Option<Arc<portrayal::Catalogue>>, std::collections::HashMap<String, Arc<serde_json::Value>>)>> =
    std::sync::LazyLock::new(|| Mutex::new((None, std::collections::HashMap::new())));

async fn s52_cached(
    app: tauri::AppHandle,
    key: String,
    make: impl FnOnce(&portrayal::Catalogue) -> serde_json::Value + Send + 'static,
) -> Result<serde_json::Value, String> {
    if let Some(v) = S52_CACHE.lock().unwrap().1.get(&key) {
        return Ok((**v).clone());
    }
    let dir = s101_dir(&app).ok_or("the S-101 portrayal catalogue is not installed")?;
    tauri::async_runtime::spawn_blocking(move || {
        let cached = S52_CACHE.lock().unwrap().0.clone();
        let cat = match cached {
            Some(c) => c,
            None => {
                let c = Arc::new(portrayal::Catalogue::open(&dir.join("s101-pc")).map_err(|e| e.to_string())?);
                S52_CACHE.lock().unwrap().0 = Some(c.clone());
                c
            }
        };
        let v = Arc::new(make(&cat));
        S52_CACHE.lock().unwrap().1.insert(key, v.clone());
        Ok((*v).clone())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The catalogue's colours (token → #rrggbb) for a palette, its line styles (with how tall
/// each is drawn, mm) and its area fills, for drawing the IHO lines and areas.
#[tauri::command]
async fn s52_styles(app: tauri::AppHandle, palette: String) -> Result<serde_json::Value, String> {
    s52_cached(app, format!("styles:{palette}"), move |cat| {
        let colours: serde_json::Map<String, serde_json::Value> = cat
            .palettes
            .get(&palette)
            .map(|p| p.keys().filter_map(|k| Some((k.clone(), serde_json::Value::String(cat.colour(&palette, k)?)))).collect())
            .unwrap_or_default();
        let lines: serde_json::Map<String, serde_json::Value> = cat
            .line_styles
            .keys()
            .filter_map(|n| {
                let s = cat.line_style(n)?;
                let h = cat.line_height(&s);
                let mut v = serde_json::to_value(&s).ok()?;
                v["height"] = serde_json::json!(h);
                Some((n.clone(), v))
            })
            .collect();
        let fills: serde_json::Map<String, serde_json::Value> =
            cat.area_fills.keys().filter_map(|n| Some((n.clone(), serde_json::to_value(cat.area_fill(n)?).ok()?))).collect();
        serde_json::json!({ "colours": colours, "lines": lines, "fills": fills })
    })
    .await
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
pub(crate) fn open_charts(handle: tauri::AppHandle) {
    open_charts_started(handle);
}

fn open_charts_started(handle: tauri::AppHandle) {
    CHARTS_READY.store(false, std::sync::atomic::Ordering::Release);
    *CHART_ERROR.lock().unwrap() = None;
    let dir = chart_dir(&handle);
    let sets_off = chart_sets_off(&handle);
    // the LINZ open data charts (TweedPlot's own chart) can be left out like an ENC set, so only
    // official ENCs are drawn
    let linz_off = sets_off.contains(&dir);
    let mut dirs = vec![coast_dir(&handle), bathy_dir(&handle), marine_dir(&handle)];
    if !linz_off {
        dirs.insert(0, dir.clone());
    }
    if let Some(w) = world_depth_dir(&handle) {
        dirs.push(w);
    }
    // S-57 ENC cells (and the IHO S-63/S-64 test data): enc and s63 in the data folders
    let enc_dirs = s63_cmd::enc_dirs(&handle);
    // the first run with a GSHHG download builds the world coastline file (a minute or two)
    let _ = handle.emit("chart-status", serde_json::json!({ "detail": "preparing charts: finding chart files" }));
    log_chart_tile("charts: opening started");
    if linz_off {
        log_chart_tile("charts: LINZ open data charts not in use");
    }
    if !sets_off.is_empty() {
        log_chart_tile(&format!("charts: {} chart set(s) not in use", sets_off.len()));
    }
    std::thread::spawn(move || {
        // convert new or changed ENC cells first, saying how far it has got (a big
        // exchange set, such as all of NOAA's, takes a while the first time)
        let t_enc = std::time::Instant::now();
        chart::enc::clear_notes();
        let mut prepared_enc_gpkgs = Vec::new();
        for d in enc_dirs.iter().filter(|d| d.ends_with("enc") || d.ends_with("s63")) {
            let h = handle.clone();
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            prepared_enc_gpkgs.extend(chart::enc::ensure_gpkgs_sets(d, &sets_off, &mut |done, todo, name| {
                if last.elapsed().as_millis() >= 500 || done == todo || todo == 0 {
                    last = std::time::Instant::now();
                    let detail = if todo == 0 { format!("converting ENC cells: {name}") } else { format!("converting ENC cells: {done} of {todo} {name}") };
                    let _ = h.emit("chart-status", serde_json::json!({ "detail": detail }));
                }
            }));
        }
        log_chart_tile(&format!("charts: ENC cells checked in {:.1} s", t_enc.elapsed().as_secs_f64()));
        // encrypted charts: the S-63 Charts plugin decrypts what is new and opens them
        let mut remote = None;
        if !sets_off.contains(&s63_cmd::plugin_set_path(&handle)) {
            if let Some(p) = plugin::get(&handle) {
                let t = std::time::Instant::now();
                match p.client.call("open", serde_json::json!({}), std::time::Duration::from_secs(3600)) {
                    Ok(v) => match serde_json::from_value::<plugin_api::protocol::Opened>(v) {
                        Ok(o) => {
                            PLUGIN_CELLS.store(o.cells, std::sync::atomic::Ordering::Relaxed);
                            log_chart_tile(&format!("charts: {} encrypted cells opened by the plugin in {:.1} s", o.cells, t.elapsed().as_secs_f64()));
                            remote = Some((p.client.clone(), o.tables));
                        }
                        Err(e) => log_chart_tile(&format!("charts: the plugin's answer can't be read: {e}")),
                    },
                    Err(e) => log_chart_tile(&format!("charts: the plugin couldn't open the encrypted charts: {e}")),
                }
            }
        }
        open_store(handle, dirs, dir, prepared_enc_gpkgs, remote)
    });
}

/// How many encrypted cells the plugin opened last (for the chart sets list).
static PLUGIN_CELLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

type PluginTables = (Arc<plugin_api::PluginClient>, Vec<plugin_api::protocol::TableMeta>);

fn open_store(handle: tauri::AppHandle, dirs: Vec<PathBuf>, dir: PathBuf, prepared_enc_gpkgs: Vec<PathBuf>, remote: Option<PluginTables>) {
    let h = handle.clone();
    let t0 = std::time::Instant::now();
    let opened = chart::ChartStore::open_dirs_with_extra_gpkgs(&dirs, &prepared_enc_gpkgs, &mut |k, n| {
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
        Ok(mut store) => {
            if let Some((client, tables)) = remote {
                let n = store.add_remote(Arc::new(plugin::Remote(client)), &tables);
                log_chart_tile(&format!("charts: {n} tables from the S-63 Charts plugin"));
            }
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
            log_chart_tile(&format!("charts: render indexes prepared in {:.1} s", t0.elapsed().as_secs_f64()));
            let n = store.tables.len();
            let world = store.has_world_land();
            s52_attach(&handle, &store);
            let store = Arc::new(store);
            *CHARTS.write().unwrap() = Some(store.clone());
            CHARTS_READY.store(true, std::sync::atomic::Ordering::Release);
            let _ = handle.emit("chart-ready", serde_json::json!({ "ready": true, "tables": n, "dir": dir, "world_land": world, "s52": store.s52_on() }));
            s52_warm(handle.clone(), store);
        }
        Err(e) => {
            *CHART_ERROR.lock().unwrap() = Some(e.to_string());
            CHARTS_READY.store(true, std::sync::atomic::Ordering::Release);
            let _ = handle.emit("chart-ready", serde_json::json!({ "ready": true, "tables": 0, "dir": dir, "error": e.to_string() }));
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
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut v = Vec::new();
        // TweedPlot's own chart from the LINZ open data (GeoPackages in the charts folder), as a set
        // of its own, so it can be left out and only official ENCs drawn
        let cd = chart_dir(&a);
        let files = std::fs::read_dir(&cd)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| {
                        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                        (n.ends_with(".zip") && !n.starts_with("gshhg")) || n.ends_with(".gpkg")
                    })
                    .count()
            })
            .unwrap_or(0);
        if files > 0 {
            v.push(serde_json::json!({ "path": cd, "name": "LINZ open data charts (TweedPlot's chart, not an official ENC)", "cells": files, "unit": "file",
                "producers": [["NZ", files]], "folder": cd, "in_use": !off.contains(&cd), "linz": true }));
        }
        for d in dirs.iter().filter(|d| d.is_dir()) {
            for s in chart::enc::chart_sets(d) {
                let on = !off.contains(&s.path);
                v.push(serde_json::json!({ "path": s.path, "name": s.name, "cells": s.cells, "producers": s.producers, "folder": d, "in_use": on }));
            }
        }
        // the encrypted charts the plugin holds, as one set
        if plugin::installed(&a) {
            let p = s63_cmd::plugin_set_path(&a);
            let n = PLUGIN_CELLS.load(std::sync::atomic::Ordering::Relaxed);
            v.push(serde_json::json!({ "path": p, "name": format!("Encrypted charts ({})", plugin::NAME), "cells": n, "producers": [], "folder": s63_cmd::enc_home(&a), "in_use": !off.contains(&p) }));
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
        .map(|s| s.tables.iter().filter(|t| matches!(t.kind, chart::Kind::DepthArea | chart::Kind::Land | chart::Kind::Contour)).count())
        .unwrap_or(0);
    let mdir = marine_dir(&app);
    let boundary_zips =
        std::fs::read_dir(mdir.join("boundaries")).map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "zip")).count()).unwrap_or(0);
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
        "linz_off": chart_sets_off(&app).contains(&chart_dir(&app)),
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
            let _ = h.emit(
                "data-progress",
                serde_json::json!({ "stage": format!("Maritime boundaries (LINZ): file {} of {total}", (done + 1).min(total)), "done": 0, "total": 0 }),
            );
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
        let want: Vec<(String, String)> =
            names.into_iter().filter_map(|n| av.get(&n).map(|p| (n, p.clone()))).filter(|(n, _)| !dir.join(format!("{n}.tiff")).exists()).collect();
        if want.len() > 60 {
            return Err(format!("{} tiles: zoom in to a smaller area (60 at a time)", want.len()));
        }
        let progress = |i: usize, n: usize, bytes: u64| {
            let _ = h.emit(
                "data-progress",
                serde_json::json!({ "stage": format!("High-resolution depths (LINZ): tile {} of {n}", (i + 1).min(n)), "done": bytes, "total": 0 }),
            );
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
    if p.parent().map(|d| d.canonicalize().ok()) != Some(dir.canonicalize().ok()) || p.extension().is_none_or(|x| x != "gpkg") {
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

#[cfg(windows)]
fn powershell_text(script: &str) -> String {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn powershell_text(_script: &str) -> String {
    String::new()
}

/// The folders the user puts data in, as (name, path, what goes there): Settings > Diagnostics.
fn data_folders(app: &tauri::AppHandle) -> Vec<(&'static str, String, &'static str)> {
    let dir = |p: PathBuf| p.to_string_lossy().to_string();
    let enc = s63_cmd::enc_home(app);
    vec![
        ("S-63 / ENC charts", dir(enc), "Exchange sets and permits (PERMIT.TXT) for S-63 charts; or use Chart › Encrypted charts (S-63)"),
        ("LINZ charts (NZ)", dir(chart_dir(app)), "LINZ chart data for New Zealand: the GeoPackage zip files as downloaded"),
        ("High-resolution depths", dir(hires_dir(app)), "LINZ coastal elevation GeoTIFFs"),
        ("Survey depths", dir(depthpacks::dir(app)), "TweedPlot depth packages (.tpdepth) from LINZ's multibeam and lidar surveys"),
        ("NIWA bathymetry", dir(nzbathy_dir(app)), "NIWA 2016 250 m grid"),
        ("Coastline", dir(coast_dir(app)), "GSHHG coastline zip or GSHHS_shp folder"),
        ("Bathymetry", dir(bathy_dir(app)), "Downloaded ETOPO areas"),
        ("Marine boundaries", dir(marine_dir(app)), "Maritime boundaries and fishing rules"),
        ("GRIB weather files", dir(download_dir(app)), "Downloaded and manually added GRIB files"),
        ("Polars", dir(user_polar_dir(app)), "Your own polar files"),
        ("Tides", dir(tide_dir(app)), "Tide station data"),
        ("Library", dir(library_dir(app)), "Marks, routes, tracks, trips, the catch log, routings and barriers (library.gpkg), exports and backups"),
        ("Race tracking", dir(tracking_dir(app)), "YB race tracking data, one folder per race"),
        ("Logs", dir(activity_log_path(app).parent().map(|p| p.to_path_buf()).unwrap_or_default()), "Activity log"),
    ]
}

/// Version, operating system, hardware and the folders the user puts data in, for
/// Settings > Diagnostics and Copy for support. Async: the hardware query takes a moment.
#[tauri::command]
async fn system_info(app: tauri::AppHandle) -> serde_json::Value {
    let hw = tauri::async_runtime::spawn_blocking(|| {
        powershell_text(
            "$o=Get-CimInstance Win32_OperatingSystem;$c=Get-CimInstance Win32_Processor|Select -First 1;\
             $g=(Get-CimInstance Win32_VideoController|%{$_.Name}) -join '; ';\
             $m=Get-CimInstance Win32_ComputerSystem;\
             @{os=\"$($o.Caption) $($o.Version)\";cpu=$c.Name;ram=[math]::Round($m.TotalPhysicalMemory/1GB,1);gpu=$g;model=\"$($m.Manufacturer) $($m.Model)\"}|ConvertTo-Json -Compress",
        )
    })
    .await
    .unwrap_or_default();
    let hw: serde_json::Value = serde_json::from_str(&hw).unwrap_or(serde_json::Value::Null);
    let folders = serde_json::Value::Array(
        data_folders(&app).into_iter().map(|(name, path, note)| serde_json::json!({ "name": name, "path": path, "note": note })).collect(),
    );
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "arch": std::env::consts::ARCH,
        "os_family": std::env::consts::OS,
        "cpu_threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "webview": tauri::webview_version().unwrap_or_default(),
        "hardware": hw,
        "folders": folders,
    })
}

/// Open one of the data folders in the file manager, creating it first if it doesn't exist
/// yet. Only the folders of [`data_folders`]: the page can't open or create anything else.
#[tauri::command]
fn open_folder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    if !data_folders(&app).iter().any(|(_, p, _)| *p == path) {
        return Err("not one of TweedPlot's data folders".into());
    }
    let p = PathBuf::from(&path);
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let prog = "explorer";
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let prog = "xdg-open";
    std::process::Command::new(prog).arg(&p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The compilation scale of the chart shown at a place (the largest-scale chart with a depth
/// area there), for the overscale indication: `{ band, scale, enc }`. LINZ's open data gives
/// each band's scale range (the largest scale is taken); ENC bands use the usual largest
/// scale of their navigational purpose.
#[tauri::command]
async fn chart_scale_at(lon: f64, lat: f64) -> Option<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let (band, table, ..) = charts()?.depth_area_at(lon, lat)?;
        let enc = table.to_ascii_lowercase().contains("_enc_b");
        let scale: u32 = if enc {
            match band {
                0..=2 => 1_500_000,
                3 => 350_000,
                4 => 90_000,
                5 => 22_000,
                _ => 8_000,
            }
        } else {
            match band {
                5 => 14_000,
                4 => 122_000,
                3 => 190_000,
                2 => 1_350_000,
                _ => 11_500_000,
            }
        };
        Some(serde_json::json!({ "band": band, "scale": scale, "enc": enc }))
    })
    .await
    .ok()
    .flatten()
}

/// The charted depth at a place (a click on open water): the depth area or dredged area of the
/// largest-scale chart there, as a feature report.
#[tauri::command]
fn chart_depth_at(lon: f64, lat: f64) -> Option<serde_json::Value> {
    let (band, table, d1, d2, dredged) = charts()?.depth_area_at(lon, lat)?;
    let m = |v: f64| format!("{} m", (v * 10.0).round() / 10.0);
    let scale = if table.to_ascii_lowercase().contains("_enc_b") {
        format!("ENC, usage band {band}")
    } else {
        match band {
            5 => "LINZ 1:14,000 to 1:122,000".into(),
            4 => "LINZ 1:122,000 to 1:190,000".into(),
            3 => "LINZ 1:190,000 to 1:1,350,000".into(),
            2 => "LINZ 1:1,350,000 to 1:11,500,000".into(),
            _ => "LINZ 1:11,500,000 and smaller".into(),
        }
    };
    let mut attrs: Vec<(String, String)> = Vec::new();
    if table.is_empty() {
        // inside the chart's coverage, but it charts no depth here (its coastline and the Topo50
        // land drawn instead differ): shown as very shallow
        attrs.push(("Depth".into(), "not charted here".into()));
        attrs.push(("Chart".into(), scale));
        attrs.push((
            "Note".into(),
            "Between the chart's coastline and the Topo50 coastline drawn: the chart has land or nothing here. Treat as very shallow.".into(),
        ));
        return Some(serde_json::json!({ "code": "", "name": "No charted depth", "band": band, "attrs": attrs }));
    }
    if dredged {
        if let Some(v) = d1 {
            attrs.push(("Dredged to".into(), m(v)));
        }
        if let Some(v) = d2 {
            attrs.push(("Deepest".into(), m(v)));
        }
    } else {
        match (d1, d2) {
            (Some(a), _) if a < 0.0 => attrs.push(("Dries".into(), m(-a))),
            (Some(a), Some(b)) => attrs.push(("Depth".into(), format!("{} to {}", m(a), m(b)))),
            (Some(a), None) => attrs.push(("Depth".into(), format!("{} and deeper", m(a)))),
            (None, _) => attrs.push(("Depth".into(), "not given".into())),
        }
    }
    attrs.push(("Chart".into(), scale));
    attrs.push(("Below".into(), "chart datum".into()));
    Some(serde_json::json!({
        "code": if dredged { "DRGARE" } else { "DEPARE" },
        "name": if dredged { "Dredged area" } else { "Depth area" },
        "band": band,
        "attrs": attrs,
    }))
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
async fn monitor_open(app: tauri::AppHandle, view: String, label: Option<String>) -> Result<String, String> {
    if !view.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || view.is_empty() {
        return Err("unknown view".into());
    }
    // a window that was open before comes back under the same label, so it keeps its colours and place
    let wanted = label
        .and_then(|l| l.strip_prefix("monitor-").and_then(|n| n.parse::<u32>().ok()))
        .filter(|n| *n > 0 && app.get_webview_window(&format!("monitor-{n}")).is_none());
    let n = wanted.unwrap_or_else(|| (1u32..).find(|n| app.get_webview_window(&format!("monitor-{n}")).is_none()).unwrap_or(1));
    let label = format!("monitor-{n}");
    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(format!("index.html?view={view}").into()))
        .title(format!("TweedPlot monitor {n}"))
        .inner_size(1024.0, 640.0)
        .min_inner_size(320.0, 240.0)
        .build()
        .map(|_| label)
        .map_err(|e| e.to_string())
}

/// The line-setting window (setter.html): the snood and float beeper, always on top, movable
/// to another screen. Opened again, it's brought forward. Async, as monitor_open.
#[tauri::command]
async fn setter_open(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("line-set") {
        let _ = w.unminimize();
        return w.set_focus().map_err(|e| e.to_string());
    }
    tauri::WebviewWindowBuilder::new(&app, "line-set", tauri::WebviewUrl::App("setter.html".into()))
        .title("TweedPlot line shooting")
        .inner_size(560.0, 760.0)
        .min_inner_size(320.0, 360.0)
        .always_on_top(true)
        // the beeper must keep time minimised or behind another window
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The monitor windows that are open, by label, in number order.
#[tauri::command]
fn monitor_list(app: tauri::AppHandle) -> Vec<String> {
    let mut labels: Vec<String> = app.webview_windows().into_keys().filter(|l| l.starts_with("monitor-")).collect();
    labels.sort_by_key(|l| l[8..].parse::<u32>().unwrap_or(u32::MAX));
    labels
}

/// Bring a monitor window forward.
#[tauri::command]
async fn monitor_focus(app: tauri::AppHandle, label: String) -> Result<(), String> {
    let w = app.get_webview_window(&label).filter(|_| label.starts_with("monitor-")).ok_or("no such window")?;
    let _ = w.unminimize();
    w.set_focus().map_err(|e| e.to_string())
}

/// Close a monitor window.
#[tauri::command]
async fn monitor_close(app: tauri::AppHandle, label: String) -> Result<(), String> {
    let w = app.get_webview_window(&label).filter(|_| label.starts_with("monitor-")).ok_or("no such window")?;
    w.close().map_err(|e| e.to_string())
}

/// Charted vertical clearances (bridges, overhead cables) within `radius_m` of a point, nearest first.
#[tauri::command]
fn chart_clearances(lat: f64, lon: f64, radius_m: f64) -> Vec<chart::Clearance> {
    charts().map(|s| s.clearances([lon, lat], radius_m.min(20_000.0))).unwrap_or_default()
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
    // images (KAP charts, GeoTIFFs, MBTiles, calibrated pictures, satellite images):
    // /img/<id>/<z>/<x>/<y>.png as drawn, /imgd/ through the calibration being worked on,
    // /imgp/ the picture itself flat (the calibration's image pane)
    for (prefix, how) in [("img/", ""), ("imgd/", "draft"), ("imgp/", "px")] {
        if let Some(rest) = path.trim_start_matches('/').strip_prefix(prefix) {
            let (body, mime) = imagery_cmd::tile(how, rest).unwrap_or_else(|| (clear_png().to_vec(), "image/png"));
            return Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::CACHE_CONTROL, "no-store")
                .header(header::CONTENT_TYPE, mime)
                .body(body)
                .expect("valid response");
        }
    }
    // own soundings: /pbathy/<z>/<x>/<y>.png
    if let Some(rest) = path.trim_start_matches('/').strip_prefix("pbathy/") {
        let p: Vec<u32> = rest.trim_end_matches(".png").split('/').filter_map(|v| v.parse().ok()).collect();
        let png = match p.as_slice() {
            [z, x, y] => pbathy::tile_png(*z as u8, *x, *y).map(|t| t.as_ref().clone()),
            _ => None,
        };
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::CONTENT_TYPE, "image/png")
            .body(png.unwrap_or_else(|| clear_png().to_vec()))
            .expect("valid response");
    }
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
    // survey depth images: /depthpack/<z>/<x>/<y>.png
    if let Some(rest) = path.trim_start_matches('/').strip_prefix("depthpack/") {
        let p: Vec<u32> = rest.trim_end_matches(".png").split('/').filter_map(|v| v.parse().ok()).collect();
        let png = match p.as_slice() {
            [z, x, y] if *z <= 22 => depthpacks::tile_png(*z as u8, *x, *y),
            _ => None,
        };
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::CONTENT_TYPE, "image/png")
            .body(png.unwrap_or_else(|| clear_png().to_vec()))
            .expect("valid response");
    }
    // NIWA bathymetry images: /nzbathy/<z>/<x>/<y>.png
    if let Some(rest) = path.trim_start_matches('/').strip_prefix("nzbathy/") {
        let p: Vec<u32> = rest.trim_end_matches(".png").split('/').filter_map(|v| v.parse().ok()).collect();
        let png = match (p.as_slice(), nzbathy()) {
            ([z, x, y], Some(n)) if *z <= 22 => n.tile_png(*z as u8, *x, *y).map(|t| t.as_ref().clone()),
            _ => None,
        };
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::CONTENT_TYPE, "image/png")
            .body(png.unwrap_or_else(|| clear_png().to_vec()))
            .expect("valid response");
    }
    let parts: Vec<&str> = path.trim_start_matches('/').trim_end_matches(".pbf").split('/').collect();
    let parsed = match parts.as_slice() {
        [z, x, y] => z.parse::<u8>().ok().zip(x.parse::<u32>().ok()).zip(y.parse::<u32>().ok()),
        _ => None,
    };
    // never cached by the WebView: a tile drawn by an older build or from older data
    // would otherwise stay on screen (the chart store keeps its own cache in memory)
    let builder = Response::builder().header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*").header(header::CACHE_CONTROL, "no-store");
    match parsed {
        Some(((z, x), y)) => match charts() {
            Some(store) => match store.try_tile(z, x, y) {
                Some(t) => builder.header(header::CONTENT_TYPE, "application/x-protobuf").body(t.as_ref().clone()),
                // drawing it failed (reported on the terminal): the chart asks again later
                None => builder.status(StatusCode::INTERNAL_SERVER_ERROR).body(Vec::new()),
            },
            None => builder.status(StatusCode::SERVICE_UNAVAILABLE).body(Vec::new()),
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
    let _guard = TILE_LOG_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner());
    let reset = std::fs::metadata(path).is_ok_and(|m| m.len() > 1_000_000);
    let file = if reset { std::fs::File::create(path) } else { std::fs::OpenOptions::new().create(true).append(true).open(path) };
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
    jobs.queued_at.insert(path.clone(), std::time::Instant::now());
    jobs.responders.insert(path, vec![responder]);
    drop(jobs);
    q.1.notify_one();
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // update checks against the signed latest.json of the newest GitHub release (updates.js)
        .plugin(tauri_plugin_updater::Builder::new().build())
        // the main window is the app: closing it closes any monitor windows too
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            system_info,
            open_folder,
            chart_info,
            chart_feature,
            chart_depth_at,
            chart_scale_at,
            chart_s52_context,
            s52_symbols,
            s52_styles,
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
            depthpacks::depthpack_status,
            depthpacks::depthpack_surveys,
            depthpacks::depthpack_height,
            depthpacks::depthpack_import,
            depthpacks::depthpack_delete,
            nzbathy_pick_file,
            nzbathy_import,
            nzbathy_status,
            nzbathy_delete,
            webmap::webmap_set,
            webmap::webmap_status,
            webmap::webmap_clear_cache,
            imagery_cmd::images_list,
            imagery_cmd::images_import,
            imagery_cmd::images_update,
            imagery_cmd::images_set_crs,
            imagery_cmd::images_calib_info,
            imagery_cmd::images_calib_fit,
            imagery_cmd::images_calib_save,
            imagery_cmd::images_calib_discard,
            imagery_cmd::images_calib_clear,
            imagery_cmd::images_reorder,
            imagery_cmd::images_order_by_scale,
            imagery_cmd::images_remove,
            imagery_cmd::images_folder,
            imagery_cmd::s2_search,
            imagery_cmd::s2_thumbnail,
            imagery_cmd::s2_download,
            imagery_cmd::s2_stop,
            pbathy::pbathy_status,
            pbathy::pbathy_pick_file,
            pbathy::pbathy_import,
            pbathy::pbathy_from_tracks,
            pbathy::pbathy_cancel,
            pbathy::pbathy_style,
            pbathy::pbathy_fit,
            pbathy::pbathy_at,
            pbathy::pbathy_left_at,
            pbathy::pbathy_live_status,
            pbathy::pbathy_live_set,
            pbathy::pbathy_clear,
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
            grib_add,
            grib_delete,
            grib_load,
            grib_unload,
            grib_loaded,
            grib_wind_grid,
            grib_grid,
            grib_cmd::grib_profile,
            grib_cmd::grib_section,
            grib_compare,
            grib_meteogram,
            grib_verify_times,
            grib_verify_scores,
            grib_verify_track,
            grib_verify_grid,
            sst_verify,
            grib_point,
            grib_along,
            radar_sim::radar_sim_start,
            radar_sim::radar_sim_stop,
            radar_sim::radar_sim_stats,
            settings::settings_get,
            settings::settings_save,
            nmea::nmea_ports,
            nmea::autopilot_send,
            nmea::nmea_sources_get,
            nmea::nmea_sources_set,
            nmea::nmea_gps_stream_set,
            nmea::nmea_capture_begin,
            nmea::nmea_capture_append,
            nmea::nmea_capture_end,
            polar_files,
            polar_table,
            polar_save,
            polar_import,
            polar_open_folder,
            polar_speed,
            polar_vmg_angles,
            route_compute,
            sk_status,
            stations_cmd::stations_latest,
            stations_cmd::station_history,
            live::ais_targets,
            live::ais_online,
            live::ow_register,
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
            setter_open,
            library_trash,
            library_backup_dir,
            library_backup_to_info,
            library_backup_to,
            library_unlock,
            library_is_protected,
            overlay_add,
            overlay_remove,
            overlay_marks,
            overlay_tracks,
            overlay_track,
            trips_list,
            events_list,
            tracks_density,
            event_save,
            events_delete,
            events_csv,
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
            library_file_contents,
            library_merge_preview,
            weather_save_png,
            screenshot::screenshot_folder,
            screenshot::screenshot_use_default,
            screenshot::screenshot_pick_folder,
            screenshot::screenshot_open_folder,
            screenshot::screenshot_take,
            pbathy::pbathy_pick_xyz_save,
            pbathy::pbathy_export_xyz,
            pbathy::pbathy_import_from_library,
            library_backups,
            library_use_backup,
            marks_update,
            tracks_export,
            tracks_set_depth_ref,
            tracks_depth_unknown,
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
            chart_texts::chart_text,
            publications::pubs_status,
            publications::pubs_download,
            publications::pub_open,
            chart_texts::chart_texts_status,
            chart_texts::chart_texts_import,
            s63_cmd::s63_status,
            s63_cmd::s63_activate,
            s63_cmd::s63_deactivate,
            s63_cmd::s63_install_permits,
            s63_cmd::s63_remove_permits,
            s63_cmd::s63_install_key,
            s63_cmd::s63_import_exchange_set,
            s63_cmd::s63_remove_cells,
            s63_cmd::s63_plugin_install_file,
            s63_cmd::s63_plugin_check,
            s63_cmd::s63_plugin_download,
            s63_cmd::s63_plugin_remove
        ])
        .register_asynchronous_uri_scheme_protocol("chart", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            // online maps have their own workers, so a slow server can't hold up chart tiles
            if webmap::is_web_path(&path) {
                webmap::serve(path, responder);
                return;
            }
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
            if let Ok(dir) = app.path().app_data_dir() {
                webmap::init(dir.join("webmap-cache"));
            }
            imagery_cmd::init(app.handle());
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
                            if let Err(e) = l.backup_if_due_on(&chrono::Local::now().format("%Y-%m-%d").to_string()) {
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
                // the depth images first (their metadata only: quick). Until each is open its
                // tiles come back clear, and the chart keeps a clear tile once it has it, so
                // it is told to ask again ("depth-images-ready", main.js)
                open_hires(&h);
                depthpacks::open(&h);
                open_nzbathy(&h);
                pbathy::open(&h);
                let _ = h.emit("depth-images-ready", ());
                s63_cmd::load(&h);
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
