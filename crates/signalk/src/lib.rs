//! Signal K client.
//!
//! Connects to a Signal K server's WebSocket stream, keeps the own vessel's
//! state up to date from the deltas, and optionally writes every message to a
//! log file (one JSON message per line, prefixed with the receive time) so a
//! session can be replayed later.
//!
//! Signal K uses SI units (radians, m/s, metres). [`VesselState`] converts to
//! the project conventions: degrees true, knots, metres.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use nav_core::LatLon;
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

/// Everything the server has: the own vessel and the AIS targets it hears.
pub const DEFAULT_URL: &str = "ws://localhost:3000/signalk/v1/stream?subscribe=all";

const MS_TO_KN: f64 = 3600.0 / 1852.0;

/// Own-vessel state built from Signal K deltas.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct VesselState {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Changes only when a valid position observation is received.
    pub position_observation: u64,
    /// Local receive time of the fix, or log time during replay.
    pub position_received_at_ms: Option<i64>,
    #[serde(skip)]
    pub position_received_at_instant: Option<std::time::Instant>,
    /// Log time represented by this snapshot; absent for live data.
    pub replay_time_ms: Option<i64>,
    /// Age of the selected position at publication, when available.
    pub position_age_ms: Option<u64>,
    /// Age and receive time for each depth measurement independently.
    pub depth_age_ms: BTreeMap<String, u64>,
    pub depth_received_at_ms: BTreeMap<String, i64>,
    #[serde(skip)]
    pub depth_received_at_instant: BTreeMap<String, std::time::Instant>,
    /// Course over ground, degrees true.
    pub cog: Option<f64>,
    /// Speed over ground, knots.
    pub sog: Option<f64>,
    /// Heading, degrees true.
    pub heading: Option<f64>,
    /// Heading, degrees magnetic, and magnetic variation (east +), degrees.
    pub heading_mag: Option<f64>,
    pub variation: Option<f64>,
    /// Speed through the water (boat speed), knots.
    pub stw: Option<f64>,
    /// Depth below transducer, metres.
    pub depth: Option<f64>,
    /// Depth below keel and below the surface, metres, when the server has them.
    pub depth_keel: Option<f64>,
    pub depth_surface: Option<f64>,
    /// Apparent wind speed (kn) and angle (degrees, + = starboard, -180..180).
    pub aws: Option<f64>,
    pub awa: Option<f64>,
    /// True wind speed (kn), angle (degrees, + = starboard) and direction (degrees true, from).
    pub tws: Option<f64>,
    pub twa: Option<f64>,
    pub twd: Option<f64>,
    /// True wind was calculated here from apparent wind, boat speed and heading.
    pub true_wind_calc: bool,
    /// True heading and wind direction were worked out here (so they are worked out again
    /// when their inputs change), not sent by the server.
    #[serde(skip)]
    heading_calc: bool,
    #[serde(skip)]
    twd_calc: bool,
    /// Total log and trip, NM.
    pub log: Option<f64>,
    pub trip: Option<f64>,
    /// Water and air temperature (°C), barometric pressure (hPa).
    pub water_temp: Option<f64>,
    pub air_temp: Option<f64>,
    pub pressure: Option<f64>,
    /// Relative humidity, %.
    pub humidity: Option<f64>,
    /// GPS date and time (`navigation.datetime`, ISO 8601 UTC), when the source sends it.
    pub datetime: Option<String>,
    /// Time stamp of the last update. Sentences without a date (GGA, GLL)
    /// get today's date from the server, so prefer `datetime` for display.
    pub timestamp: Option<String>,
}

impl VesselState {
    /// Fill in what can be worked out from what arrived: true heading from
    /// magnetic + variation, true wind from apparent wind, boat speed and heading.
    fn derive(&mut self) {
        if self.heading.is_none() || self.heading_calc {
            if let (Some(m), Some(v)) = (self.heading_mag, self.variation) {
                self.heading = Some(nav_core::norm360(m + v));
                self.heading_calc = true;
            }
        }
        let server_tws = self.tws.is_some() && !self.true_wind_calc;
        if !server_tws {
            if let (Some(aws), Some(awa), Some(bsp)) = (self.aws, self.awa, self.stw.or(self.sog)) {
                // wind relative to the boat, x along the bow, y to starboard (direction the wind comes FROM)
                let (ax, ay) = (aws * awa.to_radians().cos(), aws * awa.to_radians().sin());
                let (tx, ty) = (ax - bsp, ay);
                self.tws = Some((tx * tx + ty * ty).sqrt());
                self.twa = Some(ty.atan2(tx).to_degrees());
                self.true_wind_calc = true;
            }
        }
        if let (Some(twa), Some(h)) = (self.twa, self.heading) {
            if self.true_wind_calc || self.twd_calc || self.twd.is_none() {
                self.twd = Some(nav_core::norm360(h + twa));
                self.twd_calc = true;
            }
        }
    }

    pub fn position(&self) -> Option<LatLon> {
        Some(LatLon::new(self.lat?, self.lon?))
    }

    /// Apply one Signal K delta message. Returns true if anything changed.
    pub fn apply_delta(&mut self, msg: &Value) -> bool {
        self.apply_delta_at(msg, chrono::Utc::now().timestamp_millis())
    }

    fn apply_delta_at(&mut self, msg: &Value, received_at_ms: i64) -> bool {
        let Some(updates) = msg.get("updates").and_then(Value::as_array) else {
            return false;
        };
        let before = self.clone();
        for u in updates {
            let ts = u.get("timestamp").and_then(Value::as_str);
            let Some(values) = u.get("values").and_then(Value::as_array) else {
                continue;
            };
            for v in values {
                let (Some(path), Some(val)) = (v.get("path").and_then(Value::as_str), v.get("value")) else {
                    continue;
                };
                self.apply_value(path, val, received_at_ms);
            }
            if let Some(ts) = ts {
                self.timestamp = Some(ts.to_string());
            }
        }
        if *self != before {
            self.derive();
        }
        *self != before
    }

    fn apply_value(&mut self, path: &str, val: &Value, received_at_ms: i64) {
        let num = || val.as_f64();
        let depth_field = match path {
            "environment.depth.belowTransducer" => Some("depth"),
            "environment.depth.belowKeel" => Some("depth_keel"),
            "environment.depth.belowSurface" => Some("depth_surface"),
            _ => None,
        };
        if let Some(field) = depth_field {
            if num().is_some_and(f64::is_finite) {
                self.depth_received_at_ms.insert(field.to_owned(), received_at_ms);
                self.depth_received_at_instant.insert(field.to_owned(), std::time::Instant::now());
            } else {
                self.depth_received_at_ms.remove(field);
                self.depth_received_at_instant.remove(field);
            }
        }
        match path {
            "navigation.position" => {
                if let (Some(lat), Some(lon)) = (val.get("latitude").and_then(Value::as_f64), val.get("longitude").and_then(Value::as_f64)) {
                    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
                        self.lat = Some(lat);
                        self.lon = Some(lon);
                        self.position_observation = self.position_observation.wrapping_add(1);
                        self.position_received_at_ms = Some(received_at_ms);
                        self.position_received_at_instant = Some(std::time::Instant::now());
                    }
                }
            }
            "navigation.courseOverGroundTrue" => self.cog = num().map(|r| nav_core::norm360(r.to_degrees())),
            "navigation.speedOverGround" => self.sog = num().map(|ms| ms * MS_TO_KN),
            "navigation.headingTrue" => {
                self.heading = num().map(|r| nav_core::norm360(r.to_degrees()));
                self.heading_calc = false;
            }
            "navigation.headingMagnetic" => self.heading_mag = num().map(|r| nav_core::norm360(r.to_degrees())),
            "navigation.magneticVariation" => self.variation = num().map(f64::to_degrees),
            "navigation.speedThroughWater" => self.stw = num().map(|ms| ms * MS_TO_KN),
            "navigation.log" => self.log = num().map(|m| m / 1852.0),
            "navigation.trip.log" => self.trip = num().map(|m| m / 1852.0),
            "environment.depth.belowTransducer" => self.depth = num(),
            "environment.depth.belowKeel" => self.depth_keel = num(),
            "environment.depth.belowSurface" => self.depth_surface = num(),
            "environment.wind.speedApparent" => self.aws = num().map(|ms| ms * MS_TO_KN),
            "environment.wind.angleApparent" => self.awa = num().map(|r| nav_core::norm180(r.to_degrees())),
            "environment.wind.speedTrue" => {
                self.tws = num().map(|ms| ms * MS_TO_KN);
                self.true_wind_calc = false;
            }
            "environment.wind.angleTrueWater" => self.twa = num().map(|r| nav_core::norm180(r.to_degrees())),
            "environment.wind.directionTrue" => {
                self.twd = num().map(|r| nav_core::norm360(r.to_degrees()));
                self.twd_calc = false;
            }
            "environment.water.temperature" => self.water_temp = num().map(|k| k - 273.15),
            "environment.outside.temperature" => self.air_temp = num().map(|k| k - 273.15),
            "environment.outside.pressure" => self.pressure = num().map(|pa| pa / 100.0),
            "environment.outside.relativeHumidity" | "environment.outside.humidity" => {
                // a ratio in Signal K (0–1); some servers send percent
                self.humidity = num().map(|h| if h <= 1.0 { h * 100.0 } else { h });
            }
            "navigation.datetime" => self.datetime = val.as_str().map(str::to_string),
            _ => {}
        }
    }
}

/// What the client reports to the app.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// Connection state changed.
    Status { connected: bool, detail: String },
    /// Own-vessel state changed.
    Vessel(VesselState),
    /// Values for another context (an AIS target: `vessels.urn:mrn:imo:mmsi:…`, `atons.…`, `aircraft.…`).
    Other { context: String, values: Vec<(String, Value)> },
}

/// Splits Signal K messages into the own vessel's values and other contexts'.
#[derive(Debug, Default, Clone)]
pub struct Router {
    /// The own vessel's context, from the server's hello (`self`).
    pub self_ctx: Option<String>,
}

impl Router {
    fn is_self(&self, ctx: Option<&str>) -> bool {
        match ctx {
            None | Some("vessels.self") => true,
            Some(c) => self.self_ctx.as_deref() == Some(c) || self.self_ctx.as_deref().is_some_and(|s| s.strip_prefix("vessels.") == Some(c)),
        }
    }

    /// Route one message: apply own-vessel values to `state` (returns whether it
    /// changed) and return the other contexts' values.
    pub fn route(&mut self, state: &mut VesselState, msg: &Value) -> (bool, Vec<Event>) {
        self.route_at(state, msg, chrono::Utc::now().timestamp_millis())
    }

    fn route_at(&mut self, state: &mut VesselState, msg: &Value, received_at_ms: i64) -> (bool, Vec<Event>) {
        if let Some(s) = msg.get("self").and_then(Value::as_str) {
            self.self_ctx = Some(s.to_string());
        }
        let ctx = msg.get("context").and_then(Value::as_str);
        if self.is_self(ctx) {
            return (state.apply_delta_at(msg, received_at_ms), Vec::new());
        }
        let Some(updates) = msg.get("updates").and_then(Value::as_array) else { return (false, Vec::new()) };
        let mut values = Vec::new();
        for u in updates {
            for v in u.get("values").and_then(Value::as_array).into_iter().flatten() {
                if let (Some(p), Some(val)) = (v.get("path").and_then(Value::as_str), v.get("value")) {
                    values.push((p.to_string(), val.clone()));
                }
            }
        }
        if values.is_empty() {
            return (false, Vec::new());
        }
        (false, vec![Event::Other { context: ctx.unwrap_or_default().to_string(), values }])
    }
}

/// Client options.
#[derive(Debug, Clone)]
pub struct Options {
    pub url: String,
    /// Folder for session logs; `None` disables logging.
    pub log_dir: Option<PathBuf>,
    /// Smallest gap between vessel events sent to the app.
    pub min_interval: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self { url: DEFAULT_URL.to_string(), log_dir: None, min_interval: Duration::from_millis(250) }
    }
}

/// Run the client for ever: connect, read, reconnect after a drop.
/// Events go to `tx`; the function returns only when `tx` is closed.
pub async fn run(opts: Options, tx: mpsc::Sender<Event>) {
    let mut state = VesselState::default();
    let mut router = Router::default();
    let mut backoff = Duration::from_secs(1);
    // the session log is opened when the first message arrives (not at start, so a PC with no
    // Signal K server leaves no empty files) and stops at a size cap (a busy server sends
    // hundreds of deltas a second, AIS targets included)
    let mut log: Option<tokio::io::BufWriter<tokio::fs::File>> = None;
    let mut log_tried = opts.log_dir.is_none();
    let mut log_bytes: u64 = 0;

    loop {
        match tokio_tungstenite::connect_async(opts.url.as_str()).await {
            Ok((mut ws, _)) => {
                backoff = Duration::from_secs(1);
                if send(&tx, Event::Status { connected: true, detail: opts.url.clone() }).await {
                    return;
                }
                let mut last_sent = tokio::time::Instant::now() - opts.min_interval;
                let mut pending = false;
                loop {
                    let wait = opts.min_interval.saturating_sub(last_sent.elapsed());
                    let msg = tokio::select! {
                        m = ws.next() => m,
                        _ = tokio::time::sleep(wait), if pending => {
                            pending = false;
                            last_sent = tokio::time::Instant::now();
                            if send(&tx, Event::Vessel(state.clone())).await { return; }
                            continue;
                        }
                    };
                    let text = match msg {
                        Some(Ok(m)) if m.is_text() => m.into_text().unwrap_or_default(),
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => {
                            let _ = send(&tx, Event::Status { connected: false, detail: e.to_string() }).await;
                            break;
                        }
                        None => {
                            let _ = send(&tx, Event::Status { connected: false, detail: "server closed the stream".into() }).await;
                            break;
                        }
                    };
                    if !log_tried {
                        log_tried = true;
                        if let Some(dir) = &opts.log_dir {
                            log = open_log(dir).await;
                        }
                    }
                    if let Some(f) = log.as_mut() {
                        let line = format!("{}\t{}\n", chrono::Utc::now().to_rfc3339(), text.as_str());
                        log_bytes += line.len() as u64;
                        let _ = f.write_all(line.as_bytes()).await;
                        if log_bytes > LOG_MAX_BYTES {
                            let _ = f.flush().await;
                            log = None;
                        }
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(text.as_str()) {
                        let (own, others) = router.route(&mut state, &v);
                        for ev in others {
                            if send(&tx, ev).await {
                                return;
                            }
                        }
                        if own {
                            if last_sent.elapsed() >= opts.min_interval {
                                last_sent = tokio::time::Instant::now();
                                pending = false;
                                if send(&tx, Event::Vessel(state.clone())).await {
                                    return;
                                }
                            } else {
                                pending = true;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                if send(&tx, Event::Status { connected: false, detail: format!("{}: {e}", opts.url) }).await {
                    return;
                }
            }
        }
        if let Some(f) = log.as_mut() {
            let _ = f.flush().await;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

/// Returns true if the receiver has gone (time to stop).
async fn send(tx: &mpsc::Sender<Event>, ev: Event) -> bool {
    tx.send(ev).await.is_err()
}

pub mod replay;

/// One session's log stops here (the next start begins a new one).
const LOG_MAX_BYTES: u64 = 300_000_000;
/// Delete the empty session logs (`sk-….log`) that earlier starts left when no server was there.
async fn prune_logs(dir: &PathBuf) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("sk-") && name.ends_with(".log") && entry.metadata().await.is_ok_and(|m| m.len() == 0) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

async fn open_log(dir: &PathBuf) -> Option<tokio::io::BufWriter<tokio::fs::File>> {
    tokio::fs::create_dir_all(dir).await.ok()?;
    prune_logs(dir).await;
    let name = chrono::Utc::now().format("sk-%Y%m%d-%H%M%SZ.log").to_string();
    let f = tokio::fs::File::create(dir.join(name)).await.ok()?;
    Some(tokio::io::BufWriter::new(f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applies_position_cog_sog() {
        let mut s = VesselState::default();
        let d = json!({
            "context": "vessels.self",
            "updates": [{
                "timestamp": "2015-03-06T09:06:04.000Z",
                "values": [
                    {"path": "navigation.position", "value": {"latitude": -51.30791, "longitude": -74.14601}},
                    {"path": "navigation.courseOverGroundTrue", "value": std::f64::consts::PI},
                    {"path": "navigation.speedOverGround", "value": 2.5722}
                ]
            }]
        });
        assert!(s.apply_delta(&d));
        assert_eq!(s.lat, Some(-51.30791));
        assert!((s.cog.unwrap() - 180.0).abs() < 1e-9);
        assert!((s.sog.unwrap() - 5.0).abs() < 1e-3);
        assert_eq!(s.timestamp.as_deref(), Some("2015-03-06T09:06:04.000Z"));
        // The same coordinates can be a new valid fix.
        assert!(s.apply_delta(&d));
        assert_eq!(s.position_observation, 2);
    }

    #[test]
    fn position_observation_changes_only_for_valid_position_updates() {
        let mut s = VesselState::default();
        let delta = |path: &str, value: Value| json!({"updates": [{"values": [{"path": path, "value": value}]}]});
        assert!(s.apply_delta(&delta("navigation.position", json!({"latitude": -41.0, "longitude": 173.0}))));
        assert_eq!(s.position_observation, 1);
        assert!(s.apply_delta(&delta("environment.depth.belowTransducer", json!(5.0))));
        assert_eq!(s.position_observation, 1);
        assert!(!s.apply_delta(&delta("navigation.position", json!({"latitude": 91.0, "longitude": 173.0}))));
        assert!(!s.apply_delta(&delta("navigation.position", Value::Null)));
        assert_eq!(s.position_observation, 1);
        assert_eq!((s.lat, s.lon), (Some(-41.0), Some(173.0)));
        assert!(s.apply_delta(&delta("navigation.position", json!({"latitude": -41.0, "longitude": 173.0}))));
        assert_eq!(s.position_observation, 2);
    }

    #[test]
    fn depth_receive_times_follow_each_path_even_for_unchanged_values() {
        let mut s = VesselState::default();
        let delta = |path: &str, value: Value| json!({"updates": [{"values": [{"path": path, "value": value}]}]});
        s.apply_delta_at(&delta("environment.depth.belowKeel", json!(3.0)), 1000);
        s.apply_delta_at(&delta("environment.depth.belowTransducer", json!(5.0)), 2000);
        s.apply_delta_at(&delta("environment.wind.speedApparent", json!(5.0)), 3000);
        assert_eq!(s.depth_received_at_ms["depth_keel"], 1000);
        assert_eq!(s.depth_received_at_ms["depth"], 2000);
        assert!(s.apply_delta_at(&delta("environment.depth.belowTransducer", json!(5.0)), 4000));
        assert_eq!(s.depth_received_at_ms["depth"], 4000);
        s.apply_delta_at(&delta("environment.depth.belowKeel", Value::Null), 5000);
        assert!(!s.depth_received_at_ms.contains_key("depth_keel"));
        assert_eq!(s.depth_keel, None);
    }

    #[test]
    fn ignores_hello_and_unknown_paths() {
        let mut s = VesselState::default();
        assert!(!s.apply_delta(&json!({"name": "signalk-server", "version": "2.0", "self": "vessels.urn"})));
        let d = json!({"updates": [{"values": [{"path": "navigation.racing.startLineStb", "value": 0.3}]}]});
        assert!(!s.apply_delta(&d));
    }

    #[test]
    fn worked_out_heading_and_wind_direction_follow_their_inputs() {
        let mut s = VesselState::default();
        let delta = |path: &str, value: Value| json!({"updates": [{"values": [{"path": path, "value": value}]}]});
        s.apply_delta(&delta("navigation.magneticVariation", json!(20f64.to_radians())));
        s.apply_delta(&delta("navigation.headingMagnetic", json!(10f64.to_radians())));
        assert!((s.heading.unwrap() - 30.0).abs() < 1e-6);
        // the magnetic heading moves: the true heading moves with it, not frozen at the first value
        s.apply_delta(&delta("navigation.headingMagnetic", json!(50f64.to_radians())));
        assert!((s.heading.unwrap() - 70.0).abs() < 1e-6, "{:?}", s.heading);
        // a true angle from the server, no direction: the direction follows heading changes
        s.apply_delta(&delta("environment.wind.angleTrueWater", json!(40f64.to_radians())));
        assert!((s.twd.unwrap() - 110.0).abs() < 1e-6);
        s.apply_delta(&delta("navigation.headingMagnetic", json!(60f64.to_radians())));
        assert!((s.twd.unwrap() - 120.0).abs() < 1e-6, "{:?}", s.twd);
        // a heading or direction from the server wins from then on
        s.apply_delta(&delta("navigation.headingTrue", json!(100f64.to_radians())));
        s.apply_delta(&delta("navigation.headingMagnetic", json!(5f64.to_radians())));
        assert!((s.heading.unwrap() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn works_out_true_wind_from_apparent() {
        let mut s = VesselState::default();
        // 6 kn boat speed, 15 kn apparent at 40° to starboard, heading 000
        let d = json!({"updates": [{"values": [
            {"path": "navigation.headingTrue", "value": 0.0},
            {"path": "navigation.speedThroughWater", "value": 6.0 / MS_TO_KN},
            {"path": "environment.wind.speedApparent", "value": 15.0 / MS_TO_KN},
            {"path": "environment.wind.angleApparent", "value": 40f64.to_radians()}
        ]}]});
        assert!(s.apply_delta(&d));
        assert!(s.true_wind_calc);
        assert!((s.tws.unwrap() - 11.09).abs() < 0.05, "{:?}", s.tws);
        assert!((s.twa.unwrap() - 60.3).abs() < 0.3, "{:?}", s.twa);
        assert!((s.twd.unwrap() - 60.3).abs() < 0.3);
        // the server's own true wind wins
        let d = json!({"updates": [{"values": [{"path": "environment.wind.speedTrue", "value": 5.0}]}]});
        s.apply_delta(&d);
        assert!(!s.true_wind_calc);
    }
}
