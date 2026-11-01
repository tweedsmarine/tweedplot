//! Import a PBG with a fake chart (everything 500–2000 m) so the shallower cells are left
//! out, then draw tiles with "Show left out": `bathy_rejtest <pbg> <store> <out dir>`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let _ = std::fs::remove_file(&a[2]);
    let s = bathy::Store::open(std::path::Path::new(&a[2])).unwrap();
    let r = bathy::check::Reference::new(|b| {
        let mut g = bathy::check::RefGrid::new(b, bathy::check::REF_STEP);
        g.paint(1, 500.0, Some(2000.0), &[vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]]);
        g
    });
    let (n, rej) = s.import_pbg_checked(std::path::Path::new(&a[1]), Some(&r), &mut |_, _, _| {}, &Default::default()).unwrap();
    println!("added {n}, left out {rej}; info {:?}", s.info().unwrap().rejected);
    s.set_style(bathy::Style { rejected: true, shallow: 0.0, deep: 1500.0, ..Default::default() });
    let out = std::path::PathBuf::from(&a[3]);
    std::fs::create_dir_all(&out).unwrap();
    for (z, lon, lat) in [(10u8, 170.6f64, -41.9f64), (12, 171.0, -41.6), (13, 171.3, -41.5), (14, 171.4, -41.55)] {
        let n = (1u64 << z) as f64;
        let r = lat.to_radians();
        let (x, y) = (((lon + 180.0) / 360.0 * n) as u32, ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32);
        let mut magenta = 0;
        let mut drawn = 0;
        for dx in 0..4 {
            for dy in 0..4 {
                if let Some(p) = s.tile_png(z, x + dx, y + dy) {
                    drawn += 1;
                    let mut d = png::Decoder::new(std::io::Cursor::new(p.as_ref().clone())).read_info().unwrap();
                    let mut buf = vec![0; d.output_buffer_size()];
                    d.next_frame(&mut buf).unwrap();
                    magenta += buf.chunks(4).filter(|q| q == &[220, 0, 200, 255]).count();
                    std::fs::write(out.join(format!("{z}_{}_{}.png", x + dx, y + dy)), p.as_ref()).unwrap();
                }
            }
        }
        println!("z{z}: {drawn} tiles, {magenta} magenta pixels");
    }
}
