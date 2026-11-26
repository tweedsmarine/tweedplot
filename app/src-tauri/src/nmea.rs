use marine_nmea::fixstreams::{FixStreams, StreamSummary, Verdict};
use marine_nmea::{Framer, Observation, Sentence};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SourceTransport {
    Serial {
        port: String,
        baud_rate: u32,
    },
    Tcp {
        address: String,
    },
    /// `send_to`: where sentences TweedPlot sends go (the autopilot output, acknowledgements), host:port;
    /// a broadcast address (192.168.1.255:10110) reaches every listener on the network.
    Udp {
        bind_address: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        send_to: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Retained for backwards compatibility with pre-arbitration settings.
    #[serde(default)]
    pub use_for_navigation: bool,
    pub transport: SourceTransport,
    /// With two GPS receivers mixed on this input: the fix stream to use (its key, as
    /// "GP-5-2"); None picks the best one automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gps_stream: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataGroup {
    Position,
    Motion,
    Heading,
    SpeedThroughWater,
    Wind,
    Depth,
    WaterTemperature,
    Weather,
    RateOfTurn,
    Attitude,
    Rudder,
    Current,
    Log,
    GnssQuality,
    Engine,
}

impl DataGroup {
    pub const ALL: [Self; 15] = [
        Self::Position,
        Self::Motion,
        Self::Heading,
        Self::SpeedThroughWater,
        Self::Wind,
        Self::Depth,
        Self::WaterTemperature,
        Self::Weather,
        Self::RateOfTurn,
        Self::Attitude,
        Self::Rudder,
        Self::Current,
        Self::Log,
        Self::GnssQuality,
        Self::Engine,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Motion => "motion",
            Self::Heading => "heading",
            Self::SpeedThroughWater => "speedThroughWater",
            Self::Wind => "wind",
            Self::Depth => "depth",
            Self::WaterTemperature => "waterTemperature",
            Self::Weather => "weather",
            Self::RateOfTurn => "rateOfTurn",
            Self::Attitude => "attitude",
            Self::Rudder => "rudder",
            Self::Current => "current",
            Self::Log => "log",
            Self::GnssQuality => "gnssQuality",
            Self::Engine => "engine",
        }
    }

    pub fn is_valid_key(key: &str) -> bool {
        Self::ALL.iter().any(|group| group.key() == key)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Position => "Position",
            Self::Motion => "COG / SOG",
            Self::Heading => "Heading",
            Self::SpeedThroughWater => "Speed through water",
            Self::Wind => "Wind",
            Self::Depth => "Depth",
            Self::WaterTemperature => "Water temperature",
            Self::Weather => "Air pressure, temperature and humidity",
            Self::RateOfTurn => "Rate of turn",
            Self::Attitude => "Heel and pitch",
            Self::Rudder => "Rudder angle",
            Self::Current => "Set and drift (measured)",
            Self::Log => "Log and trip",
            Self::GnssQuality => "Position quality",
            Self::Engine => "Engine and battery",
        }
    }
}

fn default_failover_enabled() -> bool {
    true
}

fn default_stale_after_seconds() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArbitrationConfig {
    #[serde(default = "default_failover_enabled")]
    pub failover_enabled: bool,
    #[serde(default = "default_stale_after_seconds")]
    pub stale_after_seconds: u64,
    pub preferred_sources: BTreeMap<String, String>,
}

impl Default for ArbitrationConfig {
    fn default() -> Self {
        Self { failover_enabled: default_failover_enabled(), stale_after_seconds: default_stale_after_seconds(), preferred_sources: BTreeMap::new() }
    }
}

#[derive(Default)]
struct SourceObservation {
    observation: Observation,
    last_seen_by_group: BTreeMap<DataGroup, Instant>,
    depth_seen: BTreeMap<String, Instant>,
    position_observation: u64,
}

#[derive(Default)]
struct Multiplexer {
    sources: BTreeMap<String, SourceObservation>,
    active_sources: BTreeMap<DataGroup, Option<String>>,
    source_configs: Vec<SourceConfig>,
    arbitration: ArbitrationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    #[serde(skip)]
    generation: u64,
    pub id: String,
    pub name: String,
    pub state: String,
    pub detail: String,
    pub valid_sentences: u64,
    pub invalid_sentences: u64,
    pub last_sentence: Option<String>,
    pub last_seen: Option<String>,
    /// When `last_seen` was last worked out: the text is made at most twice a second, not per sentence.
    #[serde(skip)]
    last_seen_at: Option<Instant>,
    /// The GPS receivers told apart on this input (see marine_nmea::fixstreams).
    #[serde(default, skip_deserializing)]
    pub gps_streams: Vec<StreamSummary>,
}

static SOURCES: Mutex<Vec<watch::Sender<bool>>> = Mutex::new(Vec::new());
static STATUSES: Mutex<BTreeMap<String, SourceStatus>> = Mutex::new(BTreeMap::new());
static NAVIGATION_SOURCE: Mutex<Option<String>> = Mutex::new(None);
static MULTIPLEXER: Mutex<Multiplexer> = Mutex::new(Multiplexer {
    sources: BTreeMap::new(),
    active_sources: BTreeMap::new(),
    source_configs: Vec::new(),
    arbitration: ArbitrationConfig { failover_enabled: true, stale_after_seconds: 10, preferred_sources: BTreeMap::new() },
});
/// The position fix streams heard on each input (two GPS receivers mixed on one input).
static FIX_STREAMS: Mutex<BTreeMap<String, FixStreams>> = Mutex::new(BTreeMap::new());
/// The clock fix streams are timed by.
fn clock() -> f64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// New navigation data since the vessel state was last published (at most 10 a second).
static VESSEL_DIRTY: AtomicBool = AtomicBool::new(false);
static POSITION_OBSERVATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// Sources whose status (sentence counts) changed since it was last published.
static STATUS_DIRTY: Mutex<std::collections::BTreeSet<String>> = Mutex::new(std::collections::BTreeSet::new());
/// Sentences heard since the last "nmea-observation" event, per source.
static RECENT: Mutex<BTreeMap<String, (String, MonitorRing)>> = Mutex::new(BTreeMap::new());
/// AIS messages over NMEA are put together and decoded per source.
static AIS_DECODERS: Mutex<BTreeMap<String, ais::nmea::Decoder>> = Mutex::new(BTreeMap::new());
/// Sentences to send out on a connected serial or TCP source (the autopilot output), by source id.
/// A source's connection registers here while it is open and takes itself off when it closes.
static OUTBOUND: Mutex<BTreeMap<String, tokio::sync::mpsc::UnboundedSender<Vec<u8>>>> = Mutex::new(BTreeMap::new());

fn outbound_open(id: &str) -> tokio::sync::mpsc::UnboundedReceiver<Vec<u8>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    OUTBOUND.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_owned(), tx);
    rx
}

fn outbound_close(id: &str) {
    OUTBOUND.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
}

/// Send the autopilot sentences for the leg being navigated out on a connected source: RMB, APB,
/// XTE, BOD and BWC, once for each call (the app calls once a second while the mariner has
/// "Send to the autopilot" on and a mark is navigated). `talker`: "GP" unless another is set.
/// Errors when the source isn't connected (or can't be written to: UDP inputs only listen).
#[tauri::command]
pub fn autopilot_send(source_id: String, talker: Option<String>, nav: marine_nmea::autopilot::AutopilotNav) -> Result<usize, String> {
    let talker = talker.filter(|t| t.len() == 2 && t.bytes().all(|c| c.is_ascii_uppercase())).unwrap_or_else(|| "GP".into());
    let hms = chrono::Utc::now().format("%H%M%S").to_string();
    let lines = marine_nmea::autopilot::sentences(&nav, &talker, &hms);
    let bytes: Vec<u8> = lines.concat().into_bytes();
    let tx = OUTBOUND.lock().unwrap_or_else(|e| e.into_inner()).get(&source_id).cloned();
    match tx {
        Some(tx) => tx.send(bytes).map(|_| lines.len()).map_err(|_| "The connection has just closed".into()),
        None => Err("That connection isn't open (a UDP one sends only with a \"send to\" address)".into()),
    }
}

/// Acknowledge another piece of equipment's alarm on the connection it came in on: an ACK (for an
/// ALR) or ACN (for an ALF) as the equipment's alert gave it ("ACK,012"), talker EC. Only those two
/// sentences are sent this way.
#[tauri::command]
pub fn equipment_ack(source_id: String, body: String) -> Result<String, String> {
    let ok =
        (body.starts_with("ACK,") || body.starts_with("ACN,")) && body.len() <= 60 && body.bytes().all(|c| c.is_ascii_alphanumeric() || b",.-".contains(&c));
    if !ok {
        return Err("Not an acknowledgement TweedPlot sends".into());
    }
    let line = marine_nmea::autopilot::frame(&format!("EC{body}"));
    let tx = OUTBOUND.lock().unwrap_or_else(|e| e.into_inner()).get(&source_id).cloned();
    match tx {
        Some(tx) => tx.send(line.clone().into_bytes()).map(|_| line.trim_end().to_string()).map_err(|_| "The connection has just closed".into()),
        None => Err("That connection can't send (a UDP one needs a \"send to\" address)".into()),
    }
}

/// The last DSC call per MMSI, so a DSE's enhanced position can be added to it.
static DSC_LAST: Mutex<BTreeMap<String, marine_nmea::dsc::DscCall>> = Mutex::new(BTreeMap::new());
/// How often the vessel state goes to the windows when data is coming in.
const PUBLISH_EVERY: Duration = Duration::from_millis(100);
/// How many publish ticks between status and sentence-monitor updates (half a second).
const MONITOR_TICKS: u32 = 5;
/// How many publish ticks between forced publishes (stale data must drop out).
const FORCE_TICKS: u32 = 10;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorFrame {
    /// Kept as a time and only written out as text when the frame is published (the monitor
    /// sends at most a few hundred a second, a busy harbour's AIS can be many times that).
    #[serde(serialize_with = "serialize_rfc3339")]
    received_at: chrono::DateTime<chrono::Utc>,
    raw: String,
    checksum_valid: bool,
    error: Option<String>,
    /// With two GPS receivers on the input: which one sent it ("A", "B" …).
    #[serde(skip_serializing_if = "Option::is_none")]
    gps: Option<String>,
    /// A fix from the receiver not in use (or out of time order), left out.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    ignored: bool,
}

fn serialize_rfc3339<S: serde::Serializer>(at: &chrono::DateTime<chrono::Utc>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&at.to_rfc3339())
}

/// How many sentences the monitor keeps per input between two publishes: navigation sentences
/// and AIS apart, so a harbour full of AIS can't push the GPS and depth sentences off the list.
const MONITOR_KEEP: usize = 100;
const MONITOR_KEEP_AIS: usize = 20;

/// The sentences one input has heard since the monitor was last published.
#[derive(Default)]
struct MonitorRing {
    nav: VecDeque<MonitorFrame>,
    ais: VecDeque<MonitorFrame>,
}

impl MonitorRing {
    fn push(&mut self, frame: MonitorFrame, ais: bool) {
        let (ring, keep) = if ais { (&mut self.ais, MONITOR_KEEP_AIS) } else { (&mut self.nav, MONITOR_KEEP) };
        if ring.len() >= keep {
            ring.pop_front();
        }
        ring.push_back(frame);
    }

    /// Everything kept, oldest first.
    fn into_frames(self) -> Vec<MonitorFrame> {
        let mut frames: Vec<MonitorFrame> = self.nav.into_iter().chain(self.ais).collect();
        frames.sort_by_key(|frame| frame.received_at);
        frames
    }
}

fn record_monitor_frame(source: &SourceConfig, raw: String, checksum_valid: bool, error: Option<String>, gps: Option<String>, ignored: bool, ais: bool) {
    let mut recent = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    let ring = &mut recent.entry(source.id.clone()).or_insert_with(|| (source.name.clone(), MonitorRing::default())).1;
    ring.push(MonitorFrame { received_at: chrono::Utc::now(), raw, checksum_valid, error, gps, ignored }, ais);
}

/// Whether this run of the input is still the current one: after the inputs are restarted a
/// task that is still winding down must not put its data into the new state.
fn is_current(id: &str, generation: u64) -> bool {
    STATUSES.lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|status| status.generation == generation)
}

fn emit_status(app: &tauri::AppHandle, status: SourceStatus) {
    if let Err(error) = app.emit("nmea-status", status) {
        eprintln!("Could not publish NMEA source status: {error}");
    }
}

fn update_status(app: &tauri::AppHandle, id: &str, generation: u64, state: Option<&str>, detail: Option<String>, valid: bool, sentence: Option<&Sentence>) {
    let mut statuses = STATUSES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(status) = statuses.get_mut(id) else {
        return;
    };
    if status.generation != generation {
        return;
    }
    if let Some(state) = state {
        status.state = state.to_owned();
    }
    if let Some(detail) = detail {
        status.detail = detail;
    }
    if valid {
        const RECEIVING: &str = "Receiving valid NMEA data";
        status.valid_sentences += 1;
        if status.state != "receiving" {
            status.state = "receiving".into();
        }
        if status.detail != RECEIVING {
            status.detail = RECEIVING.into();
        }
        // the text of the time is made twice a second at most, not for every sentence
        if status.last_seen_at.is_none_or(|at| at.elapsed() >= Duration::from_millis(500)) {
            status.last_seen = Some(chrono::Utc::now().to_rfc3339());
            status.last_seen_at = Some(Instant::now());
        }
    }
    if let Some(sentence) = sentence {
        status.last_sentence = Some(sentence.raw.clone());
    }
    // a sentence counted: published with the next monitor tick, not one event per sentence
    if state.is_none() && sentence.is_some() {
        drop(statuses);
        STATUS_DIRTY.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_owned());
        return;
    }
    let status = status.clone();
    drop(statuses);
    if let Err(error) = app.emit("nmea-status", status) {
        eprintln!("Could not publish NMEA source status: {error}");
    }
}

/// Publish the source statuses and the sentences heard since the last time.
fn publish_monitor(app: &tauri::AppHandle) {
    let dirty = std::mem::take(&mut *STATUS_DIRTY.lock().unwrap_or_else(|e| e.into_inner()));
    let statuses: Vec<SourceStatus> = {
        let all = STATUSES.lock().unwrap_or_else(|e| e.into_inner());
        dirty.iter().filter_map(|id| all.get(id).cloned()).collect()
    };
    let now = clock();
    for mut status in statuses {
        if let Some(fs) = FIX_STREAMS.lock().unwrap_or_else(|e| e.into_inner()).get(&status.id) {
            status.gps_streams = if fs.count() > 1 { fs.summary(now) } else { Vec::new() };
        }
        emit_status(app, status);
    }
    let recent = std::mem::take(&mut *RECENT.lock().unwrap_or_else(|e| e.into_inner()));
    if recent.is_empty() {
        return;
    }
    let hub = MULTIPLEXER.lock().unwrap_or_else(|e| e.into_inner());
    for (id, (name, ring)) in recent {
        let frames = ring.into_frames();
        let observation = hub.sources.get(&id).map(|s| s.observation.clone()).unwrap_or_default();
        let sentences: Vec<&str> = frames.iter().map(|frame| frame.raw.as_str()).collect();
        if let Err(error) = app.emit(
            "nmea-observation",
            serde_json::json!({
                "sourceId": id,
                "sourceName": name,
                "sentences": sentences,
                "frames": frames,
                "observation": observation,
            }),
        ) {
            eprintln!("Could not publish NMEA observation: {error}");
        }
    }
}

fn invalid_sentence(app: &tauri::AppHandle, source: &SourceConfig, generation: u64, raw: String, error: &marine_nmea::ParseError) {
    let mut statuses = STATUSES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(status) = statuses.get_mut(&source.id) else {
        return;
    };
    if status.generation != generation {
        return;
    }
    status.invalid_sentences += 1;
    status.state = "receiving".into();
    status.detail = error.to_string();
    status.last_sentence = Some(raw.clone());
    drop(statuses);
    record_monitor_frame(source, raw, false, Some(error.to_string()), None, false, false);
    STATUS_DIRTY.lock().unwrap_or_else(|e| e.into_inner()).insert(source.id.clone());
    let _ = app;
}

fn sentence_groups(sentence: &Sentence, observation: &Observation) -> Vec<DataGroup> {
    let fields = &sentence.fields;
    let has_number = |index: usize| fields.get(index).and_then(|value| value.parse::<f64>().ok()).is_some_and(f64::is_finite);
    let valid_position = match sentence.formatter.as_str() {
        "RMC" => fields.get(1).is_some_and(|status| status == "A"),
        "GGA" => fields.get(5).and_then(|quality| quality.parse::<u8>().ok()).is_some_and(|quality| quality > 0),
        "GLL" => fields.get(5).is_some_and(|status| status == "A"),
        _ => false,
    };
    let mut groups = Vec::new();
    let mut position = Observation::default();
    if valid_position {
        position.apply(sentence);
    }
    if valid_position && position.lat.is_some() && position.lon.is_some() {
        groups.push(DataGroup::Position);
    }
    let motion_valid = match sentence.formatter.as_str() {
        "RMC" => fields.get(1).is_some_and(|status| status == "A") && (has_number(6) || has_number(7)),
        "VTG" => has_number(0) || has_number(4) || has_number(6),
        _ => false,
    };
    if motion_valid && (observation.cog.is_some() || observation.sog.is_some()) {
        groups.push(DataGroup::Motion);
    }
    let heading_valid = match sentence.formatter.as_str() {
        "HDT" => has_number(0) && fields.get(1).is_some_and(|unit| unit == "T"),
        "THS" => has_number(0) && fields.get(1).is_some_and(|mode| mode == "A" || mode == "E"),
        "HDM" => has_number(0) && fields.get(1).is_some_and(|unit| unit == "M"),
        "HDG" => has_number(0) || has_number(3),
        "VHW" => (has_number(0) && fields.get(1).is_some_and(|unit| unit == "T")) || (has_number(2) && fields.get(3).is_some_and(|unit| unit == "M")),
        _ => false,
    };
    if heading_valid && (observation.heading.is_some() || observation.heading_mag.is_some() || observation.variation.is_some()) {
        groups.push(DataGroup::Heading);
    }
    if ((sentence.formatter == "VHW"
        && ((has_number(4) && fields.get(5).is_some_and(|unit| unit == "N")) || (has_number(6) && fields.get(7).is_some_and(|unit| unit == "K"))))
        || (sentence.formatter == "VBW" && has_number(0) && fields.get(2).is_some_and(|status| status == "A")))
        && observation.stw.is_some()
    {
        groups.push(DataGroup::SpeedThroughWater);
    }
    let wind_valid = match sentence.formatter.as_str() {
        "MWV" => fields.get(4).is_some_and(|status| status == "A") && has_number(0) && has_number(2),
        "VWR" | "VWT" => has_number(0) && (has_number(2) || has_number(4) || has_number(6)),
        "MWD" => has_number(0) || has_number(4) || has_number(6),
        _ => false,
    };
    if wind_valid
        && (observation.aws.is_some() || observation.awa.is_some() || observation.tws.is_some() || observation.twa.is_some() || observation.twd.is_some())
    {
        groups.push(DataGroup::Wind);
    }
    let depth_valid = match sentence.formatter.as_str() {
        "DPT" => has_number(0),
        "DBT" | "DBK" | "DBS" => has_number(0) || has_number(2),
        _ => false,
    };
    if depth_valid && (observation.depth.is_some() || observation.depth_keel.is_some() || observation.depth_surface.is_some()) {
        groups.push(DataGroup::Depth);
    }
    if sentence.formatter == "MTW" && has_number(0) && fields.get(1).is_some_and(|unit| unit == "C") && observation.water_temp.is_some() {
        groups.push(DataGroup::WaterTemperature);
    }
    if matches!(sentence.formatter.as_str(), "MDA" | "XDR") && marine_nmea::weather(&sentence.formatter, fields).any() {
        groups.push(DataGroup::Weather);
    }
    // the readings added with ROT, XDR angles, RSA, VDR, VLW, GSA/GST and RPM: a group counts as
    // heard from a source when this sentence gave it a value
    let mut this = Observation::default();
    this.apply(sentence);
    let extra = [
        (DataGroup::RateOfTurn, this.rot.is_some()),
        (DataGroup::Attitude, this.heel.is_some() || this.pitch.is_some()),
        (DataGroup::Rudder, this.rudder.is_some()),
        (DataGroup::Current, this.set.is_some() || this.drift.is_some()),
        (DataGroup::Log, this.log.is_some() || this.trip.is_some()),
        (DataGroup::GnssQuality, this.sats.is_some() || this.hdop.is_some() || this.fix_dim.is_some() || this.pos_error.is_some() || this.datum.is_some()),
        (DataGroup::Engine, this.battery_v.is_some() || this.rpm.is_some()),
    ];
    groups.extend(extra.into_iter().filter(|(_, heard)| *heard).map(|(g, _)| g));
    groups
}

fn update_depth_seen(source: &mut SourceObservation, sentence: &Sentence, at: Instant) {
    let mut current = Observation::default();
    current.apply(sentence);
    for (field, value) in [("depth", current.depth), ("depth_keel", current.depth_keel), ("depth_surface", current.depth_surface)] {
        if value.is_some_and(f64::is_finite) {
            source.depth_seen.insert(field.to_owned(), at);
        }
    }
}

fn selected_sources(hub: &mut Multiplexer, now: Instant) -> BTreeMap<DataGroup, Option<String>> {
    let mut selected = BTreeMap::new();
    let stale_after = Duration::from_secs(hub.arbitration.stale_after_seconds);
    for group in DataGroup::ALL {
        let preferred = hub.arbitration.preferred_sources.get(group.key()).filter(|id| !id.is_empty());
        let is_fresh =
            |id: &str| hub.sources.get(id).and_then(|source| source.last_seen_by_group.get(&group)).is_some_and(|at| now.duration_since(*at) <= stale_after);
        let previous = hub.active_sources.get(&group).cloned().flatten().filter(|id| is_fresh(id));
        let fallback = || {
            hub.source_configs
                .iter()
                .filter(|source| source.enabled)
                .find(|source| preferred.is_none_or(|id| source.id != *id) && is_fresh(&source.id))
                .map(|source| source.id.clone())
        };
        let chosen = match preferred {
            Some(id) if is_fresh(id) => Some(id.clone()),
            Some(_) if !hub.arbitration.failover_enabled => None,
            Some(_) => fallback(),
            None if previous.is_some() => previous,
            None if hub.arbitration.failover_enabled || !hub.active_sources.get(&group).is_some_and(Option::is_some) => fallback(),
            None => None,
        };
        selected.insert(group, chosen);
    }
    selected
}

fn copy_group(vessel: &mut signalk::VesselState, observation: &Observation, group: DataGroup) {
    match group {
        DataGroup::Position => {
            vessel.lat = observation.lat;
            vessel.lon = observation.lon;
            vessel.datetime.clone_from(&observation.datetime);
        }
        DataGroup::Motion => {
            vessel.cog = observation.cog;
            vessel.sog = observation.sog;
        }
        DataGroup::Heading => {
            vessel.heading = observation.heading;
            vessel.heading_mag = observation.heading_mag;
            vessel.variation = observation.variation;
        }
        DataGroup::SpeedThroughWater => vessel.stw = observation.stw,
        DataGroup::Wind => {
            vessel.aws = observation.aws;
            vessel.awa = observation.awa;
            vessel.tws = observation.tws;
            vessel.twa = observation.twa;
            vessel.twd = observation.twd;
        }
        DataGroup::Depth => {
            vessel.depth = observation.depth;
            vessel.depth_keel = observation.depth_keel;
            vessel.depth_surface = observation.depth_surface;
        }
        DataGroup::WaterTemperature => vessel.water_temp = observation.water_temp,
        DataGroup::Weather => {
            vessel.pressure = observation.pressure;
            vessel.air_temp = observation.air_temp;
            vessel.humidity = observation.humidity;
        }
        DataGroup::RateOfTurn => vessel.rot = observation.rot,
        DataGroup::Attitude => {
            vessel.heel = observation.heel;
            vessel.pitch = observation.pitch;
        }
        DataGroup::Rudder => vessel.rudder = observation.rudder,
        DataGroup::Current => {
            vessel.set = observation.set;
            vessel.drift = observation.drift;
        }
        DataGroup::Log => {
            vessel.log = observation.log;
            vessel.trip = observation.trip;
        }
        DataGroup::GnssQuality => {
            vessel.sats = observation.sats;
            vessel.hdop = observation.hdop;
            vessel.fix_dim = observation.fix_dim;
            vessel.pos_error = observation.pos_error;
            vessel.datum.clone_from(&observation.datum);
        }
        DataGroup::Engine => {
            vessel.battery_v = observation.battery_v;
            vessel.rpm = observation.rpm;
        }
    }
}

fn multiplexed_vessel(hub: &Multiplexer, selected: &BTreeMap<DataGroup, Option<String>>, now: Instant) -> signalk::VesselState {
    let mut vessel = signalk::VesselState { timestamp: Some(chrono::Utc::now().to_rfc3339()), ..Default::default() };
    for (group, source_id) in selected {
        let Some(source) = source_id.as_ref().and_then(|id| hub.sources.get(id)) else {
            continue;
        };
        copy_group(&mut vessel, &source.observation, *group);
        match group {
            DataGroup::Position => {
                vessel.position_observation = source.position_observation;
                vessel.position_age_ms = source.last_seen_by_group.get(group).map(|at| now.duration_since(*at).as_millis() as u64);
            }
            DataGroup::Depth => {
                vessel.depth_age_ms = source.depth_seen.iter().map(|(field, at)| (field.clone(), now.duration_since(*at).as_millis() as u64)).collect();
            }
            _ => {}
        }
    }
    vessel
}

fn publish_multiplexed(app: &tauri::AppHandle, force: bool) {
    let now = Instant::now();
    let (vessel, group_sources, switches, changed) = {
        let mut hub = MULTIPLEXER.lock().unwrap_or_else(|e| e.into_inner());
        let selected = selected_sources(&mut hub, now);
        let changed = selected != hub.active_sources;
        let switches = DataGroup::ALL
            .into_iter()
            .filter_map(|group| {
                let before = hub.active_sources.get(&group).cloned().flatten();
                let after = selected.get(&group).cloned().flatten();
                (before.is_some() && after.is_some() && before != after).then(|| (group, before.unwrap(), after.unwrap()))
            })
            .collect::<Vec<_>>();
        hub.active_sources = selected.clone();
        let vessel = multiplexed_vessel(&hub, &selected, now);
        (
            vessel,
            selected.iter().filter_map(|(group, source)| source.as_ref().map(|source| (group.key(), source.clone()))).collect::<BTreeMap<_, _>>(),
            switches,
            changed,
        )
    };

    let group_details = {
        let hub = MULTIPLEXER.lock().unwrap_or_else(|e| e.into_inner());
        DataGroup::ALL
            .iter()
            .filter_map(|group| {
                let id = group_sources.get(group.key())?;
                let name = hub.source_configs.iter().find(|source| source.id == *id).map(|source| source.name.clone()).unwrap_or_else(|| id.clone());
                Some(format!("{}: {name}", group.label()))
            })
            .collect::<Vec<_>>()
    };
    let any_connected = STATUSES.lock().unwrap_or_else(|e| e.into_inner()).values().any(|status| status.state == "connected" || status.state == "receiving");
    for (group, from, to) in switches {
        let (from_name, to_name) = {
            let hub = MULTIPLEXER.lock().unwrap_or_else(|e| e.into_inner());
            let source_name =
                |id: &str| hub.source_configs.iter().find(|source| source.id == id).map(|source| source.name.clone()).unwrap_or_else(|| id.to_owned());
            (source_name(&from), source_name(&to))
        };
        if let Err(error) = app.emit(
            "nmea-failover",
            serde_json::json!({
                "group": group.key(),
                "label": group.label(),
                "from": from_name,
                "to": to_name,
            }),
        ) {
            eprintln!("Could not publish NMEA failover alarm: {error}");
        }
    }
    // the status line: when the sources change, and once a second
    let status_due = !force || changed;
    if let Some(Err(error)) = status_due.then(|| {
        app.emit(
            "nmea-mux-status",
            serde_json::json!({
                "connected": any_connected,
                "state": if group_sources.is_empty() { "waiting" } else { "receiving" },
                "detail": if group_details.is_empty() {
                    "Waiting for fresh NMEA data".to_owned()
                } else {
                    group_details.join(" · ")
                },
                "groupSources": group_sources,
            }),
        )
    }) {
        eprintln!("Could not publish NMEA multiplexer status: {error}");
    }
    if force || changed {
        crate::rec_vessel(app, &vessel);
        if let Err(error) = app.emit(
            "vessel",
            serde_json::json!({
                "source": "nmea0183",
                "sourceId": "nmea-multiplexer",
                "sourceName": "NMEA multiplexer",
                "groupSources": group_sources,
                "replay": false,
                "v": vessel,
            }),
        ) {
            eprintln!("Could not publish multiplexed NMEA vessel data: {error}");
        }
    }
}

fn consume_bytes(app: &tauri::AppHandle, source: &SourceConfig, generation: u64, framer: &mut Framer, observation: &mut Observation, bytes: &[u8]) {
    if !is_current(&source.id, generation) {
        return;
    }
    for (raw, parsed) in framer.push_with_raw(bytes) {
        match parsed {
            Ok(sentence) => {
                let ais = matches!(sentence.formatter.as_str(), "VDM" | "VDO");
                let (verdict, gps) = if ais { (Verdict::Use, None) } else { fix_verdict(app, source, &sentence) };
                record_monitor_frame(source, sentence.raw.clone(), sentence.checksum_valid, None, gps, verdict == Verdict::Ignore, ais);
                if ais {
                    // AIS: a target for the chart, not navigation data
                    let update = AIS_DECODERS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .entry(source.id.clone())
                        .or_default()
                        .push(&sentence.formatter, &sentence.fields);
                    if let Some(update) = update {
                        crate::live::merge_ais(update);
                    }
                } else if verdict == Verdict::Ignore {
                    // a fix from the other GPS receiver on this input (or out of time order)
                } else if matches!(sentence.formatter.as_str(), "DSC" | "DSE") {
                    dsc_sentence(app, source, &sentence);
                } else if matches!(sentence.formatter.as_str(), "ALR" | "ALF") {
                    equipment_alert(app, source, &sentence);
                } else if matches!(sentence.formatter.as_str(), "TTM" | "TLL") {
                    radar_target(app, source, &sentence);
                } else {
                    observation.apply(&sentence);
                    let mut hub = MULTIPLEXER.lock().unwrap_or_else(|e| e.into_inner());
                    let source_observation = hub.sources.entry(source.id.clone()).or_default();
                    source_observation.observation = observation.clone();
                    update_depth_seen(source_observation, &sentence, Instant::now());
                    for group in sentence_groups(&sentence, observation) {
                        source_observation.last_seen_by_group.insert(group, Instant::now());
                        if group == DataGroup::Position {
                            source_observation.position_observation = POSITION_OBSERVATION.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    VESSEL_DIRTY.store(true, Ordering::Relaxed);
                }
                update_status(app, &source.id, generation, None, None, true, Some(&sentence));
            }
            Err(error) => invalid_sentence(app, source, generation, String::from_utf8_lossy(&raw).trim().to_owned(), &error),
        }
    }
}

/// A DSC call from the VHF (distress, urgency, safety) to the app ("dsc"), and a DSE's enhanced
/// position added to the call it follows (sent again with the better position).
fn dsc_sentence(app: &tauri::AppHandle, source: &SourceConfig, sentence: &Sentence) {
    use marine_nmea::dsc;
    let call = if sentence.formatter == "DSC" {
        let Some(call) = dsc::parse_dsc(&sentence.fields) else { return };
        DSC_LAST.lock().unwrap_or_else(|e| e.into_inner()).insert(call.mmsi.clone(), call.clone());
        call
    } else {
        let Some((mmsi, dlat, dlon)) = dsc::parse_dse(&sentence.fields) else { return };
        let mut last = DSC_LAST.lock().unwrap_or_else(|e| e.into_inner());
        let Some(call) = last.get_mut(&mmsi) else { return };
        let (Some(lat), Some(lon)) = (call.lat, call.lon) else { return };
        let (lat, lon) = dsc::refine(lat, lon, dlat, dlon);
        call.lat = Some(lat);
        call.lon = Some(lon);
        call.expansion = false;
        call.clone()
    };
    if let Err(error) = app.emit("dsc", serde_json::json!({ "call": call, "source": source.name, "raw": sentence.raw })) {
        eprintln!("Could not publish a DSC call: {error}");
    }
}

/// An alarm from other equipment (ALR, ALF) to the app's alert list ("equipment-alert").
fn equipment_alert(app: &tauri::AppHandle, source: &SourceConfig, sentence: &Sentence) {
    use marine_nmea::alerts;
    let alert = if sentence.formatter == "ALR" { alerts::parse_alr(&sentence.fields) } else { alerts::parse_alf(&sentence.fields) };
    let Some(alert) = alert else { return };
    if let Err(error) =
        app.emit("equipment-alert", serde_json::json!({ "alert": alert, "talker": sentence.talker, "source": source.name, "sourceId": source.id }))
    {
        eprintln!("Could not publish an equipment alert: {error}");
    }
}

/// Radar targets by source and number: a TTM and a TLL for the same target are merged.
static RADAR: Mutex<BTreeMap<String, BTreeMap<u32, marine_nmea::radar::RadarTarget>>> = Mutex::new(BTreeMap::new());

/// A radar target (TTM, TLL) to the app ("radar-target"), with what the other sentence gave.
fn radar_target(app: &tauri::AppHandle, source: &SourceConfig, sentence: &Sentence) {
    use marine_nmea::radar;
    let parsed = if sentence.formatter == "TTM" { radar::parse_ttm(&sentence.fields) } else { radar::parse_tll(&sentence.fields) };
    let Some(t) = parsed else { return };
    let merged = {
        let mut all = RADAR.lock().unwrap_or_else(|e| e.into_inner());
        let entry = all.entry(source.id.clone()).or_default().entry(t.number).or_insert_with(|| t.clone());
        if sentence.formatter == "TTM" {
            // the TTM's movement and the latest TLL position kept together
            let (lat, lon) = (entry.lat, entry.lon);
            *entry = radar::RadarTarget { lat, lon, ..t };
        } else {
            entry.lat = t.lat;
            entry.lon = t.lon;
            entry.status = t.status;
            if t.name.is_some() {
                entry.name = t.name;
            }
        }
        entry.clone()
    };
    if let Err(error) = app.emit("radar-target", serde_json::json!({ "target": merged, "source": source.name, "sourceId": source.id })) {
        eprintln!("Could not publish a radar target: {error}");
    }
}

/// Sort a position sentence into its receiver's stream and say whether it is the one in use.
/// A change of stream is reported as a source change (an alarm), as a change of input is.
/// Also which receiver it was ("A", "B" …), when there is more than one on the input.
fn fix_verdict(app: &tauri::AppHandle, source: &SourceConfig, sentence: &Sentence) -> (Verdict, Option<String>) {
    let (verdict, switch, gps) = {
        let mut all = FIX_STREAMS.lock().unwrap_or_else(|e| e.into_inner());
        let fs = all.entry(source.id.clone()).or_insert_with(|| {
            let mut fs = FixStreams::default();
            fs.set_pinned(source.gps_stream.clone());
            fs
        });
        let verdict = fs.accept(sentence, clock());
        let gps = if fs.count() > 1 { fs.last_letter().map(String::from) } else { None };
        (verdict, fs.take_switch(), gps)
    };
    if let Some((from, to)) = switch {
        let payload = serde_json::json!({ "group": format!("gps-{}", source.id), "label": format!("GPS on {}", source.name), "from": from, "to": to });
        if let Err(error) = app.emit("nmea-failover", payload) {
            eprintln!("Could not publish GPS stream change: {error}");
        }
    }
    (verdict, gps)
}

/// Use this GPS receiver's fixes on this input (`key` as in its stream summary; None: the best
/// one automatically). Takes effect at once, without restarting the inputs, and is saved.
#[tauri::command]
pub fn nmea_gps_stream_set(app: tauri::AppHandle, source_id: String, key: Option<String>) -> Result<(), String> {
    let key = key.filter(|k| !k.is_empty());
    let mut settings = crate::settings::settings_get(app.clone())?;
    let source = settings.nmea_sources.iter_mut().find(|s| s.id == source_id).ok_or("No such NMEA input")?;
    source.gps_stream.clone_from(&key);
    crate::settings::settings_save(app, settings)?;
    if let Some(cfg) = MULTIPLEXER.lock().unwrap_or_else(|e| e.into_inner()).source_configs.iter_mut().find(|s| s.id == source_id) {
        cfg.gps_stream.clone_from(&key);
    }
    if let Some(fs) = FIX_STREAMS.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&source_id) {
        fs.set_pinned(key);
    }
    Ok(())
}

/// The file a capture is being written to (one at a time). Captures can run for hours, so the
/// page sends its sentences every few seconds and they are appended here, not kept in the page.
static CAPTURE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

/// Start a capture to a file: asks where, writes `header` and returns the path (None if cancelled).
#[tauri::command]
pub async fn nmea_capture_begin(app: tauri::AppHandle, header: String, suggest: Option<String>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    // the dialog waits for the person, so it waits on a thread of its own, not one of the runtime's
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().set_title("Save NMEA capture").add_filter("NMEA text log", &["txt", "log"]);
        if let Some(name) = suggest {
            dialog = dialog.set_file_name(&name);
        }
        dialog.blocking_save_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(path) = picked else { return Ok(None) };
    let mut path = path.into_path().map_err(|error| error.to_string())?;
    if path.extension().is_none_or(|extension| !extension.eq_ignore_ascii_case("txt") && !extension.eq_ignore_ascii_case("log")) {
        path.set_extension("txt");
    }
    std::fs::write(&path, header).map_err(|error| format!("Could not write NMEA capture {}: {error}", path.display()))?;
    *CAPTURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.clone());
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Append to the capture started with `nmea_capture_begin` (only that file can be written).
#[tauri::command]
pub fn nmea_capture_append(text: String) -> Result<(), String> {
    use std::io::Write;
    let guard = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(path) = guard.as_ref() else { return Err("No NMEA capture is running".into()) };
    let mut f = std::fs::OpenOptions::new().append(true).open(path).map_err(|error| format!("Could not write NMEA capture {}: {error}", path.display()))?;
    f.write_all(text.as_bytes()).map_err(|error| format!("Could not write NMEA capture {}: {error}", path.display()))
}

/// End the capture: `discard` deletes the file (a cancelled capture leaves nothing behind).
#[tauri::command]
pub fn nmea_capture_end(discard: bool) -> Result<(), String> {
    let Some(path) = CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).take() else { return Ok(()) };
    if discard {
        std::fs::remove_file(&path).map_err(|error| format!("Could not delete {}: {error}", path.display()))?;
    }
    Ok(())
}

fn start_sources(app: &tauri::AppHandle, sources: &[SourceConfig], mut arbitration: ArbitrationConfig) {
    if arbitration.preferred_sources.is_empty() {
        if let Some(primary) = sources.iter().find(|source| source.enabled && source.use_for_navigation) {
            for group in DataGroup::ALL {
                arbitration.preferred_sources.insert(group.key().to_owned(), primary.id.clone());
            }
        }
    }
    let mux_enabled = sources.iter().any(|source| source.enabled);
    let navigation_source = mux_enabled.then(|| serde_json::json!({ "id": "nmea-multiplexer", "name": "NMEA multiplexer" }));
    *NAVIGATION_SOURCE.lock().unwrap_or_else(|e| e.into_inner()) = navigation_source.as_ref().map(|_| "nmea-multiplexer".to_owned());
    {
        let mut hub = MULTIPLEXER.lock().unwrap_or_else(|e| e.into_inner());
        hub.sources.clear();
        AIS_DECODERS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        RECENT.lock().unwrap_or_else(|e| e.into_inner()).clear();
        STATUS_DIRTY.lock().unwrap_or_else(|e| e.into_inner()).clear();
        hub.active_sources.clear();
        hub.source_configs = sources.to_vec();
        let mut streams = FIX_STREAMS.lock().unwrap_or_else(|e| e.into_inner());
        streams.clear();
        for source in sources.iter().filter(|s| s.enabled) {
            let mut fs = FixStreams::default();
            fs.stale_after = arbitration.stale_after_seconds.max(2) as f64;
            fs.set_pinned(source.gps_stream.clone());
            streams.insert(source.id.clone(), fs);
        }
        hub.arbitration = arbitration;
    }
    if let Err(error) = app.emit("nmea-navigation-source", navigation_source) {
        eprintln!("Could not publish selected NMEA source: {error}");
    }
    static TICKER_STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if TICKER_STARTED.set(()).is_ok() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // new data out at most 10 times a second; once a second regardless, so data
            // that has gone stale drops out; the monitor (counts, sentences) twice a second
            let mut interval = tokio::time::interval(PUBLISH_EVERY);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut tick = 0u32;
            loop {
                interval.tick().await;
                tick = tick.wrapping_add(1);
                if !crate::live::is_replaying() {
                    if VESSEL_DIRTY.swap(false, Ordering::Relaxed) {
                        publish_multiplexed(&app, true);
                    } else if tick.is_multiple_of(FORCE_TICKS) {
                        publish_multiplexed(&app, false);
                    }
                }
                if tick.is_multiple_of(MONITOR_TICKS) {
                    publish_monitor(&app);
                }
            }
        });
    }
    let mut running = SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    for stop in running.drain(..) {
        let _ = stop.send(true);
    }

    let enabled = sources.iter().filter(|source| source.enabled);
    let mut statuses = STATUSES.lock().unwrap_or_else(|e| e.into_inner());
    statuses.clear();
    let mut initial_statuses = Vec::new();
    let mut to_start = Vec::new();
    for source in enabled {
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        let (stop_tx, stop_rx) = watch::channel(false);
        running.push(stop_tx);
        let status = SourceStatus {
            generation,
            id: source.id.clone(),
            name: source.name.clone(),
            state: "connecting".into(),
            detail: "Starting source".into(),
            valid_sentences: 0,
            invalid_sentences: 0,
            last_sentence: None,
            last_seen: None,
            last_seen_at: None,
            gps_streams: Vec::new(),
        };
        statuses.insert(source.id.clone(), status.clone());
        initial_statuses.push(status);
        to_start.push((source.clone(), generation, stop_rx));
    }
    drop(statuses);
    drop(running);
    // "Starting source" goes out before a source runs: a UDP source binds at once, and its
    // "Listening" must not be overtaken (and replaced on the screen) by this first status
    for status in initial_statuses {
        emit_status(app, status);
    }
    for (source, generation, stop_rx) in to_start {
        tauri::async_runtime::spawn(run_source(app.clone(), source, generation, stop_rx));
    }
    publish_multiplexed(app, true);
}

pub(crate) fn navigation_source_id() -> Option<String> {
    NAVIGATION_SOURCE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

async fn run_source(app: tauri::AppHandle, source: SourceConfig, generation: u64, mut stop: watch::Receiver<bool>) {
    match &source.transport {
        SourceTransport::Serial { port, baud_rate } => {
            run_serial(&app, &source, generation, port.clone(), *baud_rate, &mut stop).await;
        }
        SourceTransport::Tcp { address } => {
            run_tcp(&app, &source, generation, address.clone(), &mut stop).await;
        }
        SourceTransport::Udp { bind_address, send_to } => {
            run_udp(&app, &source, generation, bind_address.clone(), send_to.clone(), &mut stop).await;
        }
    }
    if *stop.borrow() {
        update_status(&app, &source.id, generation, Some("stopped"), Some("Source stopped".into()), false, None);
    }
}

async fn wait_or_stop(stop: &mut watch::Receiver<bool>, duration: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        changed = stop.changed() => changed.is_err() || *stop.borrow(),
    }
}

async fn run_tcp(app: &tauri::AppHandle, source: &SourceConfig, generation: u64, address: String, stop: &mut watch::Receiver<bool>) {
    let mut observation = Observation::default();
    loop {
        if *stop.borrow() {
            break;
        }
        update_status(app, &source.id, generation, Some("connecting"), Some(format!("Connecting to {address}")), false, None);
        let stream = tokio::select! {
            _ = stop.changed() => break,
            result = tokio::net::TcpStream::connect(&address) => result,
        };
        match stream {
            Ok(stream) => {
                let (mut stream, mut writer) = stream.into_split();
                let mut outbound = outbound_open(&source.id);
                let mut framer = Framer::default();
                update_status(app, &source.id, generation, Some("connected"), Some(format!("Connected to {address}; waiting for NMEA data")), false, None);
                let mut buf = [0u8; 2048];
                loop {
                    let read = tokio::select! {
                        _ = stop.changed() => { outbound_close(&source.id); return; }
                        Some(bytes) = outbound.recv() => {
                            if let Err(error) = writer.write_all(&bytes).await {
                                eprintln!("Could not send on {address}: {error}");
                            }
                            continue;
                        }
                        result = stream.read(&mut buf) => result,
                    };
                    match read {
                        Ok(0) => {
                            update_status(
                                app,
                                &source.id,
                                generation,
                                Some("connecting"),
                                Some("TCP peer closed the connection; retrying".into()),
                                false,
                                None,
                            );
                            break;
                        }
                        Ok(n) => {
                            consume_bytes(app, source, generation, &mut framer, &mut observation, &buf[..n]);
                        }
                        Err(error) => {
                            update_status(app, &source.id, generation, Some("error"), Some(format!("TCP read failed: {error}; retrying")), false, None);
                            break;
                        }
                    }
                }
                outbound_close(&source.id);
            }
            Err(error) => {
                update_status(app, &source.id, generation, Some("error"), Some(format!("Could not connect to {address}: {error}")), false, None);
            }
        }
        if *stop.borrow() || wait_or_stop(stop, Duration::from_secs(2)).await {
            break;
        }
    }
}

async fn run_udp(
    app: &tauri::AppHandle,
    source: &SourceConfig,
    generation: u64,
    bind_address: String,
    send_to: Option<String>,
    stop: &mut watch::Receiver<bool>,
) {
    let mut framer = Framer::default();
    let mut observation = Observation::default();
    let mut buf = [0u8; 65536];
    let send_to = send_to.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    loop {
        if *stop.borrow() {
            break;
        }
        update_status(app, &source.id, generation, Some("connecting"), Some(format!("Binding UDP {bind_address}")), false, None);
        let socket = match tokio::net::UdpSocket::bind(&bind_address).await {
            Ok(socket) => socket,
            Err(error) => {
                update_status(app, &source.id, generation, Some("error"), Some(format!("Could not bind UDP {bind_address}: {error}; retrying")), false, None);
                if *stop.borrow() || wait_or_stop(stop, Duration::from_secs(2)).await {
                    break;
                }
                continue;
            }
        };
        // sending: to a host:port (looked up once per binding); broadcast allowed for x.x.x.255
        let target = match &send_to {
            Some(to) => match tokio::net::lookup_host(to.as_str()).await.ok().and_then(|mut a| a.find(|a| a.is_ipv4()).or(None)) {
                Some(addr) => {
                    let _ = socket.set_broadcast(true);
                    Some(addr)
                }
                None => {
                    update_status(
                        app,
                        &source.id,
                        generation,
                        Some("error"),
                        Some(format!("Can't send to {to}: not a host:port this computer can reach; retrying")),
                        false,
                        None,
                    );
                    if *stop.borrow() || wait_or_stop(stop, Duration::from_secs(5)).await {
                        break;
                    }
                    continue;
                }
            },
            None => None,
        };
        let mut outbound = target.map(|_| outbound_open(&source.id));
        update_status(
            app,
            &source.id,
            generation,
            Some("connected"),
            Some(match &send_to {
                Some(to) => format!("Listening on UDP {bind_address}, sending to {to}; nothing received yet"),
                None => format!("Listening on UDP {bind_address}; nothing received yet (check the sender's address and port)"),
            }),
            false,
            None,
        );
        loop {
            let received = tokio::select! {
                _ = stop.changed() => break,
                Some(bytes) = async { match outbound.as_mut() { Some(o) => o.recv().await, None => std::future::pending().await } } => {
                    if let Some(addr) = target {
                        if let Err(error) = socket.send_to(&bytes, addr).await {
                            eprintln!("Could not send UDP to {addr}: {error}");
                        }
                    }
                    continue;
                }
                result = socket.recv_from(&mut buf) => result,
            };
            match received {
                Ok((n, _peer)) => consume_bytes(app, source, generation, &mut framer, &mut observation, &buf[..n]),
                Err(error) => {
                    // Windows reports an unreachable port of an earlier send as a receive error: carry on
                    if error.kind() == std::io::ErrorKind::ConnectionReset {
                        continue;
                    }
                    update_status(app, &source.id, generation, Some("error"), Some(format!("UDP receive failed: {error}; retrying")), false, None);
                    break;
                }
            }
        }
        if target.is_some() {
            outbound_close(&source.id);
        }
        if *stop.borrow() || wait_or_stop(stop, Duration::from_secs(2)).await {
            break;
        }
    }
}

enum SerialMessage {
    Opened,
    Data(Vec<u8>),
    Failed(String),
}

async fn run_serial(app: &tauri::AppHandle, source: &SourceConfig, generation: u64, port_name: String, baud_rate: u32, stop: &mut watch::Receiver<bool>) {
    let mut observation = Observation::default();
    loop {
        if *stop.borrow() {
            break;
        }
        update_status(app, &source.id, generation, Some("connecting"), Some(format!("Opening {port_name} at {baud_rate} baud")), false, None);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let worker_port = port_name.clone();
        let mut outbound = outbound_open(&source.id);
        let worker = tokio::task::spawn_blocking(move || {
            let mut port = match serialport::new(&worker_port, baud_rate).timeout(Duration::from_millis(250)).open() {
                Ok(port) => port,
                Err(error) => {
                    let _ = tx.send(SerialMessage::Failed(format!("Could not open {worker_port}: {error}")));
                    return;
                }
            };
            let _ = tx.send(SerialMessage::Opened);
            let mut buf = [0u8; 2048];
            while !worker_cancelled.load(Ordering::Relaxed) {
                // anything to send (the autopilot output) goes out between reads (they wait 250 ms at most)
                while let Ok(bytes) = outbound.try_recv() {
                    if let Err(error) = port.write_all(&bytes) {
                        eprintln!("Could not send on {worker_port}: {error}");
                    }
                }
                match port.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        if tx.send(SerialMessage::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(error) => {
                        let _ = tx.send(SerialMessage::Failed(format!("Read failed on {worker_port}: {error}")));
                        break;
                    }
                }
            }
        });

        let mut framer = Framer::default();
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        cancelled.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                message = rx.recv() => match message {
                    Some(SerialMessage::Opened) =>                     update_status(app, &source.id, generation, Some("connected"), Some(format!("Connected to {port_name} at {baud_rate} baud; waiting for NMEA data")), false, None),
                    Some(SerialMessage::Data(bytes)) => consume_bytes(app, source, generation, &mut framer, &mut observation, &bytes),
                    Some(SerialMessage::Failed(error)) => {
                        update_status(app, &source.id, generation, Some("error"), Some(error), false, None);
                        cancelled.store(true, Ordering::Relaxed);
                        break;
                    }
                    None => {
                        update_status(app, &source.id, generation, Some("error"), Some("Serial reader closed unexpectedly".into()), false, None);
                        break;
                    }
                }
            }
        }
        cancelled.store(true, Ordering::Relaxed);
        outbound_close(&source.id);
        if let Err(error) = worker.await {
            update_status(app, &source.id, generation, Some("error"), Some(format!("Serial reader failed: {error}")), false, None);
        }
        if *stop.borrow() || wait_or_stop(stop, Duration::from_secs(2)).await {
            break;
        }
    }
}

#[tauri::command]
pub fn nmea_ports() -> Result<Vec<serde_json::Value>, String> {
    serialport::available_ports()
        .map(|ports| {
            ports
                .into_iter()
                .map(|port| {
                    serde_json::json!({
                        "name": port.port_name,
                        "kind": format!("{:?}", port.port_type),
                    })
                })
                .collect()
        })
        .map_err(|error| format!("Could not enumerate serial ports: {error}"))
}

#[tauri::command]
pub fn nmea_sources_get(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let settings = crate::settings::settings_get(app)?;
    let mut arbitration = settings.nmea_arbitration.clone();
    if arbitration.preferred_sources.is_empty() {
        if let Some(primary) = settings.nmea_sources.iter().find(|source| source.enabled && source.use_for_navigation) {
            for group in DataGroup::ALL {
                arbitration.preferred_sources.insert(group.key().to_owned(), primary.id.clone());
            }
        }
    }
    let statuses = STATUSES.lock().map_err(|error| format!("NMEA status lock failed: {error}"))?;
    Ok(serde_json::json!({
        "sources": settings.nmea_sources,
        "arbitration": arbitration,
        "statuses": statuses.values().collect::<Vec<_>>(),
    }))
}

#[tauri::command]
pub fn nmea_sources_set(app: tauri::AppHandle, sources: Vec<SourceConfig>, arbitration: ArbitrationConfig) -> Result<(), String> {
    let mut settings = crate::settings::settings_get(app.clone())?;
    settings.nmea_sources = sources;
    settings.nmea_arbitration = arbitration;
    crate::settings::settings_save(app.clone(), settings.clone())?;
    start_sources(&app, &settings.nmea_sources, settings.nmea_arbitration);
    if let Err(error) = app.emit("nmea-sources-changed", serde_json::json!({})) {
        eprintln!("Could not publish changed NMEA sources: {error}");
    }
    Ok(())
}

pub fn start(app: &tauri::AppHandle) {
    match crate::settings::settings_get(app.clone()) {
        Ok(settings) => start_sources(app, &settings.nmea_sources, settings.nmea_arbitration),
        Err(error) => eprintln!("Could not load saved NMEA sources: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str) -> SourceConfig {
        SourceConfig {
            id: id.into(),
            name: id.into(),
            enabled: true,
            use_for_navigation: false,
            transport: SourceTransport::Tcp { address: "127.0.0.1:10110".into() },
            gps_stream: None,
        }
    }

    fn observed_position(now: Instant) -> SourceObservation {
        SourceObservation {
            observation: Observation { lat: Some(-41.0), lon: Some(173.0), ..Default::default() },
            last_seen_by_group: BTreeMap::from([(DataGroup::Position, now)]),
            depth_seen: BTreeMap::new(),
            position_observation: 1,
        }
    }

    #[test]
    fn source_configuration_uses_explicit_transport_kinds() {
        let source: SourceConfig = serde_json::from_value(serde_json::json!({
            "id": "gps-main",
            "name": "Main GPS",
            "enabled": true,
            "useForNavigation": true,
            "transport": { "type": "serial", "port": "COM4", "baudRate": 4800 }
        }))
        .unwrap();
        assert_eq!(source.transport, SourceTransport::Serial { port: "COM4".into(), baud_rate: 4800 });
        assert!(source.use_for_navigation);
    }

    #[test]
    fn source_arbitration_prefers_group_choice_then_fails_over_in_order() {
        let now = Instant::now();
        let mut hub = Multiplexer {
            source_configs: vec![source("backup"), source("preferred")],
            arbitration: ArbitrationConfig { preferred_sources: BTreeMap::from([("position".into(), "preferred".into())]), ..Default::default() },
            ..Default::default()
        };
        hub.sources.insert("backup".into(), observed_position(now));
        hub.sources.insert("preferred".into(), observed_position(now));

        assert_eq!(selected_sources(&mut hub, now).get(&DataGroup::Position), Some(&Some("preferred".into())));

        let backup_time = now - Duration::from_secs(5);
        hub.sources.insert("backup".into(), observed_position(backup_time));
        let stale_time = now - Duration::from_secs(11);
        hub.sources.insert("preferred".into(), observed_position(stale_time));
        assert_eq!(selected_sources(&mut hub, now).get(&DataGroup::Position), Some(&Some("backup".into())));
    }

    #[test]
    fn disabling_failover_keeps_a_preferred_source_or_returns_no_source() {
        let now = Instant::now();
        let stale = now - Duration::from_secs(11);
        let mut hub = Multiplexer {
            source_configs: vec![source("preferred"), source("backup")],
            arbitration: ArbitrationConfig {
                failover_enabled: false,
                preferred_sources: BTreeMap::from([("position".into(), "preferred".into())]),
                ..Default::default()
            },
            ..Default::default()
        };
        hub.sources.insert("preferred".into(), observed_position(stale));
        hub.sources.insert("backup".into(), observed_position(now));

        assert_eq!(selected_sources(&mut hub, now).get(&DataGroup::Position), Some(&None));
    }

    #[test]
    fn invalid_sentences_do_not_refresh_accumulated_group_values() {
        let sentence = Sentence {
            talker: "GP".into(),
            formatter: "RMC".into(),
            fields: vec!["120000".into(), "V".into(), "4100.0".into(), "S".into(), "17300.0".into(), "E".into(), "5.0".into(), "90.0".into()],
            raw: "$GPRMC,...".into(),
            checksum_valid: true,
        };
        let observation = Observation { lat: Some(-41.0), lon: Some(173.0), sog: Some(5.0), cog: Some(90.0), ..Default::default() };
        assert!(sentence_groups(&sentence, &observation).is_empty());
    }

    #[test]
    fn position_group_requires_valid_coordinates_in_the_current_sentence() {
        let old = Observation { lat: Some(-41.0), lon: Some(173.0), ..Default::default() };
        let make = |fields: &[&str]| Sentence {
            talker: "GP".into(),
            formatter: "RMC".into(),
            fields: fields.iter().map(|s| (*s).into()).collect(),
            raw: String::new(),
            checksum_valid: true,
        };
        for fields in [
            vec!["120000", "A"],
            vec!["120000", "A", "", "S", "", "E", "", "", "010124"],
            vec!["120000", "A", "4160.0", "S", "17300.0", "E", "", "", "010124"],
            vec!["120000", "A", "9100.0", "S", "17300.0", "E", "", "", "010124"],
            vec!["120000", "A", "4100.0", "N", "18100.0", "E", "", "", "010124"],
            vec!["120000", "A", "4100.0", "E", "17300.0", "S", "", "", "010124"],
        ] {
            assert!(!sentence_groups(&make(&fields), &old).contains(&DataGroup::Position), "{fields:?}");
        }
        for fields in
            [vec!["120000", "A", "4100.0", "S", "17300.0", "E", "", "", "010124"], vec!["120000", "A", "4100.0", "N", "17300.0", "W", "", "", "010124"]]
        {
            assert!(sentence_groups(&make(&fields), &old).contains(&DataGroup::Position));
        }
        for (formatter, fields, valid) in [
            ("GGA", vec!["120000", "4100.0", "S", "17300.0", "E", "1", "8", "1", "0"], true),
            ("GGA", vec!["120000", "", "S", "17300.0", "E", "1", "8", "1", "0"], false),
            ("GGA", vec!["120000", "4100.0", "S", "17300.0", "E", "0", "8", "1", "0"], false),
            ("GLL", vec!["4100.0", "S", "17300.0", "E", "120000", "A"], true),
            ("GLL", vec!["4100.0", "S", "17360.0", "E", "120000", "A"], false),
            ("GLL", vec!["4100.0", "S", "17300.0", "E", "120000", "V"], false),
        ] {
            let mut sentence = make(&fields);
            sentence.formatter = formatter.into();
            assert_eq!(sentence_groups(&sentence, &old).contains(&DataGroup::Position), valid, "{formatter} {fields:?}");
        }
    }

    #[test]
    fn depth_receive_times_follow_each_field_not_the_group() {
        let now = Instant::now();
        let mut source = SourceObservation::default();
        let sentence = |formatter: &str, fields: &[&str]| Sentence {
            talker: "SD".into(),
            formatter: formatter.into(),
            fields: fields.iter().map(|s| (*s).into()).collect(),
            raw: String::new(),
            checksum_valid: true,
        };
        let dpt = sentence("DPT", &["5.0"]);
        let keel = sentence("DBK", &["10", "f", "3", "M"]);
        update_depth_seen(&mut source, &keel, now);
        update_depth_seen(&mut source, &dpt, now + Duration::from_secs(11));
        update_depth_seen(&mut source, &sentence("DPT", &[""]), now + Duration::from_secs(12));
        assert_eq!(source.depth_seen["depth_keel"], now);
        assert_eq!(source.depth_seen["depth"], now + Duration::from_secs(11));
        update_depth_seen(&mut source, &dpt, now + Duration::from_secs(13));
        assert_eq!(source.depth_seen["depth"], now + Duration::from_secs(13));
    }

    #[test]
    fn captured_depth_sentences_publish_values_and_freshness_without_a_position_source() {
        let now = Instant::now();
        let mut hub = Multiplexer { source_configs: vec![source("Demo_UDP")], ..Default::default() };
        let source = hub.sources.entry("Demo_UDP".into()).or_default();
        for raw in ["$SDDBT,97.9,f,29.8,M,16.3,F*36", "$SDDPT,29.8,1.5*60"] {
            let sentence = marine_nmea::parse_sentence(raw.as_bytes(), marine_nmea::DEFAULT_MAX_FRAME_LEN).unwrap();
            source.observation.apply(&sentence);
            update_depth_seen(source, &sentence, now);
            for group in sentence_groups(&sentence, &source.observation) {
                source.last_seen_by_group.insert(group, now);
            }
        }
        let selected = selected_sources(&mut hub, now);
        assert_eq!(selected[&DataGroup::Depth].as_deref(), Some("Demo_UDP"));
        assert_eq!(selected[&DataGroup::Position], None);
        let vessel = multiplexed_vessel(&hub, &selected, now + Duration::from_millis(250));
        let payload = serde_json::to_value(vessel).unwrap();
        assert_eq!(payload["depth"], 29.8);
        assert_eq!(payload["depth_age_ms"]["depth"], 250);
        assert!(payload["position_age_ms"].is_null());
    }

    #[test]
    fn multiplexed_depth_ages_follow_the_selected_source_and_each_measurement() {
        let now = Instant::now();
        let mut hub = Multiplexer {
            source_configs: vec![source("primary"), source("backup")],
            arbitration: ArbitrationConfig { preferred_sources: BTreeMap::from([("depth".into(), "primary".into())]), ..Default::default() },
            ..Default::default()
        };
        let primary = SourceObservation {
            observation: Observation { depth: Some(5.0), depth_keel: Some(3.0), depth_surface: Some(7.0), ..Default::default() },
            last_seen_by_group: BTreeMap::from([(DataGroup::Depth, now - Duration::from_secs(11))]),
            depth_seen: BTreeMap::from([
                ("depth".into(), now - Duration::from_secs(11)),
                ("depth_keel".into(), now - Duration::from_secs(12)),
                ("depth_surface".into(), now - Duration::from_secs(13)),
            ]),
            ..Default::default()
        };
        let backup = SourceObservation {
            observation: Observation { depth: Some(9.0), depth_keel: Some(6.0), depth_surface: Some(10.0), ..Default::default() },
            last_seen_by_group: BTreeMap::from([(DataGroup::Depth, now - Duration::from_secs(2))]),
            depth_seen: BTreeMap::from([
                ("depth".into(), now - Duration::from_secs(2)),
                ("depth_keel".into(), now - Duration::from_secs(3)),
                ("depth_surface".into(), now - Duration::from_secs(4)),
            ]),
            ..Default::default()
        };
        hub.sources.insert("primary".into(), primary);
        hub.sources.insert("backup".into(), backup);
        let selected = selected_sources(&mut hub, now);
        assert_eq!(selected[&DataGroup::Depth].as_deref(), Some("backup"));
        let vessel = multiplexed_vessel(&hub, &selected, now);
        assert_eq!((vessel.depth, vessel.depth_keel, vessel.depth_surface), (Some(9.0), Some(6.0), Some(10.0)));
        assert_eq!(vessel.depth_age_ms["depth"], 2000);
        assert_eq!(vessel.depth_age_ms["depth_keel"], 3000);
        assert_eq!(vessel.depth_age_ms["depth_surface"], 4000);
    }
}
