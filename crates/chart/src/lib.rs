//! Chart data for racenav.
//!
//! Opens a folder of LINZ GeoPackages (zipped or not) and serves them as
//! Mapbox Vector Tiles, so MapLibre can draw them with no internet:
//!
//! | MVT layer | Source | Properties |
//! | --- | --- | --- |
//! | `land`   | NZ Coastlines and Islands Polygons (Topo 1:50k) | – |
//! | `depare` | Depth area polygons (Hydro), all scale bands | `drval1`, `drval2` (m), `band` |
//! | `depcnt` | Depth contour polylines (Hydro) | `valdco` (m), `band` |
//! | `soundg` | Sounding points (Hydro) | `depth` (m), `band` |
//!
//! `band` runs from 1 (1:1.5M and smaller) to 5 (1:4k–1:22k). Each band only
//! appears from its own minimum zoom, coarse bands are drawn first, and
//! soundings and contours come from the finest band that has data in a tile.
//!
//! World land outside the LINZ coverage comes from GSHHG (see [`coast`]),
//! drawn into the same `land` layer at a resolution to suit the zoom.
//!
//! LINZ data: CC BY 4.0. GSHHG: LGPL. Not for navigation.

pub mod clip;
pub mod coast;
pub mod geom;
pub mod mvt;

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clip::{clip_line, clip_ring, quantise, signed_area, simplify, Rect};
use geom::{parse_gpkg, Geom};
use mvt::{Layer, TileGeom, Value, EXTENT};
use rusqlite::{Connection, OpenFlags};

#[derive(Debug, thiserror::Error)]
pub enum ChartError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Land,
    /// GSHHG world land; `band` 1..5 = crude, low, intermediate, high, full.
    WorldLand,
    DepthArea,
    Contour,
    Sounding,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub file: PathBuf,
    pub table: String,
    pub kind: Kind,
    /// 0 = land, 1 (smallest scale) .. 5 (largest scale).
    pub band: u8,
    pub rows: i64,
    db: usize,
    rtree: String,
    pk: String,
}

/// Minimum zoom at which a band is drawn.
pub fn band_minzoom(band: u8) -> u8 {
    match band {
        0 => 6,
        1 => 0,
        2 => 7,
        3 => 9,
        4 => 11,
        _ => 13,
    }
}

pub struct ChartStore {
    dbs: Vec<Mutex<Connection>>,
    pub tables: Vec<TableInfo>,
    cache: Mutex<lru::LruCache<(u8, u32, u32), Arc<Vec<u8>>>>,
}

/// NZ mainland area covered by LINZ Topo50 land: GSHHG polygons wholly inside
/// it are left out where Topo50 is drawn, so coastlines aren't doubled.
const TOPO50_AREA: [f64; 4] = [166.0, -47.6, 179.0, -34.0];

fn inside(b: [f64; 4], area: [f64; 4]) -> bool {
    b[0] >= area[0] && b[1] >= area[1] && b[2] <= area[2] && b[3] <= area[3]
}

impl ChartStore {
    /// Open every LINZ GeoPackage in `dir`. Zip files are unpacked once into `dir/.unzipped`.
    pub fn open_dir(dir: impl AsRef<Path>) -> Result<Self, ChartError> {
        Self::open_dirs(&[dir.as_ref().to_path_buf()])
    }

    /// Open the chart data in several folders (LINZ GeoPackages and zips, GSHHG
    /// shapefiles). Missing folders are skipped.
    pub fn open_dirs(dirs: &[PathBuf]) -> Result<Self, ChartError> {
        let mut gpkgs: Vec<PathBuf> = Vec::new();
        for dir in dirs.iter().filter(|d| d.is_dir()) {
            let unz = dir.join(".unzipped");
            for e in std::fs::read_dir(dir)? {
                let p = e?.path();
                let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                match p.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref() {
                    Some("gpkg") => gpkgs.push(p),
                    Some("zip") if !name.starts_with("gshhg") => gpkgs.extend(unzip_gpkgs(&p, &unz)?),
                    _ => {}
                }
            }
            if let Some(g) = coast::ensure_gpkg(dir, &unz)? {
                gpkgs.push(g);
            }
            if unz.is_dir() {
                for e in std::fs::read_dir(&unz)? {
                    let p = e?.path();
                    if p.extension().is_some_and(|x| x == "gpkg") && !gpkgs.contains(&p) {
                        gpkgs.push(p);
                    }
                }
            }
        }
        gpkgs.sort();
        gpkgs.dedup_by(|a, b| a.file_name() == b.file_name());

        let mut dbs = Vec::new();
        let mut tables = Vec::new();
        for path in gpkgs {
            let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            let found: Vec<(String, String)> = {
                let mut st = conn.prepare(
                    "SELECT c.table_name, g.column_name FROM gpkg_contents c
                     JOIN gpkg_geometry_columns g ON g.table_name = c.table_name
                     WHERE c.data_type = 'features'",
                )?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            let db = dbs.len();
            let mut any = false;
            for (table, gcol) in found {
                let Some((kind, band)) = classify(&table) else { continue };
                let rtree = format!("rtree_{table}_{gcol}");
                let has_rtree: bool = conn
                    .query_row("SELECT count(*) FROM sqlite_master WHERE name = ?1", [&rtree], |r| r.get::<_, i64>(0))
                    .map(|n| n > 0)?;
                if !has_rtree {
                    continue;
                }
                let pk: String = conn
                    .query_row(&format!("SELECT name FROM pragma_table_info('{table}') WHERE pk = 1"), [], |r| r.get(0))
                    .unwrap_or_else(|_| "fid".into());
                let rows: i64 = conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| r.get(0))?;
                tables.push(TableInfo { file: path.clone(), table, kind, band, rows, db, rtree, pk });
                any = true;
            }
            if any {
                dbs.push(Mutex::new(conn));
            }
        }
        Ok(Self {
            dbs,
            tables,
            cache: Mutex::new(lru::LruCache::new(NonZeroUsize::new(2048).unwrap())),
        })
    }

    /// Encoded MVT tile (possibly empty).
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Arc<Vec<u8>> {
        if let Some(t) = self.cache.lock().unwrap().get(&(z, x, y)) {
            return t.clone();
        }
        let t = Arc::new(self.render(z, x, y));
        self.cache.lock().unwrap().put((z, x, y), t.clone());
        t
    }

    fn render(&self, z: u8, x: u32, y: u32) -> Vec<u8> {
        let tp = TileProj::new(z, x, y);
        let bbox = tp.lonlat_bbox(BUFFER);
        let mut land = Layer::new("land");
        let mut depare = Layer::new("depare");
        let mut depcnt = Layer::new("depcnt");
        let mut soundg = Layer::new("soundg");

        let mut bands: Vec<&TableInfo> = self.tables.iter().filter(|t| z >= band_minzoom(t.band)).collect();
        bands.sort_by_key(|t| t.band);

        // Depth areas: every visible band, coarse first.
        for t in bands.iter().filter(|t| t.kind == Kind::DepthArea) {
            self.query(t, bbox, &["drval1", "drval2"], |vals, g| {
                if let Some(tg) = tp.polygons(&g) {
                    let mut props = vec![("band", Value::UInt(t.band as u64))];
                    for (k, v) in [("drval1", vals[0]), ("drval2", vals[1])] {
                        if let Some(v) = v {
                            props.push((k, Value::F64(v)));
                        }
                    }
                    depare.add(&tg, &props);
                }
            });
        }
        // Contours and soundings: the finest band with data here.
        for kind in [Kind::Contour, Kind::Sounding] {
            let extra = if kind == Kind::Sounding { 1 } else { 0 };
            for t in bands.iter().rev().filter(|t| t.kind == kind && z >= band_minzoom(t.band) + extra) {
                let layer = if kind == Kind::Contour { &mut depcnt } else { &mut soundg };
                let before = layer.len();
                let col = if kind == Kind::Contour { "valdco" } else { "depth" };
                self.query(t, bbox, &[col], |vals, g| {
                    let tg = if kind == Kind::Contour { tp.lines(&g) } else { tp.points(&g) };
                    if let Some(tg) = tg {
                        let mut props = vec![("band", Value::UInt(t.band as u64))];
                        if let Some(v) = vals[0] {
                            props.push((col, Value::F64(v)));
                        }
                        layer.add(&tg, &props);
                    }
                });
                if layer.len() > before {
                    break;
                }
            }
        }
        for t in bands.iter().filter(|t| t.kind == Kind::Land) {
            self.query(t, bbox, &[], |_, g| {
                if let Some(tg) = tp.polygons(&g) {
                    land.add(&tg, &[]);
                }
            });
        }
        // World land at the resolution for this zoom, except inside NZ where Topo50 is drawn.
        let topo = z >= band_minzoom(0) && self.has_topo50();
        for t in self.tables.iter().filter(|t| t.kind == Kind::WorldLand) {
            let (lo, hi) = coast::zoom_range(coast::RESOLUTIONS[(t.band as usize).clamp(1, 5) - 1]);
            if z < lo || z > hi {
                continue;
            }
            self.query(t, bbox, &[], |_, g| {
                if topo && inside(geom_bounds(&g), TOPO50_AREA) {
                    return;
                }
                if let Some(tg) = tp.polygons(&g) {
                    land.add(&tg, &[]);
                }
            });
        }
        mvt::encode_tile(&[depare, depcnt, land, soundg])
    }

    fn query(&self, t: &TableInfo, b: [f64; 4], cols: &[&str], mut f: impl FnMut(&[Option<f64>], Geom)) {
        let conn = self.dbs[t.db].lock().unwrap();
        let gcol = t.rtree.trim_start_matches(&format!("rtree_{}_", t.table)).to_string();
        let sel: String = cols.iter().map(|c| format!("t.\"{c}\", ")).collect();
        let sql = format!(
            "SELECT {sel}t.\"{gcol}\" FROM \"{tbl}\" t JOIN \"{rt}\" r ON t.\"{pk}\" = r.id
             WHERE r.maxx >= ?1 AND r.minx <= ?2 AND r.maxy >= ?3 AND r.miny <= ?4",
            tbl = t.table,
            rt = t.rtree,
            pk = t.pk
        );
        let Ok(mut st) = conn.prepare_cached(&sql) else { return };
        let Ok(mut rows) = st.query(rusqlite::params![b[0], b[2], b[1], b[3]]) else { return };
        let mut vals = vec![None; cols.len()];
        while let Ok(Some(r)) = rows.next() {
            for (i, v) in vals.iter_mut().enumerate() {
                *v = r.get::<_, Option<f64>>(i).ok().flatten();
            }
            let Ok(blob) = r.get_ref(cols.len()).and_then(|v| v.as_blob().map_err(Into::into)) else { continue };
            if let Some(g) = parse_gpkg(blob) {
                f(&vals, g);
            }
        }
    }

    /// Every depth-area polygon touching `bbox` [w, s, e, n], coarse bands first:
    /// `f(band, drval1, rings)`. For building routing obstacle maps.
    pub fn for_each_depth_area(&self, bbox: [f64; 4], mut f: impl FnMut(u8, Option<f64>, &[geom::Ring])) {
        let mut tables: Vec<&TableInfo> = self.tables.iter().filter(|t| t.kind == Kind::DepthArea).collect();
        tables.sort_by_key(|t| t.band);
        for t in tables {
            self.query(t, bbox, &["drval1"], |vals, g| {
                if let Geom::Polygons(polys) = g {
                    for p in &polys {
                        f(t.band, vals[0], p);
                    }
                }
            });
        }
    }

    /// Every Topo50 land polygon touching `bbox`.
    pub fn for_each_land(&self, bbox: [f64; 4], mut f: impl FnMut(&[geom::Ring])) {
        for t in self.tables.iter().filter(|t| t.kind == Kind::Land) {
            self.query(t, bbox, &[], |_, g| {
                if let Geom::Polygons(polys) = g {
                    for p in &polys {
                        f(p);
                    }
                }
            });
        }
    }

    fn has_topo50(&self) -> bool {
        self.tables.iter().any(|t| t.kind == Kind::Land)
    }

    /// True if GSHHG world land is available.
    pub fn has_world_land(&self) -> bool {
        self.tables.iter().any(|t| t.kind == Kind::WorldLand)
    }

    /// GSHHG land polygons touching `bbox`, at a resolution to suit raster cells
    /// of `cell_m` metres; NZ polygons are left to Topo50 when it is present.
    pub fn for_each_world_land(&self, bbox: [f64; 4], cell_m: f64, mut f: impl FnMut(&[geom::Ring])) {
        let res = coast::res_for_cell(cell_m);
        let band = coast::RESOLUTIONS.iter().position(|&r| r == res).unwrap() as u8 + 1;
        let topo = self.has_topo50();
        for t in self.tables.iter().filter(|t| t.kind == Kind::WorldLand && t.band == band) {
            self.query(t, bbox, &[], |_, g| {
                if topo && inside(geom_bounds(&g), TOPO50_AREA) {
                    return;
                }
                if let Geom::Polygons(polys) = g {
                    for p in &polys {
                        f(p);
                    }
                }
            });
        }
    }

    /// Depth-area polygons whose shallowest depth (`drval1`) is less than `depth`,
    /// in lon/lat, from the finest band covering each area. For routing.
    pub fn shallow_areas(&self, bbox: [f64; 4], depth: f64, band: u8) -> Vec<Vec<geom::Ring>> {
        let mut out = Vec::new();
        for t in self.tables.iter().filter(|t| t.kind == Kind::DepthArea && t.band == band) {
            self.query(t, bbox, &["drval1"], |vals, g| {
                if vals[0].is_some_and(|d| d < depth) {
                    if let Geom::Polygons(p) = g {
                        out.extend(p);
                    }
                }
            });
        }
        out
    }
}

fn geom_bounds(g: &Geom) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut add = |p: &[f64; 2]| b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    match g {
        Geom::Points(v) => v.iter().for_each(&mut add),
        Geom::Lines(v) => v.iter().flatten().for_each(&mut add),
        Geom::Polygons(v) => v.iter().flatten().flatten().for_each(&mut add),
    }
    b
}

fn classify(table: &str) -> Option<(Kind, u8)> {
    let t = table.to_ascii_lowercase();
    if let Some(r) = t.strip_prefix("gshhs_") {
        let i = coast::RESOLUTIONS.iter().position(|c| r.starts_with(*c) && r.len() == 1)?;
        return Some((Kind::WorldLand, i as u8 + 1));
    }
    let kind = if t.contains("coastlines_and_islands") {
        return Some((Kind::Land, 0));
    } else if t.contains("depth_area") {
        Kind::DepthArea
    } else if t.contains("depth_contour") {
        Kind::Contour
    } else if t.contains("sounding") {
        Kind::Sounding
    } else {
        return None;
    };
    let band = if t.contains("14k_122k") {
        5
    } else if t.contains("122k_190k") {
        4
    } else if t.contains("190k_1350k") {
        3
    } else if t.contains("1350k_11500k") {
        2
    } else if t.contains("115mil") {
        1
    } else {
        return None;
    };
    Some((kind, band))
}

fn unzip_gpkgs(zip_path: &Path, out_dir: &Path) -> Result<Vec<PathBuf>, ChartError> {
    let mut za = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    let mut out = Vec::new();
    for i in 0..za.len() {
        let mut f = za.by_index(i)?;
        let Some(name) = f.enclosed_name().and_then(|p| p.file_name().map(|n| n.to_owned())) else { continue };
        if !name.to_string_lossy().to_ascii_lowercase().ends_with(".gpkg") {
            continue;
        }
        let dest = out_dir.join(&name);
        if !dest.exists() || std::fs::metadata(&dest)?.len() != f.size() {
            std::fs::create_dir_all(out_dir)?;
            let tmp = dest.with_extension("gpkg.part");
            let mut w = std::fs::File::create(&tmp)?;
            std::io::copy(&mut f, &mut w)?;
            drop(w);
            std::fs::rename(&tmp, &dest)?;
        }
        out.push(dest);
    }
    Ok(out)
}

const BUFFER: f64 = 64.0;

/// Web Mercator projection into one tile's pixel space.
struct TileProj {
    /// Simplification tolerance in tile units (4096 per tile = about 8 per screen pixel).
    tol: f64,
    n: f64,
    x: f64,
    y: f64,
    rect: Rect,
}

impl TileProj {
    fn new(z: u8, x: u32, y: u32) -> Self {
        let e = EXTENT as f64;
        Self {
            tol: if z >= 15 { 1.0 } else { 3.0 },
            n: 2f64.powi(z as i32),
            x: x as f64,
            y: y as f64,
            rect: Rect { x0: -BUFFER, y0: -BUFFER, x1: e + BUFFER, y1: e + BUFFER },
        }
    }

    fn px(&self, p: &[f64; 2]) -> [f64; 2] {
        let e = EXTENT as f64;
        let lat = p[1].clamp(-85.05, 85.05).to_radians();
        let mx = (p[0] + 180.0) / 360.0 * self.n;
        let my = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * self.n;
        [(mx - self.x) * e, (my - self.y) * e]
    }

    /// [west, south, east, north] of the tile plus a pixel buffer.
    fn lonlat_bbox(&self, buf: f64) -> [f64; 4] {
        let f = buf / EXTENT as f64;
        let lon = |tx: f64| tx / self.n * 360.0 - 180.0;
        let lat = |ty: f64| {
            let k = std::f64::consts::PI * (1.0 - 2.0 * ty / self.n);
            k.sinh().atan().to_degrees()
        };
        [lon(self.x - f), lat(self.y + 1.0 + f), lon(self.x + 1.0 + f), lat(self.y - f)]
    }

    fn polygons(&self, g: &Geom) -> Option<TileGeom> {
        let Geom::Polygons(polys) = g else { return None };
        let mut rings = Vec::new();
        for poly in polys {
            let mut first = true;
            for ring in poly {
                let pts: Vec<[f64; 2]> = ring.iter().map(|p| self.px(p)).collect();
                let mut q = quantise(&clip_ring(&pts, self.rect));
                if q.len() > 2 {
                    q.push(q[0]);
                    q = simplify(&q, self.tol);
                }
                if q.len() > 1 && q.first() == q.last() {
                    q.pop();
                }
                if q.len() < 3 {
                    if first {
                        break; // exterior gone: drop its holes too
                    }
                    continue;
                }
                let a = signed_area(&q);
                if a == 0 {
                    if first {
                        break;
                    }
                    continue;
                }
                if (first && a < 0) || (!first && a > 0) {
                    q.reverse();
                }
                rings.push(q);
                first = false;
            }
        }
        (!rings.is_empty()).then_some(TileGeom::Rings(rings))
    }

    fn lines(&self, g: &Geom) -> Option<TileGeom> {
        let Geom::Lines(lines) = g else { return None };
        let mut out = Vec::new();
        for l in lines {
            let pts: Vec<[f64; 2]> = l.iter().map(|p| self.px(p)).collect();
            for piece in clip_line(&pts, self.rect) {
                let q = simplify(&quantise(&piece), self.tol);
                if q.len() >= 2 {
                    out.push(q);
                }
            }
        }
        (!out.is_empty()).then_some(TileGeom::Lines(out))
    }

    fn points(&self, g: &Geom) -> Option<TileGeom> {
        let Geom::Points(pts) = g else { return None };
        let e = EXTENT as f64;
        let out: Vec<[i32; 2]> = pts
            .iter()
            .map(|p| self.px(p))
            .filter(|p| p[0] >= 0.0 && p[0] < e && p[1] >= 0.0 && p[1] < e)
            .map(|p| [p[0].round() as i32, p[1].round() as i32])
            .collect();
        (!out.is_empty()).then_some(TileGeom::Points(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_linz_tables() {
        assert_eq!(classify("depth_area_polygon_hydro_14k_122k"), Some((Kind::DepthArea, 5)));
        assert_eq!(classify("depth_contour_polyline_hydro_1350k_11500k"), Some((Kind::Contour, 2)));
        assert_eq!(classify("sounding_points_hydro_115mil_and_smaller"), Some((Kind::Sounding, 1)));
        assert_eq!(classify("nz_coastlines_and_islands_polygons_topo_150k"), Some((Kind::Land, 0)));
        assert_eq!(classify("something_else"), None);
    }

    #[test]
    fn tile_bbox_round_trip() {
        // Tile containing Nelson at z12.
        let tp = TileProj::new(12, 4019, 2525);
        let b = tp.lonlat_bbox(0.0);
        let sw = tp.px(&[b[0], b[1]]);
        let ne = tp.px(&[b[2], b[3]]);
        assert!(sw[0].abs() < 1e-6 && (sw[1] - 4096.0).abs() < 1e-6);
        assert!((ne[0] - 4096.0).abs() < 1e-6 && ne[1].abs() < 1e-6);
    }
}
