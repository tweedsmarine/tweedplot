//! Compare a CCSDS-packed copy with the original: ccsds_check <original> <ccsds copy>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let orig = grib::GribFile::from_bytes(&std::fs::read(&a[0]).unwrap());
    let t0 = std::time::Instant::now();
    let c = grib::GribFile::from_bytes(&std::fs::read(&a[1]).unwrap());
    println!("decoded {} CCSDS fields in {:.0} ms, skipped {:?}", c.fields.len(), t0.elapsed().as_secs_f64() * 1000.0, c.skipped.iter().take(3).collect::<Vec<_>>());
    for f in &c.fields {
        let o = orig.fields.iter().find(|o| o.param == f.param && o.valid_time == f.valid_time).unwrap();
        let maxd = f.values.iter().zip(&o.values).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
        let mean = f.values.iter().map(|&x| x as f64).sum::<f64>() / f.values.len() as f64;
        println!("{:?} {} n {} mean {:.4} max|diff| vs original {:.4}", f.param, grib::fmt_short(f.valid_time), f.values.len(), mean, maxd);
    }
}
