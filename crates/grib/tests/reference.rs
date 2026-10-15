//! Check the decoder against values decoded by ecCodes from the same files.
use grib::{GribFile, Param};
use serde_json::Value;

fn data(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

/// Meltemus file: ECMWF IFS 0.25 + WAM + Copernicus currents, simple packing,
/// JSON local section, a bitmap on the currents.
#[test]
fn meltemus_matches_eccodes() {
    let f = GribFile::open(data("meltemus_ecmwf_wam_smoc_small.grb2")).unwrap();
    let r: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(data("meltemus_ref.json")).unwrap()).unwrap();
    assert_eq!(f.fields.len(), r.len(), "skipped: {:?}", f.skipped);
    for (fld, r) in f.fields.iter().zip(&r) {
        let n = r["n"].as_u64().unwrap() as usize;
        assert_eq!(fld.values.len(), n);
        // ecCodes gives values in scan order (here south to north = canonical).
        let v0 = r["v0"].as_f64().unwrap();
        let vl = r["vlast"].as_f64().unwrap();
        let got0 = fld.values[0] as f64;
        let gotl = fld.values[n - 1] as f64;
        // ecCodes uses 9999 for points masked by the bitmap.
        let m = |x: f64| if x == 9999.0 { f64::NAN } else { x };
        let (v0, vl) = (m(v0), m(vl));
        assert!(v0.is_nan() && got0.is_nan() || close(got0, v0, 1e-5), "{}: {got0} vs {v0}", r["shortName"]);
        assert!(vl.is_nan() && gotl.is_nan() || close(gotl, vl, 1e-5), "{}: {gotl} vs {vl}", r["shortName"]);
        let date = r["validityDate"].as_u64().unwrap();
        let time = r["validityTime"].as_u64().unwrap();
        let t = grib::unix((date / 10000) as i32, (date / 100 % 100) as u8, (date % 100) as u8, (time / 100) as u8, (time % 100) as u8, 0);
        assert_eq!(fld.valid_time, t, "{}", r["shortName"]);
    }
    let s = f.summary();
    assert_eq!(s.model, "ECMWF 0p25");
    assert!(s.name.starts_with("ECMWF 0p25 2026-10-11 12Z"), "{}", s.name);
    assert_eq!(f.times(Param::WindU).len(), 85);

    // Wind at a grid point equals the grid value; between points it is interpolated.
    let t = f.times(Param::WindU)[0];
    let w = f.wind(-37.25, 173.75, t).unwrap();
    assert!(close(w.u, 4.6544036865234375, 1e-6));
    // Outside the GRIB's time span: no answer.
    assert!(f.wind(-37.25, 173.75, t - 3600).is_none());
    assert!(f.wind(-36.0, 174.5, t + 1800).is_some());
    // Currents are masked on land.
    assert!(!f.times(Param::CurrentU).is_empty());
}

/// Same data re-packed by ecCodes as complex packing with spatial differencing.
#[test]
fn complex_spatial_differencing_matches_eccodes() {
    let f = GribFile::open(data("complex_spatial_diff.grb2")).unwrap();
    let r: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(data("complex_ref.json")).unwrap()).unwrap();
    assert_eq!(f.fields.len(), r.len(), "skipped: {:?}", f.skipped);
    for (fld, r) in f.fields.iter().zip(&r) {
        for (i, v) in r["v"].as_array().unwrap().iter().enumerate() {
            let want = v.as_f64().unwrap();
            assert!(close(fld.values[i] as f64, want, 1e-5), "{} [{i}]: {} vs {want}", r["shortName"], fld.values[i]);
        }
        let mean: f64 = fld.values.iter().map(|&x| x as f64).sum::<f64>() / fld.values.len() as f64;
        assert!(close(mean, r["mean"].as_f64().unwrap(), 1e-5));
    }
}
