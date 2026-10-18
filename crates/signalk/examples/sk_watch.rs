//! Print own-vessel updates from a Signal K server.
//! `cargo run -p signalk --example sk_watch -- [ws-url]`
use signalk::{run, Event, Options};

#[tokio::main]
async fn main() {
    let url = std::env::args().nth(1).unwrap_or_else(|| signalk::DEFAULT_URL.to_string());
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(run(Options { url, min_interval: std::time::Duration::from_secs(1), ..Default::default() }, tx));
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Status { connected, detail } => println!("status: connected={connected} {detail}"),
            Event::Vessel(v) => println!(
                "{}  {:.5} {:.5}  COG {:>5.1}  SOG {:>4.1}",
                v.datetime.or(v.timestamp).unwrap_or_default(),
                v.lat.unwrap_or(f64::NAN),
                v.lon.unwrap_or(f64::NAN),
                v.cog.unwrap_or(f64::NAN),
                v.sog.unwrap_or(f64::NAN)
            ),
            Event::Other { context, values } => println!("{context}: {} values", values.len()),
        }
    }
}
