//! Draw the map tiles of an area from a bathymetry store, for checking by eye:
//! `cargo run -p bathy --release --example bathy_tiles -- <store> <z> <w> <s> <e> <n> <out dir> [shallow deep] [blues]`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 8 {
        eprintln!("usage: bathy_tiles <store> <z> <w> <s> <e> <n> <out dir> [shallow deep] [blues]");
        std::process::exit(2);
    }
    let s = bathy::Store::open(std::path::Path::new(&a[1])).unwrap();
    let z: u8 = a[2].parse().unwrap();
    let f = |i: usize| a[i].parse::<f64>().unwrap();
    let (w, so, e, n) = (f(3), f(4), f(5), f(6));
    let out = std::path::PathBuf::from(&a[7]);
    std::fs::create_dir_all(&out).unwrap();
    let mut st = bathy::Style::default();
    if a.len() >= 10 {
        st.shallow = a[8].parse().unwrap();
        st.deep = a[9].parse().unwrap();
    } else if let Some(r) = s.depth_range([w, so, e, n]) {
        (st.shallow, st.deep) = r;
    }
    if a.get(10).is_some_and(|v| v == "blues") {
        st.palette = bathy::Palette::Blues;
    }
    println!("range {:.0}–{:.0} m", st.shallow, st.deep);
    s.set_style(st);
    let nn = (1u64 << z) as f64;
    let tx = |lon: f64| ((lon + 180.0) / 360.0 * nn) as u32;
    let ty = |lat: f64| {
        let r = lat.to_radians();
        ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * nn) as u32
    };
    let t = std::time::Instant::now();
    let mut k = 0;
    for x in tx(w)..=tx(e) {
        for y in ty(n)..=ty(so) {
            if let Some(p) = s.tile_png(z, x, y) {
                std::fs::write(out.join(format!("{z}_{x}_{y}.png")), p.as_ref()).unwrap();
                k += 1;
            }
        }
    }
    println!("{k} tiles in {:?}", t.elapsed());
}
