//! Crop a GRIB2 file to an area and write it with simple packing: crop <in> <out> w,s,e,n
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let b: Vec<f64> = a[2].split(',').map(|x| x.parse().unwrap()).collect();
    let (fields, skipped) = grib::GribFile::decode_fields(&std::fs::read(&a[0]).unwrap());
    let mut out = Vec::new();
    for f in &fields {
        if let Some(c) = grib::write::crop(f, [b[0], b[1], b[2], b[3]]) {
            out.extend(grib::write::encode(&c, 0));
        }
    }
    std::fs::write(&a[1], &out).unwrap();
    println!("{} fields → {} bytes; skipped {}", fields.len(), out.len(), skipped.len());
}
