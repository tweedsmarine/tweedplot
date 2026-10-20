//! Chart › Encrypted charts (S-63): the system's user permit, cell permits and
//! importing exchange sets.
//!
//! Until TweedPlot has its own manufacturer ID and key from the IHO it uses the IHO's
//! generic S-64 test credentials: the test user permit, unless another is entered.

use std::path::{Path, PathBuf};

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

/// The user permit in use, every cell permit found (the newest for each cell) and what
/// happened when the cells were last converted.
#[tauri::command]
pub fn s63_status(app: tauri::AppHandle) -> serde_json::Value {
    let (up, _) = chart::enc::protection();
    let test = up == chart::enc::TEST_USER_PERMIT;
    let today = chrono::Utc::now().format("%Y%m%d").to_string();
    let mut found: Vec<chart::enc::FoundPermit> = enc_dirs(&app).iter().filter(|d| d.is_dir()).flat_map(|d| chart::enc::find_permits(d)).collect();
    // newest permit file last, so it wins
    found.sort_by(|a, b| a.date.cmp(&b.date));
    let mut by_cell: std::collections::BTreeMap<String, serde_json::Value> = std::collections::BTreeMap::new();
    for f in found {
        let file = f.file.display().to_string();
        let v = match f.permit {
            Ok(p) => {
                let days = days_between(&today, &p.expiry);
                let (state, text) = if days < 0 {
                    ("expired", "SSE 15 - Subscription service has expired. Please contact your data supplier to renew the subscription licence".to_string())
                } else if days <= 30 {
                    ("expiring", format!("SSE 20 - Subscription service will expire in less than 30 days ({days} days)"))
                } else {
                    ("ok", String::new())
                };
                serde_json::json!({ "cell": f.cell, "expiry": p.expiry, "edition": p.edition, "server": p.data_server,
                    "service": if p.service == 1 { "single purchase" } else { "subscription" }, "state": state, "text": text, "file": file, "date": f.date })
            }
            Err(e) => {
                // an unusable permit doesn't replace a good one for the same cell
                if by_cell.get(&f.cell).is_some_and(|v| v["state"] != "bad") {
                    continue;
                }
                serde_json::json!({ "cell": f.cell, "state": "bad", "text": e.to_string(), "file": file, "date": f.date })
            }
        };
        by_cell.insert(f.cell, v);
    }
    let notes: Vec<serde_json::Value> = chart::enc::notes().into_iter().map(|n| serde_json::json!({ "cell": n.cell, "file": n.file, "text": n.text })).collect();
    serde_json::json!({
        "user_permit": up,
        "test": test,
        "hw_ok": chart::enc::hw_id().is_some(),
        "enc_dir": enc_home(&app),
        "permits": by_cell.into_values().collect::<Vec<_>>(),
        "notes": notes,
    })
}

fn days_between(a: &str, b: &str) -> i64 {
    let d = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y%m%d").ok();
    match (d(a), d(b)) {
        (Some(a), Some(b)) => (b - a).num_days(),
        _ => -1,
    }
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

/// Install the permits in a PERMIT.TXT (its text) and reopen the charts. Returns what
/// happened to each permit.
#[tauri::command]
pub fn s63_install_permits(app: tauri::AppHandle, text: String) -> Result<Vec<serde_json::Value>, String> {
    let dir = enc_home(&app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let report = chart::enc::install_permits(&dir, &text).map_err(|e| e.to_string())?;
    crate::open_charts(app);
    Ok(report.into_iter().map(|(cell, what)| serde_json::json!({ "cell": cell, "text": what })).collect())
}

/// Pick an exchange set's folder (with ENC_ROOT, or a CD's folder of them), copy it into
/// the enc folder and reopen the charts.
#[tauri::command]
pub async fn s63_import_exchange_set(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let a = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || a.dialog().file().set_title("Choose the exchange set folder (the one with ENC_ROOT in it)").blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(serde_json::json!({ "cancelled": true })) };
    let from = picked.into_path().map_err(|e| e.to_string())?;
    let home = enc_home(&app);
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || import(&from, &home)).await.map_err(|e| e.to_string())??;
    crate::open_charts(a);
    Ok(r)
}

fn import(from: &Path, home: &Path) -> Result<serde_json::Value, String> {
    // ENC_ROOT itself picked: take its parent, which holds SERIAL.ENC and the PERMIT.TXT
    let from = if from.file_name().is_some_and(|n| n.eq_ignore_ascii_case("ENC_ROOT")) { from.parent().unwrap_or(from) } else { from };
    let cells = chart::enc::find_cells(from);
    if cells.is_empty() {
        return Err(format!("No ENC cells (.000 files) in {}", from.display()));
    }
    if home.starts_with(from) || from.starts_with(home) {
        return Err("That folder is TweedPlot's own ENC folder (or holds it).".into());
    }
    let serial = std::fs::read_to_string(from.join("SERIAL.ENC")).ok();
    let name = from.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "exchange set".into());
    let to = home.join(&name);
    let mut files = 0usize;
    copy_dir(from, &to, &mut files).map_err(|e| format!("Copying to {}: {e}", to.display()))?;
    let encrypted = cells.iter().filter(|c| std::fs::read(&c.base).is_ok_and(|b| !(b.len() > 24 && b[..5].iter().all(u8::is_ascii_digit) && b[6] == b'L'))).count();
    Ok(serde_json::json!({
        "to": to,
        "files": files,
        "cells": cells.len(),
        "encrypted": encrypted,
        "permit_file": from.join("PERMIT.TXT").is_file() || from.join("ENC_ROOT").join("PERMIT.TXT").is_file(),
        "serial": serial.map(|s| serial_info(&s)),
    }))
}

/// SERIAL.ENC (S-63 5.4.1): data server, week, date, base or update, format version, media.
fn serial_info(s: &str) -> serde_json::Value {
    let g = |a: usize, b: usize| s.get(a..b).unwrap_or("").trim().to_string();
    serde_json::json!({ "server": g(0, 2), "week": g(2, 12), "date": g(12, 20), "kind": g(20, 30), "version": g(30, 35), "media": g(35, 41) })
}

fn copy_dir(from: &Path, to: &Path, files: &mut usize) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t, files)?;
        } else {
            std::fs::copy(&p, &t)?;
            *files += 1;
        }
    }
    Ok(())
}
