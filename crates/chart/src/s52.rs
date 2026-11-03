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
    /// The catalogue: line styles' heights for the tiles.
    cat: portrayal::Catalogue,
    heights: Mutex<HashMap<String, f64>>,
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
            cat,
            heights: Mutex::new(HashMap::new()),
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

/// The symbols in a feature's drawing instructions, as `sy` (see the module notes), then
/// after a `|` the information symbols (the "i" of a feature with notes, INFORM01, and the
/// like: viewing groups 90020-90022, which an ECDIS shows only when asked), if any.
pub fn point_symbols(instructions: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut info: Vec<String> = Vec::new();
    let mut rot = 0.0f64;
    let mut in_info = false;
    for part in instructions.split(';') {
        let (k, v) = part.split_once(':').unwrap_or((part, ""));
        match k {
            "ViewingGroup" => in_info = v.split(',').any(|g| matches!(g.trim(), "90020" | "90021" | "90022")),
            "Rotation" => {
                // Rotation:PortrayalCRS,135 (screen) or GeographicCRS,…
                rot = v.split(',').nth(1).and_then(|a| a.trim().parse().ok()).unwrap_or(0.0);
            }
            "PointInstruction" if !v.is_empty() => {
                let s = if rot.abs() > 1e-6 { format!("{v}@{}", rot.round() as i64) } else { v.to_string() };
                if in_info {
                    info.push(s);
                } else {
                    out.push(s);
                }
            }
            _ => {}
        }
    }
    if info.is_empty() {
        out.join(",")
    } else {
        format!("{}|{}", out.join(","), info.join(","))
    }
}

/// `sy` and the information symbols (`si`) from [`point_symbols`].
pub(crate) fn split_info(s: &str) -> (&str, &str) {
    s.split_once('|').unwrap_or((s, ""))
}

/// What the rules draw for a line or an area, as tile properties (the module notes):
/// `sl` its line style (a catalogue name, or `_interval_width_colour[_start-length...]`
/// for a simple line), `slw` how tall the line is drawn (mm), `cf` and `ct` its colour
/// fill (token, transparency), `af` its area fill pattern, `sy` the symbols at its middle,
/// `vg` its display category from the rules' viewing groups (1 base, 2 standard, 3 other).
#[derive(Debug, Default, PartialEq)]
pub struct Drawn {
    pub line: Option<(String, portrayal::LineStyle)>,
    pub fill: Option<(String, f64)>,
    pub pattern: Option<String>,
    pub symbols: String,
    pub category: u8,
}

pub fn drawn(instructions: &str, cat: &portrayal::Catalogue) -> Drawn {
    let mut d = Drawn { symbols: point_symbols(instructions), ..Default::default() };
    let mut dashes: Vec<(f64, f64)> = Vec::new();
    let mut simple: Option<portrayal::LineStyle> = None;
    for part in instructions.split(';') {
        let (k, v) = part.split_once(':').unwrap_or((part, ""));
        let nums = |v: &str| -> Vec<f64> { v.split(',').map(|x| x.trim().parse().unwrap_or(0.0)).collect() };
        match k {
            "ViewingGroup" => {
                for g in v.split(',').filter_map(|g| g.trim().parse::<u32>().ok()).filter(|g| *g >= 10000) {
                    let c = (g / 10000).min(3) as u8;
                    if d.category == 0 || c < d.category {
                        d.category = c;
                    }
                }
            }
            "Dash" => {
                let n = nums(v);
                if n.len() == 2 {
                    dashes.push((n[0], n[1]));
                }
            }
            "LineStyle" => {
                // _simple_,interval,width,colour
                let f: Vec<&str> = v.split(',').collect();
                if f.len() == 4 {
                    let interval = f[1].trim().parse().unwrap_or(0.0);
                    let width = f[2].trim().parse().unwrap_or(0.32);
                    simple = Some(portrayal::LineStyle::simple(interval, width, f[3].trim(), std::mem::take(&mut dashes)));
                }
            }
            "LineInstruction" if d.line.is_none() => {
                if v == "_simple_" {
                    if let Some(s) = simple.take() {
                        let mut name = format!("_{}_{}_{}", s.interval, s.width, s.colour);
                        if s.interval > 0.0 {
                            for (a, b) in &s.dashes {
                                name.push_str(&format!("_{a}-{b}"));
                            }
                        }
                        d.line = Some((name, s));
                    }
                } else if let Some(s) = cat.line_style(v) {
                    d.line = Some((v.to_string(), s));
                }
                dashes.clear();
            }
            "ColorFill" if d.fill.is_none() => {
                let mut f = v.split(',');
                let token = f.next().unwrap_or("").trim().to_string();
                let tr = f.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
                if !token.is_empty() {
                    d.fill = Some((token, tr));
                }
            }
            "AreaFillReference" if d.pattern.is_none() && !v.is_empty() => d.pattern = Some(v.to_string()),
            _ => {}
        }
    }
    d
}

impl S52 {
    /// How tall lines in this style are drawn (mm).
    fn height(&self, name: &str, style: &portrayal::LineStyle) -> f64 {
        let mut h = self.heights.lock().unwrap_or_else(|e| e.into_inner());
        *h.entry(name.to_string()).or_insert_with(|| self.cat.line_height(style))
    }

    /// A line or area feature's tile properties (see [`Drawn`]), joined with `;`.
    fn encode(&self, d: &Drawn) -> String {
        let mut out: Vec<String> = Vec::new();
        if let Some((name, style)) = &d.line {
            out.push(format!("sl={name}"));
            out.push(format!("slw={}", (self.height(name, style) * 100.0).round() / 100.0));
        }
        if let Some((token, tr)) = &d.fill {
            out.push(format!("cf={token}"));
            if *tr > 0.0 {
                out.push(format!("ct={tr}"));
            }
        }
        if let Some(p) = &d.pattern {
            out.push(format!("af={p}"));
        }
        let (sy, si) = split_info(&d.symbols);
        if !sy.is_empty() {
            out.push(format!("sy={sy}"));
        }
        if !si.is_empty() {
            out.push(format!("si={si}"));
        }
        if d.category > 0 {
            out.push(format!("vg={}", d.category));
        }
        out.join(";")
    }

    /// The catalogue.
    pub fn catalogue(&self) -> &portrayal::Catalogue {
        &self.cat
    }
}

/// A line or area feature's tile properties from what [`S52::encode`] made.
pub(crate) fn props(encoded: &str) -> impl Iterator<Item = (&'static str, &str)> {
    encoded.split(';').filter_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        let k = match k {
            "sl" => "sl",
            "slw" => "slw",
            "cf" => "cf",
            "ct" => "ct",
            "af" => "af",
            "sy" => "sy",
            "si" => "si",
            "vg" => "vg",
            _ => return None,
        };
        Some((k, v))
    })
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
            .filter(|(_, t)| t.kind == crate::Kind::Feature && t.class.is_some_and(|c| c.code != "TOPMAR" && c.code != "SEAARE" && c.code != "LNDRGN"))
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
            // an area: the water at its middle
            let anchor = |o: &S57Object| match &o.geometry {
                Some(Geometry::Surface(rings)) => {
                    let pts = rings.first()?;
                    let (mut b0, mut b1) = ([f64::MAX; 2], [f64::MIN; 2]);
                    for q in pts {
                        b0 = [b0[0].min(q[0]), b0[1].min(q[1])];
                        b1 = [b1[0].max(q[0]), b1[1].max(q[1])];
                    }
                    (b0[0] <= b1[0]).then(|| [(b0[0] + b1[0]) / 2.0, (b0[1] + b1[1]) / 2.0])
                }
                _ => pos(o),
            };
            for (fid, o) in &rows {
                if let Some(p) = anchor(o) {
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
            let sy: Arc<str> =
                if class.geo == crate::s57::Geo::Point { point_symbols(&o.instructions).into() } else { s.encode(&drawn(&o.instructions, &s.cat)).into() };
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
        let i = super::point_symbols("ViewingGroup:27070;PointInstruction:LIGHTS13;ViewingGroup:27070,90020;DrawingPriority:24;PointInstruction:INFORM01");
        assert_eq!(super::split_info(&i), ("LIGHTS13", "INFORM01"));
    }

    #[test]
    fn lines_and_areas_from_instructions() {
        let cat = portrayal::Catalogue::open(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources/s101-pc")).unwrap();
        let r = super::drawn("ScaleMinimum:119999;ViewingGroup:26010;DrawingPriority:15;PointInstruction:INFARE51;Dash:0,3.6;LineStyle:_simple_,5.4,0.64,CHMGD;LineInstruction:_simple_;LocalOffset:0,0", &cat);
        assert_eq!(r.line.as_ref().unwrap().0, "_5.4_0.64_CHMGD_0-3.6");
        assert_eq!(r.symbols, "INFARE51");
        assert_eq!(r.category, 2);
        let u = super::drawn("ViewingGroup:11050;ColorFill:NODTA;AreaFillReference:NODATA03;LineStyle:_simple_,,0.64,CHGRD;LineInstruction:_simple_", &cat);
        assert_eq!(u.fill, Some(("NODTA".into(), 0.0)));
        assert_eq!(u.pattern.as_deref(), Some("NODATA03"));
        assert_eq!(u.line.as_ref().unwrap().0, "_0_0.64_CHGRD");
        assert_eq!(u.category, 1);
        let c = super::drawn("ViewingGroup:34070;LineInstruction:CBLSUB06", &cat);
        assert_eq!(c.line.as_ref().unwrap().1.symbols.len(), 3);
        assert_eq!(c.category, 3);
    }
}
