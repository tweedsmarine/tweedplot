//! ENC cells as GeoPackages, laid out like the LINZ layers (`light_points_hydro_enc_b4`
//! and so on, S-57 attribute acronyms as columns), so a chart store draws them with the
//! same code as the LINZ data. The band comes from the cell's intended usage (overview 2,
//! general 3, coastal 4, approach 5, harbour and berthing 6).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::catalogue;
use crate::cell::{Cell, Shape};

/// Bump when the conversion changes, so cells are converted again.
pub const VERSION: u32 = 2;
/// Bump when merging changes, so the merged files are made again (the cells needn't be).
pub const MERGE_VERSION: u32 = 2;

fn io_err(e: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

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
pub fn table_for(acr: &str, prim: u8, band: u8) -> Option<String> {
    let geo = match prim {
        1 => crate::classes::Geo::Point,
        2 => crate::classes::Geo::Line,
        3 => crate::classes::Geo::Area,
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
    let c = crate::classes::CLASSES.iter().find(|c| c.code == acr && c.geo == geo)?;
    Some(format!("{}_hydro_enc_b{band}", c.prefix))
}

#[derive(Default)]
struct Table {
    geo: &'static str,
    cols: BTreeMap<String, char>,
    rows: Vec<(Vec<(String, String)>, Vec<u8>, [f64; 4])>,
}

/// Write a cell's features into `db`, a new empty database (a file, or in memory for
/// data that is to be sealed). Returns the features written.
pub fn write_into(c: &Cell, db: &mut Connection) -> Result<usize, rusqlite::Error> {
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
        let cols: String = t
            .cols
            .iter()
            .map(|(c, ty)| {
                format!(
                    ", \"{c}\" {}",
                    match ty {
                        'R' => "REAL",
                        'I' => "INTEGER",
                        _ => "TEXT",
                    }
                )
            })
            .collect();
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
    Ok(n)
}

/// Write a cell's features into a new GeoPackage file at `out` (written beside it and
/// renamed into place). Returns the features written.
pub fn write_gpkg(c: &Cell, out: &Path) -> Result<usize, rusqlite::Error> {
    let tmp = out.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut db = Connection::open(&tmp)?;
    let n = write_into(c, &mut db)?;
    drop(db);
    let _ = std::fs::remove_file(out);
    std::fs::rename(&tmp, out).map_err(io_err)?;
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

/// Merge cells' GeoPackages (same table layout) into `db`, a new empty database.
/// `attach(db, file)` attaches each file as schema `s` (a plain file with ATTACH, or
/// data unsealed into memory); false skips the file. `stamp` (file name, mtime) is kept
/// in `merged_from`, for [`merged_stamp`].
pub fn merge_into(
    db: &mut Connection,
    files: &[PathBuf],
    stamp: &[(String, i64)],
    attach: &mut dyn FnMut(&mut Connection, &Path) -> Result<(), rusqlite::Error>,
) -> Result<(), rusqlite::Error> {
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
        if let Err(e) = attach(db, f) {
            eprintln!("ENC: merging {}: {e}", f.display());
            let _ = db.execute("DETACH DATABASE s", []);
            continue;
        }
        let r = (|| -> Result<(), rusqlite::Error> {
            let found: Vec<(String, String)> = {
                let mut st = db.prepare(
                    "SELECT c.table_name, g.geometry_type_name FROM s.gpkg_contents c JOIN s.gpkg_geometry_columns g ON g.table_name = c.table_name",
                )?;
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
    Ok(())
}

/// Attach a plain GeoPackage file as schema `s`, for [`merge_into`].
pub fn attach_file(db: &mut Connection, f: &Path) -> Result<(), rusqlite::Error> {
    db.execute("ATTACH DATABASE ?1 AS s", [f.to_string_lossy().as_ref()])?;
    Ok(())
}

/// The (file, mtime) stamp a merged database was made from, sorted.
pub fn merged_stamp(db: &Connection) -> Vec<(String, i64)> {
    let Ok(mut st) = db.prepare("SELECT file, mtime FROM merged_from ORDER BY file") else { return vec![] };
    let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) else { return vec![] };
    rows.filter_map(Result::ok).collect()
}

/// A feature table in a GeoPackage, as the chart store needs to know it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GpkgTable {
    pub table: String,
    pub geom_col: String,
    pub rtree: String,
    pub pk: String,
    pub rows: i64,
    /// column names, lower case
    pub cols: Vec<String>,
    /// extent of the rows [w, s, e, n]
    pub bounds: [f64; 4],
}

/// The feature tables of a GeoPackage that have a spatial index (`rtree_<table>_<geom>`).
pub fn scan_tables(conn: &Connection) -> Result<Vec<GpkgTable>, rusqlite::Error> {
    let found: Vec<(String, String)> = {
        let mut st = conn.prepare(
            "SELECT c.table_name, g.column_name FROM gpkg_contents c
                 JOIN gpkg_geometry_columns g ON g.table_name = c.table_name
                 WHERE c.data_type = 'features'",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut out = Vec::new();
    for (table, geom_col) in found {
        let rtree = format!("rtree_{table}_{geom_col}");
        let has_rtree: bool = conn.query_row("SELECT count(*) FROM sqlite_master WHERE name = ?1", [&rtree], |r| r.get::<_, i64>(0)).map(|n| n > 0)?;
        if !has_rtree {
            continue;
        }
        let pk: String =
            conn.query_row(&format!("SELECT name FROM pragma_table_info('{table}') WHERE pk = 1"), [], |r| r.get(0)).unwrap_or_else(|_| "fid".into());
        let rows: i64 = conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| r.get(0))?;
        let cols: Vec<String> = {
            let mut st = conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?;
            let r = st.query_map([], |r| r.get::<_, String>(0))?;
            r.filter_map(Result::ok).map(|c| c.to_ascii_lowercase()).collect()
        };
        let bounds: [f64; 4] = conn
            .query_row(&format!("SELECT min(minx), min(miny), max(maxx), max(maxy) FROM \"{rtree}\""), [], |r| {
                Ok([r.get::<_, Option<f64>>(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
            })
            .map(|v| if v.iter().all(Option::is_some) { [v[0].unwrap(), v[1].unwrap(), v[2].unwrap(), v[3].unwrap()] } else { [0.0, 0.0, -1.0, -1.0] })
            .unwrap_or([-360.0, -90.0, 360.0, 90.0]);
        out.push(GpkgTable { table, geom_col, rtree, pk, rows, cols, bounds });
    }
    Ok(out)
}
