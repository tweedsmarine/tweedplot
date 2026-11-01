//! Count the features in each layer of a tile: `tile_layers <dir>... -- z x y`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let k = a.iter().position(|s| s == "--").unwrap();
    let dirs: Vec<std::path::PathBuf> = a[..k].iter().map(Into::into).collect();
    let (z, x, y): (u8, u32, u32) = (a[k + 1].parse().unwrap(), a[k + 2].parse().unwrap(), a[k + 3].parse().unwrap());
    let s = chart::ChartStore::open_dirs(&dirs).unwrap();
    if let Ok(d) = std::env::var("LAND_DETAIL") {
        chart::coast::LAND_DETAIL.store(d.parse().unwrap(), std::sync::atomic::Ordering::Relaxed);
    }
    let t = s.tile(z, x, y);
    for l in ["wdepth", "depare", "depcnt", "far", "land", "wcoast", "fln", "soundg", "fpt"] {
        let v = chart::mvt::debug_points(&t, l);
        let mut bands: Vec<u64> = v.iter().map(|p| p.1).collect();
        bands.sort();
        bands.dedup();
        println!("{l:7} {:4} features, bands {:?}", v.len(), bands);
    }
}
