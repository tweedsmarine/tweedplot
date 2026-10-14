//! Print a polar summary: `cargo run -p polar --example polar_info -- <file>`
fn main() {
    let path = std::env::args().nth(1).expect("usage: polar_info <polar file>");
    let p = polar::Polar::from_path(&path).expect("read polar");
    println!("{}: {} TWA rows x {} TWS columns", p.name, p.twa.len(), p.tws.len());
    println!(" TWS   up TWA  up BSP  up VMG | down TWA down BSP down VMG");
    for &tws in &p.tws {
        let u = p.best_vmg_up(tws, 30.0);
        let d = p.best_vmg_down(tws);
        println!("{tws:4.0}  {:7.1} {:7.2} {:7.2} | {:8.1} {:8.2} {:8.2}", u.twa, u.bsp, u.vmg, d.twa, d.bsp, d.vmg);
    }
}
