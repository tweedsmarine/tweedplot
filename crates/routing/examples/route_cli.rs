//! Route from the command line and write GeoJSON.
//!
//! cargo run --release -p routing --example route_cli -- \
//!   --grib file.grb2 --polar Morgane_median.csv --chart C:\dev\racenav-data\linz \
//!   --start -36.838,174.805 --marks -35.565,174.705;-35.258,174.125 \
//!   [--time 1791720000] [--safety 3] [--step 300] [--eff 1.0] [--out route.geojson]
use nav_core::LatLon;
use routing::{route, Params, Raster};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let ll = |s: &str| {
        let v: Vec<f64> = s.split(',').map(|x| x.trim().parse().expect("lat,lon")).collect();
        LatLon::new(v[0], v[1])
    };
    let grib = grib::GribFile::open(get("--grib").expect("--grib")).expect("read GRIB");
    let polar = polar::Polar::from_path(get("--polar").expect("--polar")).expect("read polar");
    let start = ll(&get("--start").expect("--start"));
    let marks: Vec<LatLon> = get("--marks").expect("--marks").split(';').map(ll).collect();
    let t0: i64 = get("--time").map(|s| s.parse().unwrap()).unwrap_or_else(|| grib.times(grib::Param::WindU)[0]);
    let safety: f64 = get("--safety").map(|s| s.parse().unwrap()).unwrap_or(3.0);
    let mut p = Params::default();
    if let Some(s) = get("--step") { p.step_s = s.parse().unwrap(); }
    if let Some(s) = get("--eff") { p.efficiency = s.parse().unwrap(); }

    let raster = get("--chart").map(|dir| {
        let store = chart::ChartStore::open_dir(dir).expect("open charts");
        let mut b = [start.lon, start.lat, start.lon, start.lat];
        for m in &marks {
            b = [b[0].min(m.lon), b[1].min(m.lat), b[2].max(m.lon), b[3].max(m.lat)];
        }
        let pad = 0.25;
        let bbox = [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad];
        let c = std::time::Instant::now();
        let r = Raster::from_chart(&store, bbox, safety, 50.0, 16_000_000);
        eprintln!("obstacles: {} x {} cells of {:.0} m in {} ms", r.w, r.h, r.cell_size_m(), c.elapsed().as_millis());
        r
    });

    let r = route(start, t0, &marks, &polar, &grib, raster.as_ref(), &p);
    eprintln!("{}: {}  steps {}  {} ms", r.status, r.message, r.steps, r.compute_ms);
    if let Some(a) = r.arrival {
        let h = (a - t0) as f64 / 3600.0;
        eprintln!("ETA {}  ({:.0} h {:02.0} min), {:.1} NM", grib::fmt_run(a), h.floor(), (h.fract() * 60.0).round(), r.distance_nm);
    }
    for l in &r.legs {
        eprintln!("  leg {:.1} NM, tacks {}, gybes {}, arrival {:?}", l.distance_nm, l.tacks, l.gybes, l.arrival.map(grib::fmt_run));
    }
    let line: Vec<[f64; 2]> = r.points.iter().map(|p| [p.lon, p.lat]).collect();
    let mut feats = vec![serde_json::json!({"type":"Feature","properties":{"kind":"route"},"geometry":{"type":"LineString","coordinates":line}})];
    for iso in &r.isochrones {
        feats.push(serde_json::json!({"type":"Feature","properties":{"kind":"isochrone"},"geometry":{"type":"LineString","coordinates":iso}}));
    }
    let out = get("--out").unwrap_or_else(|| "route.geojson".into());
    std::fs::write(&out, serde_json::to_string(&serde_json::json!({"type":"FeatureCollection","features":feats})).unwrap()).unwrap();
    eprintln!("wrote {out}");
    if let Some(j) = get("--json") {
        std::fs::write(&j, serde_json::to_string(&r).unwrap()).unwrap();
        eprintln!("wrote {j}");
    }
}
