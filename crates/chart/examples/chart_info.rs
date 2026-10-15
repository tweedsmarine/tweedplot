//! List the chart layers found in a folder and time a few tiles.
//! `cargo run --release -p chart --example chart_info -- C:\dev\racenav-data\linz`
fn main() {
    let dir = std::env::args().nth(1).expect("usage: chart_info <folder>");
    let t0 = std::time::Instant::now();
    let store = chart::ChartStore::open_dir(&dir).expect("open charts");
    println!("opened in {:.1}s", t0.elapsed().as_secs_f64());
    for t in &store.tables {
        println!("{:?} band {} rows {:>7}  {}", t.kind, t.band, t.rows, t.table);
    }
    // Nelson at a range of zooms
    for (z, lon, lat) in [(8u8, 173.27, -41.26), (10, 173.27, -41.26), (12, 173.27, -41.26), (14, 173.27, -41.26), (15, 173.28, -41.255)] {
        let n = 2f64.powi(z as i32);
        let x = ((lon + 180.0) / 360.0 * n) as u32;
        let l = (lat as f64).to_radians();
        let y = ((1.0 - (l.tan() + 1.0 / l.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32;
        let t = std::time::Instant::now();
        let tile = store.tile(z, x, y);
        println!("z{z} {x}/{y}: {} bytes in {:.0} ms", tile.len(), t.elapsed().as_secs_f64() * 1000.0);
    }
}
