//! Own soundings: the store of the boat's own depth soundings gridded into a seabed,
//! imports of TimeZero PBG and XYZ files, building it from the library's tracks with depth,
//! and the depth images (`/pbathy/<z>/<x>/<y>.png` on the tile protocol).
//!
//! The store is `pbathy\bathy.sqlite` in `C:\dev\tweedplot-data` when that exists, else in
//! the app's data folder (`RACENAV_PBATHY_DIR` overrides). It's the skipper's own data, like
//! the library: never uploaded anywhere.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use tauri::{Emitter, Manager};

static STORE: RwLock<Option<Arc<bathy::Store>>> = RwLock::new(None);

// ---------- reducing soundings to chart datum ----------------------------------------------

/// Turns a depth as logged into a depth below chart datum: first below the surface (from what
/// it was measured below: the surface, the transducer plus its depth, the keel plus the draft),
/// then less the height of tide above chart datum there and then (LINZ's tables and harmonic
/// constituents, see `tides::reduce`). Depths already reduced (TimeZero's) are left alone;
/// where there is no tide (no port within reach), the depth below the surface is kept and counted.
pub(crate) struct Datum {
    tides: Option<tides::reduce::Reducer>,
    transducer_m: Option<f64>,
    draft_m: Option<f64>,
    /// Soundings reduced, kept below the surface for want of a tide, and already at chart
    /// datum (TimeZero's) so left as they were.
    pub reduced: i64,
    pub no_tide: i64,
    pub already: i64,
}

impl Datum {
    pub(crate) fn new(app: &tauri::AppHandle) -> Self {
        let v = crate::settings::settings_get(app.clone()).ok().map(|s| s.vessel);
        Self::with(crate::tide_db(), v.as_ref().and_then(|v| v.transducer_depth_m), v.as_ref().and_then(|v| v.draft_m))
    }

    fn with(db: Arc<tides::TideDb>, transducer_m: Option<f64>, draft_m: Option<f64>) -> Self {
        let r = tides::reduce::Reducer::new(db, tides::reduce::DEFAULT_MAX_KM);
        Datum { tides: r.has_ports().then_some(r), transducer_m, draft_m, reduced: 0, no_tide: 0, already: 0 }
    }

    /// Whether there is any tide data to reduce with.
    pub(crate) fn has_tides(&self) -> bool {
        self.tides.is_some()
    }

    /// Depth below the surface from a depth measured below `kind` ("" = not known: taken as
    /// below the transducer, as most sounders give it).
    fn below_surface(&self, d: f64, kind: &str, offset: Option<f64>) -> f64 {
        match kind {
            "surface" | "datum" => d,
            "keel" => d + offset.or(self.draft_m).unwrap_or(0.0),
            _ => d + offset.or(self.transducer_m).unwrap_or(0.0),
        }
    }

    /// Depth below chart datum (m, positive down).
    pub(crate) fn reduce(&mut self, lat: f64, lon: f64, t: Option<i64>, d: f64, kind: &str, offset: Option<f64>) -> f64 {
        if kind == "datum" {
            self.already += 1;
            return d;
        }
        let s = self.below_surface(d, kind, offset);
        let tide = match (t, self.tides.as_mut()) {
            (Some(t), Some(r)) => r.at(lat, lon, t),
            _ => None,
        };
        match tide {
            Some(tide) => {
                self.reduced += 1;
                s - tide.h
            }
            None => {
                self.no_tide += 1;
                s
            }
        }
    }
}
static BUSY: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_PBATHY_DIR") {
        return PathBuf::from(d);
    }
    let dev = crate::dev_root();
    if dev.is_dir() {
        return dev.join("pbathy");
    }
    app.path().app_data_dir().map(|p| p.join("pbathy")).unwrap_or_else(|_| PathBuf::from("pbathy"))
}

/// Open the store (at start-up; quick, nothing is read until drawn).
pub fn open(app: &tauri::AppHandle) {
    match bathy::Store::open(&dir(app).join("bathy.sqlite")) {
        Ok(s) => {
            let s = Arc::new(s);
            *STORE.write().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
            // a store from an older TweedPlot: the zoomed-out levels (a few tiles)
            std::thread::spawn(move || {
                if let Err(e) = s.ensure_levels() {
                    eprintln!("own soundings: {e}");
                }
            });
        }
        Err(e) => eprintln!("own soundings: {e}"),
    }
}

pub fn store() -> Option<Arc<bathy::Store>> {
    STORE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn need() -> Result<Arc<bathy::Store>, String> {
    store().ok_or_else(|| "The own soundings could not be opened.".to_string())
}

/// The image for a map tile.
pub fn tile_png(z: u8, x: u32, y: u32) -> Option<Arc<Vec<u8>>> {
    store()?.tile_png(z, x, y)
}

struct Busy;
impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}
fn busy() -> Result<Busy, String> {
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("The own soundings are already being built.".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    Ok(Busy)
}

fn progress(app: &tauri::AppHandle) -> impl FnMut(&str, usize, usize) + '_ {
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    move |stage: &str, done: usize, total: usize| {
        if last.elapsed().as_millis() >= 200 || done == total {
            last = std::time::Instant::now();
            let _ = app.emit("pbathy-progress", serde_json::json!({ "stage": stage, "done": done, "total": total }));
        }
    }
}

/// What's in the store, how it's drawn, and whether a build is running.
#[tauri::command]
pub fn pbathy_status() -> Result<serde_json::Value, String> {
    let s = need()?;
    let info = s.info().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "info": info,
        "style": s.style(),
        "generation": s.generation(),
        "busy": BUSY.load(Ordering::SeqCst),
        "path": s.path().to_string_lossy(),
    }))
}

/// Pick a TimeZero PBG or XYZ file to import.
#[tauri::command]
pub async fn pbathy_pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("Import soundings")
        .add_filter("Soundings (TimeZero PBG, XYZ)", &["tzpbg", "dbpbg", "dbwassp", "xyz", "txt", "csv"])
        .add_filter("TimeZero PBG", &["tzpbg", "dbpbg", "dbwassp"])
        .add_filter("XYZ text (longitude, latitude, height)", &["xyz", "txt", "csv"])
        .add_filter("All files", &["*"])
        .blocking_pick_file();
    let Some(p) = picked else { return Ok(None) };
    Ok(Some(p.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned()))
}

/// Import a TimeZero PBG or XYZ file into the store. `again`: import it even if a file of
/// the same name and size was imported before (its soundings would count twice).
#[tauri::command]
pub async fn pbathy_import(app: tauri::AppHandle, path: String, again: Option<bool>, check: Option<bool>) -> Result<serde_json::Value, String> {
    let s = need()?;
    let _b = busy()?;
    tauri::async_runtime::spawn_blocking(move || {
        let p = PathBuf::from(&path);
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = std::fs::metadata(&p).map_err(|e| e.to_string())?.len() as i64;
        let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let pbg = matches!(ext.as_str(), "tzpbg" | "dbpbg" | "dbwassp");
        let kind = if pbg { "TimeZero PBG" } else { "XYZ" };
        if !again.unwrap_or(false) && s.has_source(kind, &name, bytes) {
            return Ok(serde_json::json!({ "already": true, "name": name }));
        }
        let mut prog = progress(&app);
        let t = std::time::Instant::now();
        let reference = if check.unwrap_or(true) { chart_reference() } else { None };
        let (n, rejected) = if pbg { s.import_pbg_checked(&p, reference.as_ref(), &mut prog, &CANCEL) } else { s.import_xyz_checked(&p, reference.as_ref(), &mut prog, &CANCEL) }.map_err(|e| e.to_string())?;
        let back = if reference.is_some() { s.second_opinion(reference.as_ref(), &mut prog, &CANCEL).map_err(|e| e.to_string())? } else { 0 };
        Ok(serde_json::json!({ "added": n, "rejected": rejected, "back": back, "checked": reference.is_some(), "name": name, "seconds": t.elapsed().as_secs_f64() }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The chart's depth areas (LINZ, and any ENC) as the reference soundings are checked
/// against, when the charts are open and have depth areas.
fn chart_reference() -> Option<bathy::check::Reference> {
    let store = crate::charts()?;
    if !store.tables.iter().any(|t| t.kind == chart::Kind::DepthArea) {
        return None;
    }
    Some(bathy::check::Reference::new(move |b| {
        let mut g = bathy::check::RefGrid::new(b, bathy::check::REF_STEP);
        store.for_each_depth_band(b, |band, lo, hi, rings| {
            if let Some(lo) = lo {
                g.paint(band, lo.max(0.0), hi, rings);
            }
        });
        g
    }))
}

/// Add the depths logged along the library's tracks (those not added before, or changed
/// since), spikes dropped, each reduced to chart datum (see [`Datum`]): from what the track
/// says its depths are measured below, and the tide.
#[tauri::command]
pub async fn pbathy_from_tracks(app: tauri::AppHandle, check: Option<bool>) -> Result<serde_json::Value, String> {
    let s = need()?;
    let lib = crate::lib()?;
    let _b = busy()?;
    tauri::async_runtime::spawn_blocking(move || {
        let seeded = s.seeded().map_err(|e| e.to_string())?;
        let tracks = lib.tracks().map_err(|e| e.to_string())?;
        let with_depth: Vec<_> = tracks.iter().filter(|t| t.has_depth).collect();
        // tracks recorded with live soundings on (seeded as -1) came in live
        let todo: Vec<&library::TrackSummary> = with_depth.iter().copied().filter(|t| seeded.get(&t.id) != Some(&t.modified) && seeded.get(&t.id) != Some(&-1)).collect();
        // a track changed since it was added would count twice: those wait for a rebuild
        let changed = todo.iter().filter(|t| seeded.contains_key(&t.id)).count();
        let todo: Vec<String> = todo.iter().filter(|t| !seeded.contains_key(&t.id)).map(|t| t.id.clone()).collect();
        if todo.is_empty() {
            // nothing new: still give what was left out a second opinion (the rules improve)
            let reference = if check.unwrap_or(true) { chart_reference() } else { None };
            let mut prog = progress(&app);
            let back = if reference.is_some() { s.second_opinion(reference.as_ref(), &mut prog, &CANCEL).map_err(|e| e.to_string())? } else { 0 };
            return Ok(serde_json::json!({ "added": back, "back": back, "tracks": 0, "with_depth": with_depth.len(), "changed": changed }));
        }
        let modified: std::collections::HashMap<&str, i64> = tracks.iter().map(|t| (t.id.as_str(), t.modified)).collect();
        let refs: std::collections::HashMap<&str, (&str, Option<f64>)> = tracks.iter().map(|t| (t.id.as_str(), (t.depth_ref.as_str(), t.depth_offset))).collect();
        let mut datum = Datum::new(&app);
        let unknown = todo.iter().filter(|id| refs.get(id.as_str()).is_some_and(|r| r.0.is_empty())).count();
        let mut w = s.writer("Own tracks", &format!("{} tracks from {}", todo.len(), lib.name()), 0).map_err(|e| e.to_string())?;
        let mut err: Option<String> = None;
        let reference = if check.unwrap_or(true) { chart_reference() } else { None };
        let none = |_: f64, _: f64| None;
        let chart: &dyn Fn(f64, f64) -> Option<(f32, f32)> = match &reference {
            Some(r) => &|lon, lat| r.at(lon, lat),
            None => &none,
        };
        let mut counts = bathy::check::CheckCounts::default();
        let mut prog = progress(&app);
        lib.for_each_track(
            &todo,
            &mut |id, segs| {
                if err.is_some() {
                    return;
                }
                let (kind, offset) = refs.get(id).copied().unwrap_or(("", None));
                for seg in segs {
                    // reduced to chart datum first: the checks compare with the chart's depths
                    let fixes: Vec<(f64, f64, Option<f64>)> =
                        seg.iter().map(|p| (p.lon, p.lat, p.depth.map(|d| datum.reduce(p.lat, p.lon, p.t, d, kind, offset)))).collect();
                    let (verdicts, runs, unchecked) = bathy::check::check_segment(&fixes, chart);
                    counts.add(&verdicts, runs);
                    counts.unchecked += unchecked;
                    for ((p, f), v) in seg.iter().zip(&fixes).zip(verdicts) {
                        let Some(d) = f.2 else { continue };
                        let h = bathy::hours(p.t.unwrap_or(0));
                        if v == bathy::check::Verdict::Keep {
                            if let Err(e) = w.add(p.lon, p.lat, -(d as f32), h, bathy::SRC_OWN) {
                                err = Some(e.to_string());
                                return;
                            }
                        } else if d.is_finite() && d > 0.0 {
                            let why = match v {
                                bathy::check::Verdict::Spike => bathy::WHY_SPIKE,
                                bathy::check::Verdict::OffDeep => bathy::WHY_DEEP,
                                bathy::check::Verdict::Run => bathy::WHY_RUN,
                                bathy::check::Verdict::Echo => bathy::WHY_SPIKE | bathy::WHY_DEEP,
                                _ => bathy::WHY_SHALLOW,
                            };
                            w.reject(p.lon, p.lat, -(d as f32), h, bathy::SRC_OWN | why);
                        }
                    }
                }
                w.mark_seeded(id, modified.get(id).copied().unwrap_or(0));
            },
            &mut prog,
            &CANCEL,
        )
        .map_err(|e| e.to_string())?;
        if let Some(e) = err {
            return Err(e);
        }
        let n = w.finish(&mut prog, &CANCEL).map_err(|e| e.to_string())?;
        // left-out soundings the soundings beside them agree with go back in
        let back = if reference.is_some() { s.second_opinion(reference.as_ref(), &mut prog, &CANCEL).map_err(|e| e.to_string())? } else { 0 };
        s.changed();
        Ok(serde_json::json!({ "added": n + back, "back": back, "tracks": todo.len(), "with_depth": with_depth.len(), "kept": counts.kept, "dropped": counts.spikes, "counts": counts, "checked": reference.is_some(), "changed": changed,
            "reduced": datum.reduced, "no_tide": datum.no_tide, "already": datum.already, "tides": datum.has_tides(), "unknown_ref": unknown }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn pbathy_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Set how it's drawn; returns the new generation (for reloading the tiles).
#[tauri::command]
pub fn pbathy_style(style: bathy::Style) -> Result<u64, String> {
    let s = need()?;
    if s.style() != style {
        s.set_style(style);
    }
    Ok(s.generation())
}

/// A colour range (m, positive down) to suit an area [w, s, e, n].
#[tauri::command]
pub fn pbathy_fit(area: [f64; 4]) -> Option<(f32, f32)> {
    store()?.depth_range(area)
}

/// The sounding at a place.
#[tauri::command]
/// The nearest sounding within `radius_m` (the page asks for a few pixels at its zoom).
pub fn pbathy_at(lon: f64, lat: f64, radius_m: Option<f64>) -> Option<bathy::Sounding> {
    store()?.sounding_near(lon, lat, radius_m.unwrap_or(4.0).clamp(1.0, 5000.0))
}

/// A left-out sounding at a place, and why it was left out.
#[tauri::command]
pub fn pbathy_left_at(lon: f64, lat: f64, radius_m: Option<f64>) -> Option<bathy::Sounding> {
    store()?.left_out_near(lon, lat, radius_m.unwrap_or(4.0).clamp(1.0, 5000.0))
}

/// Delete all of it.
#[tauri::command]
pub fn pbathy_clear() -> Result<(), String> {
    if BUSY.load(Ordering::SeqCst) {
        return Err("Wait for the build to finish (or cancel it).".into());
    }
    need()?.clear().map_err(|e| e.to_string())
}

/// Write the own soundings into the library file at `path` (see `bathy::share`): the
/// lossless way to give them with the rest of a library. Returns the cells written.
pub fn export_into(path: &std::path::Path, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<i64, String> {
    need()?.export_into(path, progress, cancel).map_err(|e| e.to_string())
}

/// Where to write an XYZ text file of the own soundings.
#[tauri::command]
pub async fn pbathy_pick_xyz_save(app: tauri::AppHandle, suggest: Option<String>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = app.dialog().file().set_title("Write own soundings as XYZ").add_filter("XYZ text (longitude, latitude, height)", &["xyz"]);
    if let Some(s) = suggest {
        d = d.set_file_name(&s);
    }
    let Some(p) = d.blocking_save_file() else { return Ok(None) };
    let mut p = p.into_path().map_err(|e| e.to_string())?;
    if p.extension().is_none_or(|x| !x.eq_ignore_ascii_case("xyz")) {
        p.set_extension("xyz");
    }
    Ok(Some(p.to_string_lossy().into_owned()))
}

/// Write the own soundings as an XYZ text file (longitude, latitude, height in metres,
/// negative down; each cell's centre). Returns the lines written.
#[tauri::command]
pub async fn pbathy_export_xyz(app: tauri::AppHandle, path: String, job: Option<String>) -> Result<i64, String> {
    let s = need()?;
    let job = job.unwrap_or_default();
    let cancel = crate::job_begin(&job);
    let j = job.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut prog = crate::job_progress(&app, &j);
        s.export_xyz(std::path::Path::new(&path), &mut prog, &cancel).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());
    crate::job_end(&job);
    r?
}

/// Bring in the own soundings of a library file (written with them). `again`: even if
/// that file came in before, or from these soundings (they'd count twice).
#[tauri::command]
pub async fn pbathy_import_from_library(app: tauri::AppHandle, path: String, again: Option<bool>, job: Option<String>) -> Result<serde_json::Value, String> {
    let s = need()?;
    let _b = busy()?;
    let job = job.unwrap_or_default();
    let cancel = crate::job_begin(&job);
    let j = job.clone();
    let h = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let p = PathBuf::from(&path);
        let again = again.unwrap_or(false);
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = std::fs::metadata(&p).map_err(|e| e.to_string())?.len() as i64;
        if !again && s.has_source("Library file", &name, bytes) {
            return Ok(serde_json::json!({ "already": true, "name": name }));
        }
        let mut prog = crate::job_progress(&h, &j);
        let (added, left_out) = s.import_from(&p, again, &mut prog, &cancel).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({ "added": added, "left_out": left_out, "name": name }))
    })
    .await
    .map_err(|e| e.to_string());
    crate::job_end(&job);
    let r = r??;
    let _ = app.emit("pbathy-live", ());
    Ok(r)
}

// ---------- live soundings ----------------------------------------------------------------

/// Live soundings: on or off, the speed above which none are taken (0: no limit; a hull
/// planing or the sounder in aerated water reads rubbish), and what's happened today.
struct Live {
    on: bool,
    max_sog: f64,
    logger: bathy::live::Logger,
    reference: Option<Arc<bathy::check::Reference>>,
    day: String,
    added: i64,
    left: i64,
    /// When a depth last arrived (Unix s), and which kind it was.
    last_depth: f64,
    kind: &'static str,
    transducer_m: Option<f64>,
    /// When the settings (transducer depth) were last read.
    settings_at: f64,
    flushing: bool,
    /// Reducing to chart datum (made when the first depth arrives; again with the settings).
    datum: Option<Datum>,
    /// The last sounding was reduced (a tide was found).
    tide_ok: bool,
}

static LIVE: std::sync::Mutex<Option<Live>> = std::sync::Mutex::new(None);

fn now_f() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn today() -> String {
    // local date for the day's source row
    let t = chrono::Local::now();
    t.format("%Y-%m-%d").to_string()
}

fn live_state() -> std::sync::MutexGuard<'static, Option<Live>> {
    let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let (on, max_sog) =
            store().map_or((false, 0.0), |s| (s.meta("live_on").as_deref() == Some("1"), s.meta("live_max_sog").and_then(|v| v.parse().ok()).unwrap_or(0.0)));
        *g = Some(Live {
            on,
            max_sog,
            logger: Default::default(),
            reference: None,
            day: today(),
            added: 0,
            left: 0,
            last_depth: 0.0,
            kind: "",
            transducer_m: None,
            settings_at: 0.0,
            flushing: false,
            datum: None,
            tide_ok: false,
        });
    }
    g
}

/// A vessel update (Signal K or NMEA, whichever source is selected for depth): take a
/// sounding from it when live soundings are on: depth below the transducer plus the
/// transducer depth (Settings › Vessel), or below the surface when that's all there is.
pub fn live_vessel(app: &tauri::AppHandle, v: &signalk::VesselState) {
    let mut g = live_state();
    let Some(l) = g.as_mut() else { return };
    if !l.on {
        return;
    }
    let now = now_f();
    if now - l.settings_at > 60.0 {
        l.settings_at = now;
        l.transducer_m = crate::settings::settings_get(app.clone()).ok().and_then(|s| s.vessel.transducer_depth_m);
        // the settings and the tide data may have changed (a tide download)
        l.datum = Some(Datum::new(app));
    }
    let raw = if let Some(d) = v.depth {
        l.kind = "transducer";
        Some(d)
    } else if let Some(d) = v.depth_surface {
        l.kind = "surface";
        Some(d)
    } else {
        None
    };
    let (Some(raw), Some(lat), Some(lon)) = (raw, v.lat, v.lon) else { return };
    let (kind, transducer) = (l.kind, l.transducer_m);
    let Some(datum) = l.datum.as_mut() else { return };
    let before = datum.reduced;
    let d = datum.reduce(lat, lon, Some(now as i64), raw, kind, transducer);
    l.tide_ok = datum.reduced > before;
    if l.max_sog > 0.0 && v.sog.is_some_and(|s| s > l.max_sog) {
        return;
    }
    if l.logger.offer(bathy::live::Fix { t: now, lon, lat, depth: d }) {
        l.last_depth = now;
    }
    if !l.flushing {
        if let Some(batch) = l.logger.take(false) {
            l.flushing = true;
            if l.reference.is_none() {
                l.reference = chart_reference().map(Arc::new);
            }
            let reference = l.reference.clone();
            let app = app.clone();
            std::thread::spawn(move || live_flush(&app, batch, reference));
        }
    }
}

fn live_flush(app: &tauri::AppHandle, (ctx, new): (Vec<bathy::live::Fix>, Vec<bathy::live::Fix>), reference: Option<Arc<bathy::check::Reference>>) {
    let r = (|| -> Result<(i64, i64), String> {
        let s = need()?;
        if BUSY.load(Ordering::SeqCst) {
            return Err("busy".into()); // an import or build is writing: this batch waits for the next
        }
        let none = |_: f64, _: f64| None;
        let chart: &dyn Fn(f64, f64) -> Option<(f32, f32)> = match &reference {
            Some(r) => &|lon, lat| r.at(lon, lat),
            None => &none,
        };
        let mut w = s.writer("", "", 0).map_err(|e| e.to_string())?;
        let (a, l) = bathy::live::add_batch(&mut w, &ctx, &new, chart).map_err(|e| e.to_string())?;
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).map_err(|e| e.to_string())?;
        s.tally_source("Live", &today(), a, l).map_err(|e| e.to_string())?;
        // the track being recorded now came in live: Add my tracks leaves it alone
        if let Some(id) = crate::rec_track_id() {
            let _ = s.mark_seeded(&[(id, -1)]);
        }
        s.changed();
        Ok((a, l))
    })();
    let mut g = live_state();
    if let Some(l) = g.as_mut() {
        l.flushing = false;
        let day = today();
        if day != l.day {
            l.day = day;
            l.added = 0;
            l.left = 0;
        }
        match r {
            Ok((a, lo)) => {
                l.added += a;
                l.left += lo;
                let _ = app.emit("pbathy-live", serde_json::json!({ "added": l.added, "left": l.left }));
            }
            Err(e) if e == "busy" => {}
            Err(e) => eprintln!("own soundings, live: {e}"),
        }
    }
}

/// Live soundings: on or off, the speed limit, today's counts, and anything to warn about.
#[tauri::command]
pub fn pbathy_live_status(app: tauri::AppHandle) -> serde_json::Value {
    let transducer = crate::settings::settings_get(app).ok().and_then(|s| s.vessel.transducer_depth_m);
    let g = live_state();
    let Some(l) = g.as_ref() else { return serde_json::json!({}) };
    let now = now_f();
    let mut warn: Vec<&str> = Vec::new();
    if l.on {
        if l.last_depth == 0.0 || now - l.last_depth > 30.0 {
            warn.push("No depth with a position arriving from the selected source (Settings › Connections).");
        } else if l.kind == "transducer" && transducer.is_none() {
            warn.push("Transducer depth not set (Settings › Vessel): the soundings are below the transducer, its depth shallower than below the surface.");
        } else if l.kind == "surface" {
            warn.push("No depth below the transducer arriving: using the depth below the surface the source gives.");
        }
        if l.last_depth > 0.0 && !l.tide_ok {
            warn.push("Not reduced to chart datum: no tide here (no LINZ port within 30 km, or no tide data: Weather and tides › Tides › Download).");
        }
    } else if transducer.is_none() {
        warn.push("Set the transducer depth in Settings › Vessel before turning this on: soundings are taken below the transducer.");
    }
    serde_json::json!({
        "on": l.on, "max_sog": l.max_sog, "added": l.added, "left": l.left,
        "last": l.last_depth, "kind": l.kind, "transducer_m": transducer, "warnings": warn, "tide": l.tide_ok,
    })
}

/// Turn live soundings on or off, and set the speed limit (kn, 0 = none).
#[tauri::command]
pub fn pbathy_live_set(on: bool, max_sog: Option<f64>) -> Result<(), String> {
    let s = need()?;
    let mut g = live_state();
    let Some(l) = g.as_mut() else { return Ok(()) };
    l.on = on;
    if let Some(m) = max_sog {
        l.max_sog = m.max(0.0);
    }
    if !on {
        // what's waiting goes in with the next batch when turned on again; drop it now
        let _ = l.logger.take(true);
    }
    s.set_meta("live_on", if on { "1" } else { "0" }).map_err(|e| e.to_string())?;
    s.set_meta("live_max_sog", &l.max_sog.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From what the depth was measured below, then less the tide; TimeZero's left alone;
    /// no tide data: below the surface, counted.
    #[test]
    fn depths_to_chart_datum() {
        let mut none = Datum::with(Arc::new(tides::TideDb::default()), Some(1.2), Some(2.0));
        assert!(!none.has_tides());
        assert_eq!(none.reduce(-41.0, 173.0, Some(0), 10.0, "transducer", None), 11.2);
        assert_eq!(none.reduce(-41.0, 173.0, Some(0), 10.0, "transducer", Some(0.5)), 10.5);
        assert_eq!(none.reduce(-41.0, 173.0, Some(0), 10.0, "", None), 11.2, "not known: as below the transducer");
        assert_eq!(none.reduce(-41.0, 173.0, Some(0), 10.0, "keel", None), 12.0);
        assert_eq!(none.reduce(-41.0, 173.0, Some(0), 10.0, "surface", None), 10.0);
        assert_eq!(none.reduce(-41.0, 173.0, Some(0), 10.0, "datum", None), 10.0);
        assert_eq!((none.reduced, none.no_tide), (0, 5));

        // a port with the tide 3 m above chart datum all day
        let dir = std::env::temp_dir().join(format!("pbathy-datum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(tides::CONSTITUENTS_DIR)).unwrap();
        std::fs::write(
            dir.join(tides::CONSTITUENTS_DIR).join("1.txt"),
            " PORT        Units in CM\n STATION 001 TIME ZONE = 180, LATITUDE = 4100, LONGITUDE = 17300\n   1 Z0      .00000000   001 0105/1225 300.0000   0.00\n   2 M2      .08051140   001 0105/1225   0.0001 0.00\n",
        )
        .unwrap();
        let db = tides::TideDb::open(&dir);
        let mut d = Datum::with(Arc::new(db), Some(1.0), None);
        assert!(d.has_tides());
        let z = d.reduce(-41.0, 173.0, Some(1_700_000_000), 10.0, "transducer", None);
        assert!((z - 8.0).abs() < 0.01, "{z}");
        assert_eq!(d.reduce(-41.0, 173.0, Some(1_700_000_000), 10.0, "datum", None), 10.0);
        assert_eq!(d.already, 1);
        // far from the port: below the surface
        assert_eq!(d.reduce(-45.0, 170.0, Some(1_700_000_000), 10.0, "surface", None), 10.0);
        assert_eq!((d.reduced, d.no_tide), (1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
