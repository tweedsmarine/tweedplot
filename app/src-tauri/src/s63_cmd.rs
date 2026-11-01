//! Chart › Encrypted charts (S-63): activation (this computer's user permit), the SA
//! key, cell permits and importing exchange sets.
//!
//! Activation: the app shows this installation's code (computer fingerprint + device
//! key); Tweedsmarine's Licence Manager turns it into an activation code (TPA2-) holding
//! the user permit, the vessel it is licensed to and the HW_ID encrypted to this
//! installation. Removing the activation gives a release code for the Licence Manager,
//! which frees the seat. TweedPlot holds no M_KEY. A build without TweedPlot's own
//! credentials uses the IHO's generic S-64 test permit until activated.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chart::enc::{device::Device, machine, oem};
use tauri::Manager;

const CONFIG: &str = "s63.json";

fn config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(CONFIG))
}

fn read_config(app: &tauri::AppHandle) -> serde_json::Value {
    config_path(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}))
}

fn write_config(app: &tauri::AppHandle, v: &serde_json::Value) -> Result<(), String> {
    let cfg = config_path(app).ok_or("no app data folder")?;
    let _ = std::fs::create_dir_all(cfg.parent().unwrap());
    std::fs::write(&cfg, v.to_string()).map_err(|e| e.to_string())
}

fn saved_activation(app: &tauri::AppHandle) -> Option<String> {
    read_config(app)["activation"].as_str().map(str::to_string).filter(|s| !s.trim().is_empty())
}

/// This installation's device key: made on first use and kept in the local app data
/// folder (DPAPI, machine scope, on Windows), so it never roams to another computer.
static DEVICE: OnceLock<Result<Device, String>> = OnceLock::new();

fn device(app: &tauri::AppHandle) -> Result<&'static Device, String> {
    DEVICE
        .get_or_init(|| {
            let dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
            let d = Device::load_or_create(&dir.join("device.key"))?;
            chart::enc::set_device(&d);
            Ok(d)
        })
        .as_ref()
        .map_err(|e| format!("This installation's device key can't be used: {e}"))
}

/// The activation in use (what it grants; the HW_ID is only in the chart crate).
static GRANT: Mutex<Option<oem::Grant>> = Mutex::new(None);

/// Check an activation code for this computer and installation, and use it.
fn apply(app: &tauri::AppHandle, code: &str) -> Result<oem::Activation, String> {
    let a = oem::read_activation(code).map_err(|e| e.to_string())?;
    let hw = a.open(&machine::fingerprint(), device(app)?).map_err(|e| e.to_string())?;
    chart::enc::set_protection(&a.grant.user_permit, &hw);
    *GRANT.lock().unwrap() = Some(a.grant.clone());
    Ok(a)
}

/// Use the saved activation (at start, before the charts open). Works out this
/// computer's fingerprint, which takes a second or so on Windows.
pub fn load(app: &tauri::AppHandle) {
    if let Err(e) = device(app) {
        eprintln!("S-63: {e}");
    }
    if let Some(code) = saved_activation(app) {
        if let Err(e) = apply(app, &code) {
            eprintln!("S-63 activation: {e}");
        }
    }
}

/// Where permits are installed and exchange sets copied to: the enc folder of the
/// developer's data folder, else `<app data>\enc`.
pub fn enc_home(app: &tauri::AppHandle) -> PathBuf {
    let dev = PathBuf::from(r"C:\dev\tweedplot-data");
    if dev.is_dir() {
        return dev.join("enc");
    }
    app.path().app_data_dir().map(|p| p.join("enc")).unwrap_or_else(|_| PathBuf::from("enc"))
}

/// Every folder ENC cells are read from.
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

fn days_between(a: &str, b: &str) -> i64 {
    let d = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y%m%d").ok();
    match (d(a), d(b)) {
        (Some(a), Some(b)) => (b - a).num_days(),
        _ => -1,
    }
}

fn senc(app: &tauri::AppHandle) -> chart::enc::senc::Senc {
    let dir = enc_home(app);
    let _ = std::fs::create_dir_all(&dir);
    chart::enc::senc::Senc::open(&dir)
}

/// The user permit, the SA key, the installed permits (by data server) and cells, and
/// the messages from opening the charts.
#[tauri::command]
pub async fn s63_status(app: tauri::AppHandle) -> serde_json::Value {
    tauri::async_runtime::spawn_blocking(move || status(&app)).await.unwrap_or_default()
}

fn status(app: &tauri::AppHandle) -> serde_json::Value {
    let app = app.clone();
    let up = chart::enc::user_permit();
    let dev = device(&app);
    let activation = saved_activation(&app).map(|code| {
        let r = oem::read_activation(&code).map_err(|e| e.to_string()).and_then(|a| match &dev {
            Ok(d) => a.open(&machine::fingerprint(), d).map(|_| a).map_err(|e| e.to_string()),
            Err(e) => Err(e.clone()),
        });
        match r {
            Ok(a) => serde_json::json!({
                "ok": true, "issued": a.grant.issued, "vessel": a.grant.vessel, "mmsi": a.grant.mmsi,
                "licence": a.grant.licence, "seat": a.grant.seat,
            }),
            Err(e) => serde_json::json!({ "ok": false, "error": e }),
        }
    });
    let own_mmsi = crate::settings::settings_get(app.clone()).map(|s| s.vessel.mmsi).unwrap_or_default();
    let cfg = read_config(&app);
    let s = senc(&app);
    let today = chart::enc::today();
    let mut permits = Vec::new();
    for (ds, list) in s.permits() {
        for (cell, p) in list {
            permits.push(match p {
                Ok(p) => {
                    let days = days_between(&today, &p.expiry);
                    let (state, text) = if days < 0 {
                        ("expired", "SSE 15 - Subscription service has expired. Please contact your data supplier to renew the subscription licence".to_string())
                    } else if days <= 30 {
                        ("expiring", format!("SSE 20 - Subscription service will expire in less than 30 days ({days} days)"))
                    } else {
                        ("ok", String::new())
                    };
                    serde_json::json!({ "ds": ds, "cell": cell, "expiry": p.expiry, "state": state, "text": text, "service": if p.service == 1 { "single purchase" } else { "subscription" } })
                }
                Err(e) => serde_json::json!({ "ds": ds, "cell": cell, "state": "bad", "text": e.to_string() }),
            });
        }
    }
    let shown: std::collections::HashMap<String, String> = s.display_notes().into_iter().collect();
    let cells: Vec<serde_json::Value> = s
        .cells()
        .iter()
        .map(|(c, i)| serde_json::json!({ "cell": c, "ds": i.ds, "edition": i.edition, "update": i.update, "encrypted": i.encrypted, "warn": shown.get(c), "bbox": s.product(c).and_then(|p| p.bbox) }))
        .collect();
    let notes: Vec<serde_json::Value> =
        chart::enc::notes().into_iter().map(|n| serde_json::json!({ "cell": n.cell, "file": n.file, "text": n.text })).collect();
    serde_json::json!({
        "user_permit": up,
        "test": up == chart::enc::TEST_USER_PERMIT,
        "real_credentials": oem::is_real(),
        "installation_code": dev.as_ref().map(|d| oem::installation_code(&machine::fingerprint(), &d.public)).unwrap_or_default(),
        "device_error": dev.as_ref().err(),
        "device_protected": dev.as_ref().map(|d| d.protected).unwrap_or(false),
        "activation": activation,
        "own_mmsi": own_mmsi,
        "released": cfg.get("released"),
        "hw_ok": chart::enc::hw_id().is_some(),
        "key": s.key().map(|k| serde_json::json!({ "name": k.name, "sa": k.sa, "not_after": k.not_after, "subject": k.subject })),
        "enc_dir": enc_home(&app),
        "permits": permits,
        "cells": cells,
        "notes": notes,
    })
}

/// Activate with a code from Tweedsmarine: saved, used now, and the charts reopened.
#[tauri::command]
pub async fn s63_activate(app: tauri::AppHandle, code: String) -> Result<String, String> {
    let code = code.split_whitespace().collect::<String>();
    let (c, a2) = (code.clone(), app.clone());
    let a = tauri::async_runtime::spawn_blocking(move || apply(&a2, &c)).await.map_err(|e| e.to_string())??;
    let mut cfg = read_config(&app);
    cfg["activation"] = serde_json::json!(code);
    write_config(&app, &cfg).map_err(|e| format!("Activated, but it couldn't be saved: {e}"))?;
    crate::open_charts(app);
    Ok(a.grant.user_permit)
}

/// Remove the activation: the HW_ID is forgotten, the sealed chart data made with it is
/// deleted, and a release code is returned (and kept) for Tweedsmarine to free the seat.
/// A test build goes back to the IHO test permit.
#[tauri::command]
pub fn s63_deactivate(app: tauri::AppHandle) -> Option<String> {
    let grant = GRANT.lock().unwrap().take();
    let today = chrono::Utc::now().format("%Y%m%d").to_string();
    let release = match (&grant, chart::enc::hw_id()) {
        (Some(g), Some(hw)) => Some(oem::release_code(g, &hw, &today)),
        _ => None,
    };
    let mut cfg = read_config(&app);
    if let Some(obj) = cfg.as_object_mut() {
        obj.remove("activation");
    }
    if let (Some(code), Some(g)) = (&release, &grant) {
        cfg["released"] = serde_json::json!({ "code": code, "date": today, "vessel": g.vessel, "licence": g.licence, "seat": g.seat });
    }
    let _ = write_config(&app, &cfg);
    chart::enc::clear_protection();
    // the chart data decrypted under this activation
    for dir in enc_dirs(&app) {
        for e in std::fs::read_dir(dir.join(".enc")).into_iter().flatten().flatten() {
            if chart::enc::is_sealed(&e.path()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    crate::open_charts(app);
    release
}

/// Install the permits in a PERMIT.TXT (its text). Returns what happened to each.
#[tauri::command]
pub fn s63_install_permits(app: tauri::AppHandle, text: String) -> Vec<chart::enc::senc::Msg> {
    let r = senc(&app).install_permits(&text);
    crate::open_charts(app);
    r
}

/// Remove one data server's permits.
#[tauri::command]
pub fn s63_remove_permits(app: tauri::AppHandle, ds: String) -> bool {
    let done = senc(&app).remove_permits(&ds);
    crate::open_charts(app);
    done
}

/// Install an SA certificate (IHO.CRT) or public key (IHO.PUB) in place of the one installed.
#[tauri::command]
pub fn s63_install_key(app: tauri::AppHandle, name: String, bytes: Vec<u8>) -> Vec<chart::enc::senc::Msg> {
    senc(&app).install_key(&name, &bytes)
}

/// Pick an exchange set's folder (with ENC_ROOT in it, or a medium with several) and
/// import it. Returns the report and reopens the charts.
#[tauri::command]
pub async fn s63_import_exchange_set(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    use tauri::Emitter;
    use tauri_plugin_dialog::DialogExt;
    let a = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        a.dialog().file().set_title("Choose the exchange set (the folder with ENC_ROOT in it, or the medium)").blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(serde_json::json!({ "cancelled": true })) };
    let from = picked.into_path().map_err(|e| e.to_string())?;
    let a = app.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        let mut s = senc(&a);
        let mut last = std::time::Instant::now();
        s.import(&from, &mut |done, total, cell| {
            if last.elapsed().as_millis() >= 300 {
                last = std::time::Instant::now();
                let _ = a.emit("chart-status", serde_json::json!({ "detail": format!("importing ENC cells: {done} of {total} {cell}") }));
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    crate::open_charts(app);
    Ok(serde_json::json!({ "report": report }))
}

/// Remove installed cells: one, or all of them when `cell` is empty.
#[tauri::command]
pub fn s63_remove_cells(app: tauri::AppHandle, cell: String) {
    let mut s = senc(&app);
    if cell.is_empty() {
        s.remove_cells();
    } else {
        s.remove_cell(&cell);
    }
    crate::open_charts(app);
}
