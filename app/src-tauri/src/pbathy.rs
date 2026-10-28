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
/// since), spikes dropped. Heights are as logged: below the surface or transducer, not
/// reduced to chart datum.
#[tauri::command]
pub async fn pbathy_from_tracks(app: tauri::AppHandle, check: Option<bool>) -> Result<serde_json::Value, String> {
    let s = need()?;
    let lib = crate::lib()?;
    let _b = busy()?;
    tauri::async_runtime::spawn_blocking(move || {
        let seeded = s.seeded().map_err(|e| e.to_string())?;
        let tracks = lib.tracks().map_err(|e| e.to_string())?;
        let with_depth: Vec<_> = tracks.iter().filter(|t| t.has_depth).collect();
        let todo: Vec<&library::TrackSummary> = with_depth.iter().copied().filter(|t| seeded.get(&t.id) != Some(&t.modified)).collect();
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
                for seg in segs {
                    let fixes: Vec<(f64, f64, Option<f64>)> = seg.iter().map(|p| (p.lon, p.lat, p.depth)).collect();
                    let (verdicts, runs, unchecked) = bathy::check::check_segment(&fixes, chart);
                    counts.add(&verdicts, runs);
                    counts.unchecked += unchecked;
                    for (p, v) in seg.iter().zip(verdicts) {
                        let Some(d) = p.depth else { continue };
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
        Ok(serde_json::json!({ "added": n + back, "back": back, "tracks": todo.len(), "with_depth": with_depth.len(), "kept": counts.kept, "dropped": counts.spikes, "counts": counts, "checked": reference.is_some(), "changed": changed }))
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
pub fn pbathy_at(lon: f64, lat: f64) -> Option<bathy::Sounding> {
    store()?.sounding_at(lon, lat)
}

/// A left-out sounding at a place, and why it was left out.
#[tauri::command]
pub fn pbathy_left_at(lon: f64, lat: f64) -> Option<bathy::Sounding> {
    store()?.left_out_at(lon, lat)
}

/// Delete all of it.
#[tauri::command]
pub fn pbathy_clear() -> Result<(), String> {
    if BUSY.load(Ordering::SeqCst) {
        return Err("Wait for the build to finish (or cancel it).".into());
    }
    need()?.clear().map_err(|e| e.to_string())
}
