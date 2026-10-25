//! Maritime boundaries and fishing rules, built from downloads into GeoPackages the
//! chart store reads (like the GSHHG coastline):
//!
//! - **Maritime boundaries** (`<dir>/boundaries/*.zip`): LINZ's shapefile zips of the
//!   territorial sea (12 NM), contiguous zone (24 NM), exclusive economic zone (200 NM),
//!   the boundaries agreed with Australia, and the outlying islands' territorial seas.
//!   Written as lines in `mbound_lines`, with what each line is in `kind`.
//! - **Fishing rules** (`<dir>/mpi/*.json`): MPI's commercial fishing regulations,
//!   customary areas and voluntary closures as downloaded (see `gribdl::mpi`). Written as
//!   areas in `mpi_rules` (big ones cut into pieces), their outlines in `mpi_rules_ln`,
//!   and every attribute in `mpi_rules_attr` for the details shown on a click. MPI's
//!   fisheries management and statistical areas go to `mpi_areas_ln` (outlines with
//!   their code).
//!
//! Longitudes west of 0 are moved to 180–360°E, as LINZ does: everything here is
//! New Zealand's, and its boundaries cross 180°.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde_json::Value;

use crate::geom::{Pt, Ring};
use crate::gpkgw;
use crate::ChartError;

/// Bump when the built files change shape.
const BUILD_VERSION: i64 = 1;
/// Areas bigger than this many points are cut into pieces (a tile reads only its piece).
const MAX_PIECE: usize = 4000;
const LINE_CHUNK: usize = 1500;

pub const BOUNDARY_FILE: &str = "maritime_boundaries.gpkg";
pub const RULES_FILE: &str = "fishing_rules.gpkg";

/// What a boundary line is.
pub const BOUNDARY_KINDS: &[(&str, &str)] = &[
    ("ts", "Territorial sea limit (12 NM)"),
    ("cz", "Contiguous zone limit (24 NM)"),
    ("eez", "Exclusive economic zone limit (200 NM)"),
    ("cs", "Continental shelf limit"),
    ("base", "Territorial sea baseline"),
    ("agreed", "Boundary agreed with Australia"),
    ("other", "Maritime boundary"),
];

/// Build (when their sources are new or changed) the boundary and rules GeoPackages
/// from the downloads under `dir`, into `out_dir`. Returns the files there are.
pub fn ensure_gpkgs(dir: &Path, out_dir: &Path) -> Result<Vec<PathBuf>, ChartError> {
    let mut out = Vec::new();
    let zips = files(&dir.join("boundaries"), "zip");
    if !zips.is_empty() {
        let f = out_dir.join(BOUNDARY_FILE);
        if !up_to_date(&f, &zips) {
            std::fs::create_dir_all(out_dir)?;
            build_boundaries(&zips, &f)?;
        }
        out.push(f);
    }
    let jsons = files(&dir.join("mpi"), "json");
    if !jsons.is_empty() {
        let f = out_dir.join(RULES_FILE);
        if !up_to_date(&f, &jsons) {
            std::fs::create_dir_all(out_dir)?;
            build_rules(&jsons, &f)?;
        }
        out.push(f);
    }
    Ok(out)
}

fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// A note of the sources (names, sizes, times) a built file came from.
fn stamp(sources: &[PathBuf]) -> String {
    sources
        .iter()
        .map(|p| {
            let m = std::fs::metadata(p).ok();
            let t = m
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!(
                "{}:{}:{t}",
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                m.map_or(0, |m| m.len())
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn up_to_date(built: &Path, sources: &[PathBuf]) -> bool {
    let Ok(c) = Connection::open_with_flags(built, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return false;
    };
    let got: Option<(i64, String)> = c
        .query_row("SELECT version, stamp FROM racenav_build", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
    got == Some((BUILD_VERSION, stamp(sources)))
}

fn finish(db: Connection, sources: &[PathBuf], tmp: &Path, out: &Path) -> Result<(), ChartError> {
    db.execute(
        "CREATE TABLE racenav_build (version INTEGER, stamp TEXT)",
        [],
    )?;
    db.execute(
        "INSERT INTO racenav_build VALUES (?1, ?2)",
        params![BUILD_VERSION, stamp(sources)],
    )?;
    drop(db);
    // the table notes beside an old copy no longer match it
    let _ = std::fs::remove_file(out.with_extension("gpkg.tables"));
    std::fs::rename(tmp, out)?;
    Ok(())
}

fn east(p: Pt) -> Pt {
    if p[0] < 0.0 {
        [p[0] + 360.0, p[1]]
    } else {
        p
    }
}

// ---------- maritime boundaries -----------------------------------------------------

/// What a boundary shapefile holds, from its name, the zip's name and its attributes.
fn boundary_kind(zip: &str, shp: &str, attrs: &str) -> &'static str {
    let has = |s: &str, words: &[&str]| words.iter().any(|w| s.contains(w));
    for s in [shp, attrs] {
        let s = s.to_ascii_lowercase();
        if has(&s, &["baseline", "base_line", "basepoint"]) {
            return "base";
        }
        if has(
            &s,
            &[
                "contig", "24nm", "24 nm", "24_nm", "24mile", "24 mile", "24cz", "_cz",
            ],
        ) {
            return "cz";
        }
        if has(&s, &["shelf", "ecs", "350"]) {
            return "cs";
        }
        if has(
            &s,
            &["eez", "exclusive", "200nm", "200 nm", "200mile", "200 mile"],
        ) {
            return "eez";
        }
        if has(
            &s,
            &[
                "territorial",
                "12nm",
                "12 nm",
                "12mile",
                "12 mile",
                "_ts",
                "tsea",
            ],
        ) {
            return "ts";
        }
    }
    let z = zip.to_ascii_lowercase();
    if z.contains("aus") {
        "agreed"
    } else if z.contains("24cz") || z.contains("contig") {
        "cz"
    } else if z.contains("eez") {
        "eez"
    } else if z.starts_with("shp") {
        // ShpNZ and the islands (ShpCHA, ShpAUC...): territorial seas
        "ts"
    } else {
        "other"
    }
}

/// Island or area name from the zip (`ShpCHA.zip` → Chatham Islands).
fn boundary_area(zip: &str) -> &'static str {
    let z = zip.to_ascii_uppercase();
    for (code, name) in [
        ("CHA", "Chatham Islands"),
        ("AUC", "Auckland Islands"),
        ("ANT", "Antipodes Islands"),
        ("BOU", "Bounty Islands"),
        ("CAM", "Campbell Island / Motu Ihupuku"),
        ("TKI", "Three Kings Islands"),
        ("RAO", "Raoul Island"),
        ("MAC", "Macauley Island"),
        ("CCI", "Curtis and Cheeseman Islands"),
        ("ESP", "L'Esperance Rock"),
        ("SNA", "Snares Islands / Tini Heke"),
        ("AUS", "Australia–New Zealand"),
    ] {
        if z.contains(&format!("SHP{code}")) {
            return name;
        }
    }
    "New Zealand"
}

/// The shapefiles in a zip: (base name, shp, dbf, prj).
type ShpSet = (String, Vec<u8>, Option<Vec<u8>>, Option<String>);

fn zip_shapefiles(zip: &Path) -> Result<Vec<ShpSet>, ChartError> {
    let mut za = zip::ZipArchive::new(std::fs::File::open(zip)?)?;
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..za.len() {
        let mut f = za.by_index(i)?;
        let name = f.name().map(|n| n.to_string()).unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        if [".shp", ".dbf", ".prj"].iter().any(|x| lower.ends_with(x)) {
            let mut v = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut v)?;
            files.insert(lower, v);
        }
    }
    let mut out = Vec::new();
    let mut names: Vec<&String> = files.keys().filter(|k| k.ends_with(".shp")).collect();
    names.sort();
    for shp in names {
        let base = &shp[..shp.len() - 4];
        let short = base.rsplit('/').next().unwrap_or(base).to_string();
        out.push((
            short,
            files[shp].clone(),
            files.get(&format!("{base}.dbf")).cloned(),
            files
                .get(&format!("{base}.prj"))
                .map(|b| String::from_utf8_lossy(b).into_owned()),
        ));
    }
    Ok(out)
}

fn build_boundaries(zips: &[PathBuf], out: &Path) -> Result<(), ChartError> {
    let tmp = out.with_extension("gpkg.part");
    let mut db = gpkgw::create(&tmp)?;
    gpkgw::feature_table(
        &db,
        "mbound_lines",
        "LINESTRING",
        &["kind TEXT", "name TEXT"],
    )?;
    let tx = db.transaction()?;
    {
        let mut ins =
            tx.prepare("INSERT INTO mbound_lines (fid, geom, kind, name) VALUES (?1, ?2, ?3, ?4)")?;
        let mut rt =
            tx.prepare("INSERT INTO rtree_mbound_lines_geom VALUES (?1, ?2, ?3, ?4, ?5)")?;
        let mut fid = 0i64;
        for z in zips {
            let zname = z
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // the "raw" zips hold the same limits as text coordinates
            if zname.to_ascii_lowercase().starts_with("raw") {
                continue;
            }
            let area = boundary_area(&zname);
            let sets = match zip_shapefiles(z) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("maritime boundaries: {zname}: {e}");
                    continue;
                }
            };
            for (name, shp, dbf, prj) in sets {
                for s in crate::shp::read(&shp, dbf.as_deref(), prj.as_deref()) {
                    let words: String = s.attrs.iter().map(|(k, v)| format!("{k} {v} ")).collect();
                    let kind = boundary_kind(&zname, &name, &words);
                    let label = s
                        .attr("name")
                        .or(s.attr("descriptio"))
                        .or(s.attr("descr"))
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            format!(
                                "{area}: {}",
                                BOUNDARY_KINDS
                                    .iter()
                                    .find(|k| k.0 == kind)
                                    .map_or("", |k| k.1)
                            )
                        });
                    for part in &s.parts {
                        let pts: Vec<Pt> = part.iter().map(|&p| east(p)).collect();
                        for c in gpkgw::chunks(&pts, LINE_CHUNK) {
                            fid += 1;
                            let b = gpkgw::bounds(c);
                            ins.execute(params![fid, gpkgw::line(c), kind, label])?;
                            rt.execute(params![fid, b[0], b[2], b[1], b[3]])?;
                        }
                    }
                }
            }
        }
    }
    tx.commit()?;
    finish(db, zips, &tmp, out)
}

// ---------- fishing rules -----------------------------------------------------------

/// Gear flags in MPI's attributes, and the short names used in the tiles.
pub const GEAR: &[(&str, &str, &str)] = &[
    ("BottomTrawl", "bt", "Bottom trawl"),
    ("MidwaterTrawl", "mt", "Midwater trawl"),
    ("DanishSeine", "ds", "Danish seine"),
    ("PurseSeine", "ps", "Purse seine"),
    ("BeachSeineDragNet", "bs", "Beach seine / drag net"),
    ("SetNet", "sn", "Set net"),
    ("Longline", "ll", "Longline"),
    ("Potting", "pt", "Potting"),
    ("Dredge", "dr", "Dredge"),
    ("Troll", "tr", "Troll"),
    ("Handlining", "hl", "Handline"),
    ("HandDive", "hd", "Hand gathering / diving"),
];

/// Columns of `mpi_rules` and `mpi_rules_ln` (all small integers) besides fid and geom.
const RULE_COLS: &[&str] = &[
    "rid", "src", "prohib", "allp", "v46", "vlim", "glim", "seas", "tmp", "spec", "bt", "mt", "ds",
    "ps", "bs", "sn", "ll", "pt", "dr", "tr", "hl", "hd",
];

/// The rule columns as the tiles carry them (`rid` first).
pub fn rule_cols() -> &'static [&'static str] {
    RULE_COLS
}

/// Where a rule came from: the `src` column.
pub const SOURCES: &[(&str, u8)] = &[("regs", 1), ("customary", 2), ("voluntary", 3)];

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "1" | "y" | "yes" | "true"
        ),
        _ => false,
    }
}

/// A field, whatever the case of its name.
fn field<'a>(props: &'a serde_json::Map<String, Value>, name: &str) -> Option<&'a Value> {
    props.get(name).or_else(|| {
        props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    })
}

fn text(props: &serde_json::Map<String, Value>, name: &str) -> String {
    match field(props, name) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// The key a regulation is known by across MPI's layers: its identifier, else its name.
fn rule_key(props: &serde_json::Map<String, Value>) -> String {
    let id = text(props, "Identifier");
    if !id.is_empty() {
        return format!("id:{id}");
    }
    format!("name:{}", text(props, "Name").to_ascii_lowercase())
}

/// Polygons of a GeoJSON geometry, each as rings (exterior first).
fn geojson_polygons(g: &Value) -> Vec<Vec<Ring>> {
    let ring = |r: &Value| -> Ring {
        r.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some(east([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])))
                    .collect()
            })
            .unwrap_or_default()
    };
    let poly = |p: &Value| -> Vec<Ring> {
        p.as_array()
            .map(|a| a.iter().map(ring).filter(|r| r.len() >= 4).collect())
            .unwrap_or_default()
    };
    let c = &g["coordinates"];
    match g["type"].as_str() {
        Some("Polygon") => vec![poly(c)]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect(),
        Some("MultiPolygon") => c
            .as_array()
            .map(|a| a.iter().map(poly).filter(|p| !p.is_empty()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Lines of a GeoJSON geometry (for areas given as lines).
fn geojson_lines(g: &Value) -> Vec<Vec<Pt>> {
    let line = |l: &Value| -> Vec<Pt> {
        l.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some(east([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])))
                    .collect()
            })
            .unwrap_or_default()
    };
    let c = &g["coordinates"];
    match g["type"].as_str() {
        Some("LineString") => vec![line(c)],
        Some("MultiLineString") => c
            .as_array()
            .map(|a| a.iter().map(line).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Cut a polygon into pieces of at most [`MAX_PIECE`] points (exactly the same area),
/// halving its box along the longer side.
pub fn split_polygon(rings: Vec<Ring>) -> Vec<Vec<Ring>> {
    let n: usize = rings.iter().map(Vec::len).sum();
    let b = gpkgw::bounds(rings.iter().flatten());
    if n <= MAX_PIECE || (b[2] - b[0]).max(b[3] - b[1]) < 0.01 {
        return vec![rings];
    }
    use clipper2_rust::{
        boolean_op_tree_64, poly_tree_to_faces64, ClipType, FillRule, Path64, Paths64, Point64,
        PolyTree64,
    };
    const K: f64 = 1e7;
    let to = |r: &Ring| -> Path64 {
        r.iter()
            .map(|p| Point64::new((p[0] * K).round() as i64, (p[1] * K).round() as i64))
            .collect()
    };
    let back = |r: &Path64| -> Ring {
        let mut v: Ring = r.iter().map(|p| [p.x as f64 / K, p.y as f64 / K]).collect();
        if let Some(&f) = v.first() {
            v.push(f);
        }
        v
    };
    let subj: Paths64 = rings.iter().map(to).collect();
    let halves = if b[2] - b[0] >= b[3] - b[1] {
        let m = (b[0] + b[2]) / 2.0;
        [[b[0], b[1], m, b[3]], [m, b[1], b[2], b[3]]]
    } else {
        let m = (b[1] + b[3]) / 2.0;
        [[b[0], b[1], b[2], m], [b[0], m, b[2], b[3]]]
    };
    let mut out = Vec::new();
    for h in halves {
        // a little beyond the box, so the pieces overlap by nothing but share their cut
        let rect: Path64 = vec![[h[0], h[1]], [h[2], h[1]], [h[2], h[3]], [h[0], h[3]]]
            .into_iter()
            .map(|p| Point64::new((p[0] * K).round() as i64, (p[1] * K).round() as i64))
            .collect();
        let mut tree = PolyTree64::new();
        boolean_op_tree_64(
            ClipType::Intersection,
            FillRule::EvenOdd,
            &subj,
            &vec![rect],
            &mut tree,
        );
        for face in poly_tree_to_faces64(&tree) {
            let mut p = vec![back(&face.outer)];
            p.extend(face.holes.iter().map(back));
            if p[0].len() >= 4 {
                out.extend(split_polygon(p));
            }
        }
    }
    out
}

/// One rule as written: its flags (in [`RULE_COLS`] order after `rid`) and geometry.
struct Rule {
    flags: Vec<i64>,
    polys: Vec<Vec<Ring>>,
    lines: Vec<Vec<Pt>>,
    attrs: serde_json::Map<String, Value>,
    layers: Vec<String>,
}

fn rule_flags(
    src: u8,
    props: &serde_json::Map<String, Value>,
    prohib_member: bool,
    v46: bool,
) -> Vec<i64> {
    let b = |name: &str| truthy(field(props, name)) as i64;
    let allp = b("AllProhibited");
    let mut f = vec![
        src as i64,
        (prohib_member || allp == 1 || src == 2) as i64,
        allp,
        v46 as i64,
        b("VesselLimit"),
        b("GearLimit"),
        b("Seasonal"),
        b("Temporary_").max(b("Temporary")),
        b("SpeciesProhibition"),
    ];
    // MPI's method codes (voluntary closures): "BT, BPT, MW, MPT, SN"
    let codes: Vec<String> = text(props, "MethodCode").split([',', ' ', ';', '/']).filter(|c| !c.is_empty()).map(|c| c.to_ascii_uppercase()).collect();
    let coded = |short: &str| -> bool {
        let names: &[&str] = match short {
            "bt" => &["BT", "BPT", "TRAWL"],
            "mt" => &["MW", "MPT", "MWT", "TRAWL"],
            "ds" => &["DS"],
            "ps" => &["PS", "RN"],
            "bs" => &["BS", "DN"],
            "sn" => &["SN", "SETNET"],
            "ll" => &["BLL", "SLL", "LL", "DL", "TL"],
            "pt" => &["RLP", "CP", "FP", "POT"],
            "dr" => &["D", "DR", "DREDGE"],
            "tr" => &["T", "TR", "TROLL"],
            "hl" => &["HL", "H"],
            "hd" => &["DI", "DV", "HAND"],
            _ => &[],
        };
        codes.iter().any(|c| names.contains(&c.as_str()) || c == "ALL")
    };
    for (name, short, _) in GEAR {
        f.push((b(name) == 1 || coded(short)) as i64);
    }
    f
}

fn build_rules(jsons: &[PathBuf], out: &Path) -> Result<(), ChartError> {
    let mut rules: Vec<Rule> = Vec::new();
    let mut areas: Vec<(String, String, Vec<Vec<Pt>>)> = Vec::new(); // (type, code, outlines)
    for j in jsons {
        let v: Value = match std::fs::read(j)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            Some(v) => v,
            None => {
                eprintln!("fishing rules: can't read {}", j.display());
                continue;
            }
        };
        let key = v["key"].as_str().unwrap_or("");
        let layers = v["layers"].as_array().cloned().unwrap_or_default();
        match key {
            "regs" => rules.extend(regs_rules(&layers)),
            "areas" => {
                for l in &layers {
                    let lname = l["name"].as_str().unwrap_or("").to_string();
                    for f in l["features"]["features"].as_array().into_iter().flatten() {
                        let props = f["properties"].as_object().cloned().unwrap_or_default();
                        let code = ["FmaId", "StatisticalAreaCode", "Name", "FMA", "STAT_AREA", "StatArea", "Code", "Identifier"]
                            .iter()
                            .map(|n| text(&props, n))
                            .find(|s| !s.is_empty())
                            .unwrap_or_default();
                        let mut lines: Vec<Vec<Pt>> = geojson_polygons(&f["geometry"])
                            .into_iter()
                            .flatten()
                            .collect();
                        lines.extend(geojson_lines(&f["geometry"]));
                        areas.push((lname.clone(), code, lines));
                    }
                }
            }
            _ => {
                let src = SOURCES.iter().find(|s| s.0 == key).map_or(3, |s| s.1);
                for l in &layers {
                    let lname = l["name"].as_str().unwrap_or("").to_string();
                    for f in l["features"]["features"].as_array().into_iter().flatten() {
                        let props = f["properties"].as_object().cloned().unwrap_or_default();
                        rules.push(Rule {
                            flags: rule_flags(src, &props, false, false),
                            polys: geojson_polygons(&f["geometry"]),
                            lines: geojson_lines(&f["geometry"]),
                            attrs: props,
                            layers: vec![lname.clone()],
                        });
                    }
                }
            }
        }
    }

    let tmp = out.with_extension("gpkg.part");
    let mut db = gpkgw::create(&tmp)?;
    let cols: Vec<String> = RULE_COLS.iter().map(|c| format!("{c} INTEGER")).collect();
    let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
    gpkgw::feature_table(&db, "mpi_rules", "POLYGON", &cols)?;
    gpkgw::feature_table(&db, "mpi_rules_ln", "LINESTRING", &cols)?;
    gpkgw::feature_table(
        &db,
        "mpi_areas_ln",
        "LINESTRING",
        &["kind TEXT", "code TEXT"],
    )?;
    db.execute_batch(
        "CREATE TABLE mpi_rules_attr (rid INTEGER PRIMARY KEY, layers TEXT, attrs TEXT);
         INSERT INTO gpkg_contents (table_name, data_type, identifier) VALUES ('mpi_rules_attr', 'attributes', 'mpi_rules_attr');",
    )?;
    let tx = db.transaction()?;
    {
        let holders = (0..RULE_COLS.len())
            .map(|i| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let names = RULE_COLS.join(", ");
        let mut ins = tx.prepare(&format!(
            "INSERT INTO mpi_rules (fid, geom, {names}) VALUES (?1, ?2, {holders})"
        ))?;
        let mut ins_ln = tx.prepare(&format!(
            "INSERT INTO mpi_rules_ln (fid, geom, {names}) VALUES (?1, ?2, {holders})"
        ))?;
        let mut rt = tx.prepare("INSERT INTO rtree_mpi_rules_geom VALUES (?1, ?2, ?3, ?4, ?5)")?;
        let mut rt_ln =
            tx.prepare("INSERT INTO rtree_mpi_rules_ln_geom VALUES (?1, ?2, ?3, ?4, ?5)")?;
        let mut attr = tx.prepare("INSERT INTO mpi_rules_attr VALUES (?1, ?2, ?3)")?;
        let (mut fid, mut lfid) = (0i64, 0i64);
        for (i, r) in rules.into_iter().enumerate() {
            let rid = i as i64 + 1;
            let mut vals: Vec<i64> = vec![rid];
            vals.extend(&r.flags);
            let p: Vec<&dyn rusqlite::ToSql> =
                vals.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
            for poly in r.polys {
                // outlines from the whole area (the pieces' cuts would draw as lines)
                for ring in &poly {
                    for c in gpkgw::chunks(ring, LINE_CHUNK) {
                        lfid += 1;
                        let b = gpkgw::bounds(c);
                        let mut a: Vec<&dyn rusqlite::ToSql> = vec![&lfid];
                        let blob = gpkgw::line(c);
                        a.push(&blob);
                        a.extend(p.iter().copied());
                        ins_ln.execute(a.as_slice())?;
                        rt_ln.execute(params![lfid, b[0], b[2], b[1], b[3]])?;
                    }
                }
                for piece in split_polygon(poly) {
                    fid += 1;
                    let b = gpkgw::bounds(piece.iter().flatten());
                    let blob = gpkgw::polygon(&piece);
                    let mut a: Vec<&dyn rusqlite::ToSql> = vec![&fid, &blob];
                    a.extend(p.iter().copied());
                    ins.execute(a.as_slice())?;
                    rt.execute(params![fid, b[0], b[2], b[1], b[3]])?;
                }
            }
            for line in r.lines {
                for c in gpkgw::chunks(&line, LINE_CHUNK) {
                    lfid += 1;
                    let b = gpkgw::bounds(c);
                    let blob = gpkgw::line(c);
                    let mut a: Vec<&dyn rusqlite::ToSql> = vec![&lfid, &blob];
                    a.extend(p.iter().copied());
                    ins_ln.execute(a.as_slice())?;
                    rt_ln.execute(params![lfid, b[0], b[2], b[1], b[3]])?;
                }
            }
            attr.execute(params![
                rid,
                r.layers.join("; "),
                Value::Object(r.attrs).to_string()
            ])?;
        }
        let mut ins_a =
            tx.prepare("INSERT INTO mpi_areas_ln (fid, geom, kind, code) VALUES (?1, ?2, ?3, ?4)")?;
        let mut rt_a =
            tx.prepare("INSERT INTO rtree_mpi_areas_ln_geom VALUES (?1, ?2, ?3, ?4, ?5)")?;
        let mut afid = 0i64;
        for (kind, code, lines) in areas {
            for l in lines {
                for c in gpkgw::chunks(&l, LINE_CHUNK) {
                    afid += 1;
                    let b = gpkgw::bounds(c);
                    ins_a.execute(params![afid, gpkgw::line(c), kind, code])?;
                    rt_a.execute(params![afid, b[0], b[2], b[1], b[3]])?;
                }
            }
        }
    }
    tx.commit()?;
    finish(db, jsons, &tmp, out)
}

/// The commercial fishing regulations: every regulation from MPI's "all regulations"
/// layer, with the categories it is in from the other layers (matched by identifier,
/// else name). Regulations a category layer has but the "all" layer doesn't are added.
fn regs_rules(layers: &[Value]) -> Vec<Rule> {
    let mut members: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for l in layers {
        let id = l["id"].as_i64().unwrap_or(-1);
        let name = l["name"].as_str().unwrap_or("").to_string();
        for m in l["members"].as_array().into_iter().flatten() {
            if let Some(o) = m.as_object() {
                members
                    .entry(rule_key(o))
                    .or_default()
                    .push((id, name.clone()));
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut take = |f: &Value, extra: Option<(i64, String)>, out: &mut Vec<Rule>| {
        let props = f["properties"].as_object().cloned().unwrap_or_default();
        let key = rule_key(&props);
        let mut cats = members.get(&key).cloned().unwrap_or_default();
        if let Some(e) = extra {
            if !cats.contains(&e) {
                cats.push(e);
            }
        }
        // the same regulation can come back from several layers: once is enough
        if !seen.insert((key, f["geometry"].to_string().len())) {
            return;
        }
        let prohib = cats.iter().any(|(id, n)| *id == 1 || n.to_ascii_lowercase().contains("prohibition"))
            || text(&props, "Name").to_ascii_lowercase().contains(" prohibited");
        let v46 = cats.iter().any(|(id, n)| *id == 18 || n.contains("46"));
        out.push(Rule {
            flags: rule_flags(1, &props, prohib, v46),
            polys: geojson_polygons(&f["geometry"]),
            lines: geojson_lines(&f["geometry"]),
            layers: cats
                .into_iter()
                .map(|c| c.1)
                .filter(|n| !n.starts_with("All "))
                .collect(),
            attrs: props,
        });
    };
    for l in layers.iter().filter(|l| l["features"].is_object()) {
        let extra = (
            l["id"].as_i64().unwrap_or(-1),
            l["name"].as_str().unwrap_or("").to_string(),
        );
        let extra = (extra.0 > 1).then_some(extra);
        for f in l["features"]["features"].as_array().into_iter().flatten() {
            take(f, extra.clone(), &mut out);
        }
    }
    out
}

/// A fishing rule at a place: its id and flags (by [`RULE_COLS`] name, non-zero only).
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct RuleHit {
    pub rid: i64,
    pub flags: Vec<(String, i64)>,
}

impl crate::ChartStore {
    /// The fishing rules whose areas take in each of `points` ([lon, lat]), in order.
    pub fn rules_at(&self, points: &[Pt]) -> Vec<Vec<RuleHit>> {
        use geo::Contains;
        let mut out = vec![Vec::new(); points.len()];
        for t in self.tables.iter().filter(|t| t.kind == crate::Kind::Rule) {
            for (k, &p) in points.iter().enumerate() {
                let p = east(p);
                let d = 1e-6;
                self.query(
                    t,
                    [p[0] - d, p[1] - d, p[0] + d, p[1] + d],
                    RULE_COLS,
                    |vals, g| {
                        let crate::geom::Geom::Polygons(polys) = g else {
                            return;
                        };
                        let pt = geo::Point::new(p[0], p[1]);
                        let inside = polys.iter().any(|rings| {
                            let ring = |r: &Ring| {
                                geo::LineString::from(
                                    r.iter().map(|q| (q[0], q[1])).collect::<Vec<_>>(),
                                )
                            };
                            geo::Polygon::new(
                                ring(&rings[0]),
                                rings[1..].iter().map(ring).collect(),
                            )
                            .contains(&pt)
                        });
                        if !inside {
                            return;
                        }
                        let rid = vals[0].unwrap_or(0.0) as i64;
                        if out[k].iter().any(|h: &RuleHit| h.rid == rid) {
                            return; // two pieces of one area meet here
                        }
                        let flags = RULE_COLS[1..]
                            .iter()
                            .zip(&vals[1..])
                            .filter_map(|(c, v)| {
                                v.filter(|v| *v != 0.0).map(|v| (c.to_string(), v as i64))
                            })
                            .collect();
                        out[k].push(RuleHit { rid, flags });
                    },
                );
            }
        }
        out
    }

    /// A fishing rule's details: the MPI layers it is in and all its attributes.
    pub fn rule_info(&self, rid: i64) -> Option<(String, Value)> {
        let t = self.tables.iter().find(|t| t.kind == crate::Kind::Rule)?;
        let conn = self.dbs[t.db].lock().unwrap_or_else(|e| e.into_inner());
        let (layers, attrs): (String, String) = conn
            .query_row(
                "SELECT layers, attrs FROM mpi_rules_attr WHERE rid = ?1",
                [rid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        Some((layers, serde_json::from_str(&attrs).unwrap_or(Value::Null)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn square(x: f64, y: f64, s: f64) -> Value {
        json!({"type": "Polygon", "coordinates": [[[x, y], [x + s, y], [x + s, y + s], [x, y + s], [x, y]]]})
    }

    #[test]
    fn classifies_boundary_files() {
        assert_eq!(boundary_kind("ShpNZ.zip", "nz_12mile", ""), "ts");
        assert_eq!(boundary_kind("ShpNZ.zip", "baseline", ""), "base");
        assert_eq!(boundary_kind("24CZ.zip", "contiguous", ""), "cz");
        assert_eq!(boundary_kind("eez_website.zip", "nz_eez", ""), "eez");
        assert_eq!(boundary_kind("eez_website.zip", "outer", ""), "eez");
        assert_eq!(boundary_kind("ShpAUS-NZ.zip", "boundary", ""), "agreed");
        assert_eq!(boundary_kind("ShpCHA.zip", "cha", ""), "ts");
        assert_eq!(boundary_area("ShpCHA.zip"), "Chatham Islands");
    }

    #[test]
    fn splits_big_areas_without_losing_any() {
        // a jagged ring of 12 000 points with a hole
        let n = 12_000;
        let outer: Ring = (0..=n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                let r = 1.0 + 0.05 * (a * 50.0).sin();
                [174.0 + r * a.cos(), -41.0 + r * a.sin()]
            })
            .collect();
        let hole: Ring = vec![
            [173.9, -41.1],
            [173.9, -40.9],
            [174.1, -40.9],
            [174.1, -41.1],
            [173.9, -41.1],
        ];
        let area = |r: &Ring| {
            (0..r.len() - 1)
                .map(|k| r[k][0] * r[k + 1][1] - r[k + 1][0] * r[k][1])
                .sum::<f64>()
                .abs()
                / 2.0
        };
        let whole = area(&outer) - area(&hole);
        let pieces = split_polygon(vec![outer, hole]);
        assert!(pieces.len() >= 4, "{}", pieces.len());
        assert!(pieces
            .iter()
            .all(|p| p.iter().map(Vec::len).sum::<usize>() <= MAX_PIECE + 200));
        let sum: f64 = pieces
            .iter()
            .map(|p| area(&p[0]) - p[1..].iter().map(area).sum::<f64>())
            .sum();
        assert!((sum - whole).abs() / whole < 1e-6, "{sum} v {whole}");
    }

    #[test]
    fn builds_rules_with_categories() {
        let dir = std::env::temp_dir().join(format!("tp_rules_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mpi")).unwrap();
        let regs = json!({"key": "regs", "layers": [
            {"id": 0, "name": "All Commercial Fishing Regulations", "features": {"type": "FeatureCollection", "features": [
                {"type": "Feature", "geometry": square(174.0, -36.5, 0.2), "properties": {"Name": "Hauraki trawl", "Identifier": "R1", "BottomTrawl": 1, "MidwaterTrawl": 1}},
                {"type": "Feature", "geometry": square(-176.5, -44.0, 0.2), "properties": {"Name": "Chathams set net", "Identifier": "R2", "SetNet": 1, "Seasonal": 1, "SeasonStart": "1 October"}}
            ]}},
            {"id": 17, "name": "Trawl prohibitions", "members": [{"Identifier": "R1", "Name": "Hauraki trawl"}]},
            {"id": 18, "name": "Trawl prohibitions for vessels over 46m", "members": [{"Identifier": "R1"}]}
        ]});
        std::fs::write(dir.join("mpi/regs.json"), regs.to_string()).unwrap();
        let cust = json!({"key": "customary", "layers": [{"id": 0, "name": "Mātaitai", "features": {"type": "FeatureCollection", "features": [
            {"type": "Feature", "geometry": square(173.2, -41.2, 0.05), "properties": {"Name": "Delaware Bay mātaitai"}}
        ]}}]});
        std::fs::write(dir.join("mpi/customary.json"), cust.to_string()).unwrap();
        let out = dir.join(".unzipped");
        let built = ensure_gpkgs(&dir, &out).unwrap();
        assert_eq!(built, vec![out.join(RULES_FILE)]);
        let c = Connection::open(&built[0]).unwrap();
        let row: (i64, i64, i64, i64, i64) = c
            .query_row("SELECT prohib, v46, bt, mt, sn FROM mpi_rules r JOIN mpi_rules_attr a ON a.rid = r.rid WHERE a.attrs LIKE '%Hauraki%'", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        assert_eq!(row, (1, 1, 1, 1, 0));
        let layers: String = c
            .query_row(
                "SELECT layers FROM mpi_rules_attr WHERE attrs LIKE '%Hauraki%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            layers.contains("Trawl prohibitions for vessels over 46m"),
            "{layers}"
        );
        // the Chathams rule east of 180° is stored at 183.5°E
        let w: f64 = c.query_row("SELECT minx FROM rtree_mpi_rules_geom r JOIN mpi_rules m ON m.fid = r.id JOIN mpi_rules_attr a ON a.rid = m.rid WHERE a.attrs LIKE '%Chathams%'", [], |r| r.get(0)).unwrap();
        assert!((w - 183.5).abs() < 1e-9, "{w}");
        let (seas, sn): (i64, i64) = c.query_row("SELECT seas, sn FROM mpi_rules r JOIN mpi_rules_attr a ON a.rid = r.rid WHERE a.attrs LIKE '%Chathams%'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((seas, sn), (1, 1));
        // customary areas: commercial fishing closed (prohib), source 2
        let (src, prohib): (i64, i64) = c.query_row("SELECT src, prohib FROM mpi_rules r JOIN mpi_rules_attr a ON a.rid = r.rid WHERE a.attrs LIKE '%Delaware%'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((src, prohib), (2, 1));
        // the outlines are there, and a second call doesn't rebuild
        let n: i64 = c
            .query_row("SELECT count(*) FROM mpi_rules_ln", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
        drop(c);
        let t0 = std::fs::metadata(&built[0]).unwrap().modified().unwrap();
        ensure_gpkgs(&dir, &out).unwrap();
        assert_eq!(
            std::fs::metadata(&built[0]).unwrap().modified().unwrap(),
            t0
        );
        // the store finds the rules at a place, and their details
        let store = crate::ChartStore::open_dirs(&[dir.clone()]).unwrap();
        let hits = store.rules_at(&[[174.1, -36.4], [-176.4, -43.9], [170.0, -40.0]]);
        assert_eq!(hits[0].len(), 1);
        assert!(hits[0][0].flags.contains(&("v46".to_string(), 1)));
        assert_eq!(
            hits[1].len(),
            1,
            "east of 180°, asked for at a western longitude"
        );
        assert!(hits[2].is_empty());
        let (layers, attrs) = store.rule_info(hits[0][0].rid).unwrap();
        assert!(layers.contains("Trawl prohibitions"));
        assert_eq!(attrs["Name"], "Hauraki trawl");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builds_boundaries_from_zips() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("tp_bound_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("boundaries")).unwrap();
        let shp = crate::shp::tests::polyline_shp(&[vec![[-178.0, -44.0], [-176.0, -45.0]]]);
        let dbf = crate::shp::tests::dbf("NAME", &[""]);
        let mut z =
            zip::ZipWriter::new(std::fs::File::create(dir.join("boundaries/24CZ.zip")).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("24CZ/cz_line.shp", o).unwrap();
        z.write_all(&shp).unwrap();
        z.start_file("24CZ/cz_line.dbf", o).unwrap();
        z.write_all(&dbf).unwrap();
        z.finish().unwrap();
        let out = dir.join(".unzipped");
        let built = ensure_gpkgs(&dir, &out).unwrap();
        let c = Connection::open(&built[0]).unwrap();
        let (kind, name): (String, String) = c
            .query_row("SELECT kind, name FROM mbound_lines", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(kind, "cz");
        assert!(name.contains("Contiguous"), "{name}");
        let w: f64 = c
            .query_row("SELECT minx FROM rtree_mbound_lines_geom", [], |r| r.get(0))
            .unwrap();
        assert_eq!(w, 182.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
