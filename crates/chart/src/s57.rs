//! LINZ hydrographic feature layers (ENC content as GeoPackages, S-57 attributes).
//!
//! Each LINZ table such as `light_points_hydro_190k_1350k` maps to an S-57
//! object class (`LIGHTS`). This module knows which tables to draw, turns their
//! attributes into a chart icon and label for the vector tiles, and decodes the
//! S-57 codes into words for the feature details shown on a click.

pub use enc_s57::classes::{class_for, FeatureClass, Geo, CLASSES, UNKNOWN};

/// Attribute columns read for the tiles (whichever a table has).
pub const TILE_COLS: &[&str] = &[
    "objnam", "colour", "colpat", "boyshp", "bcnshp", "catlam", "catcam", "watlev", "catwrk", "valsou", "litchr", "siggrp", "sigper", "valnmr", "height",
    "sectr1", "sectr2", "orient", "scamin", "catlit", "status", "catspm", "catobs",
];

/// One feature's attributes, indexed like [`TILE_COLS`].
pub struct Attrs<'a>(pub &'a [Option<String>]);

impl Attrs<'_> {
    pub fn get(&self, name: &str) -> Option<&str> {
        let i = TILE_COLS.iter().position(|c| *c == name)?;
        self.0[i].as_deref().map(str::trim).filter(|s| !s.is_empty())
    }
    pub fn num(&self, name: &str) -> Option<f64> {
        self.get(name)?.parse().ok()
    }
    /// First code of a list attribute such as `colour` ("3,1,3").
    pub fn first(&self, name: &str) -> Option<u32> {
        self.get(name)?.split(',').next()?.trim().parse().ok()
    }
}

/// Map zoom (512-pixel tiles) from which a feature with this SCAMIN shows at latitude `lat`.
pub fn scamin_zoom(scamin: f64, lat: f64) -> f64 {
    if scamin <= 0.0 {
        return 0.0;
    }
    // ground metres per screen pixel at zoom z: 40 075 017 cos(lat) / (512 * 2^z); 0.26 mm screen pixels
    let k = 40_075_016.686 * lat.to_radians().cos() / (512.0 * 0.000_264_6 * scamin);
    k.log2().max(0.0)
}

/// Chart icon for a point feature (drawn by the front end, see `chartfeatures.js`).
pub fn icon(code: &str, a: &Attrs) -> Option<String> {
    let cols = a.get("colour").unwrap_or("").replace(' ', "");
    let pat = a.get("colpat").unwrap_or("");
    let top = match code {
        "BOYCAR" | "BCNCAR" => match a.first("catcam") {
            Some(1) => "n",
            Some(2) => "e",
            Some(3) => "s",
            Some(4) => "w",
            _ => "",
        },
        "BOYISD" | "BCNISD" => "isd",
        "BOYSAW" | "BCNSAW" => "saw",
        "BOYSPP" | "BCNSPP" => "spp",
        "BCNLAT" => match a.first("catlam") {
            Some(1) | Some(4) => "can",
            Some(2) | Some(3) => "cone",
            _ => "",
        },
        _ => "",
    };
    Some(match code {
        "BOYLAT" | "BOYCAR" | "BOYISD" | "BOYSAW" | "BOYSPP" | "BOYINB" => {
            let shape = a.first("boyshp").unwrap_or(match code {
                "BOYCAR" | "BOYISD" => 4,
                "BOYSAW" => 3,
                _ => 4,
            });
            format!("b|{shape}|{cols}|{pat}|{top}")
        }
        "BCNLAT" | "BCNCAR" | "BCNISD" | "BCNSAW" | "BCNSPP" => {
            let shape = a.first("bcnshp").unwrap_or(1);
            format!("c|{shape}|{cols}|{pat}|{top}")
        }
        "DAYMAR" => format!("d|{cols}"),
        "UWTROC" => format!("r|{}", a.first("watlev").unwrap_or(3)),
        "WRECKS" => format!("w|{}", a.first("catwrk").unwrap_or(if a.num("valsou").is_some() { 1 } else { 2 })),
        "OBSTRN" => format!("o|{}", a.first("watlev").unwrap_or(3)),
        "LNDMRK" => "lm".into(),
        "RTPBCN" => "rc".into(),
        "PILBOP" => "pb".into(),
        "ACHARE" => "an".into(),
        "ACHBRT" => "ab".into(),
        "MORFAC" => "mo".into(),
        "PILPNT" => "pi".into(),
        "OFSPLF" => "pf".into(),
        "MARCUL" => "mf".into(),
        "CTNARE" => "ca".into(),
        "RDOCAL" => "cp".into(),
        "RSCSTA" => "rs".into(),
        "SISTAT" => "ss".into(),
        "SMCFAC" | "HRBFAC" => "hf".into(),
        "FOGSIG" => "fg".into(),
        UNKNOWN => "uk".into(),
        _ => return None,
    })
}

/// Short text beside a point feature on the chart.
pub fn label(code: &str, a: &Attrs) -> Option<String> {
    let s = match code {
        "UWTROC" | "WRECKS" | "OBSTRN" => fmt_depth(a.num("valsou")?),
        "SEAARE" | "LNDRGN" => a.get("objnam")?.to_string(),
        "LIGHTS" | "FOGSIG" | "PILBOP" | "MORFAC" | "PILPNT" | "SMCFAC" | "HRBFAC" | "SISTAT" | "RSCSTA" | "ACHBRT" | "CTNARE" => return None,
        _ => a.get("objnam")?.to_string(),
    };
    Some(s)
}

fn fmt_depth(d: f64) -> String {
    if d < 0.0 {
        // drying height, underlined on paper charts
        format!("dries {}", trim_num(-d))
    } else {
        trim_num(d)
    }
}

fn trim_num(v: f64) -> String {
    let s = format!("{:.1}", v);
    s.trim_end_matches(".0").to_string()
}

/// One light record, for building the chart label of all the lights at one spot.
#[derive(Debug, Clone, Default)]
pub struct Light {
    pub litchr: Option<u32>,
    pub siggrp: Option<String>,
    pub sigper: Option<f64>,
    pub colours: Vec<u32>,
    pub height: Option<f64>,
    pub range: Option<f64>,
    pub sectors: Option<(f64, f64)>,
    pub orient: Option<f64>,
}

impl Light {
    pub fn from_attrs(a: &Attrs) -> Self {
        Light {
            litchr: a.first("litchr"),
            siggrp: a.get("siggrp").map(str::to_string),
            sigper: a.num("sigper"),
            colours: a.get("colour").map(|c| c.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default(),
            height: a.num("height"),
            range: a.num("valnmr"),
            sectors: a.num("sectr1").zip(a.num("sectr2")),
            orient: a.num("orient"),
        }
    }
}

pub fn litchr_abbr(c: u32) -> &'static str {
    match c {
        1 => "F",
        2 => "Fl",
        3 => "LFl",
        4 => "Q",
        5 => "VQ",
        6 => "UQ",
        7 => "Iso",
        8 => "Oc",
        9 => "IQ",
        10 => "IVQ",
        11 => "IUQ",
        12 => "Mo",
        13 => "FFl",
        14 => "Fl+LFl",
        15 => "OcFl",
        16 => "FLFl",
        17 => "Al.Oc",
        18 => "Al.LFl",
        19 => "Al.Fl",
        20 => "Al.Gr",
        25 => "Q+LFl",
        26 => "VQ+LFl",
        27 => "UQ+LFl",
        28 => "Al",
        29 => "Al.FFl",
        _ => "",
    }
}

pub fn colour_abbr(c: u32) -> &'static str {
    match c {
        1 => "W",
        2 => "Bk",
        3 => "R",
        4 => "G",
        5 => "Bu",
        6 => "Y",
        7 => "Gy",
        8 => "Br",
        9 => "Am",
        10 => "Vi",
        11 => "Or",
        12 => "Mg",
        13 => "Pk",
        _ => "",
    }
}

/// Chart label for the lights at one position, paper-chart style:
/// `Fl(2) WRG 10s 15m 12M`. Records with the same rhythm are merged (sector lights).
pub fn light_label(lights: &[Light]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut done = vec![false; lights.len()];
    for i in 0..lights.len() {
        if done[i] {
            continue;
        }
        let l = &lights[i];
        let js: Vec<usize> =
            (i..lights.len()).filter(|&j| !done[j] && lights[j].litchr == l.litchr && lights[j].siggrp == l.siggrp && lights[j].sigper == l.sigper).collect();
        for &j in &js {
            done[j] = true;
        }
        let same: Vec<&Light> = js.iter().map(|&j| &lights[j]).collect();
        let mut s = String::from(l.litchr.map(litchr_abbr).unwrap_or(""));
        if let Some(g) = &l.siggrp {
            let g = g.trim();
            if !g.is_empty() && g != "()" && g != "(1)" && !s.is_empty() {
                s.push_str(g);
            }
        }
        let mut cols: Vec<u32> = same.iter().flat_map(|x| x.colours.iter().copied()).collect();
        // white, red, green first, as on charts
        let rank = |c: &u32| match c {
            1 => 0,
            3 => 1,
            4 => 2,
            _ => 3 + *c,
        };
        cols.sort_by_key(rank);
        cols.dedup();
        let cs: String = cols.iter().map(|c| colour_abbr(*c)).collect();
        if !cs.is_empty() {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(&cs);
        }
        if let Some(p) = l.sigper {
            s.push_str(&format!(" {}s", trim_num(p)));
        }
        if let Some(h) = same.iter().filter_map(|x| x.height).reduce(f64::max) {
            s.push_str(&format!(" {}m", trim_num(h)));
        }
        let ranges: Vec<f64> = same.iter().filter_map(|x| x.range).collect();
        if let (Some(lo), Some(hi)) = (ranges.iter().copied().reduce(f64::min), ranges.iter().copied().reduce(f64::max)) {
            if (hi - lo).abs() < 0.05 {
                s.push_str(&format!(" {}M", trim_num(hi)));
            } else {
                s.push_str(&format!(" {}-{}M", trim_num(hi), trim_num(lo)));
            }
        }
        parts.push(s.trim().to_string());
    }
    parts.retain(|p| !p.is_empty());
    parts.dedup();
    parts.join(" / ")
}

/// Colour key for a light flare or sector: r, g, w (white and yellow), or o (other).
pub fn light_colour(colours: &[u32]) -> &'static str {
    match colours.first() {
        Some(3) => "r",
        Some(4) => "g",
        Some(1) | Some(6) | None => "w",
        _ => "o",
    }
}

// ---------- details for people ----------------------------------------------------------

/// Words for an S-57 attribute acronym.
pub fn attr_name(a: &str) -> String {
    let s = match a {
        "objl" => "S-57 object class",
        "objnam" => "Name",
        "nobjnm" => "Name (local)",
        "colour" => "Colour",
        "colpat" => "Colour pattern",
        "litchr" => "Light character",
        "siggrp" => "Signal group",
        "sigper" => "Period (s)",
        "sigseq" => "Signal sequence",
        "valnmr" => "Nominal range (M)",
        "height" => "Height (m)",
        "elevat" => "Elevation (m)",
        "verlen" => "Vertical length (m)",
        "sectr1" => "Sector from (°, from seaward)",
        "sectr2" => "Sector to (°, from seaward)",
        "orient" => "Orientation (°)",
        "catlit" => "Light category",
        "mltylt" => "Number of lights",
        "exclit" => "Exhibited",
        "litvis" => "Visibility",
        "valsou" => "Depth (m)",
        "watlev" => "Water level",
        "catwrk" => "Wreck category",
        "catobs" => "Obstruction category",
        "catrea" => "Restricted area category",
        "restrn" => "Restrictions",
        "status" => "Status",
        "inform" => "Information",
        "ninfom" => "Information (local)",
        "txtdsc" => "Text file",
        "ntxtds" => "Text file (local)",
        "scamin" => "Shown from scale 1:",
        "sordat" => "Source date",
        "sorind" => "Source",
        "datsta" => "Valid from",
        "datend" => "Valid to",
        "persta" => "Period start",
        "perend" => "Period end",
        "boyshp" => "Buoy shape",
        "bcnshp" => "Beacon shape",
        "catlam" => "Lateral category",
        "catcam" => "Cardinal category",
        "catspm" => "Special purpose category",
        "topshp" => "Topmark shape",
        "natsur" => "Nature of seabed",
        "natqua" => "Quality of seabed",
        "quasou" => "Quality of sounding",
        "souacc" => "Sounding accuracy (m)",
        "tecsou" => "Sounding technique",
        "expsou" => "Exposition of sounding",
        "verdat" => "Vertical datum",
        "veracc" => "Vertical accuracy (m)",
        "catmfa" => "Farm type",
        "catlmk" => "Landmark category",
        "functn" => "Function",
        "conrad" => "Radar conspicuous",
        "convis" => "Visually conspicuous",
        "natcon" => "Construction",
        "condtn" => "Condition",
        "marsys" => "Buoyage system",
        "picrep" => "Picture",
        "catcbl" => "Cable category",
        "catpip" => "Pipeline category",
        "catach" => "Anchorage category",
        "cattrk" => "Track category",
        "catnav" => "Navigation line category",
        "trafic" => "Traffic flow",
        "drval1" => "Depth from (m)",
        "drval2" => "Depth to (m)",
        "catfog" => "Fog signal",
        "catpil" => "Pilot boarding",
        "comctn" => "Communication",
        "catsit" => "Signal station category",
        "catrtb" => "Racon category",
        "radwal" => "Radar wavelength",
        "catsea" => "Sea area category",
        "catrgn" => "Region category",
        "catwed" => "Weed category",
        "catofp" => "Platform category",
        "catmor" => "Mooring category",
        "catple" => "Pile category",
        "catcto" => "Caution category",
        _ => return a.to_ascii_uppercase(),
    };
    s.to_string()
}

fn enum_word(attr: &str, code: u32) -> Option<&'static str> {
    Some(match (attr, code) {
        ("catslc", c) => match c {
            1 => "breakwater",
            2 => "groyne",
            3 => "mole",
            4 => "pier (jetty)",
            5 => "promenade pier",
            6 => "wharf (quay)",
            7 => "training wall",
            8 => "rip rap",
            9 => "revetment",
            10 => "sea wall",
            11 => "landing steps",
            12 => "ramp",
            13 => "slipway",
            14 => "fender",
            15 => "solid face wharf",
            16 => "open face wharf",
            17 => "log ramp",
            _ => return None,
        },
        ("colour", c) => match c {
            1 => "white",
            2 => "black",
            3 => "red",
            4 => "green",
            5 => "blue",
            6 => "yellow",
            7 => "grey",
            8 => "brown",
            9 => "amber",
            10 => "violet",
            11 => "orange",
            12 => "magenta",
            13 => "pink",
            _ => return None,
        },
        ("colpat", c) => match c {
            1 => "horizontal bands",
            2 => "vertical stripes",
            3 => "diagonal stripes",
            4 => "squared",
            5 => "stripes",
            6 => "border stripe",
            _ => return None,
        },
        ("litchr", c) => match c {
            1 => "fixed",
            2 => "flashing",
            3 => "long-flashing",
            4 => "quick-flashing",
            5 => "very quick-flashing",
            6 => "ultra quick-flashing",
            7 => "isophased",
            8 => "occulting",
            9 => "interrupted quick-flashing",
            10 => "interrupted very quick-flashing",
            11 => "interrupted ultra quick-flashing",
            12 => "morse",
            13 => "fixed and flashing",
            14 => "flashing and long-flashing",
            15 => "occulting and flashing",
            16 => "fixed and long-flashing",
            17 => "alternating occulting",
            18 => "alternating long-flashing",
            19 => "alternating flashing",
            20 => "alternating group",
            25 => "quick and long-flashing",
            26 => "very quick and long-flashing",
            27 => "ultra quick and long-flashing",
            28 => "alternating",
            29 => "fixed and alternating flashing",
            _ => return None,
        },
        ("catlit", c) => match c {
            1 => "directional",
            4 => "leading",
            5 => "aero",
            6 => "air obstruction",
            7 => "fog detector",
            8 => "flood",
            9 => "strip",
            10 => "subsidiary",
            11 => "spotlight",
            12 => "front",
            13 => "rear",
            14 => "lower",
            15 => "upper",
            16 => "moiré",
            17 => "emergency",
            18 => "bearing",
            19 => "horizontally disposed",
            20 => "vertically disposed",
            _ => return None,
        },
        ("exclit", c) => match c {
            1 => "24 hours",
            2 => "by day",
            3 => "in fog",
            4 => "at night",
            _ => return None,
        },
        ("litvis", c) => match c {
            1 => "high intensity",
            2 => "low intensity",
            3 => "faint",
            4 => "intensified",
            5 => "unintensified",
            6 => "deliberately restricted",
            7 => "obscured",
            8 => "partially obscured",
            _ => return None,
        },
        ("boyshp", c) => match c {
            1 => "conical",
            2 => "can",
            3 => "spherical",
            4 => "pillar",
            5 => "spar",
            6 => "barrel",
            7 => "super-buoy",
            8 => "ice buoy",
            _ => return None,
        },
        ("bcnshp", c) => match c {
            1 => "stake or pole",
            2 => "withy",
            3 => "beacon tower",
            4 => "lattice beacon",
            5 => "pile beacon",
            6 => "cairn",
            7 => "buoyant beacon",
            _ => return None,
        },
        ("catlam", c) => match c {
            1 => "port",
            2 => "starboard",
            3 => "preferred channel to starboard",
            4 => "preferred channel to port",
            _ => return None,
        },
        ("catcam", c) => match c {
            1 => "north",
            2 => "east",
            3 => "south",
            4 => "west",
            _ => return None,
        },
        ("watlev", c) => match c {
            1 => "partly submerged at high water",
            2 => "always dry",
            3 => "always under water",
            4 => "covers and uncovers",
            5 => "awash",
            6 => "subject to flooding",
            7 => "floating",
            _ => return None,
        },
        ("catwrk", c) => match c {
            1 => "non-dangerous",
            2 => "dangerous",
            3 => "distributed remains",
            4 => "mast showing",
            5 => "hull showing",
            _ => return None,
        },
        ("catobs", c) => match c {
            1 => "snag or stump",
            2 => "wellhead",
            3 => "diffuser",
            4 => "crib",
            5 => "fish haven",
            6 => "foul area",
            7 => "foul ground",
            8 => "ice boom",
            9 => "ground tackle",
            10 => "boom",
            _ => return None,
        },
        ("catrea", c) => match c {
            1 => "offshore safety zone",
            4 => "nature reserve",
            5 => "bird sanctuary",
            6 => "game reserve",
            7 => "seal sanctuary",
            8 => "degaussing range",
            9 => "military area",
            10 => "historic wreck area",
            12 => "navigational aid safety zone",
            14 => "minefield",
            18 => "swimming area",
            19 => "waiting area",
            20 => "research area",
            21 => "dredging area",
            22 => "fish sanctuary",
            23 => "ecological reserve",
            24 => "no wake area",
            25 => "swinging area",
            26 => "water skiing area",
            27 => "environmentally sensitive area",
            28 => "particularly sensitive sea area",
            _ => return None,
        },
        ("restrn", c) => match c {
            1 => "anchoring prohibited",
            2 => "anchoring restricted",
            3 => "fishing prohibited",
            4 => "fishing restricted",
            5 => "trawling prohibited",
            6 => "trawling restricted",
            7 => "entry prohibited",
            8 => "entry restricted",
            9 => "dredging prohibited",
            10 => "dredging restricted",
            11 => "diving prohibited",
            12 => "diving restricted",
            13 => "no wake",
            14 => "area to be avoided",
            15 => "construction prohibited",
            16 => "discharging prohibited",
            17 => "discharging restricted",
            20 => "drilling prohibited",
            21 => "drilling restricted",
            22 => "removal of historical artefacts prohibited",
            24 => "dragging prohibited",
            25 => "stopping prohibited",
            26 => "landing prohibited",
            27 => "speed restricted",
            _ => return None,
        },
        ("status", c) => match c {
            1 => "permanent",
            2 => "occasional",
            3 => "recommended",
            4 => "not in use",
            5 => "periodic or intermittent",
            6 => "reserved",
            7 => "temporary",
            8 => "private",
            9 => "mandatory",
            11 => "extinguished",
            12 => "illuminated",
            13 => "historic",
            14 => "public",
            15 => "synchronised",
            16 => "watched",
            17 => "unwatched",
            18 => "existence doubtful",
            _ => return None,
        },
        ("topshp", c) => match c {
            1 => "cone, point up",
            2 => "cone, point down",
            3 => "sphere",
            4 => "two spheres",
            5 => "cylinder (can)",
            6 => "board",
            7 => "x-shape",
            8 => "upright cross",
            9 => "cube, point up",
            10 => "two cones, point to point",
            11 => "two cones, base to base",
            12 => "rhombus",
            13 => "two cones, points up",
            14 => "two cones, points down",
            17 => "flag",
            19 => "square",
            24 => "triangle, point up",
            25 => "triangle, point down",
            26 => "circle",
            _ => return None,
        },
        ("natsur", c) => match c {
            1 => "mud",
            2 => "clay",
            3 => "silt",
            4 => "sand",
            5 => "stone",
            6 => "gravel",
            7 => "pebbles",
            8 => "cobbles",
            9 => "rock",
            11 => "lava",
            14 => "coral",
            17 => "shells",
            18 => "boulder",
            _ => return None,
        },
        ("quasou", c) => match c {
            1 => "depth known",
            2 => "depth unknown",
            3 => "doubtful sounding",
            4 => "unreliable sounding",
            5 => "no bottom found",
            6 => "least depth known",
            7 => "least depth unknown, safe clearance at value shown",
            8 => "value reported (not surveyed)",
            9 => "value reported (not confirmed)",
            10 => "maintained depth",
            11 => "not regularly maintained",
            _ => return None,
        },
        ("expsou", c) => match c {
            1 => "within the range of the surrounding depths",
            2 => "shoaler than the surrounding depths",
            3 => "deeper than the surrounding depths",
            _ => return None,
        },
        ("catmfa", c) => match c {
            1 => "crustaceans",
            2 => "oysters or mussels",
            3 => "fish",
            4 => "seaweed",
            5 => "pearl culture",
            _ => return None,
        },
        ("functn", c) => match c {
            2 => "harbour master's office",
            3 => "custom office",
            4 => "health office",
            5 => "hospital",
            6 => "post office",
            7 => "hotel",
            8 => "railway station",
            9 => "police station",
            10 => "water-police station",
            11 => "pilot office",
            12 => "pilot lookout",
            13 => "bank office",
            14 => "headquarters for district control",
            15 => "transit shed / warehouse",
            16 => "factory",
            17 => "power station",
            18 => "administrative",
            19 => "educational facility",
            20 => "church",
            21 => "chapel",
            22 => "temple",
            23 => "pagoda",
            24 => "shinto shrine",
            25 => "buddhist temple",
            26 => "mosque",
            27 => "marabout",
            28 => "lookout",
            29 => "communication",
            30 => "television",
            31 => "radio",
            32 => "radar",
            33 => "light support",
            34 => "microwave",
            35 => "cooling",
            36 => "observation",
            37 => "time ball",
            38 => "clock",
            39 => "control",
            40 => "airship mooring",
            41 => "stadium",
            42 => "bus station",
            _ => return None,
        },
        ("catlmk", c) => match c {
            1 => "cairn",
            2 => "cemetery",
            3 => "chimney",
            4 => "dish aerial",
            5 => "flagstaff",
            6 => "flare stack",
            7 => "mast",
            8 => "windsock",
            9 => "monument",
            10 => "column",
            11 => "memorial plaque",
            12 => "obelisk",
            13 => "statue",
            14 => "cross",
            15 => "dome",
            16 => "radar scanner",
            17 => "tower",
            18 => "windmill",
            19 => "wind turbine",
            20 => "spire or minaret",
            21 => "boulder",
            _ => return None,
        },
        ("conrad", c) | ("convis", c) => match c {
            1 => "conspicuous",
            2 => "not conspicuous",
            3 => "conspicuous (radar reflector)",
            _ => return None,
        },
        ("natcon", c) => match c {
            1 => "masonry",
            2 => "concreted",
            3 => "loose boulders",
            4 => "hard surfaced",
            5 => "unsurfaced",
            6 => "wooden",
            7 => "metal",
            8 => "GRP",
            9 => "painted",
            _ => return None,
        },
        ("marsys", c) => match c {
            1 => "IALA A",
            2 => "IALA B",
            9 => "none",
            10 => "other",
            _ => return None,
        },
        ("catcbl", c) => match c {
            1 => "power line",
            3 => "transmission line",
            4 => "telephone",
            5 => "telegraph",
            6 => "mooring cable or chain",
            _ => return None,
        },
        ("condtn", c) => match c {
            1 => "under construction",
            2 => "ruined",
            3 => "under reclamation",
            4 => "wingless",
            5 => "planned construction",
            _ => return None,
        },
        _ => return None,
    })
}

/// A value in words: list codes decoded where known (`3,1` → "red, white").
pub fn describe(attr: &str, value: &str) -> String {
    let v = value.trim();
    if matches!(attr, "sordat" | "datsta" | "datend" | "persta" | "perend") && v.len() == 8 && v.bytes().all(|b| b.is_ascii_digit()) {
        return format!("{}-{}-{}", &v[..4], &v[4..6], &v[6..]);
    }
    let codes: Vec<&str> = v.split(',').map(str::trim).collect();
    if codes.iter().all(|c| c.parse::<u32>().is_ok()) {
        let words: Vec<String> = codes
            .iter()
            .map(|c| {
                let n: u32 = c.parse().unwrap();
                enum_word(attr, n).map(str::to_string).unwrap_or_else(|| c.to_string())
            })
            .collect();
        if words.iter().zip(&codes).any(|(w, c)| w != c) {
            return words.join(", ");
        }
    }
    if let Ok(x) = v.parse::<f64>() {
        if x.fract() == 0.0 && x.abs() < 1e12 {
            return format!("{}", x as i64);
        }
        return trim_num_n(x);
    }
    v.to_string()
}

fn trim_num_n(x: f64) -> String {
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_objects_get_the_question_mark() {
        let v = vec![None; TILE_COLS.len()];
        assert_eq!(icon(UNKNOWN, &Attrs(&v)).as_deref(), Some("uk"));
        assert_eq!(class_for("other_object_polygon_hydro_enc_b4").unwrap().code, UNKNOWN);
        assert_eq!(class_for("traffic_separation_zone_polygon_hydro_enc_b3").unwrap().code, "TSEZNE");
        assert_eq!(enc_s57::gpkg::table_for("UNSARE", 3, 4).as_deref(), Some("unsurveyed_area_polygon_hydro_enc_b4"));
        assert_eq!(enc_s57::gpkg::table_for("SNDWAV", 3, 4).as_deref(), Some("other_object_polygon_hydro_enc_b4"));
        assert_eq!(enc_s57::gpkg::table_for("BUISGL", 1, 4), None);
        assert_eq!(enc_s57::gpkg::table_for("M_QUAL", 3, 4), None);
    }

    #[test]
    fn finds_classes() {
        assert_eq!(class_for("light_points_hydro_190k_1350k").unwrap().code, "LIGHTS");
        assert_eq!(class_for("obstruction_polygon_hydro_14k_122k").unwrap().geo, Geo::Area);
        assert_eq!(class_for("obstruction_points_hydro_14k_122k").unwrap().geo, Geo::Point);
        assert!(class_for("depth_area_polygon_hydro_190k_1350k").is_none());
        assert!(class_for("light_points_topo").is_none());
    }

    #[test]
    fn light_labels() {
        let l = |c: u32, g: &str, p: f64, col: &[u32], h: f64, r: f64| Light {
            litchr: Some(c),
            siggrp: Some(g.into()),
            sigper: Some(p),
            colours: col.to_vec(),
            height: Some(h),
            range: Some(r),
            ..Default::default()
        };
        assert_eq!(light_label(&[l(2, "(1)", 6.0, &[4], 4.0, 3.0)]), "Fl G 6s 4m 3M");
        // a sector light: three records, one label
        let s = [l(2, "(2)", 10.0, &[3], 15.0, 9.0), l(2, "(2)", 10.0, &[1], 15.0, 12.0), l(2, "(2)", 10.0, &[4], 15.0, 9.0)];
        assert_eq!(light_label(&s), "Fl(2) WRG 10s 15m 12-9M");
        assert_eq!(light_label(&[l(4, "()", 0.0, &[1], 8.0, 5.0)]).split(' ').next(), Some("Q"));
    }

    #[test]
    fn describes_codes() {
        assert_eq!(describe("colour", "3,1,3"), "red, white, red");
        assert_eq!(describe("watlev", "4"), "covers and uncovers");
        assert_eq!(describe("valsou", "2.50"), "2.5");
        assert_eq!(describe("objnam", "Fishermans Rock"), "Fishermans Rock");
        assert_eq!(describe("sordat", "20150701"), "2015-07-01");
    }

    #[test]
    fn scamin_zooms() {
        let z = scamin_zoom(259_999.0, -41.0);
        assert!(z > 9.0 && z < 10.5, "{z}");
        assert!(scamin_zoom(22_000.0, -41.0) > 13.0);
    }
}
