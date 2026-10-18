//! List charted vertical clearances near a point: clearances <dir> <lat> <lon> [radius m]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let store = chart::ChartStore::open_dirs(&[std::path::PathBuf::from(&a[1])]).unwrap();
    let (lat, lon): (f64, f64) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let r = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(2000.0);
    for c in store.clearances([lon, lat], r) {
        println!("{:>7.0} m  {:>5.1}°  {:>5.1} m  {} {}", c.distance_m, c.bearing, c.clearance_m, c.class, c.name.unwrap_or_default());
    }
}
