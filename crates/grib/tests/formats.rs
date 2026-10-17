//! CCSDS (template 5.42, as ECMWF open data) and GRIB1 decoding, against
//! values ecCodes decoded from the same files (made with ecCodes from GFS 10 m U).
use grib::{GribFile, Param};

fn check(file: &str, valid: usize, mean: f64, nw: f64, se: Option<f64>) -> GribFile {
    let g = GribFile::open(format!("{}/tests/data/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    assert!(g.skipped.is_empty(), "{:?}", g.skipped);
    let f = &g.fields[0];
    assert_eq!(f.param, Param::WindU);
    assert_eq!((f.grid.ni, f.grid.nj), (64, 60));
    assert_eq!(f.valid_time - f.ref_time, 6 * 3600);
    let v: Vec<f64> = f.values.iter().filter(|x| x.is_finite()).map(|&x| x as f64).collect();
    assert_eq!(v.len(), valid);
    let m = v.iter().sum::<f64>() / v.len() as f64;
    assert!((m - mean).abs() < 1e-4, "{m} vs {mean}");
    // canonical order: rows south to north; the file's first value is the north-west corner
    assert!((f.values[59 * 64] as f64 - nw).abs() < 1e-4, "{}", f.values[59 * 64]);
    match se {
        Some(x) => assert!((f.values[63] as f64 - x).abs() < 1e-4),
        None => assert!(f.values[63].is_nan()),
    }
    g
}

#[test]
fn ccsds() {
    let g = check("ccsds_10u.grb2", 3840, 5.063749249776205, 10.146573066711426, Some(-0.3031339645385742));
    assert_eq!(g.model().0, "ECMWF");
}

#[test]
fn ccsds_with_bitmap() {
    check("ccsds_10u_bitmap.grb2", 3291, 5.065102650234671, 10.146573066711426, None);
}

#[test]
fn grib1_ecmwf_table_128_with_bitmap() {
    let g = check("grib1_ecmwf_10u_bitmap.grb", 3291, 5.065102650234671, 10.146573066711426, None);
    assert_eq!(g.model().0, "ECMWF");
}
