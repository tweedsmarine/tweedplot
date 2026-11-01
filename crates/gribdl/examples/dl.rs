//! Download a GRIB: dl <model id> w,s,e,n hours every [params...]  e.g. dl ecmwf_ifs0p25 165,-48,180,-33 48 3 wind mslp
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let model: gribdl::Model = serde_json::from_value(serde_json::Value::String(a[0].clone())).expect("model");
    let b: Vec<f64> = a[1].split(',').map(|x| x.parse().unwrap()).collect();
    let req = gribdl::Request {
        model,
        area: [b[0], b[1], b[2], b[3]],
        params: a[4..].to_vec(),
        hours: a[2].parse().unwrap(),
        every: a[3].parse().unwrap(),
        depth: std::env::var("DEPTH").ok().and_then(|x| x.parse().ok()).unwrap_or(0.0),
    };
    let now = std::env::var("NOW")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64);
    let run = gribdl::latest_run(model, *model.hours(req.hours, req.every).last().unwrap(), now).expect("run");
    println!("run {:?}; download ~{} MB", run, gribdl::download_bytes(&req) / 1_000_000);
    let t = std::time::Instant::now();
    let p = gribdl::download(
        &req,
        &run,
        std::path::Path::new("."),
        &|d, n, b| eprint!("\r{d}/{n} {:.1} MB ", b as f64 / 1e6),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("download");
    let g = grib::GribFile::open(&p).unwrap();
    println!(
        "\n{} in {:.1} s: {} {:?}, {} fields",
        p.display(),
        t.elapsed().as_secs_f64(),
        g.model().0,
        g.summary().params.iter().map(|p| (format!("{:?}", p.param), p.times.len())).collect::<Vec<_>>(),
        g.fields.len()
    );
}
