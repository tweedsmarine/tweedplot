//! An S-100 feature catalogue (the S-101 one): the feature, information, attribute,
//! association and role types the portrayal rules ask about, with the S-57 acronyms
//! (aliases) they correspond to.

use std::collections::BTreeMap;

use crate::dom::{self, Node};

#[derive(Debug, Clone)]
pub struct Binding {
    /// Attribute code.
    pub code: String,
    pub lower: u32,
    /// None: unbounded.
    pub upper: Option<u32>,
    pub sequential: bool,
    pub permitted: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct SimpleAttribute {
    pub code: String,
    pub name: String,
    pub aliases: Vec<String>,
    /// "boolean", "enumeration", "integer", "real", "text", "date", ...
    pub value_type: String,
    pub listed: Vec<(i64, String)>,
}

#[derive(Debug, Clone)]
pub struct ComplexAttribute {
    pub code: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone)]
pub struct FeatureType {
    pub code: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub is_abstract: bool,
    pub use_type: String,
    pub primitives: Vec<String>,
    pub bindings: Vec<Binding>,
    pub super_type: Option<String>,
    pub sub_types: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InformationType {
    pub code: String,
    pub name: String,
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone)]
pub struct Association {
    pub code: String,
    pub name: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Default)]
pub struct FeatureCatalogue {
    pub version: String,
    pub simple: BTreeMap<String, SimpleAttribute>,
    pub complex: BTreeMap<String, ComplexAttribute>,
    pub features: BTreeMap<String, FeatureType>,
    pub information: BTreeMap<String, InformationType>,
    pub feature_associations: BTreeMap<String, Association>,
    pub information_associations: BTreeMap<String, Association>,
    pub roles: Vec<String>,
}

fn aliases(n: &Node) -> Vec<String> {
    n.all("alias").map(|a| a.text.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

fn bindings(n: &Node, name: &str) -> Vec<Binding> {
    n.all(name)
        .filter_map(|b| {
            let code = b.child("attribute")?.attr("ref")?.to_string();
            let m = b.child("multiplicity");
            let lower = m.and_then(|m| m.text_of("lower")).and_then(|s| s.parse().ok()).unwrap_or(0);
            let upper = m.and_then(|m| m.child("upper")).and_then(|u| if u.attr("infinite") == Some("true") { None } else { u.text.trim().parse().ok() });
            let permitted = b.child("permittedValues").map(|p| p.all("value").filter_map(|v| v.text.trim().parse().ok()).collect()).unwrap_or_default();
            Some(Binding { code, lower, upper, sequential: b.attr("sequential") == Some("true"), permitted })
        })
        .collect()
}

impl FeatureCatalogue {
    pub fn parse(text: &str) -> Result<Self, String> {
        let root = dom::parse(text)?;
        let mut fc = FeatureCatalogue { version: root.text_of("versionNumber").unwrap_or_default(), ..Default::default() };
        let name = |n: &Node| n.text_of("name").unwrap_or_default();
        let code = |n: &Node| n.text_of("code").unwrap_or_default();
        if let Some(s) = root.child("S100_FC_SimpleAttributes") {
            for a in s.all("S100_FC_SimpleAttribute") {
                let listed = a
                    .child("listedValues")
                    .map(|l| l.all("listedValue").filter_map(|v| Some((v.text_of("code")?.parse().ok()?, v.text_of("label").unwrap_or_default()))).collect())
                    .unwrap_or_default();
                let sa = SimpleAttribute { code: code(a), name: name(a), aliases: aliases(a), value_type: a.text_of("valueType").unwrap_or_default(), listed };
                fc.simple.insert(sa.code.clone(), sa);
            }
        }
        if let Some(s) = root.child("S100_FC_ComplexAttributes") {
            for a in s.all("S100_FC_ComplexAttribute") {
                let ca = ComplexAttribute { code: code(a), name: name(a), aliases: aliases(a), bindings: bindings(a, "subAttributeBinding") };
                fc.complex.insert(ca.code.clone(), ca);
            }
        }
        if let Some(s) = root.child("S100_FC_Roles") {
            fc.roles = s.all("S100_FC_Role").map(code).collect();
        }
        for (list, item, out) in [
            ("S100_FC_FeatureAssociations", "S100_FC_FeatureAssociation", &mut fc.feature_associations),
            ("S100_FC_InformationAssociations", "S100_FC_InformationAssociation", &mut fc.information_associations),
        ] {
            if let Some(s) = root.child(list) {
                for a in s.all(item) {
                    let roles = a.all("role").filter_map(|r| r.attr("ref").map(str::to_string)).collect();
                    out.insert(code(a), Association { code: code(a), name: name(a), roles });
                }
            }
        }
        if let Some(s) = root.child("S100_FC_InformationTypes") {
            for t in s.all("S100_FC_InformationType") {
                fc.information.insert(code(t), InformationType { code: code(t), name: name(t), bindings: bindings(t, "attributeBinding") });
            }
        }
        if let Some(s) = root.child("S100_FC_FeatureTypes") {
            for t in s.all("S100_FC_FeatureType") {
                let ft = FeatureType {
                    code: code(t),
                    name: name(t),
                    aliases: aliases(t),
                    is_abstract: t.attr("isAbstract") == Some("true"),
                    use_type: t.text_of("featureUseType").unwrap_or_default(),
                    primitives: t.all("permittedPrimitives").map(|p| p.text.trim().to_string()).collect(),
                    bindings: bindings(t, "attributeBinding"),
                    super_type: t.child("superType").map(|s| s.text.trim().to_string()).filter(|s| !s.is_empty()),
                    sub_types: t.all("subType").map(|s| s.text.trim().to_string()).collect(),
                };
                fc.features.insert(ft.code.clone(), ft);
            }
        }
        if fc.features.is_empty() {
            return Err("no feature types: not an S-100 feature catalogue".into());
        }
        // inherited attribute bindings: a feature type has its super types' too
        let codes: Vec<String> = fc.features.keys().cloned().collect();
        for c in codes {
            let mut extra = Vec::new();
            let mut sup = fc.features[&c].super_type.clone();
            while let Some(s) = sup {
                let Some(st) = fc.features.get(&s) else { break };
                extra.extend(st.bindings.iter().cloned());
                sup = st.super_type.clone();
            }
            let f = fc.features.get_mut(&c).unwrap();
            for b in extra {
                if !f.bindings.iter().any(|x| x.code == b.code) {
                    f.bindings.push(b);
                }
            }
        }
        Ok(fc)
    }

    /// The S-101 feature type for an S-57 object class acronym ("BOYCAR" → CardinalBuoy).
    pub fn feature_for_s57(&self, acronym: &str) -> Vec<&FeatureType> {
        self.features.values().filter(|f| !f.is_abstract && f.aliases.iter().any(|a| a == acronym)).collect()
    }

    /// The S-101 simple attribute for an S-57 attribute acronym ("CATCAM" → categoryOfCardinalMark).
    pub fn simple_for_s57(&self, acronym: &str) -> Vec<&SimpleAttribute> {
        self.simple.values().filter(|a| a.aliases.iter().any(|x| x.eq_ignore_ascii_case(acronym))).collect()
    }
}
