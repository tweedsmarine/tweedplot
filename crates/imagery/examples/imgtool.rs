//! Look at an image the way TweedPlot sees it, and try the Sentinel-2 download from the
//! command line:
//!
//!   cargo run -p imagery --example imgtool -- info <file.kap|file.tif>
//!   cargo run -p imagery --example imgtool -- tile <file> <z> <x> <y> <out.png>
//!   cargo run -p imagery --example imgtool -- copy <in.tif> <out.tif>      (rewrite as TweedPlot's GeoTIFF)
//!   cargo run -p imagery --example imgtool -- search <w> <s> <e> <n> <from> <to> [max cloud %]
//!   cargo run -p imagery --example imgtool -- get <w> <s> <e> <n> <from> <to> <out.tif> [max px]

use std::path::Path;
use std::sync::Arc;

use imagery::render::Raster;

fn open(p: &str) -> Arc<dyn Raster> {
    let path = Path::new(p);
    let kind = imagery::store::sniff(path).unwrap_or_else(|e| panic!("{e}"));
    imagery::store::open_raster(kind, path).unwrap_or_else(|e| panic!("{e}"))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f = |i: usize| a[i].parse::<f64>().expect("a number");
    match a.first().map(String::as_str) {
        Some("info") => {
            let r = open(&a[1]);
            println!(
                "size {:?}, {} levels, bounds {:?}, {:.2} m/pixel, native zoom {}",
                r.size(),
                r.levels(),
                r.bounds(),
                r.resolution_m(),
                imagery::render::native_zoom(r.as_ref())
            );
            if let Ok(k) = imagery::kap::Kap::open(Path::new(&a[1])) {
                println!(
                    "KAP {} {} ({}), 1:{:?}, {} {}, DTM {:?}, {} REF points fit to {:.2} px RMS",
                    k.number,
                    k.name,
                    k.edition,
                    k.scale,
                    k.projection,
                    k.datum,
                    k.dtm,
                    k.refs.len(),
                    k.fit_rms_px
                );
            }
        }
        Some("tile") => {
            let r = open(&a[1]);
            let png =
                imagery::render::render_tile(r.as_ref(), a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap()).expect("nothing in that tile");
            std::fs::write(&a[5], png).unwrap();
        }
        Some("copy") => {
            let t = imagery::tiff::Tiff::open(Arc::new(imagery::source::FileSource::open(Path::new(&a[1])).unwrap())).unwrap();
            let bbox = t.bounds();
            let p = imagery::s2::Progress::default();
            let cut = imagery::s2::cut(&t, bbox, 100_000, 4, &p).unwrap();
            imagery::tiffw::write_geotiff(Path::new(&a[2]), cut.w, cut.h, &cut.rgba, &cut.geo, "imgtool").unwrap();
            println!("{}x{} at {:.2} m", cut.w, cut.h, cut.res_m);
        }
        Some("search") => {
            let max = if a.len() > 7 { f(7) } else { 30.0 };
            for s in imagery::s2::search([f(1), f(2), f(3), f(4)], &a[5], &a[6], max).unwrap_or_else(|e| panic!("{e}")) {
                println!("{} {} cloud {:?}% cover {:.0}% {}", s.datetime, s.id, s.cloud, s.cover * 100.0, s.visual);
            }
        }
        Some("get") => {
            let bbox = [f(1), f(2), f(3), f(4)];
            let scenes = imagery::s2::search(bbox, &a[5], &a[6], 20.0).unwrap_or_else(|e| panic!("{e}"));
            let s = scenes.first().expect("no scenes");
            let max = if a.len() > 8 { a[8].parse().unwrap() } else { 4096 };
            let p = imagery::s2::Progress::default();
            let cut = imagery::s2::download(s, bbox, max, Path::new(&a[7]), &p).unwrap_or_else(|e| panic!("{e}"));
            println!("{}: {}x{} at {:.1} m, {}", s.id, cut.w, cut.h, cut.res_m, imagery::s2::credit(&s.datetime));
        }
        _ => eprintln!("imgtool info|tile|copy|search|get … (see the top of examples/imgtool.rs)"),
    }
}
