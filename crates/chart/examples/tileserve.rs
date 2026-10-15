//! Serve chart tiles over HTTP for testing the front end in a browser.
//! `cargo run --release -p chart --example tileserve -- <folder> [port]`
//! Tiles at http://localhost:7070/{z}/{x}/{y}.pbf
fn main() {
    let dir = std::env::args().nth(1).expect("usage: tileserve <folder> [port]");
    let port = std::env::args().nth(2).unwrap_or_else(|| "7070".into());
    let store = std::sync::Arc::new(chart::ChartStore::open_dir(&dir).expect("open charts"));
    let server = tiny_http::Server::http(format!("0.0.0.0:{port}")).expect("bind");
    println!("serving {} tables on :{port}", store.tables.len());
    for req in server.incoming_requests() {
        let parts: Vec<&str> = req.url().trim_start_matches('/').trim_end_matches(".pbf").split('/').collect();
        let tile = match parts.as_slice() {
            [z, x, y] => match (z.parse(), x.parse(), y.parse()) {
                (Ok(z), Ok(x), Ok(y)) => Some(store.tile(z, x, y)),
                _ => None,
            },
            _ => None,
        };
        let cors = tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap();
        let resp = match tile {
            Some(t) => tiny_http::Response::from_data(t.as_ref().clone())
                .with_header(tiny_http::Header::from_bytes("Content-Type", "application/x-protobuf").unwrap())
                .with_header(cors),
            None => tiny_http::Response::from_data(Vec::new()).with_status_code(404).with_header(cors),
        };
        let _ = req.respond(resp);
    }
}
