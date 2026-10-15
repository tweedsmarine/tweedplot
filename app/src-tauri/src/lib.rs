//! racenav desktop app: wires the Rust crates to the MapLibre front end.
//!
//! Events sent to the front end:
//! - `sk-status`   `{ connected, detail }`
//! - `vessel`      own-vessel state (degrees true, knots, metres)
//! - `chart-ready` `{ tables, dir }` once the chart folder has been opened
//!
//! The `chart` URI scheme serves vector tiles: `chart://localhost/{z}/{x}/{y}.pbf`
//! (`http://chart.localhost/...` on Windows).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tauri::http::{header, Response, StatusCode};
use tauri::{Emitter, Manager};

static CHARTS: OnceLock<Arc<chart::ChartStore>> = OnceLock::new();

/// Signal K URL: `RACENAV_SK_URL` environment variable, else the local server.
fn sk_url() -> String {
    std::env::var("RACENAV_SK_URL").unwrap_or_else(|_| signalk::DEFAULT_URL.to_string())
}

/// Chart folder: `RACENAV_CHART_DIR`, else `C:\dev\racenav-data\linz` if present,
/// else `<app data>\charts`.
fn chart_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_CHART_DIR") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(r"C:\dev\racenav-data\linz");
    if dev.is_dir() {
        return dev;
    }
    app.path().app_data_dir().map(|p| p.join("charts")).unwrap_or_else(|_| PathBuf::from("charts"))
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> serde_json::Value {
    let log_dir = app.path().app_data_dir().ok().map(|p| p.join("logs"));
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "signalk_url": sk_url(),
        "log_dir": log_dir,
        "chart_dir": chart_dir(&app),
    })
}

#[tauri::command]
fn chart_info() -> serde_json::Value {
    match CHARTS.get() {
        Some(s) => serde_json::json!({
            "tables": s.tables.len(),
            "layers": s.tables.iter().map(|t| serde_json::json!({
                "table": t.table, "kind": format!("{:?}", t.kind), "band": t.band, "rows": t.rows
            })).collect::<Vec<_>>(),
        }),
        None => serde_json::json!({ "tables": 0 }),
    }
}

fn tile_response(path: &str) -> Response<Vec<u8>> {
    let parts: Vec<&str> = path.trim_start_matches('/').trim_end_matches(".pbf").split('/').collect();
    let parsed = match parts.as_slice() {
        [z, x, y] => z.parse::<u8>().ok().zip(x.parse::<u32>().ok()).zip(y.parse::<u32>().ok()),
        _ => None,
    };
    let builder = Response::builder().header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    match (parsed, CHARTS.get()) {
        (Some(((z, x), y)), Some(store)) => builder
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .body(store.tile(z, x, y).as_ref().clone()),
        (Some(_), None) => builder.status(StatusCode::SERVICE_UNAVAILABLE).body(Vec::new()),
        _ => builder.status(StatusCode::NOT_FOUND).body(Vec::new()),
    }
    .expect("valid response")
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info, chart_info])
        .register_asynchronous_uri_scheme_protocol("chart", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(tile_response(&path)));
        })
        .setup(|app| {
            // Charts: open in the background (the first run unzips the LINZ downloads).
            let handle = app.handle().clone();
            let dir = chart_dir(&handle);
            std::thread::spawn(move || match chart::ChartStore::open_dir(&dir) {
                Ok(store) => {
                    let n = store.tables.len();
                    let _ = CHARTS.set(Arc::new(store));
                    let _ = handle.emit("chart-ready", serde_json::json!({ "tables": n, "dir": dir }));
                }
                Err(e) => {
                    let _ = handle.emit("chart-ready", serde_json::json!({ "tables": 0, "dir": dir, "error": e.to_string() }));
                }
            });

            // Signal K
            let handle = app.handle().clone();
            let log_dir = app.path().app_data_dir().ok().map(|p| p.join("logs"));
            let opts = signalk::Options { url: sk_url(), log_dir, min_interval: Duration::from_millis(250) };
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
