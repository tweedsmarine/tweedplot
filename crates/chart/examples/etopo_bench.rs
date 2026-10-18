//! Time the ETOPO depth-area tracing on a synthetic shelf-like grid.
fn main() {
    let (nx, ny) = (2400usize, 2400usize);
    let mut z = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            let (x, y) = (i as f64 / 60.0, j as f64 / 60.0);
            // a shelf around 100 m with ripples, dropping to 4000 m in the east
            let d = 95.0 + 12.0 * (x * 3.1).sin() * (y * 2.3).cos() + 6.0 * ((x + y) * 9.7).sin() + if x > 25.0 { (x - 25.0) * 300.0 } else { 0.0 };
            z.push(-(d as f32));
        }
    }
    let g = chart::bathy::Grid { west: -66.0, south: -55.0, dlon: 1.0 / 240.0, dlat: 1.0 / 240.0, nx, ny, z };
    let t = std::time::Instant::now();
    let dir = std::env::temp_dir().join("etopo-bench");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = chart::bathy::write_grid(&g, &dir.join("etopo_bench.gpkg"), "bench").unwrap();
    println!("{:?} {} bytes {:.1} s", out, std::fs::metadata(&out).unwrap().len(), t.elapsed().as_secs_f64());
}
