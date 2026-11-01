//! Signal K client.
//!
//! Connects to a Signal K server's WebSocket stream, keeps the own vessel's
//! state up to date from the deltas, and optionally writes every message to a
//! log file (one JSON message per line, prefixed with the receive time) so a
//! session can be replayed later.
//!
//! Signal K uses SI units (radians, m/s, metres). [`VesselState`] converts to
//! the project conventions: degrees true, knots, metres.

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
    /// Total log and trip, NM.
    pub log: Option<f64>,
    pub trip: Option<f64>,
    /// Water and air temperature (°C), barometric pressure (hPa).
    pub water_temp: Option<f64>,
    pub air_temp: Option<f64>,
    pub pressure: Option<f64>,
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
        if self.heading.is_none() {
            if let (Some(m), Some(v)) = (self.heading_mag, self.variation) {
                self.heading = Some(nav_core::norm360(m + v));
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
            if self.true_wind_calc || self.twd.is_none() {
                self.twd = Some(nav_core::norm360(h + twa));
            }
        }
    }

    pub fn position(&self) -> Option<LatLon> {
        Some(LatLon::new(self.lat?, self.lon?))
    }

    /// Apply one Signal K delta message. Returns true if anything changed.
    pub fn apply_delta(&mut self, msg: &Value) -> bool {
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
                self.apply_value(path, val);
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

    fn apply_value(&mut self, path: &str, val: &Value) {
        let num = || val.as_f64();
        match path {
            "navigation.position" => {
                if let (Some(lat), Some(lon)) = (val.get("latitude").and_then(Value::as_f64), val.get("longitude").and_then(Value::as_f64)) {
                    self.lat = Some(lat);
                    self.lon = Some(lon);
                }
            }
            "navigation.courseOverGroundTrue" => self.cog = num().map(|r| nav_core::norm360(r.to_degrees())),
            "navigation.speedOverGround" => self.sog = num().map(|ms| ms * MS_TO_KN),
            "navigation.headingTrue" => self.heading = num().map(|r| nav_core::norm360(r.to_degrees())),
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
            "environment.wind.directionTrue" => self.twd = num().map(|r| nav_core::norm360(r.to_degrees())),
            "environment.water.temperature" => self.water_temp = num().map(|k| k - 273.15),
            "environment.outside.temperature" => self.air_temp = num().map(|k| k - 273.15),
            "environment.outside.pressure" => self.pressure = num().map(|pa| pa / 100.0),
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
        if let Some(s) = msg.get("self").and_then(Value::as_str) {
            self.self_ctx = Some(s.to_string());
        }
        let ctx = msg.get("context").and_then(Value::as_str);
        if self.is_self(ctx) {
            return (state.apply_delta(msg), Vec::new());
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
    let mut log = match &opts.log_dir {
        Some(dir) => open_log(dir).await,
        None => None,
    };

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
                    if let Some(f) = log.as_mut() {
                        let line = format!("{}\t{}\n", chrono::Utc::now().to_rfc3339(), text.as_str());
                        let _ = f.write_all(line.as_bytes()).await;
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

async fn open_log(dir: &PathBuf) -> Option<tokio::io::BufWriter<tokio::fs::File>> {
    tokio::fs::create_dir_all(dir).await.ok()?;
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
        // same delta again changes nothing
        assert!(!s.apply_delta(&d));
    }

    #[test]
    fn ignores_hello_and_unknown_paths() {
        let mut s = VesselState::default();
        assert!(!s.apply_delta(&json!({"name": "signalk-server", "version": "2.0", "self": "vessels.urn"})));
        let d = json!({"updates": [{"values": [{"path": "navigation.racing.startLineStb", "value": 0.3}]}]});
        assert!(!s.apply_delta(&d));
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
