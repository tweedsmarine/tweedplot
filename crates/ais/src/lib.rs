//! AIS targets for the chart: ships, aids to navigation, SAR aircraft, base
//! stations and the distress beacons (AIS-SART, MOB, EPIRB).
//!
//! Targets come from two places and are merged by MMSI:
//! - the Signal K server (an AIS receiver on board), see [`Target::apply_signalk`];
//! - an online feed of shore stations, [aisstream.io](https://aisstream.io), see [`stream`].
//!
//! Units: degrees true, knots, metres; times are Unix seconds.

pub mod stream;

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

const MS_TO_KN: f64 = 3600.0 / 1852.0;

/// What a target is, from its MMSI and the messages it sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Vessel,
    Aton,
    Aircraft,
    Base,
    /// AIS search-and-rescue transmitter (MMSI 970…).
    Sart,
    /// Man-overboard device (972…).
    Mob,
    /// EPIRB with AIS locating signal (974…).
    Epirb,
}

impl Kind {
    /// The kind an MMSI number says, if it says one.
    pub fn from_mmsi(mmsi: u32) -> Option<Kind> {
        let s = format!("{mmsi:09}");
        if s.starts_with("970") {
            Some(Kind::Sart)
        } else if s.starts_with("972") {
            Some(Kind::Mob)
        } else if s.starts_with("974") {
            Some(Kind::Epirb)
        } else if s.starts_with("111") {
            Some(Kind::Aircraft)
        } else if s.starts_with("99") {
            Some(Kind::Aton)
        } else if s.starts_with("00") {
            Some(Kind::Base)
        } else {
            None
        }
    }
    /// A distress beacon: always an alarm.
    pub fn is_distress(self) -> bool {
        matches!(self, Kind::Sart | Kind::Mob | Kind::Epirb)
    }
}

/// One AIS target as the app shows it.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Target {
    pub mmsi: u32,
    pub kind: Kind,
    pub name: Option<String>,
    pub callsign: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Course and speed over ground, heading (true).
    pub cog: Option<f64>,
    pub sog: Option<f64>,
    pub heading: Option<f64>,
    /// AIS navigational status code (0 under way using engine, 1 at anchor, 5 moored, 7 fishing, 8 sailing …).
    pub status: Option<u8>,
    /// AIS ship and cargo type (30 fishing, 36 sailing, 37 pleasure, 52 tug, 60–69 passenger, 70–79 cargo, 80–89 tanker …).
    pub ship_type: Option<u16>,
    /// Class "A", "B", or None (not a ship, or not known yet).
    pub class: Option<String>,
    pub length: Option<f64>,
    pub beam: Option<f64>,
    /// Where its position comes from (its GPS antenna): m aft of the bow, and m to
    /// starboard of the centre line (negative: to port).
    pub from_bow: Option<f64>,
    pub from_centre: Option<f64>,
    pub draught: Option<f64>,
    pub destination: Option<String>,
    /// For an aid to navigation: its type code (AIS message 21) and whether it is virtual (no buoy there).
    pub aton_type: Option<u8>,
    pub virtual_aton: bool,
    pub off_position: bool,
    /// Altitude, m (SAR aircraft).
    pub altitude: Option<f64>,
    /// When the last position came, Unix s.
    pub last_pos: Option<i64>,
    /// When anything last came, Unix s.
    pub last_seen: i64,
    /// Where it was heard: "signalk", "aisstream", or both joined by "+".
    pub source: String,
}

impl Target {
    pub fn new(mmsi: u32) -> Target {
        Target { mmsi, kind: Kind::from_mmsi(mmsi).unwrap_or_default(), ..Default::default() }
    }

    fn heard(&mut self, source: &str, now: i64) {
        self.last_seen = now;
        if !self.source.split('+').any(|s| s == source) {
            self.source = if self.source.is_empty() { source.to_string() } else { format!("{}+{source}", self.source) };
        }
    }

    fn set_pos(&mut self, lat: f64, lon: f64, now: i64) {
        // 91 / 181 are AIS for "not available"
        if lat.abs() <= 90.0 && lon.abs() <= 180.0 && !(lat == 0.0 && lon == 0.0) {
            self.lat = Some(lat);
            self.lon = Some(lon);
            self.last_pos = Some(now);
        }
    }

    /// Apply the values of one Signal K delta for this target's context.
    /// Returns true if anything changed.
    pub fn apply_signalk(&mut self, values: &[(String, Value)], now: i64) -> bool {
        let before = self.clone();
        for (path, val) in values {
            let num = || val.as_f64();
            match path.as_str() {
                "navigation.position" => {
                    if let (Some(la), Some(lo)) = (val.get("latitude").and_then(Value::as_f64), val.get("longitude").and_then(Value::as_f64)) {
                        self.set_pos(la, lo, now);
                    }
                }
                "navigation.courseOverGroundTrue" => self.cog = num().map(|r| norm360(r.to_degrees())),
                "navigation.speedOverGround" => self.sog = num().map(|v| v * MS_TO_KN),
                "navigation.headingTrue" => self.heading = num().map(|r| norm360(r.to_degrees())),
                "navigation.state" => self.status = val.as_str().and_then(status_code),
                "navigation.destination.commonName" => self.destination = val.as_str().map(str::to_string),
                "design.length" => self.length = val.get("overall").and_then(Value::as_f64).or(num()),
                "design.beam" => self.beam = num(),
                "sensors.ais.fromBow" => self.from_bow = num(),
                "sensors.ais.fromCenter" => self.from_centre = num(),
                "design.draft" => self.draught = val.get("current").or(val.get("maximum")).and_then(Value::as_f64).or(num()),
                "design.aisShipType" => self.ship_type = val.get("id").and_then(Value::as_u64).map(|v| v as u16),
                "sensors.ais.class" => self.class = val.as_str().map(str::to_string),
                "atonType" => {
                    self.kind = Kind::Aton;
                    self.aton_type = val.get("id").and_then(Value::as_u64).map(|v| v as u8);
                }
                "virtual" => self.virtual_aton = val.as_bool().unwrap_or(false),
                "offPosition" => self.off_position = val.as_bool().unwrap_or(false),
                "navigation.position.altitude" | "navigation.altitude" => self.altitude = num(),
                "communication.callsignVhf" => self.callsign = val.as_str().map(str::to_string),
                "name" => self.name = val.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                // the empty path carries the identity: {"name": …, "mmsi": …, "communication": {"callsignVhf": …}}
                "" => {
                    if let Some(n) = val.get("name").and_then(Value::as_str) {
                        let n = n.trim();
                        if !n.is_empty() {
                            self.name = Some(n.to_string());
                        }
                    }
                    if let Some(c) = val.pointer("/communication/callsignVhf").and_then(Value::as_str) {
                        self.callsign = Some(c.to_string());
                    }
                }
                _ => {}
            }
        }
        self.heard("signalk", now);
        if self.status == Some(14) && self.kind == Kind::Vessel {
            self.kind = Kind::Sart; // "ais-sart" state from a ship MMSI (test or active SART on board)
        }
        let mut same = before;
        same.last_seen = self.last_seen;
        same != *self
    }
}

/// AIS navigational status code from the Signal K `navigation.state` text.
fn status_code(s: &str) -> Option<u8> {
    Some(match s {
        "motoring" => 0,
        "anchored" => 1,
        "not under command" => 2,
        "restricted manouverability" | "restricted manoeuvrability" => 3,
        "constrained by draft" => 4,
        "moored" => 5,
        "aground" => 6,
        "fishing" => 7,
        "sailing" => 8,
        "hazardous material high speed" => 9,
        "hazardous material wing in ground" => 10,
        "power-driven vessel towing astern" => 11,
        "power-driven vessel pushing ahead or towing alongside" => 12,
        "ais-sart" => 14,
        "default" => 15,
        _ => return None,
    })
}

fn norm360(d: f64) -> f64 {
    d.rem_euclid(360.0)
}

/// The MMSI in a Signal K context such as `vessels.urn:mrn:imo:mmsi:512000123`,
/// `atons.urn:mrn:imo:mmsi:995126001` or `aircraft.urn:mrn:imo:mmsi:111512001`.
pub fn mmsi_of_context(ctx: &str) -> Option<u32> {
    let i = ctx.find("mmsi:")?;
    ctx[i + 5..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// All targets, by MMSI, with the ones changed since the last [`Targets::take_changes`].
#[derive(Default)]
pub struct Targets {
    pub map: HashMap<u32, Target>,
    changed: std::collections::HashSet<u32>,
    /// Each target's recent positions [t, lon, lat] (see [`TRACK_S`]).
    pub tracks: HashMap<u32, Vec<[f64; 3]>>,
    /// The own vessel's MMSI: never a target.
    pub own: Option<u32>,
}

/// How much track is kept per target, s, and the most points.
pub const TRACK_S: i64 = 6 * 3600;
const TRACK_MAX: usize = 3000;

/// Targets not heard from for this long are dropped.
pub const EXPIRE_S: i64 = 20 * 60;

impl Targets {
    /// Apply a Signal K delta's values for `context` (not the own vessel).
    pub fn apply_signalk(&mut self, context: &str, values: &[(String, Value)], now: i64) {
        let Some(mmsi) = mmsi_of_context(context) else { return };
        if self.own == Some(mmsi) {
            return;
        }
        let t = self.map.entry(mmsi).or_insert_with(|| Target::new(mmsi));
        if context.starts_with("atons.") {
            t.kind = Kind::Aton;
        } else if context.starts_with("aircraft.") {
            t.kind = Kind::Aircraft;
        } else if context.starts_with("shore.basestations") {
            t.kind = Kind::Base;
        }
        if t.apply_signalk(values, now) {
            self.changed.insert(mmsi);
            self.track(mmsi, now);
        }
    }

    /// Add the target's position to its track if it moved (20 m) or 60 s passed.
    fn track(&mut self, mmsi: u32, now: i64) {
        let Some(t) = self.map.get(&mmsi) else { return };
        let (Some(lat), Some(lon)) = (t.lat, t.lon) else { return };
        let tr = self.tracks.entry(mmsi).or_default();
        if let Some(l) = tr.last() {
            let dx = (lon - l[1]) * 111_320.0 * lat.to_radians().cos();
            let dy = (lat - l[2]) * 111_320.0;
            if dx.hypot(dy) < 20.0 && (now as f64 - l[0]) < 60.0 {
                return;
            }
        }
        tr.push([now as f64, lon, lat]);
        let cut = (now - TRACK_S) as f64;
        if tr.first().is_some_and(|p| p[0] < cut) {
            tr.retain(|p| p[0] >= cut);
        }
        if tr.len() > TRACK_MAX {
            // thin the older half
            let half = tr.len() / 2;
            let mut kept: Vec<[f64; 3]> = tr[..half].iter().step_by(2).copied().collect();
            kept.extend_from_slice(&tr[half..]);
            *tr = kept;
        }
    }

    /// Set the own vessel's MMSI (dropping it if it is already a target).
    pub fn set_own(&mut self, own: Option<u32>) {
        self.own = own;
        if let Some(m) = own {
            if self.map.remove(&m).is_some() {
                self.changed.remove(&m);
                self.tracks.remove(&m);
            }
        }
    }

    /// Merge a target update from another source (fields it has replace ours).
    pub fn merge(&mut self, u: Update, now: i64) {
        let mmsi = u.mmsi;
        if self.own == Some(mmsi) {
            return;
        }
        let t = self.map.entry(mmsi).or_insert_with(|| Target::new(mmsi));
        let mut before = t.clone();
        u.apply(t, now);
        before.last_seen = t.last_seen;
        if *t != before {
            self.changed.insert(mmsi);
            self.track(mmsi, now);
        }
    }

    /// Targets changed since the last call, and MMSIs dropped as expired.
    pub fn take_changes(&mut self, now: i64) -> (Vec<Target>, Vec<u32>) {
        let gone: Vec<u32> = self.map.iter().filter(|(_, t)| now - t.last_seen > EXPIRE_S).map(|(m, _)| *m).collect();
        for m in &gone {
            self.map.remove(m);
            self.changed.remove(m);
            self.tracks.remove(m);
        }
        let ch = self.changed.drain().filter_map(|m| self.map.get(&m).cloned()).collect();
        (ch, gone)
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.changed.clear();
        self.tracks.clear();
    }

    /// Every target (for a newly opened view).
    pub fn all(&self) -> Vec<Target> {
        self.map.values().cloned().collect()
    }
}

/// A partial target from one AIS message (fields not in the message are None).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Update {
    pub mmsi: u32,
    pub source: &'static str,
    pub kind: Option<Kind>,
    pub name: Option<String>,
    pub callsign: Option<String>,
    pub pos: Option<(f64, f64)>,
    pub cog: Option<f64>,
    pub sog: Option<f64>,
    pub heading: Option<f64>,
    pub status: Option<u8>,
    pub ship_type: Option<u16>,
    pub class: Option<&'static str>,
    pub length: Option<f64>,
    pub beam: Option<f64>,
    pub from_bow: Option<f64>,
    pub from_centre: Option<f64>,
    pub draught: Option<f64>,
    pub destination: Option<String>,
    pub aton_type: Option<u8>,
    pub virtual_aton: Option<bool>,
    pub off_position: Option<bool>,
    pub altitude: Option<f64>,
}

impl Update {
    fn apply(self, t: &mut Target, now: i64) {
        if let Some(k) = self.kind {
            if !t.kind.is_distress() {
                t.kind = k;
            }
        }
        macro_rules! set {
            ($($f:ident),*) => { $( if self.$f.is_some() { t.$f = self.$f; } )* };
        }
        set!(name, callsign, cog, sog, heading, status, ship_type, length, beam, from_bow, from_centre, draught, destination, aton_type, altitude);
        if let Some(c) = self.class {
            t.class = Some(c.to_string());
        }
        if let Some(v) = self.virtual_aton {
            t.virtual_aton = v;
        }
        if let Some(v) = self.off_position {
            t.off_position = v;
        }
        if let Some((la, lo)) = self.pos {
            t.set_pos(la, lo, now);
        }
        if t.status == Some(14) && t.kind == Kind::Vessel {
            t.kind = Kind::Sart;
        }
        t.heard(self.source, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vals(v: Value) -> Vec<(String, Value)> {
        v.as_array().unwrap().iter().map(|x| (x["path"].as_str().unwrap().to_string(), x["value"].clone())).collect()
    }

    #[test]
    fn signalk_vessel_and_identity() {
        let mut ts = Targets::default();
        let v = vals(json!([
            {"path": "navigation.position", "value": {"latitude": -41.25, "longitude": 173.28}},
            {"path": "navigation.courseOverGroundTrue", "value": std::f64::consts::FRAC_PI_2},
            {"path": "navigation.speedOverGround", "value": 5.0 / MS_TO_KN},
            {"path": "navigation.state", "value": "fishing"},
            {"path": "", "value": {"name": "SANFORD  ", "mmsi": "512000123"}},
            {"path": "design.aisShipType", "value": {"id": 30, "name": "Fishing"}},
            {"path": "sensors.ais.class", "value": "A"}
        ]));
        ts.apply_signalk("vessels.urn:mrn:imo:mmsi:512000123", &v, 1000);
        let (ch, gone) = ts.take_changes(1000);
        assert!(gone.is_empty());
        let t = &ch[0];
        assert_eq!((t.mmsi, t.kind, t.name.as_deref(), t.status, t.ship_type), (512000123, Kind::Vessel, Some("SANFORD"), Some(7), Some(30)));
        assert!((t.cog.unwrap() - 90.0).abs() < 1e-9 && (t.sog.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(t.class.as_deref(), Some("A"));
        // nothing new: no change reported; then it expires
        ts.apply_signalk("vessels.urn:mrn:imo:mmsi:512000123", &v, 1001);
        assert!(ts.take_changes(1001).0.len() <= 1);
        let (_, gone) = ts.take_changes(1001 + EXPIRE_S + 1);
        assert_eq!(gone, vec![512000123]);
    }

    #[test]
    fn beacons_atons_and_merging() {
        assert_eq!(Kind::from_mmsi(970123456), Some(Kind::Sart));
        assert_eq!(Kind::from_mmsi(972000001), Some(Kind::Mob));
        assert_eq!(Kind::from_mmsi(974000001), Some(Kind::Epirb));
        assert_eq!(Kind::from_mmsi(995121001), Some(Kind::Aton));
        assert_eq!(Kind::from_mmsi(5120001), Some(Kind::Base)); // 005120001
        assert_eq!(mmsi_of_context("atons.urn:mrn:imo:mmsi:995121001"), Some(995121001));
        let mut ts = Targets::default();
        ts.merge(Update { mmsi: 512000123, source: "aisstream", pos: Some((-41.0, 173.0)), sog: Some(3.0), ..Default::default() }, 10);
        ts.apply_signalk("vessels.urn:mrn:imo:mmsi:512000123", &vals(json!([{"path": "navigation.speedOverGround", "value": 4.0 / MS_TO_KN}])), 11);
        let t = &ts.map[&512000123];
        assert_eq!(t.source, "aisstream+signalk");
        assert!((t.sog.unwrap() - 4.0).abs() < 1e-9);
        assert_eq!(t.lat, Some(-41.0));
        // "not available" positions are ignored
        ts.merge(Update { mmsi: 512000123, source: "aisstream", pos: Some((91.0, 181.0)), ..Default::default() }, 12);
        assert_eq!(ts.map[&512000123].lat, Some(-41.0));
        // the own MMSI is never a target
        ts.set_own(Some(512000123));
        assert!(!ts.map.contains_key(&512000123));
        ts.merge(Update { mmsi: 512000123, source: "aisstream", pos: Some((-41.0, 173.0)), ..Default::default() }, 13);
        assert!(!ts.map.contains_key(&512000123));
        // tracks build up as a target moves
        for k in 0..5 {
            ts.merge(Update { mmsi: 512000555, source: "aisstream", pos: Some((-41.0 + k as f64 * 0.001, 173.0)), ..Default::default() }, 100 + k * 10);
        }
        assert_eq!(ts.tracks[&512000555].len(), 5);
        // an AIS-SART state turns a ship MMSI into a SART alarm
        ts.merge(Update { mmsi: 512000999, source: "aisstream", status: Some(14), pos: Some((-41.1, 173.1)), ..Default::default() }, 12);
        assert_eq!(ts.map[&512000999].kind, Kind::Sart);
    }
}
