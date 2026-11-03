//! IHO S-64 display test data through TweedPlot: for each ENC cell, every object class
//! with what TweedPlot keeps of it (its table, or nothing), the S-101 feature it becomes
//! and what the IHO rules draw for it (display category, symbols, lines, fills), and
//! GeoPackages of the cells for looking at them in the chart (`tileserve`).
//!
//! `cargo run --release -p chart --example s64_display -- <out dir> <cell.000>...`
//!
//! Categories: B base, S standard, O other (from the rules' viewing groups).
use std::collections::BTreeMap;
use std::path::Path;

use portrayal::{Catalogue, Converter, FeatureCatalogue, Geometry, Portrayer, S57Object};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let out = Path::new(&a[0]);
    std::fs::create_dir_all(out).unwrap();
    let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources");
    let cat = Catalogue::open(&res.join("s101-pc")).unwrap();
    let fc_text = std::fs::read_to_string(res.join("s101-fc/101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml")).unwrap();
    let fc = FeatureCatalogue::parse(&fc_text).unwrap();
    let conv = Converter::new(&fc);
    let mut p = Portrayer::new(&cat, FeatureCatalogue::parse(&fc_text).unwrap()).unwrap();
    for (k, v) in [("SafetyContour", "10"), ("SafetyDepth", "10"), ("ShallowWaterDangers", "false")] {
        p.set(k, v);
    }
    for cell_path in &a[1..] {
        let cell = enc_s57::cell::Cell::open(Path::new(cell_path), &[]).unwrap();
        let band = enc_s57::gpkg::band_for(cell.intu);
        let gpkg = out.join(format!("{}.gpkg", cell.name));
        let _ = std::fs::remove_file(&gpkg);
        let n = enc_s57::gpkg::write_gpkg(&cell, &gpkg).unwrap();
        println!("\n## {} (usage {}, band {band}, 1:{}): {n} features written", cell.name, cell.intu, cell.cscl);
        let mut feats = Vec::new();
        let mut info: BTreeMap<String, (String, String)> = BTreeMap::new(); // id -> (class, table)
        let mut topmarks: Vec<([f64; 2], S57Object)> = Vec::new();
        for f in cell.features.values() {
            let Some(acr) = enc_s57::catalogue::object(f.objl) else { continue };
            if acr.starts_with("M_") || acr.starts_with("C_") {
                continue;
            }
            let attrs: Vec<(String, String)> = f.attf.iter().filter_map(|(k, v)| Some((enc_s57::catalogue::attribute(*k)?.0.to_string(), v.clone()))).collect();
            let g = match cell.shape(f) {
                Some(enc_s57::cell::Shape::Points(pt)) if acr == "SOUNDG" => Geometry::MultiPoint(pt),
                Some(enc_s57::cell::Shape::Points(pt)) => Geometry::Point([pt[0][0], pt[0][1]]),
                Some(enc_s57::cell::Shape::Lines(l)) => Geometry::Curve(l[0].clone()),
                Some(enc_s57::cell::Shape::Polygons(pg)) => Geometry::Surface(pg[0].clone()),
                None => continue,
            };
            let table = enc_s57::gpkg::table_for(acr, f.prim, band).unwrap_or_else(|| "DROPPED".into());
            let prim = ["", "pt", "ln", "ar"][f.prim.min(3) as usize];
            let o = S57Object { class: acr.to_string(), attrs, geometry: Some(g) };
            if acr == "TOPMAR" {
                if let Some(Geometry::Point(q)) = &o.geometry {
                    topmarks.push((*q, o.clone()));
                }
            }
            let id = format!("{acr}/{}", f.rcid);
            info.insert(id.clone(), (format!("{acr} {prim}"), table));
            feats.push((id, o));
        }
        let mut features = Vec::new();
        for (id, o) in &feats {
            let mut f = conv.convert(id, o, None);
            if let Some(Geometry::Point(q)) = &o.geometry {
                if o.class.starts_with("BOY") || o.class.starts_with("BCN") {
                    if let Some((_, t)) = topmarks.iter().find(|(tq, _)| (tq[0] - q[0]).abs() < 1e-7 && (tq[1] - q[1]).abs() < 1e-7) {
                        conv.add_topmark(&mut f, t);
                    }
                }
            }
            features.push(f);
        }
        let mut trace = Vec::new();
        let outp = p.portray(&features, &mut trace).unwrap();
        let by: BTreeMap<&str, &str> = outp.iter().map(|o| (o.id.as_str(), o.instructions.as_str())).collect();
        // S64_DUMP=LIGHTS,BOYLAT: each such object's drawing instructions
        if let Ok(d) = std::env::var("S64_DUMP") {
            for o in &outp {
                if d.split(',').any(|c| o.id.starts_with(&format!("{c}/"))) {
                    println!("{} {}", o.id, o.instructions);
                }
            }
        }
        // per class: count, table, S-101 code, categories, a sample of what is drawn
        struct Row {
            n: usize,
            table: String,
            code: String,
            cats: BTreeMap<char, usize>,
            draws: BTreeMap<String, usize>,
        }
        let mut rows: BTreeMap<String, Row> = BTreeMap::new();
        for f in &features {
            let (cls, table) = &info[&f.id];
            let ins = by.get(f.id.as_str()).copied().unwrap_or("");
            let c = match chart::s52::category(ins) {
                1 => 'B',
                2 => 'S',
                3 => 'O',
                _ => '-',
            };
            let d = chart::s52::drawn(ins, &cat);
            let mut draw = Vec::new();
            if !d.symbols.is_empty() {
                draw.push(d.symbols.clone());
            }
            if let Some((n, _)) = &d.line {
                draw.push(format!("line {n}"));
            }
            if let Some((t, _)) = &d.fill {
                draw.push(format!("fill {t}"));
            }
            if let Some(af) = &d.pattern {
                draw.push(format!("pattern {af}"));
            }
            if ins.contains("TextInstruction") {
                draw.push("text".into());
            }
            if ins.contains("ArcByRadius") || ins.contains("AugmentedRay") {
                draw.push("sectors".into());
            }
            let r = rows.entry(cls.clone()).or_insert(Row { n: 0, table: table.clone(), code: f.code.clone(), cats: BTreeMap::new(), draws: BTreeMap::new() });
            r.n += 1;
            *r.cats.entry(c).or_default() += 1;
            *r.draws.entry(if draw.is_empty() { "NOTHING".into() } else { draw.join(" + ") }).or_default() += 1;
        }
        for (cls, r) in &rows {
            let cats: Vec<String> = r.cats.iter().map(|(c, n)| format!("{c}{n}")).collect();
            let draws: Vec<String> = r.draws.iter().take(3).map(|(d, n)| format!("{n}× {d}")).collect();
            let table = r.table.trim_end_matches(&format!("_enc_b{band}")).trim_end_matches("_hydro");
            println!("{cls:10} {:4}  {table:38} {:24} {:10} {}", r.n, r.code, cats.join(","), draws.join(" | "));
        }
        if !trace.is_empty() {
            println!("rule messages: {}", trace.len());
            for t in trace.iter().take(5) {
                println!("  {}", t.chars().take(160).collect::<String>());
            }
        }
    }
}
