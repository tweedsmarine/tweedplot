//! S-57 chart objects (LINZ layers, ENC cells) turned into S-101 features for the
//! portrayal rules. The S-101 Feature Catalogue names the S-57 acronym of most feature
//! types and attributes (its aliases), so the mapping comes from it: `BOYCAR` is
//! CardinalBuoy, `CATCAM` its categoryOfCardinalMark. An S-57 attribute that S-101
//! moved into a complex attribute is put there (OBJNAM goes in featureName.name, SIGPER
//! in rhythmOfLight.signalPeriod, SECTR1 in ...sectorLimit.sectorLimitOne.sectorBearing),
//! by finding the shortest chain of complex attributes from the feature type to it.
//!
//! What S-57 kept in separate objects and S-101 keeps on the feature is merged by the
//! caller with the helpers here: a topmark (TOPMAR) on its buoy or beacon, the records
//! of a sector light at one position into one LightSectored.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::engine::{Attrs, Feature, Geometry, Value};
use crate::fc::{Binding, FeatureCatalogue, FeatureType};

/// One S-57 object: its class acronym, attributes (acronym, value as in the data:
/// lists comma separated), and geometry.
#[derive(Debug, Clone)]
pub struct S57Object {
    pub class: String,
    pub attrs: Vec<(String, String)>,
    pub geometry: Option<Geometry>,
}

impl S57Object {
    pub fn get(&self, acronym: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(acronym)).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    }
}

/// Attributes not portrayed or not part of the feature: national language copies, source
/// and file references, ids.
const SKIP: &[&str] = &["NOBJNM", "NINFOM", "NTXTDS", "TXTDSC", "PICREP", "SORDAT", "SORIND", "FIDN", "RECDAT", "RECIND", "VERACC", "VERDAT", "OBJL"];

/// Where an attribute goes in a feature type: the complex attributes leading to it
/// (outermost first) and its own code.
type Place = (Vec<String>, String);

pub struct Converter<'a> {
    fc: &'a FeatureCatalogue,
    places: Mutex<HashMap<(String, String), Option<Place>>>,
}

fn primitive(g: &Option<Geometry>) -> &'static str {
    match g {
        Some(Geometry::Point(_)) => "point",
        Some(Geometry::MultiPoint(_)) => "pointSet",
        Some(Geometry::Curve(_)) => "curve",
        Some(Geometry::Surface(_)) => "surface",
        None => "noGeometry",
    }
}

impl<'a> Converter<'a> {
    pub fn new(fc: &'a FeatureCatalogue) -> Self {
        Converter { fc, places: Mutex::new(HashMap::new()) }
    }

    /// The S-101 feature type for an S-57 object, or None (then the rules draw it as an
    /// unknown object).
    pub fn feature_type(&self, o: &S57Object) -> Option<&'a FeatureType> {
        let class = o.class.to_ascii_uppercase();
        let prim = primitive(&o.geometry);
        let pick = |code: &str| self.fc.features.get(code);
        match class.as_str() {
            "LIGHTS" => {
                let air = o.get("CATLIT").is_some_and(|c| c.split(',').any(|x| x.trim() == "6"));
                let sector = match (o.get("SECTR1").and_then(|v| v.parse::<f64>().ok()), o.get("SECTR2").and_then(|v| v.parse::<f64>().ok())) {
                    (Some(a), Some(b)) => ((b - a).rem_euclid(360.0) - 0.0).abs() > 1e-6,
                    _ => false,
                };
                return pick(if air {
                    "LightAirObstruction"
                } else if sector {
                    "LightSectored"
                } else {
                    "LightAllAround"
                });
            }
            // S-101 split the mooring facility by its category
            "MORFAC" => {
                return match o.get("CATMOR").and_then(|c| c.split(',').next()).map(str::trim) {
                    Some("1" | "2") => pick("Dolphin"),
                    Some("3") => pick("Bollard"),
                    Some("5") => pick("Pile"),
                    Some("6") if prim == "surface" => pick("MooringTrot"),
                    Some("7") => pick("MooringBuoy"),
                    // no (or another) category: an area of moorings
                    _ if prim == "surface" => pick("MooringArea"),
                    // S-52's default for a mooring point (MORFAC03) is the dolphin's
                    _ if prim == "point" => pick("Dolphin"),
                    // a tie-up wall or a trot as a line: S-101 has no mooring line
                    _ if prim == "curve" => pick("ShorelineConstruction"),
                    _ => None,
                };
            }
            // S-101 has one feature for both
            "TSEZNE" | "TSELNE" => return pick("SeparationZoneOrLine"),
            "DRGARE" => return pick("DredgedArea"),
            "UNSARE" => return pick("UnsurveyedArea"),
            "CTNARE" => return pick("CautionArea"),
            "BRIDGE" => return pick("Bridge"),
            "ADMARE" => return pick("AdministrationArea"),
            _ => {}
        }
        let all = self.fc.feature_for_s57(&class);
        all.iter().copied().find(|f| f.primitives.iter().any(|p| p == prim)).or_else(|| all.first().copied())
    }

    fn binds<'b>(&self, container: &'b str) -> Option<&'a [Binding]> {
        if let Some(f) = self.fc.features.get(container) {
            return Some(&f.bindings);
        }
        self.fc.complex.get(container).map(|c| c.bindings.as_slice())
    }

    /// Shortest chain of complex attributes from `container` to one that binds `target`
    /// (a simple or complex attribute code). Empty when the container binds it itself.
    fn chain(&self, container: &str, target: &str) -> Option<Vec<String>> {
        let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::from([(container.to_string(), vec![])]);
        while let Some((c, path)) = queue.pop_front() {
            let Some(bs) = self.binds(&c) else { continue };
            if bs.iter().any(|b| b.code == target) {
                return Some(path);
            }
            if path.len() >= 5 {
                continue;
            }
            for b in bs {
                if self.fc.complex.contains_key(&b.code) && !path.contains(&b.code) {
                    let mut p = path.clone();
                    p.push(b.code.clone());
                    queue.push_back((b.code.clone(), p));
                }
            }
        }
        None
    }

    /// Where S-57 attribute `acronym` goes in feature type `code`.
    fn place(&self, code: &str, acronym: &str) -> Option<Place> {
        let key = (code.to_string(), acronym.to_string());
        if let Some(p) = self.places.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return p.clone();
        }
        let mut found = None;
        // an attribute S-101 made complex (SECTR1 is sectorLimitOne): its simple part
        // with the same alias
        for ca in self.fc.complex.values().filter(|c| c.aliases.iter().any(|a| a.eq_ignore_ascii_case(acronym))) {
            let Some(mut path) = self.chain(code, &ca.code) else { continue };
            path.push(ca.code.clone());
            let inner = ca.bindings.iter().find(|b| self.fc.simple.get(&b.code).is_some_and(|s| s.aliases.iter().any(|a| a.eq_ignore_ascii_case(acronym))));
            if let Some(b) = inner {
                found = Some((path, b.code.clone()));
                break;
            }
        }
        if found.is_none() {
            let mut best: Option<Place> = None;
            for sa in self.fc.simple_for_s57(acronym) {
                if let Some(path) = self.chain(code, &sa.code) {
                    if best.as_ref().is_none_or(|(p, _)| path.len() < p.len()) {
                        best = Some((path, sa.code.clone()));
                    }
                }
            }
            found = best;
        }
        self.places.lock().unwrap_or_else(|e| e.into_inner()).insert(key, found.clone());
        found
    }

    /// Whether the binding of `code` in `container` takes more than one value.
    fn many(&self, container: &str, code: &str) -> bool {
        self.binds(container).and_then(|bs| bs.iter().find(|b| b.code == code)).is_some_and(|b| b.upper != Some(1))
    }

    /// An S-57 value as S-101 encodes it.
    fn values(&self, code: &str, raw: &str, many: bool) -> Vec<String> {
        let ty = self.fc.simple.get(code).map(|s| s.value_type.as_str()).unwrap_or("text");
        let mut v: Vec<String> = match ty {
            "enumeration" | "integer" => raw.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
            "real" => vec![trim_real(raw)],
            // S-57 encoded these as an enumeration: 1 is yes
            "boolean" => vec![if matches!(raw.trim(), "1" | "true") { "1".into() } else { "0".into() }],
            _ => vec![raw.trim().to_string()],
        };
        if !many {
            v.truncate(1);
        }
        v
    }

    /// Put a simple value at `path` (complex attributes, one instance each) in `attrs`.
    fn put(attrs: &mut Attrs, path: &[String], code: &str, vals: Vec<String>) {
        let mut a = attrs;
        for c in path {
            let entry = a.entry(c.clone()).or_insert_with(|| Value::Complex(vec![Attrs::new()]));
            if let Value::Simple(_) = entry {
                *entry = Value::Complex(vec![Attrs::new()]);
            }
            let Value::Complex(items) = entry else { unreachable!() };
            if items.is_empty() {
                items.push(Attrs::new());
            }
            a = &mut items[0];
        }
        a.insert(code.to_string(), Value::Simple(vals));
    }

    /// Convert one S-57 object. `surrounding_depth`: for a rock, wreck or obstruction,
    /// the DRVAL1 of the depth area it lies in (S-101 records it on the danger).
    pub fn convert(&self, id: &str, o: &S57Object, surrounding_depth: Option<f64>) -> Feature {
        let Some(ft) = self.feature_type(o) else {
            // unknown to S-101: the rules give it the unknown-object symbol
            return Feature { id: id.into(), code: o.class.to_ascii_uppercase(), attrs: Attrs::new(), geometry: o.geometry.clone() };
        };
        let mut attrs = Attrs::new();
        for (k, v) in &o.attrs {
            let acr = k.to_ascii_uppercase();
            if v.trim().is_empty() || SKIP.contains(&acr.as_str()) {
                continue;
            }
            let Some((path, code)) = self.place(&ft.code, &acr) else { continue };
            let container = path.last().map(String::as_str).unwrap_or(&ft.code);
            let vals = self.values(&code, v, self.many(container, &code));
            if !vals.is_empty() {
                Self::put(&mut attrs, &path, &code, vals);
            }
        }
        if let Some(d) = surrounding_depth {
            if ft.bindings.iter().any(|b| b.code == "surroundingDepth") {
                attrs.insert("surroundingDepth".into(), Value::Simple(vec![trim_real(&d.to_string())]));
            }
        }
        Feature { id: id.into(), code: ft.code.clone(), attrs, geometry: o.geometry.clone() }
    }

    /// Put a TOPMAR object's shape and colours on the buoy or beacon it stands on.
    pub fn add_topmark(&self, f: &mut Feature, topmark: &S57Object) {
        let Some(ft) = self.fc.features.get(&f.code) else { return };
        if !ft.bindings.iter().any(|b| b.code == "topmark") {
            return;
        }
        let mut t = Attrs::new();
        for (acr, code) in [("TOPSHP", "topmarkDaymarkShape"), ("COLOUR", "colour"), ("COLPAT", "colourPattern")] {
            if let Some(v) = topmark.get(acr) {
                let vals = self.values(code, v, self.many("topmark", code));
                if !vals.is_empty() {
                    t.insert(code.into(), Value::Simple(vals));
                }
            }
        }
        if !t.is_empty() {
            f.attrs.insert("topmark".into(), Value::Complex(vec![t]));
        }
    }

    /// The S-57 records of one sector light (one per sector, at one position) as one
    /// LightSectored: records with the same rhythm share a sectorCharacteristics, each
    /// sector is a lightSector in it.
    pub fn sectored_light(&self, id: &str, records: &[S57Object]) -> Feature {
        let mut out = self.convert(id, &records[0], None);
        for r in &records[1..] {
            let f = self.convert(id, r, None);
            let Some(Value::Complex(add)) = f.attrs.get("sectorCharacteristics").cloned() else { continue };
            let entry = out.attrs.entry("sectorCharacteristics".into()).or_insert_with(|| Value::Complex(vec![]));
            let Value::Complex(have) = entry else { continue };
            for sc in add {
                let same = |a: &Attrs| ["lightCharacteristic", "signalGroup", "signalPeriod"].iter().all(|k| simple(a, k) == simple(&sc, k));
                match have.iter_mut().find(|h| same(h)) {
                    Some(h) => {
                        if let (Some(Value::Complex(hs)), Some(Value::Complex(ns))) = (h.get_mut("lightSector"), sc.get("lightSector")) {
                            hs.extend(ns.iter().cloned());
                        }
                    }
                    None => have.push(sc),
                }
            }
        }
        out
    }
}

fn simple<'b>(a: &'b Attrs, k: &str) -> Option<&'b Vec<String>> {
    match a.get(k) {
        Some(Value::Simple(v)) => Some(v),
        _ => None,
    }
}

fn trim_real(s: &str) -> String {
    match s.trim().parse::<f64>() {
        Ok(x) => {
            let t = format!("{x}");
            if t.contains('e') {
                format!("{x:.6}")
            } else {
                t
            }
        }
        Err(_) => s.trim().to_string(),
    }
}
