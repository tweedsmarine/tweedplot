use std::path::Path;

use portrayal::{Catalogue, Converter, FeatureCatalogue, Geometry, Portrayer, S57Object, Value};

fn res(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/resources").join(p)
}
fn fc() -> FeatureCatalogue {
    FeatureCatalogue::parse(&std::fs::read_to_string(res("s101-fc/101_Feature_Catalogue_2.1.0.20260612_Unsigned.xml")).unwrap()).unwrap()
}
fn obj(class: &str, attrs: &[(&str, &str)], g: Geometry) -> S57Object {
    S57Object { class: class.into(), attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), geometry: Some(g) }
}

#[test]
fn attributes_go_where_s101_keeps_them() {
    let fc = fc();
    let c = Converter::new(&fc);
    let p = Geometry::Point([173.0, -41.0]);
    let mut b = c.convert(
        "b",
        &obj("BOYLAT", &[("boyshp", "2"), ("catlam", "1"), ("colour", "3"), ("objnam", "No 3"), ("scamin", "59999"), ("sorind", "x")], p.clone()),
        None,
    );
    assert_eq!(b.code, "LateralBuoy");
    let Some(Value::Complex(names)) = b.attrs.get("featureName") else { panic!("{:?}", b.attrs) };
    assert!(matches!(names[0].get("name"), Some(Value::Simple(v)) if v == &vec!["No 3".to_string()]));
    assert!(matches!(b.attrs.get("scaleMinimum"), Some(Value::Simple(v)) if v == &vec!["59999".to_string()]));
    assert!(!b.attrs.contains_key("sourceIndication"));
    c.add_topmark(&mut b, &obj("TOPMAR", &[("topshp", "3"), ("colour", "3")], p.clone()));
    assert!(matches!(b.attrs.get("topmark"), Some(Value::Complex(t)) if t[0].contains_key("topmarkDaymarkShape")));

    // a sector light: three records, one feature with one rhythm and three sectors
    let rec = |col: &str, s1: &str, s2: &str| {
        obj("LIGHTS", &[("litchr", "2"), ("siggrp", "(2)"), ("sigper", "10"), ("colour", col), ("sectr1", s1), ("sectr2", s2), ("valnmr", "9")], p.clone())
    };
    let l = c.sectored_light("l", &[rec("3", "90", "120"), rec("1", "120", "130"), rec("4", "130", "160")]);
    assert_eq!(l.code, "LightSectored");
    let Some(Value::Complex(sc)) = l.attrs.get("sectorCharacteristics") else { panic!("{:?}", l.attrs) };
    assert_eq!(sc.len(), 1);
    let Some(Value::Complex(sectors)) = sc[0].get("lightSector") else { panic!() };
    assert_eq!(sectors.len(), 3);
    let Some(Value::Complex(lim)) = sectors[0].get("sectorLimit") else { panic!("{:?}", sectors[0]) };
    assert!(lim[0].contains_key("sectorLimitOne") && lim[0].contains_key("sectorLimitTwo"));

    // and the rules draw it as a sector light, with legs and arcs
    let cat = Catalogue::open(&res("s101-pc")).unwrap();
    let pr = Portrayer::new(&cat, self::fc()).unwrap();
    let out = pr.portray(&[l, b], &mut Vec::new()).unwrap();
    let light = &out.iter().find(|o| o.id == "l").unwrap().instructions;
    assert!(light.contains("AugmentedRay") && light.contains("ArcByRadius"), "{light}");
    assert!(light.contains("LITRD") && light.contains("LITGN"), "{light}");
    let buoy = &out.iter().find(|o| o.id == "b").unwrap().instructions;
    assert!(buoy.contains("PointInstruction:BOYCAN") && buoy.contains("TOPMAR"), "{buoy}");
}

#[test]
fn the_iho_test_cell_converts() {
    let cell = enc_s57::cell::Cell::from_bytes(include_bytes!("../../chart/tests/data/1B5X02NE.000"), &[]).unwrap();
    let fc = fc();
    let c = Converter::new(&fc);
    let mut features = Vec::new();
    for f in cell.features.values() {
        let Some(acr) = enc_s57::catalogue::object(f.objl) else { continue };
        if acr.starts_with("M_") {
            continue;
        }
        let attrs: Vec<(String, String)> = f.attf.iter().filter_map(|(k, v)| Some((enc_s57::catalogue::attribute(*k)?.0.to_string(), v.clone()))).collect();
        let g = match cell.shape(f) {
            Some(enc_s57::cell::Shape::Points(p)) if acr == "SOUNDG" => Geometry::MultiPoint(p),
            Some(enc_s57::cell::Shape::Points(p)) => Geometry::Point([p[0][0], p[0][1]]),
            Some(enc_s57::cell::Shape::Lines(l)) => Geometry::Curve(l[0].clone()),
            Some(enc_s57::cell::Shape::Polygons(p)) => Geometry::Surface(p[0].clone()),
            None => continue,
        };
        features.push(c.convert(&format!("{acr}/{}", f.rcid), &S57Object { class: acr.into(), attrs, geometry: Some(g) }, None));
    }
    let codes: Vec<&str> = features.iter().map(|f| f.code.as_str()).collect();
    for want in ["DepthArea", "DepthContour", "Sounding", "LandArea", "Coastline"] {
        assert!(codes.contains(&want), "{want} in {codes:?}");
    }
    let cat = Catalogue::open(&res("s101-pc")).unwrap();
    let mut pr = Portrayer::new(&cat, self::fc()).unwrap();
    pr.set("SafetyContour", "5");
    let mut trace = Vec::new();
    let out = pr.portray(&features, &mut trace).unwrap();
    assert!(trace.is_empty(), "{trace:?}");
    let get =
        |code: &str| features.iter().filter(|f| f.code == code).map(|f| out.iter().find(|o| o.id == f.id).unwrap().instructions.clone()).collect::<Vec<_>>();
    assert!(get("DepthArea").iter().all(|i| i.contains("ColorFill:DEP")), "{:?}", get("DepthArea"));
    assert!(get("Sounding").iter().all(|i| i.contains("SOUNDS")), "{:?}", get("Sounding"));
    assert!(get("LandArea").iter().all(|i| i.contains("ColorFill:LANDA")), "{:?}", get("LandArea"));
}

/// S-52 UDWHAZ05: a 2 m rock is an isolated danger in 20 m of water, not in 2 m of water
/// unless "shallow water dangers" is on (the catalogue's default; TweedPlot's is off).
#[test]
fn isolated_dangers_follow_the_surrounding_depth() {
    let fc = fc();
    let c = Converter::new(&fc);
    let cat = Catalogue::open(&res("s101-pc")).unwrap();
    let mut p = Portrayer::new(&cat, self::fc()).unwrap();
    p.set("SafetyContour", "5");
    let rock = |id: &str, around: Option<f64>| c.convert(id, &obj("UWTROC", &[("watlev", "3"), ("valsou", "2")], Geometry::Point([173.0, -41.0])), around);
    let fs = vec![rock("deep", Some(20.0)), rock("shallow", Some(2.0)), rock("drying", Some(-1.0))];
    let iso = |p: &Portrayer| -> Vec<bool> { p.portray(&fs, &mut Vec::new()).unwrap().iter().map(|o| o.instructions.contains("ISODGR01")).collect() };
    p.set("ShallowWaterDangers", "false");
    assert_eq!(iso(&p), [true, false, false]);
    p.set("ShallowWaterDangers", "true");
    assert_eq!(iso(&p), [true, true, false]);
}
