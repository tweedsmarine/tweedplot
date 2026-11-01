//! Writing GeoPackages: the few tables the chart store reads (features with an R-tree,
//! plus plain attribute tables). Used for data built from downloads (maritime
//! boundaries, fishing rules).

use rusqlite::Connection;

use crate::geom::{Pt, Ring};
use crate::ChartError;

/// A new, empty GeoPackage (journal off: files are built in one go and then renamed).
pub fn create(path: &std::path::Path) -> Result<Connection, ChartError> {
    let _ = std::fs::remove_file(path);
    let db = Connection::open(path)?;
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
    Ok(db)
}

/// A feature table `table` (fid, geom, then `cols` as "name TYPE") with its R-tree.
pub fn feature_table(db: &Connection, table: &str, geom_type: &str, cols: &[&str]) -> Result<(), ChartError> {
    let extra: String = cols.iter().map(|c| format!(", {c}")).collect();
    db.execute_batch(&format!(
        "CREATE TABLE \"{table}\" (fid INTEGER PRIMARY KEY, geom BLOB{extra});
         CREATE VIRTUAL TABLE \"rtree_{table}_geom\" USING rtree(id, minx, maxx, miny, maxy);
         INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) VALUES ('{table}', 'features', '{table}', 4326);
         INSERT INTO gpkg_geometry_columns VALUES ('{table}', 'geom', '{geom_type}', 4326, 0, 0);"
    ))?;
    Ok(())
}

/// Bounds [w, s, e, n] of some points.
pub fn bounds<'a>(pts: impl IntoIterator<Item = &'a Pt>) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

fn header(v: &mut Vec<u8>) {
    v.extend_from_slice(b"GP");
    v.push(0); // version
    v.push(0x01); // little-endian header, no envelope
    v.extend_from_slice(&4326i32.to_le_bytes());
}

fn ring(v: &mut Vec<u8>, r: &[Pt]) {
    v.extend_from_slice(&(r.len() as u32).to_le_bytes());
    for p in r {
        v.extend_from_slice(&p[0].to_le_bytes());
        v.extend_from_slice(&p[1].to_le_bytes());
    }
}

/// GeoPackage blob of a polygon: exterior ring first, then holes.
pub fn polygon(rings: &[Ring]) -> Vec<u8> {
    let n: usize = rings.iter().map(Vec::len).sum();
    let mut v = Vec::with_capacity(8 + 9 + rings.len() * 4 + n * 16);
    header(&mut v);
    v.push(1);
    v.extend_from_slice(&3u32.to_le_bytes());
    v.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for r in rings {
        ring(&mut v, r);
    }
    v
}

/// GeoPackage blob of a line string.
pub fn line(pts: &[Pt]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + 9 + pts.len() * 16);
    header(&mut v);
    v.push(1);
    v.extend_from_slice(&2u32.to_le_bytes());
    ring(&mut v, pts);
    v
}

/// Lines in chunks of at most `n` points that overlap by a point (so a tile reads a
/// short piece of a long boundary, not all of it).
pub fn chunks(pts: &[Pt], n: usize) -> Vec<&[Pt]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < pts.len() {
        out.push(&pts[i..(i + n + 1).min(pts.len())]);
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_read_back() {
        let rings = vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]], vec![[1.0, 1.0], [1.0, 2.0], [2.0, 2.0], [1.0, 1.0]]];
        assert_eq!(crate::geom::parse_gpkg(&polygon(&rings)), Some(crate::geom::Geom::Polygons(vec![rings])));
        let l = vec![[1.0, 2.0], [3.0, 4.0]];
        assert_eq!(crate::geom::parse_gpkg(&line(&l)), Some(crate::geom::Geom::Lines(vec![l])));
        let long: Vec<Pt> = (0..10).map(|i| [i as f64, 0.0]).collect();
        let c = chunks(&long, 4);
        assert_eq!(c.iter().map(|c| c.len()).collect::<Vec<_>>(), vec![5, 5, 2]);
    }
}
