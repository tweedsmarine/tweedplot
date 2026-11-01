//! Print the charted depth at a place: `cargo run -p chart --example depth_at -- <chart dir> <lon> <lat>`,
//! or with a fourth argument (a step in degrees), how many places on a grid around it have one.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let store = chart::ChartStore::open_dir(&a[1]).expect("charts");
    let (lon, lat): (f64, f64) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    match a.get(4).and_then(|s| s.parse::<f64>().ok()) {
        None => println!("{:?}", store.depth_area_at(lon, lat)),
        Some(step) => {
            let mut hits = Vec::new();
            for i in -20..=20 {
                for j in -20..=20 {
                    let (x, y) = (lon + i as f64 * step, lat + j as f64 * step);
                    if let Some(h) = store.depth_area_at(x, y) {
                        hits.push((x, y, h.2));
                    }
                }
            }
            println!("{} of 1681 places; first {:?}", hits.len(), hits.first());
        }
    }
}
