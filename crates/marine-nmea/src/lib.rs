//! NMEA 0183 frame validation and normalized navigation observations.
//!
//! Every valid sentence, including sentences not decoded yet, is retained as
//! a frame so catalog coverage can be extended without losing source data.

pub mod alerts;
pub mod autopilot;
pub mod dsc;
pub mod radar;
pub mod fixstreams;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_MAX_FRAME_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sentence {
    pub talker: String,
    pub formatter: String,
    pub fields: Vec<String>,
    pub raw: String,
    pub checksum_valid: bool,
}

fn relative_angle(value: Option<f64>) -> Option<f64> {
    value.map(|a| if a > 180.0 { a - 360.0 } else { a })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub cog: Option<f64>,
    pub sog: Option<f64>,
    pub heading: Option<f64>,
    pub heading_mag: Option<f64>,
    pub variation: Option<f64>,
    pub stw: Option<f64>,
    pub depth: Option<f64>,
    pub depth_keel: Option<f64>,
    pub depth_surface: Option<f64>,
    pub aws: Option<f64>,
    pub awa: Option<f64>,
    pub twa: Option<f64>,
    pub tws: Option<f64>,
    pub twd: Option<f64>,
    pub water_temp: Option<f64>,
    /// Air temperature (°C), barometric pressure (hPa) and relative humidity (%),
    /// from MDA or XDR.
    pub air_temp: Option<f64>,
    pub pressure: Option<f64>,
    pub humidity: Option<f64>,
    pub datetime: Option<String>,
    /// Rate of turn, degrees a minute, + = to starboard (ROT).
    pub rot: Option<f64>,
    /// Heel (roll) and pitch, degrees: heel + = to starboard, pitch + = bow up (XDR angles).
    pub heel: Option<f64>,
    pub pitch: Option<f64>,
    /// Rudder angle, degrees, + = to starboard (RSA).
    pub rudder: Option<f64>,
    /// Set (degrees true, the way the current flows) and drift (knots), as the instruments
    /// work them out (VDR).
    pub set: Option<f64>,
    pub drift: Option<f64>,
    /// Total log and trip through the water, NM (VLW).
    pub log: Option<f64>,
    pub trip: Option<f64>,
    /// Position quality: satellites used (GGA), horizontal dilution of precision (GGA, GSA),
    /// fix (GSA: 1 none, 2 2D, 3 3D) and the estimated position error in metres (GST).
    pub sats: Option<f64>,
    pub hdop: Option<f64>,
    pub fix_dim: Option<f64>,
    pub pos_error: Option<f64>,
    /// Battery voltage (XDR, type U, a name with BAT) and engine revolutions a minute (RPM, XDR).
    pub battery_v: Option<f64>,
    pub rpm: Option<f64>,
    /// The GNSS receiver's local datum (DTM), as its code: W84 is WGS 84, what the charts use.
    pub datum: Option<String>,
    pub extra: BTreeMap<String, String>,
    /// The heading came from a true-heading sensor (HDT, VHW), so a magnetic heading
    /// doesn't replace it.
    #[serde(skip)]
    pub heading_from_true: bool,
    /// Magnetic headings since the last true one: when the true-heading sensor goes
    /// quiet (MAG_TAKES_OVER of them), the magnetic heading takes over again.
    #[serde(skip)]
    pub mag_since_true: u32,
}

/// See [`Observation::mag_since_true`].
const MAG_TAKES_OVER: u32 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    TooLong,
    BadStart,
    MissingChecksum,
    BadChecksum,
    Malformed,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLong => "sentence exceeds configured maximum length",
            Self::BadStart => "sentence must start with $ or !",
            Self::MissingChecksum => "sentence has no checksum",
            Self::BadChecksum => "sentence checksum does not match",
            Self::Malformed => "sentence identifier is malformed",
        })
    }
}

impl std::error::Error for ParseError {}

/// Accumulates byte-stream input, emits validated CR/LF-delimited sentences.
#[derive(Debug)]
pub struct Framer {
    buffer: Vec<u8>,
    max_len: usize,
    discarding: bool,
}

impl Default for Framer {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FRAME_LEN)
    }
}

impl Framer {
    pub fn new(max_len: usize) -> Self {
        Self { buffer: Vec::new(), max_len: max_len.max(16), discarding: false }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Sentence, ParseError>> {
        self.push_with_raw(bytes).into_iter().map(|(_, parsed)| parsed).collect()
    }

    /// Like [`Framer::push`], but also retains the input bytes for diagnostics when
    /// parsing fails.
    pub fn push_with_raw(&mut self, bytes: &[u8]) -> Vec<(Vec<u8>, Result<Sentence, ParseError>)> {
        let mut out = Vec::new();
        for &byte in bytes {
            if byte == b'\n' || byte == b'\r' {
                if !self.discarding && !self.buffer.is_empty() {
                    let raw = std::mem::take(&mut self.buffer);
                    let parsed = parse_sentence(&raw, self.max_len);
                    out.push((raw, parsed));
                }
                self.buffer.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.buffer.len() >= self.max_len {
                    let raw = std::mem::take(&mut self.buffer);
                    self.discarding = true;
                    out.push((raw, Err(ParseError::TooLong)));
                } else {
                    self.buffer.push(byte);
                }
            }
        }
        out
    }
}

pub fn parse_sentence(bytes: &[u8], max_len: usize) -> Result<Sentence, ParseError> {
    if bytes.len() > max_len {
        return Err(ParseError::TooLong);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::Malformed)?.trim();
    if !text.starts_with(['$', '!']) {
        return Err(ParseError::BadStart);
    }
    let star = text.rfind('*').ok_or(ParseError::MissingChecksum)?;
    if star + 3 != text.len() {
        return Err(ParseError::Malformed);
    }
    let expected = u8::from_str_radix(&text[star + 1..], 16).map_err(|_| ParseError::Malformed)?;
    let checksum = text.as_bytes()[1..star].iter().fold(0u8, |sum, byte| sum ^ byte);
    if checksum != expected {
        return Err(ParseError::BadChecksum);
    }

    let body = &text[1..star];
    let mut fields = body.split(',').map(str::to_owned);
    let id = fields.next().ok_or(ParseError::Malformed)?;
    if !id.is_ascii() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(ParseError::Malformed);
    }
    let (talker, formatter): (&str, &str) = if id.starts_with('P') {
        if id.len() < 4 {
            return Err(ParseError::Malformed);
        }
        (&id[..1], id.as_str())
    } else {
        if id.len() < 5 {
            return Err(ParseError::Malformed);
        }
        let split = id.len().saturating_sub(3);
        id.split_at(split)
    };
    if formatter.is_empty() {
        return Err(ParseError::Malformed);
    }
    Ok(Sentence { talker: talker.to_owned(), formatter: formatter.to_owned(), fields: fields.collect(), raw: text.to_owned(), checksum_valid: true })
}

impl Observation {
    /// Fold one sentence into the observation. A field the sentence leaves empty keeps
    /// its last value (whether it is still fresh is the multiplexer's business: it only
    /// counts a data group as heard when the sentence had the numbers).
    pub fn apply(&mut self, sentence: &Sentence) {
        /// Overwrite only with a value.
        fn put(slot: &mut Option<f64>, v: Option<f64>) {
            if v.is_some() {
                *slot = v;
            }
        }
        let f = &sentence.fields;
        let mut heading_true = None;
        match sentence.formatter.as_str() {
            "RMC" if f.len() >= 9 && f[1] == "A" => {
                self.position(&f[2], &f[3], &f[4], &f[5]);
                put(&mut self.sog, number(&f[6]));
                put(&mut self.cog, angle(&f[7]));
                if let Some(datetime) = rmc_datetime(&f[8], &f[0]) {
                    self.datetime = Some(datetime);
                }
                put(&mut self.variation, signed(number(f.get(9).map(String::as_str).unwrap_or("")), f.get(10).map(String::as_str)));
            }
            "GGA" if f.len() >= 9 => {
                if number(&f[5]).is_some_and(|quality| quality > 0.0) {
                    self.position(&f[1], &f[2], &f[3], &f[4]);
                    put(&mut self.sats, number(&f[6]));
                    put(&mut self.hdop, number(&f[7]).filter(|h| *h > 0.0 && *h < 99.0));
                }
            }
            // fix mode (1 no fix, 2 2D, 3 3D), the satellites in use, PDOP, HDOP, VDOP
            "GSA" if f.len() >= 17 => {
                put(&mut self.fix_dim, number(&f[1]).filter(|d| (1.0..=3.0).contains(d)));
                put(&mut self.hdop, number(&f[15]).filter(|h| *h > 0.0 && *h < 99.0));
            }
            // pseudorange errors: the 1-sigma latitude and longitude errors, metres
            "GST" if f.len() >= 7 => {
                if let (Some(la), Some(lo)) = (number(&f[5]), number(&f[6])) {
                    self.pos_error = Some((la * la + lo * lo).sqrt());
                }
            }
            // rate of turn, degrees a minute (- = to port), and A when valid
            "ROT" if !f.is_empty() && f.get(1).is_none_or(|s| s == "A") => put(&mut self.rot, number(&f[0])),
            // rudder: starboard (or single) rudder angle, A when valid; - = to port
            "RSA" if f.len() >= 2 && f[1] == "A" => put(&mut self.rudder, number(&f[0])),
            // set and drift: direction true, T, direction magnetic, M, speed, N
            "VDR" if f.len() >= 6 => {
                if f[1] == "T" {
                    put(&mut self.set, angle(&f[0]));
                }
                if f[5] == "N" {
                    put(&mut self.drift, number(&f[4]));
                }
            }
            // total and trip distance through the water, NM
            "VLW" if f.len() >= 4 => {
                if f[1] == "N" {
                    put(&mut self.log, number(&f[0]));
                }
                if f[3] == "N" {
                    put(&mut self.trip, number(&f[2]));
                }
            }
            // relative (VWR) and true (VWT) wind: angle 0–180 off the bow, L or R, then knots (N),
            // metres a second (M) or km/h (K) — the older sentences many instruments still send
            "VWR" | "VWT" if f.len() >= 4 => {
                let a = number(&f[0]).filter(|a| (0.0..=180.0).contains(a));
                let side = match f[1].as_str() {
                    "L" => Some(-1.0),
                    "R" => Some(1.0),
                    _ => None,
                };
                // the first of knots, m/s, km/h that has a figure (instruments often fill only one)
                let unit = |i: usize, u: &str, k: f64| f.get(i + 1).filter(|x| *x == u).and(f.get(i)).and_then(|v| number(v)).map(|v| v * k);
                let speed = unit(2, "N", 1.0).or_else(|| unit(4, "M", 1.943_844_5)).or_else(|| unit(6, "K", 0.539_956_8));
                let angle = a.zip(side).map(|(a, s)| a * s);
                if sentence.formatter == "VWR" {
                    put(&mut self.awa, angle);
                    put(&mut self.aws, speed);
                } else {
                    put(&mut self.twa, angle);
                    put(&mut self.tws, speed);
                }
            }
            // engine (E) or shaft (S) revolutions: source, number, rpm, pitch %, A when valid
            "RPM" if f.len() >= 5 && f[4] == "A" && f[0] == "E" => put(&mut self.rpm, number(&f[2])),
            "GLL" if f.len() >= 6 && f[5] == "A" => {
                self.position(&f[0], &f[1], &f[2], &f[3]);
            }
            "VTG" if f.len() >= 6 => {
                put(&mut self.cog, angle(&f[0]));
                put(
                    &mut self.sog,
                    if f.get(5).is_some_and(|unit| unit == "N") {
                        number(&f[4])
                    } else if f.get(7).is_some_and(|unit| unit == "K") {
                        number(&f[6]).map(|k| k * 0.539_956_8)
                    } else {
                        None
                    },
                );
            }
            "HDT" if f.len() >= 2 && f[1] == "T" => heading_true = angle(&f[0]),
            // true heading and its mode, as satellite compasses send it: A autonomous or E estimated
            // (dead reckoning) are used; M manual, S simulator and V not valid are not
            "THS" if f.len() >= 2 && matches!(f[1].as_str(), "A" | "E") => heading_true = angle(&f[0]),
            // dual-axis speed log: longitudinal speed through the water (kn) when its status is A
            "VBW" if f.len() >= 3 && f[2] == "A" => put(&mut self.stw, number(&f[0])),
            // the receiver's datum: local datum code first (W84, W72, S85, P90, 999 user-defined)
            "DTM" if !f.is_empty() && !f[0].trim().is_empty() => self.datum = Some(f[0].trim().to_string()),
            "HDM" if f.len() >= 2 && f[1] == "M" => put(&mut self.heading_mag, angle(&f[0])),
            "HDG" if !f.is_empty() => {
                put(&mut self.heading_mag, angle(&f[0]));
                if f.len() >= 5 {
                    put(&mut self.variation, signed(number(&f[3]), f.get(4).map(String::as_str)));
                }
            }
            "VHW" if f.len() >= 6 => {
                if f.get(1).is_some_and(|unit| unit == "T") {
                    heading_true = angle(&f[0]);
                }
                if f.get(3).is_some_and(|unit| unit == "M") {
                    put(&mut self.heading_mag, angle(&f[2]));
                }
                put(
                    &mut self.stw,
                    if f.get(5).is_some_and(|unit| unit == "N") {
                        number(&f[4])
                    } else if f.get(7).is_some_and(|unit| unit == "K") {
                        number(&f[6]).map(|k| k * 0.539_956_8)
                    } else {
                        None
                    },
                );
            }
            "MWV" if f.len() >= 5 && f[4] == "A" => {
                let angle = angle(&f[0]);
                let speed = number(&f[2]).and_then(|s| speed_knots(s, &f[3]));
                match f[1].as_str() {
                    "R" => {
                        put(&mut self.awa, relative_angle(angle));
                        put(&mut self.aws, speed);
                    }
                    "T" => {
                        put(&mut self.twa, relative_angle(angle));
                        put(&mut self.tws, speed);
                    }
                    _ => {}
                }
            }
            "MWD" if f.len() >= 8 => {
                if f[1] == "T" {
                    put(&mut self.twd, angle(&f[0]));
                }
                put(
                    &mut self.tws,
                    if f[5] == "N" {
                        number(&f[4])
                    } else if f[7] == "M" {
                        number(&f[6]).map(|m| m * 1.943_844_5)
                    } else {
                        None
                    },
                );
            }
            "DPT" if !f.is_empty() => put(&mut self.depth, number(&f[0])),
            "DBT" if f.len() >= 5 => put(&mut self.depth, number(&f[2]).or_else(|| number(&f[0]).map(|ft| ft * 0.3048))),
            "DBK" if f.len() >= 4 => put(&mut self.depth_keel, number(&f[2]).or_else(|| number(&f[0]).map(|ft| ft * 0.3048))),
            "DBS" if f.len() >= 4 => put(&mut self.depth_surface, number(&f[2]).or_else(|| number(&f[0]).map(|ft| ft * 0.3048))),
            "MTW" if f.len() >= 2 && f[1] == "C" => put(&mut self.water_temp, number(&f[0])),
            "MDA" | "XDR" => {
                let w = weather(&sentence.formatter, f);
                put(&mut self.pressure, w.pressure);
                put(&mut self.air_temp, w.air_temp);
                put(&mut self.humidity, w.humidity);
                if sentence.formatter == "XDR" {
                    let x = xdr_other(f);
                    put(&mut self.heel, x.heel);
                    put(&mut self.pitch, x.pitch);
                    put(&mut self.battery_v, x.battery_v);
                    put(&mut self.rpm, x.rpm);
                }
            }
            "ZDA" if f.len() >= 4 => {
                if let Some(datetime) = zda_datetime(&f[0], &f[1], &f[2], &f[3]) {
                    self.datetime = Some(datetime);
                }
            }
            _ => {}
        }
        self.extra.insert(sentence.formatter.clone(), sentence.fields.join(","));
        // True heading: from a true-heading sensor (HDT, VHW true) when there is one,
        // else the magnetic heading corrected for variation, worked out afresh each time
        // the magnetic heading comes in (a fluxgate compass sends only HDG or HDM).
        if heading_true.is_some() {
            self.heading = heading_true;
            self.heading_from_true = true;
            self.mag_since_true = 0;
        } else if matches!(sentence.formatter.as_str(), "HDG" | "HDM") && self.heading_from_true {
            self.mag_since_true += 1;
            if self.mag_since_true >= MAG_TAKES_OVER {
                self.heading_from_true = false;
            }
        }
        if heading_true.is_none() && !self.heading_from_true && matches!(sentence.formatter.as_str(), "HDG" | "HDM" | "VHW" | "RMC") {
            if let (Some(magnetic), Some(variation)) = (self.heading_mag, self.variation) {
                self.heading = Some((magnetic + variation).rem_euclid(360.0));
            }
        }
        if matches!(sentence.formatter.as_str(), "HDT" | "THS" | "HDG" | "HDM" | "VHW" | "MWD") {
            if let (Some(direction), Some(heading)) = (self.twd, self.heading) {
                self.twa = Some((direction - heading + 180.0).rem_euclid(360.0) - 180.0);
            }
        }
    }

    fn position(&mut self, lat: &str, ns: &str, lon: &str, ew: &str) {
        self.lat = nmea_coord(lat, ns, true);
        self.lon = nmea_coord(lon, ew, false);
    }
}

/// Barometric pressure (hPa), air temperature (°C) and relative humidity (%) in one sentence.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Weather {
    pub pressure: Option<f64>,
    pub air_temp: Option<f64>,
    pub humidity: Option<f64>,
}

impl Weather {
    pub fn any(&self) -> bool {
        self.pressure.is_some() || self.air_temp.is_some() || self.humidity.is_some()
    }
}

/// A barometer reads 870–1090 hPa at sea level; anything else (an engine's oil pressure in
/// an XDR, a wrong unit) is not the air pressure.
fn barometric(hpa: Option<f64>) -> Option<f64> {
    hpa.filter(|p| (870.0..=1090.0).contains(p))
}

/// The weather in an MDA (meteorological composite) or XDR (transducer) sentence.
///
/// MDA: pressure in inches of mercury (I) and bars (B), air temperature (C), water
/// temperature, relative humidity (%), … — the bars are used when present.
/// XDR: groups of type, value, unit, name. Pressure (P) in bars (B) or pascals (P)
/// within the barometric range; temperature (C, in C) whose name says air (Air,
/// TempAir, ENV_OUTSIDE_T…); humidity (H, in P = percent).
pub fn weather(formatter: &str, f: &[String]) -> Weather {
    let field = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
    let mut w = Weather::default();
    match formatter {
        "MDA" => {
            w.pressure = barometric(if field(3) == "B" { number(field(2)).map(|b| b * 1000.0) } else { None }.or_else(|| {
                if field(1) == "I" {
                    number(field(0)).map(|inhg| inhg * 33.863_886)
                } else {
                    None
                }
            }));
            w.air_temp = if field(5) == "C" { number(field(4)) } else { None };
            w.humidity = number(field(8)).filter(|h| (0.0..=100.0).contains(h));
        }
        "XDR" => {
            for g in f.chunks(4) {
                let (kind, value, unit) = (g[0].as_str(), g.get(1).and_then(|v| number(v)), g.get(2).map(String::as_str).unwrap_or(""));
                let name = g.get(3).map(|n| n.to_ascii_uppercase()).unwrap_or_default();
                match kind {
                    "P" => {
                        let hpa = match unit {
                            "B" => value.map(|b| b * 1000.0),
                            "P" => value.map(|pa| pa / 100.0),
                            _ => None,
                        };
                        if w.pressure.is_none() {
                            w.pressure = barometric(hpa);
                        }
                    }
                    "C" if unit == "C" && (name.contains("AIR") || name.contains("OUTSIDE") || name == "ENV_OUTAIR_T") => {
                        w.air_temp = w.air_temp.or(value.filter(|t| (-60.0..=70.0).contains(t)));
                    }
                    "H" if unit == "P" => w.humidity = w.humidity.or(value.filter(|h| (0.0..=100.0).contains(h))),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    w
}

/// What else an XDR carries besides the weather: heel and pitch, battery voltage, engine revs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct XdrOther {
    pub heel: Option<f64>,
    pub pitch: Option<f64>,
    pub battery_v: Option<f64>,
    pub rpm: Option<f64>,
}

impl XdrOther {
    pub fn any(&self) -> bool {
        self.heel.is_some() || self.pitch.is_some() || self.battery_v.is_some() || self.rpm.is_some()
    }
    pub fn attitude(&self) -> bool {
        self.heel.is_some() || self.pitch.is_some()
    }
    pub fn engine(&self) -> bool {
        self.battery_v.is_some() || self.rpm.is_some()
    }
}

/// Angles (type A, in D degrees) named ROLL or HEEL (+ = to starboard) and PTCH or PITCH
/// (+ = bow up), as Airmar, B&G and Signal K's NMEA output name them; voltage (U, in V)
/// with BAT in its name; tachometer (T, in R rpm).
pub fn xdr_other(f: &[String]) -> XdrOther {
    let mut x = XdrOther::default();
    for g in f.chunks(4) {
        let (kind, value, unit) = (g[0].as_str(), g.get(1).and_then(|v| number(v)), g.get(2).map(String::as_str).unwrap_or(""));
        let name = g.get(3).map(|n| n.to_ascii_uppercase()).unwrap_or_default();
        match kind {
            "A" if unit == "D" && (name.contains("ROLL") || name.contains("HEEL")) => {
                x.heel = x.heel.or(value.filter(|a| (-90.0..=90.0).contains(a)));
            }
            "A" if unit == "D" && (name.contains("PTCH") || name.contains("PITCH")) => {
                x.pitch = x.pitch.or(value.filter(|a| (-90.0..=90.0).contains(a)));
            }
            "U" if unit == "V" && name.contains("BAT") => x.battery_v = x.battery_v.or(value.filter(|v| (0.0..=100.0).contains(v))),
            "T" if unit == "R" => x.rpm = x.rpm.or(value.filter(|r| (0.0..=20_000.0).contains(r))),
            _ => {}
        }
    }
    x
}

fn number(s: &str) -> Option<f64> {
    let n = s.parse::<f64>().ok()?;
    n.is_finite().then_some(n)
}

fn angle(s: &str) -> Option<f64> {
    number(s).map(|a| a.rem_euclid(360.0))
}

fn signed(value: Option<f64>, side: Option<&str>) -> Option<f64> {
    let value = value?;
    match side? {
        "L" | "W" | "P" => Some(-value),
        "R" | "E" | "S" => Some(value),
        _ => None,
    }
}

fn speed_knots(value: f64, unit: &str) -> Option<f64> {
    match unit {
        "N" => Some(value),
        "K" => Some(value * 0.539_956_8),
        "M" => Some(value * 1.943_844_5),
        _ => None,
    }
}

fn nmea_coord(value: &str, hemisphere: &str, latitude: bool) -> Option<f64> {
    let raw = number(value)?;
    if raw < 0.0 {
        return None;
    }
    let degrees = (raw / 100.0).floor();
    let minutes = raw - degrees * 100.0;
    if !(0.0..60.0).contains(&minutes) {
        return None;
    }
    let mut result = degrees + minutes / 60.0;
    let (positive, negative, limit) = if latitude { ("N", "S", 90.0) } else { ("E", "W", 180.0) };
    if result > limit {
        return None;
    }
    match hemisphere {
        h if h == positive => {}
        h if h == negative => result = -result,
        _ => return None,
    }
    Some(result)
}

fn format_nmea_time(time: &str) -> String {
    if time.len() < 6 {
        return time.to_owned();
    }
    format!("{}:{}:{}", &time[0..2], &time[2..4], &time[4..])
}

fn rmc_datetime(date: &str, time: &str) -> Option<String> {
    validate_time(time)?;
    if date.len() != 6 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let day = date[0..2].parse::<u32>().ok()?;
    let month = date[2..4].parse::<u32>().ok()?;
    let short_year = date[4..6].parse::<u32>().ok()?;
    let year = if short_year >= 80 { 1900 + short_year } else { 2000 + short_year };
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}T{}Z", format_nmea_time(time)))
}

fn zda_datetime(time: &str, day: &str, month: &str, year: &str) -> Option<String> {
    validate_time(time)?;
    let day = day.parse::<u32>().ok()?;
    let month = month.parse::<u32>().ok()?;
    let year = year.parse::<u32>().ok()?;
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) || year == 0 {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}T{}Z", format_nmea_time(time)))
}

fn validate_time(time: &str) -> Option<()> {
    if time.len() < 6 || !time.is_ascii() || !time[..6].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hour = time[0..2].parse::<u32>().ok()?;
    let minute = time[2..4].parse::<u32>().ok()?;
    let second = time[4..6].parse::<u32>().ok()?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let suffix = &time[6..];
    if !suffix.is_empty() && (!suffix.starts_with('.') || suffix.len() == 1 || !suffix[1..].bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Coverage {
    Decoded,
    PreservedRaw,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub formatter: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub coverage: Coverage,
}

macro_rules! entry {
    ($formatter:literal, $name:literal, $category:literal, $coverage:ident) => {
        CatalogEntry { formatter: $formatter, name: $name, category: $category, coverage: Coverage::$coverage }
    };
}

pub const CATALOG: &[CatalogEntry] = &[
    entry!("AAM", "Waypoint arrival alarm", "Waypoint", PreservedRaw),
    entry!("VDM", "AIS messages from other vessels", "AIS", PreservedRaw),
    entry!("VDO", "AIS own-vessel messages", "AIS", PreservedRaw),
    entry!("ALR", "Alarm", "Safety", Decoded),
    entry!("APB", "Autopilot sentence B", "Autopilot", PreservedRaw),
    entry!("BOD", "Bearing origin to destination", "Waypoint", PreservedRaw),
    entry!("BWC", "Bearing and distance to waypoint", "Waypoint", PreservedRaw),
    entry!("BWW", "Bearing waypoint to waypoint", "Waypoint", PreservedRaw),
    entry!("DSC", "Digital selective calling (distress, urgency, safety)", "Safety", Decoded),
    entry!("DSE", "DSC expansion (enhanced position)", "Safety", Decoded),
    entry!("THS", "True heading and status", "Heading", Decoded),
    entry!("VBW", "Dual ground/water speed", "Speed", Decoded),
    entry!("DTM", "Datum reference", "GNSS", Decoded),
    entry!("ALF", "Alert sentence (bridge alert management)", "Safety", Decoded),
    entry!("TTM", "Tracked target message (radar)", "Radar", Decoded),
    entry!("TLL", "Target latitude and longitude (radar)", "Radar", Decoded),
    entry!("DBK", "Depth below keel", "Depth", Decoded),
    entry!("DBS", "Depth below surface", "Depth", Decoded),
    entry!("DBT", "Depth below transducer", "Depth", Decoded),
    entry!("DPT", "Depth", "Depth", Decoded),
    entry!("GGA", "GNSS fix data", "Position", Decoded),
    entry!("GLL", "Geographic position", "Position", Decoded),
    entry!("GNS", "GNSS fix data", "Position", PreservedRaw),
    entry!("GSA", "GNSS DOP and active satellites", "GNSS", Decoded),
    entry!("GST", "GNSS pseudorange error statistics", "GNSS", Decoded),
    entry!("GSV", "Satellites in view", "GNSS", PreservedRaw),
    entry!("HDG", "Heading, deviation and variation", "Heading", Decoded),
    entry!("HDM", "Heading, magnetic", "Heading", Decoded),
    entry!("HDT", "Heading, true", "Heading", Decoded),
    entry!("MWD", "Wind direction and speed", "Wind", Decoded),
    entry!("MWV", "Wind angle and speed", "Wind", Decoded),
    entry!("MDA", "Meteorological composite (pressure, air temperature, humidity)", "Environment", Decoded),
    entry!("MTW", "Mean water temperature", "Environment", Decoded),
    entry!("RMB", "Recommended minimum navigation data", "Waypoint", PreservedRaw),
    entry!("RMC", "Recommended minimum GNSS data", "Position", Decoded),
    entry!("ROT", "Rate of turn", "Motion", Decoded),
    entry!("RPM", "Engine or shaft revolutions", "Engine", Decoded),
    entry!("RSA", "Rudder sensor angle", "Steering", Decoded),
    entry!("RTE", "Routes", "Waypoint", PreservedRaw),
    entry!("VDR", "Set and drift", "Current", Decoded),
    entry!("VHW", "Water speed and heading", "Speed", Decoded),
    entry!("VLW", "Distance travelled through water", "Log", Decoded),
    entry!("VTG", "Course and speed over ground", "Position", Decoded),
    entry!("VWR", "Relative wind speed and angle", "Wind", Decoded),
    entry!("VWT", "True wind speed and angle", "Wind", Decoded),
    entry!("WPL", "Waypoint location", "Waypoint", PreservedRaw),
    entry!("XDR", "Transducer measurements (barometer, air temperature, humidity, heel, pitch, battery voltage and engine revs used)", "Sensors", Decoded),
    entry!("XTE", "Cross-track error", "Waypoint", PreservedRaw),
    entry!("ZDA", "Time and date", "Time", Decoded),
];

pub fn search_catalog(query: &str) -> Vec<CatalogEntry> {
    let q = query.trim().to_ascii_lowercase();
    CATALOG
        .iter()
        .copied()
        .filter(|e| {
            q.is_empty()
                || e.formatter.to_ascii_lowercase().contains(&q)
                || e.name.to_ascii_lowercase().contains(&q)
                || e.category.to_ascii_lowercase().contains(&q)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> String {
        let sum = body.bytes().fold(0u8, |s, b| s ^ b);
        format!("${body}*{sum:02X}\r\n")
    }

    #[test]
    fn frames_across_chunks_and_rejects_bad_checksum() {
        let sentence = frame("GPRMC,092750.000,A,5321.6802,N,00630.3372,W,5.5,84.4,230394,3.1,E");
        let mut f = Framer::default();
        assert!(f.push(&sentence.as_bytes()[..12]).is_empty());
        let out = f.push(&sentence.as_bytes()[12..]);
        let parsed = out.into_iter().next().unwrap().unwrap();
        assert_eq!(parsed.formatter, "RMC");
        assert!(parse_sentence(b"$GPHDT,123.4,T*00", 1024).is_err());
        assert_eq!(parse_sentence(frame("RMC,field").trim().as_bytes(), 1024), Err(ParseError::Malformed));
    }

    #[test]
    fn diagnostic_framing_keeps_invalid_sentence_bytes_and_recovers() {
        let valid = frame("GPRMC,092750.000,A,5321.6802,N,00630.3372,W,5.5,84.4,230394,3.1,E");
        let mut f = Framer::default();
        let input = format!("$GPHDT,123.4,T*00\r\n{valid}");
        let out = f.push_with_raw(input.as_bytes());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, b"$GPHDT,123.4,T*00");
        assert_eq!(out[0].1, Err(ParseError::BadChecksum));
        assert_eq!(out[1].1.as_ref().unwrap().formatter, "RMC");
    }

    #[test]
    fn decodes_rmc_position_and_motion() {
        let s = parse_sentence(frame("GPRMC,092750.000,A,5321.6802,N,00630.3372,W,5.5,84.4,230394,3.1,E").trim().as_bytes(), 1024).unwrap();
        let mut o = Observation::default();
        o.apply(&s);
        assert!((o.lat.unwrap() - 53.3613367).abs() < 1e-5);
        assert!((o.lon.unwrap() + 6.50562).abs() < 1e-4);
        assert_eq!(o.sog, Some(5.5));
        assert_eq!(o.cog, Some(84.4));
        assert_eq!(o.datetime.as_deref(), Some("1994-03-23T09:27:50.000Z"));
    }

    #[test]
    fn frames_unknown_valid_sentences_and_searches_catalog() {
        let s = parse_sentence(frame("PXYZ,one,two").trim().as_bytes(), 1024).unwrap();
        assert_eq!(s.formatter, "PXYZ");
        assert!(search_catalog("depth").iter().any(|entry| entry.formatter == "DPT"));
        let mut o = Observation::default();
        o.apply(&s);
        assert_eq!(o.extra["PXYZ"], "one,two");
    }

    #[test]
    fn decodes_heading_wind_depth_and_zda_field_order() {
        let mut o = Observation::default();
        for body in
            ["IIHDG,123.4,,,3.1,W", "WIMWV,225.0,R,10.0,K,A", "WIMWD,225.0,T,224.0,M,10.0,N,5.1,M", "SDDPT,12.5,0.3,100.0", "GPZDA,201530.00,04,07,2002,00,00"]
        {
            let sentence = parse_sentence(frame(body).trim().as_bytes(), 1024).unwrap();
            o.apply(&sentence);
        }
        assert_eq!(o.heading_mag, Some(123.4));
        assert_eq!(o.variation, Some(-3.1));
        assert!((o.heading.unwrap() - 120.3).abs() < 1e-9);
        assert_eq!(o.awa, Some(-135.0));
        assert!((o.aws.unwrap() - 5.399_568).abs() < 1e-6);
        assert_eq!(o.twd, Some(225.0));
        assert!((o.twa.unwrap() - 104.7).abs() < 1e-9);
        assert_eq!(o.tws, Some(10.0));
        assert_eq!(o.depth, Some(12.5));
        assert_eq!(o.datetime.as_deref(), Some("2002-07-04T20:15:30.00Z"));

        let sentence = parse_sentence(frame("WIMWV,225.0,T,10.0,K,A").trim().as_bytes(), 1024).unwrap();
        o.apply(&sentence);
        assert_eq!(o.twa, Some(-135.0));
    }

    #[test]
    fn does_not_accept_void_gll_or_invalid_rmc_datetime() {
        let mut o = Observation::default();
        let gll = parse_sentence(frame("GPGLL,5321.6802,N,00630.3372,W,092750.000,V").trim().as_bytes(), 1024).unwrap();
        o.apply(&gll);
        assert_eq!(o.lat, None);
        let rmc = parse_sentence(frame("GPRMC,250000.000,A,5321.6802,N,00630.3372,W,5.5,84.4,230394").trim().as_bytes(), 1024).unwrap();
        o.apply(&rmc);
        assert_eq!(o.datetime, None);
    }

    fn sentence(body: &str) -> Sentence {
        let c = body.bytes().fold(0u8, |a, b| a ^ b);
        parse_sentence(format!("${body}*{c:02X}").as_bytes(), DEFAULT_MAX_FRAME_LEN).unwrap()
    }

    #[test]
    fn heading_from_a_magnetic_compass_follows_it() {
        let mut o = Observation::default();
        o.apply(&sentence("HCHDG,100.0,,,20.0,E"));
        assert_eq!(o.heading, Some(120.0));
        o.apply(&sentence("HCHDG,150.0,,,20.0,E"));
        assert_eq!(o.heading, Some(170.0));
        // variation from the GPS when the compass doesn't give it
        o.apply(&sentence("HCHDM,200.0,M"));
        assert_eq!(o.heading, Some(220.0));
        // a true heading sensor wins from then on
        o.apply(&sentence("HEHDT,90.0,T"));
        o.apply(&sentence("HCHDG,10.0,,,20.0,E"));
        assert_eq!(o.heading, Some(90.0));
        assert_eq!(o.heading_mag, Some(10.0));
        // ...until it goes quiet
        for _ in 0..MAG_TAKES_OVER {
            o.apply(&sentence("HCHDG,10.0,,,20.0,E"));
        }
        assert_eq!(o.heading, Some(30.0));
    }

    #[test]
    fn empty_fields_keep_the_last_value() {
        let mut o = Observation::default();
        o.apply(&sentence("GPRMC,120000,A,3650.0,S,17445.0,E,6.0,90.0,251026,19.5,E"));
        o.apply(&sentence("GPVTG,,T,,M,,N,,K"));
        assert_eq!((o.sog, o.cog), (Some(6.0), Some(90.0)));
        o.apply(&sentence("GPRMC,120001,A,3650.0,S,17445.0,E,6.1,91.0,251026,,"));
        assert_eq!(o.variation, Some(19.5));
        assert_eq!(o.sog, Some(6.1));
        // VHW with only the speed doesn't clear the heading
        o.apply(&sentence("HCHDG,100.0,,,20.0,E"));
        o.apply(&sentence("VWVHW,,T,,M,5.2,N,,K"));
        assert_eq!((o.heading, o.stw), (Some(120.0), Some(5.2)));
    }

    #[test]
    fn mda_and_xdr_weather() {
        let mut o = Observation::default();
        let s = parse_sentence(frame("WIMDA,29.9213,I,1.0132,B,18.5,C,,,62.0,,,,,,,,,,,").trim().as_bytes(), 200).unwrap();
        o.apply(&s);
        assert!((o.pressure.unwrap() - 1013.2).abs() < 0.01);
        assert_eq!(o.air_temp, Some(18.5));
        assert_eq!(o.humidity, Some(62.0));
        // inches only
        let w = weather("MDA", &"30.00,I,,,,,,,,,,,,,,,,,,".split(',').map(str::to_owned).collect::<Vec<_>>());
        assert!((w.pressure.unwrap() - 1015.9).abs() < 0.1);
        // XDR: barometer in bars and pascals, air temperature, humidity; oil pressure ignored
        let x = |t: &str| weather("XDR", &t.split(',').map(str::to_owned).collect::<Vec<_>>());
        assert!((x("P,1.0081,B,Barometer").pressure.unwrap() - 1008.1).abs() < 0.01);
        assert!((x("P,101500,P,BARO").pressure.unwrap() - 1015.0).abs() < 0.01);
        assert_eq!(x("P,3.2,B,OIL").pressure, None);
        let w = x("C,21.4,C,ENV_OUTSIDE_T,H,55,P,ENV_OUTSIDE_H,C,80,C,ENGINE");
        assert_eq!(w.air_temp, Some(21.4));
        assert_eq!(w.humidity, Some(55.0));
        assert_eq!(x("C,80,C,ENGINE").air_temp, None);
    }

    #[test]
    fn decodes_the_connectivity_batch() {
        let mut o = Observation::default();
        // rate of turn: - to port; V (not valid) ignored
        o.apply(&sentence("TIROT,-12.5,A"));
        assert_eq!(o.rot, Some(-12.5));
        o.apply(&sentence("TIROT,30.0,V"));
        assert_eq!(o.rot, Some(-12.5));
        // rudder
        o.apply(&sentence("IIRSA,-4.5,A,,V"));
        assert_eq!(o.rudder, Some(-4.5));
        // set and drift
        o.apply(&sentence("IIVDR,045.0,T,025.0,M,1.3,N"));
        assert_eq!((o.set, o.drift), (Some(45.0), Some(1.3)));
        // log and trip
        o.apply(&sentence("IIVLW,12345.6,N,12.3,N"));
        assert_eq!((o.log, o.trip), (Some(12345.6), Some(12.3)));
        // the older wind sentences: port is negative
        o.apply(&sentence("IIVWR,32.0,L,15.2,N,7.8,M,28.2,K"));
        assert_eq!((o.awa, o.aws), (Some(-32.0), Some(15.2)));
        o.apply(&sentence("IIVWT,45.0,R,,N,6.0,M,,K"));
        assert_eq!(o.twa, Some(45.0));
        assert!((o.tws.unwrap() - 11.663).abs() < 0.01);
        // position quality
        o.apply(&sentence("GPGGA,092750.000,5321.6802,N,00630.3372,W,1,08,1.03,61.7,M,55.2,M,,"));
        assert_eq!((o.sats, o.hdop), (Some(8.0), Some(1.03)));
        o.apply(&sentence("GPGSA,A,3,04,05,,09,12,,,24,,,,,2.5,1.3,2.1"));
        assert_eq!((o.fix_dim, o.hdop), (Some(3.0), Some(1.3)));
        o.apply(&sentence("GPGST,172814.0,0.006,0.023,0.020,273.6,3.0,4.0,0.033"));
        assert_eq!(o.pos_error, Some(5.0));
        // XDR: heel and pitch (Airmar and B&G names), battery, engine
        o.apply(&sentence("YXXDR,A,-1.2,D,PTCH,A,8.5,D,ROLL"));
        assert_eq!((o.pitch, o.heel), (Some(-1.2), Some(8.5)));
        o.apply(&sentence("IIXDR,A,-12.0,D,Heel,U,12.7,V,BATTERY#0,T,1850,R,ENGINE#0"));
        assert_eq!((o.heel, o.battery_v, o.rpm), (Some(-12.0), Some(12.7), Some(1850.0)));
        // the weather in an XDR still read, and an XDR without these leaves them alone
        o.apply(&sentence("WIXDR,P,1.0132,B,Barometer"));
        assert_eq!(o.pressure, Some(1013.2));
        assert_eq!(o.heel, Some(-12.0));
        // engine revolutions (E), not the shaft (S)
        o.apply(&sentence("IIRPM,S,1,400,,A"));
        assert_eq!(o.rpm, Some(1850.0));
        o.apply(&sentence("IIRPM,E,1,2100,,A"));
        assert_eq!(o.rpm, Some(2100.0));
    }

    #[test]
    fn decodes_ths_vbw_dtm() {
        let mut o = Observation::default();
        o.apply(&sentence("GNTHS,123.4,A"));
        assert_eq!(o.heading, Some(123.4));
        // not valid: kept as it was
        o.apply(&sentence("GNTHS,200.0,V"));
        assert_eq!(o.heading, Some(123.4));
        // a magnetic heading doesn't replace the satellite compass's true heading
        o.apply(&sentence("HCHDG,100.0,,,20.0,E"));
        assert_eq!(o.heading, Some(123.4));
        o.apply(&sentence("VMVBW,6.4,0.2,A,6.9,0.3,A,,V,,V"));
        assert_eq!(o.stw, Some(6.4));
        o.apply(&sentence("VMVBW,9.9,0.2,V,6.9,0.3,A,,V,,V"));
        assert_eq!(o.stw, Some(6.4));
        o.apply(&sentence("GPDTM,W84,,0.0,N,0.0,E,0.0,W84"));
        assert_eq!(o.datum.as_deref(), Some("W84"));
        o.apply(&sentence("GPDTM,999,,0.08,N,0.07,E,-47.7,W84"));
        assert_eq!(o.datum.as_deref(), Some("999"));
    }
}
