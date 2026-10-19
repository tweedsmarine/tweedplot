//! What the chart store puts in the tile under a point, for lights and marks:
//! `cargo run --release -p chart --example chart_probe -- 173.349 -41.172 11.5`
//! Opens the same folders as the app (racenav-data\linz, coastline, bathy).
fn main() {
    let a: Vec<f64> = std::env::args().skip(1).take(3).map(|s| s.parse().expect("lon lat zoom")).collect();
    let (lon, lat, zoom) = (a[0], a[1], a[2]);
    let root = std::path::PathBuf::from(std::env::var("RACENAV_DATA").unwrap_or_else(|_| r"C:\dev\racenav-data".into()));
    let dirs: Vec<_> = ["linz", "coastline", "bathy"].iter().map(|d| root.join(d)).collect();
    let store = chart::ChartStore::open_dirs(&dirs).expect("open charts");
    println!("{} tables. Lights, buoys and beacons:", store.tables.len());
    for t in &store.tables {
        if let Some(c) = t.class {
            if c.code == "LIGHTS" || c.code.starts_with("BOY") || c.code.starts_with("BCN") {
                println!("  {} band {} rows {:>6}  {}", c.code, t.band, t.rows, t.file.display());
            }
        }
    }
    let z = zoom.floor() as u8;
    let n = 2f64.powi(z as i32);
    let fx = (lon + 180.0) / 360.0 * n;
    let l = lat.to_radians();
    let fy = (1.0 - (l.tan() + 1.0 / l.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    let (x, y) = (fx as u32, fy as u32);
    let (px, py) = (((fx - x as f64) * 4096.0) as i64, ((fy - y as f64) * 4096.0) as i64);
    let tile = store.tile(z, x, y);
    println!("tile {z}/{x}/{y}: {} bytes; the point is at {px},{py}", tile.len());
    // features within ~2 km of the point, decoded from the tile
    let r = (2000.0 / (40_075_016.0 * l.cos() / n) * 4096.0) as i64;
    for (cls, band, minz, gx, gy) in chart::mvt::debug_points(&tile, "fpt") {
        if (gx - px).abs() <= r && (gy - py).abs() <= r && (cls == "LIGHTS" || cls.starts_with("BOY") || cls.starts_with("BCN")) {
            println!("  {cls:7} band {band} minz {minz:>4}  at {gx},{gy}  {}", if minz <= zoom.floor() { "shown" } else { "hidden at this zoom" });
        }
    }
}
