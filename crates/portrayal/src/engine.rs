//! Running the catalogue's Lua portrayal rules on features: TweedPlot is the "host" of
//! S-100 Part 9a. Each run gets a fresh Lua 5.1 state (the rules keep caches keyed by
//! feature id), loads the rules, describes the feature catalogue to them, sets the
//! context parameters (safety contour, four shades, ...) and collects the drawing
//! instructions each feature emits.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mlua::{Lua, Table, Value as LuaValue};

use crate::fc::{Binding, FeatureCatalogue};

/// An attribute value: simple values as their encoded strings (enumerations as their
/// code, "1"), complex ones as their sub-attributes, one map per instance.
#[derive(Debug, Clone)]
pub enum Value {
    Simple(Vec<String>),
    Complex(Vec<Attrs>),
}

pub type Attrs = BTreeMap<String, Value>;

#[derive(Debug, Clone)]
pub enum Geometry {
    Point([f64; 2]),
    /// Soundings: x, y, depth.
    MultiPoint(Vec<[f64; 3]>),
    Curve(Vec<[f64; 2]>),
    /// Exterior ring first.
    Surface(Vec<Vec<[f64; 2]>>),
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub id: String,
    /// S-101 feature type code ("CardinalBuoy").
    pub code: String,
    pub attrs: Attrs,
    pub geometry: Option<Geometry>,
}

/// What the rules drew for one feature.
#[derive(Debug, Clone, PartialEq)]
pub struct Portrayal {
    pub id: String,
    /// S-100 drawing instructions, `;` separated ("ViewingGroup:27010;...;PointInstruction:BOYCAR01").
    pub instructions: String,
    /// The context parameters the rules looked at for it.
    pub observed: String,
}

#[derive(Debug, Clone)]
pub struct ContextParameter {
    pub id: String,
    /// "Double", "Boolean", "Integer", "String", "Date"
    pub kind: String,
    pub default: String,
}

pub struct Portrayer {
    rules: PathBuf,
    fc: FeatureCatalogue,
    context: Vec<ContextParameter>,
    values: BTreeMap<String, String>,
}

const HOST: &str = include_str!("host.lua");

impl Portrayer {
    pub fn new(cat: &crate::Catalogue, fc: FeatureCatalogue) -> Result<Self, crate::Error> {
        let main = cat.dir.join("portrayal_catalogue.xml");
        let text = std::fs::read_to_string(&main)?;
        let root = crate::dom::parse(&text).map_err(|e| crate::Error::Xml(main.clone(), e))?;
        let context = root
            .child("context")
            .map(|c| {
                c.all("parameter")
                    .filter_map(|p| {
                        Some(ContextParameter {
                            id: p.attr("id")?.to_string(),
                            kind: p.text_of("type").unwrap_or_default(),
                            default: p.text_of("default").unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Portrayer { rules: cat.dir.join("Rules"), fc, context, values: BTreeMap::new() })
    }

    pub fn context(&self) -> &[ContextParameter] {
        &self.context
    }

    /// Set a context parameter ("SafetyContour", "5"); unknown names are ignored.
    pub fn set(&mut self, id: &str, value: &str) {
        if self.context.iter().any(|c| c.id == id) {
            self.values.insert(id.to_string(), value.to_string());
        }
    }

    /// A context parameter's value: TweedPlot's, or the catalogue's default.
    pub fn value(&self, id: &str) -> Option<&str> {
        self.values.get(id).map(String::as_str).or_else(|| self.context.iter().find(|c| c.id == id).map(|c| c.default.as_str()))
    }

    pub fn feature_catalogue(&self) -> &FeatureCatalogue {
        &self.fc
    }

    /// Portray features. Rule errors are the rules' business: they fall back to the
    /// catalogue's default symbology (a magenta "?"), as the standard asks. `trace` gets
    /// the rules' debug messages.
    pub fn portray(&self, features: &[Feature], trace: &mut Vec<String>) -> mlua::Result<Vec<Portrayal>> {
        let lua = Lua::new();
        let g = lua.globals();
        let package: Table = g.get("package")?;
        // The rules are found by a loader of our own, not through package.path: Lua's
        // search path uses '?' as the placeholder, and a Windows install path can hold
        // one (\\?\C:\...), which sent `require 'main'` to "//main/C:/...".
        package.set("path", "")?;
        package.set("cpath", "")?;
        let rules = self.rules.clone();
        let loader = lua.create_function(move |lua, name: String| {
            if name.contains(['/', '\\']) || name.contains("..") {
                return Ok(mlua::Value::String(lua.create_string(format!("\n\tno rule '{name}'"))?));
            }
            let file = rules.join(format!("{name}.lua"));
            match std::fs::read(&file) {
                Ok(src) => Ok(mlua::Value::Function(lua.load(src).set_name(format!("@{name}.lua")).into_function()?)),
                Err(e) => Ok(mlua::Value::String(lua.create_string(format!("\n\tno rule {}: {e}", file.display()))?)),
            }
        })?;
        let loaders: Table = package.get("loaders")?;
        loaders.raw_insert(2, loader)?;
        g.set("HostFC", self.fc_table(&lua)?)?;
        g.set("HostEmitted", lua.create_table()?)?;
        g.set("HostTrace", lua.create_table()?)?;
        g.set("HostDS", self.dataset(&lua, features)?)?;
        lua.load("require 'main'").set_name("main").exec()?;
        lua.load(HOST).set_name("host.lua").exec()?;
        // context parameters: the catalogue's defaults, then TweedPlot's values
        let create: mlua::Function = g.get("PortrayalCreateContextParameter")?;
        let params = lua.create_table()?;
        for (i, c) in self.context.iter().enumerate() {
            let v = self.values.get(&c.id).unwrap_or(&c.default);
            params.set(i + 1, create.call::<LuaValue>((c.id.as_str(), c.kind.as_str(), v.as_str()))?)?;
        }
        g.get::<mlua::Function>("PortrayalInitializeContextParameters")?.call::<()>(params)?;
        g.get::<mlua::Function>("PortrayalMain")?.call::<LuaValue>(LuaValue::Nil)?;
        let mut out = Vec::new();
        for e in g.get::<Table>("HostEmitted")?.sequence_values::<Table>() {
            let e = e?;
            out.push(Portrayal { id: e.get("id")?, instructions: e.get("instructions")?, observed: e.get::<Option<String>>("observed")?.unwrap_or_default() });
        }
        for t in g.get::<Table>("HostTrace")?.sequence_values::<String>() {
            trace.push(t?);
        }
        Ok(out)
    }

    fn bindings(lua: &Lua, list: &[Binding]) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        for (i, b) in list.iter().enumerate() {
            let x = lua.create_table()?;
            x.set("code", b.code.as_str())?;
            x.set("lower", b.lower)?;
            x.set("upper", b.upper)?;
            x.set("sequential", b.sequential)?;
            x.set("permitted", lua.create_sequence_from(b.permitted.iter().copied())?)?;
            t.set(i + 1, x)?;
        }
        Ok(t)
    }

    /// The feature catalogue as the plain tables host.lua expects.
    fn fc_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let fc = &self.fc;
        let t = lua.create_table()?;
        let features = lua.create_table()?;
        for f in fc.features.values() {
            let x = lua.create_table()?;
            x.set("code", f.code.as_str())?;
            x.set("name", f.name.as_str())?;
            x.set("aliases", lua.create_sequence_from(f.aliases.iter().map(String::as_str))?)?;
            x.set("abstract", f.is_abstract)?;
            x.set("useType", f.use_type.as_str())?;
            x.set("primitives", lua.create_sequence_from(f.primitives.iter().map(String::as_str))?)?;
            x.set("bindings", Self::bindings(lua, &f.bindings)?)?;
            features.set(f.code.as_str(), x)?;
        }
        t.set("features", features)?;
        let simple = lua.create_table()?;
        for a in fc.simple.values() {
            let x = lua.create_table()?;
            x.set("code", a.code.as_str())?;
            x.set("name", a.name.as_str())?;
            x.set("aliases", lua.create_sequence_from(a.aliases.iter().map(String::as_str))?)?;
            x.set("valueType", a.value_type.as_str())?;
            let listed = lua.create_table()?;
            for (i, (code, label)) in a.listed.iter().enumerate() {
                let v = lua.create_table()?;
                v.set("code", *code)?;
                v.set("label", label.as_str())?;
                listed.set(i + 1, v)?;
            }
            x.set("listed", listed)?;
            simple.set(a.code.as_str(), x)?;
        }
        t.set("simple", simple)?;
        let complex = lua.create_table()?;
        for a in fc.complex.values() {
            let x = lua.create_table()?;
            x.set("code", a.code.as_str())?;
            x.set("name", a.name.as_str())?;
            x.set("aliases", lua.create_sequence_from(a.aliases.iter().map(String::as_str))?)?;
            x.set("bindings", Self::bindings(lua, &a.bindings)?)?;
            complex.set(a.code.as_str(), x)?;
        }
        t.set("complex", complex)?;
        let information = lua.create_table()?;
        for i in fc.information.values() {
            let x = lua.create_table()?;
            x.set("code", i.code.as_str())?;
            x.set("name", i.name.as_str())?;
            x.set("aliases", lua.create_table()?)?;
            x.set("bindings", Self::bindings(lua, &i.bindings)?)?;
            information.set(i.code.as_str(), x)?;
        }
        t.set("information", information)?;
        for (name, map) in [("fassoc", &fc.feature_associations), ("iassoc", &fc.information_associations)] {
            let x = lua.create_table()?;
            for a in map.values() {
                x.set(a.code.as_str(), lua.create_sequence_from(a.roles.iter().map(String::as_str))?)?;
            }
            t.set(name, x)?;
        }
        t.set("roles", lua.create_sequence_from(fc.roles.iter().map(String::as_str))?)?;
        Ok(t)
    }

    fn attrs(lua: &Lua, attrs: &Attrs) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        for (k, v) in attrs {
            match v {
                Value::Simple(vals) => t.set(k.as_str(), lua.create_sequence_from(vals.iter().map(String::as_str))?)?,
                Value::Complex(items) => {
                    let list = lua.create_table()?;
                    for (i, a) in items.iter().enumerate() {
                        list.set(i + 1, Self::attrs(lua, a)?)?;
                    }
                    t.set(k.as_str(), list)?;
                }
            }
        }
        Ok(t)
    }

    /// The features as host.lua expects them: ids, features (code, attrs, spatial), spatials.
    fn dataset(&self, lua: &Lua, features: &[Feature]) -> mlua::Result<Table> {
        let ds = lua.create_table()?;
        let ids = lua.create_table()?;
        let fs = lua.create_table()?;
        let spatials = lua.create_table()?;
        let pair = |p: &[f64; 2]| lua.create_sequence_from([p[0], p[1]]);
        for (i, f) in features.iter().enumerate() {
            ids.set(i + 1, f.id.as_str())?;
            let x = lua.create_table()?;
            x.set("code", f.code.as_str())?;
            x.set("attrs", Self::attrs(lua, &f.attrs)?)?;
            if let Some(geom) = &f.geometry {
                let sid = format!("{}#g", f.id);
                let s = lua.create_table()?;
                let kind = match geom {
                    Geometry::Point(p) => {
                        s.set("x", p[0])?;
                        s.set("y", p[1])?;
                        "Point"
                    }
                    Geometry::MultiPoint(pts) => {
                        let t = lua.create_table()?;
                        for (j, p) in pts.iter().enumerate() {
                            t.set(j + 1, lua.create_sequence_from([p[0], p[1], p[2]])?)?;
                        }
                        s.set("points", t)?;
                        "MultiPoint"
                    }
                    Geometry::Curve(pts) => {
                        let t = lua.create_table()?;
                        for (j, p) in pts.iter().enumerate() {
                            t.set(j + 1, pair(p)?)?;
                        }
                        s.set("points", t)?;
                        "Curve"
                    }
                    Geometry::Surface(rings) => {
                        let ids = lua.create_table()?;
                        for (r, ring) in rings.iter().enumerate() {
                            let rid = format!("{sid}#r{r}");
                            let c = lua.create_table()?;
                            c.set("kind", "Curve")?;
                            let t = lua.create_table()?;
                            for (j, p) in ring.iter().enumerate() {
                                t.set(j + 1, pair(p)?)?;
                            }
                            c.set("points", t)?;
                            spatials.set(rid.as_str(), c)?;
                            ids.set(r + 1, rid)?;
                        }
                        s.set("rings", ids)?;
                        "Surface"
                    }
                };
                s.set("kind", kind)?;
                spatials.set(sid.as_str(), s)?;
                x.set("spatial", sid)?;
                x.set("spatialType", kind)?;
            }
            fs.set(f.id.as_str(), x)?;
        }
        ds.set("ids", ids)?;
        ds.set("features", fs)?;
        ds.set("spatials", spatials)?;
        ds.set("spatialFeatures", lua.create_table()?)?;
        Ok(ds)
    }
}
