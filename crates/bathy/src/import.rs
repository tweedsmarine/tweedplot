//! Imports: TimeZero PBG files (`.tzpbg`) and XYZ text, and cleaning a track's depths.
//!
//! ## TimeZero .tzpbg
//! A zip of SQLite databases: `PbgSeeds01.dbpbg` (the soundings) and `PbgWassp01.dbwassp`
//! (WASSP multibeam, same layout). Table `MX_RCL (r, c, l, datablob)`: level `l` (7–16, 16
//! the finest), column `c` from 180° W, row `r` from the **south**, in ellipsoidal Mercator
//! (as [`crate::cell_of`]). A blob: u8 1, f32 min and max height, three bytes 00 01 02, u16
//! count, u32 length, then zlib of 6-byte cells (u8 x east, u8 y down from the tile's top,
//! f32 height in m, negative down). Only the finest level is read; the coarser ones are
//! built again here.

use std::io::{BufRead, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::{hours, Cell, Error, Result, Store, MAX_LEVEL, SRC_PBG, SRC_XYZ};

/// One tile of a PBG database: its level, row from the top, column, and cells.
pub struct PbgTile {
    pub level: u8,
    pub ty: u32,
    pub tx: u32,
    pub cells: Vec<(u8, u8, f32)>,
}

/// The cells of a PBG blob.
pub fn decode_pbg_blob(b: &[u8]) -> Result<Vec<(u8, u8, f32)>> {
    if b.len() < 18 || b[0] != 1 {
        return Err(Error::Format("not a TimeZero PBG tile".into()));
    }
    let len = u32::from_le_bytes(b[14..18].try_into().unwrap()) as usize;
    let end = (18 + len).min(b.len());
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&b[18..end]).read_to_end(&mut raw)?;
    Ok(raw.chunks_exact(6).map(|r| (r[0], r[1], f32::from_le_bytes(r[2..6].try_into().unwrap()))).collect())
}

/// Read the finest level of a PBG database, calling `f` for each tile. Returns the level
/// read and when the file was made (Unix seconds, 0 if unknown).
pub fn pbg_level16(db: &Path, f: &mut dyn FnMut(PbgTile) -> Result<()>, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<(u8, i64)> {
    pbg_rows(
        db,
        &mut |level, ty, tx, b| match decode_pbg_blob(&b) {
            Ok(cells) => f(PbgTile { level, ty, tx, cells }),
            Err(_) => Ok(()), // a damaged tile: skip it
        },
        progress,
        cancel,
    )
}

/// The finest level's tiles of a PBG database as stored: `f(level, row from the top,
/// column, blob)`. Returns the level and when the file was made (Unix seconds, 0 if unknown).
fn pbg_rows(db: &Path, f: &mut dyn FnMut(u8, u32, u32, Vec<u8>) -> Result<()>, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<(u8, i64)> {
    let c = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let has: Option<String> = c.query_row("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'MX_RCL'", [], |r| r.get(0)).optional()?;
    if has.is_none() {
        return Err(Error::Format("not a TimeZero PBG database (no MX_RCL table)".into()));
    }
    let made = pbg_made(db);
    let Some(level): Option<u8> = c.query_row("SELECT max(l) FROM MX_RCL", [], |r| r.get(0))? else {
        return Ok((MAX_LEVEL, made)); // empty
    };
    let total: i64 = c.query_row("SELECT count(*) FROM MX_RCL WHERE l = ?1", [level], |r| r.get(0))?;
    let mut q = c.prepare("SELECT r, c, datablob FROM MX_RCL WHERE l = ?1 ORDER BY r, c")?;
    let mut rows = q.query([level])?;
    let mut i = 0usize;
    let span = 1u64 << level;
    while let Some(row) = rows.next()? {
        if i % 1000 == 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            progress("Reading", i, total as usize);
        }
        i += 1;
        let (r, col): (i64, i64) = (row.get(0)?, row.get(1)?);
        let Some(b): Option<Vec<u8>> = row.get(2)? else { continue };
        if r < 0 || col < 0 || r as u64 >= span || col as u64 >= span {
            continue;
        }
        f(level, (span - 1 - r as u64) as u32, col as u32, b)?;
    }
    progress("Reading", total as usize, total as usize);
    Ok((level, made))
}

/// When a PBG database was made (its catalogue's date, Unix seconds; 0 if unknown).
pub fn pbg_made(db: &Path) -> i64 {
    let Ok(c) = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return 0 };
    let made: Option<String> = c.query_row("SELECT CAT_PRODDATE FROM MX_CATALOG LIMIT 1", [], |r| r.get(0)).optional().ok().flatten().flatten();
    made.as_deref().and_then(parse_time).unwrap_or(0)
}

/// "2024-02-27 23:57:12" as Unix seconds.
fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let (d, t) = s.split_once([' ', 'T']).unwrap_or((s, "00:00:00"));
    let mut p = d.split('-').map(|v| v.parse::<i64>().ok());
    let (y, m, dd) = (p.next()??, p.next()??, p.next()??);
    let mut q = t.split(':').map(|v| v.trim_end_matches('Z').parse::<f64>().ok());
    let (hh, mm, ss) = (q.next().flatten().unwrap_or(0.0), q.next().flatten().unwrap_or(0.0), q.next().flatten().unwrap_or(0.0));
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86400 + (hh * 3600.0 + mm * 60.0 + ss) as i64)
}

fn mtime_s(p: &Path) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// A height worth keeping: real, not above 100 m (land) or below the deepest ocean.
fn plausible(z: f32) -> bool {
    z.is_finite() && z < 100.0 && z > -11_500.0
}

impl Store {
    /// Import a TimeZero PBG file (`.tzpbg`, or a `.dbpbg` / `.dbwassp` taken out of one).
    /// Returns the cells added. `progress(stage, done, total)`.
    pub fn import_pbg(&self, path: &Path, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<i64> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = std::fs::metadata(path)?.len() as i64;
        let mut magic = [0u8; 4];
        std::fs::File::open(path)?.read_exact(&mut magic)?;
        let mut w = self.writer("TimeZero PBG", &name, bytes)?;
        let fallback = hours(mtime_s(path));
        let add = |db: &Path, w: &mut crate::Writer, progress: &mut dyn FnMut(&str, usize, usize)| -> Result<()> {
            let made = pbg_made(db);
            let h = if made > 0 { hours(made) } else { fallback };
            // tiles a batch at a time: unpacked and merged on all cores
            let mut batch: Vec<(u8, u32, u32, Vec<u8>)> = Vec::with_capacity(BATCH);
            pbg_rows(
                db,
                &mut |level, ty, tx, b| {
                    batch.push((level, ty, tx, b));
                    if batch.len() >= BATCH {
                        put_batch(w, std::mem::take(&mut batch), h)?;
                    }
                    Ok(())
                },
                progress,
                cancel,
            )?;
            put_batch(w, batch, h)?;
            Ok(())
        };
        if &magic[..2] == b"PK" {
            let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
            let dir = self.path().parent().unwrap_or(Path::new(".")).to_path_buf();
            let mut found = 0;
            for i in 0..zip.len() {
                let mut e = zip.by_index(i)?;
                let ename = e.name()?.to_string();
                let en = ename.to_ascii_lowercase();
                if !(en.ends_with(".dbpbg") || en.ends_with(".dbwassp")) || e.size() == 0 {
                    continue;
                }
                found += 1;
                let tmp = dir.join(format!("pbg-import-{}-{i}.tmp", std::process::id()));
                progress(&format!("Unpacking {ename}"), 0, 0);
                {
                    let mut out = std::fs::File::create(&tmp)?;
                    std::io::copy(&mut e, &mut out)?;
                }
                drop(e);
                let r = add(&tmp, &mut w, progress);
                let _ = std::fs::remove_file(&tmp);
                r?;
            }
            if found == 0 {
                return Err(Error::Format("no PBG database in this file".into()));
            }
        } else if &magic == b"SQLi" {
            add(path, &mut w, progress)?;
        } else {
            return Err(Error::Format("not a TimeZero PBG file".into()));
        }
        let n = w.finish(progress, cancel)?;
        self.changed();
        Ok(n)
    }

    /// Import an XYZ text file: longitude, latitude, height (m, negative down) on each line,
    /// separated by commas, semicolons, tabs or spaces; lines that don't parse (a header)
    /// are skipped. Returns the soundings added.
    pub fn import_xyz(&self, path: &Path, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<i64> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = std::fs::metadata(path)?.len();
        let h = hours(mtime_s(path));
        let mut w = self.writer("XYZ", &name, bytes as i64)?;
        let f = std::fs::File::open(path)?;
        let mut r = std::io::BufReader::with_capacity(1 << 20, f);
        let mut line = String::new();
        let (mut read, mut k) = (0u64, 0usize);
        // which column is latitude: decided by the first lines (a value beyond ±90 is a longitude)
        let mut swap: Option<bool> = None;
        loop {
            line.clear();
            let n = r.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            read += n as u64;
            k += 1;
            if k % 200_000 == 0 {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                progress("Reading", (read >> 10) as usize, (bytes >> 10) as usize);
            }
            let mut it = line.split([',', ';', '\t', ' ']).filter(|s| !s.is_empty()).map(|s| s.trim().parse::<f64>());
            let (Some(Ok(a)), Some(Ok(b)), Some(Ok(z))) = (it.next(), it.next(), it.next()) else { continue };
            let sw = *swap.get_or_insert(a.abs() <= 90.0 && b.abs() > 90.0);
            let (lon, lat) = if sw { (b, a) } else { (a, b) };
            let z = z as f32;
            if !plausible(z) {
                continue;
            }
            w.add(lon, lat, z, h, SRC_XYZ)?;
        }
        let n = w.finish(progress, cancel)?;
        self.changed();
        Ok(n)
    }
}

const BATCH: usize = 512;

/// Put PBG tiles (as stored) into the store: unpacked in parallel, the finest level merged
/// a batch at a time, any other level cell by cell.
fn put_batch(w: &mut crate::Writer, batch: Vec<(u8, u32, u32, Vec<u8>)>, h: u32) -> Result<()> {
    use rayon::prelude::*;
    let tiles: Vec<PbgTile> = batch
        .into_par_iter()
        .filter_map(|(level, ty, tx, b)| decode_pbg_blob(&b).ok().map(|cells| PbgTile { level, ty, tx, cells }))
        .collect();
    let mut finest = Vec::with_capacity(tiles.len());
    for t in tiles {
        if t.level == MAX_LEVEL {
            let cells: Vec<(u8, u8, Cell)> = t.cells.iter().filter(|c| plausible(c.2)).map(|&(x, y, z)| (x, y, Cell::one(z, h, SRC_PBG))).collect();
            if !cells.is_empty() {
                finest.push((t.ty, t.tx, cells));
            }
        } else {
            put(w, t, h)?;
        }
    }
    w.add_tiles(finest)
}

/// Put a PBG tile into the store: straight in when it's at the finest level, else cell by cell.
fn put(w: &mut crate::Writer, t: PbgTile, h: u32) -> Result<()> {
    if t.level == MAX_LEVEL {
        let cells: Vec<(u8, u8, Cell)> = t.cells.iter().filter(|c| plausible(c.2)).map(|&(x, y, z)| (x, y, Cell::one(z, h, SRC_PBG))).collect();
        if !cells.is_empty() {
            w.add_tile(t.ty, t.tx, &cells)?;
        }
        return Ok(());
    }
    for &(x, y, z) in &t.cells {
        if !plausible(z) {
            continue;
        }
        let (gx, gy) = ((t.tx as u64) * 256 + x as u64, (t.ty as u64) * 256 + y as u64);
        let (cx, cy) = if t.level < MAX_LEVEL {
            let s = MAX_LEVEL - t.level;
            ((gx << s) + (1 << (s - 1)), (gy << s) + (1 << (s - 1)))
        } else {
            let s = t.level - MAX_LEVEL;
            (gx >> s, gy >> s)
        };
        w.add_cell(cx as u32, cy as u32, Cell::one(z, h, SRC_PBG))?;
    }
    Ok(())
}

/// Which of a track segment's depths (m, positive down) to keep: real depths (0.3 m to
/// 11 000 m) not more than 2 m or 10 % (whichever is more) off the median of the seven
/// fixes around them. A sounder losing the bottom, seeing fish or a wake gives spikes.
pub fn clean_depths(d: &[Option<f64>]) -> Vec<bool> {
    let ok = |v: f64| v.is_finite() && v > 0.3 && v < 11_000.0;
    let mut keep = vec![false; d.len()];
    let mut win: Vec<f64> = Vec::with_capacity(7);
    for i in 0..d.len() {
        let Some(v) = d[i].filter(|v| ok(*v)) else { continue };
        win.clear();
        for j in i.saturating_sub(3)..(i + 4).min(d.len()) {
            if let Some(w) = d[j].filter(|w| ok(*w)) {
                win.push(w);
            }
        }
        if win.len() < 3 {
            keep[i] = true;
            continue;
        }
        win.sort_by(f64::total_cmp);
        let med = win[win.len() / 2];
        keep[i] = (v - med).abs() <= (0.1 * med).max(2.0);
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// A PBG file like TimeZero's: a zip holding a SQLite database with tiles at levels 15 and 16.
    fn make_pbg(path: &Path, cells: &[(f64, f64, f32)]) {
        let tmp = path.with_extension("dbtmp");
        let _ = std::fs::remove_file(&tmp);
        {
            let c = Connection::open(&tmp).unwrap();
            c.execute_batch(
                "CREATE TABLE [MX_CATALOG] ([CAT_NAME] VARCHAR(400) PRIMARY KEY, [CAT_TYPE] INTEGER, [CAT_FORMAT] VARCHAR(3), [CAT_PRODDATE] TIMESTAMP);
                 INSERT INTO MX_CATALOG VALUES ('TEMP\\PbgSeeds_TEMP01.dbpbg', 8, 'PBG', '2024-02-27 23:57:12');
                 CREATE TABLE [MX_RCL] ([r] int NOT NULL, [c] int NOT NULL, [l] tinyint NOT NULL, [datablob] image NULL, PRIMARY KEY ([r],[c],[l]));",
            )
            .unwrap();
            let mut tiles: std::collections::BTreeMap<(u32, u32), Vec<(u8, u8, f32)>> = Default::default();
            for &(lon, lat, z) in cells {
                let (cx, cy) = crate::cell_of(lon, lat, 16).unwrap();
                tiles.entry((cy >> 8, cx >> 8)).or_default().push(((cx & 255) as u8, (cy & 255) as u8, z));
            }
            for ((ty, tx), v) in &tiles {
                let mut raw = Vec::new();
                for (x, y, z) in v {
                    raw.push(*x);
                    raw.push(*y);
                    raw.extend_from_slice(&z.to_le_bytes());
                }
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&raw).unwrap();
                let zc = e.finish().unwrap();
                let mut b = vec![1u8];
                b.extend_from_slice(&(-50f32).to_le_bytes());
                b.extend_from_slice(&(-10f32).to_le_bytes());
                b.extend_from_slice(&[0, 1, 2]);
                b.extend_from_slice(&(v.len() as u16).to_le_bytes());
                b.extend_from_slice(&(zc.len() as u32).to_le_bytes());
                b.extend_from_slice(&zc);
                let r = (1u32 << 16) - 1 - ty;
                c.execute("INSERT INTO MX_RCL VALUES (?1, ?2, 16, ?3)", rusqlite::params![r, tx, b]).unwrap();
                // a coarser overview tile that must be ignored
                c.execute("INSERT OR IGNORE INTO MX_RCL VALUES (?1, ?2, 15, ?3)", rusqlite::params![r / 2, tx / 2, vec![1u8, 0, 0]]).unwrap();
            }
        }
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        z.start_file("PbgSeeds01.dbpbg", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&std::fs::read(&tmp).unwrap()).unwrap();
        z.start_file("PbgWassp01.dbwassp", zip::write::SimpleFileOptions::default()).unwrap();
        z.finish().unwrap();
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn imports_a_pbg_file() {
        let dir = std::env::temp_dir().join(format!("bathy-pbg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pbg = dir.join("test.tzpbg");
        let pts: Vec<(f64, f64, f32)> = (0..300).map(|i| (171.99 + i as f64 * 0.00005, -40.158, -300.0 - i as f32 * 0.05)).collect();
        make_pbg(&pbg, &pts);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let n = s.import_pbg(&pbg, &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert!(n > 250 && n <= 300, "{n}");
        let p = s.sounding_at(171.99 + 100.0 * 0.00005, -40.158).unwrap();
        assert!((p.z + 305.0).abs() < 0.2 && p.off_m < 3.0, "{p:?}");
        assert_eq!(p.src, SRC_PBG);
        assert_eq!(p.newest, 1709078400 - 3600); // the file's date, to the hour (27 Feb 2024 23:00 UTC)
        assert!(s.has_source("TimeZero PBG", "test.tzpbg", std::fs::metadata(&pbg).unwrap().len() as i64));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_xyz_and_parses_dates() {
        let dir = std::env::temp_dir().join(format!("bathy-xyz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.xyz");
        std::fs::write(&p, "long, lat, elevation\n171.991846561432, -40.1582038082353, -308.4\n171.99193239212, -40.1582038082353, -309.1\nrubbish\n").unwrap();
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        assert_eq!(s.import_xyz(&p, &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap(), 2);
        // the points are cell corners: the cell is the one to the south-east
        let c = s.sounding_at(171.991846561432 + 1e-5, -40.1582038082353 - 1e-5).unwrap();
        assert!((c.z + 308.4).abs() < 1e-3, "{c:?}");
        assert_eq!(parse_time("1970-01-02 00:00:01"), Some(86401));
        assert_eq!(parse_time("2024-02-27 23:57:12"), Some(1709078232));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spikes_are_dropped() {
        let d: Vec<Option<f64>> = [30.0, 30.2, 30.1, 4.0, 30.3, 30.4, f64::NAN, 30.2, 300.0, 30.0, 29.9]
            .iter()
            .map(|&v| Some(v))
            .chain([None, Some(0.1)])
            .collect();
        let k = clean_depths(&d);
        assert_eq!(k, vec![true, true, true, false, true, true, false, true, false, true, true, false, false]);
    }
}
