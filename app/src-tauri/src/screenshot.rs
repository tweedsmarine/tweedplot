//! Screenshots: the toolbar's Screenshot button saves a picture of the TweedPlot window, as
//! the crew sees it (drawers, panel, dialogs and all), for their own records or to send for
//! support. They go into one folder, chosen the first time (Desktop\TweedPlot-Screenshots to
//! begin with) and changed in Settings › Display. The folder is kept here, not by the page,
//! so the page can only ever write screenshots into the folder the crew picked.
//!
//! The window is captured by the operating system (xcap: PrintWindow with full content on
//! Windows, the window list on macOS), because a web page can't take a picture of itself
//! with the chart's WebGL canvas and every open box in it. macOS asks once for Screen
//! Recording permission.

use std::path::{Path, PathBuf};
use tauri::Manager;

const FOLDER_NAME: &str = "TweedPlot-Screenshots";

fn store(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("screenshots.json"))
}

/// The folder chosen for screenshots, if one has been.
fn chosen(app: &tauri::AppHandle) -> Option<PathBuf> {
    let s = std::fs::read_to_string(store(app)?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("dir").and_then(|d| d.as_str()).filter(|d| !d.is_empty()).map(PathBuf::from)
}

fn choose(app: &tauri::AppHandle, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Can't make the folder {}: {e}", dir.display()))?;
    let f = store(app).ok_or("no settings folder")?;
    if let Some(p) = f.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::write(&f, serde_json::json!({ "dir": dir.to_string_lossy() }).to_string()).map_err(|e| e.to_string())
}

/// Desktop\TweedPlot-Screenshots (the Desktop OneDrive may have moved, as Windows reports it),
/// or the home folder when there is no desktop.
fn default_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path().desktop_dir().or_else(|_| app.path().home_dir()).unwrap_or_else(|_| PathBuf::from(".")).join(FOLDER_NAME)
}

/// `{ dir, default }`: the chosen folder (null before the first screenshot) and the suggested one.
#[tauri::command]
pub(crate) fn screenshot_folder(app: tauri::AppHandle) -> serde_json::Value {
    serde_json::json!({
        "dir": chosen(&app).map(|d| d.to_string_lossy().into_owned()),
        "default": default_dir(&app).to_string_lossy(),
    })
}

/// Use the suggested folder (Desktop\TweedPlot-Screenshots); returns it.
#[tauri::command]
pub(crate) fn screenshot_use_default(app: tauri::AppHandle) -> Result<String, String> {
    let d = default_dir(&app);
    choose(&app, &d)?;
    Ok(d.to_string_lossy().into_owned())
}

/// Ask for a folder; returns it, or None if cancelled.
#[tauri::command]
pub(crate) async fn screenshot_pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let a = app.clone();
    let start = chosen(&app).unwrap_or_else(|| default_dir(&app));
    let start = if start.exists() { start } else { start.parent().map(Path::to_path_buf).unwrap_or(start) };
    let picked = tauri::async_runtime::spawn_blocking(move || a.dialog().file().set_title("Choose the folder for screenshots").set_directory(start).blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    let Some(p) = picked else { return Ok(None) };
    let p = p.into_path().map_err(|e| e.to_string())?;
    choose(&app, &p)?;
    Ok(Some(p.to_string_lossy().into_owned()))
}

/// Open the screenshots folder in the file manager.
#[tauri::command]
pub(crate) fn screenshot_open_folder(app: tauri::AppHandle) -> Result<(), String> {
    let d = chosen(&app).ok_or("No screenshots folder has been chosen yet.")?;
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let prog = "explorer";
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let prog = "xdg-open";
    std::process::Command::new(prog).arg(&d).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Take a screenshot of the window the button was pressed in and save it in the chosen folder
/// as TweedPlot-2026-11-23-125930.png (local time). Returns the file's path. The PNG carries
/// TweedPlot's version and the time in UTC as text, for support.
#[tauri::command]
pub(crate) async fn screenshot_take(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<String, String> {
    let dir = chosen(&app).ok_or("No screenshots folder has been chosen yet.")?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let img = capture((pos.x, pos.y, size.width, size.height))?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("Can't make the folder {}: {e}", dir.display()))?;
        let now = chrono::Local::now();
        let mut path = dir.join(format!("TweedPlot-{}.png", now.format("%Y-%m-%d-%H%M%S")));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("TweedPlot-{}-{n}.png", now.format("%Y-%m-%d-%H%M%S")));
            n += 1;
        }
        let png = encode(&img, &[("Software", format!("TweedPlot {version}")), ("Creation Time", chrono::Utc::now().to_rfc3339())])?;
        std::fs::write(&path, png).map_err(|e| format!("Can't save {}: {e}", path.display()))?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A picture of the screen: width, height and RGBA rows.
struct Shot {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

/// This process's window nearest to the given place and size (TweedPlot may have several
/// windows open, the monitor windows among them).
#[cfg(any(windows, target_os = "macos"))]
fn capture((x, y, w, h): (i32, i32, u32, u32)) -> Result<Shot, String> {
    let me = std::process::id();
    let wins = xcap::Window::all().map_err(|e| format!("Can't list the windows: {e}"))?;
    let off = |v: &xcap::Window| -> Option<i64> {
        if v.pid().ok()? != me || v.is_minimized().unwrap_or(false) {
            return None;
        }
        let d = |a: i64, b: i64| (a - b).abs();
        Some(d(v.x().ok()?.into(), x.into()) + d(v.y().ok()?.into(), y.into()) + d(v.width().ok()?.into(), w.into()) + d(v.height().ok()?.into(), h.into()))
    };
    let win = wins.iter().filter_map(|v| off(v).map(|o| (o, v))).min_by_key(|(o, _)| *o).map(|(_, v)| v).ok_or("TweedPlot's window wasn't found")?;
    let img = win.capture_image().map_err(|e| {
        if cfg!(target_os = "macos") {
            format!("The window couldn't be captured ({e}). Allow TweedPlot in System Settings › Privacy & Security › Screen Recording.")
        } else {
            format!("The window couldn't be captured: {e}")
        }
    })?;
    Ok(Shot { w: img.width(), h: img.height(), rgba: img.into_raw() })
}

/// Linux isn't a TweedPlot platform (it is only built there for the checks).
#[cfg(not(any(windows, target_os = "macos")))]
fn capture(_: (i32, i32, u32, u32)) -> Result<Shot, String> {
    Err("Screenshots work on Windows and macOS.".into())
}

fn encode(s: &Shot, text: &[(&str, String)]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, s.w, s.h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Fast);
        for (k, v) in text {
            e.add_text_chunk((*k).to_string(), v.clone()).map_err(|e| e.to_string())?;
        }
        let mut wr = e.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&s.rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Shot, encode};

    #[test]
    fn png_with_text() {
        let s = Shot { w: 2, h: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] };
        let png = encode(&s, &[("Software", "TweedPlot 0.0.0".into())]).unwrap();
        let d = png::Decoder::new(std::io::Cursor::new(png));
        let r = d.read_info().unwrap();
        assert_eq!((r.info().width, r.info().height), (2, 1));
        assert!(r.info().uncompressed_latin1_text.iter().any(|t| t.keyword == "Software" && t.text == "TweedPlot 0.0.0"));
    }
}
