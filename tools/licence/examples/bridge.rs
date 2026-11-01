//! Development only: serves the Licence Manager page and its commands over HTTP, so the
//! page can be tried in a browser against the real vault code (TWEEDPLOT_OEM_DIR should
//! point at a scratch folder). `cargo run -p tweedplot-licence --example bridge -- 8765`
//! Save/open dialogs are replaced by files in the OEM folder.

use licence::{Customer, IssueRequest, Vault};
use serde_json::{json, Value};

fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(8765);
    let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let dir = licence::oem_dir();
    let mut test = true;
    let mut v = Vault::open(&dir, test).unwrap();
    let server = tiny_http::Server::http(("127.0.0.1", port)).unwrap();
    println!("http://127.0.0.1:{port}/  (OEM folder {})", dir.display());
    for mut req in server.incoming_requests() {
        let url = req.url().to_string();
        if let Some(cmd) = url.strip_prefix("/invoke/") {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let a: Value = serde_json::from_str(&body).unwrap_or(json!({}));
            let s = |k: &str| a[k].as_str().unwrap_or("").to_string();
            let r: Result<Value, String> = (|| {
                Ok(match cmd {
                    "lm_status" => json!(v.status()),
                    "lm_set_mode" => {
                        test = a["test"].as_bool().unwrap_or(true);
                        v = Vault::open(&dir, test)?;
                        json!(v.status())
                    }
                    "lm_set_credentials" => json!(v.set_credentials(&s("mId"), &s("mKey"))?),
                    "lm_import_old" => json!(v.import_old_files()?),
                    "lm_licences" => json!(v.licences()),
                    "lm_licence" => {
                        let id = a["id"].as_i64().unwrap_or(0);
                        json!({ "licence": v.licence(id)?, "log": v.log_entries(Some(id), 200) })
                    }
                    "lm_customers" => json!(v.customers()),
                    "lm_code_info" => json!(v.code_info(&s("code"))?),
                    "lm_issue" => {
                        let r: IssueRequest = serde_json::from_value(a["req"].clone()).map_err(|e| e.to_string())?;
                        json!(v.issue(&r)?)
                    }
                    "lm_release" => json!(v.release(&s("code"))?),
                    "lm_end_seat" => json!(v.end_seat(a["seatId"].as_i64().unwrap_or(0), &s("why"))?),
                    "lm_revoke" => json!(v.revoke(a["id"].as_i64().unwrap_or(0), &s("why"))?),
                    "lm_update_licence" => json!(v.update_licence(
                        a["id"].as_i64().unwrap_or(0),
                        &s("vessel"),
                        &s("mmsi"),
                        &s("callSign"),
                        a["seats"].as_u64().unwrap_or(3) as u32,
                        &s("notes")
                    )?),
                    "lm_update_customer" => {
                        let c: Customer = serde_json::from_value(a["customer"].clone()).map_err(|e| e.to_string())?;
                        json!(v.update_customer(&c)?)
                    }
                    "lm_log" => json!(v.log_entries(None, a["limit"].as_u64().unwrap_or(100) as u32)),
                    "lm_export_register" => {
                        let p = dir.join("register.csv.out");
                        std::fs::write(&p, v.register_csv()).map_err(|e| e.to_string())?;
                        json!(p.display().to_string())
                    }
                    "lm_export_backup" => {
                        let p = dir.join("bridge-backup.tplb");
                        v.export_backup(&p, &s("pass"))?;
                        json!(p.display().to_string())
                    }
                    "lm_restore_backup" => {
                        let p = dir.join("bridge-backup.tplb");
                        v.restore_backup(&p, &s("pass"))?;
                        json!(p.display().to_string())
                    }
                    "lm_mail" => json!(null),
                    _ => return Err(format!("unknown command {cmd}")),
                })
            })();
            let (code, out) = match r {
                Ok(v) => (200, v.to_string()),
                Err(e) => (500, json!(e).to_string()),
            };
            let _ = req.respond(tiny_http::Response::from_string(out).with_status_code(code));
            continue;
        }
        let name = if url == "/" { "index.html".to_string() } else { url.trim_start_matches('/').to_string() };
        let path = ui.join(&name);
        match std::fs::read(&path) {
            Ok(b) => {
                let ct = match path.extension().and_then(|e| e.to_str()) {
                    Some("html") => "text/html; charset=utf-8",
                    Some("css") => "text/css",
                    Some("js") => "text/javascript",
                    Some("svg") => "image/svg+xml",
                    _ => "application/octet-stream",
                };
                let _ = req.respond(tiny_http::Response::from_data(b).with_header(tiny_http::Header::from_bytes("Content-Type", ct).unwrap()));
            }
            Err(_) => {
                let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
            }
        }
    }
}
