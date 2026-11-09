//! Decode a TIFF picture (PICREP) and write it as PNG: `cargo run -p enc-s57 --example picture -- in.tif out.png`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = std::fs::read(&a[1]).expect("read");
    let t = std::time::Instant::now();
    let p = enc_s57::picture::decode_tiff(&b).unwrap_or_else(|e| panic!("{e}"));
    let png = enc_s57::picture::to_png(&p).unwrap();
    println!("{} × {}, {} channel(s), {} → {} bytes in {:?}", p.width, p.height, p.channels, b.len(), png.len(), t.elapsed());
    std::fs::write(&a[2], png).expect("write");
}
