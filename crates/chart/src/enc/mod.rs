//! S-57 ENC cells (`.000` with updates `.001`, `.002` ...) for the chart store.
//!
//! Each cell found under a chart folder is converted once into a GeoPackage in that
//! folder's `.enc` subfolder (see [`enc_s57::gpkg`]), so the store draws ENC cells with
//! the same code as the LINZ data. A cell is converted again when it or an update
//! changes, or when the converter changes ([`VERSION`]).
//!
//! Encrypted (S-63) cells are not handled here: they belong to the TweedPlot S-63 Charts
//! plugin, a separate program that imports, decrypts and keeps them, and serves their
//! tables to the chart store (see `crate::remote`). One found in a chart folder is left
//! alone with a note.

pub use enc_s57::files::{find_cells, find_cells_except, CellFiles};
pub use enc_s57::gpkg::{band_for, table_for, write_gpkg, MERGE_VERSION, VERSION};
pub use enc_s57::{catalogue, cell, iso8211, picture, texts};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use cell::Cell;
#[cfg(test)]
use cell::Shape;

/// Open one of TweedPlot's GeoPackages read-only.
pub fn open_db(path: &Path) -> Result<Connection, rusqlite::Error> {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
}

/// What an encrypted cell found in a chart folder is told.
pub const NOT_IMPORTED: &str =
    "is an encrypted (S-63) cell. Encrypted charts need the TweedPlot S-63 Charts plugin: install it and import the exchange set in Chart › Encrypted charts";

/// A plain cell and its updates. Updates are applied in sequence from the base cell's own
/// update number; a missing one stops them there (SSE 23).
fn open_cell(c: &CellFiles) -> Result<Cell, cell::CellError> {
    let base = std::fs::read(&c.base)?;
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
        match std::fs::read(u) {
            Ok(b) => ups.push(b),
            Err(e) => {
                note(u, e.to_string());
                break;
            }
        }
        next += 1;
    }
    let refs: Vec<&[u8]> = ups.iter().map(Vec::as_slice).collect();
    Cell::from_bytes(&base, &refs)
}

/// What happened to a cell that could not be opened, or opens with a warning .
#[derive(Debug, Clone, serde::Serialize)]
pub struct Note {
    pub file: String,
    pub cell: String,
    pub text: String,
}

static NOTES: std::sync::Mutex<Vec<Note>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn note(file: &Path, text: impl Into<String>) {
    let text = text.into();
    eprintln!("ENC {}: {text}", file.display());
    let cell = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut n = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    n.retain(|x| !(x.file == file.display().to_string() && x.text == text));
    n.push(Note { file: file.display().to_string(), cell, text });
}

/// Notes from converting cells since the charts were last opened.
pub fn notes() -> Vec<Note> {
    NOTES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Forget the notes (before the charts are opened again).
pub fn clear_notes() {
    NOTES.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
    ensure_gpkgs_sets(dir, &[], progress)
}

/// A chart set: one top-level folder of an ENC folder (an exchange set such as NOAA's
/// ENC_ROOT, or the IHO's test data), or the cells straight in the ENC folder. Sets can be
/// left out of use. (Cells installed from S-63 exchange sets belong to the S-63 Charts
/// plugin, which offers them as a set of its own.)
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChartSet {
    /// What turns it on and off: the set's folder (the ENC folder itself for its loose cells).
    pub path: PathBuf,
    pub name: String,
    pub cells: usize,
    /// Producer codes (the first two letters of the cell names: US, NZ, GB …) and how many cells each.
    pub producers: Vec<(String, usize)>,
}

/// The set a cell belongs to: the first folder under `dir` ("" for cells straight in `dir`).
fn set_of(dir: &Path, base: &Path) -> String {
    base.strip_prefix(dir)
        .ok()
        .and_then(|r| {
            let mut c = r.components();
            let first = c.next()?;
            c.next().map(|_| first.as_os_str().to_string_lossy().to_string())
        })
        .unwrap_or_default()
}

fn set_path(dir: &Path, set: &str) -> PathBuf {
    if set.is_empty() {
        dir.to_path_buf()
    } else {
        dir.join(set)
    }
}

/// The chart sets in an ENC folder (walks every cell: for a settings list, not at start-up).
pub fn chart_sets(dir: &Path) -> Vec<ChartSet> {
    let cells = find_cells(dir);
    let mut by: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for c in &cells {
        let code: String = c.base.file_stem().map(|s| s.to_string_lossy().to_ascii_uppercase().chars().take(2).collect()).unwrap_or_default();
        *by.entry(set_of(dir, &c.base)).or_default().entry(code).or_default() += 1;
    }
    by.into_iter()
        .map(|(set, prod)| {
            let mut producers: Vec<(String, usize)> = prod.into_iter().collect();
            producers.sort_by_key(|p| std::cmp::Reverse(p.1));
            ChartSet {
                path: set_path(dir, &set),
                name: match set.as_str() {
                    "" => "Cells in the ENC folder itself".into(),
                    s => s.to_string(),
                },
                cells: producers.iter().map(|p| p.1).sum(),
                producers,
            }
        })
        .collect()
}

/// [`ensure_gpkgs_with`], leaving out the chart sets `off` (their folders; `<dir>` itself
/// for the cells straight in it). Each set has its
/// own merged files, so a set turned off and on again is ready at once.
pub fn ensure_gpkgs_sets(dir: &Path, off: &[PathBuf], progress: &mut dyn FnMut(usize, usize, &str)) -> Vec<PathBuf> {
    let loose_off = off.iter().any(|o| o == dir);
    let cells: Vec<CellFiles> = find_cells_except(dir, off).into_iter().filter(|c| !(loose_off && set_of(dir, &c.base).is_empty())).collect();
    if cells.is_empty() {
        return vec![];
    }
    let out_dir = dir.join(".enc");
    let _ = std::fs::create_dir_all(&out_dir);
    // what is there already, read once (a big exchange set has thousands of cells), and
    // indexed by each conversion's "<CELL>-<folder hash>" so finding a cell's is one look-up
    // (searching the whole list for each of NOAA's 7,000 cells took tens of seconds)
    let mut existing_time: std::collections::HashMap<String, std::time::SystemTime> = std::collections::HashMap::new();
    let existing: Vec<String> = std::fs::read_dir(&out_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                existing_time.insert(n.clone(), t);
            }
            n
        })
        .collect();
    let mut by_key: std::collections::HashMap<&str, Vec<&String>> = std::collections::HashMap::new();
    for e in &existing {
        let mut dash = e.match_indices('-').map(|(i, _)| i);
        if let (Some(_), Some(second)) = (dash.next(), dash.next()) {
            by_key.entry(&e[..second]).or_default().push(e);
        }
    }
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let suffix = format!("-v{VERSION}.gpkg");
    let encrypted = |p: &Path| -> bool {
        let mut b = [0u8; 8];
        std::fs::File::open(p).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).is_ok() && !(b[..5].iter().all(u8::is_ascii_digit) && b[6] == b'L')
    };
    // encrypted (S-63) cells are the plugin's: imported with it, never read here
    let cells: Vec<CellFiles> = cells
        .into_iter()
        .filter(|c| {
            if encrypted(&c.base) {
                let name = c.base.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                note(&c.base, format!("{name} {NOT_IMPORTED}"));
                return false;
            }
            true
        })
        .collect();
    // each cell's own conversion: "<CELL>-<folder hash>-b<band>-v<version>.gpkg" (or .tpx)
    let plan: Vec<(CellFiles, String, Option<PathBuf>)> = cells
        .into_iter()
        .map(|c| {
            let stem = c.base.file_stem().map(|s| s.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            let rel = c.base.parent().and_then(|p| p.strip_prefix(dir).ok()).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let key = format!("{stem}-{:08x}", hash(&rel));
            // its conversion, if one is newer than the cell's files
            let kb = format!("{key}-b");
            let newest = c.newest;
            let fresh = by_key
                .get(key.as_str())
                .into_iter()
                .flatten()
                .filter(|e| e.starts_with(&kb) && e.ends_with(&suffix))
                .find(|e| matches!((newest, existing_time.get(e.as_str())), (Some(n), Some(m)) if *m >= n))
                .map(|e| out_dir.join(e.as_str()));
            (c, key, fresh)
        })
        .collect();
    let todo = plan.iter().filter(|p| p.2.is_none()).count();
    let mut done = 0;
    let mut by_band: BTreeMap<(String, u8), Vec<PathBuf>> = BTreeMap::new();
    for (c, key, fresh) in plan {
        let set = set_of(dir, &c.base);
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
                let kd = format!("{key}-");
                for e in existing.iter().filter(|e| e.starts_with(&kd)) {
                    let _ = std::fs::remove_file(out_dir.join(e));
                }
                let cell = match open_cell(&c) {
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
            by_band.entry((set, b)).or_default().push(gpkg);
        }
    }
    if todo > 0 {
        progress(todo, todo, "");
    }
    // then the cells of each band merged into one file, which is what the chart store
    // opens: one table per layer and band instead of one per cell (thousands of cells)
    // one merged file per set and band: "all-<set tag>-b<band>-m<version>"
    let set_tag = |set: &str| format!("{:08x}", hash(&format!("{}|{set}", dir.to_string_lossy())));
    let mut out = Vec::new();
    let mut tags_in_use = std::collections::HashSet::new();
    for ((set, band), files) in by_band {
        let tag = set_tag(&set);
        tags_in_use.insert(tag.clone());
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
    // merged files of bands no longer present in the sets in use (a set that's off keeps
    // its merged files, ready for when it's turned on), and the older one-per-folder files
    let old_tag = format!("all-{:08x}-", hash(&dir.to_string_lossy()));
    for e in existing.iter().filter(|e| e.starts_with("all-")) {
        let tag = e.get(4..12).unwrap_or("");
        if !(tags_in_use.contains(tag) || e.starts_with(&old_tag)) {
            continue;
        }
        let p = out_dir.join(e);
        if !out.iter().any(|o| p.starts_with(o) || e.starts_with(&o.file_name().unwrap().to_string_lossy().to_string())) {
            let _ = std::fs::remove_file(p);
        }
    }
    out
}

/// Whether a merged file was made from exactly these cell files (names and times).
fn merged_is(merged: &Path, stamp: &[(String, i64)]) -> bool {
    if !merged.is_file() {
        return false;
    }
    let Ok(db) = open_db(merged) else { return false };
    let mut want = stamp.to_vec();
    want.sort();
    enc_s57::gpkg::merged_stamp(&db) == want
}

/// Merge cells' GeoPackages into one file.
fn merge(files: &[PathBuf], stamp: &[(String, i64)], out: &Path) -> Result<(), rusqlite::Error> {
    let tmp = out.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut db = Connection::open(&tmp)?;
    enc_s57::gpkg::merge_into(&mut db, files, stamp, &mut enc_s57::gpkg::attach_file)?;
    drop(db);
    let _ = std::fs::remove_file(PathBuf::from(format!("{}.tables", out.display())));
    let _ = std::fs::remove_file(out);
    std::fs::rename(&tmp, out).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
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
        // (19: with the land detail kept for the IHO portrayal)
        assert_eq!(write_gpkg(&c, &out).unwrap(), 19);
        // the chart store reads it like the LINZ layers
        let store = crate::ChartStore::open_dirs(std::slice::from_ref(&dir)).unwrap();
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
        let store = crate::ChartStore::open_dirs(std::slice::from_ref(&dir)).unwrap();
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

    /// Two chart sets: one left out isn't opened, and back in use its merged file is reused.
    #[test]
    fn chart_sets_on_and_off() {
        let dir = std::env::temp_dir().join(format!("enc_sets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for set in ["NOAA/ENC_ROOT/US/1B5X02NE/1/0", "IHO test"] {
            std::fs::create_dir_all(dir.join(set)).unwrap();
            std::fs::write(dir.join(set).join("1B5X02NE.000"), include_bytes!("../../tests/data/1B5X02NE.000")).unwrap();
        }
        let sets = chart_sets(&dir);
        assert_eq!(sets.iter().map(|s| (s.name.as_str(), s.cells)).collect::<Vec<_>>(), vec![("IHO test", 1), ("NOAA", 1)]);
        assert_eq!(sets[0].producers, vec![("1B".to_string(), 1)]);
        let both = ensure_gpkgs_sets(&dir, &[], &mut |_, _, _| {});
        assert_eq!(both.len(), 2, "a merged file per set");
        let noaa_off = ensure_gpkgs_sets(&dir, &[dir.join("NOAA")], &mut |_, _, _| {});
        assert_eq!(noaa_off.len(), 1);
        let kept: Vec<_> = both.iter().filter(|p| p.exists()).collect();
        assert_eq!(kept.len(), 2, "a set that's off keeps its merged file");
        let made = std::fs::metadata(both.iter().find(|p| !noaa_off.contains(p)).unwrap()).unwrap().modified().unwrap();
        let again = ensure_gpkgs_sets(&dir, &[], &mut |_, _, _| {});
        assert_eq!(again.len(), 2);
        let noaa = again.iter().find(|p| !noaa_off.contains(p)).unwrap();
        assert_eq!(std::fs::metadata(noaa).unwrap().modified().unwrap(), made, "back in use without merging again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An encrypted (S-63) cell in a chart folder is left alone, with a note.
    #[test]
    fn encrypted_cells_are_left_to_the_plugin() {
        let dir = std::env::temp_dir().join(format!("enc_s63_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("set")).unwrap();
        // not an ISO 8211 file: what an S-63 cell looks like from outside
        std::fs::write(dir.join("set/GB61021B.000"), [0x9fu8; 64]).unwrap();
        assert!(ensure_gpkgs(&dir).is_empty());
        assert!(notes().iter().any(|n| n.text.contains("S-63 Charts plugin")));
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

#[cfg(test)]
mod scale_check {
    /// cargo test --release -p chart enc_check_at_noaa_scale -- --ignored --nocapture
    /// 7,000 cells in an exchange set's layout, all converted already: how long the start-up check takes.
    #[test]
    #[ignore]
    fn enc_check_at_noaa_scale() {
        let dir = std::env::temp_dir().join(format!("tp_encscale_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join(".enc");
        std::fs::create_dir_all(&out).unwrap();
        let mut names = Vec::new();
        for k in 0..7000 {
            let cell = format!("US{}X{:04}", 1 + k % 6, k);
            let leaf = dir.join("ENC_ROOT").join(&cell).join("1").join("0");
            std::fs::create_dir_all(&leaf).unwrap();
            std::fs::write(leaf.join(format!("{cell}.000")), b"000003LE").unwrap();
            let rel = leaf.strip_prefix(&dir).unwrap().to_string_lossy().to_string();
            names.push(format!("{cell}-{:08x}-b{}-v{}.gpkg", super::hash(&rel), 1 + k % 6, super::VERSION));
        }
        for n in &names {
            std::fs::write(out.join(n), b"x").unwrap();
            // plus an old version's file, as a real .enc folder has
            std::fs::write(out.join(n.replace(&format!("-v{}", super::VERSION), "-v1")), b"x").unwrap();
        }
        let t = std::time::Instant::now();
        let cells = super::find_cells(&dir);
        println!("find_cells: {} in {:?}", cells.len(), t.elapsed());
        let t = std::time::Instant::now();
        let mut todo = 0;
        let _ = super::ensure_gpkgs_with(&dir, &mut |_, n, _| todo = n);
        println!("whole check: {:?} (cells to convert {todo})", t.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
