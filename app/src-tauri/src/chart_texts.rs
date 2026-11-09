//! Chart notes: the text files ENC features refer to (TXTDSC, NTXTDS), shown on a
//! feature's card. They come with ENC exchange sets, unencrypted even in S-63 sets, so
//! they are copied into `<enc home>/.texts` when an exchange set is imported (with or
//! without permits for its cells), or from any ENC folder (Chart › Chart notes). The LINZ
//! open data names the same files but doesn't carry them: those are found here too.
//! Plain ENC folders (enc, s63 in the data folders) are searched as well.

use base64::Engine;
use chart::enc::{picture, texts};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Name → file, built on first use and again after an import.
static INDEX: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);

/// Pictures already turned into PNG (name → the reply), cleared with the index.
static PICTURES: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);

/// Where imported notes are kept.
pub fn store_dir(app: &tauri::AppHandle) -> PathBuf {
    crate::s63_cmd::enc_home(app).join(".texts")
}

fn build_index(app: &tauri::AppHandle) -> HashMap<String, PathBuf> {
    let mut m = HashMap::new();
    // plain ENC folders first, so the imported copies (newest) win
    for d in crate::s63_cmd::enc_dirs(app) {
        if d.is_dir() {
            m.extend(texts::find_ancillary(&d));
        }
    }
    if let Ok(rd) = std::fs::read_dir(store_dir(app)) {
        for e in rd.flatten() {
            m.insert(e.file_name().to_string_lossy().to_ascii_uppercase(), e.path());
        }
    }
    m
}

/// Copy the notes of the cells under `from` into the store. (texts, pictures)
pub fn import_from(app: &tauri::AppHandle, from: &Path) -> Result<(usize, usize), String> {
    let n = texts::copy_ancillary(from, &store_dir(app)).map_err(|e| e.to_string())?;
    *INDEX.lock().unwrap() = None;
    *PICTURES.lock().unwrap() = None;
    Ok(n)
}

/// How many notes are in the store.
fn stored(app: &tauri::AppHandle) -> usize {
    std::fs::read_dir(store_dir(app)).map(|rd| rd.flatten().count()).unwrap_or(0)
}

/// A note by its file name (as the feature gives it): `{ name, text }`; for a picture
/// (PICREP, TIFF in ENC sets) `{ name, picture: "data:image/png;base64,…", width, height }`, or
/// `{ name, picture: null, error }` when it can't be read; null when TweedPlot doesn't have it.
#[tauri::command]
pub async fn chart_text(app: tauri::AppHandle, name: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = name.trim().to_ascii_uppercase();
        // a bare name only, never a path
        if key.is_empty() || key.contains(['/', '\\']) || key.contains("..") {
            return Value::Null;
        }
        let path = {
            let mut g = INDEX.lock().unwrap();
            g.get_or_insert_with(|| build_index(&app)).get(&key).cloned()
        };
        let Some(path) = path else { return Value::Null };
        if !key.ends_with(".TXT") {
            if let Some(v) = PICTURES.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
                return v.clone();
            }
            let v = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| picture_reply(&b)) {
                Ok((url, w, h)) => json!({ "name": key, "picture": url, "width": w, "height": h }),
                Err(e) => return json!({ "name": key, "picture": null, "error": e }),
            };
            PICTURES.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, v.clone());
            return v;
        }
        match std::fs::read(&path) {
            Ok(b) => json!({ "name": key, "text": texts::decode_text(&b) }),
            Err(_) => Value::Null,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// A picture file as a data URL for an <img>, with its size (0 × 0 when passed through).
fn picture_reply(b: &[u8]) -> Result<(String, u32, u32), String> {
    let (bytes, mime) = picture::picture_for_display(b).map_err(|e| e.to_string())?;
    let (w, h) = if mime == "image/png" && bytes.len() > 24 {
        (u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]), u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]))
    } else {
        (0, 0)
    };
    Ok((format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)), w, h))
}

/// How many chart notes TweedPlot has (Chart › Chart notes).
#[tauri::command]
pub async fn chart_texts_status(app: tauri::AppHandle) -> Value {
    json!({ "stored": stored(&app), "folder": store_dir(&app) })
}

/// Choose an ENC folder (an exchange set, a chart CD, another program's ENC folder) and copy
/// the notes kept with its cells.
#[tauri::command]
pub async fn chart_texts_import(app: tauri::AppHandle) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let a = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        a.dialog().file().set_title("Choose an ENC folder (an exchange set, or the folder with ENC_ROOT in it)").blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(json!({ "cancelled": true })) };
    let from = picked.into_path().map_err(|e| e.to_string())?;
    let a = app.clone();
    let (t, p) = tauri::async_runtime::spawn_blocking(move || import_from(&a, &from)).await.map_err(|e| e.to_string())??;
    Ok(json!({ "texts": t, "pictures": p, "stored": stored(&app) }))
}
