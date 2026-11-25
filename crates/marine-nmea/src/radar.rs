//! Radar (ARPA) tracked targets: TTM (range and bearing from own ship, the target's course and
//! speed, and the radar's own CPA and TCPA) and TLL (the target's latitude and longitude).
//!
//! TTM: target number, distance, bearing, T/R (true or relative), speed, course, T/R, CPA, TCPA
//! (minutes, - when past), units (K km and km/h, N NM and knots, S statute), name, status
//! (L lost, Q being acquired, T tracking), reference target (R), time, acquisition (A auto,
//! M manual, R reported).
//! TLL: target number, latitude, N/S, longitude, E/W, name, time, status, reference.

use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarTarget {
    pub number: u32,
    /// From own ship: range (NM) and bearing (degrees), true or relative to the heading.
    pub range_nm: Option<f64>,
    pub bearing: Option<f64>,
    pub bearing_true: bool,
    /// The target's speed (kn) and course (degrees), true or relative.
    pub speed_kn: Option<f64>,
    pub course: Option<f64>,
    pub course_true: bool,
    /// The radar's closest point of approach (NM) and time to it (minutes).
    pub cpa_nm: Option<f64>,
    pub tcpa_min: Option<f64>,
    pub name: Option<String>,
    /// lost, acquiring or tracking
    pub status: &'static str,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

fn num(s: Option<&String>) -> Option<f64> {
    s.and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
}

fn status(s: Option<&String>) -> &'static str {
    match s.map(|v| v.trim()) {
        Some("L") => "lost",
        Some("Q") => "acquiring",
        _ => "tracking",
    }
}

fn name(s: Option<&String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// A TTM's fields.
pub fn parse_ttm(f: &[String]) -> Option<RadarTarget> {
    let number = f.first()?.trim().parse::<u32>().ok()?;
    // distances in NM and speeds in knots, whatever the units
    let (dk, sk) = match f.get(9).map(|u| u.trim()) {
        Some("K") => (1.0 / 1.852, 1.0 / 1.852),
        Some("S") => (1.0 / 1.150_779, 1.0 / 1.150_779),
        _ => (1.0, 1.0),
    };
    Some(RadarTarget {
        number,
        range_nm: num(f.get(1)).map(|d| d * dk),
        bearing: num(f.get(2)).map(|b| b.rem_euclid(360.0)),
        bearing_true: f.get(3).is_some_and(|t| t == "T"),
        speed_kn: num(f.get(4)).map(|v| v * sk),
        course: num(f.get(5)).map(|c| c.rem_euclid(360.0)),
        course_true: f.get(6).is_some_and(|t| t == "T"),
        cpa_nm: num(f.get(7)).map(|d| d * dk),
        tcpa_min: num(f.get(8)),
        name: name(f.get(10)),
        status: status(f.get(11)),
        lat: None,
        lon: None,
    })
}

/// A TLL's fields: the number, position, name and status.
pub fn parse_tll(f: &[String]) -> Option<RadarTarget> {
    let number = f.first()?.trim().parse::<u32>().ok()?;
    let coord = |v: Option<&String>, hemi: Option<&String>, deg_digits: usize| -> Option<f64> {
        let v = v?.trim();
        if v.len() < deg_digits + 1 {
            return None;
        }
        let d = v[..deg_digits].parse::<f64>().ok()?;
        let m = v[deg_digits..].parse::<f64>().ok()?;
        let x = d + m / 60.0;
        Some(match hemi.map(|h| h.trim()) {
            Some("S") | Some("W") => -x,
            _ => x,
        })
    };
    Some(RadarTarget {
        number,
        lat: coord(f.get(1), f.get(2), 2),
        lon: coord(f.get(3), f.get(4), 3),
        name: name(f.get(5)),
        status: status(f.get(7)),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Vec<String> {
        s.split(',').map(str::to_string).collect()
    }

    #[test]
    fn reads_ttm() {
        let t = parse_ttm(&f("05,2.50,045.0,T,12.0,270.0,T,0.30,8.5,N,TUG,T,,123456.00,A")).unwrap();
        assert_eq!((t.number, t.range_nm, t.bearing, t.bearing_true), (5, Some(2.5), Some(45.0), true));
        assert_eq!((t.speed_kn, t.course, t.cpa_nm, t.tcpa_min), (Some(12.0), Some(270.0), Some(0.3), Some(8.5)));
        assert_eq!((t.name.as_deref(), t.status), (Some("TUG"), "tracking"));
        // kilometres, relative bearing, lost
        let k = parse_ttm(&f("7,3.704,010.0,R,18.52,090.0,R,,,K,,L,,,M")).unwrap();
        assert!((k.range_nm.unwrap() - 2.0).abs() < 1e-9 && (k.speed_kn.unwrap() - 10.0).abs() < 1e-9);
        assert!(!k.bearing_true && k.status == "lost" && k.name.is_none());
    }

    #[test]
    fn reads_tll() {
        let t = parse_tll(&f("05,4110.500,S,17315.250,E,TUG,123456.00,T,")).unwrap();
        assert!((t.lat.unwrap() + 41.175).abs() < 1e-9 && (t.lon.unwrap() - 173.254_166_67).abs() < 1e-6);
        assert_eq!(t.status, "tracking");
    }
}
