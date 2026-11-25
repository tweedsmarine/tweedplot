//! DSC calls a VHF radio passes on (IEC 61162-1 DSC and DSE): distress alerts, distress relays and
//! acknowledgements, and urgency and safety calls, with the caller's position when it is given.
//!
//! DSC fields: 1 format specifier (12 distress, 16 all ships, 20 individual, 02 area, 14 group),
//! 2 address (the MMSI, ten digits with a trailing 0), 3 category (blank for distress; 12 distress,
//! 10 urgency, 08 safety, 00 routine), 4 nature of distress (distress) or first telecommand,
//! 5 type of communication or second telecommand, 6 position (quadrant digit, then latitude ddmm,
//! longitude dddmm; nines when unknown), 7 time UTC hhmm (8888 when unknown), 8 MMSI of the ship in
//! distress (relays and acknowledgements), 9 nature of distress (relays), 10 acknowledgement
//! (R request, B acknowledgement, S neither), 11 expansion (E: a DSE follows).
//! DSE (expansion): total, number, flag, MMSI, then code and data pairs; code 00 is the enhanced
//! position: four more digits of the latitude's minutes, then four of the longitude's.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DscCall {
    /// distress, urgency, safety or routine
    pub priority: &'static str,
    /// alert (the ship in distress calling), relay (another station passing it on),
    /// acknowledgement, or call (urgency, safety, routine)
    pub kind: &'static str,
    /// The ship in distress, or the caller (nine digits).
    pub mmsi: String,
    /// The station that sent it, when it isn't the ship in distress (a relay or acknowledgement).
    pub from_mmsi: Option<String>,
    /// Nature of distress in words (Fire, Flooding, …, Man overboard, EPIRB emission).
    pub nature: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// "hh:mm" UTC as sent.
    pub time_utc: Option<String>,
    /// A DSE with the enhanced position follows.
    pub expansion: bool,
}

/// The nine-digit MMSI from a DSC address field (ten digits, the last a 0).
fn mmsi(field: &str) -> Option<String> {
    let d: String = field.chars().filter(char::is_ascii_digit).collect();
    match d.len() {
        10 => Some(d[..9].to_string()),
        9 => Some(d),
        _ => None,
    }
}

fn nature(code: &str) -> Option<String> {
    Some(
        match code {
            "00" => "Fire, explosion",
            "01" => "Flooding",
            "02" => "Collision",
            "03" => "Grounding",
            "04" => "Listing, in danger of capsizing",
            "05" => "Sinking",
            "06" => "Disabled and adrift",
            "07" => "Undesignated distress",
            "08" => "Abandoning ship",
            "09" => "Piracy or armed robbery",
            "10" => "Man overboard",
            "12" => "EPIRB emission",
            _ => return None,
        }
        .to_string(),
    )
}

/// Quadrant, latitude ddmm, longitude dddmm (10 digits); None when unknown (nines) or malformed.
pub fn position(field: &str) -> Option<(f64, f64)> {
    let d = field.trim();
    if d.len() != 10 || !d.bytes().all(|c| c.is_ascii_digit()) || d.starts_with('9') {
        return None;
    }
    let q = &d[..1];
    let lat = d[1..3].parse::<f64>().ok()? + d[3..5].parse::<f64>().ok()? / 60.0;
    let lon = d[5..8].parse::<f64>().ok()? + d[8..10].parse::<f64>().ok()? / 60.0;
    if lat > 90.0 || lon > 180.0 {
        return None;
    }
    // 0 NE, 1 NW, 2 SE, 3 SW
    let (ns, ew) = match q {
        "0" => (1.0, 1.0),
        "1" => (1.0, -1.0),
        "2" => (-1.0, 1.0),
        "3" => (-1.0, -1.0),
        _ => return None,
    };
    Some((lat * ns, lon * ew))
}

/// A DSC sentence's fields (after the talker and DSC), when it is worth telling the mariner about:
/// distress (alerts, relays, acknowledgements) and urgency and safety calls.
pub fn parse_dsc(f: &[String]) -> Option<DscCall> {
    let field = |i: usize| f.get(i).map(|s| s.trim()).unwrap_or("");
    let format = field(0);
    let category = field(2);
    let priority = if format == "12" || category == "12" {
        "distress"
    } else if category == "10" {
        "urgency"
    } else if category == "08" {
        "safety"
    } else {
        return None;
    };
    let caller = mmsi(field(1))?;
    let distressed = mmsi(field(7));
    let ack = field(9);
    let (kind, ship, from, nat) = if format == "12" {
        ("alert", caller, None, nature(field(3)))
    } else if priority == "distress" {
        // a relay or an acknowledgement: the ship in distress is in field 8 when given
        let kind = if ack == "B" || field(3) == "20" { "acknowledgement" } else { "relay" };
        match distressed {
            Some(d) if d != caller => (kind, d, Some(caller), nature(field(8))),
            _ => (kind, caller, None, nature(field(8))),
        }
    } else {
        ("call", caller, None, None)
    };
    let pos = position(field(5));
    let t = field(6);
    let time_utc = (t.len() == 4 && t.bytes().all(|c| c.is_ascii_digit()) && t != "8888").then(|| format!("{}:{}", &t[..2], &t[2..]));
    Some(DscCall {
        priority,
        kind,
        mmsi: ship,
        from_mmsi: from,
        nature: nat,
        lat: pos.map(|p| p.0),
        lon: pos.map(|p| p.1),
        time_utc,
        expansion: field(10) == "E",
    })
}

/// The enhanced position in a DSE (code 00): the extra minutes digits to add to the DSC position,
/// as (MMSI, extra latitude minutes, extra longitude minutes), each 0–0.9999.
pub fn parse_dse(f: &[String]) -> Option<(String, f64, f64)> {
    let ship = mmsi(f.get(3)?)?;
    let mut i = 4;
    while i + 1 < f.len() {
        if f[i] == "00" {
            let d = f[i + 1].trim();
            if d.len() == 8 && d.bytes().all(|c| c.is_ascii_digit()) {
                return Some((ship, d[..4].parse::<f64>().ok()? / 10_000.0, d[4..].parse::<f64>().ok()? / 10_000.0));
            }
        }
        i += 2;
    }
    None
}

/// The DSC position refined with a DSE's extra minutes (moving away from the equator and the
/// meridian, as the minutes do).
pub fn refine(lat: f64, lon: f64, dlat_min: f64, dlon_min: f64) -> (f64, f64) {
    (lat + lat.signum() * dlat_min / 60.0, lon + lon.signum() * dlon_min / 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Vec<String> {
        s.split(',').map(str::to_string).collect()
    }

    #[test]
    fn reads_a_distress_alert() {
        // flooding, 41°10'S 173°15'E (quadrant 2, south-east), 03:12 UTC, a DSE follows
        let c = parse_dsc(&f("12,5120012340,,01,09,2411017315,0312,,,S,E")).unwrap();
        assert_eq!((c.priority, c.kind, c.mmsi.as_str()), ("distress", "alert", "512001234"));
        assert_eq!(c.nature.as_deref(), Some("Flooding"));
        assert!((c.lat.unwrap() + 41.1667).abs() < 1e-3 && (c.lon.unwrap() - 173.25).abs() < 1e-3);
        assert_eq!(c.time_utc.as_deref(), Some("03:12"));
        assert!(c.expansion);
    }

    #[test]
    fn reads_a_relay_and_unknown_position() {
        // all ships, distress category, relay by 503000010 of 512009990 in distress, sinking
        let c = parse_dsc(&f("16,5030000100,12,12,00,9999999999,8888,5120099900,05,S,")).unwrap();
        assert_eq!((c.kind, c.mmsi.as_str(), c.from_mmsi.as_deref()), ("relay", "512009990", Some("503000010")));
        assert_eq!(c.nature.as_deref(), Some("Sinking"));
        assert_eq!((c.lat, c.time_utc.clone()), (None, None));
    }

    #[test]
    fn urgency_and_safety_but_not_routine() {
        assert_eq!(parse_dsc(&f("16,5120012340,10,00,09,,,,,S,")).unwrap().priority, "urgency");
        assert_eq!(parse_dsc(&f("16,5120012340,08,00,09,,,,,S,")).unwrap().priority, "safety");
        assert!(parse_dsc(&f("20,5120012340,00,00,09,,,,,S,")).is_none());
    }

    #[test]
    fn enhanced_position() {
        let (m, dla, dlo) = parse_dse(&f("1,1,A,5120012340,00,45673421")).unwrap();
        assert_eq!(m, "512001234");
        let (lat, lon) = refine(-41.1667, 173.25, dla, dlo);
        assert!((lat - (-41.1667 - 0.4567 / 60.0)).abs() < 1e-9 && (lon - (173.25 + 0.3421 / 60.0)).abs() < 1e-9);
    }
}
