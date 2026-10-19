//! Convert every ENC cell under a folder and report: `enc_all <folder>`.
fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("folder"));
    for cf in chart::enc::find_cells(&dir) {
        let rel = cf.base.strip_prefix(&dir).unwrap().display().to_string();
        match chart::enc::cell::Cell::open(&cf.base, &cf.updates) {
            Ok(c) => {
                let with = c.features.values().filter(|f| c.shape(f).is_some()).count();
                let geo = c.features.values().filter(|f| f.prim != 255).count();
                println!("OK   {rel}  +{} upd  ed {} upd {} usage {} 1:{}  {} features ({}/{} geometry)", cf.updates.len(), c.edition, c.update, c.intu, c.cscl, c.features.len(), with, geo);
            }
            Err(e) => println!("FAIL {rel}: {e}"),
        }
    }
}
