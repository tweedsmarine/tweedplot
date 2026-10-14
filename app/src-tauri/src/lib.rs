//! racenav desktop app: wires the Rust crates to the MapLibre front end.
//!
//! Events sent to the front end:
//! - `sk-status`  `{ connected, detail }`
//! - `vessel`     own-vessel state (degrees true, knots, metres)

use std::time::Duration;

use tauri::{Emitter, Manager};

/// Signal K URL: `RACENAV_SK_URL` environment variable, else the local server.
fn sk_url() -> String {
    std::env::var("RACENAV_SK_URL").unwrap_or_else(|_| signalk::DEFAULT_URL.to_string())
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> serde_json::Value {
    let log_dir = app.path().app_data_dir().ok().map(|p| p.join("logs"));
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "signalk_url": sk_url(),
        "log_dir": log_dir,
    })
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .setup(|app| {
            let handle = app.handle().clone();
            let log_dir = app.path().app_data_dir().ok().map(|p| p.join("logs"));
            let opts = signalk::Options {
                url: sk_url(),
                log_dir,
                min_interval: Duration::from_millis(250),
            };
            let (tx, mut rx) = tokio::sync::mpsc::channel(256);
            tauri::async_runtime::spawn(signalk::run(opts, tx));
            tauri::async_runtime::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    let _ = match ev {
                        signalk::Event::Status { connected, detail } => handle.emit(
                            "sk-status",
                            serde_json::json!({ "connected": connected, "detail": detail }),
                        ),
                        signalk::Event::Vessel(v) => handle.emit("vessel", v),
                    };
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running racenav");
}
