//! World coastline from GSHHG (Global Self-consistent, Hierarchical,
//! High-resolution Geography; Wessel & Smith, LGPL), shapefile edition.
//!
//! GSHHG comes at five resolutions: crude, low, intermediate, high and full
//! (full keeps every island, ~100 m). The shapefiles are read once from the
//! zip (`gshhg-shp-2.3.7.zip`) or an unpacked `GSHHS_shp` folder and written
//! to a GeoPackage with an R-tree per resolution, so tiles and routing read
//! only what they need:
//!
//! | table | resolution | drawn at zoom |
//! | --- | --- | --- |
//! | `gshhs_c` | crude | 0–1 |
//! | `gshhs_l` | low | 2–3 |
//! | `gshhs_i` | intermediate | 4–5 |
//! | `gshhs_h` | high | 6–7 |
//! | `gshhs_f` | full | 8 and up |
//!
//! Levels used: L1 (land and ocean islands), L2 (lakes), L3 (islands in lakes) and
//! L5 (Antarctic ice front). L1 takes in the lakes, so tiles cut the L2 lakes out of
//! it and put the L3 islands back (the `level` column says which is which).
//!
//! Each resolution has land polygons cut into pieces of at most a few thousand
//! points (`gshhs_<r>`, for the land fill) and the coastline as lines in
//! chunks (`gshhs_<r>_ln`, for the coast line: the cuts would draw as lines
//! across the land if the polygon outlines were used).

use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::ChartError;

pub const RESOLUTIONS: [char; 5] = ['c', 'l', 'i', 'h', 'f'];
const LEVELS: [u8; 4] = [1, 2, 3, 5];
/// GSHHG level of lake polygons, and of islands in lakes.
pub const LAKE: u8 = 2;
pub const LAKE_ISLAND: u8 = 3;
/// Bump when the built file changes shape, so old builds are redone.
const BUILD_VERSION: i64 = 3;
/// Land polygons bigger than this are cut into pieces (so a tile reads only
/// the piece it needs: Eurasia at full resolution is over a million points).
const MAX_PIECE: usize = 4000;
/// Coastline lines are kept in chunks of this many points.
const LINE_CHUNK: usize = 1500;

/// Zoom range a resolution is drawn at.
/// How much finer than standard the drawn coastline is: 0 standard (the table
/// above), 1 one resolution finer at each zoom, 2 two finer. Set from the app.
pub static LAND_DETAIL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The resolution drawn at zoom `z` with `detail` steps finer than standard.
pub fn res_for_zoom(z: u8, detail: u8) -> char {
    let base = match z {
        0..=1 => 0,
        2..=3 => 1,
        4..=5 => 2,
        6..=7 => 3,
        _ => 4,
    };
    RESOLUTIONS[(base + detail as usize).min(4)]
}

/// Resolution to use for a routing obstacle map with cells of `cell_m` metres.
pub fn res_for_cell(cell_m: f64) -> char {
    if cell_m < 400.0 {
        'f'
    } else if cell_m < 2000.0 {
        'h'
    } else {
        'i'
    }
}

enum Source {
    Zip(PathBuf),
    Dir(PathBuf),
}

/// A GSHHG shapefile set in `dir`: a gshhg-shp zip, or a GSHHS_shp folder.
fn find_source(dir: &Path) -> Option<Source> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut zip = None;
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().to_ascii_lowercase();
        if p.is_dir() && name == "gshhs_shp" {
            return Some(Source::Dir(p));
        }
        if name.starts_with("gshhg-shp") && name.ends_with(".zip") {
            zip = Some(Source::Zip(p));
        }
    }
    zip
}

/// Build (once) the GeoPackage from GSHHG shapefiles found in `dir`;
/// returns its path, or None when there is no GSHHG in `dir`.
pub fn ensure_gpkg(dir: &Path, out_dir: &Path) -> Result<Option<PathBuf>, ChartError> {
    let Some(src) = find_source(dir) else { return Ok(None) };
    let out = out_dir.join("gshhg_land.gpkg");
    let stamp = match &src {
        Source::Zip(p) => std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0),
        Source::Dir(_) => 0,
    };
    if out.exists() {
        if let Ok(c) = Connection::open(&out) {
            let ok: Option<(i64, i64)> = c
                .query_row("SELECT version, source_size FROM racenav_build", [], |r| Ok((r.get(0)?, r.get(1)?)))
                .ok();
            if ok == Some((BUILD_VERSION, stamp)) {
                return Ok(Some(out));
            }
        }
    }
    std::fs::create_dir_all(out_dir)?;
    let tmp = out.with_extension("gpkg.part");
    let _ = std::fs::remove_file(&tmp);
    let mut db = Connection::open(&tmp)?;
    init_gpkg(&db)?;
    for res in RESOLUTIONS {
        let table = format!("gshhs_{res}");
        let ltable = format!("gshhs_{res}_ln");
        create_table(&db, &table, "POLYGON")?;
        create_table(&db, &ltable, "LINESTRING")?;
        let tx = db.transaction()?;
        {
            let mut ins = tx.prepare(&format!("INSERT INTO \"{table}\" (fid, geom, level) VALUES (?1, ?2, ?3)"))?;
            let mut rt = tx.prepare(&format!("INSERT INTO \"rtree_{table}_geom\" (id, minx, maxx, miny, maxy) VALUES (?1, ?2, ?3, ?4, ?5)"))?;
            let mut lins = tx.prepare(&format!("INSERT INTO \"{ltable}\" (fid, geom, level) VALUES (?1, ?2, ?3)"))?;
            let mut lrt = tx.prepare(&format!("INSERT INTO \"rtree_{ltable}_geom\" (id, minx, maxx, miny, maxy) VALUES (?1, ?2, ?3, ?4, ?5)"))?;
            let (mut fid, mut lfid) = (0i64, 0i64);
            for level in LEVELS {
                let Some(bytes) = read_shp(&src, res, level)? else { continue };
                for ring in shp_rings(&bytes) {
                    let b = bounds(&ring);
                    for piece in split_ring(ring.clone(), b) {
                        fid += 1;
                        let pb = bounds(&piece);
                        ins.execute(rusqlite::params![fid, gpkg_polygon(&piece), level])?;
                        rt.execute(rusqlite::params![fid, pb[0], pb[2], pb[1], pb[3]])?;
                    }
                    // the coastline itself, in chunks that overlap by a point
                    let mut i = 0;
                    while i + 1 < ring.len() {
                        let chunk = &ring[i..(i + LINE_CHUNK + 1).min(ring.len())];
                        lfid += 1;
                        let cb = bounds(chunk);
                        lins.execute(rusqlite::params![lfid, gpkg_line(chunk), level])?;
                        lrt.execute(rusqlite::params![lfid, cb[0], cb[2], cb[1], cb[3]])?;
                        i += LINE_CHUNK;
                    }
                }
            }
        }
        tx.commit()?;
    }
    db.execute_batch(&format!(
        "CREATE TABLE racenav_build (version INTEGER, source_size INTEGER); INSERT INTO racenav_build VALUES ({BUILD_VERSION}, {stamp});"
    ))?;
    drop(db);
    std::fs::rename(&tmp, &out)?;
    Ok(Some(out))
}

fn read_shp(src: &Source, res: char, level: u8) -> Result<Option<Vec<u8>>, ChartError> {
    let rel = format!("{res}/GSHHS_{res}_L{level}.shp");
    match src {
        Source::Dir(d) => {
            let p = d.join(&rel);
            Ok(if p.exists() { Some(std::fs::read(p)?) } else { None })
        }
        Source::Zip(z) => {
            let mut za = zip::ZipArchive::new(std::fs::File::open(z)?)?;
            let want = format!("GSHHS_shp/{rel}");
            let name = (0..za.len()).find_map(|i| {
                let n = za.by_index(i).ok()?.name().ok()?.to_string();
                (n == want || n.ends_with(&format!("/{want}"))).then_some(n)
            });
            let Some(name) = name else { return Ok(None) };
            let mut f = za.by_name(&name)?;
            let mut v = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut v)?;
            Ok(Some(v))
        }
    }
}

/// Every ring of every polygon record in a shapefile (shape types 5, 15, 25).
/// GSHHG land polygons have no holes, so each ring is a polygon of its own.
pub fn shp_rings(b: &[u8]) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    if b.len() < 100 {
        return out;
    }
    let be_i32 = |o: usize| b.get(o..o + 4).map(|x| i32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    let le_i32 = |o: usize| b.get(o..o + 4).map(|x| i32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    let le_f64 = |o: usize| b.get(o..o + 8).map(|x| f64::from_le_bytes(x.try_into().unwrap()));
    let mut o = 100;
    while o + 8 <= b.len() {
        let Some(len_words) = be_i32(o + 4) else { break };
        let content = o + 8;
        let end = content + len_words.max(0) as usize * 2;
        if end > b.len() {
            break;
        }
        let ty = le_i32(content).unwrap_or(0);
        if matches!(ty, 5 | 15 | 25) {
            let np = le_i32(content + 36).unwrap_or(0).max(0) as usize;
            let npt = le_i32(content + 40).unwrap_or(0).max(0) as usize;
            let parts: Vec<usize> = (0..np).filter_map(|i| le_i32(content + 44 + i * 4)).map(|v| v.max(0) as usize).collect();
            let pts0 = content + 44 + np * 4;
            for (k, &s) in parts.iter().enumerate() {
                let e = parts.get(k + 1).copied().unwrap_or(npt).min(npt);
                let ring: Vec<[f64; 2]> =
                    (s..e).filter_map(|i| Some([le_f64(pts0 + i * 16)?, le_f64(pts0 + i * 16 + 8)?])).collect();
                if ring.len() >= 4 {
                    out.push(ring);
                }
            }
        }
        o = end;
    }
    out
}

fn bounds(r: &[[f64; 2]]) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in r {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

/// GeoPackage geometry blob: GP header (little endian, no envelope, SRS 4326) + WKB polygon.
fn gpkg_polygon(ring: &[[f64; 2]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + 13 + ring.len() * 16);
    v.extend_from_slice(b"GP");
    v.push(0); // version
    v.push(0x01); // little-endian header, no envelope
    v.extend_from_slice(&4326i32.to_le_bytes());
    v.push(1); // WKB little endian
    v.extend_from_slice(&3u32.to_le_bytes()); // polygon
    v.extend_from_slice(&1u32.to_le_bytes()); // one ring
    v.extend_from_slice(&(ring.len() as u32).to_le_bytes());
    for p in ring {
        v.extend_from_slice(&p[0].to_le_bytes());
        v.extend_from_slice(&p[1].to_le_bytes());
    }
    v
}

fn init_gpkg(db: &Connection) -> Result<(), ChartError> {
    db.execute_batch(
        "PRAGMA application_id = 1196444487; PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
         CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
           organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
         INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\"]', NULL);
         CREATE TABLE gpkg_contents (table_name TEXT PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT, description TEXT DEFAULT '',
           last_change DATETIME DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
           srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));",
    )?;
    Ok(())
}

fn create_table(db: &Connection, table: &str, geom: &str) -> Result<(), ChartError> {
    db.execute_batch(&format!(
        "CREATE TABLE \"{table}\" (fid INTEGER PRIMARY KEY, geom BLOB, level INTEGER);
         CREATE VIRTUAL TABLE \"rtree_{table}_geom\" USING rtree(id, minx, maxx, miny, maxy);
         INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) VALUES ('{table}', 'features', '{table}', 4326);
         INSERT INTO gpkg_geometry_columns VALUES ('{table}', 'geom', '{geom}', 4326, 0, 0);"
    ))?;
    Ok(())
}

/// GeoPackage blob of a line string.
fn gpkg_line(pts: &[[f64; 2]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + 9 + pts.len() * 16);
    v.extend_from_slice(b"GP");
    v.push(0);
    v.push(0x01);
    v.extend_from_slice(&4326i32.to_le_bytes());
    v.push(1);
    v.extend_from_slice(&2u32.to_le_bytes()); // line string
    v.extend_from_slice(&(pts.len() as u32).to_le_bytes());
    for p in pts {
        v.extend_from_slice(&p[0].to_le_bytes());
        v.extend_from_slice(&p[1].to_le_bytes());
    }
    v
}

/// Cut a land polygon into pieces of at most [`MAX_PIECE`] points by halving
/// its box along the longer side, clipping to each half (Sutherland–Hodgman;
/// the pieces fill exactly the same land).
pub fn split_ring(ring: Vec<[f64; 2]>, b: [f64; 4]) -> Vec<Vec<[f64; 2]>> {
    if ring.len() <= MAX_PIECE || (b[2] - b[0]).max(b[3] - b[1]) < 0.02 {
        return vec![ring];
    }
    let halves = if b[2] - b[0] >= b[3] - b[1] {
        let m = (b[0] + b[2]) / 2.0;
        [[b[0], b[1], m, b[3]], [m, b[1], b[2], b[3]]]
    } else {
        let m = (b[1] + b[3]) / 2.0;
        [[b[0], b[1], b[2], m], [b[0], m, b[2], b[3]]]
    };
    let mut out = Vec::new();
    for h in halves {
        let mut piece = crate::clip::clip_ring(&ring, crate::clip::Rect { x0: h[0], y0: h[1], x1: h[2], y1: h[3] });
        if piece.len() < 3 {
            continue;
        }
        piece.push(piece[0]);
        let pb = bounds(&piece);
        out.extend(split_ring(piece, pb));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_big_rings_into_pieces_of_the_same_area() {
        // a jagged circle of 20 000 points
        let n = 20_000;
        let ring: Vec<[f64; 2]> = (0..=n).map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let r = 1.0 + 0.05 * (a * 40.0).sin();
            [r * a.cos(), r * a.sin()]
        }).collect();
        let area = |r: &[[f64; 2]]| (0..r.len() - 1).map(|k| r[k][0] * r[k + 1][1] - r[k + 1][0] * r[k][1]).sum::<f64>().abs() / 2.0;
        let whole = area(&ring);
        let pieces = super::split_ring(ring.clone(), super::bounds(&ring));
        assert!(pieces.len() >= 5, "{}", pieces.len());
        assert!(pieces.iter().all(|p| p.len() <= super::MAX_PIECE + 400), "{:?}", pieces.iter().map(|p| p.len()).collect::<Vec<_>>());
        let sum: f64 = pieces.iter().map(|p| area(p)).sum();
        assert!((sum - whole).abs() / whole < 1e-9, "{sum} v {whole}");
    }

    #[test]
    fn finer_detail_shifts_the_resolutions() {
        use super::res_for_zoom;
        assert_eq!([0, 2, 4, 6, 8, 12].map(|z| res_for_zoom(z, 0)), ['c', 'l', 'i', 'h', 'f', 'f']);
        assert_eq!([0, 2, 4, 6].map(|z| res_for_zoom(z, 1)), ['l', 'i', 'h', 'f']);
        assert_eq!([0, 2, 4].map(|z| res_for_zoom(z, 2)), ['i', 'h', 'f']);
    }

    use super::*;

    /// A shapefile with one polygon record of two rings (a square and a triangle).
    pub(crate) fn tiny_shp() -> Vec<u8> {
        let rings: [&[[f64; 2]]; 2] = [
            &[[170.0, -40.0], [170.0, -39.0], [171.0, -39.0], [171.0, -40.0], [170.0, -40.0]],
            &[[172.0, -41.0], [172.5, -40.5], [173.0, -41.0], [172.0, -41.0]],
        ];
        let npt: usize = rings.iter().map(|r| r.len()).sum();
        let mut c = Vec::new();
        c.extend_from_slice(&5i32.to_le_bytes());
        for v in [170.0f64, -41.0, 173.0, -39.0] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        c.extend_from_slice(&2i32.to_le_bytes());
        c.extend_from_slice(&(npt as i32).to_le_bytes());
        c.extend_from_slice(&0i32.to_le_bytes());
        c.extend_from_slice(&(rings[0].len() as i32).to_le_bytes());
        for r in rings {
            for p in r {
                c.extend_from_slice(&p[0].to_le_bytes());
                c.extend_from_slice(&p[1].to_le_bytes());
            }
        }
        let mut f = vec![0u8; 100];
        f[0..4].copy_from_slice(&9994i32.to_be_bytes());
        f.extend_from_slice(&1i32.to_be_bytes());
        f.extend_from_slice(&((c.len() / 2) as i32).to_be_bytes());
        f.extend_from_slice(&c);
        let total = (f.len() / 2) as i32;
        f[24..28].copy_from_slice(&total.to_be_bytes());
        f
    }

    #[test]
    fn reads_polygon_rings() {
        let r = shp_rings(&tiny_shp());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].len(), 5);
        assert_eq!(r[1][1], [172.5, -40.5]);
    }

    #[test]
    fn builds_a_geopackage_the_store_can_read() {
        let base = std::env::temp_dir().join(format!("racenav-gshhg-{}", std::process::id()));
        let src = base.join("GSHHS_shp");
        for res in RESOLUTIONS {
            std::fs::create_dir_all(src.join(res.to_string())).unwrap();
            std::fs::write(src.join(format!("{res}/GSHHS_{res}_L1.shp")), tiny_shp()).unwrap();
        }
        let out = ensure_gpkg(&base, &base.join(".unzipped")).unwrap().unwrap();
        // second call reuses the build
        let t = std::fs::metadata(&out).unwrap().modified().unwrap();
        ensure_gpkg(&base, &base.join(".unzipped")).unwrap();
        assert_eq!(std::fs::metadata(&out).unwrap().modified().unwrap(), t);

        let store = crate::ChartStore::open_dirs(std::slice::from_ref(&base)).unwrap();
        assert_eq!(store.tables.iter().filter(|t| t.kind == crate::Kind::WorldLand).count(), 5);
        let mut n = 0;
        store.for_each_world_land([169.0, -42.0, 174.0, -38.0], 100.0, |_| n += 1);
        assert_eq!(n, 2);
        // a z9 tile over the square has land in it
        let tile = store.tile(9, 498, 317);
        assert!(!tile.is_empty());
        assert!(store.tile(9, 10, 10).is_empty() || store.tile(9, 10, 10).len() < tile.len());
        let _ = std::fs::remove_dir_all(base);
    }
}
