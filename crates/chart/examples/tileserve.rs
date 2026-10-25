//! Serve chart tiles over HTTP for testing the front end in a browser.
//! `cargo run --release -p chart --example tileserve -- <folder>[,<folder>...] [port]`
//! Tiles at http://localhost:7070/{z}/{x}/{y}.pbf
fn main() {
    let dir = std::env::args().nth(1).expect("usage: tileserve <folder> [port]");
    let port = std::env::args().nth(2).unwrap_or_else(|| "7070".into());
    let dirs: Vec<std::path::PathBuf> = dir.split(',').map(Into::into).collect();
    let store = std::sync::Arc::new(chart::ChartStore::open_dirs(&dirs).expect("open charts"));
    let server = tiny_http::Server::http(format!("0.0.0.0:{port}")).expect("bind");
    // high-resolution depths for testing: TILESERVE_HIRES=<folder>, served at /hires/z/x/y.png
    let hires = std::env::var("TILESERVE_HIRES").ok().map(|d| chart::hires::Hires::open(std::path::Path::new(&d)));
    println!("serving {} tables on :{port}", store.tables.len());
    for req in server.incoming_requests() {
        let parts: Vec<&str> = req.url().split('?').next().unwrap_or("").trim_start_matches('/').trim_end_matches(".pbf").split('/').collect();
        let tile = match parts.as_slice() {
            [z, x, y] => match (z.parse(), x.parse(), y.parse()) {
                (Ok(z), Ok(x), Ok(y)) => Some(store.tile(z, x, y)),
                _ => None,
            },
            _ => None,
        };
        let cors = tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap();
        // feature details for testing: /feature/<table>/<fid> as JSON
        if let ["feature", t, fid] = parts.as_slice() {
            let info = t.parse().ok().zip(fid.parse().ok()).and_then(|(t, f)| store.feature(t, f));
            let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
            let body = match info {
                Some(i) => format!(
                    "{{\"code\":{},\"name\":{},\"band\":{},\"attrs\":[{}]}}",
                    q(i.code),
                    q(i.class_name),
                    i.band,
                    i.attrs.iter().map(|(k, v)| format!("[{},{}]", q(k), q(v))).collect::<Vec<_>>().join(",")
                ),
                None => "null".into(),
            };
            let _ = req.respond(tiny_http::Response::from_string(body).with_header(cors));
            continue;
        }
        if let (Some(h), ["hires", z, x, y]) = (&hires, parts.as_slice()) {
            let t0 = std::time::Instant::now();
            let png = match (z.parse(), x.parse(), y.trim_end_matches(".png").parse()) {
                (Ok(z), Ok(x), Ok(y)) => h.tile_png(z, x, y),
                _ => None,
            };
            if png.is_some() {
                eprintln!("hires {z}/{x}/{y} {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
            }
            let resp = match png {
                Some(p) => tiny_http::Response::from_data(p.as_ref().clone()).with_header(tiny_http::Header::from_bytes("Content-Type", "image/png").unwrap()),
                None => tiny_http::Response::from_data(Vec::new()).with_status_code(204),
            };
            let _ = req.respond(resp.with_header(cors));
            continue;
        }
        // fishing rules for testing: /rules_at/<lon>/<lat> and /rule/<rid> as JSON
        if let ["rules_at", lon, lat] = parts.as_slice() {
            let p = [lon.parse().unwrap_or(0.0), lat.parse().unwrap_or(0.0)];
            let body = serde_json::to_string(&store.rules_at(&[p])).unwrap();
            let _ = req.respond(tiny_http::Response::from_string(body).with_header(cors));
            continue;
        }
        if let ["rule", rid] = parts.as_slice() {
            let body = match store.rule_info(rid.parse().unwrap_or(0)) {
                Some((layers, attrs)) => serde_json::json!({ "layers": layers, "attrs": attrs }).to_string(),
                None => "null".into(),
            };
            let _ = req.respond(tiny_http::Response::from_string(body).with_header(cors));
            continue;
        }
        let resp = match tile {
            Some(t) => tiny_http::Response::from_data(t.as_ref().clone())
                .with_header(tiny_http::Header::from_bytes("Content-Type", "application/x-protobuf").unwrap())
                .with_header(cors),
            None => tiny_http::Response::from_data(Vec::new()).with_status_code(404).with_header(cors),
        };
        let _ = req.respond(resp);
    }
}
