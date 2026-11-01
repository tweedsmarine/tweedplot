//! TweedPlot Licence Manager window (Tauri). The work is in the `licence` library.
//!
//! Run on the PC that holds the OEM folder: `cargo run --release -p tweedplot-licence`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;

use licence::{Customer, IssueRequest, Vault};
use tauri_plugin_dialog::DialogExt;

static VAULT: Mutex<Option<Vault>> = Mutex::new(None);

fn mode_file() -> PathBuf {
    licence::oem_dir().join("mode.json")
}

fn test_mode() -> bool {
    std::fs::read_to_string(mode_file()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v["test"].as_bool()).unwrap_or(true)
}

fn with<T>(f: impl FnOnce(&mut Vault) -> Result<T, String>) -> Result<T, String> {
    let mut g = VAULT.lock().map_err(|e| e.to_string())?;
    if g.is_none() {
        *g = Some(Vault::open(&licence::oem_dir(), test_mode())?);
    }
    f(g.as_mut().unwrap())
}

#[tauri::command]
fn lm_status() -> Result<licence::Status, String> {
    with(|v| Ok(v.status()))
}

#[tauri::command]
fn lm_set_mode(test: bool) -> Result<licence::Status, String> {
    let _ = std::fs::create_dir_all(licence::oem_dir());
    std::fs::write(mode_file(), serde_json::json!({ "test": test }).to_string()).map_err(|e| e.to_string())?;
    let v = Vault::open(&licence::oem_dir(), test)?;
    let st = v.status();
    *VAULT.lock().map_err(|e| e.to_string())? = Some(v);
    Ok(st)
}

#[tauri::command]
fn lm_set_credentials(m_id: String, m_key: String) -> Result<(), String> {
    with(|v| v.set_credentials(&m_id, &m_key))
}

#[tauri::command]
fn lm_import_old() -> Result<Vec<String>, String> {
    with(|v| v.import_old_files())
}

#[tauri::command]
fn lm_licences() -> Result<Vec<licence::Licence>, String> {
    with(|v| Ok(v.licences()))
}

#[tauri::command]
fn lm_licence(id: i64) -> Result<serde_json::Value, String> {
    with(|v| {
        let l = v.licence(id)?;
        Ok(serde_json::json!({ "licence": l, "log": v.log_entries(Some(id), 200) }))
    })
}

#[tauri::command]
fn lm_customers() -> Result<Vec<Customer>, String> {
    with(|v| Ok(v.customers()))
}

#[tauri::command]
fn lm_code_info(code: String) -> Result<licence::CodeInfo, String> {
    with(|v| v.code_info(&code))
}

#[tauri::command]
fn lm_issue(req: IssueRequest) -> Result<licence::Issued, String> {
    with(|v| v.issue(&req))
}

#[tauri::command]
fn lm_release(code: String) -> Result<String, String> {
    with(|v| v.release(&code))
}

#[tauri::command]
fn lm_end_seat(seat_id: i64, why: String) -> Result<(), String> {
    with(|v| v.end_seat(seat_id, &why))
}

#[tauri::command]
fn lm_revoke(id: i64, why: String) -> Result<(), String> {
    with(|v| v.revoke(id, &why))
}

#[tauri::command]
fn lm_update_licence(id: i64, vessel: String, mmsi: String, call_sign: String, seats: u32, notes: String) -> Result<(), String> {
    with(|v| v.update_licence(id, &vessel, &mmsi, &call_sign, seats, &notes))
}

#[tauri::command]
fn lm_update_customer(customer: Customer) -> Result<(), String> {
    with(|v| v.update_customer(&customer))
}

#[tauri::command]
fn lm_log(limit: u32) -> Result<Vec<licence::LogEntry>, String> {
    with(|v| Ok(v.log_entries(None, limit)))
}

fn save_path(app: &tauri::AppHandle, title: &str, name: &str, ext: (&str, &str)) -> Option<PathBuf> {
    app.dialog().file().set_title(title).set_file_name(name).add_filter(ext.0, &[ext.1]).blocking_save_file().and_then(|p| p.into_path().ok())
}

#[tauri::command]
async fn lm_export_register(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let csv = with(|v| Ok(v.register_csv()))?;
    let name = format!("tweedplot-hwid-register-{}.csv", chrono::Utc::now().format("%Y-%m-%d"));
    let Some(p) =
        tauri::async_runtime::spawn_blocking(move || save_path(&app, "Save the HW_ID register", &name, ("CSV", "csv"))).await.map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    std::fs::write(&p, csv).map_err(|e| e.to_string())?;
    Ok(Some(p.display().to_string()))
}

#[tauri::command]
async fn lm_export_backup(app: tauri::AppHandle, pass: String) -> Result<Option<String>, String> {
    let name = format!("tweedplot-licences-{}.tplb", chrono::Utc::now().format("%Y-%m-%d"));
    let Some(p) = tauri::async_runtime::spawn_blocking(move || save_path(&app, "Save the passphrase backup", &name, ("Licence Manager backup", "tplb")))
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let p2 = p.clone();
    tauri::async_runtime::spawn_blocking(move || with(|v| v.export_backup(&p2, &pass))).await.map_err(|e| e.to_string())??;
    Ok(Some(p.display().to_string()))
}

#[tauri::command]
async fn lm_restore_backup(app: tauri::AppHandle, pass: String) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose the passphrase backup")
            .add_filter("Licence Manager backup", &["tplb"])
            .blocking_pick_file()
            .and_then(|p| p.into_path().ok())
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(p) = picked else { return Ok(None) };
    let p2 = p.clone();
    tauri::async_runtime::spawn_blocking(move || with(|v| v.restore_backup(&p2, &pass))).await.map_err(|e| e.to_string())??;
    Ok(Some(p.display().to_string()))
}

/// Open a new email in the mail program (mailto:).
#[tauri::command]
fn lm_mail(to: String, subject: String, body: String) -> Result<(), String> {
    let enc = |s: &str| {
        s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~@".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>()
    };
    if to.chars().any(|c| c.is_control() || c == '"' || c == ' ') {
        return Err("that email address has spaces or odd characters in it".into());
    }
    let url = format!("mailto:{}?subject={}&body={}", enc(&to), enc(&subject), enc(&body));
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &url]).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            lm_status,
            lm_set_mode,
            lm_set_credentials,
            lm_import_old,
            lm_licences,
            lm_licence,
            lm_customers,
            lm_code_info,
            lm_issue,
            lm_release,
            lm_end_seat,
            lm_revoke,
            lm_update_licence,
            lm_update_customer,
            lm_log,
            lm_export_register,
            lm_export_backup,
            lm_restore_backup,
            lm_mail
        ])
        .run(tauri::generate_context!())
        .expect("TweedPlot Licence Manager");
}
