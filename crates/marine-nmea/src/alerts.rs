//! Alarms from other equipment on the NMEA network: ALR (the older alarm sentence, IEC 61162-1)
//! and ALF (bridge alert management, IEC 62923), so a sounder's shallow alarm or an AIS's own
//! alarm shows in TweedPlot's alert list beside its own.
//!
//! ALR: time, alarm number, condition (A threshold exceeded, V not), acknowledged (A, V), text.
//! ALF: total sentences, sentence number, sequential id, time, category (A, B, C), priority
//! (E emergency alarm, A alarm, W warning, C caution), state (V active unacknowledged, S active
//! silenced, A active acknowledged, O active responsibility transferred, U rectified
//! unacknowledged, N normal), manufacturer mnemonic, alert identifier, instance, revision,
//! escalation, text. A second ALF sentence (number 2) carries more text only.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentAlert {
    /// Unique for the alert on its talker: "ALR 012" or "ALF SDR 3015.1".
    pub id: String,
    /// alarm, warning or caution
    pub priority: &'static str,
    /// The condition still holds (ALR A; ALF V, S, A, O).
    pub active: bool,
    /// Acknowledged at the equipment.
    pub acked: bool,
    pub text: String,
}

/// An ALR's fields, when it is an alarm (number and text).
pub fn parse_alr(f: &[String]) -> Option<EquipmentAlert> {
    let num = f.get(1)?.trim();
    if num.is_empty() {
        return None;
    }
    Some(EquipmentAlert {
        id: format!("ALR {num}"),
        // ALR has no priority: an alarm, as the equipment calls it one
        priority: "alarm",
        active: f.get(2).is_some_and(|c| c == "A"),
        acked: f.get(3).is_some_and(|a| a == "A"),
        text: f.get(4).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("Alarm {num}")),
    })
}

/// The first sentence of an ALF (the one with the state and priority); later ones only add text.
pub fn parse_alf(f: &[String]) -> Option<EquipmentAlert> {
    if f.get(1).map(String::as_str) != Some("1") {
        return None;
    }
    let field = |i: usize| f.get(i).map(|s| s.trim()).unwrap_or("");
    let priority = match field(5) {
        "E" | "A" => "alarm",
        "W" => "warning",
        "C" => "caution",
        _ => return None,
    };
    let state = field(6);
    let (active, acked) = match state {
        "V" => (true, false),
        "S" | "A" | "O" => (true, true),
        "U" => (false, false),
        "N" => (false, true),
        _ => return None,
    };
    let id = format!("ALF {} {}.{}", field(7), field(8), if field(9).is_empty() { "1" } else { field(9) });
    let text = field(12);
    Some(EquipmentAlert {
        id,
        priority,
        active,
        acked,
        text: if text.is_empty() { format!("Alert {} from {}", field(8), field(7)) } else { text.to_string() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Vec<String> {
        s.split(',').map(str::to_string).collect()
    }

    #[test]
    fn reads_alr() {
        let a = parse_alr(&f("220516.00,012,A,V,SHALLOW WATER")).unwrap();
        assert_eq!((a.id.as_str(), a.active, a.acked, a.text.as_str()), ("ALR 012", true, false, "SHALLOW WATER"));
        let b = parse_alr(&f("220530.00,012,V,A,SHALLOW WATER")).unwrap();
        assert!(!b.active && b.acked);
    }

    #[test]
    fn reads_alf() {
        // a warning, active and unacknowledged: an echo sounder's (SDR) alert 3015, instance 1
        let a = parse_alf(&f("1,1,0,124304.50,A,W,V,SDR,3015,1,0,0,Depth below shallow limit")).unwrap();
        assert_eq!((a.id.as_str(), a.priority, a.active, a.acked), ("ALF SDR 3015.1", "warning", true, false));
        assert_eq!(a.text, "Depth below shallow limit");
        // rectified: no longer active
        assert!(!parse_alf(&f("1,1,1,124310.00,A,W,N,SDR,3015,1,1,0,Depth below shallow limit")).unwrap().active);
        // the second sentence of a pair only adds text
        assert!(parse_alf(&f("2,2,0,,,,,,,,,,more text")).is_none());
    }
}
