//! Write a tile's raw bytes: `tile_dump <out.mvt> <dir>... -- z x y`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let k = a.iter().position(|s| s == "--").unwrap();
    let dirs: Vec<std::path::PathBuf> = a[1..k].iter().map(Into::into).collect();
    let (z, x, y): (u8, u32, u32) = (a[k + 1].parse().unwrap(), a[k + 2].parse().unwrap(), a[k + 3].parse().unwrap());
    let s = chart::ChartStore::open_dirs(&dirs).unwrap();
    if std::env::var_os("TILESERVE_S52").is_some() {
        let res = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources");
        s.set_s52(Some(chart::s52::S52::open(&res.join("s101-pc"), &res.join("s101-fc/101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml")).unwrap()));
        s.set_s52_context("SafetyContour", "5");
        // TILESERVE_CTX="FullLightLines=true,ShallowWaterDangers=true": more context
        for kv in std::env::var("TILESERVE_CTX").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
            if let Some((k, v)) = kv.split_once('=') {
                s.set_s52_context(k, v);
            }
        }
        if let Ok(v) = std::env::var("TILESERVE_SWD") {
            s.set_s52_context("ShallowWaterDangers", &v);
        }
        let n = s.s52_warm(&|_, _| {});
        eprintln!("portrayed {n} tables");
    }
    std::fs::write(&a[0], s.tile(z, x, y).as_ref()).unwrap();
}
