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

    /// A context parameter as TweedPlot has set it.
    pub fn value(&self, id: &str) -> Option<String> {
        self.context.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| k == id).map(|(_, v)| v.clone())
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
    let mut f = s.splitn(4, '|');
    (f.next().unwrap_or(""), f.next().unwrap_or(""))
}

/// The display category the rules give a feature, from its viewing groups (S-52: 1xxxx
/// display base, 2xxxx standard, 3xxxx other; the lowest counts), 0 when none.
pub fn category(instructions: &str) -> u8 {
    let mut cat = 0u8;
    for part in instructions.split(';') {
        if let Some(v) = part.strip_prefix("ViewingGroup:") {
            for g in v.split(',').filter_map(|g| g.trim().parse::<u32>().ok()).filter(|g| (10000..90000).contains(g)) {
                let c = (g / 10000).min(3) as u8;
                if cat == 0 || c < cat {
                    cat = c;
                }
            }
        }
    }
    cat
}

/// A point feature's table entry: `sy|si|sectors|category` (see [`point_symbols`],
/// [`light_sectors`], [`category`]).
pub fn point_entry(instructions: &str) -> String {
    let e = point_symbols(instructions);
    let (sy, si) = split_info(&e);
    let sec = encode_sectors(&light_sectors(instructions));
    format!("{sy}|{si}|{sec}|{}", category(instructions))
}

/// The display category in a point entry (0: none).
pub(crate) fn entry_category(s: &str) -> u8 {
    s.split('|').nth(3).and_then(|c| c.parse().ok()).unwrap_or(0)
}

/// Whether a point entry's feature carries a light (`|1` after the category).
pub(crate) fn entry_lit(s: &str) -> bool {
    s.split('|').nth(4) == Some("1")
}

/// A light's sector drawing from the rules (S-52 LIGHTS05/06): legs and arcs.
#[derive(Debug, Clone, PartialEq)]
pub enum SectorPart {
    /// A sector limit from the light: bearing (degrees true, from the light), and length in
    /// metres on the ground, or else in millimetres on the screen.
    Leg { bearing: f64, metres: Option<f64>, mm: f64 },
    /// An arc round the light (radius mm on the screen) from `start` clockwise through
    /// `extent` degrees, drawn in each style in turn: (dashed, width mm, colour token).
    Arc { radius: f64, start: f64, extent: f64, styles: Vec<(bool, f64, String)> },
}

/// The legs and arcs in a light's drawing instructions.
pub fn light_sectors(instructions: &str) -> Vec<SectorPart> {
    enum G {
        Ray(f64, Option<f64>, f64),
        Arc(f64, f64, f64),
    }
    let mut out: Vec<SectorPart> = Vec::new();
    let mut pending_arc: Option<(f64, f64, f64)> = None;
    let mut geom: Option<G> = None;
    let mut dash = false;
    let mut style: Option<(bool, f64, String)> = None;
    for part in instructions.split(';') {
        let (k, v) = part.split_once(':').unwrap_or((part, ""));
        let f: Vec<&str> = v.split(',').map(str::trim).collect();
        let n = |i: usize| f.get(i).and_then(|x| x.parse::<f64>().ok());
        match k {
            "AugmentedRay" => {
                // GeographicCRS,bearing,crs of the length,length
                if let (Some(b), Some(len)) = (n(1), n(3)) {
                    geom = Some(if f.get(2) == Some(&"GeographicCRS") { G::Ray(b, Some(len), 0.0) } else { G::Ray(b, None, len) });
                }
            }
            "ArcByRadius" => {
                if let (Some(r), Some(s), Some(e)) = (n(2), n(3), n(4)) {
                    pending_arc = Some((r, s, e));
                }
            }
            "AugmentedPath" => geom = pending_arc.take().map(|(r, s, e)| G::Arc(r, s, e)),
            "Dash" => dash = true,
            "LineStyle" => {
                // _simple_,interval,width,colour
                if f.len() == 4 {
                    style = Some((dash, n(2).unwrap_or(0.32), f[3].to_string()));
                }
                dash = false;
            }
            "LineInstruction" => {
                let Some(st) = style.clone() else { continue };
                match &geom {
                    Some(G::Ray(b, m, mm)) => {
                        let leg = SectorPart::Leg { bearing: *b, metres: *m, mm: *mm };
                        if !out.contains(&leg) {
                            out.push(leg);
                        }
                    }
                    Some(G::Arc(r, s, e)) => {
                        let same = |p: &SectorPart| matches!(p, SectorPart::Arc { radius, start, extent, .. } if radius == r && start == s && extent == e);
                        match out.iter_mut().rev().find(|p| same(p)) {
                            Some(SectorPart::Arc { styles, .. }) => styles.push(st),
                            _ => out.push(SectorPart::Arc { radius: *r, start: *s, extent: *e, styles: vec![st] }),
                        }
                    }
                    None => {}
                }
            }
            "ClearGeometry" => geom = None,
            _ => {}
        }
    }
    out
}

/// [`light_sectors`] written into a tile table entry (after `sy|si|`), and back.
fn encode_sectors(parts: &[SectorPart]) -> String {
    parts
        .iter()
        .map(|p| match p {
            SectorPart::Leg { bearing, metres: Some(m), .. } => format!("G {bearing} {m}"),
            SectorPart::Leg { bearing, metres: None, mm } => format!("L {bearing} {mm}"),
            SectorPart::Arc { radius, start, extent, styles } => {
                let st: Vec<String> = styles.iter().map(|(d, w, c)| format!("{}{w}{c}", if *d { "d" } else { "s" })).collect();
                format!("A {radius} {start} {extent} {}", st.join("+"))
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn sectors(s: &str) -> Vec<SectorPart> {
    let Some(enc) = s.split('|').nth(2).filter(|e| !e.is_empty()) else { return Vec::new() };
    enc.split('/')
        .filter_map(|p| {
            let f: Vec<&str> = p.split(' ').collect();
            let n = |i: usize| f.get(i).and_then(|x| x.parse::<f64>().ok());
            match *f.first()? {
                "G" => Some(SectorPart::Leg { bearing: n(1)?, metres: n(2), mm: 0.0 }),
                "L" => Some(SectorPart::Leg { bearing: n(1)?, metres: None, mm: n(2)? }),
                "A" => {
                    let styles = f
                        .get(4)?
                        .split('+')
                        .filter_map(|s| {
                            let dash = s.starts_with('d');
                            let body = &s[1..];
                            let cut = body.find(|c: char| c.is_ascii_alphabetic())?;
                            Some((dash, body[..cut].parse().ok()?, body[cut..].to_string()))
                        })
                        .collect();
                    Some(SectorPart::Arc { radius: n(1)?, start: n(2)?, extent: n(3)?, styles })
                }
                _ => None,
            }
        })
        .collect()
}

/// A text the rules draw (S-100 TextInstruction) with its style at the time.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub text: String,
    /// Horizontal alignment: 'S' start (text to the right), 'C' centre, 'E' end.
    pub h: char,
    /// Vertical alignment: 'T' top (text below), 'C' centre, 'B' bottom or baseline.
    pub v: char,
    /// Offset from the feature's point, mm on the screen, y up.
    pub dx: f64,
    pub dy: f64,
    /// Colour token (CHBLK...).
    pub colour: String,
    /// Font size, points.
    pub size: f64,
    /// Its text group (S-52: 10-19 important text, 20- other text; 23 light descriptions...).
    pub group: u16,
    /// Display category of the text (1 base, 2 standard, 3 other).
    pub category: u8,
    /// Rotation, degrees clockwise.
    pub rot: f64,
}

/// The texts in a feature's drawing instructions, each with the text style in force.
pub fn texts(instructions: &str) -> Vec<Text> {
    let mut out = Vec::new();
    let (mut h, mut v, mut dx, mut dy, mut colour, mut size, mut rot) = ('S', 'B', 0.0, 0.0, "CHBLK".to_string(), 10.0, 0.0);
    let (mut group, mut cat) = (0u16, 0u8);
    for part in instructions.split(';') {
        let (k, val) = part.split_once(':').unwrap_or((part, ""));
        let nums: Vec<f64> = val.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        match k {
            "TextAlignHorizontal" => h = val.chars().next().unwrap_or('S'),
            "TextAlignVertical" => v = val.chars().next().map(|c| if c == 'B' { 'B' } else { c }).unwrap_or('B'),
            "LocalOffset" if nums.len() == 2 => (dx, dy) = (nums[0], nums[1]),
            "FontColor" if !val.is_empty() => colour = val.split(',').next().unwrap_or("CHBLK").trim().to_string(),
            "FontSize" if !nums.is_empty() => size = nums[0],
            "Rotation" => rot = val.split(',').nth(1).and_then(|a| a.trim().parse().ok()).unwrap_or(0.0),
            "ViewingGroup" => {
                let g: Vec<u32> = val.split(',').filter_map(|g| g.trim().parse().ok()).collect();
                // a text's: its text group, then its feature's viewing group
                if g.len() >= 2 && g[0] < 100 {
                    group = g[0] as u16;
                    cat = (g[1] / 10000).clamp(1, 3) as u8;
                } else {
                    group = 0;
                }
            }
            "TextInstruction" if group > 0 => {
                let text: String = val.chars().map(|c| if matches!(c, '|' | ';') || c.is_control() { ' ' } else { c }).collect();
                let text = text.trim();
                if !text.is_empty() {
                    out.push(Text { text: text.to_string(), h, v, dx, dy, colour: colour.clone(), size, group, category: cat, rot });
                }
            }
            _ => {}
        }
    }
    out
}

const TEXT_SEP: char = '\u{1f}';
const FIELD_SEP: char = '\u{1e}';

/// Texts as one string for a table entry (see [`decode_texts`]).
pub(crate) fn encode_texts(t: &[Text]) -> String {
    let r = |x: f64| (x * 100.0).round() / 100.0;
    t.iter()
        .map(|t| {
            format!(
                "{}{FIELD_SEP}{}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}",
                t.text,
                t.h,
                t.v,
                r(t.dx),
                r(t.dy),
                t.colour,
                r(t.size),
                t.group,
                t.category,
                r(t.rot)
            )
        })
        .collect::<Vec<_>>()
        .join(&TEXT_SEP.to_string())
}

pub(crate) fn decode_texts(s: &str) -> Vec<Text> {
    s.split(TEXT_SEP)
        .filter_map(|t| {
            let f: Vec<&str> = t.split(FIELD_SEP).collect();
            if f.len() != 9 {
                return None;
            }
            let mut a = f[1].chars();
            let n = |i: usize| f[i].parse::<f64>().ok();
            Some(Text {
                text: f[0].to_string(),
                h: a.next()?,
                v: a.next()?,
                dx: n(2)?,
                dy: n(3)?,
                colour: f[4].to_string(),
                size: n(5)?,
                group: f[6].parse().ok()?,
                category: f[7].parse().ok()?,
                rot: n(8)?,
            })
        })
        .collect()
}

/// The drawing priority in a point entry (after the texts).
pub(crate) fn entry_priority(s: &str) -> Option<u8> {
    s.split('|').nth(6)?.parse().ok()
}

/// The texts in a point entry (after the light flag).
pub(crate) fn entry_texts(s: &str) -> Vec<Text> {
    s.split('|').nth(5).map(decode_texts).unwrap_or_default()
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
    pub texts: Vec<Text>,
    /// The rules' drawing priorities (S-100 DrawingPriority, S-52's ×3) of its fill or
    /// pattern and of its line: what draws over what.
    pub fill_priority: Option<u8>,
    pub line_priority: Option<u8>,
}

/// The drawing priority of a feature's first point symbol (not its information symbols).
pub fn point_priority(instructions: &str) -> Option<u8> {
    let mut prio = None;
    let mut info = false;
    for part in instructions.split(';') {
        let (k, v) = part.split_once(':').unwrap_or((part, ""));
        match k {
            "DrawingPriority" => prio = v.trim().parse::<f64>().ok().map(|p| p.clamp(0.0, 99.0) as u8),
            "ViewingGroup" => info = v.split(',').any(|g| matches!(g.trim(), "90020" | "90021" | "90022")),
            "PointInstruction" if !v.is_empty() && !info => return prio,
            _ => {}
        }
    }
    None
}

pub fn drawn(instructions: &str, cat: &portrayal::Catalogue) -> Drawn {
    let mut d = Drawn { symbols: point_symbols(instructions), category: category(instructions), texts: texts(instructions), ..Default::default() };
    let mut dashes: Vec<(f64, f64)> = Vec::new();
    let mut simple: Option<portrayal::LineStyle> = None;
    let mut prio: Option<u8> = None;
    for part in instructions.split(';') {
        let (k, v) = part.split_once(':').unwrap_or((part, ""));
        let nums = |v: &str| -> Vec<f64> { v.split(',').map(|x| x.trim().parse().unwrap_or(0.0)).collect() };
        match k {
            "DrawingPriority" => prio = v.trim().parse::<f64>().ok().map(|p| p.clamp(0.0, 99.0) as u8),
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
                        d.line_priority = prio;
                    }
                } else if let Some(s) = cat.line_style(v) {
                    d.line = Some((v.to_string(), s));
                    d.line_priority = prio;
                }
                dashes.clear();
            }
            "ColorFill" if d.fill.is_none() => {
                let mut f = v.split(',');
                let token = f.next().unwrap_or("").trim().to_string();
                let tr = f.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
                if !token.is_empty() {
                    d.fill = Some((token, tr));
                    d.fill_priority = d.fill_priority.max(prio);
                }
            }
            "AreaFillReference" if d.pattern.is_none() && !v.is_empty() => {
                d.pattern = Some(v.to_string());
                d.fill_priority = d.fill_priority.max(prio);
            }
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
            if name.starts_with('_') && (style.dashes.is_empty() || style.interval <= 0.0) && style.symbols.is_empty() {
                // a plain solid line: drawn as a line, not a pattern (crisper, and wide ones
                // don't show the pattern's seams)
                out.push(format!("lc={}", style.colour));
                out.push(format!("lw={}", style.width));
            } else {
                out.push(format!("sl={name}"));
                out.push(format!("slw={}", (self.height(name, style) * 100.0).round() / 100.0));
            }
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
        if let Some(p) = d.fill_priority.filter(|_| d.fill.is_some() || d.pattern.is_some()) {
            out.push(format!("pf={p}"));
        }
        if let Some(p) = d.line_priority.filter(|_| d.line.is_some()) {
            out.push(format!("pl={p}"));
        }
        if !d.texts.is_empty() {
            out.push(format!("tx={}", encode_texts(&d.texts)));
        }
        out.join(";")
    }

    /// The catalogue.
    pub fn catalogue(&self) -> &portrayal::Catalogue {
        &self.cat
    }
}

/// How S-52 draws an S-57 object the S-101 rules have no feature for (they give the "?"),
/// as tile properties: a recommended track charted as an area is a dashed grey outline
/// (S-52 RECTRC, the arrows are its lines'), the place a tide or tidal stream table applies
/// a dashed grey outline with the table's symbol in the middle; others keep the "?".
fn s52_fallback(class: &str, geo: crate::s57::Geo) -> Option<String> {
    match (class, geo) {
        ("RECTRC", crate::s57::Geo::Area) => Some("sl=_5.4_0.32_CHGRD_0-3.6;slw=0.32;vg=2".into()),
        // tide and tidal stream tables: where they apply, and their symbol at the middle
        (c, crate::s57::Geo::Area) => {
            let sy = tide_symbol(c)?;
            Some(format!("sl=_5.4_0.32_CHGRD_0-3.6;slw=0.32;vg=3;sy={sy}"))
        }
        _ => None,
    }
}

/// Drawing instructions for an S-57 point the S-101 rules give only a "?", as S-52 draws
/// it: a tidal stream without its rates (flood, ebb or plain stream arrow, turned to its
/// ORIENT, or the stream symbol without a direction), the places of tide and stream
/// tables, a control point, an opening bridge as a point.
fn point_fallback(o: &S57Object) -> Option<String> {
    let attr = |k: &str| o.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.trim());
    if let Some(i) = attr("SYMINS").and_then(symins) {
        return Some(i);
    }
    match o.class.as_str() {
        "TS_FEB" => {
            let Some(orient) = attr("ORIENT").and_then(|v| v.parse::<f64>().ok()) else {
                return Some("ViewingGroup:33060;PointInstruction:CURDEF01".into());
            };
            let sym = match attr("CAT_TS") {
                Some("1") => "FLDSTR01",
                Some("2") => "EBBSTR01",
                _ => "CURENT01",
            };
            Some(format!("ViewingGroup:33060;Rotation:GeographicCRS,{orient};PointInstruction:{sym}"))
        }
        "T_HMON" | "T_NHMN" | "T_TIMS" | "TS_PNH" | "TS_PRH" | "TS_TIS" | "TS_PAD" => {
            Some(format!("ViewingGroup:33060;PointInstruction:{}", tide_symbol(&o.class)?))
        }
        "CTRPNT" => Some("ViewingGroup:32250;PointInstruction:POSGEN04".into()),
        "ICNARE" => Some("ViewingGroup:26040;PointInstruction:INFARE51".into()),
        // an opening bridge's symbol; S-52 draws nothing for any other bridge as a point
        "BRIDGE" if attr("CATBRG").is_some_and(|c| c.split(',').any(|c| matches!(c.trim(), "2" | "3" | "4" | "5" | "7"))) => {
            Some("ViewingGroup:12210;PointInstruction:BRIDGE01".into())
        }
        "BRIDGE" => Some("ViewingGroup:12210".into()),
        _ => None,
    }
}

/// An object's own S-52 drawing (its SYMINS attribute: a new object class, a temporary
/// notice) as drawing instructions like the IHO rules': its symbols (SY), lines (LC, LS)
/// and fills (AC, AP), and its texts (TX with the text itself).
pub(crate) fn symins(s: &str) -> Option<String> {
    let mut out = vec!["ViewingGroup:26050".to_string()];
    for cmd in s.split(';') {
        let cmd = cmd.trim();
        let Some((op, rest)) = cmd.split_once('(') else { continue };
        let args: Vec<&str> = rest.trim_end_matches(')').split(',').map(str::trim).collect();
        let a0 = args.first().copied().unwrap_or("");
        match op.trim() {
            "SY" if !a0.is_empty() => {
                if let Some(r) = args.get(1).and_then(|r| r.parse::<f64>().ok()) {
                    out.push(format!("Rotation:GeographicCRS,{r}"));
                }
                out.push(format!("PointInstruction:{a0}"));
            }
            "LC" if !a0.is_empty() => out.push(format!("LineInstruction:{a0}")),
            "LS" if args.len() >= 3 => {
                // S-52 widths are in 0.32 mm pixels; dashes 3.6 mm with 1.8 mm gaps, dots 0.6/1.2
                let w = args[1].parse::<f64>().unwrap_or(1.0) * 0.32;
                let (interval, dash) = match a0 {
                    "DASH" => (5.4, Some(3.6)),
                    "DOTT" => (1.8, Some(0.6)),
                    _ => (0.0, None),
                };
                if let Some(d) = dash {
                    out.push(format!("Dash:0,{d}"));
                }
                out.push(format!("LineStyle:_simple_,{interval},{w},{}", args[2]));
                out.push("LineInstruction:_simple_".into());
            }
            "AC" if !a0.is_empty() => {
                let tr = args.get(1).and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0) * 0.25;
                out.push(format!("ColorFill:{a0},{tr}"));
            }
            "AP" if !a0.is_empty() => out.push(format!("AreaFillReference:{a0}")),
            // TX('text',hjust,vjust,space,'chars',xoffs,yoffs,colour,group): a quoted text
            // only (an attribute's value isn't at hand here)
            "TX" if args.len() >= 9 && a0.starts_with('\'') => {
                let text = a0.trim_matches('\'');
                let h = match args[1] { "1" => "Center", "2" => "End", _ => "Start" };
                let v = match args[2] { "1" => "Bottom", "2" => "Center", _ => "Top" };
                let size = args[4].trim_matches('\'').get(3..).and_then(|n| n.parse::<f64>().ok()).unwrap_or(10.0);
                // offsets in body sizes (about 3.51 mm), y down
                let off = |i: usize| args[i].parse::<f64>().unwrap_or(0.0) * 3.51;
                out.push(format!(
                    "FontColor:{};FontSize:{size};TextAlignHorizontal:{h};TextAlignVertical:{v};LocalOffset:{},{};ViewingGroup:{},26050;TextInstruction:{text}",
                    args[7],
                    off(5),
                    -off(6),
                    args[8].parse::<u32>().unwrap_or(26).min(99)
                ));
            }
            _ => {}
        }
    }
    (out.len() > 1).then(|| out.join(";"))
}

/// The S-52 symbol of a tide (height) or tidal stream table's place.
fn tide_symbol(class: &str) -> Option<&'static str> {
    match class {
        "T_HMON" | "T_NHMN" | "T_TIMS" => Some("TIDEHT01"),
        "TS_PNH" | "TS_PRH" | "TS_TIS" | "TS_PAD" => Some("TIDSTR01"),
        "TS_FEB" => Some("CURDEF01"),
        _ => None,
    }
}

/// A line or area feature's tile properties from what [`S52::encode`] made.
pub(crate) fn props(encoded: &str) -> impl Iterator<Item = (&'static str, &str)> {
    encoded.split(';').filter_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        let k = match k {
            "sl" => "sl",
            "slw" => "slw",
            "lc" => "lc",
            "lw" => "lw",
            "cf" => "cf",
            "ct" => "ct",
            "af" => "af",
            "sy" => "sy",
            "si" => "si",
            "vg" => "vg",
            "tx" => "tx",
            "pf" => "pf",
            "pl" => "pl",
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

    /// Whether light sector limits are drawn to the lights' nominal range (FullLightLines).
    pub(crate) fn s52_full_light_lines(&self) -> bool {
        let s = self.s52.read().unwrap_or_else(|e| e.into_inner()).clone();
        s.is_some_and(|s| s.value("FullLightLines").as_deref() == Some("true"))
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
            let cls = if class.code == crate::s57::UNKNOWN || class.code == crate::s57::CHART_OBJECT {
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
        // which of them carry a light of the same band (a lighthouse, a beacon): the chart
        // shows such a structure whenever it shows the light, so a light never stands alone
        let mut lit: std::collections::HashSet<i64> = std::collections::HashSet::new();
        if class.geo == crate::s57::Geo::Point && class.code != "LIGHTS" {
            let mut grid: HashMap<(i64, i64), Vec<[f64; 2]>> = HashMap::new();
            let cell = |p: [f64; 2]| ((p[0] * 1000.0).floor() as i64, (p[1] * 1000.0).floor() as i64);
            for lt in self.tables.iter().filter(|x| x.band == t.band && x.class.is_some_and(|c| c.code == "LIGHTS")) {
                self.query_text(lt, t.bounds, &[], |row| {
                    if let Geom::Points(p) = &row.geom {
                        for q in p {
                            grid.entry(cell(*q)).or_default().push(*q);
                        }
                    }
                });
            }
            if !grid.is_empty() {
                for (fid, o) in &rows {
                    let Some(p) = pos(o) else { continue };
                    let (cx, cy) = cell(p);
                    let near = (-1..=1)
                        .flat_map(|dx| (-1..=1).map(move |dy| (cx + dx, cy + dy)))
                        .any(|c| grid.get(&c).is_some_and(|v| v.iter().any(|q| (q[0] - p[0]).abs() < 2e-4 && (q[1] - p[1]).abs() < 2e-4)));
                    if near {
                        lit.insert(*fid);
                    }
                }
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
        // each feature's S-57 class (an unknown-object table holds several)
        let s57_class: HashMap<String, String> = rows.iter().map(|(f, o)| (f.to_string(), o.class.clone())).collect();
        let s57_symins: HashMap<String, String> = rows
            .iter()
            .filter_map(|(f, o)| Some((f.to_string(), symins(o.attrs.iter().find(|(k, _)| k == "SYMINS")?.1.as_str())?)))
            .collect();
        let s57_point: HashMap<String, String> =
            rows.iter().filter_map(|(f, o)| Some((f.to_string(), point_fallback(o)?))).collect();
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
            let sy: Arc<str> = if class.geo == crate::s57::Geo::Point {
                // a class the S-101 rules can't draw from the old data ("?"): the S-52 symbol
                // (a new object's own drawing, SYMINS, always: S-101 makes every one a
                // virtual AIS aid)
                let newobj = s57_class.get(&o.id).is_some_and(|c| c == "NEWOBJ");
                let ins = match s57_point.get(&o.id) {
                    Some(f) if newobj || o.instructions.contains("QUESMRK1") => f.as_str(),
                    _ => o.instructions.as_str(),
                };
                let mut e = point_entry(ins);
                let lit = o.id.parse::<i64>().is_ok_and(|f| lit.contains(&f));
                let tx = encode_texts(&texts(ins));
                let pr = point_priority(ins);
                if lit || !tx.is_empty() || pr.is_some() {
                    e.push_str(if lit { "|1" } else { "|" });
                }
                if !tx.is_empty() || pr.is_some() {
                    e.push('|');
                    e.push_str(&tx);
                }
                if let Some(p) = pr {
                    e.push_str(&format!("|{p}"));
                }
                e.into()
            } else {
                let unknown = o.instructions.contains("QUESMRK1");
                let newobj = s57_class.get(&o.id).is_some_and(|c| c == "NEWOBJ");
                match s57_symins.get(&o.id).filter(|_| unknown || newobj) {
                    // its own S-52 drawing
                    Some(ins) => s.encode(&drawn(ins, &s.cat)).into(),
                    // an incineration area (no S-101 feature): S-52's information-area symbol and
                    // boundary, symbolised or plain as chosen
                    None if unknown && class.geo == crate::s57::Geo::Area && s57_class.get(&o.id).is_some_and(|c| c == "ICNARE") => {
                        let line = if s.value("PlainBoundaries").as_deref() == Some("false") {
                            "LineInstruction:CTYARE51".to_string()
                        } else {
                            "Dash:0,3.6;LineStyle:_simple_,5.4,0.64,CHMGD;LineInstruction:_simple_".to_string()
                        };
                        s.encode(&drawn(&format!("ViewingGroup:26040;PointInstruction:INFARE51;{line}"), &s.cat)).into()
                    }
                    None => {
                        let fallback = unknown.then(|| s57_class.get(&o.id).and_then(|c| s52_fallback(c, class.geo))).flatten();
                        fallback.map(Into::into).unwrap_or_else(|| s.encode(&drawn(&o.instructions, &s.cat)).into())
                    }
                }
            };
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
    fn drawing_priorities() {
        let cat = portrayal::Catalogue::open(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources/s101-pc")).unwrap();
        let d = super::drawn("ViewingGroup:26010;DrawingPriority:9;ColorFill:CHBRN;DrawingPriority:18;LineInstruction:CTYARE51", &cat);
        assert_eq!((d.fill_priority, d.line_priority), (Some(9), Some(18)));
        assert_eq!(super::point_priority("ViewingGroup:27010;DrawingPriority:24;PointInstruction:BOYLAT13"), Some(24));
        assert_eq!(super::point_priority("ViewingGroup:27010,90020;DrawingPriority:24;PointInstruction:INFORM01"), None);
        assert_eq!(super::entry_priority("BOYLAT13|||2|||24"), Some(24));
        assert!(super::entry_texts("BOYLAT13|||2|||24").is_empty());
        assert!(!super::entry_lit("BOYLAT13|||2|||24"));
    }

    #[test]
    fn texts_with_their_style() {
        let i = "ViewingGroup:27070;Rotation:PortrayalCRS,135;PointInstruction:LIGHTS13;Rotation:PortrayalCRS,0;FontColor:CHBLK;LocalOffset:7.02,0;TextAlignHorizontal:Start;TextAlignVertical:Center;ViewingGroup:23,27070;DrawingPriority:24;TextInstruction:Fl W;ViewingGroup:27070";
        let t = super::texts(i);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].text.as_str(), t[0].h, t[0].v, t[0].dx, t[0].group, t[0].category), ("Fl W", 'S', 'C', 7.02, 23, 2));
        assert_eq!(super::decode_texts(&super::encode_texts(&t)), t);
        let e = format!("LIGHTS13|||2|1|{}", super::encode_texts(&t));
        assert!(super::entry_lit(&e));
        assert_eq!(super::entry_texts(&e), t);
        assert_eq!(super::entry_texts("LIGHTS13|||2|1"), vec![]);
    }

    #[test]
    fn symins_to_instructions() {
        let i = super::symins("SY(CTNARE51);LC(CTNARE51);TX(OBJNAM,1,2,3,'15110',0,0,CHBLK,26)").unwrap();
        assert_eq!(i, "ViewingGroup:26050;PointInstruction:CTNARE51;LineInstruction:CTNARE51");
        let i = super::symins("LS(DASH,2,CHMGD);AC(DEPVS,2)").unwrap();
        assert_eq!(i, "ViewingGroup:26050;Dash:0,3.6;LineStyle:_simple_,5.4,0.64,CHMGD;LineInstruction:_simple_;ColorFill:DEPVS,0.5");
        let t = super::texts(&super::symins("TX('V-AIS',3,2,2,'15110',2,0,CHMGD,11)").unwrap());
        assert_eq!((t[0].text.as_str(), t[0].h, t[0].v, t[0].dx, t[0].colour.as_str(), t[0].group), ("V-AIS", 'S', 'C', 7.02, "CHMGD", 11));
    }

    #[test]
    fn symbols_from_instructions() {
        let s = super::point_symbols("ViewingGroup:27010;PointInstruction:BOYPIL24;ViewingGroup:27010,27050;PointInstruction:TOPMAR06");
        assert_eq!(s, "BOYPIL24,TOPMAR06");
        let l = super::point_symbols("Hover:true;Rotation:PortrayalCRS,135;PointInstruction:LIGHTS11;Rotation:PortrayalCRS,0;FontColor:CHBLK");
        assert_eq!(l, "LIGHTS11@135");
        let i = super::point_symbols("ViewingGroup:27070;PointInstruction:LIGHTS13;ViewingGroup:27070,90020;DrawingPriority:24;PointInstruction:INFORM01");
        assert_eq!(super::split_info(&i), ("LIGHTS13", "INFORM01"));
        // a sector of a light: two legs, an arc drawn black then red (S-101 LightSectored)
        let ins = "ViewingGroup:27070;AugmentedRay:GeographicCRS,190,LocalCRS,25;Dash:0,3.6;LineStyle:_simple_,5.4,0.32,CHBLK;LineInstruction:_simple_;\
            AugmentedRay:GeographicCRS,250,LocalCRS,25;LineInstruction:_simple_;ArcByRadius:0,0,20,190,60;AugmentedPath:LocalCRS,GeographicCRS,LocalCRS;\
            LineStyle:_simple_,,1.28,CHBLK;LineInstruction:_simple_;LineStyle:_simple_,,0.64,LITRD;LineInstruction:_simple_;ClearGeometry";
        let sec = super::light_sectors(ins);
        assert_eq!(sec.len(), 3, "{sec:?}");
        assert_eq!(sec[0], super::SectorPart::Leg { bearing: 190.0, metres: None, mm: 25.0 });
        let super::SectorPart::Arc { radius, extent, styles, .. } = &sec[2] else { panic!() };
        assert_eq!((*radius, *extent), (20.0, 60.0));
        assert_eq!(styles, &vec![(false, 1.28, "CHBLK".to_string()), (false, 0.64, "LITRD".to_string())]);
        let enc = super::point_entry(ins);
        assert_eq!(super::sectors(&enc), sec);
        assert_eq!(super::split_info(&enc), ("", ""));
        assert_eq!(super::entry_category(&enc), 2);
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
