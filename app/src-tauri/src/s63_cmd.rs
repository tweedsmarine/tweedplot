//! Chart › Encrypted charts (S-63): the system's user permit, cell permits and
//! importing exchange sets.
//!
//! Until TweedPlot has its own manufacturer ID and key from the IHO it uses the IHO's
//! generic S-64 test credentials: the test user permit, unless another is entered.

use std::path::PathBuf;

use tauri::Manager;

const CONFIG: &str = "s63.json";

fn config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(CONFIG))
}

/// Set the saved user permit (at start).
pub fn load(app: &tauri::AppHandle) {
    let saved = config_path(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let up = saved.as_ref().and_then(|v| v["user_permit"].as_str()).unwrap_or("").trim().to_string();
    if !up.is_empty() && chart::enc::s63::hw_id_from_user_permit(&up, chart::enc::TEST_M_KEY).is_ok() {
        chart::enc::set_protection(&up, chart::enc::TEST_M_KEY);
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
pub fn s63_status(app: tauri::AppHandle) -> serde_json::Value {
    let (up, _) = chart::enc::protection();
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
    let notes: Vec<serde_json::Value> = chart::enc::notes().into_iter().map(|n| serde_json::json!({ "cell": n.cell, "file": n.file, "text": n.text })).collect();
    serde_json::json!({
        "user_permit": up,
        "test": up == chart::enc::TEST_USER_PERMIT,
        "hw_ok": chart::enc::hw_id().is_some(),
        "key": s.key().map(|k| serde_json::json!({ "name": k.name, "sa": k.sa, "not_after": k.not_after, "subject": k.subject })),
        "enc_dir": enc_home(&app),
        "permits": permits,
        "cells": cells,
        "notes": notes,
    })
}

/// Use another user permit (empty: back to the IHO test permit) and reopen the charts.
#[tauri::command]
pub fn s63_set_user_permit(app: tauri::AppHandle, permit: String) -> Result<String, String> {
    let p = permit.trim().to_ascii_uppercase();
    let p = if p.is_empty() { chart::enc::TEST_USER_PERMIT.to_string() } else { p };
    chart::enc::s63::hw_id_from_user_permit(&p, chart::enc::TEST_M_KEY).map_err(|e| e.to_string())?;
    chart::enc::set_protection(&p, chart::enc::TEST_M_KEY);
    if let Some(cfg) = config_path(&app) {
        let _ = std::fs::create_dir_all(cfg.parent().unwrap());
        let keep = if p == chart::enc::TEST_USER_PERMIT { "" } else { p.as_str() };
        std::fs::write(&cfg, serde_json::json!({ "user_permit": keep }).to_string()).map_err(|e| format!("Could not save it: {e}"))?;
    }
    crate::open_charts(app);
    Ok(p)
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
    let picked = tauri::async_runtime::spawn_blocking(move || a.dialog().file().set_title("Choose the exchange set (the folder with ENC_ROOT in it, or the medium)").blocking_pick_folder())
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
