//! Time opening the chart store, and drawing a tile: `open_time <dir> [z x y]`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let s = chart::ChartStore::open_dirs(&[a[0].clone().into()]).unwrap();
    println!("{} tables", s.tables.len());
    if a.len() >= 4 {
        let (z, x, y): (u8, u32, u32) = (a[1].parse().unwrap(), a[2].parse().unwrap(), a[3].parse().unwrap());
        for k in 0..3 {
            let t = std::time::Instant::now();
            let n = s.tile(z, x + k, y).len();
            println!("tile {n} bytes in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
