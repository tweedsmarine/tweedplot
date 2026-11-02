//! Portray every feature in a folder of LINZ (or ENC-converted) GeoPackages with the IHO
//! rules, and report what they drew: `linz_portray <folder> [safety contour]`.
use std::collections::BTreeMap;
use std::path::Path;

use portrayal::{Catalogue, Converter, FeatureCatalogue, Geometry, Portrayer, S57Object};

fn res(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources").join(p)
}

/// The S-57 class of a LINZ table: the chart's classes, and a few it doesn't draw.
fn class(table: &str) -> Option<String> {
    if let Some(c) = enc_s57::classes::class_for(table) {
        return Some(c.code.to_string());
    }
    let t = table.to_ascii_lowercase();
    let extra = [
        ("topmark_points", "TOPMAR"),
        ("seabed_area_points", "SBDARE"),
        ("water_turbulence_points", "WATTUR"),
        ("unsurveyed_area_polygon", "UNSARE"),
        ("wreck_polygon", "WRECKS"),
        ("anchorage_area_points", "ACHARE"),
    ];
    extra.iter().find(|(p, _)| t.starts_with(p)).map(|(_, c)| c.to_string())
}

fn rd(b: &[u8], i: &mut usize, le: bool) -> f64 {
    let a: [u8; 8] = b[*i..*i + 8].try_into().unwrap();
    *i += 8;
    if le {
        f64::from_le_bytes(a)
    } else {
        f64::from_be_bytes(a)
    }
}
fn ru(b: &[u8], i: &mut usize, le: bool) -> u32 {
    let a: [u8; 4] = b[*i..*i + 4].try_into().unwrap();
    *i += 4;
    if le {
        u32::from_le_bytes(a)
    } else {
        u32::from_be_bytes(a)
    }
}
/// GeoPackage geometry (WKB after the header) as the first point, line or polygon.
fn geom(blob: &[u8]) -> Option<Geometry> {
    let env = (blob[3] >> 1) & 7;
    let mut i = 8 + [0, 32, 48, 48, 64][env as usize];
    fn one(b: &[u8], i: &mut usize) -> Option<Geometry> {
        let le = b[*i] == 1;
        *i += 1;
        let ty = ru(b, i, le) % 1000;
        let pt = |i: &mut usize| [rd(b, i, le), rd(b, i, le)];
        Some(match ty {
            1 => Geometry::Point(pt(i)),
            2 => Geometry::Curve((0..ru(b, i, le)).map(|_| pt(i)).collect()),
            3 => Geometry::Surface((0..ru(b, i, le)).map(|_| (0..ru(b, i, le)).map(|_| pt(i)).collect()).collect()),
            4..=6 => {
                let n = ru(b, i, le);
                if n == 0 {
                    return None;
                }
                one(b, i)?
            }
            _ => return None,
        })
    }
    one(blob, &mut i)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cat = Catalogue::open(&res("s101-pc")).unwrap();
    let fc = FeatureCatalogue::parse(&std::fs::read_to_string(res("s101-fc/101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml")).unwrap()).unwrap();
    let conv_fc = FeatureCatalogue::parse(&std::fs::read_to_string(res("s101-fc/101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml")).unwrap()).unwrap();
    let conv = Converter::new(&conv_fc);
    let mut objs: Vec<(String, S57Object)> = Vec::new();
    for e in std::fs::read_dir(&args[0]).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("gpkg") {
            continue;
        }
        let db = rusqlite::Connection::open(&p).unwrap();
        let tables: Vec<String> = db.prepare("select table_name from gpkg_contents").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
        for t in tables {
            let Some(cls) = class(&t) else { continue };
            let cols: Vec<String> = db.prepare(&format!("pragma table_info(\"{t}\")")).unwrap().query_map([], |r| r.get(1)).unwrap().flatten().collect();
            let mut st = db.prepare(&format!("select * from \"{t}\"")).unwrap();
            let mut rows = st.query([]).unwrap();
            let mut n = 0;
            while let Some(r) = rows.next().unwrap() {
                let mut attrs = Vec::new();
                let mut g = None;
                for (i, c) in cols.iter().enumerate() {
                    use rusqlite::types::ValueRef;
                    match r.get_ref(i).unwrap() {
                        ValueRef::Blob(b) if c == "geom" => g = geom(b),
                        ValueRef::Text(s) => attrs.push((c.clone(), String::from_utf8_lossy(s).into_owned())),
                        ValueRef::Integer(x) if c != "fid" => attrs.push((c.clone(), x.to_string())),
                        ValueRef::Real(x) => attrs.push((c.clone(), x.to_string())),
                        _ => {}
                    }
                }
                n += 1;
                objs.push((format!("{t}/{n}"), S57Object { class: cls.clone(), attrs, geometry: g }));
            }
        }
    }
    // topmarks onto the buoy or beacon at the same place; sector lights at one place together
    let key = |o: &S57Object| match &o.geometry {
        Some(Geometry::Point(p)) => Some(((p[0] * 1e5).round() as i64, (p[1] * 1e5).round() as i64)),
        _ => None,
    };
    let mut topmarks: BTreeMap<(i64, i64), S57Object> = BTreeMap::new();
    let mut sectors: BTreeMap<(i64, i64), Vec<S57Object>> = BTreeMap::new();
    let mut features = Vec::new();
    for (id, o) in &objs {
        if o.class == "TOPMAR" {
            if let Some(k) = key(o) {
                topmarks.insert(k, o.clone());
            }
        } else if o.class == "LIGHTS" && conv.feature_type(o).is_some_and(|f| f.code == "LightSectored") {
            sectors.entry(key(o).unwrap()).or_default().push(o.clone());
        } else {
            let _ = id;
        }
    }
    let mut used_top = 0;
    for (id, o) in &objs {
        if o.class == "TOPMAR" || (o.class == "LIGHTS" && conv.feature_type(o).is_some_and(|f| f.code == "LightSectored")) {
            continue;
        }
        let mut f = conv.convert(id, o, None);
        if let Some(t) = key(o).and_then(|k| topmarks.get(&k)) {
            if f.attrs.is_empty() || o.class.starts_with("BOY") || o.class.starts_with("BCN") {
                conv.add_topmark(&mut f, t);
                if f.attrs.contains_key("topmark") {
                    used_top += 1;
                }
            }
        }
        features.push(f);
    }
    for (k, recs) in &sectors {
        features.push(conv.sectored_light(&format!("sector/{}/{}", k.0, k.1), recs));
    }
    let mut p = Portrayer::new(&cat, fc).unwrap();
    p.set("SafetyContour", args.get(1).map(String::as_str).unwrap_or("5"));
    let t0 = std::time::Instant::now();
    let mut trace = Vec::new();
    let out = p.portray(&features, &mut trace).unwrap();
    println!(
        "{} objects, {} features ({} with topmarks, {} sector lights), portrayed in {:.2} s",
        objs.len(),
        features.len(),
        used_top,
        sectors.len(),
        t0.elapsed().as_secs_f64()
    );
    let by_id: BTreeMap<&str, &str> = out.iter().map(|o| (o.id.as_str(), o.instructions.as_str())).collect();
    let mut stats: BTreeMap<String, (usize, usize, String)> = BTreeMap::new();
    for f in &features {
        let ins = by_id.get(f.id.as_str()).copied().unwrap_or("");
        let default = ins.contains("QUESMRK") || ins.is_empty();
        let e = stats.entry(f.code.clone()).or_insert((0, 0, String::new()));
        e.0 += 1;
        if default {
            e.1 += 1;
        }
        if e.2.is_empty() || (default && !e.2.contains("QUESMRK")) {
            e.2 = ins.chars().take(260).collect();
        }
    }
    for (code, (n, d, ex)) in &stats {
        println!("{code:28} {n:5}  default {d:4}  e.g. {ex}");
    }
    let mut errs: BTreeMap<String, usize> = BTreeMap::new();
    for t in &trace {
        let t: String = t.split(" ID=").next().unwrap_or(t).chars().take(200).collect();
        *errs.entry(t).or_default() += 1;
    }
    println!("--- {} trace lines", trace.len());
    for (t, n) in errs.iter().take(40) {
        println!("{n:5} {t}");
    }
}
