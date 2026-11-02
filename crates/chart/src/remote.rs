//! Tables served by another program: the TweedPlot S-63 Charts plugin keeps encrypted
//! charts to itself and answers, one table and one area at a time, what the chart store
//! asks of them. The store treats those tables like its own GeoPackage tables, so tiles,
//! popups, routing and alarms work the same on encrypted charts.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use plugin_api::protocol::TableMeta;
use plugin_api::Row;
use rusqlite::Connection;

/// The questions the chart store asks of a table it doesn't hold.
pub trait RemoteSource: Send + Sync {
    /// The rows of `table` (in database `db`) whose extent touches `bbox` [w, s, e, n],
    /// with the columns `cols` (Null where the table hasn't one).
    fn rows(&self, db: u32, table: &str, bbox: [f64; 4], cols: &[&str]) -> Vec<Row>;
    /// One row by its key.
    fn feature(&self, db: u32, table: &str, fid: i64, cols: &[&str]) -> Option<Row>;
    /// The median of a numeric column (for SCAMIN), or None.
    fn median(&self, db: u32, table: &str, col: &str) -> Option<f64>;
}

/// A database of the chart store: a GeoPackage it opened, or one a plugin holds.
pub(crate) enum Db {
    Local(Mutex<Connection>),
    Remote(Arc<dyn RemoteSource>, u32),
}

impl Db {
    pub(crate) fn local(&self) -> Option<std::sync::MutexGuard<'_, Connection>> {
        match self {
            Db::Local(c) => Some(c.lock().unwrap_or_else(|e| e.into_inner())),
            Db::Remote(..) => None,
        }
    }
}

impl crate::ChartStore {
    /// Add the tables a plugin serves (from its `open` answer). Tables whose names the
    /// store doesn't draw are left out. Returns how many were added.
    pub fn add_remote(&mut self, src: Arc<dyn RemoteSource>, tables: &[TableMeta]) -> usize {
        let mut n = 0;
        let mut db_of: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for m in tables {
            let t = &m.table;
            let Some((kind, band)) = crate::classify(&t.table) else { continue };
            let db = *db_of.entry(m.db).or_insert_with(|| {
                self.dbs.push(Db::Remote(src.clone(), m.db));
                self.dbs.len() - 1
            });
            self.tables.push(crate::TableInfo {
                file: PathBuf::from(format!("plugin:{}", m.db)),
                table: t.table.clone(),
                kind,
                band,
                rows: t.rows,
                class: if kind == crate::Kind::Feature { crate::s57::class_for(&t.table) } else { None },
                cols: t.cols.clone(),
                db,
                rtree: t.rtree.clone(),
                pk: t.pk.clone(),
                bounds: t.bounds,
            });
            n += 1;
        }
        if n > 0 {
            self.clear_cache();
            self.coverage = std::sync::OnceLock::new();
        }
        n
    }

    /// Whether any table comes from a plugin.
    pub fn has_remote(&self) -> bool {
        self.dbs.iter().any(|d| matches!(d, Db::Remote(..)))
    }
}

/// A [`RemoteSource`] over GeoPackage files in this process: what the plugin does with
/// its sealed databases, for tests and for checking that remote tables draw the same.
pub struct LocalFiles {
    dbs: Vec<(Mutex<Connection>, std::collections::HashMap<String, enc_s57::GpkgTable>)>,
}

impl LocalFiles {
    /// Open the files; returns the source and the table list a plugin would send.
    pub fn open(files: &[PathBuf]) -> Result<(LocalFiles, Vec<TableMeta>), rusqlite::Error> {
        let mut dbs = Vec::new();
        let mut metas = Vec::new();
        for (i, f) in files.iter().enumerate() {
            let c = crate::enc::open_db(f)?;
            let mut by = std::collections::HashMap::new();
            for t in enc_s57::gpkg::scan_tables(&c)? {
                by.insert(t.table.clone(), t.clone());
                metas.push(TableMeta { db: i as u32, table: t });
            }
            dbs.push((Mutex::new(c), by));
        }
        Ok((LocalFiles { dbs }, metas))
    }
}

/// The plugin's queries, as SQL on a GeoPackage connection (the plugin uses these too).
pub fn query_rows(c: &Connection, t: &enc_s57::GpkgTable, bbox: [f64; 4], cols: &[&str]) -> Vec<Row> {
    let sel: String = cols.iter().map(|col| if t.cols.iter().any(|x| x == col) { format!("t.\"{col}\", ") } else { "NULL, ".into() }).collect();
    let sql = format!(
        "SELECT t.\"{pk}\", {sel}t.\"{g}\" FROM \"{tbl}\" t JOIN \"{rt}\" r ON t.\"{pk}\" = r.id
         WHERE r.maxx >= ?1 AND r.minx <= ?2 AND r.maxy >= ?3 AND r.miny <= ?4",
        pk = t.pk,
        g = t.geom_col,
        tbl = t.table,
        rt = t.rtree
    );
    let Ok(mut st) = c.prepare_cached(&sql) else { return vec![] };
    let Ok(mut rows) = st.query(rusqlite::params![bbox[0], bbox[2], bbox[1], bbox[3]]) else { return vec![] };
    let mut out = Vec::new();
    while let Ok(Some(r)) = rows.next() {
        out.push(Row {
            fid: r.get(0).unwrap_or(0),
            vals: (0..cols.len()).map(|i| value(r.get_ref(i + 1).ok())).collect(),
            geom: r.get_ref(cols.len() + 1).ok().and_then(|v| v.as_blob().ok()).map(<[u8]>::to_vec).unwrap_or_default(),
        });
    }
    out
}

/// One row by key (see [`query_rows`]).
pub fn query_feature(c: &Connection, t: &enc_s57::GpkgTable, fid: i64, cols: &[&str]) -> Option<Row> {
    let sel: String = cols.iter().map(|col| if t.cols.iter().any(|x| x == col) { format!("\"{col}\", ") } else { "NULL, ".into() }).collect();
    let sql = format!("SELECT {sel}\"{}\" FROM \"{}\" WHERE \"{}\" = ?1", t.geom_col, t.table, t.pk);
    let mut st = c.prepare_cached(&sql).ok()?;
    let mut rows = st.query([fid]).ok()?;
    let r = rows.next().ok()??;
    Some(Row {
        fid,
        vals: (0..cols.len()).map(|i| value(r.get_ref(i).ok())).collect(),
        geom: r.get_ref(cols.len()).ok().and_then(|v| v.as_blob().ok()).map(<[u8]>::to_vec).unwrap_or_default(),
    })
}

/// The median of a column (see [`query_rows`]).
pub fn query_median(c: &Connection, t: &enc_s57::GpkgTable, col: &str) -> Option<f64> {
    if !t.cols.iter().any(|x| x == col) {
        return None;
    }
    let table = &t.table;
    let n: i64 = c.query_row(&format!("SELECT count(\"{col}\") FROM \"{table}\""), [], |r| r.get(0)).ok()?;
    if n == 0 {
        return None;
    }
    c.query_row(&format!("SELECT \"{col}\" FROM \"{table}\" WHERE \"{col}\" IS NOT NULL ORDER BY \"{col}\" LIMIT 1 OFFSET {}", n / 2), [], |r| {
        r.get::<_, f64>(0)
    })
    .ok()
}

fn value(v: Option<rusqlite::types::ValueRef>) -> plugin_api::Value {
    use rusqlite::types::ValueRef::*;
    match v {
        Some(Integer(i)) => plugin_api::Value::Int(i),
        Some(Real(f)) => plugin_api::Value::Real(f),
        Some(Text(t)) => plugin_api::Value::Text(String::from_utf8_lossy(t).into_owned()),
        _ => plugin_api::Value::Null,
    }
}

impl RemoteSource for LocalFiles {
    fn rows(&self, db: u32, table: &str, bbox: [f64; 4], cols: &[&str]) -> Vec<Row> {
        let Some((c, by)) = self.dbs.get(db as usize) else { return vec![] };
        let Some(t) = by.get(table) else { return vec![] };
        query_rows(&c.lock().unwrap(), t, bbox, cols)
    }
    fn feature(&self, db: u32, table: &str, fid: i64, cols: &[&str]) -> Option<Row> {
        let (c, by) = self.dbs.get(db as usize)?;
        query_feature(&c.lock().unwrap(), by.get(table)?, fid, cols)
    }
    fn median(&self, db: u32, table: &str, col: &str) -> Option<f64> {
        let (c, by) = self.dbs.get(db as usize)?;
        query_median(&c.lock().unwrap(), by.get(table)?, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile_of(lon: f64, lat: f64, z: u8) -> (u32, u32) {
        let n = (1u32 << z) as f64;
        let x = ((lon + 180.0) / 360.0 * n) as u32;
        let r = lat.to_radians();
        let y = ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32;
        (x, y)
    }

    /// The same cell drawn from the store's own file and through a remote source gives
    /// the same tiles and the same feature details.
    #[test]
    fn remote_tables_draw_the_same() {
        let dir = std::env::temp_dir().join(format!("tp_remote_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("local")).unwrap();
        let c = enc_s57::cell::Cell::from_bytes(include_bytes!("../tests/data/1B5X02NE.000"), &[]).unwrap();
        let file = dir.join("local/cell.gpkg");
        crate::enc::write_gpkg(&c, &file).unwrap();
        let local = crate::ChartStore::open_dirs(&[dir.join("local")]).unwrap();
        let (src, metas) = LocalFiles::open(std::slice::from_ref(&file)).unwrap();
        let mut remote = crate::ChartStore::open_dirs(&[dir.join("none")]).unwrap();
        assert_eq!(remote.add_remote(Arc::new(src), &metas), local.tables.len());
        assert!(remote.has_remote());
        let b = local.tables.iter().find(|t| t.kind == crate::Kind::DepthArea).unwrap().bounds;
        let (lon, lat) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        let mut drawn = 0;
        for z in [6u8, 9, 12, 14] {
            let (x, y) = tile_of(lon, lat, z);
            let (a, r) = (local.tile(z, x, y), remote.tile(z, x, y));
            assert_eq!(a, r, "tile {z}/{x}/{y}");
            drawn += a.len();
        }
        assert!(drawn > 1000, "the tiles have something in them ({drawn} bytes)");
        // a feature's details, by table index and key
        let (ti, _) = local.tables.iter().enumerate().find(|(_, t)| t.kind == crate::Kind::Feature).unwrap();
        let rt = remote.tables.iter().position(|t| t.table == local.tables[ti].table).unwrap();
        let fa = local.feature(ti, 1).map(|f| f.attrs);
        assert!(fa.is_some());
        assert_eq!(fa, remote.feature(rt, 1).map(|f| f.attrs));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
