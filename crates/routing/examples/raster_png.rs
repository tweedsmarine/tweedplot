//! Dump an obstacle raster as a PGM image: raster_png <chartdir> w s e n safety out.pgm
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = |i: usize| a[i].parse::<f64>().unwrap();
    let store = chart::ChartStore::open_dir(&a[1]).unwrap();
    let r = routing::Raster::from_chart(&store, [f(2), f(3), f(4), f(5)], f(6), 50.0, 16_000_000);
    let mut out = format!("P5 {} {} 255\n", r.w, r.h).into_bytes();
    for j in (0..r.h).rev() {
        for i in 0..r.w {
            out.push(match r.cells[j * r.w + i] { 0 => 200, 1 => 255, _ => 40 });
        }
    }
    std::fs::write(&a[7], out).unwrap();
    eprintln!("{} x {}", r.w, r.h);
}
