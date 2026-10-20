//! Render the tiles over an area at a zoom, in parallel and again, and report any whose
//! land layer is missing or differs between runs: `land_check <dir>... -- z w s e n [runs]`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let k = a.iter().position(|s| s == "--").unwrap();
    let dirs: Vec<std::path::PathBuf> = a[..k].iter().map(Into::into).collect();
    let z: u8 = a[k + 1].parse().unwrap();
    let b: Vec<f64> = a[k + 2..k + 6].iter().map(|v| v.parse().unwrap()).collect();
    let runs: usize = a.get(k + 6).and_then(|v| v.parse().ok()).unwrap_or(3);
    let s = std::sync::Arc::new(chart::ChartStore::open_dirs(&dirs).unwrap());
    let n = 2f64.powi(z as i32);
    let tx = |lon: f64| ((lon + 180.0) / 360.0 * n) as u32;
    let ty = |lat: f64| { let r = lat.to_radians(); ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32 };
    let tiles: Vec<(u32, u32)> = (tx(b[0])..=tx(b[2])).flat_map(|x| (ty(b[3])..=ty(b[1])).map(move |y| (x, y))).collect();
    let mut first: std::collections::HashMap<(u32, u32), usize> = Default::default();
    for run in 0..runs {
        s.clear_cache();
        let t0 = std::time::Instant::now();
        let res: Vec<((u32, u32), usize, f64)> = std::thread::scope(|sc| {
            let hs: Vec<_> = tiles.iter().map(|&(x, y)| { let s = s.clone(); sc.spawn(move || { let t = std::time::Instant::now(); let v = s.tile(z, x, y); (( x, y), chart::mvt::debug_points(&v, "land").len(), t.elapsed().as_secs_f64()) }) }).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let slow = res.iter().map(|r| r.2).fold(0.0, f64::max);
        println!("run {run}: {} tiles in {:.1} s (slowest {slow:.1} s)", res.len(), t0.elapsed().as_secs_f64());
        for (t, l, _) in &res {
            match first.get(t) {
                None => { first.insert(*t, *l); }
                Some(f) if f != l => println!("  {z}/{}/{}: land {} then {}", t.0, t.1, f, l),
                _ => {}
            }
        }
    }
    let mut v: Vec<_> = first.iter().collect();
    v.sort();
    println!("land features per tile: {:?}", v);
}
