fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let g = grib::GribFile::open(&a[0]).unwrap();
    let s = g.summary();
    for p in &s.params { println!("{:?} {:?} bbox {:?}", p.param, p.times.iter().map(|t| grib::fmt_short(*t)).collect::<Vec<_>>(), p.bbox); }
    let (lat, lon): (f64, f64) = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let p = s.params[0].param; let t = s.params[0].times[0];
    println!("{} {:?} at {lat},{lon}: {:?}", s.name, p, g.value(p, lat, lon, t));
    println!("scan: {:?}", grib::scan(&a[0]).map(|i| i.label));
}
