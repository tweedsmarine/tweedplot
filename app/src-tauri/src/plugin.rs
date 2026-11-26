//! The TweedPlot S-63 Charts plugin: finding it, checking its signature, starting it, and
//! letting the chart store read its tables.
//!
//! Encrypted (S-63) charts are handled entirely by the plugin, a separate program from
//! Tweedsmarine (not part of this GPL app). It is installed from a signed package
//! (`.tpplugin`) into `<local app data>\plugins\s63-charts`, and keeps its own keys and
//! activation in `plugins\s63-charts-data`. Each start checks the package signature
//! against the release keys in `plugin-release.pub`, then talks to the program over its
//! pipes (`plugin_api`).
//!
//! For development, `TWEEDPLOT_CHARTS_PLUGIN=<path to tweedplot-charts.exe>` starts that
//! program instead, unchecked (shown as a development plugin).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use plugin_api::protocol::{FeatureQuery, RowsQuery};
use plugin_api::{PluginClient, Row};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

pub const NAME: &str = "TweedPlot S-63 Charts";
/// Where the current plugin packages are listed (`IndexEntry` per platform).
const INDEX_URL: &str = "https://tweedsmarine.nz/tweedplot/plugins/s63-charts/index.json";
/// Tweedsmarine's release keys for plugin packages (hex Ed25519, one per line).
const RELEASE_KEYS: &str = include_str!("../plugin-release.pub");

const QUICK: Duration = Duration::from_secs(30);
/// Importing a big exchange set or converting thousands of cells.
const LONG: Duration = Duration::from_secs(60 * 60);

pub struct Plugin {
    pub client: Arc<PluginClient>,
    pub version: String,
    pub dev: bool,
}

static PLUGIN: Mutex<Option<Arc<Plugin>>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn release_keys() -> Vec<[u8; 32]> {
    RELEASE_KEYS
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| plugin_api::package::unhex(l)?.try_into().ok())
        .collect()
}

fn base(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_local_data_dir().ok().map(|d| d.join("plugins"))
}

pub fn install_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    base(app).map(|b| b.join("s63-charts"))
}

fn data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    base(app).map(|b| b.join("s63-charts-data"))
}

fn dev_exe() -> Option<PathBuf> {
    std::env::var_os("TWEEDPLOT_CHARTS_PLUGIN").map(PathBuf::from).filter(|p| p.is_file())
}

/// Whether a plugin is installed (or given for development).
pub fn installed(app: &tauri::AppHandle) -> bool {
    dev_exe().is_some() || install_dir(app).is_some_and(|d| d.join("manifest.json").is_file())
}

/// The installed package's manifest, checked.
fn verified(app: &tauri::AppHandle) -> Result<(plugin_api::package::Manifest, PathBuf), String> {
    let dir = install_dir(app).ok_or("no app data folder")?;
    let mut last = String::from("no release key in this build");
    for k in release_keys() {
        match plugin_api::package::verify_installed(&dir, &k) {
            Ok(v) => return Ok(v),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn start(app: &tauri::AppHandle) -> Result<Arc<Plugin>, String> {
    let (exe, version, dev) = match dev_exe() {
        Some(p) => (p, "development".to_string(), true),
        None => {
            let (m, exe) = verified(app)?;
            if m.api != plugin_api::API_VERSION {
                return Err(format!(
                    "{NAME} {} is for plugin interface {}, this TweedPlot {}: update {}",
                    m.version,
                    m.api,
                    plugin_api::API_VERSION,
                    if m.api < plugin_api::API_VERSION { "the plugin" } else { "TweedPlot" }
                ));
            }
            (exe, m.version, false)
        }
    };
    let client = Arc::new(PluginClient::spawn(&exe, &[]).map_err(|e| format!("{NAME} couldn't be started: {e}"))?);
    let h = app.clone();
    client.on_event(move |name, data| {
        if name == "progress" {
            let what = if data["what"] == "import" { "importing ENC cells" } else { "decrypting ENC cells" };
            let (done, total) = (data["done"].as_u64().unwrap_or(0), data["total"].as_u64().unwrap_or(0));
            let cell = data["cell"].as_str().unwrap_or("");
            let detail = if total > 0 { format!("{what}: {done} of {total} {cell}") } else { format!("{what}: {cell}") };
            let _ = h.emit("chart-status", json!({ "detail": detail }));
        }
    });
    let enc = crate::s63_cmd::enc_home(app);
    let data = data_dir(app).ok_or("no app data folder")?;
    client
        .call("hello", json!({ "api": plugin_api::API_VERSION, "app_version": env!("CARGO_PKG_VERSION"), "data_dir": data, "enc_dir": enc }), QUICK)
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(Plugin { client, version, dev }))
}

/// The running plugin, started (or restarted after a crash) if it is installed.
pub fn get(app: &tauri::AppHandle) -> Option<Arc<Plugin>> {
    let mut g = PLUGIN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = g.as_ref() {
        if p.client.alive() {
            return Some(p.clone());
        }
        *g = None;
    }
    if !installed(app) {
        return None;
    }
    match start(app) {
        Ok(p) => {
            *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *g = Some(p.clone());
            Some(p)
        }
        Err(e) => {
            eprintln!("{NAME}: {e}");
            *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
            None
        }
    }
}

/// Stop the plugin (before replacing or removing it).
pub fn stop() {
    if let Some(p) = PLUGIN.lock().unwrap_or_else(|e| e.into_inner()).take() {
        p.client.stop();
    }
}

/// Call the plugin; an error if it isn't installed or running.
pub fn call(app: &tauri::AppHandle, method: &str, params: Value) -> Result<Value, String> {
    let p = get(app).ok_or_else(not_running)?;
    p.client.call(method, params, if matches!(method, "import" | "open") { LONG } else { QUICK }).map_err(|e| e.to_string())
}

/// [`call`] with a body.
pub fn request(app: &tauri::AppHandle, method: &str, params: Value, body: &[u8]) -> Result<Value, String> {
    let p = get(app).ok_or_else(not_running)?;
    p.client.request(method, params, body, QUICK).map(|(v, _)| v).map_err(|e| e.to_string())
}

fn not_running() -> String {
    LAST_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| format!("{NAME} isn't installed. Encrypted charts need it: get it in Chart › Encrypted charts"))
}

/// What the encrypted charts card shows about the plugin.
pub fn info(app: &tauri::AppHandle) -> Value {
    let running = PLUGIN.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|p| p.client.alive());
    let manifest = verified(app).ok().map(|(m, _)| m);
    json!({
        "name": NAME,
        "installed": installed(app),
        "running": running.is_some(),
        "version": running.as_ref().map(|p| p.version.clone()).or(manifest.map(|m| m.version)),
        "dev": running.as_ref().is_some_and(|p| p.dev) || dev_exe().is_some(),
        "error": LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        "platform": plugin_api::package::this_platform(),
    })
}

/// Install a package (from a file or a download), replacing any installed version.
pub fn install_package(app: &tauri::AppHandle, bytes: &[u8]) -> Result<String, String> {
    let dir = install_dir(app).ok_or("no app data folder")?;
    let _ = std::fs::create_dir_all(dir.parent().unwrap());
    let keys = release_keys();
    // check before stopping the one that is running
    let mut last = String::from("no release key in this build");
    let mut good = None;
    for k in &keys {
        match plugin_api::package::open(bytes, k) {
            Ok((m, ..)) => {
                good = Some((k, m));
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let (key, m) = good.ok_or(last)?;
    if m.platform != plugin_api::package::this_platform() {
        return Err(format!("That package is for {}, this computer is {}", m.platform, plugin_api::package::this_platform()));
    }
    stop();
    plugin_api::package::install(bytes, key, &dir).map_err(|e| e.to_string())?;
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(m.version)
}

/// Remove the installed plugin. Its data (activation, device key) stays, so installing it
/// again needs no new activation.
pub fn remove(app: &tauri::AppHandle) -> Result<(), String> {
    stop();
    if let Some(d) = install_dir(app) {
        if d.exists() {
            std::fs::remove_dir_all(&d).map_err(|e| e.to_string())?;
        }
    }
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

fn index_url() -> String {
    std::env::var("TWEEDPLOT_PLUGIN_INDEX").unwrap_or_else(|_| INDEX_URL.to_string())
}

/// The newest package for this computer on the download page.
pub fn latest() -> Result<plugin_api::package::IndexEntry, String> {
    let text = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .and_then(|c| c.get(index_url()).send())
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| format!("The plugin download page can't be reached: {e}"))?;
    let list: Vec<plugin_api::package::IndexEntry> = serde_json::from_str(&text).map_err(|e| format!("The plugin download list can't be read: {e}"))?;
    let me = plugin_api::package::this_platform();
    list.into_iter()
        .filter(|e| e.platform == me && e.api == plugin_api::API_VERSION)
        .max_by(|a, b| version_key(&a.version).cmp(&version_key(&b.version)))
        .ok_or_else(|| format!("No {NAME} download for {me} yet"))
}

/// "1.2.10" → [1, 2, 10], for comparing versions.
pub fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

/// Download and install a package from the index entry.
pub fn download(app: &tauri::AppHandle, e: &plugin_api::package::IndexEntry) -> Result<String, String> {
    let bytes = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .and_then(|c| c.get(&e.url).send())
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|err| format!("The download failed: {err}"))?;
    if plugin_api::package::sha256_hex(&bytes) != e.sha256.to_ascii_lowercase() {
        return Err("The download doesn't match the download list (damaged or changed): not installed".into());
    }
    install_package(app, &bytes)
}

/// The chart store's view of the plugin's tables.
pub struct Remote(pub Arc<PluginClient>);

impl Remote {
    fn warn(&self, what: &str, e: plugin_api::Error) {
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("{NAME}: {what}: {e}");
        }
    }
}

impl chart::remote::RemoteSource for Remote {
    fn rows(&self, db: u32, table: &str, bbox: [f64; 4], cols: &[&str]) -> Vec<Row> {
        let q = RowsQuery { db, table: table.into(), bbox, cols: cols.iter().map(|c| c.to_string()).collect() };
        match self.0.request("rows", serde_json::to_value(q).unwrap_or_default(), &[], QUICK) {
            Ok((_, body)) => plugin_api::rows::decode(&body).unwrap_or_default(),
            Err(e) => {
                self.warn("rows", e);
                vec![]
            }
        }
    }
    fn feature(&self, db: u32, table: &str, fid: i64, cols: &[&str]) -> Option<Row> {
        let q = FeatureQuery { db, table: table.into(), fid, cols: cols.iter().map(|c| c.to_string()).collect() };
        let (_, body) = self.0.request("feature", serde_json::to_value(q).ok()?, &[], QUICK).ok()?;
        plugin_api::rows::decode(&body).ok()?.into_iter().next()
    }
    fn median(&self, db: u32, table: &str, col: &str) -> Option<f64> {
        self.0.call("median", json!({ "db": db, "table": table, "col": col }), QUICK).ok()?["value"].as_f64()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions_compare() {
        assert!(super::version_key("1.2.10") > super::version_key("1.2.9"));
        assert!(super::version_key("2.0.0") > super::version_key("1.99.99"));
        assert!(!super::release_keys().is_empty(), "plugin-release.pub has a key");
    }
}
