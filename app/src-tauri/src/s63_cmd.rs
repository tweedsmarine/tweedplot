//! Chart › Encrypted charts (S-63). Everything S-63 is done by the TweedPlot S-63 Charts
//! plugin (see `plugin.rs`): activation, permits, the SA key, importing exchange sets,
//! decrypting. These commands pass the card's requests on to it, and manage the plugin
//! itself (install from a file, download, update, remove).

use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::Manager;

use crate::plugin;

/// Where encrypted charts are installed (the plugin's ENC store): the enc folder of the
/// developer's data folder, else `<app data>\enc`.
pub fn enc_home(app: &tauri::AppHandle) -> PathBuf {
    let dev = PathBuf::from(r"C:\dev\tweedplot-data");
    if dev.is_dir() {
        return dev.join("enc");
    }
    app.path().app_data_dir().map(|p| p.join("enc")).unwrap_or_else(|_| PathBuf::from("enc"))
}

/// Every folder plain ENC cells are read from.
pub fn enc_dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for root in [PathBuf::from(r"C:\dev\tweedplot-data"), PathBuf::from(r"C:\dev\racenav-data")] {
        v.push(root.join("enc"));
        v.push(root.join("s63"));
    }
    if let Ok(d) = app.path().app_data_dir() {
        v.push(d.join("enc"));
    }
    v.dedup();
    v
}

/// The chart-set path that stands for the plugin's charts (Chart data › Chart sets).
pub fn plugin_set_path(app: &tauri::AppHandle) -> PathBuf {
    enc_home(app).join(".senc")
}

/// Start the plugin at launch if it is installed, and tidy what TweedPlot versions
/// before the plugin left: their sealed conversions, which only the plugin can open now.
pub fn load(app: &tauri::AppHandle) {
    for d in enc_dirs(app) {
        for e in std::fs::read_dir(d.join(".enc")).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "tpx") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = plugin::get(app);
}

/// The plugin, and (when it runs) everything it says about activation, permits and cells.
#[tauri::command]
pub async fn s63_status(app: tauri::AppHandle) -> Value {
    tauri::async_runtime::spawn_blocking(move || {
        let mut st = if plugin::get(&app).is_some() { plugin::call(&app, "status", json!({})).unwrap_or_else(|e| json!({ "error": e })) } else { json!({}) };
        st["plugin"] = plugin::info(&app);
        // and the encrypted cells found in plain chart folders (not imported)
        let mut notes: Vec<Value> = st["notes"].as_array().cloned().unwrap_or_default();
        notes.extend(chart::enc::notes().into_iter().map(|n| json!({ "cell": n.cell, "file": n.file, "text": n.text })));
        st["notes"] = json!(notes);
        st["own_mmsi"] = json!(crate::settings::settings_get(app.clone()).map(|s| s.vessel.mmsi).unwrap_or_default());
        st
    })
    .await
    .unwrap_or_default()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Activate with a code from Tweedsmarine; the charts are opened again.
#[tauri::command]
pub async fn s63_activate(app: tauri::AppHandle, code: String) -> Result<String, String> {
    let a = app.clone();
    let r = blocking(move || plugin::call(&a, "activate", json!({ "code": code }))).await?;
    crate::open_charts(app);
    Ok(r["user_permit"].as_str().unwrap_or("").to_string())
}

/// Remove the activation: returns the release code for Tweedsmarine.
#[tauri::command]
pub async fn s63_deactivate(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let a = app.clone();
    let r = blocking(move || plugin::call(&a, "deactivate", json!({}))).await?;
    crate::open_charts(app);
    Ok(r["release_code"].as_str().map(str::to_string))
}

/// Install the permits in a PERMIT.TXT (its text). Returns what happened to each.
#[tauri::command]
pub async fn s63_install_permits(app: tauri::AppHandle, text: String) -> Result<Value, String> {
    let a = app.clone();
    let r = blocking(move || plugin::call(&a, "install_permits", json!({ "text": text }))).await?;
    crate::open_charts(app);
    Ok(r)
}

/// Remove one data server's permits.
#[tauri::command]
pub async fn s63_remove_permits(app: tauri::AppHandle, ds: String) -> Result<bool, String> {
    let a = app.clone();
    let r = blocking(move || plugin::call(&a, "remove_permits", json!({ "ds": ds }))).await?;
    crate::open_charts(app);
    Ok(r.as_bool().unwrap_or(false))
}

/// Install an SA certificate (IHO.CRT) or public key (IHO.PUB) in place of the one installed.
#[tauri::command]
pub async fn s63_install_key(app: tauri::AppHandle, name: String, bytes: Vec<u8>) -> Result<Value, String> {
    blocking(move || plugin::request(&app, "install_key", json!({ "name": name }), &bytes)).await
}

/// Pick an exchange set's folder (with ENC_ROOT in it, or a medium with several) and
/// import it. Returns the report and reopens the charts.
#[tauri::command]
pub async fn s63_import_exchange_set(app: tauri::AppHandle) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let a = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        a.dialog().file().set_title("Choose the exchange set (the folder with ENC_ROOT in it, or the medium)").blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(json!({ "cancelled": true })) };
    let from = picked.into_path().map_err(|e| e.to_string())?;
    let a = app.clone();
    let r = blocking(move || plugin::call(&a, "import", json!({ "path": from }))).await?;
    crate::open_charts(app);
    Ok(r)
}

/// Remove installed cells: one, or all of them when `cell` is empty.
#[tauri::command]
pub async fn s63_remove_cells(app: tauri::AppHandle, cell: String) -> Result<(), String> {
    let a = app.clone();
    blocking(move || plugin::call(&a, "remove_cells", json!({ "cell": cell }))).await?;
    crate::open_charts(app);
    Ok(())
}

/// Install the plugin from a package file (`.tpplugin`), e.g. from a USB stick.
#[tauri::command]
pub async fn s63_plugin_install_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let a = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        a.dialog().file().set_title(format!("Choose the {} package", plugin::NAME)).add_filter("TweedPlot plugin", &["tpplugin"]).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(p) = picked else { return Ok(None) };
    let path = p.into_path().map_err(|e| e.to_string())?;
    let a = app.clone();
    let v = blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        plugin::install_package(&a, &bytes)
    })
    .await?;
    crate::open_charts(app);
    Ok(Some(v))
}

/// The newest plugin on the download page, and whether it is newer than the one installed.
#[tauri::command]
pub async fn s63_plugin_check(app: tauri::AppHandle) -> Result<Value, String> {
    blocking(move || {
        let e = plugin::latest()?;
        let have = plugin::info(&app)["version"].as_str().map(str::to_string);
        let newer = have.as_deref().is_none_or(|h| plugin::version_key(&e.version) > plugin::version_key(h));
        Ok(json!({ "version": e.version, "newer": newer, "installed": have }))
    })
    .await
}

/// Download the newest plugin and install it.
#[tauri::command]
pub async fn s63_plugin_download(app: tauri::AppHandle) -> Result<String, String> {
    let a = app.clone();
    let v = blocking(move || {
        let e = plugin::latest()?;
        plugin::download(&a, &e)
    })
    .await?;
    crate::open_charts(app);
    Ok(v)
}

/// Remove the plugin (its activation stays, for when it is installed again).
#[tauri::command]
pub async fn s63_plugin_remove(app: tauri::AppHandle) -> Result<(), String> {
    let a = app.clone();
    blocking(move || plugin::remove(&a)).await?;
    crate::open_charts(app);
    Ok(())
}
