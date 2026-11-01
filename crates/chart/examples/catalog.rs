//! Cells in an exchange set's CATALOG.031 touching a box: `catalog <CATALOG.031> w s e n`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let m = chart::enc::iso8211::Module::parse(&std::fs::read(&a[0]).unwrap()).unwrap();
    let b: Vec<f64> = a[1..5].iter().map(|v| v.parse().unwrap()).collect();
    for r in &m.records {
        let Some(f) = r.field("CATD") else { continue };
        let g = |l: &str| {
            f.get(l).and_then(|v| match v {
                chart::enc::iso8211::Val::Float(x) => Some(*x),
                chart::enc::iso8211::Val::Int(i) => Some(*i as f64),
                chart::enc::iso8211::Val::Text(t) => t.trim().parse().ok(),
                _ => None,
            })
        };
        let file = f.get("FILE").and_then(|v| v.text()).unwrap_or("");
        if !file.ends_with(".000") {
            continue;
        }
        if std::env::var_os("NOBOUNDS").is_some() && g("SLAT").is_none() {
            println!("no bounds: {file}");
        }
        if let (Some(s), Some(w), Some(n), Some(e)) = (g("SLAT"), g("WLON"), g("NLAT"), g("ELON")) {
            if w <= b[2] && e >= b[0] && s <= b[3] && n >= b[1] {
                println!("{file} {s} {w} {n} {e}");
            }
        }
    }
}
