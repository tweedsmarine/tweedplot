//! Agreement between GRIB files at their first common time: compare <a.grb2> <b.grb2> [...]
fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let files: Vec<grib::GribFile> = paths.iter().map(|p| grib::GribFile::open(p).expect("open")).collect();
    for (p, f) in paths.iter().zip(&files) {
        let s = f.summary();
        let b = f.series(grib::Param::WindU).first().or(f.series(grib::Param::WindSpeed).first()).map(|x| x.grid.bbox());
        let ts = f.times(grib::Param::WindU);
        let ts = if ts.is_empty() { f.times(grib::Param::WindSpeed) } else { ts };
        println!(
            "{} | {} run {} | bbox {:?} | {} times {}..{}",
            p.rsplit('/').next().unwrap(),
            s.name,
            grib::fmt_run(f.model().1),
            b,
            ts.len(),
            ts.first().map(|t| grib::fmt_short(*t)).unwrap_or_default(),
            ts.last().map(|t| grib::fmt_short(*t)).unwrap_or_default()
        );
    }
    let refs: Vec<&grib::GribFile> = files.iter().collect();
    // a common time: the latest first time
    let t = files
        .iter()
        .map(|f| {
            let ts = f.times(grib::Param::WindU);
            *(if ts.is_empty() { f.times(grib::Param::WindSpeed) } else { ts }).first().unwrap()
        })
        .max()
        .unwrap()
        + 24 * 3600;
    let bbox: [f64; 4] = std::env::var("BBOX")
        .ok()
        .map(|s| {
            let v: Vec<f64> = s.split(',').map(|x| x.parse().unwrap()).collect();
            [v[0], v[1], v[2], v[3]]
        })
        .unwrap_or([172.0, -38.0, 177.0, -34.0]);
    for m in ["vector", "speed", "direction", "pressure", "trend"] {
        let t0 = std::time::Instant::now();
        match grib::compare::agreement(&refs, grib::compare::Metric::parse(m).unwrap(), t, bbox, 20000) {
            Some(g) => {
                let v: Vec<f32> = g.values.iter().flatten().copied().collect();
                let mean = v.iter().sum::<f32>() / v.len().max(1) as f32;
                let max = v.iter().cloned().fold(f32::MIN, f32::max);
                println!(
                    "{m:9} at {}: {}x{} step {:.3}°, {} values, mean {mean:.1}, max {max:.1}, used {:?}, {:.0} ms",
                    grib::fmt_short(t),
                    g.ni,
                    g.nj,
                    g.dlon,
                    v.len(),
                    g.used,
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }
            None => println!("{m:9}: not available"),
        }
    }
}
