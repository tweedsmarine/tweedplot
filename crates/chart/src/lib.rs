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

pub mod bathy;
pub mod clip;
pub mod coast;
pub mod enc;
pub mod geom;
pub mod mvt;
pub mod s57;
pub mod features;
pub use features::{Clearance, FeatureInfo};

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
    /// GSHHG coastline lines (same bands).
    WorldCoast,
    DepthArea,
    Contour,
    Sounding,
    /// Another LINZ hydrographic layer (lights, buoys, rocks, areas...): see [`s57`].
    Feature,
    /// World depth bands where there is no chart: band 0 = Natural Earth (light, small scales), 1 = Natural Earth, 2 = ETOPO area. See [`bathy`].
    WorldDepth,
    /// World depth contours (ETOPO areas).
    WorldContour,
    /// Where a band has chart data (S-57 M_COVR): a finer band's coverage
    /// replaces the coarser bands' features inside it.
    Coverage,
    /// Inland water: lakes and wide rivers (LINZ Topo50 lake and river polygons,
    /// and the hydrographic charts' lake and river areas). Cut out of the Topo50
    /// land, so the charted depths, or plain water, show through.
    Lake,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub file: PathBuf,
    pub table: String,
    pub kind: Kind,
    /// 0 = land, 1 (smallest scale) .. 5 (largest scale).
    pub band: u8,
    pub rows: i64,
    /// For `Kind::Feature`: the S-57 class.
    pub class: Option<&'static s57::FeatureClass>,
    /// Column names (lower case).
    cols: Vec<String>,
    db: usize,
    rtree: String,
    pk: String,
    /// Extent of the table's rows [w, s, e, n], so tiles elsewhere skip it without a query
    /// (thousands of ENC cells each bring their own tables).
    pub bounds: [f64; 4],
}

impl TableInfo {
    /// Whether the table may have rows in `b` (or its copy shifted by 360°).
    pub(crate) fn touches(&self, b: [f64; 4]) -> bool {
        let t = self.bounds;
        wrap_shifts(b).iter().any(|s| t[0] <= b[2] + s && t[2] >= b[0] + s && t[1] <= b[3] && t[3] >= b[1])
    }
}

/// Minimum zoom at which a band is drawn.
pub fn band_minzoom(band: u8) -> u8 {
    match band {
        0 => 6,
        1 => 0,
        2 => 7,
        3 => 9,
        4 => 11,
        5 => 13,
        _ => 14,
    }
}

/// How many zooms before its usual start a band is drawn where no smaller-scale chart
/// covers the place (as an ECDIS shows the best chart available, underscaled): an ENC
/// area with only harbour cells, say, still shows its land and depths zoomed out.
const UNDERSCALE: u8 = 4;

pub struct ChartStore {
    dbs: Vec<Mutex<Connection>>,
    pub tables: Vec<TableInfo>,
    cache: Mutex<lru::LruCache<(u8, u32, u32), Arc<Vec<u8>>>>,
    /// Coverage polygons of each band, loaded on first use: (band, bbox, polygon).
    coverage: std::sync::OnceLock<Vec<(u8, [f64; 4], geo::Polygon<f64>)>>,
    /// Per feature table: its median SCAMIN (when its features usually start to show).
    scamin_median: Mutex<std::collections::HashMap<usize, Option<f64>>>,
}

/// NZ mainland area covered by LINZ Topo50 land: GSHHG polygons wholly inside
/// it are left out where Topo50 is drawn, so coastlines aren't doubled.
const TOPO50_AREA: [f64; 4] = [166.0, -47.6, 179.0, -34.0];
/// The Chatham Islands (also in Topo50), in both longitude frames.
const CHATHAMS: [[f64; 4]; 2] = [[-177.2, -44.7, -175.5, -43.3], [182.8, -44.7, 184.5, -43.3]];

fn inside(b: [f64; 4], area: [f64; 4]) -> bool {
    let i = |a: [f64; 4]| b[0] >= a[0] && b[1] >= a[1] && b[2] <= a[2] && b[3] <= a[3];
    i(area) || (area == TOPO50_AREA && CHATHAMS.iter().any(|c| i(*c)))
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
            // S-57 ENC cells anywhere below, converted once into dir/.enc
            gpkgs.extend(enc::ensure_gpkgs(dir));
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
                let cols: Vec<String> = {
                    let mut st = conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?;
                    let r = st.query_map([], |r| r.get::<_, String>(0))?;
                    r.filter_map(Result::ok).map(|c| c.to_ascii_lowercase()).collect()
                };
                let class = if kind == Kind::Feature { s57::class_for(&table) } else { None };
                let bounds: [f64; 4] = conn
                    .query_row(&format!("SELECT min(minx), min(miny), max(maxx), max(maxy) FROM \"{rtree}\""), [], |r| {
                        Ok([r.get::<_, Option<f64>>(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
                    })
                    .map(|v| if v.iter().all(Option::is_some) { [v[0].unwrap(), v[1].unwrap(), v[2].unwrap(), v[3].unwrap()] } else { [0.0, 0.0, -1.0, -1.0] })
                    .unwrap_or([-360.0, -90.0, 360.0, 90.0]);
                tables.push(TableInfo { file: path.clone(), table, kind, band, rows, class, cols, db, rtree, pk, bounds });
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
            coverage: std::sync::OnceLock::new(),
            scamin_median: Mutex::new(std::collections::HashMap::new()),
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

    /// Forget the rendered tiles (after a drawing setting changes).
    pub fn clear_cache(&self) {
        self.cache.lock().unwrap().clear();
    }

    /// The zoom each band starts at in `bbox`: its usual zoom, or earlier where no
    /// smaller-scale band's coverage reaches (see [`UNDERSCALE`]).
    pub(crate) fn band_starts(&self, bbox: [f64; 4]) -> [u8; 8] {
        let mut out = [0u8; 8];
        let cov = self.coverage();
        for (band, o) in out.iter_mut().enumerate() {
            let m = band_minzoom(band as u8);
            let coarser = band <= 1 || cov.iter().any(|(b, bb, _)| (*b as usize) < band && *b >= 1 && bb[0] <= bbox[2] && bb[2] >= bbox[0] && bb[1] <= bbox[3] && bb[3] >= bbox[1]);
            *o = if coarser { m } else { m.saturating_sub(UNDERSCALE) };
        }
        out
    }

    fn render(&self, z: u8, x: u32, y: u32) -> Vec<u8> {
        let tp = TileProj::new(z, x, y);
        let bbox = tp.lonlat_bbox(BUFFER);
        let mut land = Layer::new("land");
        let mut depare = Layer::new("depare");
        let mut depcnt = Layer::new("depcnt");
        let mut soundg = Layer::new("soundg");
        let mut wdepth = Layer::new("wdepth");
        let mut wdcnt = Layer::new("wdcnt");

        // World depths (under the charts): Natural Earth everywhere (a lighter copy below
        // zoom 4), ETOPO areas from zoom 4.
        let has_lo = self.tables.iter().any(|t| t.kind == Kind::WorldDepth && t.band == 0);
        let mut wd: Vec<&TableInfo> = self
            .tables
            .iter()
            .filter(|t| t.kind == Kind::WorldDepth && if t.band == 0 { z < 4 } else if t.band == 1 { z >= 4 || !has_lo } else { z >= 4 })
            .collect();
        wd.sort_by_key(|t| t.band);
        for t in wd {
            self.query(t, bbox, &["depth"], |vals, g| {
                if let Some(tg) = tp.polygons(&g) {
                    wdepth.add(&tg, &[("d", Value::F64(vals[0].unwrap_or(0.0))), ("src", Value::UInt(t.band as u64))]);
                }
            });
        }
        if z >= 7 {
            for t in self.tables.iter().filter(|t| t.kind == Kind::WorldContour) {
                self.query(t, bbox, &["depth"], |vals, g| {
                    if let Some(tg) = tp.lines(&g) {
                        wdcnt.add(&tg, &[("d", Value::F64(vals[0].unwrap_or(0.0)))]);
                    }
                });
            }
        }

        let starts = self.band_starts(bbox);
        let start = |b: u8| if b == 0 { band_minzoom(0) } else { starts[(b as usize).min(7)] };
        let mut bands: Vec<&TableInfo> = self.tables.iter().filter(|t| z >= start(t.band)).collect();
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
        // Contours and soundings: the finest band with data here, and where it has none
        // (outside its coverage) the next band down, cut to outside the finer coverage, so a
        // tile reaching past a large-scale chart's edge keeps its contours beyond it.
        for kind in [Kind::Contour, Kind::Sounding] {
            let extra = if kind == Kind::Sounding { 1 } else { 0 };
            let mut drawn: Vec<u8> = Vec::new();
            for t in bands.iter().rev().filter(|t| t.kind == kind && z >= start(t.band) + extra) {
                if drawn.contains(&t.band) {
                    continue;
                }
                // a finer band without coverage polygons: no telling where it stops
                if drawn.iter().any(|b| !self.coverage().iter().any(|(cb, _, _)| cb == b)) {
                    break;
                }
                let cover = if drawn.is_empty() { None } else { self.cover_in(&drawn, bbox) };
                let layer = if kind == Kind::Contour { &mut depcnt } else { &mut soundg };
                let before = layer.len();
                let col = if kind == Kind::Contour { "valdco" } else { "depth" };
                self.query(t, bbox, &[col], |vals, g| {
                    let g = match &cover {
                        None => g,
                        Some(c) if kind == Kind::Contour => match features::cut_outside(&g, c) {
                            Some(g) => g,
                            None => return,
                        },
                        Some(c) => {
                            let Geom::Points(pts) = g else { return };
                            let pts: Vec<[f64; 2]> = pts.into_iter().filter(|p| {
                                use geo::Contains;
                                !c.contains(&geo::Point::new(p[0], p[1]))
                            }).collect();
                            if pts.is_empty() {
                                return;
                            }
                            Geom::Points(pts)
                        }
                    };
                    let tg = if kind == Kind::Contour { tp.lines(&drop_cell_edges(g)) } else { tp.points(&g) };
                    if let Some(tg) = tg {
                        let mut props = vec![("band", Value::UInt(t.band as u64))];
                        if let Some(v) = vals[0] {
                            props.push((col, Value::F64(v)));
                        }
                        layer.add(&tg, &props);
                    }
                });
                if layer.len() > before {
                    drawn.push(t.band);
                }
            }
        }
        // Topo50 land, with the lakes and rivers cut out (it has none of its own)
        let mut topo_land: Vec<Vec<[i32; 2]>> = Vec::new();
        for t in bands.iter().filter(|t| t.kind == Kind::Land) {
            self.query(t, bbox, &[], |_, g| {
                if let Some(TileGeom::Rings(r)) = tp.polygons(&g) {
                    topo_land.extend(r);
                }
            });
        }
        if !topo_land.is_empty() {
            let mut water: Vec<Vec<[i32; 2]>> = Vec::new();
            for t in self.tables.iter().filter(|t| t.kind == Kind::Lake) {
                self.query(t, bbox, &[], |_, g| {
                    if let Some(TileGeom::Rings(r)) = tp.polygons(&g) {
                        water.extend(r);
                    }
                });
            }
            let rings = if water.is_empty() { topo_land } else { cut_water(topo_land, water) };
            if !rings.is_empty() {
                land.add(&TileGeom::Rings(rings), &[]);
            }
        }
        // World land at the resolution for this zoom, except inside NZ where Topo50 is drawn.
        let topo = z >= band_minzoom(0) && self.has_topo50();
        let res = coast::res_for_zoom(z, coast::LAND_DETAIL.load(std::sync::atomic::Ordering::Relaxed));
        let mut wcoast = Layer::new("wcoast");
        for t in self.tables.iter().filter(|t| matches!(t.kind, Kind::WorldLand | Kind::WorldCoast)) {
            if coast::RESOLUTIONS[(t.band as usize).clamp(1, 5) - 1] != res {
                continue;
            }
            let is_land = t.kind == Kind::WorldLand;
            self.query(t, bbox, &[], |_, g| {
                if topo && inside(geom_bounds(&g), TOPO50_AREA) {
                    return;
                }
                if is_land {
                    if let Some(tg) = tp.polygons(&g) {
                        // "w": world land, whose outline is drawn from the wcoast lines
                        land.add(&tg, &[("w", Value::UInt(1))]);
                    }
                } else if let Some(tg) = tp.lines(&g) {
                    wcoast.add(&tg, &[]);
                }
            });
        }
        let [far, fln, fpt, lsec] = self.render_features(&tp, z);
        mvt::encode_tile(&[wdepth, wdcnt, depare, depcnt, far, land, wcoast, fln, soundg, lsec, fpt])
    }

    fn query(&self, t: &TableInfo, b: [f64; 4], cols: &[&str], mut f: impl FnMut(&[Option<f64>], Geom)) {
        if !t.touches(b) {
            return;
        }
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
        // read the rows, then let go of the file before the (slower) geometry
        // work, so tiles being drawn at the same time don't queue on the lock
        let mut got: Vec<(Vec<Option<f64>>, Vec<u8>, f64)> = Vec::new();
        {
            let Ok(mut st) = conn.prepare_cached(&sql) else { return };
            for shift in wrap_shifts(b) {
                let Ok(mut rows) = st.query(rusqlite::params![b[0] + shift, b[2] + shift, b[1], b[3]]) else { continue };
                while let Ok(Some(r)) = rows.next() {
                    let vals: Vec<Option<f64>> = (0..cols.len()).map(|i| r.get::<_, Option<f64>>(i).ok().flatten()).collect();
                    let Ok(blob) = r.get_ref(cols.len()).and_then(|v| v.as_blob().map_err(Into::into)) else { continue };
                    got.push((vals, blob.to_vec(), shift));
                }
            }
        }
        drop(conn);
        for (vals, blob, shift) in got {
            if let Some(mut g) = parse_gpkg(&blob) {
                shift_geom(&mut g, -shift);
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

    /// Every lake polygon touching `bbox` (inland water), and the rivers too if `rivers`.
    pub fn for_each_lake(&self, bbox: [f64; 4], rivers: bool, mut f: impl FnMut(&[geom::Ring])) {
        for t in self.tables.iter().filter(|t| t.kind == Kind::Lake && (rivers || !t.table.to_ascii_lowercase().contains("river"))) {
            self.query(t, bbox, &[], |_, g| {
                if let Geom::Polygons(polys) = g {
                    for p in &polys {
                        f(p);
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
        self.tables.iter().any(|t| t.kind == Kind::Land && t.band == 0)
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

/// Longitude shifts to query for a bbox near the date line. LINZ stores the
/// Chathams and other places east of 180° as 180..190, GSHHG as -180..-170, while
/// map tiles always ask in -180..180: so a bbox reaching west of -120 is also
/// looked up 360° further east, and one reaching past 180 also 360° further
/// west; what is found is shifted back into the bbox's frame.
pub(crate) fn wrap_shifts(b: [f64; 4]) -> Vec<f64> {
    let mut v = vec![0.0];
    if b[0] < -120.0 {
        v.push(360.0);
    }
    if b[2] > 180.0 {
        v.push(-360.0);
    }
    v
}

/// Add `dlon` to every longitude of a geometry.
pub(crate) fn shift_geom(g: &mut Geom, dlon: f64) {
    if dlon == 0.0 {
        return;
    }
    match g {
        Geom::Points(v) => v.iter_mut().for_each(|p| p[0] += dlon),
        Geom::Lines(v) => v.iter_mut().flatten().for_each(|p| p[0] += dlon),
        Geom::Polygons(v) => v.iter_mut().flatten().flatten().for_each(|p| p[0] += dlon),
    }
}

pub(crate) fn geom_bounds(g: &Geom) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut add = |p: &[f64; 2]| b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    match g {
        Geom::Points(v) => v.iter().for_each(&mut add),
        Geom::Lines(v) => v.iter().flatten().for_each(&mut add),
        Geom::Polygons(v) => v.iter().flatten().flatten().for_each(&mut add),
    }
    b
}

/// LINZ contours are cut at chart cell edges, and some carry a piece running
/// along the edge (a straight east–west or north–south line, often a degree
/// long, drawn across the chart). Split lines at such pieces and drop them:
/// runs of (nearly, 2e-4°) constant latitude or longitude longer than 0.02°,
/// and any segment over a degree long (one in the 1:1.5M band runs 21° along
/// 36.2365°S).
pub fn drop_cell_edges(g: Geom) -> Geom {
    let Geom::Lines(lines) = g else { return g };
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let n = l.len();
        if n < 2 {
            continue;
        }
        // mark segments that belong to a long run of constant latitude or longitude
        let mut edge = vec![false; n - 1];
        for axis in 0..2 {
            let other = 1 - axis;
            let mut i = 0;
            while i < n - 1 {
                let mut j = i;
                while j < n - 1 && (l[j + 1][other] - l[i][other]).abs() < 2e-4 {
                    j += 1;
                }
                if j > i {
                    let (lo, hi) = l[i..=j].iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p[axis]), hi.max(p[axis])));
                    if hi - lo > 0.02 {
                        for e in edge.iter_mut().take(j).skip(i) {
                            *e = true;
                        }
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
        // and any single segment over a degree long (no contour runs 60 NM straight)
        for k in 0..n - 1 {
            if (l[k + 1][0] - l[k][0]).abs().max((l[k + 1][1] - l[k][1]).abs()) > 1.0 {
                edge[k] = true;
            }
        }
        let mut cur: Vec<[f64; 2]> = Vec::new();
        for k in 0..n - 1 {
            if edge[k] {
                if cur.len() >= 2 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                continue;
            }
            if cur.is_empty() {
                cur.push(l[k]);
            }
            cur.push(l[k + 1]);
        }
        if cur.len() >= 2 {
            out.push(cur);
        }
    }
    Geom::Lines(out)
}

/// Land rings minus water rings, in tile units (exteriors positive, holes
/// negative, as `TileProj::polygons` gives them). Non-zero filling, so
/// overlapping lakes (Topo50 and a chart's own) count once.
fn cut_water(land: Vec<Vec<[i32; 2]>>, water: Vec<Vec<[i32; 2]>>) -> Vec<Vec<[i32; 2]>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let f = |v: Vec<Vec<[i32; 2]>>| -> Vec<Vec<[f64; 2]>> { v.into_iter().map(|r| r.into_iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect() };
    let (subj, clip) = (f(land), f(water));
    let shapes = subj.overlay(&clip, OverlayRule::Difference, FillRule::NonZero);
    let mut out = Vec::new();
    for shape in shapes {
        for (k, c) in shape.into_iter().enumerate() {
            let mut q: Vec<[i32; 2]> = c.iter().map(|p| [p[0].round() as i32, p[1].round() as i32]).collect();
            q.dedup();
            if q.len() > 1 && q.first() == q.last() {
                q.pop();
            }
            if q.len() < 3 {
                continue;
            }
            let a = signed_area(&q);
            if a == 0 {
                continue;
            }
            // the outer contour positive, holes negative
            if (k == 0) != (a > 0) {
                q.reverse();
            }
            out.push(q);
        }
    }
    out
}

fn classify(table: &str) -> Option<(Kind, u8)> {
    let t = table.to_ascii_lowercase();
    match t.as_str() {
        "wdepth_nelo" => return Some((Kind::WorldDepth, 0)),
        "wdepth_ne" => return Some((Kind::WorldDepth, 1)),
        "wdepth_etopo" => return Some((Kind::WorldDepth, 2)),
        "wdcnt_etopo" => return Some((Kind::WorldContour, 1)),
        _ => {}
    }
    if let Some(r) = t.strip_prefix("gshhs_") {
        let (res, kind) = match r.strip_suffix("_ln") {
            Some(x) => (x, Kind::WorldCoast),
            None => (r, Kind::WorldLand),
        };
        let i = coast::RESOLUTIONS.iter().position(|c| res.starts_with(*c) && res.len() == 1)?;
        return Some((kind, i as u8 + 1));
    }
    // S-57 ENC cells converted by `enc`: "<layer>_enc_b<band>"
    if let Some(i) = t.rfind("_enc_b") {
        let band: u8 = t[i + 6..].parse().ok()?;
        let kind = if t.starts_with("land_area_polygon") {
            Kind::Land
        } else if t.starts_with("lake_polygon") || t.starts_with("river_polygon") {
            Kind::Lake
        } else if t.starts_with("coverage_polygon") {
            Kind::Coverage
        } else if t.starts_with("depth_area") {
            Kind::DepthArea
        } else if t.starts_with("depth_contour") {
            Kind::Contour
        } else if t.starts_with("sounding") {
            Kind::Sounding
        } else if s57::class_for(&t).is_some() {
            Kind::Feature
        } else {
            return None;
        };
        return Some((kind, band));
    }
    if t.contains("coastlines_and_islands") {
        return Some((Kind::Land, 0));
    }
    // Topo50 lakes and rivers: "nz_lake_polygons_topo_150k", "nz_river_polygons_topo_150k"
    if (t.contains("lake_polygons") || t.contains("river_polygons")) && t.contains("topo") {
        return Some((Kind::Lake, 0));
    }
    let kind = if t.starts_with("lake_polygon") || t.starts_with("river_polygon") {
        Kind::Lake
    } else if t.contains("coverage_polygon") {
        Kind::Coverage
    } else if t.contains("depth_area") {
        Kind::DepthArea
    } else if t.contains("depth_contour") {
        Kind::Contour
    } else if t.contains("sounding") {
        Kind::Sounding
    } else if s57::class_for(&t).is_some() {
        Kind::Feature
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
        // Which zip each unpacked file came from: a sidecar "<name>.src".
        let src_file = out_dir.join(format!("{}.src", name.to_string_lossy()));
        let zip_name = zip_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let from = std::fs::read_to_string(&src_file).ok();
        // Unpack when missing; when this same zip is newer than the copy (a fresh download of
        // it); or when another zip holds the same layer with more in it. LINZ exports of
        // several layers can be cut to an area, and a small one must not replace the whole
        // country's layer from its own download (or win just by being newer).
        let zip_time = std::fs::metadata(zip_path).and_then(|m| m.modified()).ok();
        let stale = match std::fs::metadata(&dest) {
            Err(_) => true,
            Ok(m) if from.as_deref() == Some(zip_name.as_str()) => match (zip_time, m.modified().ok()) {
                (Some(z), Some(d)) => z > d,
                _ => m.len() != f.size(),
            },
            // unknown origin (unpacked before the sidecar) or another zip: the bigger wins
            Ok(m) => f.size() > m.len(),
        };
        if stale {
            std::fs::create_dir_all(out_dir)?;
            let tmp = dest.with_extension("gpkg.part");
            let mut w = std::fs::File::create(&tmp)?;
            std::io::copy(&mut f, &mut w)?;
            drop(w);
            std::fs::rename(&tmp, &dest)?;
            let _ = std::fs::write(&src_file, &zip_name);
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
mod edge_tests {
    use super::*;

    #[test]
    fn drops_contour_pieces_along_cell_edges() {
        let g = Geom::Lines(vec![
            vec![[172.9, -36.2365], [172.91, -36.2365], [172.92, -36.2365], [174.4, -36.2365]], // all edge, in short steps: gone
            vec![[174.0, -36.0], [174.01, -36.1], [174.01, -36.3], [174.02, -36.31]],         // a north–south edge in the middle: split
            vec![[174.0, -36.0], [174.01, -36.0], [174.05, -36.02]],                          // short flat piece: kept
            vec![[180.0, -36.2367198], [158.6, -36.2364045]],                                  // the real one: gone
        ]);
        let Geom::Lines(out) = drop_cell_edges(g) else { panic!() };
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(out[0], vec![[174.0, -36.0], [174.01, -36.1]]);
        assert_eq!(out[1], vec![[174.01, -36.3], [174.02, -36.31]]);
        assert_eq!(out[2].len(), 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_export_does_not_replace_a_whole_layer() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("racenav-unzip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, bytes: usize| {
            let p = dir.join(name);
            let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            z.start_file("light-points-hydro-122k-190k.gpkg", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(&vec![7u8; bytes]).unwrap();
            z.finish().unwrap();
            p
        };
        let out = dir.join(".unzipped");
        let size = || std::fs::metadata(out.join("light-points-hydro-122k-190k.gpkg")).unwrap().len();
        let big = mk("lds-oceania-279layers-GPKG.zip", 5000);
        unzip_gpkgs(&big, &out).unwrap();
        // a later, smaller export holding the same layer: kept out, in either order
        std::thread::sleep(std::time::Duration::from_millis(20));
        let small = mk("lds-oceania-50layers-GPKG.zip", 300);
        unzip_gpkgs(&small, &out).unwrap();
        assert_eq!(size(), 5000);
        unzip_gpkgs(&big, &out).unwrap();
        assert_eq!(size(), 5000);
        // a copy unpacked by an older build (no record of its zip) gives way to a bigger one
        std::fs::remove_file(out.join("light-points-hydro-122k-190k.gpkg.src")).unwrap();
        std::fs::write(out.join("light-points-hydro-122k-190k.gpkg"), vec![1u8; 300]).unwrap();
        unzip_gpkgs(&big, &out).unwrap();
        assert_eq!(size(), 5000);
        // a fresh download of the same zip replaces its own copy, even if smaller
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let big2 = mk("lds-oceania-279layers-GPKG.zip", 4800);
        unzip_gpkgs(&big2, &out).unwrap();
        assert_eq!(size(), 4800);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lakes_are_cut_out_of_land() {
        let sq = |x0: i32, y0: i32, x1: i32, y1: i32| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        let pos = |mut r: Vec<[i32; 2]>| { if signed_area(&r) < 0 { r.reverse(); } r };
        let land = vec![pos(sq(0, 0, 1000, 1000))];
        // two overlapping lakes (Topo50 and a chart's) and an island in the first
        let mut island = pos(sq(300, 300, 350, 350));
        island.reverse();
        let water = vec![pos(sq(200, 200, 600, 600)), island, pos(sq(500, 500, 700, 700))];
        let out = cut_water(land, water);
        let area: i64 = out.iter().map(|r| signed_area(r)).sum::<i64>() / 2;
        // land 1e6 − (160000 + 40000 − 10000 overlap) + island 2500
        assert_eq!(area, 1_000_000 - 190_000 + 2_500, "{out:?}");
        assert!(out.iter().filter(|r| signed_area(r) > 0).count() == 2); // the land and the island
    }

    #[test]
    fn classifies_linz_tables() {
        assert_eq!(classify("depth_area_polygon_hydro_14k_122k"), Some((Kind::DepthArea, 5)));
        assert_eq!(classify("depth_contour_polyline_hydro_1350k_11500k"), Some((Kind::Contour, 2)));
        assert_eq!(classify("sounding_points_hydro_115mil_and_smaller"), Some((Kind::Sounding, 1)));
        assert_eq!(classify("nz_coastlines_and_islands_polygons_topo_150k"), Some((Kind::Land, 0)));
        assert_eq!(classify("nz_lake_polygons_topo_150k"), Some((Kind::Lake, 0)));
        assert_eq!(classify("nz_river_polygons_topo_150k"), Some((Kind::Lake, 0)));
        assert_eq!(classify("lake_polygon_hydro_14k_122k"), Some((Kind::Lake, 5)));
        assert_eq!(classify("river_polygon_hydro_190k_1350k"), Some((Kind::Lake, 3)));
        assert_eq!(classify("something_else"), None);
        assert_eq!(classify("light_points_hydro_190k_1350k"), Some((Kind::Feature, 3)));
        assert_eq!(classify("building_single_points_hydro_190k_1350k"), None);
    }

    #[test]
    fn date_line_shifts() {
        assert_eq!(wrap_shifts([170.0, -45.0, 175.0, -40.0]), vec![0.0]);
        assert_eq!(wrap_shifts([-177.0, -45.0, -175.0, -43.0]), vec![0.0, 360.0]);
        assert_eq!(wrap_shifts([179.0, -45.0, 180.1, -43.0]), vec![0.0, -360.0]);
        let mut g = Geom::Points(vec![[183.5, -44.0]]);
        shift_geom(&mut g, -360.0);
        assert!(matches!(g, Geom::Points(ref p) if (p[0][0] + 176.5).abs() < 1e-9));
        assert!(inside([-176.9, -44.4, -176.1, -43.6], TOPO50_AREA));
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
