//! The catalogue's line styles and area fills (S-100 Part 9 `lineStyle`,
//! `compositeLineStyle`, `symbolFill`), read into plain values for drawing.

use serde::Serialize;

use crate::{dom, Catalogue};

/// A line style: a pen, dashes and symbols repeated every `interval` mm along the line
/// (no interval: a plain line). A composite style is drawn as its `parts`, one over the
/// other. Lengths in millimetres.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LineStyle {
    pub interval: f64,
    pub width: f64,
    /// Colour token ("CHMGD").
    pub colour: String,
    /// (start, length) of each dash in the interval; none: solid.
    pub dashes: Vec<(f64, f64)>,
    /// (symbol, position along the interval); the symbol's x axis runs along the line.
    pub symbols: Vec<(String, f64)>,
    pub parts: Vec<LineStyle>,
}

/// An area fill: a symbol repeated on the lattice `v1`, `v2` (mm).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AreaFill {
    pub symbol: String,
    pub v1: [f64; 2],
    pub v2: [f64; 2],
}

fn num(n: &dom::Node, name: &str) -> f64 {
    n.text_of(name).and_then(|t| t.parse().ok()).unwrap_or(0.0)
}

fn line_style(n: &dom::Node) -> LineStyle {
    let pen = n.child("pen");
    LineStyle {
        interval: num(n, "intervalLength"),
        width: pen.and_then(|p| p.attr("width")).and_then(|w| w.parse().ok()).unwrap_or(0.32),
        colour: pen.and_then(|p| p.text_of("color")).unwrap_or_else(|| "CHBLK".into()),
        dashes: n.all("dash").map(|d| (num(d, "start"), num(d, "length"))).collect(),
        symbols: n.all("symbol").filter_map(|s| Some((s.attr("reference")?.to_string(), num(s, "position")))).collect(),
        parts: n.all("lineStyle").map(line_style).collect(),
    }
}

impl LineStyle {
    /// A simple line from a rule's `LineStyle:_simple_,interval,width,colour` and its
    /// `Dash:start,length` instructions.
    pub fn simple(interval: f64, width: f64, colour: &str, dashes: Vec<(f64, f64)>) -> Self {
        LineStyle { interval, width, colour: colour.into(), dashes, ..Default::default() }
    }
}

impl Catalogue {
    /// A line style by name ("CBLSUB06").
    pub fn line_style(&self, name: &str) -> Option<LineStyle> {
        let text = std::fs::read_to_string(self.line_styles.get(name)?).ok()?;
        Some(line_style(&dom::parse(&text).ok()?))
    }

    /// An area fill by name ("NODATA03"). Only symbol fills (all the S-101 catalogue has).
    pub fn area_fill(&self, name: &str) -> Option<AreaFill> {
        let text = std::fs::read_to_string(self.area_fills.get(name)?).ok()?;
        let n = dom::parse(&text).ok()?;
        let v = |k: &str| n.child(k).map(|c| [num(c, "x"), num(c, "y")]);
        Some(AreaFill { symbol: n.child("symbol")?.attr("reference")?.to_string(), v1: v("v1")?, v2: v("v2")? })
    }

    /// A symbol's view box in mm (min x, min y, width, height; the pivot is 0,0).
    pub fn symbol_box(&self, id: &str) -> Option<[f64; 4]> {
        let text = std::fs::read_to_string(&self.symbols.get(id)?.file).ok()?;
        crate::view_box(&text)
    }

    /// How tall a line drawn in `style` is, across the line (mm): its pen, or its symbols
    /// reaching out either side, whichever is more. The line runs along its middle.
    pub fn line_height(&self, style: &LineStyle) -> f64 {
        let mut h = style.width;
        for (s, _) in &style.symbols {
            if let Some(b) = self.symbol_box(s) {
                h = h.max(2.0 * b[1].abs().max((b[1] + b[3]).abs()));
            }
        }
        for p in &style.parts {
            h = h.max(self.line_height(p));
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalogue {
        Catalogue::open(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources/s101-pc")).unwrap()
    }

    #[test]
    fn line_styles_and_fills() {
        let c = cat();
        let l = c.line_style("CBLSUB06").unwrap();
        assert_eq!(l.interval, 30.9);
        assert_eq!(l.colour, "CHMGD");
        assert_eq!(l.dashes, vec![(0.6, 2.0), (23.8, 2.0)]);
        assert_eq!(l.symbols[1], ("EMCBLAR1".to_string(), 13.8));
        assert!(c.line_height(&l) > 1.0);
        let k = c.line_style("INDHLT02").unwrap();
        assert_eq!(k.parts.len(), 2);
        assert_eq!(k.parts[0].width, 1.28);
        let f = c.area_fill("NODATA03").unwrap();
        assert_eq!(f.symbol, "NODATA03P");
        assert_eq!(f.v2, [3.51, 4.96]);
        // every style and fill reads, and their symbols exist
        for n in c.line_styles.keys() {
            let s = c.line_style(n).unwrap_or_else(|| panic!("{n}"));
            for (sym, _) in s.symbols.iter().chain(s.parts.iter().flat_map(|p| p.symbols.iter())) {
                assert!(c.symbols.contains_key(sym), "{n}: {sym}");
            }
        }
        for n in c.area_fills.keys() {
            let f = c.area_fill(n).unwrap_or_else(|| panic!("{n}"));
            assert!(c.symbols.contains_key(&f.symbol), "{n}: {}", f.symbol);
        }
    }
}
