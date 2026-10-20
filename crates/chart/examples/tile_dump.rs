//! Write a tile's raw bytes: `tile_dump <out.mvt> <dir>... -- z x y`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let k = a.iter().position(|s| s == "--").unwrap();
    let dirs: Vec<std::path::PathBuf> = a[1..k].iter().map(Into::into).collect();
    let (z, x, y): (u8, u32, u32) = (a[k + 1].parse().unwrap(), a[k + 2].parse().unwrap(), a[k + 3].parse().unwrap());
    let s = chart::ChartStore::open_dirs(&dirs).unwrap();
    std::fs::write(&a[0], s.tile(z, x, y).as_ref()).unwrap();
}
