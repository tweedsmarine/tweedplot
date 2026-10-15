//! Summarise a GRIB file and print the wind at a point.
//! `cargo run -p grib --example grib_info -- file.grb2 [lat lon]`
fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("usage: grib_info <file> [lat lon]");
    let f = grib::GribFile::open(&path).expect("read GRIB");
    let s = f.summary();
    println!("{}  ({} fields, {} skipped)", s.name, f.fields.len(), s.skipped);
    for p in &s.params {
        println!(
            "  {:?}: {} times {} .. {}, grid {:.3}° x {:.3}°, bbox {:?}",
            p.param,
            p.times.len(),
            grib::fmt_run(*p.times.first().unwrap_or(&0)),
            grib::fmt_run(*p.times.last().unwrap_or(&0)),
            p.dlat,
            p.dlon,
            p.bbox
        );
    }
    for msg in f.skipped.iter().take(5) {
        println!("  skipped: {msg}");
    }
    if let (Some(lat), Some(lon)) = (a.next().and_then(|s| s.parse().ok()), a.next().and_then(|s| s.parse().ok())) {
        for t in f.times(grib::Param::WindU).into_iter().take(12) {
            match f.wind(lat, lon, t) {
                Some(w) => println!("  {}  {:5.1} kn from {:03.0}°", grib::fmt_run(t), w.tws, w.twd),
                None => println!("  {}  no data", grib::fmt_run(t)),
            }
        }
    }
}
