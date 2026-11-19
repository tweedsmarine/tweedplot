//! Images on the chart (Chart › Layers › Images): KAP raster charts and GeoTIFFs the user
//! imports, and Sentinel-2 satellite images downloaded from Earth Search on AWS. The files
//! live in the images folder; the imagery crate draws them as tiles on the chart protocol
//! (`/img/<id>/<z>/<x>/<y>.png`).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

use imagery::s2::{Progress, Scene};
use imagery::store::{Entry, Patch, Store};
use tauri::{Emitter, Manager};

static STORE: OnceLock<Store> = OnceLock::new();
/// The download running now (one at a time), for Stop.
static RUNNING: Mutex<Option<Arc<Progress>>> = Mutex::new(None);

/// `RACENAV_IMAGES_DIR`, else `<dev data>\images` on the developer's PC, else `<app data>\images`.
fn images_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_IMAGES_DIR") {
        return PathBuf::from(d);
    }
    let dev = crate::dev_root();
    if dev.is_dir() {
        return dev.join("images");
    }
    app.path().app_data_dir().map(|p| p.join("images")).unwrap_or_else(|_| PathBuf::from("images"))
}

pub(crate) fn init(app: &tauri::AppHandle) {
    let _ = STORE.set(Store::open(&images_dir(app)));
}

fn store() -> Result<&'static Store, String> {
    STORE.get().ok_or_else(|| "The images folder isn't open yet.".to_string())
}

/// `/img/<id>/<z>/<x>/<y>.png` → the tile's PNG (None: nothing there).
pub(crate) fn tile_png(rest: &str) -> Option<Vec<u8>> {
    let mut it = rest.trim_end_matches(".png").split('/');
    let id = it.next()?;
    let z: u8 = it.next()?.parse().ok()?;
    let x: u32 = it.next()?.parse().ok()?;
    let y: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || z > 24 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    STORE.get()?.tile_png(id, z, x, y)
}

#[tauri::command]
pub(crate) fn images_list() -> Result<Vec<Entry>, String> {
    Ok(store()?.list())
}

/// Pick KAP charts or GeoTIFFs and copy them in. Each file that can't be shown is reported
/// and the rest still added.
#[tauri::command]
pub(crate) async fn images_import(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("Import KAP charts or GeoTIFF images")
        .add_filter("KAP charts and GeoTIFFs", &["kap", "KAP", "tif", "tiff", "TIF", "TIFF"])
        .add_filter("All files", &["*"])
        .blocking_pick_files();
    let Some(list) = picked else { return Ok(serde_json::json!({ "added": [], "errors": [] })) };
    let paths: Vec<PathBuf> = list.into_iter().filter_map(|f| f.into_path().ok()).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let s = store()?;
        let (mut added, mut errors) = (Vec::new(), Vec::new());
        for p in paths {
            match s.import(&p) {
                Ok(e) => added.push(e),
                Err(e) => errors.push(format!("{}: {e}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
            }
        }
        Ok(serde_json::json!({ "added": added, "errors": errors }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn images_update(id: String, patch: Patch) -> Result<Entry, String> {
    store()?.update(&id, patch).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn images_reorder(ids: Vec<String>) -> Result<(), String> {
    store()?.reorder(&ids).map_err(|e| e.to_string())
}

/// The raster charts in scale order, most detailed on top.
#[tauri::command]
pub(crate) fn images_order_by_scale() -> Result<Vec<Entry>, String> {
    let s = store()?;
    s.order_by_scale().map_err(|e| e.to_string())?;
    Ok(s.list())
}

#[tauri::command]
pub(crate) fn images_remove(id: String) -> Result<(), String> {
    store()?.remove(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn images_folder() -> Result<String, String> {
    Ok(store()?.dir().to_string_lossy().into_owned())
}

fn check_area(b: &[f64; 4]) -> Result<(), String> {
    if !b.iter().all(|v| v.is_finite()) || b[0] >= b[2] || b[1] >= b[3] || b[1] < -85.0 || b[3] > 85.0 {
        return Err("The area isn't valid.".into());
    }
    if b[2] - b[0] > 3.0 || b[3] - b[1] > 2.0 {
        return Err("The area is too large: zoom in to the part you need (under about 150 km across).".into());
    }
    Ok(())
}

/// Sentinel-2 scenes over an area between two dates (YYYY-MM-DD).
#[tauri::command]
pub(crate) async fn s2_search(bbox: [f64; 4], from: String, to: String, max_cloud: f64) -> Result<Vec<Scene>, String> {
    check_area(&bbox)?;
    tauri::async_runtime::spawn_blocking(move || imagery::s2::search(bbox, &from, &to, max_cloud.clamp(0.0, 100.0)).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// A scene's preview picture, as a data: URL for the list.
#[tauri::command]
pub(crate) async fn s2_thumbnail(url: String) -> Result<String, String> {
    use base64::Engine;
    tauri::async_runtime::spawn_blocking(move || {
        let b = imagery::s2::thumbnail(&url).map_err(|e| e.to_string())?;
        let mime = if b.starts_with(&[0x89, b'P']) { "image/png" } else { "image/jpeg" };
        Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(b)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download the area from a scene into the images folder and add it. Progress comes as
/// `s2-progress` events ({ done, total } blocks).
#[tauri::command]
pub(crate) async fn s2_download(app: tauri::AppHandle, scene: Scene, bbox: [f64; 4], max_px: Option<usize>) -> Result<Entry, String> {
    check_area(&bbox)?;
    let progress = Arc::new(Progress::default());
    {
        let mut r = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if r.is_some() {
            return Err("A satellite image is already downloading.".into());
        }
        *r = Some(progress.clone());
    }
    let max_px = max_px.unwrap_or(4096).clamp(256, 8192);
    let p2 = progress.clone();
    let h = app.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let d2 = done.clone();
    std::thread::spawn(move || {
        while !d2.load(Ordering::Relaxed) {
            let _ = h.emit("s2-progress", serde_json::json!({ "done": p2.done.load(Ordering::Relaxed), "total": p2.total.load(Ordering::Relaxed) }));
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    });
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<Entry, String> {
        let s = store()?;
        std::fs::create_dir_all(s.dir()).map_err(|e| e.to_string())?;
        let date = scene.datetime.get(..10).unwrap_or("").to_string();
        let file = s.free_name(&format!("S2 {} {}.tif", scene.tile, date));
        let path = s.dir().join(&file);
        imagery::s2::download(&scene, bbox, max_px, &path, &progress).map_err(|e| {
            let _ = std::fs::remove_file(path.with_extension("tif.part"));
            if e.to_string() == "stopped" {
                "Stopped.".to_string()
            } else {
                e.to_string()
            }
        })?;
        let raster = imagery::store::open_raster("geotiff", &path).map_err(|e| e.to_string())?;
        let platform = scene.platform.replace("sentinel-", "Sentinel-").replace("-2a", "-2A").replace("-2b", "-2B").replace("-2c", "-2C");
        let name = format!("{} {} {}", if platform.is_empty() { "Sentinel-2".to_string() } else { platform }, date, scene.tile).trim().to_string();
        s.add(&file, "geotiff", &name, "Sentinel-2", &date, &imagery::s2::credit(&scene.datetime), None, None, raster.as_ref()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    done.store(true, Ordering::Relaxed);
    *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = None;
    r
}

#[tauri::command]
pub(crate) fn s2_stop() {
    if let Some(p) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        p.stop.store(true, Ordering::Relaxed);
    }
}
