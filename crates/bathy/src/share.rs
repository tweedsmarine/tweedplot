//! Sharing own soundings: written into a library file (lossless: the finest-level cells with
//! their statistics, and the soundings the checks left out) or as an XYZ text file, and
//! brought in from a library file again.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

use crate::{decode, Cell, Error, Result, Store, LEFT_OUT, MAX_LEVEL, SEABED, TILE};

/// The table in a library file holding the soundings.
pub const TABLE: &str = "own_soundings";
const BATCH: usize = 256;

/// What a library file holds of own soundings.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct FileInfo {
    /// Cells with soundings.
    pub cells: i64,
    /// Cells of soundings the checks left out.
    pub left_out: i64,
}

fn has_table(c: &Connection) -> Result<bool> {
    Ok(c.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [TABLE], |_| Ok(())).optional()?.is_some())
}

/// What the library file at `path` holds of own soundings (None when none).
pub fn file_info(path: &Path) -> Result<Option<FileInfo>> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if !has_table(&c)? {
        return Ok(None);
    }
    let sum = |kind: &str| c.query_row(&format!("SELECT coalesce(sum(n), 0) FROM {TABLE} WHERE kind = ?1"), [kind], |r| r.get::<_, i64>(0));
    let info = FileInfo { cells: sum("seabed")?, left_out: sum("left_out")? };
    Ok(if info.cells + info.left_out > 0 { Some(info) } else { None })
}

impl Store {
    /// Tells this store apart from another (so soundings exported from it aren't merged
    /// back into it); made on first use, forgotten when the store is cleared.
    fn origin(&self) -> Result<String> {
        let db = self.db();
        if let Some(v) = db.query_row("SELECT v FROM meta WHERE k = 'origin'", [], |r| r.get::<_, String>(0)).optional()? {
            return Ok(v);
        }
        let h = || RandomState::new().build_hasher().finish();
        let v = format!("{:016x}{:016x}", h(), h());
        db.execute("INSERT OR REPLACE INTO meta (k, v) VALUES ('origin', ?1)", [&v])?;
        Ok(v)
    }

    /// Rows of a table at the finest level, as (ty, tx, n, blob), one connection of its own.
    fn rows(&self, table: &str) -> Result<(Connection, i64)> {
        let c = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let total: i64 = c.query_row(&format!("SELECT count(*) FROM {table} WHERE l = ?1"), [MAX_LEVEL], |r| r.get(0))?;
        Ok((c, total))
    }

    /// Write the soundings into the library file `dest` (as [`TABLE`]), the finest level
    /// only: the coarser ones are built again on import. Returns the cells written.
    pub fn export_into(&self, dest: &Path, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<i64> {
        let origin = self.origin()?;
        let mut out = Connection::open(dest)?;
        out.execute_batch(&format!(
            "DROP TABLE IF EXISTS {TABLE};
             CREATE TABLE {TABLE} (kind TEXT NOT NULL, ty INTEGER NOT NULL, tx INTEGER NOT NULL, n INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (kind, ty, tx)) WITHOUT ROWID;"
        ))?;
        let tx = out.transaction()?;
        tx.execute(&format!("INSERT INTO {TABLE} (kind, ty, tx, n, data) VALUES ('origin', 0, 0, 0, ?1)"), [origin.as_bytes()])?;
        let mut cells = 0i64;
        for (kind, table) in [("seabed", SEABED), ("left_out", LEFT_OUT)] {
            let (c, total) = self.rows(table)?;
            let mut q = c.prepare(&format!("SELECT ty, tx, n, data FROM {table} WHERE l = ?1 ORDER BY ty, tx"))?;
            let mut rows = q.query([MAX_LEVEL])?;
            let mut ins = tx.prepare(&format!("INSERT INTO {TABLE} (kind, ty, tx, n, data) VALUES (?1, ?2, ?3, ?4, ?5)"))?;
            let mut k = 0usize;
            while let Some(r) = rows.next()? {
                if k % 512 == 0 {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    progress(if kind == "seabed" { "Own soundings" } else { "Soundings left out" }, k, total as usize);
                }
                k += 1;
                let (ty, txi, n, blob): (i64, i64, i64, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                ins.execute(params![kind, ty, txi, n, blob])?;
                if kind == "seabed" {
                    cells += n;
                }
            }
        }
        tx.commit()?;
        Ok(cells)
    }

    /// Write the soundings as an XYZ text file: longitude, latitude, height (m, negative
    /// down) of each cell's centre, one per line, ready for other software or for
    /// [`Store::import_xyz`]. What the checks left out isn't written. Returns the lines.
    pub fn export_xyz(&self, path: &Path, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<i64> {
        let r = (|| -> Result<i64> {
            let (c, total) = self.rows(SEABED)?;
            let mut q = c.prepare(&format!("SELECT ty, tx, data FROM {SEABED} WHERE l = ?1 ORDER BY ty, tx"))?;
            let mut rows = q.query([MAX_LEVEL])?;
            let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?);
            let (mut k, mut lines) = (0usize, 0i64);
            while let Some(r) = rows.next()? {
                if k % 64 == 0 {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    progress("Writing soundings", k, total as usize);
                }
                k += 1;
                let (ty, tx, blob): (u32, u32, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                let mut err = None;
                decode(&blob, |x, y, cell| {
                    let (lon, lat) = crate::cell_centre(tx * TILE as u32 + x as u32, ty * TILE as u32 + y as u32, MAX_LEVEL);
                    if let Err(e) = writeln!(w, "{lon:.7},{lat:.7},{:.3}", cell.mean) {
                        err = Some(e);
                    }
                    lines += 1;
                })?;
                if let Some(e) = err {
                    return Err(e.into());
                }
            }
            w.flush()?;
            Ok(lines)
        })();
        if r.is_err() {
            let _ = std::fs::remove_file(path);
        }
        r
    }

    /// Bring in the soundings of a library file (made by [`Store::export_into`]), merged
    /// with what's here. Refused when the file came from this store (they'd count twice),
    /// unless `again`. Returns (cells added, cells left out).
    pub fn import_from(&self, path: &Path, again: bool, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<(i64, i64)> {
        let src = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        if !has_table(&src)? {
            return Err(Error::Format("no own soundings in this file".into()));
        }
        if !again {
            let from: Option<Vec<u8>> = src.query_row(&format!("SELECT data FROM {TABLE} WHERE kind = 'origin'"), [], |r| r.get(0)).optional()?;
            if let Some(from) = from {
                if from == self.origin()?.as_bytes() {
                    return Err(Error::Format("these soundings were written from the soundings already here".into()));
                }
            }
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = std::fs::metadata(path)?.len() as i64;
        let mut w = self.writer("Library file", &name, bytes)?;
        let total: i64 = src.query_row(&format!("SELECT count(*) FROM {TABLE} WHERE kind IN ('seabed', 'left_out')"), [], |r| r.get(0))?;
        let mut done = 0usize;
        {
            let mut q = src.prepare(&format!("SELECT ty, tx, data FROM {TABLE} WHERE kind = 'seabed' ORDER BY ty, tx"))?;
            let mut rows = q.query([])?;
            let mut batch: Vec<(u32, u32, Vec<(u8, u8, Cell)>)> = Vec::with_capacity(BATCH);
            loop {
                let next = rows.next()?;
                if let Some(r) = next {
                    let (ty, tx, blob): (u32, u32, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                    let mut cells = Vec::new();
                    decode(&blob, |x, y, c| cells.push((x as u8, y as u8, c)))?;
                    batch.push((ty, tx, cells));
                    done += 1;
                }
                if batch.len() >= BATCH || (next.is_none() && !batch.is_empty()) {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    progress("Own soundings", done, total as usize);
                    w.add_tiles(std::mem::take(&mut batch))?;
                }
                if next.is_none() {
                    break;
                }
            }
        }
        {
            let mut q = src.prepare(&format!("SELECT ty, tx, data FROM {TABLE} WHERE kind = 'left_out' ORDER BY ty, tx"))?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (ty, tx, blob): (u32, u32, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                decode(&blob, |x, y, c| w.reject_cell(tx * TILE as u32 + x as u32, ty * TILE as u32 + y as u32, c))?;
                done += 1;
                if done % 256 == 0 {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    progress("Soundings left out", done, total as usize);
                }
            }
        }
        let rejected = w.rejected();
        let n = w.finish(progress, cancel)?;
        self.changed();
        Ok((n, rejected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SRC_OWN;

    fn store(name: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("bathy-share-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Store::open(&dir.join("bathy.sqlite")).unwrap(), dir)
    }

    #[test]
    fn soundings_travel_in_a_library_file_and_as_xyz() {
        let (a, da) = store("a");
        let (b, db) = store("b");
        let stop = AtomicBool::new(false);
        let mut w = a.writer("own", "t", 0).unwrap();
        for i in 0..50 {
            w.add(174.0 + i as f64 * 0.001, -36.0, -30.0 - i as f32, 10, SRC_OWN).unwrap();
        }
        w.reject(174.5, -36.0, -2.0, 10, SRC_OWN | crate::WHY_SPIKE);
        w.finish(&mut |_, _, _| {}, &stop).unwrap();

        let file = da.join("share.gpkg");
        {
            Connection::open(&file).unwrap().execute_batch("CREATE TABLE marks (id TEXT)").unwrap();
        }
        assert!(file_info(&file).unwrap().is_none());
        let cells = a.export_into(&file, &mut |_, _, _| {}, &stop).unwrap();
        assert_eq!(cells, 50);
        let info = file_info(&file).unwrap().unwrap();
        assert_eq!((info.cells, info.left_out), (50, 1));

        // not back into the store it came from, but into another
        assert!(a.import_from(&file, false, &mut |_, _, _| {}, &stop).is_err());
        let (n, left) = b.import_from(&file, false, &mut |_, _, _| {}, &stop).unwrap();
        assert_eq!((n, left), (50, 1));
        let (ia, ib) = (a.info().unwrap(), b.info().unwrap());
        assert_eq!(ia.cells, ib.cells);
        assert_eq!(ia.rejected, ib.rejected);
        let (sa, sb) = (a.sounding_at(174.01, -36.0), b.sounding_at(174.01, -36.0));
        assert!((sa.unwrap().z - sb.unwrap().z).abs() < 1e-4);

        // XYZ reads back into a store as the same cells
        let xyz = da.join("out.xyz");
        assert_eq!(a.export_xyz(&xyz, &mut |_, _, _| {}, &stop).unwrap(), 50);
        let (c, dc) = store("c");
        assert_eq!(c.import_xyz(&xyz, &mut |_, _, _| {}, &stop).unwrap(), 50);
        assert_eq!(c.info().unwrap().cells, 50);
        for d in [da, db, dc] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
