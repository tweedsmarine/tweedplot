//! Drawing LINZ hydrographic features (lights, buoys, beacons, dangers, areas)
//! into the vector tiles, looking one up for its details, and listing the
//! dangers a route must avoid.
//!
//! MVT layers added:
//!
//! | Layer | Content | Properties |
//! | --- | --- | --- |
//! | `fpt`  | point features | `cls` (S-57 class), `icon`, `lbl`, `name`, `t`, `fid`, `band`, `minz` |
//! | `fln`  | lines (cables, pipelines, leading lines, tracks) | `cls`, `name`, `t`, `fid`, `band`, `minz` |
//! | `far`  | areas (restricted, farms, anchorages, ...) | `cls`, `cat`, `name`, `t`, `fid`, `band`, `minz` |
//! | `lsec` | light sector arcs and limits | `col` (r, g, w, o), `k` (arc, leg), `minz` |
//!
//! A feature class may exist in several scale bands: points from a coarser
//! band are left out where the finer band has one of the same class within
//! 150 m, and a line or area from a coarser band where the finer band charts
//! the water (its depth areas cover most of the feature). That is decided feature by feature, not tile by
//! tile, so a feature never changes shape at a tile edge.
//!
//! Hazards to the vessel (rocks, wrecks, obstructions, piles, platforms, marine
//! farms, hulks, bridges, overhead cables and pipelines) are never hidden by
//! zoom: every band is searched at every zoom, SCAMIN is ignored for them (it
//! still comes through as `minz`, for those deeper than the safety contour), and
//! they carry `hz` = 1 so the chart draws them whatever else is switched off.
//! Where they would pile up at small scales they are thinned to one per few
//! pixels, keeping the most dangerous, and an area too small to see is also
//! drawn as its symbol. (IMO's ECDIS performance standard keeps isolated
//! dangers in the display base, which can't be removed; small-scale chart
//! displays dropping a reef were behind the Vestas Wind stranding in 2014.)
//!
//! Other properties: `db` = 1 on what IMO's ECDIS performance standard puts in the
//! display base besides dangers (buoys and beacons, ships' routeing); `wd` on a rock,
//! wreck or obstruction is the least DRVAL1 of the depth areas at and just around it
//! (its own band; land counts as 0), so the chart can
//! give an isolated danger (shallower than the safety contour, in water deeper than
//! it: S-52 UDWHAZ05) the isolated danger symbol, and `nd` = 1 marks one charted as
//! not dangerous; `rot` turns a symbol with the chart (a traffic lane's arrow).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use rstar::{
    primitives::{GeomWithData, Rectangle},
    RTree, AABB,
};

use crate::clip::{clip_line, quantise};
use crate::geom::{parse_gpkg, Geom, Ring};
use crate::mvt::{Layer, TileGeom, Value, EXTENT};
use crate::s57::{self, Attrs, Geo, Light, TILE_COLS};
use crate::{band_minzoom, geom_bounds, ChartStore, Kind, TableInfo, TileProj};

type CoveragePolygon = (u8, [f64; 4], geo::Polygon<f64>);
type FeatureBoundsTree = RTree<GeomWithData<Rectangle<(f64, f64)>, (u8, f64)>>;

fn bounds_tree(entries: impl IntoIterator<Item = ([f64; 4], (u8, f64))>) -> FeatureBoundsTree {
    RTree::bulk_load(entries.into_iter().map(|(b, data)| GeomWithData::new(Rectangle::from_corners((b[0], b[1]), (b[2], b[3])), data)).collect())
}

fn bounds_envelope(b: [f64; 4]) -> AABB<(f64, f64)> {
    AABB::from_corners((b[0], b[1]), (b[2], b[3]))
}

pub(crate) struct CoverageIndex {
    pub polygons: Vec<CoveragePolygon>,
    tree: RTree<GeomWithData<Rectangle<(f64, f64)>, usize>>,
}

impl CoverageIndex {
    fn new(polygons: Vec<CoveragePolygon>) -> Self {
        let tree = RTree::bulk_load(
            polygons.iter().enumerate().map(|(i, (_, b, _))| GeomWithData::new(Rectangle::from_corners((b[0], b[1]), (b[2], b[3])), i)).collect(),
        );
        Self { polygons, tree }
    }

    pub(crate) fn intersecting(&self, b: [f64; 4]) -> impl Iterator<Item = &CoveragePolygon> {
        let envelope = AABB::from_corners((b[0], b[1]), (b[2], b[3]));
        self.tree.locate_in_envelope_intersecting(&envelope).map(|entry| &self.polygons[entry.data])
    }
}

/// Hazards to the vessel: shown at every zoom (see the module notes).
pub fn is_hazard(code: &str) -> bool {
    matches!(code, "UWTROC" | "WRECKS" | "OBSTRN" | "PILPNT" | "OFSPLF" | "MARCUL" | "HULKES" | "BRIDGE" | "CBLOHD" | "PIPOHD")
}

/// Information a stranger entering port needs to find early (pilot boarding, radio
/// calling-in points, signal and rescue stations): shown from `INFO_ZOOM` whatever the
/// chart's SCAMIN, which often hides them until the harbour chart.
/// Classes IMO's ECDIS performance standard puts in the display base (with the dangers):
/// buoys and beacons, whether aids to navigation or not, and traffic routeing systems.
pub fn is_display_base(code: &str) -> bool {
    code.starts_with("BOY") || code.starts_with("BCN") || matches!(code, "TSSLPT" | "TSSBND" | "TSSCRS" | "TSSRON" | "TSEZNE" | "TSELNE" | "ISTZNE")
}

/// Charted as not dangerous to surface navigation (S-52 leaves these out of the
/// isolated-danger test): a non-dangerous wreck or distributed remains, foul ground.
fn not_dangerous(code: &str, a: &Attrs) -> bool {
    match code {
        "WRECKS" => matches!(a.first("catwrk"), Some(1 | 3)),
        "OBSTRN" => matches!(a.first("catobs"), Some(5..=7)),
        _ => false,
    }
}

/// The depth areas of a tile, to find the water a danger lies in.
pub(crate) struct DepthAreas(Vec<(u8, f64, [f64; 4], Vec<Ring>)>);

impl DepthAreas {
    /// One band's depth areas in `bbox`.
    pub(crate) fn load_band(store: &ChartStore, band: u8, bbox: [f64; 4]) -> Self {
        let mut v = Vec::new();
        for t in store.tables.iter().filter(|t| t.kind == Kind::DepthArea && t.band == band) {
            store.query(t, bbox, &["drval1"], |vals, g| {
                if let (Some(d1), Geom::Polygons(polys)) = (vals[0], g) {
                    for rings in polys {
                        let Some(outer) = rings.first() else { continue };
                        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                        for p in outer {
                            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
                        }
                        v.push((band, d1, b, rings));
                    }
                }
            });
        }
        DepthAreas(v)
    }

    fn load(store: &ChartStore, bbox: [f64; 4]) -> Self {
        let mut v = Vec::new();
        store.for_each_depth_band(bbox, |band, d1, _, rings| {
            if let (Some(d1), Some(outer)) = (d1, rings.first()) {
                let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                for p in outer {
                    b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
                }
                v.push((band, d1, b, rings.to_vec()));
            }
        });
        DepthAreas(v)
    }

    /// DRVAL1 of the depth area at `p` in the band, if any.
    fn at(&self, band: u8, p: [f64; 2]) -> Option<f64> {
        self.0
            .iter()
            .filter(|(b, _, bb, _)| *b == band && p[0] >= bb[0] && p[0] <= bb[2] && p[1] >= bb[1] && p[1] <= bb[3])
            .find(|(_, _, _, rings)| crate::point_in_polygon(rings, p[0], p[1]))
            .map(|(_, d1, _, _)| *d1)
    }

    /// The water a danger lies in, for the isolated danger test: the least DRVAL1 of the
    /// danger's own band at it and around it, about half a millimetre away at the band's
    /// scale. As S-52 looks in the danger's own cell, other bands don't count. Land or no
    /// depth area there counts as dry, so a rock at the foot of a cliff, or one on the
    /// edge of shallower water the shading already shows, isn't an isolated danger; one
    /// with deep water all round is. None where the band has no depth area at the danger.
    pub(crate) fn around(&self, band: u8, p: [f64; 2]) -> Option<f64> {
        let mut least = self.at(band, p)?;
        let r_m = match band {
            0 | 1 => 2500.0,
            2 => 700.0,
            3 => 200.0,
            4 => 50.0,
            5 => 20.0,
            _ => 10.0,
        };
        let (dlat, dlon) = (r_m / 111_320.0, r_m / (111_320.0 * p[1].to_radians().cos().max(0.05)));
        for k in 0..8 {
            let a = k as f64 * std::f64::consts::FRAC_PI_4;
            let q = [p[0] + dlon * a.cos(), p[1] + dlat * a.sin()];
            least = least.min(self.at(band, q).unwrap_or(0.0));
        }
        Some(least)
    }
}

pub fn is_info(code: &str) -> bool {
    matches!(code, "PILBOP" | "RDOCAL" | "SISTAT" | "RSCSTA")
}
pub const INFO_ZOOM: f64 = 9.0;
const HAZARD_CLIP_MIN_ZOOM: u8 = 7;

fn clip_coverage_for(hazard: bool, z: u8) -> bool {
    !hazard || z >= HAZARD_CLIP_MIN_ZOOM
}

/// A finer chart's lines and areas are read two zooms before that chart is drawn, for
/// the ones a coarser chart already shows (its copy can be cut off at its cell's edge):
/// see `shown_from`.
fn early_band(z: u8, t: &TableInfo) -> bool {
    z < band_minzoom(t.band) && z + 2 >= band_minzoom(t.band) && t.class.is_some_and(|c| c.geo != Geo::Point)
}

/// How dangerous a hazard point is, lower first: drying or awash, then of unknown
/// depth, then by depth over it; other hazards after.
fn hazard_rank(code: &str, a: &Attrs) -> f64 {
    match code {
        "UWTROC" | "WRECKS" | "OBSTRN" => match a.first("watlev") {
            Some(1) | Some(2) | Some(4) | Some(5) => -2.0,
            _ => a.num("valsou").unwrap_or(-1.0),
        },
        _ => 1000.0,
    }
}

// Shape operations with Clipper2 (a pure Rust port) on lon/lat scaled to integers:
// i_overlay (inside the geo crate's BooleanOps) ran away on some charts, asking for
// tens of gigabytes.
const SCALE: f64 = 1e7;

fn to_path(r: &[[f64; 2]], outer: bool) -> clipper2_rust::Path64 {
    let mut p: clipper2_rust::Path64 = r.iter().map(|q| clipper2_rust::Point64::new((q[0] * SCALE).round() as i64, (q[1] * SCALE).round() as i64)).collect();
    p.dedup();
    while p.len() > 1 && p.first() == p.last() {
        p.pop();
    }
    // outer rings anticlockwise, holes clockwise, so the non-zero rule merges overlaps
    let a: f64 = (0..p.len())
        .map(|k| {
            let (u, v) = (p[k], p[(k + 1) % p.len()]);
            u.x as f64 * v.y as f64 - v.x as f64 * u.y as f64
        })
        .sum();
    if (a > 0.0) != outer {
        p.reverse();
    }
    p
}

fn polys_to_paths(polys: &[Vec<Ring>]) -> clipper2_rust::Paths64 {
    let mut out = clipper2_rust::Paths64::new();
    for rings in polys {
        for (k, r) in rings.iter().enumerate() {
            if r.len() >= 3 {
                out.push(to_path(r, k == 0));
            }
        }
    }
    out
}

fn cover_paths(cover: &geo::MultiPolygon<f64>) -> clipper2_rust::Paths64 {
    let mut out = clipper2_rust::Paths64::new();
    for p in &cover.0 {
        let r = |l: &geo::LineString<f64>| -> Ring { l.coords().map(|c| [c.x, c.y]).collect() };
        out.push(to_path(&r(p.exterior()), true));
        for h in p.interiors() {
            out.push(to_path(&r(h), false));
        }
    }
    out
}

fn from_path(p: &clipper2_rust::Path64) -> Ring {
    let mut r: Ring = p.iter().map(|q| [q.x as f64 / SCALE, q.y as f64 / SCALE]).collect();
    if let Some(f) = r.first().copied() {
        r.push(f);
    }
    r
}

/// Polygons (as faces with their holes) from a Clipper tree.
fn tree_polys(tree: &clipper2_rust::PolyTree64) -> Vec<Vec<Ring>> {
    clipper2_rust::poly_tree_to_faces64(tree)
        .into_iter()
        .filter(|f| f.outer.len() >= 3)
        .map(|f| std::iter::once(from_path(&f.outer)).chain(f.holes.iter().filter(|h| h.len() >= 3).map(from_path)).collect())
        .collect()
}

fn clip_geom(g: &Geom, cover: &geo::MultiPolygon<f64>, outside: bool) -> Option<Geom> {
    clip_geom_paths(g, &cover_paths(cover), outside)
}

fn clip_geom_paths(g: &Geom, clip: &clipper2_rust::Paths64, outside: bool) -> Option<Geom> {
    use clipper2_rust::{ClipType, Clipper64, FillRule, Paths64, PolyTree64};
    let ct = if outside { ClipType::Difference } else { ClipType::Intersection };
    match g {
        Geom::Polygons(polys) => {
            let mut tree = PolyTree64::new();
            clipper2_rust::boolean_op_tree_64(ct, FillRule::NonZero, &polys_to_paths(polys), clip, &mut tree);
            let out: Vec<Vec<Ring>> = tree_polys(&tree)
                .into_iter()
                .filter(|p| {
                    let b = geom_bounds(&Geom::Polygons(vec![vec![p[0].clone()]]));
                    (b[2] - b[0]) * (b[3] - b[1]) > 1e-12
                })
                .collect();
            (!out.is_empty()).then_some(Geom::Polygons(out))
        }
        Geom::Lines(lines) => {
            let open: Paths64 = lines
                .iter()
                .filter(|l| l.len() >= 2)
                .map(|l| l.iter().map(|q| clipper2_rust::Point64::new((q[0] * SCALE).round() as i64, (q[1] * SCALE).round() as i64)).collect())
                .collect();
            let mut c = Clipper64::new();
            c.add_open_subject(&open);
            c.add_clip(clip);
            let mut closed = Paths64::new();
            let mut res = Paths64::new();
            c.execute(ct, FillRule::NonZero, &mut closed, Some(&mut res));
            let out: Vec<Ring> = res.iter().filter(|p| p.len() >= 2).map(|p| p.iter().map(|q| [q.x as f64 / SCALE, q.y as f64 / SCALE]).collect()).collect();
            (!out.is_empty()).then_some(Geom::Lines(out))
        }
        Geom::Points(_) => None,
    }
}

pub(crate) struct CoverageClip {
    bounds: Option<[f64; 4]>,
    paths: clipper2_rust::Paths64,
}

impl CoverageClip {
    pub(crate) fn new(cover: &geo::MultiPolygon<f64>) -> Self {
        use geo::BoundingRect;
        let bounds = cover.bounding_rect().map(|r| [r.min().x, r.min().y, r.max().x, r.max().y]);
        Self { bounds, paths: cover_paths(cover) }
    }

    /// Whether a point is inside the coverage.
    pub(crate) fn contains(&self, p: [f64; 2]) -> bool {
        use clipper2_rust::core::{point_in_polygon, Point64, PointInPolygonResult};
        let Some(b) = self.bounds else { return false };
        if p[0] < b[0] || p[0] > b[2] || p[1] < b[1] || p[1] > b[3] {
            return false;
        }
        let q = Point64::new((p[0] * SCALE).round() as i64, (p[1] * SCALE).round() as i64);
        self.paths.iter().filter(|path| point_in_polygon(q, path) != PointInPolygonResult::IsOutside).count() % 2 == 1
    }

    fn reaches(&self, g: &Geom) -> bool {
        let Some(b) = self.bounds else { return false };
        let gb = geom_bounds(g);
        b[0] <= gb[2] && b[2] >= gb[0] && b[1] <= gb[3] && b[3] >= gb[1]
    }
}

pub(crate) fn cut_outside_cached(g: &Geom, cover: &CoverageClip) -> Option<Geom> {
    if matches!(g, Geom::Points(_)) || !cover.reaches(g) {
        return Some(g.clone());
    }
    clip_geom_paths(g, &cover.paths, true)
}

pub(crate) fn cut_inside_cached(g: &Geom, cover: &CoverageClip) -> Option<Geom> {
    if matches!(g, Geom::Points(_)) || !cover.reaches(g) {
        return None;
    }
    clip_geom_paths(g, &cover.paths, false)
}

/// The part of a line or area geometry outside `cover`: None when nothing is left,
/// the geometry itself when it doesn't reach the cover.
pub(crate) fn cut_outside(g: &Geom, cover: &geo::MultiPolygon<f64>) -> Option<Geom> {
    if matches!(g, Geom::Points(_)) || !reaches(g, cover) {
        return Some(g.clone());
    }
    clip_geom(g, cover, true)
}

/// Whether the geometry's box meets the cover's box (a quick test before clipping).
fn reaches(g: &Geom, cover: &geo::MultiPolygon<f64>) -> bool {
    use geo::BoundingRect;
    let b = geom_bounds(g);
    cover.bounding_rect().is_some_and(|r| r.min().x <= b[2] && r.max().x >= b[0] && r.min().y <= b[3] && r.max().y >= b[1])
}

/// Light sector radius and limit-line length in tile units (4096 per tile, 8 per pixel).
const ARC_R: f64 = 440.0;
const LEG_R: f64 = 760.0;
const SAME_M: f64 = 150.0;

/// One row of a feature table.
pub(crate) struct Row {
    pub fid: i64,
    pub vals: Vec<Option<String>>,
    pub geom: Geom,
}

/// A feature's details for people.
#[derive(Debug, Clone)]
pub struct FeatureInfo {
    pub code: &'static str,
    pub class_name: &'static str,
    pub band: u8,
    /// [lon, lat] of the first point.
    pub position: Option<[f64; 2]>,
    /// (attribute in words, value in words), empty values left out.
    pub attrs: Vec<(String, String)>,
}

fn metres(a: [f64; 2], b: [f64; 2]) -> f64 {
    let k = 111_320.0;
    let dx = (a[0] - b[0]) * k * a[1].to_radians().cos();
    let dy = (a[1] - b[1]) * k;
    (dx * dx + dy * dy).sqrt()
}

fn text(v: rusqlite::types::ValueRef) -> Option<String> {
    use rusqlite::types::ValueRef::*;
    match v {
        Null => None,
        Integer(i) => Some(i.to_string()),
        Real(f) => Some(if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { format!("{f}") }),
        Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        Blob(_) => None,
    }
}

impl ChartStore {
    /// Rows of a feature table in `b` [w, s, e, n], with the columns asked for
    /// (None where the table hasn't that column).
    pub(crate) fn query_text(&self, t: &TableInfo, b: [f64; 4], cols: &[&str], mut f: impl FnMut(Row)) {
        if !t.touches(b) {
            return;
        }
        let conn = match &self.dbs[t.db] {
            crate::remote::Db::Local(c) => c.lock().unwrap_or_else(|e| e.into_inner()),
            crate::remote::Db::Remote(src, db) => {
                for shift in crate::wrap_shifts(b) {
                    for r in src.rows(*db, &t.table, [b[0] + shift, b[1], b[2] + shift, b[3]], cols) {
                        let vals: Vec<Option<String>> = r.vals.iter().map(plugin_api::Value::as_text).collect();
                        if let Some(mut geom) = parse_gpkg(&r.geom) {
                            crate::shift_geom(&mut geom, -shift);
                            f(Row { fid: r.fid, vals, geom });
                        }
                    }
                }
                return;
            }
        };
        let gcol = t.rtree.trim_start_matches(&format!("rtree_{}_", t.table)).to_string();
        let have: Vec<bool> = cols.iter().map(|c| t.cols.iter().any(|x| x == c)).collect();
        let sel: String = cols.iter().zip(&have).map(|(c, h)| if *h { format!("t.\"{c}\", ") } else { "NULL, ".into() }).collect();
        let sql = format!(
            "SELECT t.\"{pk}\", {sel}t.\"{gcol}\" FROM \"{tbl}\" t JOIN \"{rt}\" r ON t.\"{pk}\" = r.id
             WHERE r.maxx >= ?1 AND r.minx <= ?2 AND r.maxy >= ?3 AND r.miny <= ?4",
            tbl = t.table,
            rt = t.rtree,
            pk = t.pk
        );
        let Ok(mut st) = conn.prepare_cached(&sql) else {
            return;
        };
        // data east of 180° may be stored as 180..190: look there too (see crate::wrap_shifts)
        for shift in crate::wrap_shifts(b) {
            let Ok(mut rows) = st.query(rusqlite::params![b[0] + shift, b[2] + shift, b[1], b[3]]) else {
                continue;
            };
            while let Ok(Some(r)) = rows.next() {
                let fid: i64 = r.get(0).unwrap_or(0);
                let vals: Vec<Option<String>> = (0..cols.len()).map(|i| r.get_ref(i + 1).ok().and_then(text)).collect();
                let Ok(blob) = r.get_ref(cols.len() + 1).and_then(|v| v.as_blob().map_err(Into::into)) else {
                    continue;
                };
                if let Some(mut geom) = parse_gpkg(blob) {
                    crate::shift_geom(&mut geom, -shift);
                    f(Row { fid, vals, geom });
                }
            }
        }
    }

    /// The zoom from which a table's features usually show at latitude `lat`: from its
    /// median SCAMIN, else its band's.
    fn takeover_zoom(&self, ti: usize, t: &TableInfo, lat: f64) -> f64 {
        let median = self.scamin_median.lock().unwrap_or_else(|e| e.into_inner()).entry(ti).or_insert_with(|| Arc::new(OnceLock::new())).clone();
        let med = *median.get_or_init(|| {
            if !t.cols.iter().any(|c| c == "scamin") {
                return None;
            }
            let conn = match &self.dbs[t.db] {
                crate::remote::Db::Local(c) => c.lock().unwrap_or_else(|e| e.into_inner()),
                crate::remote::Db::Remote(src, db) => return src.median(*db, &t.table, "scamin"),
            };
            let n: i64 = conn.query_row(&format!("SELECT count(scamin) FROM \"{}\"", t.table), [], |r| r.get(0)).unwrap_or(0);
            if n == 0 {
                return None;
            }
            conn.query_row(&format!("SELECT scamin FROM \"{}\" WHERE scamin IS NOT NULL ORDER BY scamin LIMIT 1 OFFSET {}", t.table, n / 2), [], |r| {
                r.get::<_, f64>(0)
            })
            .ok()
        });
        med.map_or(band_minzoom(t.band) as f64, |s| s57::scamin_zoom(s, lat))
    }

    fn feature_tables(&self, z: u8, bbox: [f64; 4], starts: &[u8; 8]) -> Vec<(usize, &TableInfo)> {
        self.tables
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.kind == Kind::Feature
                    && t.touches(bbox)
                    && (z >= starts[(t.band as usize).min(7)]
                        || early_band(z, t)
                        || t.class.is_some_and(|c| is_hazard(c.code) || (is_info(c.code) && z as f64 >= INFO_ZOOM)))
            })
            .collect()
    }

    /// Every band's coverage polygons (data areas, CATCOV 1), loaded once.
    pub(crate) fn coverage(&self) -> &CoverageIndex {
        self.coverage_with_progress(&mut |_, _| {})
    }

    pub(crate) fn coverage_with_progress(&self, progress: &mut dyn FnMut(usize, usize)) -> &CoverageIndex {
        self.coverage.get_or_init(|| {
            let tables: Vec<_> = self.tables.iter().filter(|t| t.kind == Kind::Coverage).collect();
            let total = tables.len();
            progress(0, total);
            let mut out = Vec::new();
            for (index, t) in tables.into_iter().enumerate() {
                self.query_text(t, [-180.0, -90.0, 190.0, 90.0], &["catcov"], |row| {
                    if row.vals[0].as_deref().is_some_and(|c| c.trim() == "2") {
                        return; // "no coverage available"
                    }
                    if let Geom::Polygons(polys) = &row.geom {
                        for rings in polys {
                            let Some(outer) = rings.first() else { continue };
                            let ls = |r: &Ring| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
                            let poly = geo::Polygon::new(ls(outer), rings[1..].iter().map(ls).collect());
                            let b = geom_bounds(&Geom::Polygons(vec![vec![outer.clone()]]));
                            out.push((t.band, b, poly));
                        }
                    }
                });
                progress(index + 1, total);
            }
            CoverageIndex::new(out)
        })
    }

    /// The coverage of bands in `bands` that touches `b`, merged.
    pub(crate) fn cover_in(&self, bands: &[u8], b: [f64; 4]) -> Option<geo::MultiPolygon<f64>> {
        let polys: Vec<&geo::Polygon<f64>> = self
            .coverage()
            .intersecting(b)
            .filter(|(band, cb, _)| bands.contains(band) && cb[2] >= b[0] && cb[0] <= b[2] && cb[3] >= b[1] && cb[1] <= b[3])
            .map(|(_, _, p)| p)
            .collect();
        if polys.is_empty() {
            return None;
        }
        // merged with Clipper2 (see clip_geom)
        let r = |l: &geo::LineString<f64>| -> Ring { l.coords().map(|c| [c.x, c.y]).collect() };
        let subj: Vec<Vec<Ring>> = polys.iter().map(|p| std::iter::once(r(p.exterior())).chain(p.interiors().iter().map(r)).collect()).collect();
        let mut tree = clipper2_rust::PolyTree64::new();
        clipper2_rust::boolean_op_tree_64(
            clipper2_rust::ClipType::Union,
            clipper2_rust::FillRule::NonZero,
            &polys_to_paths(&subj),
            &clipper2_rust::Paths64::new(),
            &mut tree,
        );
        let ls = |r: &Ring| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
        let m = geo::MultiPolygon::new(tree_polys(&tree).iter().map(|rings| geo::Polygon::new(ls(&rings[0]), rings[1..].iter().map(ls).collect())).collect());
        Some(m)
    }

    /// Whether `p` lies in the coverage of one of `bands`.
    pub(crate) fn covered(&self, bands: &[u8], p: [f64; 2]) -> bool {
        use geo::Contains;
        let pt = geo::Point::new(p[0], p[1]);
        self.coverage()
            .intersecting([p[0], p[1], p[0], p[1]])
            .any(|(band, b, poly)| bands.contains(band) && p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3] && poly.contains(&pt))
    }

    /// The four feature layers of one tile: [areas, lines, points, light sectors].
    pub(crate) fn render_features(&self, tp: &TileProj, z: u8, starts: &[u8; 8]) -> [Layer; 4] {
        let mut far = Layer::new("far");
        let mut fln = Layer::new("fln");
        let mut fpt = Layer::new("fpt");
        let mut lsec = Layer::new("lsec");
        let bbox = tp.lonlat_bbox(crate::BUFFER);
        let all = self.feature_tables(z, bbox, starts);
        if all.is_empty() {
            return [far, fln, fpt, lsec];
        }
        // classes in the order of s57::CLASSES, tables finest band first
        let mut groups: Vec<(&'static s57::FeatureClass, Vec<(usize, &TableInfo)>)> = Vec::new();
        for (i, t) in &all {
            let c = t.class.unwrap();
            match groups.iter_mut().find(|(g, _)| std::ptr::eq(*g, c)) {
                Some((_, v)) => v.push((*i, t)),
                None => groups.push((c, vec![(*i, t)])),
            }
        }
        for (_, v) in groups.iter_mut() {
            v.sort_by_key(|(_, t)| std::cmp::Reverse(t.band));
        }
        let mut coverage_clips = HashMap::new();
        for (_, tables) in &groups {
            for (k, (_, table)) in tables.iter().enumerate() {
                let mut finer_bands: Vec<u8> = tables[..k].iter().map(|(_, finer)| finer.band).filter(|band| *band > table.band).collect();
                finer_bands.dedup();
                if !finer_bands.is_empty() {
                    coverage_clips.entry(finer_bands.clone()).or_insert_with(|| self.cover_in(&finer_bands, bbox).map(|cover| CoverageClip::new(&cover)));
                }
            }
        }
        let keep = |minz: f64| z >= 16 || (z as f64 + 1.0) > minz;
        let e = EXTENT as f64;
        let inside = |p: [f64; 2]| p[0] >= 0.0 && p[0] < e && p[1] >= 0.0 && p[1] < e;
        // hazard symbols at small scales: one per cell of this many tile units (8 per pixel)
        let thin = if z >= 12 {
            0.0
        } else if z >= 9 {
            48.0
        } else {
            96.0
        };
        type Props = Vec<(&'static str, Value)>;
        let mut hz_pts: HashMap<(i64, i64), (f64, [i32; 2], Props)> = HashMap::new();
        let mut put_hazard = |px: [f64; 2], rank: f64, props: Props, fpt: &mut Layer| {
            let q = [px[0].round() as i32, px[1].round() as i32];
            if thin == 0.0 {
                fpt.add(&TileGeom::Points(vec![q]), &props);
                return;
            }
            let k = ((px[0] / thin).floor() as i64, (px[1] / thin).floor() as i64);
            if hz_pts.get(&k).is_none_or(|(r, _, _)| rank < *r) {
                hz_pts.insert(k, (rank, q, props));
            }
        };

        // buoys, beacons and other marks drawn in this tile: (position, band, minz)
        let mut marks: Vec<([f64; 2], u8, f64)> = Vec::new();
        // the depth areas, read the first time a danger needs them
        let mut depth_areas: Option<DepthAreas> = None;
        for (class, tables) in &groups {
            match class.geo {
                // lights last, so they can follow the marks they stand on (below)
                Geo::Point if class.code == "LIGHTS" => {}
                Geo::Point => {
                    // finer-band copies in this tile: (position, band, minz)
                    let mut taken: Vec<([f64; 2], u8, f64)> = Vec::new();
                    for (k, (ti, t)) in tables.iter().enumerate() {
                        let mut finer_bands: Vec<u8> = tables[..k].iter().map(|(_, ft)| ft.band).filter(|b| *b > t.band).collect();
                        finer_bands.dedup();
                        let mut mine = Vec::new();
                        // the IHO portrayal of this table (worked out before reading it)
                        let syms = self.s52_table(*ti);
                        self.query_text(t, bbox, TILE_COLS, |row| {
                            let Geom::Points(pts) = &row.geom else { return };
                            let a = Attrs(&row.vals);
                            for &p in pts {
                                // a finer chart's copy of this mark: it stands from its own zoom; this
                                // coarser copy fills in until then (maxz), so the mark never goes missing
                                let finer = taken.iter().filter(|(q, b, _)| *b > t.band && metres(p, *q) < SAME_M).map(|(_, _, m)| *m).fold(f64::MAX, f64::min);
                                if finer <= z as f64 {
                                    continue;
                                }
                                // a hazard where a finer band charts the water: the finer band's copy
                                // (or its absence) stands, even if the coarse copy was moved when the
                                // smaller-scale chart was drawn. (Other marks keep the coarser copy
                                // until the finer one shows, as lights do, so a buoy and its light
                                // always come together.)
                                if (is_hazard(class.code) || is_info(class.code)) && !finer_bands.is_empty() && self.covered(&finer_bands, p) {
                                    continue;
                                }
                                let mut minz = a.num("scamin").map_or(0.0, |s| s57::scamin_zoom(s, p[1]));
                                if is_info(class.code) {
                                    minz = minz.min(INFO_ZOOM);
                                }
                                let hazard = is_hazard(class.code);
                                if !hazard && !keep(minz) {
                                    continue;
                                }
                                // only a feature actually drawn at this zoom hides a coarser band's copy
                                mine.push((p, t.band, minz));
                                if !hazard {
                                    marks.push((p, t.band, minz));
                                }
                                let px = tp.px(&p);
                                if !inside(px) {
                                    continue;
                                }
                                let Some(icon) = s57::icon(class.code, &a) else {
                                    continue;
                                };
                                let mut props = vec![
                                    ("cls", Value::Str(class.code.into())),
                                    ("icon", Value::Str(icon)),
                                    ("t", Value::UInt(*ti as u64)),
                                    ("fid", Value::UInt(row.fid.max(0) as u64)),
                                    ("band", Value::UInt(t.band as u64)),
                                    ("minz", Value::F64((minz * 10.0).round() / 10.0)),
                                ];
                                if finer < f64::MAX {
                                    props.push(("maxz", Value::F64((finer * 10.0).round() / 10.0)));
                                }
                                if let Some(l) = s57::label(class.code, &a) {
                                    props.push(("lbl", Value::Str(l)));
                                }
                                // depth over a rock, wreck or obstruction, so the chart can leave
                                // the danger symbol off those deeper than the safety contour
                                if matches!(class.code, "UWTROC" | "WRECKS" | "OBSTRN") && matches!(a.first("watlev"), None | Some(3)) {
                                    if let Some(d) = a.num("valsou") {
                                        props.push(("dep", Value::F64(d)));
                                    }
                                }
                                // the water it lies in, for the isolated danger test (S-52 UDWHAZ05)
                                if matches!(class.code, "UWTROC" | "WRECKS" | "OBSTRN") {
                                    let da = depth_areas
                                        .get_or_insert_with(|| DepthAreas::load(self, [bbox[0] - 0.01, bbox[1] - 0.01, bbox[2] + 0.01, bbox[3] + 0.01]));
                                    if let Some(wd) = da.around(t.band, p) {
                                        props.push(("wd", Value::F64(wd)));
                                    }
                                    if not_dangerous(class.code, &a) {
                                        props.push(("nd", Value::UInt(1)));
                                    }
                                }
                                if is_display_base(class.code) {
                                    props.push(("db", Value::UInt(1)));
                                }
                                // the IHO portrayal's symbols, when it is on
                                if let Some(e) = syms.as_ref().and_then(|m| m.get(&row.fid)) {
                                    let (sy, si) = crate::s52::split_info(e);
                                    match crate::s52::entry_category(e) {
                                        0 => {}
                                        c => props.push(("vg", Value::UInt(c as u64))),
                                    }
                                    if !sy.is_empty() {
                                        props.push(("sy", Value::Str(sy.into())));
                                    }
                                    if !si.is_empty() {
                                        props.push(("si", Value::Str(si.into())));
                                    }
                                }
                                if let Some(n) = a.get("objnam") {
                                    props.push(("name", Value::Str(n.into())));
                                }
                                if hazard {
                                    props.push(("hz", Value::UInt(1)));
                                    put_hazard(px, hazard_rank(class.code, &a), props, &mut fpt);
                                } else {
                                    fpt.add(&TileGeom::Points(vec![[px[0].round() as i32, px[1].round() as i32]]), &props);
                                }
                            }
                        });
                        taken.extend(mine);
                    }
                }
                Geo::Line | Geo::Area => {
                    let hazard = is_hazard(class.code);
                    let layer = if class.geo == Geo::Line { &mut fln } else { &mut far };
                    // every copy of this class met so far (finer bands first): bounds, minz
                    let mut seen: Vec<([f64; 4], f64)> = Vec::new();
                    // every copy's bounds and SCAMIN zoom, by band: a finer copy of a feature a
                    // coarser chart already shows here shows from that zoom too, not its own later
                    // SCAMIN (a coarser chart's copy is often cut off at the edge of its cell, as
                    // Lyttelton's dumping grounds are, so the finer whole one has to stand in)
                    let lat_m = (bbox[1] + bbox[3]) / 2.0;
                    // the IHO portrayal's results for these tables (before the queries: each
                    // holds the table's database while it runs)
                    let s52: Vec<Option<crate::s52::TableSymbols>> = tables.iter().map(|(ti, _)| self.s52_table(*ti)).collect();
                    let rows_by_table: Vec<Vec<Row>> = tables
                        .iter()
                        .map(|(_, t)| {
                            let mut rows = Vec::new();
                            self.query_text(t, bbox, &["objnam", "scamin", "catrea", "restrn", "catmfa", "orient", "watlev"], |row| rows.push(row));
                            rows
                        })
                        .collect();
                    let all_copies = bounds_tree(tables.iter().enumerate().flat_map(|(k, (_, t))| {
                        rows_by_table[k].iter().map(move |row| {
                            let minz = row.vals[1].as_deref().and_then(|s| s.parse().ok()).map_or(0.0, |s| s57::scamin_zoom(s, lat_m));
                            (geom_bounds(&row.geom), (t.band, minz))
                        })
                    }));
                    let shown_from = |band: u8, gb: &[f64; 4], minz: f64| {
                        all_copies
                            .locate_in_envelope_intersecting(&bounds_envelope(*gb))
                            .filter(|copy| copy.data.0 < band)
                            .map(|copy| copy.data.1)
                            .fold(minz, f64::min)
                    };
                    for (k, (ti, t)) in tables.iter().enumerate() {
                        // the finer bands drawn here: those of this class's tables (finest first)
                        let mut finer_bands: Vec<u8> = tables[..k].iter().map(|(_, ft)| ft.band).filter(|b| *b > t.band).collect();
                        finer_bands.dedup();
                        let cover = coverage_clips.get(&finer_bands).and_then(Option::as_ref);
                        // At small scales, per-feature hazard clipping against chart coverage
                        // dominates tile time. Keep the geometry intact until it is legible.
                        let cover = cover.filter(|_| clip_coverage_for(hazard, z));
                        // when the finer bands' copies of this class start to show here: until
                        // then this band's copy stands inside their coverage too (maxz)
                        let lat_c = (bbox[1] + bbox[3]) / 2.0;
                        let general =
                            tables[..k].iter().filter(|(_, ft)| ft.band > t.band).map(|(fi, ft)| self.takeover_zoom(*fi, ft, lat_c)).fold(f64::MAX, f64::min);
                        let finer_seen: Vec<([f64; 4], f64)> = std::mem::take(&mut seen);
                        let finer_index = bounds_tree(finer_seen.iter().map(|(bounds, minz)| (*bounds, (0, *minz))));
                        let mut mine_seen: Vec<([f64; 4], f64)> = Vec::new();
                        for row in &rows_by_table[k] {
                            // Inside a finer band's coverage that band's features are the ones
                            // to show (a farm or cable is often drawn differently at each scale):
                            // a coarser band's feature is cut back to outside it, as an ECDIS does,
                            // from the zoom the finer copies show; before that the inside part stays.
                            let gb = geom_bounds(&row.geom);
                            let lat0 = (bbox[1] + bbox[3]) / 2.0;
                            let own = row.vals[1]
                                .as_deref()
                                .and_then(|s| s.parse().ok())
                                .map_or(0.0, |s| s57::scamin_zoom(s, lat0))
                                .min(if is_info(class.code) { INFO_ZOOM } else { f64::MAX });
                            mine_seen.push((gb, shown_from(t.band, &gb, own)));
                            // the finer copies of this feature (those overlapping it) show from the
                            // latest of their zooms; with none here, from when that band's usually do
                            let local = finer_index.locate_in_envelope_intersecting(&bounds_envelope(gb)).map(|copy| copy.data.1).fold(f64::MIN, f64::max);
                            let takeover = if local > f64::MIN { local } else { general };
                            let mut geom_cut = None;
                            let mut inside_part = None;
                            if let Some(cover) = cover {
                                if !hazard && takeover > z as f64 && takeover < f64::MAX {
                                    inside_part = cut_inside_cached(&row.geom, cover);
                                }
                                match cut_outside_cached(&row.geom, cover) {
                                    Some(g) => geom_cut = Some(g),
                                    None if inside_part.is_none() => continue, // all of it inside
                                    None => geom_cut = Some(Geom::Lines(vec![])),
                                }
                            }
                            let geom = geom_cut.as_ref().unwrap_or(&row.geom);
                            let tg = if class.geo == Geo::Line { tp.lines(geom) } else { tp.polygons(geom) };
                            let lat = (bbox[1] + bbox[3]) / 2.0;
                            let mut minz = row.vals[1].as_deref().and_then(|s| s.parse().ok()).map_or(0.0, |s| s57::scamin_zoom(s, lat));
                            if is_info(class.code) {
                                minz = minz.min(INFO_ZOOM);
                            }
                            let own_minz = minz;
                            minz = shown_from(t.band, &gb, minz);
                            if !hazard && !keep(minz) {
                                continue;
                            }
                            // a chart read early: only its copies of what a coarser chart shows
                            if early_band(z, t) && z < starts[(t.band as usize).min(7)] && minz >= own_minz {
                                continue;
                            }
                            let mut props = vec![
                                ("cls", Value::Str(class.code.into())),
                                ("t", Value::UInt(*ti as u64)),
                                ("fid", Value::UInt(row.fid.max(0) as u64)),
                                ("band", Value::UInt(t.band as u64)),
                                ("minz", Value::F64((minz * 10.0).round() / 10.0)),
                            ];
                            if hazard {
                                props.push(("hz", Value::UInt(1)));
                            }
                            if is_display_base(class.code) {
                                props.push(("db", Value::UInt(1)));
                            }
                            if let Some(n) = row.vals[0].as_deref().filter(|s| !s.trim().is_empty()) {
                                props.push(("name", Value::Str(n.trim().into())));
                            }
                            // how the IHO rules draw it (`s5`: they do)
                            let drawn = s52[k].as_ref().and_then(|m| m.get(&row.fid)).filter(|e| !e.is_empty());
                            let mut centre_symbol = None;
                            let mut centre_info = None;
                            if let Some(e) = drawn {
                                props.push(("s5", Value::UInt(1)));
                                for (key, v) in crate::s52::props(e) {
                                    match key {
                                        "slw" | "ct" => props.push((key, Value::F64(v.parse().unwrap_or(0.0)))),
                                        "vg" => props.push((key, Value::UInt(v.parse().unwrap_or(3)))),
                                        "sy" => centre_symbol = Some(v.to_string()),
                                        "si" => centre_info = Some(v.to_string()),
                                        _ => props.push((key, Value::Str(v.into()))),
                                    }
                                }
                            }
                            // restrictions that matter to a yacht: entry prohibited / restricted, area to be avoided
                            if let Some(r) = row.vals[3].as_deref() {
                                if r.split(',').any(|c| matches!(c.trim(), "7" | "8" | "14")) {
                                    props.push(("entry", Value::UInt(1)));
                                }
                            }
                            // a hazard area too small to see at this scale: its symbol as well
                            if hazard && class.geo == Geo::Area {
                                let g = geom_bounds(geom);
                                let (p0, p1) = (tp.px(&[g[0], g[1]]), tp.px(&[g[2], g[3]]));
                                if (p1[0] - p0[0]).abs().max((p1[1] - p0[1]).abs()) < 64.0 {
                                    let c = tp.px(&[(g[0] + g[2]) / 2.0, (g[1] + g[3]) / 2.0]);
                                    let icon = match class.code {
                                        "MARCUL" => Some("mf".to_string()),
                                        "OBSTRN" => Some(format!(
                                            "o|{}",
                                            row.vals[6].as_deref().and_then(|v| v.split(',').next()?.trim().parse::<u32>().ok()).unwrap_or(3)
                                        )),
                                        _ => None,
                                    };
                                    if let (Some(icon), true) = (icon, inside(c)) {
                                        let mut pp = props.clone();
                                        pp.push(("icon", Value::Str(icon)));
                                        if centre_symbol.is_some() {
                                            // the IHO symbol at its middle stands in for it (below)
                                            pp.push(("po", Value::UInt(1)));
                                        }
                                        put_hazard(c, 500.0, pp, &mut fpt);
                                    }
                                }
                            }
                            // an area's symbol at its middle: a traffic lane's direction arrow (S-52
                            // TSSLPT51, turned to its ORIENT), or the "?" of an object without a symbol
                            if class.geo == Geo::Area && matches!(class.code, "TSSLPT" | s57::UNKNOWN) {
                                let g = geom_bounds(geom);
                                let mid = [(g[0] + g[2]) / 2.0, (g[1] + g[3]) / 2.0];
                                // the middle of the bounds may fall outside an odd shape: then a vertex
                                let at = match geom {
                                    Geom::Polygons(p) if !p.iter().any(|r| crate::point_in_polygon(r, mid[0], mid[1])) => {
                                        p.first().and_then(|r| r.first()).and_then(|r| r.first()).copied().unwrap_or(mid)
                                    }
                                    _ => mid,
                                };
                                let c = tp.px(&at);
                                let orient = row.vals[5].as_deref().and_then(|v| v.trim().parse::<f64>().ok());
                                if inside(c) && (class.code != "TSSLPT" || orient.is_some()) {
                                    let mut pp = props.clone();
                                    pp[0] = ("cls", Value::Str(class.code.into()));
                                    if let Some(o) = orient {
                                        pp.push(("icon", Value::Str("ts".into())));
                                        pp.push(("rot", Value::F64(o)));
                                    } else {
                                        pp.push(("icon", Value::Str("uk".into())));
                                    }
                                    fpt.add(&TileGeom::Points(vec![[c[0].round() as i32, c[1].round() as i32]]), &pp);
                                }
                            }
                            // the IHO rules' symbol at an area's middle (its anchorage anchor, the
                            // "i" of an area with notes, ...), and at a line's
                            // (from the whole feature: a coarser chart's copy cut back to outside a
                            // finer chart's coverage keeps its symbol only where the finer copy
                            // doesn't show yet, so one feature has one symbol)
                            if centre_symbol.is_some() || centre_info.is_some() {
                                let geom = &row.geom;
                                let at = match geom {
                                    Geom::Polygons(p) => {
                                        let g = geom_bounds(geom);
                                        let mid = [(g[0] + g[2]) / 2.0, (g[1] + g[3]) / 2.0];
                                        if p.iter().any(|r| crate::point_in_polygon(r, mid[0], mid[1])) {
                                            Some(mid)
                                        } else {
                                            p.first().and_then(|r| r.first()).and_then(|r| r.first()).copied()
                                        }
                                    }
                                    Geom::Lines(l) => l.iter().max_by_key(|l| l.len()).and_then(|l| l.get(l.len() / 2)).copied(),
                                    Geom::Points(_) => None,
                                };
                                // where a finer chart covers the middle: only until its copy shows
                                let finer = at.and_then(|at| cover.filter(|c| c.contains(at)).map(|_| takeover));
                                let shown = finer.is_none_or(|tz| tz > z as f64 && tz < f64::MAX);
                                if let (Some(at), true) = (at, shown) {
                                    let c = tp.px(&at);
                                    if inside(c) {
                                        let mut pp: Vec<(&str, Value)> =
                                            props.iter().filter(|(k, _)| !matches!(*k, "sl" | "slw" | "cf" | "ct" | "af" | "maxz")).cloned().collect();
                                        if let Some(sy) = &centre_symbol {
                                            pp.push(("sy", Value::Str(sy.clone())));
                                        }
                                        if let Some(si) = &centre_info {
                                            pp.push(("si", Value::Str(si.clone())));
                                        }
                                        if let Some(tz) = finer {
                                            pp.push(("maxz", Value::F64((tz.min(30.0) * 10.0).round() / 10.0)));
                                        }
                                        pp.push(("ac", Value::UInt(1)));
                                        fpt.add(&TileGeom::Points(vec![[c[0].round() as i32, c[1].round() as i32]]), &pp);
                                    }
                                }
                            }
                            if let Some(tg) = tg {
                                layer.add(&tg, &props);
                            }
                            // the part a finer chart covers, until its copy shows
                            if let Some(ig) = inside_part {
                                let itg = if class.geo == Geo::Line { tp.lines(&ig) } else { tp.polygons(&ig) };
                                if let Some(itg) = itg {
                                    let mut pp = props.clone();
                                    pp.push(("maxz", Value::F64((takeover.min(30.0) * 10.0).round() / 10.0)));
                                    layer.add(&itg, &pp);
                                }
                            }
                        }
                        seen = finer_seen;
                        seen.extend(mine_seen);
                    }
                }
            }
        }
        for (_, (_, q, props)) in hz_pts {
            fpt.add(&TileGeom::Points(vec![q]), &props);
        }
        if let Some((_, tables)) = groups.iter().find(|(c, _)| c.code == "LIGHTS") {
            self.render_lights(tp, z, tables, &marks, &mut fpt, &mut lsec);
        }
        [far, fln, fpt, lsec]
    }

    /// `marks`: the buoys and beacons drawn in this tile. A light whose mark comes from
    /// a finer chart (which may not carry the light itself: LINZ's layers are not always
    /// complete in every band) shows from the same zoom as that mark, so a light's flare
    /// never stands on the chart without its buoy or beacon.
    fn render_lights(&self, tp: &TileProj, z: u8, tables: &[(usize, &TableInfo)], marks: &[([f64; 2], u8, f64)], fpt: &mut Layer, lsec: &mut Layer) {
        // lights a little outside the tile still throw sectors into it
        let mut bbox = tp.lonlat_bbox(LEG_R + 48.0);
        // full light lines: lights up to about 30 M away reach into the tile
        if self.s52_full_light_lines() {
            let dlat = 30.0 * 1852.0 / 111_320.0;
            let dlon = dlat / ((bbox[1] + bbox[3]) / 2.0).to_radians().cos().max(0.1);
            bbox = [bbox[0] - dlon, (bbox[1] - dlat).max(-85.0), bbox[2] + dlon, (bbox[3] + dlat).min(85.0)];
        }
        struct Rec {
            p: [f64; 2],
            band: u8,
            t: usize,
            fid: i64,
            light: Light,
            minz: f64,
        }
        let mut recs: Vec<Rec> = Vec::new();
        for (ti, t) in tables {
            self.query_text(t, bbox, TILE_COLS, |row| {
                let Geom::Points(pts) = &row.geom else { return };
                let a = Attrs(&row.vals);
                for &p in pts {
                    let minz = a.num("scamin").map_or(0.0, |s| s57::scamin_zoom(s, p[1]));
                    recs.push(Rec { p, band: t.band, t: *ti, fid: row.fid, light: Light::from_attrs(&a), minz });
                }
            });
        }
        // group the records at one spot (sector lights are one record per sector)
        let mut groups: HashMap<(i64, i64, u8), Vec<usize>> = HashMap::new();
        for (i, r) in recs.iter().enumerate() {
            groups.entry(((r.p[0] * 1e5).round() as i64, (r.p[1] * 1e5).round() as i64, r.band)).or_default().push(i);
        }
        let mut keys: Vec<_> = groups.keys().copied().collect();
        keys.sort_by_key(|k| (std::cmp::Reverse(k.2), k.0, k.1));
        let mut kept: Vec<([f64; 2], u8, f64)> = Vec::new();
        let e = EXTENT as f64;
        let keep = |minz: f64| z >= 16 || (z as f64 + 1.0) > minz;
        for k in keys {
            let idx = &groups[&k];
            let first = &recs[idx[0]];
            // a finer chart's copy stands from its own zoom; this one until then
            let finer = kept.iter().filter(|(q, b, _)| *b > first.band && metres(first.p, *q) < SAME_M).map(|(_, _, m)| *m).fold(f64::MAX, f64::min);
            if finer <= z as f64 {
                continue;
            }
            let mut minz = idx.iter().map(|&i| recs[i].minz).fold(f64::MAX, f64::min);
            // on a mark: appear no sooner than the first copy of the mark drawn here (a finer
            // chart may carry the buoy but not its light)
            let mark = marks.iter().filter(|(q, _, _)| metres(first.p, *q) < SAME_M).map(|(_, _, m)| *m).fold(f64::MAX, f64::min);
            if mark < f64::MAX {
                minz = minz.max(mark);
            }
            if !keep(minz) {
                continue;
            }
            kept.push((first.p, first.band, minz));
            let c = tp.px(&first.p);
            // the IHO portrayal of this light (symbols, and its sectors as S-52 draws them)
            let s52 = self.s52_table(first.t).and_then(|m| m.get(&first.fid).cloned()).filter(|e| !e.is_empty());
            let s52_sectors = s52.as_deref().map(crate::s52::sectors).unwrap_or_default();
            let lights: Vec<Light> = idx.iter().map(|&i| recs[i].light.clone()).collect();
            let cols: Vec<&str> = lights.iter().map(|l| s57::light_colour(&l.colours)).collect();
            let col = if cols.iter().all(|x| *x == cols[0]) { cols[0] } else { "m" };
            if c[0] >= 0.0 && c[0] < e && c[1] >= 0.0 && c[1] < e {
                let mut props = vec![
                    ("cls", Value::Str("LIGHTS".into())),
                    ("icon", Value::Str(format!("l|{col}"))),
                    ("t", Value::UInt(first.t as u64)),
                    ("fid", Value::UInt(first.fid.max(0) as u64)),
                    ("band", Value::UInt(first.band as u64)),
                    ("minz", Value::F64((minz * 10.0).round() / 10.0)),
                ];
                if finer < f64::MAX {
                    props.push(("maxz", Value::F64((finer * 10.0).round() / 10.0)));
                }
                if let Some(e) = &s52 {
                    props.push(("s5", Value::UInt(1)));
                    match crate::s52::entry_category(e) {
                        0 => {}
                        c => props.push(("vg", Value::UInt(c as u64))),
                    }
                    let (sy, si) = crate::s52::split_info(e);
                    if !sy.is_empty() {
                        props.push(("sy", Value::Str(sy.into())));
                    }
                    if !si.is_empty() {
                        props.push(("si", Value::Str(si.into())));
                    }
                }
                let lbl = s57::light_label(&lights);
                if !lbl.is_empty() {
                    props.push(("lbl", Value::Str(lbl)));
                }
                fpt.add(&TileGeom::Points(vec![[c[0].round() as i32, c[1].round() as i32]]), &props);
            }
            let mz = Value::F64((minz * 10.0).round() / 10.0);
            // no finer copy: shown at every zoom from minz
            let mx = Value::F64(if finer < f64::MAX { (finer * 10.0).round() / 10.0 } else { 99.0 });
            // S-52 sectors (drawn with the IHO symbols): legs dashed, 25 mm (or their charted
            // length), arcs 20 mm round the light in the light's colour over black, dashed thin
            // where the light is obscured
            const UNITS_PER_MM: f64 = 3.78 * EXTENT as f64 / 512.0;
            let at = |b: f64, r: f64| {
                let br = b.to_radians();
                [c[0] + r * br.sin(), c[1] - r * br.cos()]
            };
            for part in &s52_sectors {
                match part {
                    crate::s52::SectorPart::Leg { bearing, metres, mm } => {
                        let end = match metres {
                            Some(m) => {
                                let br = bearing.to_radians();
                                let q =
                                    [first.p[0] + m * br.sin() / (111_320.0 * first.p[1].to_radians().cos().max(0.05)), first.p[1] + m * br.cos() / 111_320.0];
                                tp.px(&q)
                            }
                            None => at(*bearing, mm * UNITS_PER_MM),
                        };
                        add_line(
                            lsec,
                            tp,
                            &[c, end],
                            &[("k", Value::Str("sdash".into())), ("tok", Value::Str("CHBLK".into())), ("minz", mz.clone()), ("maxz", mx.clone())],
                        );
                    }
                    crate::s52::SectorPart::Arc { radius, start, extent, styles } => {
                        let r = radius * UNITS_PER_MM;
                        let n = (extent.abs() / 3.0).ceil().max(2.0) as usize;
                        let arc: Vec<[f64; 2]> = (0..=n).map(|i| at(start + extent * i as f64 / n as f64, r)).collect();
                        for (dash, w, tok) in styles {
                            add_line(
                                lsec,
                                tp,
                                &arc,
                                &[
                                    ("k", Value::Str(if *dash { "sdash" } else { "sarc" }.into())),
                                    ("tok", Value::Str(tok.clone())),
                                    ("w", Value::F64(*w)),
                                    ("minz", mz.clone()),
                                    ("maxz", mx.clone()),
                                ],
                            );
                        }
                    }
                }
            }
            // sectors: limits are bearings from seaward, so the light shines the opposite way
            // (TweedPlot's own drawing; `s5`: the IHO one above stands in for it)
            for l in &lights {
                let Some((s1, s2)) = l.sectors else { continue };
                let (a0, mut a1) = ((s1 + 180.0) % 360.0, (s2 + 180.0) % 360.0);
                if a1 <= a0 {
                    a1 += 360.0;
                }
                if a1 - a0 >= 359.0 || a1 - a0 < 0.5 {
                    continue;
                }
                let at = |b: f64, r: f64| {
                    let br = b.to_radians();
                    [c[0] + r * br.sin(), c[1] - r * br.cos()]
                };
                let n = ((a1 - a0) / 3.0).ceil().max(2.0) as usize;
                let arc: Vec<[f64; 2]> = (0..=n).map(|i| at(a0 + (a1 - a0) * i as f64 / n as f64, ARC_R)).collect();
                let col = s57::light_colour(&l.colours);
                let mut pa = vec![("col", Value::Str(col.into())), ("k", Value::Str("arc".into())), ("minz", mz.clone()), ("maxz", mx.clone())];
                if !s52_sectors.is_empty() {
                    pa.push(("s5", Value::UInt(1)));
                }
                add_line(lsec, tp, &arc, &pa);
                for b in [a0, a1] {
                    let mut pl = pa.clone();
                    pl[1] = ("k", Value::Str("leg".into()));
                    add_line(lsec, tp, &[c, at(b, LEG_R)], &pl);
                }
            }
        }
    }

    /// Bridges, overhead cables and pipelines within `radius_m` of `at`
    /// ([lon, lat]) that give a vertical clearance: the nearest point of each,
    /// for an air-draft alarm. Clearance is the charted VERCLR (or the safe
    /// clearance VERCSA of a power line when it is lower), metres above HAT.
    pub fn clearances(&self, at: [f64; 2], radius_m: f64) -> Vec<Clearance> {
        let dlat = radius_m / 111_320.0;
        let dlon = dlat / at[1].to_radians().cos().max(0.05);
        let bbox = [at[0] - dlon, at[1] - dlat, at[0] + dlon, at[1] + dlat];
        let mut out: Vec<Clearance> = Vec::new();
        for t in self.tables.iter().filter(|t| t.kind == Kind::Feature && t.class.is_some_and(|c| matches!(c.code, "BRIDGE" | "CBLOHD" | "PIPOHD"))) {
            let class = t.class.unwrap();
            self.query_text(t, bbox, &["objnam", "verclr", "vercsa"], |row| {
                let num = |i: usize| row.vals[i].as_deref().and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| *v > 0.0);
                let Some(clear) = [num(1), num(2)].into_iter().flatten().reduce(f64::min) else {
                    return;
                };
                let pts: Vec<[f64; 2]> = match &row.geom {
                    Geom::Lines(ls) => ls.iter().flatten().copied().collect(),
                    Geom::Polygons(ps) => ps.iter().flatten().flatten().copied().collect(),
                    Geom::Points(p) => p.clone(),
                };
                // nearest point on the outline (segments), in local metres
                let k = at[1].to_radians().cos();
                let xy = |p: [f64; 2]| [(p[0] - at[0]) * 111_320.0 * k, (p[1] - at[1]) * 111_320.0];
                let mut best = (f64::INFINITY, [0.0, 0.0]);
                let segs: Vec<([f64; 2], [f64; 2])> = match &row.geom {
                    Geom::Lines(ls) => ls.iter().flat_map(|l| l.windows(2).map(|w| (w[0], w[1]))).collect(),
                    Geom::Polygons(ps) => ps.iter().flatten().flat_map(|r| r.windows(2).map(|w| (w[0], w[1]))).collect(),
                    Geom::Points(_) => pts.iter().map(|p| (*p, *p)).collect(),
                };
                for (a, b) in segs {
                    let (a, b) = (xy(a), xy(b));
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let l2 = dx * dx + dy * dy;
                    let u = if l2 > 0.0 { ((-a[0]) * dx + (-a[1]) * dy) / l2 } else { 0.0 }.clamp(0.0, 1.0);
                    let q = [a[0] + u * dx, a[1] + u * dy];
                    let d = q[0].hypot(q[1]);
                    if d < best.0 {
                        best = (d, q);
                    }
                }
                if best.0 > radius_m {
                    return;
                }
                let bearing = best.1[0].atan2(best.1[1]).to_degrees().rem_euclid(360.0);
                out.push(Clearance {
                    code: class.code,
                    class: class.name,
                    name: row.vals[0].clone().filter(|s| !s.trim().is_empty()),
                    clearance_m: clear,
                    distance_m: best.0,
                    bearing,
                });
            });
        }
        out.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
        out
    }

    /// All the attributes of one feature (table index and fid from the tile), in words.
    pub fn feature(&self, table: usize, fid: i64) -> Option<FeatureInfo> {
        let t = self.tables.get(table)?;
        let class = t.class?;
        let gcol = t.rtree.trim_start_matches(&format!("rtree_{}_", t.table)).to_string();
        let cols: Vec<&String> = t.cols.iter().filter(|c| **c != gcol.to_ascii_lowercase() && *c != "fid" && *c != "fidn" && **c != t.pk).collect();
        // the row's values in words, and its geometry
        let (vals, geom): (Vec<Option<String>>, Vec<u8>) = match &self.dbs[t.db] {
            crate::remote::Db::Remote(src, db) => {
                let names: Vec<&str> = cols.iter().map(|c| c.as_str()).collect();
                let r = src.feature(*db, &t.table, fid, &names)?;
                (r.vals.iter().map(plugin_api::Value::as_text).collect(), r.geom)
            }
            crate::remote::Db::Local(c) => {
                let conn = c.lock().unwrap_or_else(|e| e.into_inner());
                let sel: String = cols.iter().map(|c| format!("\"{c}\", ")).collect();
                let sql = format!("SELECT {sel}\"{gcol}\" FROM \"{}\" WHERE \"{}\" = ?1", t.table, t.pk);
                let mut st = conn.prepare(&sql).ok()?;
                let mut rows = st.query([fid]).ok()?;
                let r = rows.next().ok()??;
                let vals = (0..cols.len()).map(|i| r.get_ref(i).ok().and_then(text)).collect();
                let geom = r.get_ref(cols.len()).ok().and_then(|v| v.as_blob().ok()).map(<[u8]>::to_vec).unwrap_or_default();
                (vals, geom)
            }
        };
        let mut attrs = Vec::new();
        let mut raw: Vec<(String, String)> = Vec::new();
        for (c, v) in cols.iter().zip(vals) {
            let Some(v) = v else {
                continue;
            };
            if v.trim().is_empty() {
                continue;
            }
            attrs.push((s57::attr_name(c), s57::describe(c, &v)));
            raw.push((c.to_ascii_lowercase(), v));
        }
        // a light: its chart characteristic first ("Fl(4) W 30s 29m 14M")
        if class.code == "LIGHTS" {
            let vals: Vec<Option<String>> = s57::TILE_COLS.iter().map(|n| raw.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone())).collect();
            let lbl = s57::light_label(&[s57::Light::from_attrs(&s57::Attrs(&vals))]);
            if !lbl.is_empty() {
                attrs.insert(0, ("Characteristic".to_string(), lbl));
            }
        }
        if let Some(i) = attrs.iter().position(|(k, _)| k == "Name") {
            let n = attrs.remove(i);
            attrs.insert(0, n);
        }
        let position = parse_gpkg(&geom).and_then(|g| match g {
            Geom::Points(p) => p.first().copied(),
            Geom::Lines(l) => l.first().and_then(|x| x.first().copied()),
            Geom::Polygons(p) => p.first().and_then(|x| x.first()).and_then(|x| x.first().copied()),
        });
        Some(FeatureInfo { code: class.code, class_name: class.name, band: t.band, position, attrs })
    }

    /// Charted dangers in `bbox` a boat drawing `safety_depth` must keep off:
    /// rocks, wrecks and obstructions that dry, are awash, or have less water
    /// (or an unknown depth); and, if `farms`, marine farms. Points and polygons.
    pub fn for_each_danger(&self, bbox: [f64; 4], safety_depth: f64, dangers: bool, farms: bool, mut f: impl FnMut(&Geom)) {
        for t in self.tables.iter().filter(|t| t.kind == Kind::Feature) {
            let code = t.class.unwrap().code;
            let want = match code {
                "UWTROC" | "WRECKS" | "OBSTRN" => dangers,
                "MARCUL" => farms,
                _ => false,
            };
            if !want {
                continue;
            }
            self.query_text(t, bbox, TILE_COLS, |row| {
                let a = Attrs(&row.vals);
                if code == "MARCUL" || is_danger(code, &a, safety_depth) {
                    f(&row.geom);
                }
            });
        }
    }
}

/// Whether a rock, wreck or obstruction is a danger to a boat needing `safety_depth`.
pub fn is_danger(code: &str, a: &Attrs, safety_depth: f64) -> bool {
    if matches!(a.first("watlev"), Some(1 | 2 | 4 | 5 | 7)) {
        return true;
    }
    if let Some(d) = a.num("valsou") {
        return d < safety_depth;
    }
    match code {
        "WRECKS" => !matches!(a.first("catwrk"), Some(1 | 3)),
        // foul ground and fish havens are not a danger to surface navigation
        "OBSTRN" => !matches!(a.get("catobs").and_then(|s| s.split(',').next()).and_then(|s| s.trim().parse::<u32>().ok()), Some(5..=7)),
        _ => true,
    }
}

fn add_line(layer: &mut Layer, tp: &TileProj, pts: &[[f64; 2]], props: &[(&str, Value)]) {
    let mut out = Vec::new();
    for piece in clip_line(pts, tp.rect) {
        let q = quantise(&piece);
        if q.len() >= 2 {
            out.push(q);
        }
    }
    if !out.is_empty() {
        layer.add(&TileGeom::Lines(out), props);
    }
}

/// Rings of a polygon geometry (for callers that only want areas).
pub fn polygons(g: &Geom) -> &[Vec<Ring>] {
    match g {
        Geom::Polygons(p) => p,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hazard_coverage_clipping_starts_at_chart_scales() {
        assert!(!clip_coverage_for(true, 6));
        assert!(clip_coverage_for(true, HAZARD_CLIP_MIN_ZOOM));
        assert!(clip_coverage_for(false, 0));
    }

    #[test]
    fn coverage_index_returns_only_intersecting_bounds() {
        let polygon = geo::Polygon::new(geo::LineString::from(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)]), vec![]);
        let index = CoverageIndex::new(vec![
            (2, [0.0, 0.0, 1.0, 1.0], polygon.clone()),
            (3, [10.0, 10.0, 11.0, 11.0], polygon.clone()),
            (4, [1.5, 1.5, 2.5, 2.5], polygon),
        ]);
        let mut found: Vec<(u8, [f64; 4])> = index.intersecting([0.5, 0.5, 2.0, 2.0]).map(|(band, bounds, _)| (*band, *bounds)).collect();
        found.sort_by_key(|(band, _)| *band);
        assert_eq!(found, vec![(2, [0.0, 0.0, 1.0, 1.0]), (4, [1.5, 1.5, 2.5, 2.5])]);
    }

    #[test]
    fn feature_bounds_tree_returns_only_intersecting_copies() {
        let index = bounds_tree([([0.0, 0.0, 1.0, 1.0], (2, 4.0)), ([10.0, 10.0, 11.0, 11.0], (3, 5.0)), ([1.5, 1.5, 2.5, 2.5], (4, 6.0))]);
        let mut found: Vec<(u8, f64)> = index.locate_in_envelope_intersecting(&bounds_envelope([0.5, 0.5, 2.0, 2.0])).map(|copy| copy.data).collect();
        found.sort_by_key(|a| a.0);
        assert_eq!(found, vec![(2, 4.0), (4, 6.0)]);
    }

    #[test]
    fn cached_coverage_clip_matches_uncached_clip() {
        let ring = |x0, y0, x1, y1| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]];
        let cover = geo::MultiPolygon::new(vec![geo::Polygon::new(geo::LineString::from(ring(0.0, 0.0, 1.0, 1.0)), vec![])]);
        let geom = Geom::Polygons(vec![vec![ring(-0.5, -0.5, 1.5, 1.5)]]);
        let cached = CoverageClip::new(&cover);
        assert_eq!(cut_inside_cached(&geom, &cached), clip_geom(&geom, &cover, false));
        assert_eq!(cut_outside_cached(&geom, &cached), cut_outside(&geom, &cover));
    }

    fn attrs(pairs: &[(&str, &str)]) -> Vec<Option<String>> {
        TILE_COLS.iter().map(|c| pairs.iter().find(|(k, _)| k == c).map(|(_, v)| v.to_string())).collect()
    }

    #[test]
    fn dangers() {
        let v = attrs(&[("watlev", "4")]);
        assert!(is_danger("UWTROC", &Attrs(&v), 3.0));
        let v = attrs(&[("watlev", "3"), ("valsou", "12")]);
        assert!(!is_danger("UWTROC", &Attrs(&v), 3.0));
        let v = attrs(&[("watlev", "3"), ("valsou", "1.8")]);
        assert!(is_danger("UWTROC", &Attrs(&v), 3.0));
        let v = attrs(&[("catwrk", "1")]);
        assert!(!is_danger("WRECKS", &Attrs(&v), 3.0));
        let v = attrs(&[("catobs", "7")]);
        assert!(!is_danger("OBSTRN", &Attrs(&v), 3.0));
        let v = attrs(&[]);
        assert!(is_danger("UWTROC", &Attrs(&v), 3.0));
    }
}

/// A charted vertical clearance near the boat (see [`ChartStore::clearances`]).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Clearance {
    pub code: &'static str,
    pub class: &'static str,
    pub name: Option<String>,
    pub clearance_m: f64,
    pub distance_m: f64,
    /// True bearing from the boat to the nearest point.
    pub bearing: f64,
}
