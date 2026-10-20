//! S-57 ENC cells (`.000` with updates `.001`, `.002` ...) for the chart store.
//!
//! Each cell found under a chart folder is converted once into a GeoPackage in that
//! folder's `.enc` subfolder, laid out like the LINZ layers (`light_points_hydro_enc_b4`
//! and so on, S-57 attribute acronyms as columns), so the store draws ENC cells with the
//! same code as the LINZ data. A cell is converted again when it or an update changes,
//! or when the converter changes ([`VERSION`]). The band comes from the cell's intended
//! usage (overview 2, general 3, coastal 4, approach 5, harbour and berthing 6).

pub mod catalogue;
pub mod cell;
pub mod iso8211;
pub mod s63;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use cell::{Cell, Shape};

/// Bump when the conversion changes, so cells are converted again.
pub const VERSION: u32 = 2;
/// Bump when merging changes, so the merged files are made again (the cells needn't be).
pub const MERGE_VERSION: u32 = 2;

/// The chart band for a cell's intended usage (DSID INTU).
pub fn band_for(intu: u8) -> u8 {
    match intu {
        1 => 2,
        2 => 3,
        3 => 4,
        4 => 5,
        5 | 6 => 6,
        _ => 4,
    }
}

/// The table a feature goes in, or None for those the chart doesn't draw.
fn table_for(acr: &str, prim: u8, band: u8) -> Option<String> {
    let geo = match prim {
        1 => crate::s57::Geo::Point,
        2 => crate::s57::Geo::Line,
        3 => crate::s57::Geo::Area,
        _ => return None,
    };
    let fixed = match (acr, prim) {
        ("DEPARE" | "DRGARE", 3) => Some("depth_area_polygon_hydro"),
        ("DEPCNT", 2) => Some("depth_contour_polyline_hydro"),
        ("SOUNDG", 1) => Some("sounding_points_hydro"),
        ("M_COVR", 3) => Some("coverage_polygon_hydro"),
        ("LNDARE", 3) => Some("land_area_polygon"),
        ("LAKARE", 3) => Some("lake_polygon_hydro"),
        ("RIVERS", 3) => Some("river_polygon_hydro"),
        _ => None,
    };
    if let Some(p) = fixed {
        return Some(format!("{p}_enc_b{band}"));
    }
    let c = crate::s57::CLASSES.iter().find(|c| c.code == acr && c.geo == geo)?;
    Some(format!("{}_hydro_enc_b{band}", c.prefix))
}

#[derive(Default)]
struct Table {
    geo: &'static str,
    cols: BTreeMap<String, char>,
    rows: Vec<(Vec<(String, String)>, Vec<u8>, [f64; 4])>,
}

/// Write a cell's features into a new GeoPackage at `out`. Returns the features written.
pub fn write_gpkg(c: &Cell, out: &Path) -> Result<usize, rusqlite::Error> {
    let band = band_for(c.intu);
    let mut tables: BTreeMap<String, Table> = BTreeMap::new();
    let mut n = 0;
    for f in c.features.values() {
        let Some(acr) = catalogue::object(f.objl) else { continue };
        let Some(tname) = table_for(acr, f.prim, band) else { continue };
        let Some(shape) = c.shape(f) else { continue };
        let mut vals: Vec<(String, String)> = Vec::new();
        for (k, v) in f.attf.iter().chain(&f.natf) {
            if let Some((a, ty)) = catalogue::attribute(*k) {
                let ty = match ty {
                    b'F' => 'R',
                    b'I' => 'I',
                    _ => 'T',
                };
                vals.push((a.to_ascii_lowercase(), v.trim().to_string()));
                let t = tables.entry(tname.clone()).or_default();
                t.cols.insert(a.to_ascii_lowercase(), ty);
            }
        }
        let t = tables.entry(tname.clone()).or_default();
        match shape {
            Shape::Points(pts) if acr == "SOUNDG" => {
                t.geo = "POINT";
                t.cols.insert("depth".into(), 'R');
                for p in pts {
                    let mut v = vals.clone();
                    v.push(("depth".into(), format!("{}", p[2])));
                    t.rows.push((v, gp_point(p[0], p[1]), [p[0], p[0], p[1], p[1]]));
                }
            }
            Shape::Points(pts) => {
                t.geo = "POINT";
                let p = pts[0];
                t.rows.push((vals, gp_point(p[0], p[1]), [p[0], p[0], p[1], p[1]]));
            }
            Shape::Lines(ls) => {
                t.geo = "MULTILINESTRING";
                let env = envelope(ls.iter().flatten());
                t.rows.push((vals, gp_lines(&ls, env), env));
            }
            Shape::Polygons(ps) => {
                t.geo = "MULTIPOLYGON";
                let env = envelope(ps.iter().flat_map(|p| p[0].iter()));
                t.rows.push((vals, gp_polys(&ps, env), env));
            }
        }
        n += 1;
    }

    let _ = std::fs::remove_file(out);
    let tmp = out.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut db = Connection::open(&tmp)?;
    db.execute_batch(
        "PRAGMA application_id = 1196444487; PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
         CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
           organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
         INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]', NULL);
         CREATE TABLE gpkg_contents (table_name TEXT PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT, description TEXT DEFAULT '',
           last_change DATETIME, min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
           srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));
         CREATE TABLE enc_meta (key TEXT PRIMARY KEY, value TEXT);",
    )?;
    let tx = db.transaction()?;
    for (k, v) in [
        ("version", VERSION.to_string()),
        ("cell", c.name.clone()),
        ("edition", c.edition.clone()),
        ("update", c.update.clone()),
        ("issue_date", c.issue_date.clone()),
        ("intended_usage", c.intu.to_string()),
        ("compilation_scale", c.cscl.to_string()),
    ] {
        tx.execute("INSERT INTO enc_meta VALUES (?1, ?2)", rusqlite::params![k, v])?;
    }
    for (name, t) in &tables {
        let cols: String = t.cols.iter().map(|(c, ty)| format!(", \"{c}\" {}", match ty { 'R' => "REAL", 'I' => "INTEGER", _ => "TEXT" })).collect();
        tx.execute_batch(&format!(
            "CREATE TABLE \"{name}\" (fid INTEGER PRIMARY KEY, geom BLOB{cols});
             CREATE VIRTUAL TABLE \"rtree_{name}_geom\" USING rtree(id, minx, maxx, miny, maxy);"
        ))?;
        tx.execute("INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) VALUES (?1, 'features', ?1, 4326)", [name])?;
        tx.execute("INSERT INTO gpkg_geometry_columns VALUES (?1, 'geom', ?2, 4326, 0, 0)", rusqlite::params![name, t.geo])?;
        let names: Vec<&String> = t.cols.keys().collect();
        let ph: String = (0..names.len()).map(|i| format!(", ?{}", i + 3)).collect();
        let cl: String = names.iter().map(|c| format!(", \"{c}\"")).collect();
        let mut ins = tx.prepare(&format!("INSERT INTO \"{name}\" (fid, geom{cl}) VALUES (?1, ?2{ph})"))?;
        let mut rt = tx.prepare(&format!("INSERT INTO \"rtree_{name}_geom\" VALUES (?1, ?2, ?3, ?4, ?5)"))?;
        for (i, (vals, blob, env)) in t.rows.iter().enumerate() {
            let fid = i as i64 + 1;
            let mut params: Vec<rusqlite::types::Value> = vec![fid.into(), blob.clone().into()];
            for c in &names {
                let v = vals.iter().find(|(k, _)| k == *c).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty());
                params.push(match (v, t.cols[*c]) {
                    (None, _) => rusqlite::types::Value::Null,
                    (Some(v), 'R') => v.parse::<f64>().map(Into::into).unwrap_or(rusqlite::types::Value::Null),
                    (Some(v), 'I') => v.parse::<i64>().map(Into::into).unwrap_or(rusqlite::types::Value::Null),
                    (Some(v), _) => v.to_string().into(),
                });
            }
            ins.execute(rusqlite::params_from_iter(params))?;
            rt.execute(rusqlite::params![fid, env[0], env[1], env[2], env[3]])?;
        }
    }
    tx.commit()?;
    drop(db);
    std::fs::rename(&tmp, out).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(n)
}

fn envelope<'a>(pts: impl Iterator<Item = &'a [f64; 2]>) -> [f64; 4] {
    let mut e = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
    for p in pts {
        e = [e[0].min(p[0]), e[1].max(p[0]), e[2].min(p[1]), e[3].max(p[1])];
    }
    e
}

fn header(env: Option<[f64; 4]>) -> Vec<u8> {
    let mut b = vec![b'G', b'P', 0, if env.is_some() { 0x03 } else { 0x01 }];
    b.extend_from_slice(&4326i32.to_le_bytes());
    if let Some(e) = env {
        for v in e {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b
}

fn put_pts(b: &mut Vec<u8>, pts: &[[f64; 2]]) {
    b.extend_from_slice(&(pts.len() as u32).to_le_bytes());
    for p in pts {
        b.extend_from_slice(&p[0].to_le_bytes());
        b.extend_from_slice(&p[1].to_le_bytes());
    }
}

fn gp_point(x: f64, y: f64) -> Vec<u8> {
    let mut b = header(None);
    b.push(1);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&x.to_le_bytes());
    b.extend_from_slice(&y.to_le_bytes());
    b
}

fn gp_lines(ls: &[Vec<[f64; 2]>], env: [f64; 4]) -> Vec<u8> {
    let mut b = header(Some(env));
    b.push(1);
    b.extend_from_slice(&5u32.to_le_bytes());
    b.extend_from_slice(&(ls.len() as u32).to_le_bytes());
    for l in ls {
        b.push(1);
        b.extend_from_slice(&2u32.to_le_bytes());
        put_pts(&mut b, l);
    }
    b
}

fn gp_polys(ps: &[Vec<Vec<[f64; 2]>>], env: [f64; 4]) -> Vec<u8> {
    let mut b = header(Some(env));
    b.push(1);
    b.extend_from_slice(&6u32.to_le_bytes());
    b.extend_from_slice(&(ps.len() as u32).to_le_bytes());
    for p in ps {
        b.push(1);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&(p.len() as u32).to_le_bytes());
        for r in p {
            put_pts(&mut b, r);
        }
    }
    b
}

/// A base cell and its updates, in order.
pub struct CellFiles {
    pub base: PathBuf,
    pub updates: Vec<PathBuf>,
}

/// Every ENC base cell under `dir` (not in hidden folders), with its updates.
pub fn find_cells(dir: &Path) -> Vec<CellFiles> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if p.is_dir() {
                if !hidden {
                    stack.push(p);
                }
            } else {
                files.push(p);
            }
        }
        let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        // S-63 signature files share the cell's name with the usage digit as a letter I-N
        let is_signature = |p: &Path| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            n.chars().nth(2).is_some_and(|c| ('I'..='N').contains(&c)) && std::fs::read(p).is_ok_and(|b| b.starts_with(b"//") || b.starts_with(b"\xef\xbb\xbf//"))
        };
        for b in files.iter().filter(|p| ext(p) == "000" && !is_signature(p)) {
            let stem = b.file_stem().map(|s| s.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            let mut updates: Vec<PathBuf> = files
                .iter()
                .filter(|p| {
                    let e = ext(p);
                    e.len() == 3 && e != "000" && e.bytes().all(|c| c.is_ascii_digit()) && p.file_stem().is_some_and(|s| s.to_string_lossy().to_ascii_uppercase() == stem)
                })
                .cloned()
                .collect();
            // an exchange set's layout, ENC_ROOT/<agency>/<cell>/<edition>/<update>/: the
            // updates are in the edition folder's other update folders
            let numeric = |p: Option<&Path>| p.and_then(|p| p.file_name()).is_some_and(|n| n.to_string_lossy().bytes().all(|c| c.is_ascii_digit()));
            if numeric(b.parent()) && numeric(b.parent().and_then(|p| p.parent())) {
                let ed = b.parent().unwrap().parent().unwrap();
                for e in std::fs::read_dir(ed).into_iter().flatten().flatten() {
                    let d = e.path();
                    if !d.is_dir() || Some(d.as_path()) == b.parent() || !numeric(Some(&d)) {
                        continue;
                    }
                    for f in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                        let p = f.path();
                        let e = ext(&p);
                        if e.len() == 3 && e != "000" && e.bytes().all(|c| c.is_ascii_digit()) && p.file_stem().is_some_and(|s| s.to_string_lossy().to_ascii_uppercase() == stem) {
                            updates.push(p);
                        }
                    }
                }
            }
            updates.sort_by_key(|p| ext(p));
            updates.dedup_by_key(|p| ext(p));
            out.push(CellFiles { base: b.clone(), updates });
        }
    }
    out.sort_by(|a, b| a.base.cmp(&b.base));
    out
}

fn hash(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

/// Convert the ENC cells under `dir` that are new or changed into `dir/.enc`, and return
/// the GeoPackages of all of them. Cells that can't be read (encrypted S-63 cells
/// without a permit, damaged files) are skipped.
pub fn ensure_gpkgs(dir: &Path) -> Vec<PathBuf> {
    ensure_gpkgs_with(dir, &mut |_, _, _| {})
}

/// [`ensure_gpkgs`], telling `progress(converted so far, cells to convert, cell)` as it goes.
pub fn ensure_gpkgs_with(dir: &Path, progress: &mut dyn FnMut(usize, usize, &str)) -> Vec<PathBuf> {
    let cells = find_cells(dir);
    if cells.is_empty() {
        return vec![];
    }
    let out_dir = dir.join(".enc");
    let _ = std::fs::create_dir_all(&out_dir);
    // what is there already, read once (a big exchange set has thousands of cells)
    let existing: Vec<String> = std::fs::read_dir(&out_dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let suffix = format!("-v{VERSION}.gpkg");
    // each cell's own conversion: "<CELL>-<folder hash>-b<band>-v<version>.gpkg"
    let plan: Vec<(CellFiles, String, Option<PathBuf>)> = cells
        .into_iter()
        .map(|c| {
            let stem = c.base.file_stem().map(|s| s.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            let rel = c.base.parent().and_then(|p| p.strip_prefix(dir).ok()).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let key = format!("{stem}-{:08x}", hash(&rel));
            let have = existing.iter().find(|e| e.starts_with(&format!("{key}-b")) && e.ends_with(&suffix)).map(|e| out_dir.join(e));
            let newest = std::iter::once(&c.base).chain(&c.updates).filter_map(|p| mtime(p)).max();
            let fresh = have.filter(|g| matches!((newest, mtime(g)), (Some(n), Some(m)) if m >= n));
            (c, key, fresh)
        })
        .collect();
    let todo = plan.iter().filter(|p| p.2.is_none()).count();
    // S-63 cell permits from any PERMIT.TXT in the folder (only read when a cell needs converting)
    let permits = if todo > 0 { s63_permits(dir) } else { Default::default() };
    let mut done = 0;
    let mut by_band: BTreeMap<u8, Vec<PathBuf>> = BTreeMap::new();
    for (c, key, fresh) in plan {
        let gpkg = match fresh {
            Some(g) => g,
            None => {
                let name = c.base.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                progress(done, todo, &name);
                if done % 100 == 0 {
                    eprintln!("ENC: converting {} of {todo} ({name})", done + 1);
                }
                done += 1;
                // older conversions of this cell (other versions, notes)
                for e in existing.iter().filter(|e| e.starts_with(&format!("{key}-"))) {
                    let _ = std::fs::remove_file(out_dir.join(e));
                }
                let cell = match open_cell(&c, &permits) {
                    Ok(cell) => cell,
                    Err(e) => {
                        note(&c.base, e.to_string());
                        continue;
                    }
                };
                let g = out_dir.join(format!("{key}-b{}{suffix}", band_for(cell.intu)));
                if let Err(e) = write_gpkg(&cell, &g) {
                    eprintln!("ENC {}: {e}", c.base.display());
                    continue;
                }
                g
            }
        };
        let band = gpkg.file_name().and_then(|n| {
            let n = n.to_string_lossy().to_string();
            let i = n.rfind("-b")?;
            n[i + 2..].split('-').next()?.parse::<u8>().ok()
        });
        if let Some(b) = band {
            by_band.entry(b).or_default().push(gpkg);
        }
    }
    if todo > 0 {
        progress(todo, todo, "");
    }
    // then the cells of each band merged into one file, which is what the chart store
    // opens: one table per layer and band instead of one per cell (thousands of cells)
    let tag = format!("{:08x}", hash(&dir.to_string_lossy()));
    let mut out = Vec::new();
    for (band, files) in by_band {
        let merged = out_dir.join(format!("all-{tag}-b{band}-m{MERGE_VERSION}{suffix}"));
        let stamp: Vec<(String, i64)> = files
            .iter()
            .map(|f| {
                let t = mtime(f).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                (f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), t)
            })
            .collect();
        if !merged_is(&merged, &stamp) {
            progress(0, 0, &format!("merging band {band} ({} cells)", files.len()));
            eprintln!("ENC: merging {} cells of band {band}", files.len());
            if let Err(e) = merge(&files, &stamp, &merged) {
                eprintln!("ENC: merging band {band}: {e}");
                continue;
            }
        }
        out.push(merged);
    }
    // merged files of bands no longer present
    for e in existing.iter().filter(|e| e.starts_with(&format!("all-{tag}-"))) {
        let p = out_dir.join(e);
        if !out.iter().any(|o| p.starts_with(o) || e.starts_with(&o.file_name().unwrap().to_string_lossy().to_string())) {
            let _ = std::fs::remove_file(p);
        }
    }
    out
}

/// The S-63 credentials in use: the system's user permit and the manufacturer key. Until
/// TweedPlot has its own from the IHO, the IHO's generic S-64 test credentials.
static PROTECTION: std::sync::RwLock<Option<(String, String)>> = std::sync::RwLock::new(None);

/// The IHO's generic S-64 test credentials: user permit and manufacturer key (M_KEY).
pub const TEST_USER_PERMIT: &str = "66B5CBFDF7E4139D5B6086C23130";
pub const TEST_M_KEY: &str = "10121";

/// Set the user permit and manufacturer key (M_KEY) for decrypting S-63 cells.
pub fn set_protection(user_permit: &str, m_key: &str) {
    *PROTECTION.write().unwrap() = Some((user_permit.trim().to_ascii_uppercase(), m_key.to_string()));
}

/// The user permit and M_KEY in use.
pub fn protection() -> (String, String) {
    PROTECTION.read().unwrap().clone().unwrap_or_else(|| (TEST_USER_PERMIT.into(), TEST_M_KEY.into()))
}

/// The HW_ID from the user permit in use.
pub fn hw_id() -> Option<[u8; 5]> {
    let p = protection();
    s63::hw_id_from_user_permit(&p.0, &p.1).ok()
}

/// What happened to a cell that could not be opened, or opens with a warning (S-63 codes).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Note {
    pub file: String,
    pub cell: String,
    pub text: String,
}

static NOTES: std::sync::Mutex<Vec<Note>> = std::sync::Mutex::new(Vec::new());

fn note(file: &Path, text: impl Into<String>) {
    let text = text.into();
    eprintln!("ENC {}: {text}", file.display());
    let cell = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut n = NOTES.lock().unwrap();
    n.retain(|x| !(x.file == file.display().to_string() && x.text == text));
    n.push(Note { file: file.display().to_string(), cell, text });
}

/// Notes from converting cells since the charts were last opened.
pub fn notes() -> Vec<Note> {
    NOTES.lock().unwrap().clone()
}

/// Forget the notes (before the charts are opened again).
pub fn clear_notes() {
    NOTES.lock().unwrap().clear();
}

/// The IHO Scheme Administrator's public key (S-63), for checking cell signatures.
pub const SA_PUBLIC_KEY: &str = "// BIG p
FCA6 82CE 8E12 CABA 26EF CCF7 110E 526D B078 B05E DECB CD1E B4A2 08F3 AE16 17AE 01F3 5B91 A47E 6DF6 3413 C5E1 2ED0 899B CD13 2ACD 50D9 9151 BDC4 3EE7 3759 2E17.
// BIG q
962E DDCC 369C BA8E BB26 0EE6 B6A1 26D9 346E 38C5.
// BIG g
6784 71B2 7A9C F44E E91A 49C5 147D B1A9 AAF2 44F0 5A43 4D64 8693 1D2D 1427 1B9E 3503 0B71 FD73 DA17 9069 B32E 2935 630E 1C20 6235 4D0D A20A 6C41 6E50 BE79 4CA4.
// BIG y
963F 14E3 2BA5 3729 28F2 4F15 B073 0C49 D31B 28E5 C764 1002 564D B959 95B1 5CF8 800E D54E 3548 67B8 2BB9 597B 1582 69E0 79F0 C4F4 926B 1776 1CC8 9EB7 7C9B 7EF8.
";

/// One permit as found: where, its file's date, and the permit or why it can't be used.
#[derive(Debug, Clone)]
pub struct FoundPermit {
    pub file: PathBuf,
    /// the PERMIT.TXT :DATE, "YYYYMMDD HH:MM"
    pub date: String,
    pub cell: String,
    pub permit: Result<s63::CellPermit, s63::S63Error>,
}

fn permit_date(text: &str) -> String {
    text.lines().find_map(|l| l.trim().strip_prefix(":DATE").map(|d| d.trim().to_string())).unwrap_or_default()
}

/// Every cell permit in the PERMIT.TXT files under `dir` (installed permits and those that
/// came with exchange sets), checked against the HW_ID.
pub fn find_permits(dir: &Path) -> Vec<FoundPermit> {
    let mut out = Vec::new();
    let Some(hw) = hw_id() else { return out };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if p.is_dir() {
                // hidden folders are TweedPlot's own (".enc"), except the installed permits
                if !name.starts_with('.') || name == ".permits" {
                    stack.push(p);
                }
            } else if name.eq_ignore_ascii_case("PERMIT.TXT") {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let date = permit_date(&text);
                match s63::read_permits(&text, &hw) {
                    Ok(v) => out.extend(v.into_iter().map(|(cell, permit)| FoundPermit { file: p.clone(), date: date.clone(), cell, permit })),
                    Err(e) => note(&p, e.to_string()),
                }
            }
        }
    }
    out
}

/// The permit to use for each cell: a valid one from the newest permit file.
fn s63_permits(dir: &Path) -> std::collections::HashMap<String, s63::CellPermit> {
    let mut all = find_permits(dir);
    all.sort_by(|a, b| a.date.cmp(&b.date));
    let mut out = std::collections::HashMap::new();
    for f in all {
        if let Ok(p) = f.permit {
            out.insert(f.cell, p);
        }
    }
    out
}

/// Install a PERMIT.TXT: its valid permits replace older ones for the same cells in the
/// installed permits (`dir/.permits/PERMIT.TXT`). Returns each permit with what was done.
pub fn install_permits(dir: &Path, text: &str) -> Result<Vec<(String, String)>, s63::S63Error> {
    let hw = hw_id().ok_or(s63::S63Error::UserPermit)?;
    let found = s63::read_permits(text, &hw)?;
    let store = dir.join(".permits").join("PERMIT.TXT");
    let mut lines: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    if let Ok(old) = std::fs::read_to_string(&store) {
        for l in old.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(':')) {
            lines.insert(l.get(..8).unwrap_or(l).to_string(), l.to_string());
        }
    }
    let now = today();
    let mut report = Vec::new();
    let records: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(':')).collect();
    for ((cell, r), line) in found.into_iter().zip(records) {
        let what = match r {
            Ok(p) => {
                lines.insert(cell.clone(), line.to_string());
                if p.expiry < now {
                    "installed. SSE 15 - Subscription service has expired. Please contact your data supplier to renew the subscription licence".to_string()
                } else if days_between(&now, &p.expiry) <= 30 {
                    format!("installed, expires {}. SSE 20 - Subscription service will expire in less than 30 days", p.expiry)
                } else {
                    format!("installed, expires {}", p.expiry)
                }
            }
            Err(e) => format!("not installed. {e}"),
        };
        report.push((cell, what));
    }
    let _ = std::fs::create_dir_all(store.parent().unwrap());
    let stamp = {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        format!("{} {:02}:{:02}", now, (t / 3600) % 24, (t / 60) % 60)
    };
    let mut out = format!(":DATE {stamp}\r\n:VERSION 2\r\n:ENC\r\n");
    for l in lines.values() {
        out.push_str(l);
        out.push_str("\r\n");
    }
    std::fs::write(&store, out).map_err(|_| s63::S63Error::PermitFormat)?;
    Ok(report)
}

fn days(d: &str) -> i64 {
    let (y, m, dd): (i64, i64, i64) = (d.get(..4).and_then(|v| v.parse().ok()).unwrap_or(0), d.get(4..6).and_then(|v| v.parse().ok()).unwrap_or(1), d.get(6..8).and_then(|v| v.parse().ok()).unwrap_or(1));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + dd - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

fn days_between(a: &str, b: &str) -> i64 {
    days(b) - days(a)
}

/// Read a cell's file, decrypting it when it is S-63: its signature is checked against
/// the IHO key (an unauthenticated cell is not decrypted), then it is decrypted with
/// the cell's permit.
fn read_cell_file(path: &Path, permits: &std::collections::HashMap<String, s63::CellPermit>) -> Result<Vec<u8>, cell::CellError> {
    let data = std::fs::read(path)?;
    // an ISO 8211 file starts with its record length (five digits) and the leader
    if data.len() > 24 && data[..5].iter().all(u8::is_ascii_digit) && data[6] == b'L' {
        return Ok(data);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
    let other = |m: String| cell::CellError::Other(m);
    let permit = permits.get(name.get(..8).unwrap_or("")).ok_or_else(|| other("SSE 11 - Cell permit not found. Load the permit file provided by the data supplier".into()))?;
    let sig = s63::signature_name(&name).map(|s| path.with_file_name(s)).ok_or_else(|| other("SSE 26 - This ENC is not authenticated by the IHO acting as the Scheme Administrator (no signature file)".into()))?;
    let sig = std::fs::read(&sig).map_err(|_| other("SSE 26 - This ENC is not authenticated by the IHO acting as the Scheme Administrator (no signature file)".into()))?;
    let sa = s63::parse_public_key(SA_PUBLIC_KEY.as_bytes()).map_err(|e| other(e.to_string()))?;
    s63::verify_cell(&sig, &data, &sa).map_err(|e| other(e.to_string()))?;
    if permit.expiry.as_str() < today().as_str() {
        note(path, format!("SSE 25 - The permit for ENC {} has expired ({}). This cell may be out of date and MUST NOT be used for primary navigation", &name[..8], permit.expiry));
    }
    s63::decrypt_cell(&data, permit).map_err(|e| other(e.to_string()))
}

fn today() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86400) as i64;
    // days since 1970 to y-m-d (civil from days)
    let z = d + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!("{year:04}{month:02}{day:02}")
}

/// A cell and its updates, decrypting S-63 files. Updates are applied in sequence from
/// the base cell's own update number; a missing one stops them there (SSE 23).
fn open_cell(c: &CellFiles, permits: &std::collections::HashMap<String, s63::CellPermit>) -> Result<Cell, cell::CellError> {
    let base = read_cell_file(&c.base, permits)?;
    let first = Cell::from_bytes(&base, &[])?;
    let mut next: u32 = first.update.trim().parse::<u32>().unwrap_or(0) + 1;
    let mut ups = Vec::new();
    for u in &c.updates {
        let n: u32 = u.extension().and_then(|e| e.to_str()).and_then(|e| e.parse().ok()).unwrap_or(0);
        if n < next {
            continue; // already in a re-issued base cell
        }
        if n > next {
            note(u, format!("SSE 23 - Non sequential update, update {next} is missing: updates from {n} on are not applied"));
            break;
        }
        match read_cell_file(u, permits) {
            Ok(b) => ups.push(b),
            Err(e) => {
                // apply the updates up to the first one that can't be read
                note(u, e.to_string());
                break;
            }
        }
        next += 1;
    }
    let refs: Vec<&[u8]> = ups.iter().map(Vec::as_slice).collect();
    Cell::from_bytes(&base, &refs)
}

/// Whether a merged file was made from exactly these cell files (names and times).
fn merged_is(merged: &Path, stamp: &[(String, i64)]) -> bool {
    let Ok(db) = Connection::open_with_flags(merged, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return false };
    let Ok(mut st) = db.prepare("SELECT file, mtime FROM merged_from ORDER BY file") else { return false };
    let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) else { return false };
    let have: Vec<(String, i64)> = rows.filter_map(Result::ok).collect();
    let mut want = stamp.to_vec();
    want.sort();
    have == want
}

/// Merge cells' GeoPackages (same table layout) into one.
fn merge(files: &[PathBuf], stamp: &[(String, i64)], out: &Path) -> Result<(), rusqlite::Error> {
    let tmp = out.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let db = Connection::open(&tmp)?;
    db.execute_batch(
        "PRAGMA application_id = 1196444487; PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
         CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
           organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
         INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\"]', NULL);
         CREATE TABLE gpkg_contents (table_name TEXT PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT, description TEXT DEFAULT '',
           last_change DATETIME, min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
           srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));
         CREATE TABLE merged_from (file TEXT PRIMARY KEY, mtime INTEGER);
         CREATE TABLE cells (cell TEXT, edition TEXT, upd TEXT, issue_date TEXT, file TEXT);",
    )?;
    // per table: its columns (name, type) and the next fid
    let mut tables: std::collections::HashMap<String, (Vec<(String, String)>, i64)> = std::collections::HashMap::new();
    for f in files {
        db.execute("ATTACH DATABASE ?1 AS s", [f.to_string_lossy().as_ref()])?;
        let r = (|| -> Result<(), rusqlite::Error> {
            let found: Vec<(String, String)> = {
                let mut st = db.prepare("SELECT c.table_name, g.geometry_type_name FROM s.gpkg_contents c JOIN s.gpkg_geometry_columns g ON g.table_name = c.table_name")?;
                let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(Result::ok).collect();
                v
            };
            db.execute_batch("BEGIN")?;
            for (t, geo) in found {
                let cols: Vec<(String, String)> = {
                    let mut st = db.prepare(&format!("SELECT name, type FROM pragma_table_info('{t}', 's') WHERE name NOT IN ('fid', 'geom')"))?;
                    let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(Result::ok).collect();
                    v
                };
                let entry = match tables.get_mut(&t) {
                    Some(e) => e,
                    None => {
                        db.execute_batch(&format!(
                            "CREATE TABLE \"{t}\" (fid INTEGER PRIMARY KEY, geom BLOB);
                             CREATE VIRTUAL TABLE \"rtree_{t}_geom\" USING rtree(id, minx, maxx, miny, maxy);"
                        ))?;
                        db.execute("INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) VALUES (?1, 'features', ?1, 4326)", [&t])?;
                        db.execute("INSERT INTO gpkg_geometry_columns VALUES (?1, 'geom', ?2, 4326, 0, 0)", rusqlite::params![t, geo])?;
                        tables.entry(t.clone()).or_insert((Vec::new(), 0))
                    }
                };
                for (c, ty) in &cols {
                    if !entry.0.iter().any(|(n, _)| n == c) {
                        db.execute_batch(&format!("ALTER TABLE \"{t}\" ADD COLUMN \"{c}\" {ty}"))?;
                        entry.0.push((c.clone(), ty.clone()));
                    }
                }
                let off = entry.1;
                let list: String = cols.iter().map(|(c, _)| format!(", \"{c}\"")).collect();
                db.execute_batch(&format!(
                    "INSERT INTO \"{t}\" (fid, geom{list}) SELECT fid + {off}, geom{list} FROM s.\"{t}\";
                     INSERT INTO \"rtree_{t}_geom\" SELECT id + {off}, minx, maxx, miny, maxy FROM s.\"rtree_{t}_geom\";"
                ))?;
                entry.1 = db.query_row(&format!("SELECT coalesce(max(fid), 0) FROM \"{t}\""), [], |r| r.get(0))?;
            }
            let meta = |k: &str| -> String { db.query_row("SELECT value FROM s.enc_meta WHERE key = ?1", [k], |r| r.get(0)).unwrap_or_default() };
            let (c, e, u, d) = (meta("cell"), meta("edition"), meta("update"), meta("issue_date"));
            db.execute("INSERT INTO cells VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![c, e, u, d, f.file_name().map(|n| n.to_string_lossy().to_string())])?;
            db.execute_batch("COMMIT")
        })();
        if r.is_err() {
            let _ = db.execute_batch("ROLLBACK");
        }
        db.execute("DETACH DATABASE s", [])?;
        if let Err(e) = r {
            eprintln!("ENC: merging {}: {e}", f.display());
        }
    }
    for (f, t) in stamp {
        db.execute("INSERT OR REPLACE INTO merged_from VALUES (?1, ?2)", rusqlite::params![f, t])?;
    }
    drop(db);
    let _ = std::fs::remove_file(out);
    let _ = std::fs::remove_file(PathBuf::from(format!("{}.tables", out.display())));
    std::fs::rename(&tmp, out).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small real cell (from GDAL's test data, MIT licence), checked against GDAL's dump of it.
    #[test]
    fn reads_a_cell() {
        let c = Cell::from_bytes(include_bytes!("../../tests/data/1B5X02NE.000"), &[]).unwrap();
        assert_eq!((c.name.as_str(), c.intu, c.comf, c.somf), ("1B5X02NE.000", 5, 500000.0, 10.0));
        let v = &c.vectors[&(110, 2)];
        assert_eq!(v.coords.len(), 4);
        assert_eq!(v.coords[0], [-16247245, 30490822, 34]);
        assert_eq!(c.features.len(), 21);
        let mut areas = 0;
        for f in c.features.values() {
            if let Some(Shape::Polygons(ps)) = c.shape(f) {
                areas += 1;
                for r in ps.iter().flatten() {
                    assert_eq!(r.first(), r.last(), "closed rings");
                }
            }
        }
        assert_eq!(areas, 9);
        let dir = std::env::temp_dir().join("enc_test");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("t.gpkg");
        assert_eq!(write_gpkg(&c, &out).unwrap(), 14);
        // the chart store reads it like the LINZ layers
        let store = crate::ChartStore::open_dirs(&[dir.clone()]).unwrap();
        assert!(store.tables.iter().any(|t| t.kind == crate::Kind::DepthArea && t.band == 6));
        assert!(store.tables.iter().any(|t| t.kind == crate::Kind::Land && t.band == 6));
    }

    /// A folder of cells becomes one merged file per band, made again only when a cell changes.
    #[test]
    fn merges_cells_by_band() {
        let dir = std::env::temp_dir().join(format!("enc_merge_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("a/1B5X02NE.000"), include_bytes!("../../tests/data/1B5X02NE.000")).unwrap();
        let out = ensure_gpkgs(&dir);
        assert_eq!(out.len(), 1);
        assert!(out[0].file_name().unwrap().to_string_lossy().contains("-b6-"));
        let made = std::fs::metadata(&out[0]).unwrap().modified().unwrap();
        assert_eq!(ensure_gpkgs(&dir), out);
        assert_eq!(std::fs::metadata(&out[0]).unwrap().modified().unwrap(), made, "not made again");
        let store = crate::ChartStore::open_dirs(&[dir.clone()]).unwrap();
        assert!(store.tables.iter().any(|t| t.kind == crate::Kind::DepthArea && t.band == 6 && t.rows == 4));
        // the attribute values themselves, not the column names (SQLite reads an unknown
        // "name" as a string)
        let db = Connection::open(&out[0]).unwrap();
        let v: Option<f64> = db.query_row("SELECT max(drval1) FROM depth_area_polygon_hydro_enc_b6", [], |r| r.get(0)).unwrap();
        assert!(v.is_some_and(|d| d > 0.0));
        let bad: i64 = db.query_row("SELECT count(*) FROM depth_area_polygon_hydro_enc_b6 WHERE typeof(drval1) = 'text'", [], |r| r.get(0)).unwrap();
        assert_eq!(bad, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An S-63 cell from the IHO's S-64 test data (test 6a): its signature is checked,
    /// it is decrypted with the generic test credentials, and its update applied.
    #[test]
    fn decrypts_s63_cells() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/s63");
        let dir = std::env::temp_dir().join(format!("enc_s63_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        fn copy(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() { copy(&p, &b.join(e.file_name())) } else { std::fs::copy(&p, b.join(e.file_name())).unwrap(); }
            }
        }
        copy(&src, &dir);
        let cells = find_cells(&dir);
        assert_eq!(cells.len(), 1, "signature files are not cells");
        assert_eq!(cells[0].updates.len(), 1);
        let c = open_cell(&cells[0], &s63_permits(&dir)).unwrap();
        assert_eq!((c.name.as_str(), c.update.as_str()), ("GB61021B.000", "1"));
        assert!(c.features.len() > 200);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_permits() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/s63");
        let dir = std::env::temp_dir().join(format!("enc_s63i_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        fn copy(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() { copy(&p, &b.join(e.file_name())) } else if e.file_name() != "PERMIT.TXT" { std::fs::copy(&p, b.join(e.file_name())).unwrap(); }
            }
        }
        copy(&src, &dir);
        let cells = find_cells(&dir);
        let e = open_cell(&cells[0], &s63_permits(&dir)).err().unwrap().to_string();
        assert!(e.contains("SSE 11"), "{e}");
        let text = std::fs::read_to_string(src.join("PERMIT.TXT")).unwrap();
        let done = install_permits(&dir, &text).unwrap();
        assert!(done.iter().any(|(c, w)| c == "GB61021B" && w.starts_with("installed")), "{done:?}");
        assert!(dir.join(".permits/PERMIT.TXT").is_file());
        assert!(open_cell(&cells[0], &s63_permits(&dir)).is_ok());
        // installing again keeps one line per cell
        install_permits(&dir, &text).unwrap();
        let store = std::fs::read_to_string(dir.join(".permits/PERMIT.TXT")).unwrap();
        assert_eq!(store.lines().filter(|l| l.starts_with("GB61021B")).count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tables() {
        assert_eq!(table_for("LIGHTS", 1, 5).as_deref(), Some("light_points_hydro_enc_b5"));
        assert_eq!(table_for("DEPARE", 3, 4).as_deref(), Some("depth_area_polygon_hydro_enc_b4"));
        assert_eq!(table_for("LIGHTS", 3, 4), None);
        assert_eq!(band_for(5), 6);
    }
}
