//! Verification of earlier runs against later runs' hour 0: verify <a.grb2> <b.grb2> [...]
//! (BBOX=w,s,e,n limits the area; default the whole common grid)
fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let files: Vec<grib::GribFile> = paths.iter().map(|p| grib::GribFile::open(p).expect("open")).collect();
    let refs: Vec<&grib::GribFile> = files.iter().collect();
    let bbox: [f64; 4] = std::env::var("BBOX")
        .ok()
        .map(|s| {
            let v: Vec<f64> = s.split(',').map(|x| x.parse().unwrap()).collect();
            [v[0], v[1], v[2], v[3]]
        })
        .unwrap_or([-180.0, -90.0, 180.0, 90.0]);
    for vt in grib::verify::verify_times(&refs) {
        println!("verify at {}: reference {:?}", grib::fmt_short(vt.t), vt.refs.iter().map(|&i| files[i].model()).collect::<Vec<_>>());
        let t0 = std::time::Instant::now();
        for s in grib::verify::scores(&refs, &vt, bbox, 6000) {
            println!(
                "  {} +{:.0} h: n {} rms {:.1} kn, bias {:+.1} kn, |dir| {:?}, veer {:?}, msl {:?} ({:.0} ms)",
                files[s.file].model().0,
                s.lead_h,
                s.n,
                s.rms_kn,
                s.speed_bias_kn,
                s.dir_abs_deg.map(|x| x.round()),
                s.veer_deg.map(|x| x.round()),
                s.msl_bias_hpa.map(|x| (x * 10.0).round() / 10.0),
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
    for r in grib::verify::track_record(&refs, bbox, 4000) {
        println!("track {} {}: {} times, rms {:.1}", r.model, r.band, r.count, r.rms_kn);
    }
}
