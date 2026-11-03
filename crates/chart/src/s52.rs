//! IHO S-52 / S-101 portrayal of the chart's features: each feature table is run
//! through the IHO S-101 portrayal rules (crate `portrayal`) the first time a tile
//! needs it, and the symbols the rules choose go into the tiles as `sy` on point
//! features: symbol names, separated by commas, each with `@angle` when the rules turn
//! it ("BOYPIL24,TOPMAR06", "LIGHTS11@135"). The app draws them from the catalogue's SVGs.
//!
//! Results are kept per table until the context (safety contour...) changes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use portrayal::{Converter, Geometry, Portrayer, S57Object};

use crate::geom::Geom;
use crate::{features::DepthAreas, ChartStore};

pub(crate) type TableSymbols = Arc<HashMap<i64, Arc<str>>>;

/// The IHO portrayal: the rules, the context TweedPlot has set, and the results.
///
/// A context change (safety contour...) doesn't drop the results: the chart keeps the
/// symbols it has until a fresh run has done every table, then swaps them in at once.
/// A run is abandoned as soon as the context changes again, so a burst of changes (the
/// arrows of a number box) costs one run, not one per click.
pub struct S52 {
    portrayer: RwLock<Portrayer>,
    /// The context as TweedPlot wants it, applied at the start of each run.
    context: Mutex<Vec<(String, String)>>,
    /// Raised by every context change.
    generation: AtomicU64,
    /// The results, and the generation they were made for.
    done: RwLock<(u64, Arc<HashMap<usize, TableSymbols>>)>,
    /// One run at a time.
    running: Mutex<()>,
}

impl S52 {
    /// The portrayal catalogue folder (`s101-pc`) and the feature catalogue XML.
    pub fn open(pc: &std::path::Path, fc: &std::path::Path) -> Result<Self, String> {
        let cat = portrayal::Catalogue::open(pc).map_err(|e| e.to_string())?;
        let text = std::fs::read_to_string(fc).map_err(|e| format!("{}: {e}", fc.display()))?;
        let fc = portrayal::FeatureCatalogue::parse(&text)?;
        let p = Portrayer::new(&cat, fc).map_err(|e| e.to_string())?;
        let s = S52 {
            portrayer: RwLock::new(p),
            context: Mutex::new(Vec::new()),
            generation: AtomicU64::new(1),
            done: RwLock::new((0, Arc::new(HashMap::new()))),
            running: Mutex::new(()),
        };
        // TweedPlot's defaults where they differ from the catalogue's: isolated dangers in
        // shallow water (the catalogue has it on) is a mariner's choice, off to start with
        s.set("ShallowWaterDangers", "false");
        Ok(s)
    }

    /// Set a context parameter ("SafetyContour", "5"). True when it changed: the next
    /// run portrays everything again. Never waits for a run.
    pub fn set(&self, id: &str, value: &str) -> bool {
        let mut c = self.context.lock().unwrap_or_else(|e| e.into_inner());
        if c.iter().any(|(k, v)| k == id && v == value) {
            return false;
        }
        c.retain(|(k, _)| k != id);
        c.push((id.to_string(), value.to_string()));
        self.generation.fetch_add(1, Ordering::SeqCst);
        true
    }
}

/// The symbols in a feature's drawing instructions, as `sy` (see the module notes).
pub fn point_symbols(instructions: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut rot = 0.0f64;
    for part in instructions.split(';') {
        let (k, v) = part.split_once(':').unwrap_or((part, ""));
        match k {
            "Rotation" => {
                // Rotation:PortrayalCRS,135 (screen) or GeographicCRS,…
                rot = v.split(',').nth(1).and_then(|a| a.trim().parse().ok()).unwrap_or(0.0);
            }
            "PointInstruction" if !v.is_empty() => {
                if rot.abs() > 1e-6 {
                    out.push(format!("{v}@{}", rot.round() as i64));
                } else {
                    out.push(v.to_string());
                }
            }
            _ => {}
        }
    }
    out.join(",")
}

fn geometry(g: &Geom) -> Option<Geometry> {
    match g {
        Geom::Points(p) => p.first().map(|p| Geometry::Point(*p)),
        Geom::Lines(l) => l.first().map(|l| Geometry::Curve(l.clone())),
        Geom::Polygons(p) => p.first().map(|r| Geometry::Surface(r.clone())),
    }
}

fn key(p: [f64; 2]) -> (i64, i64) {
    ((p[0] * 1e5).round() as i64, (p[1] * 1e5).round() as i64)
}

impl ChartStore {
    /// Turn on the IHO portrayal (or off with None). Drops the rendered tiles.
    pub fn set_s52(&self, s52: Option<S52>) {
        *self.s52.write().unwrap_or_else(|e| e.into_inner()) = s52.map(Arc::new);
        self.clear_cache();
    }

    /// Set a portrayal context parameter ("SafetyContour", "SafetyDepth",
    /// "ShallowWaterDangers", ...). True when it changed: then [`s52_warm`](Self::s52_warm)
    /// again. The chart keeps its symbols until that run is done.
    pub fn set_s52_context(&self, id: &str, value: &str) -> bool {
        let s = self.s52.read().unwrap_or_else(|e| e.into_inner()).clone();
        s.is_some_and(|s| s.set(id, value))
    }

    /// A table's `sy` by fid; None when the portrayal is off or hasn't run yet
    /// ([`s52_warm`](Self::s52_warm) does that, in the background): a tile never waits
    /// for it, it is drawn again when the portrayal is ready.
    pub(crate) fn s52_table(&self, table: usize) -> Option<TableSymbols> {
        let s = self.s52.read().unwrap_or_else(|e| e.into_inner()).clone()?;
        let done = s.done.read().unwrap_or_else(|e| e.into_inner());
        done.1.get(&table).cloned()
    }

    /// Whether the portrayal is on.
    pub fn s52_on(&self) -> bool {
        self.s52.read().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Portray every point feature table for the current context (in parallel), swap the
    /// results in and drop the rendered tiles so they are drawn with them. Returns the
    /// number of tables portrayed: 0 when the results are already current, or when the
    /// context changed during the run (the run is abandoned; the caller of that change
    /// runs again). `progress(done, total)`.
    pub fn s52_warm(&self, progress: &(dyn Fn(usize, usize) + Sync)) -> usize {
        use rayon::prelude::*;
        let Some(s) = self.s52.read().unwrap_or_else(|e| e.into_inner()).clone() else { return 0 };
        let _one = s.running.lock().unwrap_or_else(|e| e.into_inner());
        let generation = s.generation.load(Ordering::SeqCst);
        if s.done.read().unwrap_or_else(|e| e.into_inner()).0 == generation {
            return 0;
        }
        {
            let ctx = s.context.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let mut p = s.portrayer.write().unwrap_or_else(|e| e.into_inner());
            for (k, v) in &ctx {
                p.set(k, v);
            }
        }
        let todo: Vec<usize> = self
            .tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == crate::Kind::Feature && t.class.is_some_and(|c| c.geo == crate::s57::Geo::Point && c.code != "TOPMAR"))
            .map(|(i, _)| i)
            .collect();
        let total = todo.len();
        let n = std::sync::atomic::AtomicUsize::new(0);
        let stale = || s.generation.load(Ordering::SeqCst) != generation;
        let out: Vec<(usize, TableSymbols)> = todo
            .par_iter()
            .filter_map(|&ti| {
                if stale() {
                    return None;
                }
                let r = Arc::new(self.portray_table(&s, ti));
                progress(n.fetch_add(1, Ordering::Relaxed) + 1, total);
                Some((ti, r))
            })
            .collect();
        if stale() {
            return 0;
        }
        *s.done.write().unwrap_or_else(|e| e.into_inner()) = (generation, Arc::new(out.into_iter().collect()));
        self.clear_cache();
        total.max(1)
    }

    /// Run one table's features through the rules: fid → `sy`.
    fn portray_table(&self, s: &S52, ti: usize) -> HashMap<i64, Arc<str>> {
        let t0 = std::time::Instant::now();
        let t = &self.tables[ti];
        let Some(class) = t.class else { return HashMap::new() };
        let p = s.portrayer.read().unwrap_or_else(|e| e.into_inner());
        let conv = Converter::new(p.feature_catalogue());
        let gcol = t.rtree.trim_start_matches(&format!("rtree_{}_", t.table)).to_string();
        let cols: Vec<&str> = t.cols.iter().map(String::as_str).filter(|c| *c != gcol && *c != t.pk && *c != "fid").collect();
        let mut rows: Vec<(i64, S57Object)> = Vec::new();
        self.query_text(t, t.bounds, &cols, |row| {
            let attrs: Vec<(String, String)> = cols.iter().zip(row.vals).filter_map(|(c, v)| Some((c.to_ascii_uppercase(), v?))).collect();
            let cls = if class.code == crate::s57::UNKNOWN {
                attrs.iter().find(|(k, _)| k == "OBJL").map(|(_, v)| v.clone()).unwrap_or_default()
            } else {
                class.code.to_string()
            };
            rows.push((row.fid, S57Object { class: cls, attrs, geometry: geometry(&row.geom) }));
        });
        if rows.is_empty() {
            return HashMap::new();
        }
        let pos = |o: &S57Object| match &o.geometry {
            Some(Geometry::Point(p)) => Some(*p),
            _ => None,
        };
        // topmarks of the same band, for buoys and beacons
        let mut topmarks: HashMap<(i64, i64), S57Object> = HashMap::new();
        if class.code.starts_with("BOY") || class.code.starts_with("BCN") {
            for tt in self.tables.iter().filter(|x| x.band == t.band && x.class.is_some_and(|c| c.code == "TOPMAR")) {
                let tcols: Vec<&str> = tt.cols.iter().map(String::as_str).filter(|c| matches!(*c, "topshp" | "colour" | "colpat")).collect();
                self.query_text(tt, t.bounds, &tcols, |row| {
                    if let Geom::Points(p) = &row.geom {
                        if let Some(p) = p.first() {
                            let attrs = tcols.iter().zip(row.vals).filter_map(|(c, v)| Some((c.to_ascii_uppercase(), v?))).collect();
                            topmarks.insert(key(*p), S57Object { class: "TOPMAR".into(), attrs, geometry: None });
                        }
                    }
                });
            }
        }
        // the water around dangers (S-101 surroundingDepth), from the band's depth areas,
        // read a cell of about 20 km at a time
        const C: f64 = 0.2;
        let mut depths: HashMap<i64, Option<f64>> = HashMap::new();
        if matches!(class.code, "UWTROC" | "WRECKS" | "OBSTRN") {
            let mut cells: HashMap<(i64, i64), Vec<(i64, [f64; 2])>> = HashMap::new();
            for (fid, o) in &rows {
                if let Some(p) = pos(o) {
                    cells.entry(((p[0] / C).floor() as i64, (p[1] / C).floor() as i64)).or_default().push((*fid, p));
                }
            }
            for ((cx, cy), pts) in cells {
                let b = [cx as f64 * C - 0.03, cy as f64 * C - 0.03, (cx + 1) as f64 * C + 0.03, (cy + 1) as f64 * C + 0.03];
                let da = DepthAreas::load_band(self, t.band, b);
                for (fid, p) in pts {
                    depths.insert(fid, da.around(t.band, p));
                }
            }
        }
        // features; a sector light's records (one per sector) at one place as one feature
        let mut features = Vec::new();
        let mut same: HashMap<String, Vec<i64>> = HashMap::new();
        if class.code == "LIGHTS" {
            let mut groups: HashMap<(i64, i64), Vec<(i64, S57Object)>> = HashMap::new();
            let mut others = Vec::new();
            for (fid, o) in rows {
                let sectored = conv.feature_type(&o).is_some_and(|f| f.code == "LightSectored");
                match (sectored, pos(&o)) {
                    (true, Some(p)) => groups.entry(key(p)).or_default().push((fid, o)),
                    _ => others.push((fid, o)),
                }
            }
            for (_, g) in groups {
                let id = g[0].0.to_string();
                same.insert(id.clone(), g.iter().map(|(f, _)| *f).collect());
                let recs: Vec<S57Object> = g.into_iter().map(|(_, o)| o).collect();
                features.push(conv.sectored_light(&id, &recs));
            }
            rows = others;
        }
        for (fid, o) in &rows {
            let mut f = conv.convert(&fid.to_string(), o, depths.get(fid).copied().flatten());
            if let Some(tm) = pos(o).and_then(|p| topmarks.get(&key(p))) {
                conv.add_topmark(&mut f, tm);
            }
            features.push(f);
        }
        let mut trace = Vec::new();
        let out = match p.portray(&features, &mut trace) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("charts: portrayal of {} failed: {e}", t.table);
                return HashMap::new();
            }
        };
        let mut map = HashMap::new();
        for o in out {
            let sy: Arc<str> = point_symbols(&o.instructions).into();
            match same.get(&o.id) {
                Some(fids) => {
                    for f in fids {
                        map.insert(*f, sy.clone());
                    }
                }
                None => {
                    if let Ok(f) = o.id.parse::<i64>() {
                        map.insert(f, sy);
                    }
                }
            }
        }
        if std::env::var_os("TWEEDPLOT_TILE_LOG").is_some() || t0.elapsed().as_secs_f64() > 1.0 {
            eprintln!(
                "charts: portrayed {} ({} features) in {:.1} s{}",
                t.table,
                map.len(),
                t0.elapsed().as_secs_f64(),
                if trace.is_empty() { String::new() } else { format!(", {} rule messages", trace.len()) }
            );
        }
        map
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn symbols_from_instructions() {
        let s = super::point_symbols("ViewingGroup:27010;PointInstruction:BOYPIL24;ViewingGroup:27010,27050;PointInstruction:TOPMAR06");
        assert_eq!(s, "BOYPIL24,TOPMAR06");
        let l = super::point_symbols("Hover:true;Rotation:PortrayalCRS,135;PointInstruction:LIGHTS11;Rotation:PortrayalCRS,0;FontColor:CHBLK");
        assert_eq!(l, "LIGHTS11@135");
    }
}
