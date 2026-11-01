//! World depths where there is no hydrographic chart.
//!
//! Two sources, both stored as GeoPackages the [`ChartStore`](crate::ChartStore)
//! reads like any other chart file and draws under the LINZ depth areas:
//!
//! - **Natural Earth 10 m bathymetry** (public domain): nested polygons deeper
//!   than 0, 200, 1000, 2000 … 10000 m, worldwide. Bundled with the app as
//!   `world_depth.gpkg`, tables `wdepth_ne` and a lighter `wdepth_nelo` for small
//!   scales, column `depth`.
//! - **ETOPO 2022** (NOAA NCEI, free to use and redistribute): a 15/30/60
//!   arc-second height grid downloaded for an area. [`write_grid`] turns it
//!   into depth bands (cell runs merged into rectangles, table `wdepth_etopo`)
//!   and smooth contours (marching squares, table `wdcnt_etopo`), one file per
//!   area.
//!
//! Neither is for navigation: Natural Earth is coarse, and ETOPO's coastal
//! depths are often interpolated. Routing does not use them.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::Connection;

use crate::ChartError;

/// Depth bands for ETOPO areas (metres, lower bound of each band).
pub const ETOPO_LEVELS: [f64; 16] = [0.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 3000.0, 4000.0, 5000.0, 6000.0, 8000.0];
/// Depths drawn as contour lines.
pub const CONTOUR_LEVELS: [f64; 12] = [5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 3000.0, 4000.0, 5000.0];

/// A regular height grid (metres, positive up; NaN = missing), rows south to north.
#[derive(Debug, Clone)]
pub struct Grid {
    pub west: f64,
    pub south: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub nx: usize,
    pub ny: usize,
    pub z: Vec<f32>,
}

impl Grid {
    fn depth(&self, i: usize, j: usize) -> f64 {
        -(self.z[j * self.nx + i] as f64)
    }
    fn lon(&self, i: f64) -> f64 {
        self.west + i * self.dlon
    }
    fn lat(&self, j: f64) -> f64 {
        self.south + j * self.dlat
    }
}

// ---------- GeoPackage writing ----------------------------------------------------------

fn header(v: &mut Vec<u8>) {
    v.extend_from_slice(b"GP");
    v.push(0);
    v.push(0x01);
    v.extend_from_slice(&4326i32.to_le_bytes());
}

/// GeoPackage blob of a polygon with any number of rings (exterior first).
pub fn gpkg_polygon(rings: &[Vec<[f64; 2]>]) -> Vec<u8> {
    let n: usize = rings.iter().map(|r| r.len()).sum();
    let mut v = Vec::with_capacity(17 + rings.len() * 4 + n * 16);
    header(&mut v);
    v.push(1);
    v.extend_from_slice(&3u32.to_le_bytes());
    v.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for r in rings {
        v.extend_from_slice(&(r.len() as u32).to_le_bytes());
        for p in r {
            v.extend_from_slice(&p[0].to_le_bytes());
            v.extend_from_slice(&p[1].to_le_bytes());
        }
    }
    v
}

/// GeoPackage blob of a line string.
pub fn gpkg_line(pts: &[[f64; 2]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(17 + pts.len() * 16);
    header(&mut v);
    v.push(1);
    v.extend_from_slice(&2u32.to_le_bytes());
    v.extend_from_slice(&(pts.len() as u32).to_le_bytes());
    for p in pts {
        v.extend_from_slice(&p[0].to_le_bytes());
        v.extend_from_slice(&p[1].to_le_bytes());
    }
    v
}

fn bounds(pts: impl Iterator<Item = [f64; 2]>) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

/// A GeoPackage being written: open a new file, add feature tables, insert, finish.
pub struct GpkgWriter {
    db: Connection,
    tmp: std::path::PathBuf,
    out: std::path::PathBuf,
}

impl GpkgWriter {
    pub fn create(out: &Path) -> Result<Self, ChartError> {
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = out.with_extension("gpkg.part");
        let _ = std::fs::remove_file(&tmp);
        let db = Connection::open(&tmp)?;
        db.execute_batch(
            "PRAGMA application_id = 1196444487; PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
             CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
               organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
             INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\"]', NULL);
             CREATE TABLE gpkg_contents (table_name TEXT PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT, description TEXT DEFAULT '',
               last_change DATETIME DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
             CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
               srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));
             CREATE TABLE racenav_meta (key TEXT PRIMARY KEY, value TEXT);",
        )?;
        Ok(Self { db, tmp, out: out.to_path_buf() })
    }

    /// A feature table with a `depth` column.
    pub fn table(&self, name: &str, geom_type: &str) -> Result<(), ChartError> {
        self.db.execute_batch(&format!(
            "CREATE TABLE \"{name}\" (fid INTEGER PRIMARY KEY, geom BLOB, depth REAL);
             CREATE VIRTUAL TABLE \"rtree_{name}_geom\" USING rtree(id, minx, maxx, miny, maxy);
             INSERT INTO gpkg_contents (table_name, data_type, identifier, srs_id) VALUES ('{name}', 'features', '{name}', 4326);
             INSERT INTO gpkg_geometry_columns VALUES ('{name}', 'geom', '{geom_type}', 4326, 0, 0);"
        ))?;
        Ok(())
    }

    pub fn meta(&self, key: &str, value: &str) -> Result<(), ChartError> {
        self.db.execute("INSERT OR REPLACE INTO racenav_meta VALUES (?1, ?2)", rusqlite::params![key, value])?;
        Ok(())
    }

    /// Insert features `(geometry blob, bounds, depth)` into `table` in one transaction.
    pub fn insert(&mut self, table: &str, rows: impl IntoIterator<Item = (Vec<u8>, [f64; 4], f64)>) -> Result<usize, ChartError> {
        let tx = self.db.transaction()?;
        let mut n = 0;
        {
            let mut ins = tx.prepare(&format!("INSERT INTO \"{table}\" (geom, depth) VALUES (?1, ?2)"))?;
            let mut rt = tx.prepare(&format!("INSERT INTO \"rtree_{table}_geom\" (id, minx, maxx, miny, maxy) VALUES (?1, ?2, ?3, ?4, ?5)"))?;
            for (g, b, d) in rows {
                ins.execute(rusqlite::params![g, d])?;
                let id = tx.last_insert_rowid();
                rt.execute(rusqlite::params![id, b[0], b[2], b[1], b[3]])?;
                n += 1;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    pub fn finish(self) -> Result<std::path::PathBuf, ChartError> {
        self.db.execute_batch("VACUUM")?;
        drop(self.db);
        std::fs::rename(&self.tmp, &self.out)?;
        Ok(self.out)
    }
}

/// Insert polygons (rings, depth) as features.
pub fn polygon_rows(polys: Vec<(Vec<Vec<[f64; 2]>>, f64)>) -> impl Iterator<Item = (Vec<u8>, [f64; 4], f64)> {
    polys.into_iter().filter(|(r, _)| !r.is_empty() && r[0].len() >= 4).map(|(rings, d)| {
        let b = bounds(rings[0].iter().copied());
        (gpkg_polygon(&rings), b, d)
    })
}

// ---------- ETOPO grid to depth bands and contours ------------------------------------------

/// Band index of a depth (index into ETOPO_LEVELS, or None for land / missing).
fn band(depth: f64) -> Option<usize> {
    if !depth.is_finite() || depth <= 0.0 {
        return None;
    }
    ETOPO_LEVELS.iter().rposition(|&l| depth > l)
}

/// Cells per side of the blocks the depth areas are traced in (keeps each
/// polygon small, so a tile reads only what is near it).
const BLOCK: usize = 256;

/// Nested depth areas: for each level, the water deeper than it, as polygons
/// traced round the grid cells (cells centred on the grid nodes), in blocks of
/// [`BLOCK`] cells. Drawn shallow to deep, each level covers the one before, so
/// no gap can open between bands however the tiles round the edges (row-by-row
/// rectangles left slivers that vanished when zoomed out).
pub fn bands(g: &Grid) -> Vec<(Vec<Vec<[f64; 2]>>, f64)> {
    let mut out = Vec::new();
    for (k, &level) in ETOPO_LEVELS.iter().enumerate() {
        // band(d) >= k  <=>  d > level (and wet)
        let wet = |i: usize, j: usize| band(g.depth(i, j)).is_some_and(|b| b >= k);
        for j0 in (0..g.ny).step_by(BLOCK) {
            for i0 in (0..g.nx).step_by(BLOCK) {
                let (i1, j1) = ((i0 + BLOCK).min(g.nx), (j0 + BLOCK).min(g.ny));
                for poly in trace(i0, i1, j0, j1, &wet) {
                    let rings = poly
                        .into_iter()
                        .map(|r| {
                            let mut pts: Vec<[f64; 2]> = r.into_iter().map(|(ci, cj)| [g.lon(ci as f64 - 0.5), g.lat(cj as f64 - 0.5)]).collect();
                            pts.push(pts[0]); // closed, as GeoPackage rings are
                            pts
                        })
                        .collect();
                    out.push((rings, level));
                }
            }
        }
    }
    out
}

type Corner = (i64, i64);

/// Outlines of the cells where `inside(i, j)` within the block, as polygons
/// (outer ring first, then its holes), in cell-corner coordinates: corner
/// (i, j) is the south-west corner of cell (i, j).
fn trace(i0: usize, i1: usize, j0: usize, j1: usize, inside: &dyn Fn(usize, usize) -> bool) -> Vec<Vec<Vec<Corner>>> {
    let (w, h) = (i1 - i0, j1 - j0);
    let mut mask = vec![false; w * h];
    let mut any = false;
    for j in 0..h {
        for i in 0..w {
            let v = inside(i0 + i, j0 + j);
            mask[j * w + i] = v;
            any |= v;
        }
    }
    if !any {
        return Vec::new();
    }
    let at = |i: i64, j: i64| i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h && mask[j as usize * w + i as usize];
    // boundary edges with the inside on the left (outer rings anticlockwise)
    let mut edges: HashMap<Corner, Vec<Corner>> = HashMap::new();
    let mut n_edges = 0usize;
    for j in 0..h as i64 {
        for i in 0..w as i64 {
            if !at(i, j) {
                continue;
            }
            let mut add = |a: Corner, b: Corner| {
                edges.entry(a).or_default().push(b);
                n_edges += 1;
            };
            if !at(i, j - 1) {
                add((i, j), (i + 1, j));
            }
            if !at(i + 1, j) {
                add((i + 1, j), (i + 1, j + 1));
            }
            if !at(i, j + 1) {
                add((i + 1, j + 1), (i, j + 1));
            }
            if !at(i - 1, j) {
                add((i, j + 1), (i, j));
            }
        }
    }
    // chain the edges into rings; where two rings touch at a corner, turn left
    let mut rings: Vec<Vec<Corner>> = Vec::new();
    while n_edges > 0 {
        let start = *edges.iter().find(|(_, v)| !v.is_empty()).unwrap().0;
        let mut ring = vec![start];
        let mut cur = start;
        let mut dir: Option<Corner> = None;
        loop {
            let outs = edges.get_mut(&cur).unwrap();
            let pick = match dir {
                Some((dx, dy)) if outs.len() > 1 => {
                    let left = (cur.0 - dy, cur.1 + dx);
                    outs.iter().position(|&p| p == left).unwrap_or(0)
                }
                _ => 0,
            };
            let next = outs.swap_remove(pick);
            n_edges -= 1;
            dir = Some((next.0 - cur.0, next.1 - cur.1));
            cur = next;
            if cur == start {
                break;
            }
            ring.push(cur);
        }
        // drop the corners along straight runs
        let n = ring.len();
        let kept: Vec<Corner> = (0..n)
            .filter(|&k| {
                let (p, c, q) = (ring[(k + n - 1) % n], ring[k], ring[(k + 1) % n]);
                (c.0 - p.0) * (q.1 - c.1) != (c.1 - p.1) * (q.0 - c.0)
            })
            .map(|k| ring[k])
            .collect();
        if kept.len() >= 4 {
            rings.push(kept.into_iter().map(|(i, j)| (i + i0 as i64, j + j0 as i64)).collect());
        }
    }
    let area2 = |r: &[Corner]| -> i64 {
        (0..r.len())
            .map(|k| {
                let (a, b) = (r[k], r[(k + 1) % r.len()]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum()
    };
    let contains = |r: &[Corner], x: f64, y: f64| {
        let mut c = false;
        for k in 0..r.len() {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            let (ax, ay, bx, by) = (a.0 as f64, a.1 as f64, b.0 as f64, b.1 as f64);
            if (ay > y) != (by > y) && x < ax + (y - ay) / (by - ay) * (bx - ax) {
                c = !c;
            }
        }
        c
    };
    let (outers, holes): (Vec<_>, Vec<_>) = rings.into_iter().map(|r| (area2(&r), r)).partition(|(a, _)| *a > 0);
    let mut polys: Vec<Vec<Vec<Corner>>> = outers.iter().map(|(_, r)| vec![r.clone()]).collect();
    for (_, hole) in holes {
        // a point just inside the hole: right of its first edge
        let (a, b) = (hole[0], hole[1]);
        let (dx, dy) = ((b.0 - a.0).signum() as f64, (b.1 - a.1).signum() as f64);
        let (x, y) = (a.0 as f64 + dx * 0.5 + dy * 0.25, a.1 as f64 + dy * 0.5 - dx * 0.25);
        let owner = outers.iter().enumerate().filter(|(_, (_, r))| contains(r, x, y)).min_by_key(|(_, (ar, _))| *ar).map(|(k, _)| k);
        if let Some(k) = owner {
            polys[k].push(hole);
        }
    }
    polys
}

/// Contour lines at `level` metres depth (marching squares, segments joined into lines).
pub fn contours(g: &Grid, level: f64) -> Vec<Vec<[f64; 2]>> {
    let v = |i: usize, j: usize| g.depth(i, j) - level;
    // edge midpoint keys: (i, j, 0 = horizontal edge from (i,j) to (i+1,j), 1 = vertical edge to (i,j+1))
    type Key = (usize, usize, u8);
    let mut segs: Vec<(Key, Key)> = Vec::new();
    for j in 0..g.ny.saturating_sub(1) {
        for i in 0..g.nx.saturating_sub(1) {
            let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1));
            if !(a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite()) {
                continue;
            }
            let idx = (a > 0.0) as u8 | ((b > 0.0) as u8) << 1 | ((c > 0.0) as u8) << 2 | ((d > 0.0) as u8) << 3;
            let (bot, right, top, left): (Key, Key, Key, Key) = ((i, j, 0), (i + 1, j, 1), (i, j + 1, 0), (i, j, 1));
            let mut add = |p: Key, q: Key| segs.push((p, q));
            match idx {
                0 | 15 => {}
                1 | 14 => add(left, bot),
                2 | 13 => add(bot, right),
                3 | 12 => add(left, right),
                4 | 11 => add(right, top),
                6 | 9 => add(bot, top),
                7 | 8 => add(left, top),
                5 => {
                    add(left, top);
                    add(bot, right);
                }
                10 => {
                    add(left, bot);
                    add(right, top);
                }
                _ => {}
            }
        }
    }
    let point = |k: Key| -> [f64; 2] {
        let (i, j, dir) = k;
        let (i2, j2) = if dir == 0 { (i + 1, j) } else { (i, j + 1) };
        let (a, b) = (v(i, j), v(i2, j2));
        let t = if (a - b).abs() > 1e-12 { (a / (a - b)).clamp(0.0, 1.0) } else { 0.5 };
        [g.lon(i as f64 + t * (i2 as f64 - i as f64)), g.lat(j as f64 + t * (j2 as f64 - j as f64))]
    };
    // join segments end to end
    let mut adj: HashMap<Key, Vec<usize>> = HashMap::new();
    for (n, (p, q)) in segs.iter().enumerate() {
        adj.entry(*p).or_default().push(n);
        adj.entry(*q).or_default().push(n);
    }
    let mut used = vec![false; segs.len()];
    let mut lines = Vec::new();
    for s in 0..segs.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let mut chain: std::collections::VecDeque<Key> = [segs[s].0, segs[s].1].into_iter().collect();
        for forward in [true, false] {
            loop {
                let end = if forward { *chain.back().unwrap() } else { *chain.front().unwrap() };
                let next = adj.get(&end).and_then(|l| l.iter().copied().find(|&n| !used[n]));
                let Some(n) = next else { break };
                used[n] = true;
                let other = if segs[n].0 == end { segs[n].1 } else { segs[n].0 };
                if forward {
                    chain.push_back(other);
                } else {
                    chain.push_front(other);
                }
            }
        }
        lines.push(chain.into_iter().map(point).collect::<Vec<_>>());
    }
    lines
}

/// Write an ETOPO area as a GeoPackage of depth bands and contours.
pub fn write_grid(g: &Grid, out: &Path, source: &str) -> Result<std::path::PathBuf, ChartError> {
    let mut w = GpkgWriter::create(out)?;
    w.table("wdepth_etopo", "POLYGON")?;
    w.table("wdcnt_etopo", "LINESTRING")?;
    let e = g.west + (g.nx as f64 - 1.0) * g.dlon;
    let n = g.south + (g.ny as f64 - 1.0) * g.dlat;
    w.meta("area", &format!("{},{},{},{}", g.west, g.south, e, n))?;
    w.meta("source", source)?;
    w.meta("spacing_arcsec", &format!("{}", (g.dlat * 3600.0).round()))?;
    w.meta("format", &FORMAT.to_string())?;
    w.insert("wdepth_etopo", polygon_rows(bands(g)))?;
    let mut lines = Vec::new();
    for l in CONTOUR_LEVELS {
        for line in contours(g, l) {
            if line.len() >= 2 {
                let b = bounds(line.iter().copied());
                lines.push((gpkg_line(&line), b, l));
            }
        }
    }
    w.insert("wdcnt_etopo", lines)?;
    w.finish()
}

/// Version of the depth areas [`write_grid`] writes: 2 = nested traced polygons
/// (files without it have row rectangles that break up when zoomed out).
pub const FORMAT: u32 = 2;

/// The format of an ETOPO GeoPackage (1 for files written before it was recorded).
pub fn file_format(path: &Path) -> u32 {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()
        .and_then(|c| c.query_row("SELECT value FROM racenav_meta WHERE key = 'format'", [], |r| r.get::<_, String>(0)).ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// The area and source written into an ETOPO GeoPackage by [`write_grid`].
pub fn area_info(path: &Path) -> Option<([f64; 4], String, f64)> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let get = |k: &str| c.query_row("SELECT value FROM racenav_meta WHERE key = ?1", [k], |r| r.get::<_, String>(0)).ok();
    let a: Vec<f64> = get("area")?.split(',').filter_map(|x| x.parse().ok()).collect();
    if a.len() != 4 {
        return None;
    }
    Some(([a[0], a[1], a[2], a[3]], get("source").unwrap_or_default(), get("spacing_arcsec").and_then(|s| s.parse().ok()).unwrap_or(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bowl: deepest in the middle (100 m), land at the edges.
    fn bowl() -> Grid {
        let (nx, ny) = (41, 31);
        let mut z = Vec::new();
        for j in 0..ny {
            for i in 0..nx {
                let dx = (i as f64 - 20.0) / 20.0;
                let dy = (j as f64 - 15.0) / 15.0;
                z.push((-(1.0 - (dx * dx + dy * dy)) * 110.0 + 5.0) as f32);
            }
        }
        Grid { west: 173.0, south: -41.5, dlon: 1.0 / 240.0, dlat: 1.0 / 240.0, nx, ny, z }
    }

    fn ring_area(r: &[[f64; 2]]) -> f64 {
        (0..r.len() - 1)
            .map(|k| {
                let (a, b) = (r[k], r[k + 1]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            / 2.0
    }

    #[test]
    fn nested_areas_cover_each_level_exactly() {
        let g = bowl();
        let b = bands(&g);
        let cell = g.dlon * g.dlat;
        for (k, &level) in ETOPO_LEVELS.iter().enumerate() {
            let want = (0..g.ny).flat_map(|j| (0..g.nx).map(move |i| (i, j))).filter(|&(i, j)| band(g.depth(i, j)).is_some_and(|x| x >= k)).count();
            // outer rings minus holes = the cells deeper than the level
            let got: f64 = b
                .iter()
                .filter(|(_, d)| *d == level)
                .map(|(rings, _)| ring_area(&rings[0]).abs() - rings[1..].iter().map(|h| ring_area(h).abs()).sum::<f64>())
                .sum();
            assert!((got / cell - want as f64).abs() < 0.01, "level {level}: {} v {want}", got / cell);
        }
        // the bowl is one round area per level: one polygon each, no slivers
        assert_eq!(b.iter().filter(|(_, d)| *d == 50.0).count(), 1);
        assert!(b.iter().all(|(r, _)| r[0].len() >= 4));
    }

    #[test]
    fn traces_holes_and_blocks() {
        // a ring of water round an island, in a grid bigger than one block
        let (nx, ny) = (300usize, 20usize);
        let mut z = vec![-30.0f32; nx * ny];
        for j in 5..15 {
            for i in 100..110 {
                z[j * nx + i] = 10.0; // island
            }
        }
        let g = Grid { west: 0.0, south: 0.0, dlon: 0.01, dlat: 0.01, nx, ny, z };
        let b: Vec<_> = bands(&g).into_iter().filter(|(_, d)| *d == 20.0).collect();
        assert_eq!(b.len(), 2, "one polygon per block"); // 0..256 and 256..300
        let first = b.iter().find(|(r, _)| r[0].iter().any(|p| p[0] < 0.5)).unwrap();
        assert_eq!(first.0.len(), 2, "outer ring and the island's hole");
        let cell = g.dlon * g.dlat;
        let area = ring_area(&first.0[0]).abs() - ring_area(&first.0[1]).abs();
        assert!((area / cell - (256.0 * 20.0 - 100.0)).abs() < 0.01, "{}", area / cell);
        assert_eq!(first.0[1].len(), 5, "the hole is a square (closed): straight runs merged");
    }

    #[test]
    fn contours_are_closed_rings_round_the_bowl() {
        let g = bowl();
        let c = contours(&g, 50.0);
        assert_eq!(c.len(), 1, "{}", c.len());
        let l = &c[0];
        assert!(l.len() > 20);
        let (a, z) = (l[0], *l.last().unwrap());
        assert!((a[0] - z[0]).abs() < 1e-9 && (a[1] - z[1]).abs() < 1e-9, "closed");
        // the 50 m line lies where the depth is 50 m: radius √(1 − 55/110) ≈ 0.707 of the bowl
        let mid = [173.0 + 20.0 / 240.0, -41.5 + 15.0 / 240.0];
        for p in l {
            let r = (((p[0] - mid[0]) * 240.0 / 20.0).powi(2) + ((p[1] - mid[1]) * 240.0 / 15.0).powi(2)).sqrt();
            assert!((r - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.05, "{r}");
        }
    }

    #[test]
    fn writes_a_geopackage_the_store_draws() {
        let dir = std::env::temp_dir().join(format!("racenav-bathy-{}", std::process::id()));
        let out = write_grid(&bowl(), &dir.join("etopo_test.gpkg"), "ETOPO 2022 15s").unwrap();
        let (a, src, sp) = area_info(&out).unwrap();
        assert_eq!(src, "ETOPO 2022 15s");
        assert_eq!(sp, 15.0);
        assert_eq!(file_format(&out), FORMAT);
        assert!((a[0] - 173.0).abs() < 1e-9);
        let store = crate::ChartStore::open_dirs(std::slice::from_ref(&dir)).unwrap();
        assert!(store.tables.iter().any(|t| t.kind == crate::Kind::WorldDepth));
        assert!(store.tables.iter().any(|t| t.kind == crate::Kind::WorldContour));
        // a z10 tile over the bowl carries depth polygons and contours
        let n = 1u32 << 10;
        let x = ((173.08 + 180.0) / 360.0 * n as f64) as u32;
        let lat = (-41.44f64).to_radians();
        let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n as f64) as u32;
        let t = store.tile(10, x, y);
        let s = String::from_utf8_lossy(&t);
        assert!(s.contains("wdepth") && s.contains("wdcnt"), "{} bytes", t.len());
        let _ = std::fs::remove_dir_all(dir);
    }
}
