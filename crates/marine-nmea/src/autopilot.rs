//! NMEA 0183 sentences that steer an autopilot along the leg being navigated: RMB, APB, XTE,
//! BOD, BWC and AAM (IEC 61162-1), the set chart plotters send. An autopilot in Track or Nav mode
//! follows the cross-track error and the bearing to the next mark; most read RMB or APB, some
//! XTE, and BOD/BWC give the waypoint names and bearings for their display.

/// The leg being navigated, as the app works it out once a second.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotNav {
    /// Cross-track error, NM: + = the boat is to the right of the track (steer left).
    pub xte_nm: f64,
    /// The mark the leg starts from and the mark steered for (names; shortened and cleaned here).
    pub origin_id: String,
    pub dest_id: String,
    pub dest_lat: f64,
    pub dest_lon: f64,
    /// From the boat to the mark: distance (NM) and bearing (degrees true).
    pub range_nm: f64,
    pub bearing_true: f64,
    /// Bearing of the leg, origin to mark, degrees true.
    pub bearing_leg_true: f64,
    /// Speed made good towards the mark, knots (velocity made good on course).
    pub vmc_kn: Option<f64>,
    /// Heading to steer to get back on track, degrees true.
    pub steer_true: Option<f64>,
    /// Inside the arrival circle; past the line square to the leg through the mark.
    pub arrived: bool,
    pub passed: bool,
    /// The arrival circle's radius, NM (for AAM; 0.1 when not given).
    #[serde(default)]
    pub arrival_radius_nm: Option<f64>,
    /// Magnetic variation, degrees, east +: the magnetic bearings are given when it's known.
    pub variation: Option<f64>,
}

/// `$BODY*CS\r\n`.
pub fn frame(body: &str) -> String {
    let cs = body.bytes().fold(0u8, |a, b| a ^ b);
    format!("${body}*{cs:02X}\r\n")
}

/// A waypoint name an autopilot can show: letters, digits, spaces, dot and dash only (commas and
/// asterisks would break the sentence), at most 8 characters (two of them and a position must fit
/// RMB's 82); "WPT" when nothing is left.
pub fn waypoint_id(name: &str) -> String {
    waypoint_id_max(name, 8)
}

fn waypoint_id_max(name: &str, max: usize) -> String {
    let s: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '.' | '_'))
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim()
        .to_string();
    if s.is_empty() {
        "WPT".into()
    } else {
        s
    }
}

fn norm(d: f64) -> f64 {
    d.rem_euclid(360.0)
}

/// ddmm.mmm,N and dddmm.mmm,E.
fn lat_lon(lat: f64, lon: f64) -> (String, String) {
    let part = |v: f64, deg_digits: usize| {
        let a = v.abs();
        let mut d = a.trunc();
        let mut m = ((a - d) * 60.0 * 1000.0).round() / 1000.0;
        if m >= 60.0 {
            d += 1.0;
            m -= 60.0;
        }
        format!("{:0w$}{:06.3}", d as u32, m, w = deg_digits)
    };
    (format!("{},{}", part(lat, 2), if lat < 0.0 { "S" } else { "N" }), format!("{},{}", part(lon, 3), if lon < 0.0 { "W" } else { "E" }))
}

/// The six sentences for one moment (`talker`: "GP" is what most autopilots expect; `utc_hms`
/// "hhmmss" for BWC). Each framed with its checksum and CR LF, none over 82 characters.
pub fn sentences(nav: &AutopilotNav, talker: &str, utc_hms: &str) -> Vec<String> {
    // names shortened further if a sentence would still be longer than NMEA's 82 characters
    for max in [8, 6, 4] {
        let out = sentences_with(nav, talker, utc_hms, max);
        if out.iter().all(|s| s.len() <= 82) || max == 4 {
            return out;
        }
    }
    unreachable!()
}

fn sentences_with(nav: &AutopilotNav, talker: &str, utc_hms: &str, max: usize) -> Vec<String> {
    let xte = nav.xte_nm.abs().min(9.99);
    // the way to steer: left when the boat is right of the track
    let steer = if nav.xte_nm >= 0.0 { "L" } else { "R" };
    let orig = waypoint_id_max(&nav.origin_id, max);
    let dest = waypoint_id_max(&nav.dest_id, max);
    let (lat, lon) = lat_lon(nav.dest_lat, nav.dest_lon);
    let range = nav.range_nm.clamp(0.0, 999.99);
    let brg = norm(nav.bearing_true);
    let leg = norm(nav.bearing_leg_true);
    let mag = |t: f64| nav.variation.map(|v| format!("{:.1}", norm(t - v))).unwrap_or_default();
    let vmc = nav.vmc_kn.map(|v| format!("{:.1}", v.clamp(-99.9, 99.9))).unwrap_or_default();
    let steer_to = nav.steer_true.map(norm).unwrap_or(brg);
    let arrived = if nav.arrived { "A" } else { "V" };
    let passed = if nav.passed { "A" } else { "V" };
    vec![
        frame(&format!("{talker}RMB,A,{xte:.2},{steer},{orig},{dest},{lat},{lon},{range:.2},{brg:.1},{vmc},{arrived},A")),
        frame(&format!("{talker}APB,A,A,{xte:.2},{steer},N,{arrived},{passed},{leg:.1},T,{dest},{brg:.1},T,{steer_to:.1},T,A")),
        frame(&format!("{talker}XTE,A,A,{xte:.2},{steer},N,A")),
        frame(&format!("{talker}BOD,{leg:.1},T,{},M,{dest},{orig}", mag(leg))),
        frame(&format!("{talker}BWC,{utc_hms},{lat},{lon},{brg:.1},T,{},M,{range:.2},N,{dest},A", mag(brg))),
        // arrival alarm: circle entered, perpendicular passed, the circle's radius, the mark
        frame(&format!("{talker}AAM,{arrived},{passed},{:.2},N,{dest}", nav.arrival_radius_nm.unwrap_or(0.1).clamp(0.0, 9.99))),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_sentence, DEFAULT_MAX_FRAME_LEN};

    fn nav() -> AutopilotNav {
        AutopilotNav {
            xte_nm: 0.123,
            origin_id: "Current Basin".into(),
            dest_id: "French Ps, SW*".into(),
            dest_lat: -40.9333,
            dest_lon: 173.8260,
            range_nm: 3.31,
            bearing_true: 54.2,
            bearing_leg_true: 51.0,
            vmc_kn: Some(8.71),
            steer_true: Some(48.3),
            arrived: false,
            passed: false,
            arrival_radius_nm: Some(0.05),
            variation: Some(22.0),
        }
    }

    #[test]
    fn builds_valid_sentences() {
        let out = sentences(&nav(), "GP", "021500");
        assert_eq!(out.len(), 6);
        for s in &out {
            assert!(s.len() <= 82, "{s}");
            assert!(s.ends_with("\r\n"));
            // our own parser accepts each (checksum and framing right)
            let p = parse_sentence(s.trim_end().as_bytes(), DEFAULT_MAX_FRAME_LEN).expect(s);
            assert_eq!(p.talker, "GP");
        }
        assert_eq!(out[0].split('*').next().unwrap(), "$GPRMB,A,0.12,L,Current,French P,4055.998,S,17349.560,E,3.31,54.2,8.7,V,A");
        assert_eq!(out[1].split('*').next().unwrap(), "$GPAPB,A,A,0.12,L,N,V,V,51.0,T,French P,54.2,T,48.3,T,A");
        assert_eq!(out[2].split('*').next().unwrap(), "$GPXTE,A,A,0.12,L,N,A");
        assert_eq!(out[3].split('*').next().unwrap(), "$GPBOD,51.0,T,29.0,M,French P,Current");
        assert_eq!(out[4].split('*').next().unwrap(), "$GPBWC,021500,4055.998,S,17349.560,E,54.2,T,32.2,M,3.31,N,French P,A");
        assert_eq!(out[5].split('*').next().unwrap(), "$GPAAM,V,V,0.05,N,French P");
    }

    #[test]
    fn steers_right_when_left_of_track_and_marks_arrival() {
        let mut n = nav();
        n.xte_nm = -0.5;
        n.arrived = true;
        n.passed = true;
        n.variation = None;
        n.vmc_kn = None;
        let out = sentences(&n, "EC", "000000");
        assert!(out[0].starts_with("$ECRMB,A,0.50,R,"));
        assert!(out[0].contains(",54.2,,A,A*"));
        assert!(out[1].starts_with("$ECAPB,A,A,0.50,R,N,A,A,"));
        assert!(out[3].starts_with("$ECBOD,51.0,T,,M,"));
        assert!(out[5].starts_with("$ECAAM,A,A,0.05,N,"));
    }

    #[test]
    fn cleans_waypoint_names() {
        assert_eq!(waypoint_id("A,B*C"), "ABC");
        assert_eq!(waypoint_id("  "), "WPT");
        assert_eq!(waypoint_id("Wellington entrance"), "Wellingt");
    }
}
