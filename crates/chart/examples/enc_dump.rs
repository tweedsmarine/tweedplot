//! Read an S-57 cell (with any updates beside it), print a summary of its features and
//! write the GeoPackage the chart store would use: `enc_dump <cell.000> [out.gpkg]`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let base = std::path::PathBuf::from(&a[0]);
    let dir = base.parent().unwrap().to_path_buf();
    let cells = chart::enc::find_cells(&dir);
    let cf = cells.iter().find(|c| c.base == base).expect("cell");
    let c = chart::enc::cell::Cell::open(&cf.base, &cf.updates).expect("read");
    println!(
        "{} edition {} update {} usage {} scale 1:{} comf {} somf {} ({} updates)",
        c.name,
        c.edition,
        c.update,
        c.intu,
        c.cscl,
        c.comf,
        c.somf,
        cf.updates.len()
    );
    println!("{} vector records, {} features", c.vectors.len(), c.features.len());
    let mut counts: std::collections::BTreeMap<(String, u8), (usize, usize)> = Default::default();
    for f in c.features.values() {
        let acr = chart::enc::catalogue::object(f.objl).unwrap_or("?").to_string();
        let e = counts.entry((acr, f.prim)).or_default();
        e.0 += 1;
        if c.shape(f).is_some() {
            e.1 += 1;
        }
    }
    for ((a, p), (n, g)) in counts {
        println!("  {a:7} prim {p:3}  {n:4} features, {g:4} with geometry");
    }
    if let Some(out) = a.get(1) {
        let n = chart::enc::write_gpkg(&c, std::path::Path::new(out)).expect("write");
        println!("wrote {n} features to {out}");
    }
}
