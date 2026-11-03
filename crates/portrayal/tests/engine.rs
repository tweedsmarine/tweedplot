use std::path::Path;

use portrayal::{Attrs, Catalogue, Feature, FeatureCatalogue, Geometry, Portrayer, Value};

fn res(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources").join(p)
}

fn portrayer() -> Portrayer {
    let cat = Catalogue::open(&res("s101-pc")).unwrap();
    let text = std::fs::read_to_string(res("s101-fc/101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml")).unwrap();
    Portrayer::new(&cat, FeatureCatalogue::parse(&text).unwrap()).unwrap()
}

fn s(v: &[&str]) -> Value {
    Value::Simple(v.iter().map(|x| x.to_string()).collect())
}

fn attrs(a: &[(&str, &[&str])]) -> Attrs {
    a.iter().map(|(k, v)| (k.to_string(), s(v))).collect()
}

#[test]
fn feature_catalogue_maps_s57() {
    let p = portrayer();
    let fc = p.feature_catalogue();
    assert_eq!(fc.version, "2.1.0");
    assert_eq!(fc.feature_for_s57("BOYCAR")[0].code, "CardinalBuoy");
    assert_eq!(fc.simple_for_s57("CATCAM")[0].code, "categoryOfCardinalMark");
    assert!(p.context().iter().any(|c| c.id == "SafetyContour"));
}

#[test]
fn rules_portray_features() {
    let mut p = portrayer();
    p.set("SafetyContour", "5");
    p.set("SafetyDepth", "3");
    let fs = vec![
        Feature {
            id: "buoy".into(),
            code: "CardinalBuoy".into(),
            attrs: attrs(&[("buoyShape", &["4"]), ("categoryOfCardinalMark", &["1"]), ("colour", &["2", "6"]), ("colourPattern", &["1"])]),
            geometry: Some(Geometry::Point([173.25, -41.25])),
        },
        Feature {
            id: "depth".into(),
            code: "DepthArea".into(),
            attrs: attrs(&[("depthRangeMinimumValue", &["2"]), ("depthRangeMaximumValue", &["5"])]),
            geometry: Some(Geometry::Surface(vec![vec![[173.0, -41.0], [173.1, -41.0], [173.1, -41.1], [173.0, -41.0]]])),
        },
        Feature {
            id: "rock".into(),
            code: "UnderwaterAwashRock".into(),
            attrs: attrs(&[("waterLevelEffect", &["3"]), ("valueOfSounding", &["1.5"])]),
            geometry: Some(Geometry::Point([173.2, -41.2])),
        },
    ];
    let mut trace = Vec::new();
    let out = p.portray(&fs, &mut trace).unwrap();
    for o in &out {
        println!("{}: {}", o.id, o.instructions);
    }
    for t in &trace {
        println!("trace: {t}");
    }
    let get = |id: &str| out.iter().find(|o| o.id == id).map(|o| o.instructions.clone()).unwrap_or_default();
    assert!(get("buoy").contains("PointInstruction:BOY"), "{}", get("buoy"));
    assert!(get("depth").contains("ColorFill:DEP"), "{}", get("depth"));
    assert!(get("rock").contains("PointInstruction:"), "{}", get("rock"));
}

/// The catalogue under a folder whose name holds a '?', as a Windows install path does
/// (\\?\C:\...): the rules must still be found.
#[cfg(unix)]
#[test]
fn rules_found_under_a_path_with_a_question_mark() {
    let dir = std::env::temp_dir().join(format!("tp?{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("s101-pc");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(res("s101-pc").canonicalize().unwrap(), &link).unwrap();
    let cat = Catalogue::open(&link).unwrap();
    let text = std::fs::read_to_string(res("s101-fc/101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml")).unwrap();
    let p = Portrayer::new(&cat, FeatureCatalogue::parse(&text).unwrap()).unwrap();
    let f = Feature {
        id: "buoy".into(),
        code: "CardinalBuoy".into(),
        attrs: attrs(&[("buoyShape", &["4"]), ("categoryOfCardinalMark", &["1"]), ("colour", &["2", "6"]), ("colourPattern", &["1"])]),
        geometry: Some(Geometry::Point([173.25, -41.25])),
    };
    let out = p.portray(&[f], &mut Vec::new());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.unwrap()[0].instructions.contains("PointInstruction:BOYPIL"));
}
