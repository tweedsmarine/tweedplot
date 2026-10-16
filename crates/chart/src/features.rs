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
//! 150 m, and lines and areas come from the finest band with any in the tile.

use std::collections::HashMap;

use crate::clip::{clip_line, quantise};
use crate::geom::{parse_gpkg, Geom, Ring};
use crate::mvt::{Layer, TileGeom, Value, EXTENT};
use crate::s57::{self, Attrs, Geo, Light, TILE_COLS};
use crate::{band_minzoom, ChartStore, Kind, TableInfo, TileProj};

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
        let conn = self.dbs[t.db].lock().unwrap();
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
        let Ok(mut st) = conn.prepare_cached(&sql) else { return };
        let Ok(mut rows) = st.query(rusqlite::params![b[0], b[2], b[1], b[3]]) else { return };
        while let Ok(Some(r)) = rows.next() {
            let fid: i64 = r.get(0).unwrap_or(0);
            let vals: Vec<Option<String>> = (0..cols.len()).map(|i| r.get_ref(i + 1).ok().and_then(text)).collect();
            let Ok(blob) = r.get_ref(cols.len() + 1).and_then(|v| v.as_blob().map_err(Into::into)) else { continue };
            if let Some(geom) = parse_gpkg(blob) {
                f(Row { fid, vals, geom });
            }
        }
    }

    fn feature_tables(&self, z: u8) -> Vec<(usize, &TableInfo)> {
        self.tables.iter().enumerate().filter(|(_, t)| t.kind == Kind::Feature && z >= band_minzoom(t.band)).collect()
    }

    /// The four feature layers of one tile: [areas, lines, points, light sectors].
    pub(crate) fn render_features(&self, tp: &TileProj, z: u8) -> [Layer; 4] {
        let mut far = Layer::new("far");
        let mut fln = Layer::new("fln");
        let mut fpt = Layer::new("fpt");
        let mut lsec = Layer::new("lsec");
        let bbox = tp.lonlat_bbox(crate::BUFFER);
        let all = self.feature_tables(z);
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
        let keep = |minz: f64| z >= 16 || (z as f64 + 1.0) > minz;
        let e = EXTENT as f64;
        let inside = |p: [f64; 2]| p[0] >= 0.0 && p[0] < e && p[1] >= 0.0 && p[1] < e;

        for (class, tables) in &groups {
            match class.geo {
                Geo::Point if class.code == "LIGHTS" => self.render_lights(tp, z, tables, &mut fpt, &mut lsec),
                Geo::Point => {
                    let mut taken: Vec<([f64; 2], u8)> = Vec::new();
                    for (ti, t) in tables {
                        let mut mine = Vec::new();
                        self.query_text(t, bbox, TILE_COLS, |row| {
                            let Geom::Points(pts) = &row.geom else { return };
                            let a = Attrs(&row.vals);
                            for &p in pts {
                                if taken.iter().any(|(q, b)| *b > t.band && metres(p, *q) < SAME_M) {
                                    continue;
                                }
                                mine.push((p, t.band));
                                let minz = a.num("scamin").map_or(0.0, |s| s57::scamin_zoom(s, p[1]));
                                if !keep(minz) {
                                    continue;
                                }
                                let px = tp.px(&p);
                                if !inside(px) {
                                    continue;
                                }
                                let Some(icon) = s57::icon(class.code, &a) else { continue };
                                let mut props = vec![
                                    ("cls", Value::Str(class.code.into())),
                                    ("icon", Value::Str(icon)),
                                    ("t", Value::UInt(*ti as u64)),
                                    ("fid", Value::UInt(row.fid.max(0) as u64)),
                                    ("band", Value::UInt(t.band as u64)),
                                    ("minz", Value::F64((minz * 10.0).round() / 10.0)),
                                ];
                                if let Some(l) = s57::label(class.code, &a) {
                                    props.push(("lbl", Value::Str(l)));
                                }
                                if let Some(n) = a.get("objnam") {
                                    props.push(("name", Value::Str(n.into())));
                                }
                                fpt.add(&TileGeom::Points(vec![[px[0].round() as i32, px[1].round() as i32]]), &props);
                            }
                        });
                        taken.extend(mine);
                    }
                }
                Geo::Line | Geo::Area => {
                    let layer = if class.geo == Geo::Line { &mut fln } else { &mut far };
                    for (ti, t) in tables {
                        let before = layer.len();
                        self.query_text(t, bbox, &["objnam", "scamin", "catrea", "restrn", "catmfa", "orient"], |row| {
                            let tg = if class.geo == Geo::Line { tp.lines(&row.geom) } else { tp.polygons(&row.geom) };
                            let Some(tg) = tg else { return };
                            let lat = (bbox[1] + bbox[3]) / 2.0;
                            let minz = row.vals[1].as_deref().and_then(|s| s.parse().ok()).map_or(0.0, |s| s57::scamin_zoom(s, lat));
                            if !keep(minz) {
                                return;
                            }
                            let mut props = vec![
                                ("cls", Value::Str(class.code.into())),
                                ("t", Value::UInt(*ti as u64)),
                                ("fid", Value::UInt(row.fid.max(0) as u64)),
                                ("band", Value::UInt(t.band as u64)),
                                ("minz", Value::F64((minz * 10.0).round() / 10.0)),
                            ];
                            if let Some(n) = row.vals[0].as_deref().filter(|s| !s.trim().is_empty()) {
                                props.push(("name", Value::Str(n.trim().into())));
                            }
                            // restrictions that matter to a yacht: entry prohibited / restricted, area to be avoided
                            if let Some(r) = row.vals[3].as_deref() {
                                if r.split(',').any(|c| matches!(c.trim(), "7" | "8" | "14")) {
                                    props.push(("entry", Value::UInt(1)));
                                }
                            }
                            layer.add(&tg, &props);
                        });
                        if layer.len() > before {
                            break;
                        }
                    }
                }
            }
        }
        [far, fln, fpt, lsec]
    }

    fn render_lights(&self, tp: &TileProj, z: u8, tables: &[(usize, &TableInfo)], fpt: &mut Layer, lsec: &mut Layer) {
        // lights a little outside the tile still throw sectors into it
        let bbox = tp.lonlat_bbox(LEG_R + 16.0);
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
        let mut kept: Vec<([f64; 2], u8)> = Vec::new();
        let e = EXTENT as f64;
        let keep = |minz: f64| z >= 16 || (z as f64 + 1.0) > minz;
        for k in keys {
            let idx = &groups[&k];
            let first = &recs[idx[0]];
            if kept.iter().any(|(q, b)| *b > first.band && metres(first.p, *q) < SAME_M) {
                continue;
            }
            kept.push((first.p, first.band));
            let minz = idx.iter().map(|&i| recs[i].minz).fold(f64::MAX, f64::min);
            if !keep(minz) {
                continue;
            }
            let c = tp.px(&first.p);
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
                let lbl = s57::light_label(&lights);
                if !lbl.is_empty() {
                    props.push(("lbl", Value::Str(lbl)));
                }
                fpt.add(&TileGeom::Points(vec![[c[0].round() as i32, c[1].round() as i32]]), &props);
            }
            // sectors: limits are bearings from seaward, so the light shines the opposite way
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
                let mz = Value::F64((minz * 10.0).round() / 10.0);
                add_line(lsec, tp, &arc, &[("col", Value::Str(col.into())), ("k", Value::Str("arc".into())), ("minz", mz.clone())]);
                for b in [a0, a1] {
                    add_line(lsec, tp, &[c, at(b, LEG_R)], &[("col", Value::Str(col.into())), ("k", Value::Str("leg".into())), ("minz", mz.clone())]);
                }
            }
        }
    }

    /// All the attributes of one feature (table index and fid from the tile), in words.
    pub fn feature(&self, table: usize, fid: i64) -> Option<FeatureInfo> {
        let t = self.tables.get(table)?;
        let class = t.class?;
        let conn = self.dbs[t.db].lock().unwrap();
        let gcol = t.rtree.trim_start_matches(&format!("rtree_{}_", t.table)).to_string();
        let cols: Vec<&String> = t.cols.iter().filter(|c| **c != gcol.to_ascii_lowercase() && *c != "fid" && *c != "fidn" && **c != t.pk).collect();
        let sel: String = cols.iter().map(|c| format!("\"{c}\", ")).collect();
        let sql = format!("SELECT {sel}\"{gcol}\" FROM \"{}\" WHERE \"{}\" = ?1", t.table, t.pk);
        let mut st = conn.prepare(&sql).ok()?;
        let mut rows = st.query([fid]).ok()?;
        let r = rows.next().ok()??;
        let mut attrs = Vec::new();
        for (i, c) in cols.iter().enumerate() {
            let Some(v) = r.get_ref(i).ok().and_then(text) else { continue };
            if v.trim().is_empty() {
                continue;
            }
            attrs.push((s57::attr_name(c), s57::describe(c, &v)));
        }
        if let Some(i) = attrs.iter().position(|(k, _)| k == "Name") {
            let n = attrs.remove(i);
            attrs.insert(0, n);
        }
        let position = r.get_ref(cols.len()).ok().and_then(|v| v.as_blob().ok()).and_then(parse_gpkg).and_then(|g| match g {
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
        "OBSTRN" => !matches!(a.get("catobs").and_then(|s| s.split(',').next()).and_then(|s| s.trim().parse::<u32>().ok()), Some(5 | 6 | 7)),
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
