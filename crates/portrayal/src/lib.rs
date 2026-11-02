//! Chart portrayal from an IHO S-100 portrayal catalogue: the S-101 Portrayal Catalogue
//! (IHO, NOAA and the contributors; bundled unchanged in
//! `app/src-tauri/resources/s101-pc`, licence in `LICENSE-IHO.txt` there).
//!
//! This first part reads what the catalogue holds: the day, dusk and night colour
//! palettes (S-52 colour tokens such as `CHBLK`, `DEPVS`, `CHMGD` with their sRGB values),
//! and the symbols (SVG, sized in millimetres), line styles, area fills and portrayal
//! rules (Lua) by name. Running the rules on chart features comes next.

pub mod dom;
pub mod engine;
pub mod fc;

pub use engine::{Attrs, Feature, Geometry, Portrayal, Portrayer, Value};
pub use fc::FeatureCatalogue;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use quick_xml::events::Event;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("xml in {0}: {1}")]
    Xml(PathBuf, String),
    #[error("not a portrayal catalogue (no {0})")]
    Missing(&'static str),
}

/// One palette: colour token → sRGB.
pub type Palette = BTreeMap<String, [u8; 3]>;

/// A symbol: its SVG file and drawn size in millimetres.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub file: PathBuf,
    pub width_mm: f64,
    pub height_mm: f64,
}

/// A portrayal catalogue opened from its folder (the one holding `portrayal_catalogue.xml`).
#[derive(Debug)]
pub struct Catalogue {
    pub dir: PathBuf,
    /// The catalogue's `version` attribute ("2.1.0").
    pub version: String,
    /// "Day", "Dusk", "Night".
    pub palettes: BTreeMap<String, Palette>,
    pub symbols: BTreeMap<String, Symbol>,
    pub line_styles: BTreeMap<String, PathBuf>,
    pub area_fills: BTreeMap<String, PathBuf>,
    pub rules: BTreeMap<String, PathBuf>,
}

impl Catalogue {
    /// Open the catalogue in `dir`, or in `dir/PortrayalCatalog`.
    pub fn open(dir: &Path) -> Result<Self, Error> {
        let dir = if dir.join("portrayal_catalogue.xml").exists() { dir.to_path_buf() } else { dir.join("PortrayalCatalog") };
        let main = dir.join("portrayal_catalogue.xml");
        if !main.exists() {
            return Err(Error::Missing("portrayal_catalogue.xml"));
        }
        let version = root_attr(&main, "version")?.unwrap_or_default();
        let palettes = read_colour_profile(&dir.join("ColorProfiles").join("colorProfile.xml"))?;
        let mut symbols = BTreeMap::new();
        for (id, file) in files(&dir.join("Symbols"), "svg")? {
            let (w, h) = svg_size_mm(&file)?;
            symbols.insert(id, Symbol { file, width_mm: w, height_mm: h });
        }
        Ok(Catalogue {
            version,
            palettes,
            symbols,
            line_styles: files(&dir.join("LineStyles"), "xml")?,
            area_fills: files(&dir.join("AreaFills"), "xml")?,
            rules: files(&dir.join("Rules"), "lua")?,
            dir,
        })
    }

    /// A colour by token in a palette ("Day", "Dusk", "Night"), as `#rrggbb`.
    pub fn colour(&self, palette: &str, token: &str) -> Option<String> {
        self.palettes.get(palette)?.get(token).map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
    }
}

/// Files with extension `ext` in `dir`, by name without the extension.
fn files(dir: &Path, ext: &str) -> Result<BTreeMap<String, PathBuf>, Error> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.insert(stem.to_string(), p.clone());
            }
        }
    }
    Ok(out)
}

/// Element or attribute name without its namespace prefix.
fn local(name: impl AsRef<[u8]>) -> String {
    let s = String::from_utf8_lossy(name.as_ref());
    s.rsplit(':').next().unwrap_or("").to_string()
}

/// An attribute's value by local name.
fn attr(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| local(a.key.into_inner()) == name).map(|a| {
        let v: &str = &a.value;
        v.to_string()
    })
}

fn xml_err(p: &Path, e: impl std::fmt::Display) -> Error {
    Error::Xml(p.to_path_buf(), e.to_string())
}

/// An attribute of the root element.
fn root_attr(p: &Path, name: &str) -> Result<Option<String>, Error> {
    let text = std::fs::read_to_string(p)?;
    let mut r = quick_xml::Reader::from_str(&text);
    loop {
        match r.read_event().map_err(|e| xml_err(p, e))? {
            Event::Start(e) | Event::Empty(e) => {
                for a in e.attributes().flatten() {
                    if local(a.key.into_inner()) == name {
                        let v: &str = &a.value;
                        return Ok(Some(v.to_string()));
                    }
                }
                return Ok(None);
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// The palettes of an S-100 colour profile: `<palette name="Day">` with
/// `<item token="CHBLK">…<srgb><red/><green/><blue/></srgb></item>`.
fn read_colour_profile(p: &Path) -> Result<BTreeMap<String, Palette>, Error> {
    let text = std::fs::read_to_string(p)?;
    let mut r = quick_xml::Reader::from_str(&text);
    let mut out: BTreeMap<String, Palette> = BTreeMap::new();
    let (mut palette, mut token, mut in_srgb, mut field) = (None::<String>, None::<String>, false, None::<usize>);
    let mut rgb = [0u8; 3];
    loop {
        match r.read_event().map_err(|e| xml_err(p, e))? {
            Event::Start(e) => match local(e.name().into_inner()).as_str() {
                "palette" => palette = attr(&e, "name"),
                "item" => token = attr(&e, "token"),
                "srgb" => in_srgb = true,
                "red" if in_srgb => field = Some(0),
                "green" if in_srgb => field = Some(1),
                "blue" if in_srgb => field = Some(2),
                _ => {}
            },
            Event::Text(t) => {
                if let Some(i) = field {
                    let s: &str = &t;
                    rgb[i] = s.trim().parse().map_err(|_| xml_err(p, format!("bad colour value {s:?}")))?;
                }
            }
            Event::End(e) => match local(e.name().into_inner()).as_str() {
                "red" | "green" | "blue" => field = None,
                "srgb" => {
                    in_srgb = false;
                    if let (Some(pl), Some(tk)) = (&palette, &token) {
                        out.entry(pl.clone()).or_default().insert(tk.clone(), rgb);
                    }
                }
                "item" => token = None,
                "palette" => palette = None,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if out.is_empty() {
        return Err(Error::Missing("palettes in the colour profile"));
    }
    Ok(out)
}

/// A symbol's size from its root `<svg width="4.34mm" height="5.35mm">`.
fn svg_size_mm(p: &Path) -> Result<(f64, f64), Error> {
    let mm = |s: Option<String>| s.and_then(|v| v.trim().trim_end_matches("mm").parse::<f64>().ok()).unwrap_or(0.0);
    let text = std::fs::read_to_string(p)?;
    let mut r = quick_xml::Reader::from_str(&text);
    loop {
        match r.read_event().map_err(|e| xml_err(p, e))? {
            Event::Start(e) | Event::Empty(e) if local(e.name().into_inner()) == "svg" => {
                return Ok((mm(attr(&e, "width")), mm(attr(&e, "height"))));
            }
            Event::Eof => return Ok((0.0, 0.0)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled() -> Catalogue {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources/s101-pc");
        Catalogue::open(&dir).expect("the bundled S-101 portrayal catalogue")
    }

    #[test]
    fn reads_the_bundled_catalogue() {
        let c = bundled();
        assert_eq!(c.version, "2.1.0");
        assert_eq!(c.palettes.keys().map(String::as_str).collect::<Vec<_>>(), ["Day", "Dusk", "Night"]);
        // every token in every palette
        let day = &c.palettes["Day"];
        assert!(day.len() > 60, "{} day colours", day.len());
        for p in ["Dusk", "Night"] {
            assert_eq!(c.palettes[p].len(), day.len(), "{p} has every token");
        }
        assert_eq!(c.colour("Day", "NODTA").as_deref(), Some("#93aebb"));
        for t in ["CHBLK", "CHMGD", "DEPVS", "DEPMS", "DEPMD", "DEPDW", "DEPIT", "DEPSC", "LANDA", "CSTLN"] {
            assert!(day.contains_key(t), "{t}");
        }
        // symbols with their size, line styles, area fills, rules
        assert!(c.symbols.len() > 700);
        let s = &c.symbols["ACHARE02"];
        assert!((s.width_mm - 4.34).abs() < 1e-9 && (s.height_mm - 5.35).abs() < 1e-9);
        for id in ["ISODGR01", "QUESMRK1", "BOYCAR01", "LIGHTS11", "TSSLPT51"] {
            assert!(c.symbols.contains_key(id), "symbol {id}");
        }
        assert!(c.line_styles.contains_key("CBLSUB06"));
        assert!(c.area_fills.contains_key("DRGARE01") && c.area_fills.contains_key("NODATA03"));
        for r in ["main", "CardinalBuoy", "DepthArea", "LightAllAround"] {
            assert!(c.rules.contains_key(r), "rule {r}");
        }
    }
}
