//! Draw every tile over a box at a range of zooms (finding crashes and slow tiles):
//! `sweep <w> <s> <e> <n> <zmin> <zmax> <dir>...`. Set TWEEDPLOT_TILE_LOG=1 to see each tile.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let b: Vec<f64> = a[..4].iter().map(|v| v.parse().unwrap()).collect();
    let (z0, z1): (u8, u8) = (a[4].parse().unwrap(), a[5].parse().unwrap());
    let dirs: Vec<std::path::PathBuf> = a[6..].iter().map(Into::into).collect();
    let s = chart::ChartStore::open_dirs(&dirs).unwrap();
    for z in z0..=z1 {
        let n = (1u32 << z) as f64;
        let tx = |lon: f64| ((lon + 180.0) / 360.0 * n) as u32;
        let ty = |lat: f64| { let r = lat.to_radians(); ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32 };
        let t0 = std::time::Instant::now();
        let mut k = 0;
        for x in tx(b[0])..=tx(b[2]) {
            for y in ty(b[3])..=ty(b[1]) {
                s.tile(z, x, y);
                k += 1;
            }
        }
        println!("zoom {z}: {k} tiles in {:.1} s", t0.elapsed().as_secs_f64());
    }
}
