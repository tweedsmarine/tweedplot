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
        Ok(s) => *STORE.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(s)),
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
pub async fn pbathy_import(app: tauri::AppHandle, path: String, again: Option<bool>) -> Result<serde_json::Value, String> {
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
        let n = if pbg { s.import_pbg(&p, &mut prog, &CANCEL) } else { s.import_xyz(&p, &mut prog, &CANCEL) }.map_err(|e| e.to_string())?;
        Ok(serde_json::json!({ "added": n, "name": name, "seconds": t.elapsed().as_secs_f64() }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Add the depths logged along the library's tracks (those not added before, or changed
/// since), spikes dropped. Heights are as logged: below the surface or transducer, not
/// reduced to chart datum.
#[tauri::command]
pub async fn pbathy_from_tracks(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
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
            return Ok(serde_json::json!({ "added": 0, "tracks": 0, "with_depth": with_depth.len(), "changed": changed }));
        }
        let modified: std::collections::HashMap<&str, i64> = tracks.iter().map(|t| (t.id.as_str(), t.modified)).collect();
        let mut w = s.writer("Own tracks", &format!("{} tracks from {}", todo.len(), lib.name()), 0).map_err(|e| e.to_string())?;
        let mut err: Option<String> = None;
        let (mut kept, mut dropped) = (0usize, 0usize);
        let mut prog = progress(&app);
        lib.for_each_track(
            &todo,
            &mut |id, segs| {
                if err.is_some() {
                    return;
                }
                for seg in segs {
                    let d: Vec<Option<f64>> = seg.iter().map(|p| p.depth).collect();
                    let keep = bathy::clean_depths(&d);
                    for (p, k) in seg.iter().zip(keep) {
                        if p.depth.is_none() {
                            continue;
                        }
                        if !k {
                            dropped += 1;
                            continue;
                        }
                        kept += 1;
                        let h = bathy::hours(p.t.unwrap_or(0));
                        if let Err(e) = w.add(p.lon, p.lat, -(p.depth.unwrap_or(0.0) as f32), h, bathy::SRC_OWN) {
                            err = Some(e.to_string());
                            return;
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
        s.changed();
        Ok(serde_json::json!({ "added": n, "tracks": todo.len(), "with_depth": with_depth.len(), "kept": kept, "dropped": dropped, "changed": changed }))
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

/// Delete all of it.
#[tauri::command]
pub fn pbathy_clear() -> Result<(), String> {
    if BUSY.load(Ordering::SeqCst) {
        return Err("Wait for the build to finish (or cancel it).".into());
    }
    need()?.clear().map_err(|e| e.to_string())
}
