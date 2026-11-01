use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub vessel: VesselProfile,
    pub nmea_sources: Vec<crate::nmea::SourceConfig>,
    pub nmea_arbitration: crate::nmea::ArbitrationConfig,
    pub schema_version: u32,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VesselProfile {
    pub name: String,
    pub imo: String,
    pub call_sign: String,
    pub mmsi: String,
    pub draft_m: Option<f64>,
    pub length_m: Option<f64>,
    pub breadth_m: Option<f64>,
    pub air_draft_m: Option<f64>,
    pub reference_lat: Option<f64>,
    pub reference_lon: Option<f64>,
    pub gps_antenna_from_bow_m: Option<f64>,
    pub gps_antenna_from_centerline_m: Option<f64>,
    /// Depth of the sounder's transducer below the waterline (m): added to the depth below the
    /// transducer for own soundings.
    pub transducer_depth_m: Option<f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map(|dir| dir.join("settings.json")).map_err(|e| format!("Could not locate the application settings folder: {e}"))
}

fn read_settings(path: &std::path::Path) -> Result<Settings, String> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("Could not parse {}: {e}", path.display()))
}

#[tauri::command]
pub fn settings_get(app: tauri::AppHandle) -> Result<Settings, String> {
    let _guard = SETTINGS_LOCK.lock().map_err(|e| format!("Settings lock failed: {e}"))?;
    read_settings(&settings_path(&app)?)
}

#[tauri::command]
pub fn settings_save(app: tauri::AppHandle, mut settings: Settings) -> Result<(), String> {
    validate(&settings)?;
    let _guard = SETTINGS_LOCK.lock().map_err(|e| format!("Settings lock failed: {e}"))?;
    settings.schema_version = 1;
    let path = settings_path(&app)?;
    let parent = path.parent().ok_or("Settings path has no parent folder.")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Could not create {}: {e}", parent.display()))?;

    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| format!("Could not serialize settings: {e}"))?;
    let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), std::thread::current().name().unwrap_or("settings")));
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp).map_err(|e| format!("Could not write temporary settings file: {e}"))?;
        file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(|e| format!("Could not flush settings: {e}"))?;
        std::fs::rename(&temp, &path).map_err(|e| format!("Could not replace {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn validate(s: &Settings) -> Result<(), String> {
    let v = &s.vessel;
    for (label, text, max) in [("Vessel name", &v.name, 100), ("Call sign", &v.call_sign, 32), ("IMO number", &v.imo, 7)] {
        if text.chars().count() > max {
            return Err(format!("{label} must be at most {max} characters."));
        }
    }
    if !v.imo.is_empty() && (v.imo.len() != 7 || !v.imo.bytes().all(|b| b.is_ascii_digit()) || !valid_imo(&v.imo)) {
        return Err("IMO number must be a valid seven-digit IMO number.".into());
    }
    if !v.mmsi.is_empty() && (v.mmsi.len() != 9 || !v.mmsi.bytes().all(|b| b.is_ascii_digit())) {
        return Err("MMSI must contain exactly nine digits.".into());
    }
    for (label, value, max) in [
        ("Draft", v.draft_m, 200.0),
        ("Length", v.length_m, 1000.0),
        ("Breadth", v.breadth_m, 200.0),
        ("Air draft", v.air_draft_m, 300.0),
        ("Transducer depth", v.transducer_depth_m, 50.0),
        ("GPS antenna distance from bow", v.gps_antenna_from_bow_m, 1000.0),
    ] {
        if value.is_some_and(|n| !n.is_finite() || n < 0.0 || n > max) {
            return Err(format!("{label} must be between 0 and {max} metres."));
        }
    }
    if v.gps_antenna_from_centerline_m.is_some_and(|n| !n.is_finite() || !(-500.0..=500.0).contains(&n)) {
        return Err("GPS antenna offset from centreline must be between -500 and 500 metres.".into());
    }
    if s.nmea_sources.len() > 32 {
        return Err("At most 32 NMEA sources can be configured.".into());
    }
    if !(5..=60).contains(&s.nmea_arbitration.stale_after_seconds) {
        return Err("NMEA source timeout must be between 5 and 60 seconds.".into());
    }
    let mut source_ids = std::collections::BTreeSet::new();
    let enabled_serial_ports: Vec<String> = s
        .nmea_sources
        .iter()
        .filter(|source| source.enabled)
        .filter_map(|source| match &source.transport {
            crate::nmea::SourceTransport::Serial { port, .. } => Some(port.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    if enabled_serial_ports.len() != enabled_serial_ports.iter().collect::<std::collections::BTreeSet<_>>().len() {
        return Err("A serial port can only be assigned to one enabled NMEA source.".into());
    }
    let enabled_udp_addresses: Vec<String> = s
        .nmea_sources
        .iter()
        .filter(|source| source.enabled)
        .filter_map(|source| match &source.transport {
            crate::nmea::SourceTransport::Udp { bind_address } => Some(bind_address.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    if enabled_udp_addresses.len() != enabled_udp_addresses.iter().collect::<std::collections::BTreeSet<_>>().len() {
        return Err("A UDP bind address can only be assigned to one enabled NMEA source.".into());
    }
    for source in &s.nmea_sources {
        if source.id.trim().is_empty() || source.id.chars().count() > 80 {
            return Err("Each NMEA source needs an ID of at most 80 characters.".into());
        }
        if !source_ids.insert(&source.id) {
            return Err(format!("NMEA source ID '{}' is duplicated.", source.id));
        }
        if source.name.trim().is_empty() || source.name.chars().count() > 100 {
            return Err("Each NMEA source needs a name of at most 100 characters.".into());
        }
        match &source.transport {
            crate::nmea::SourceTransport::Serial { port, baud_rate } => {
                if port.trim().is_empty() || port.chars().count() > 128 {
                    return Err(format!("Source '{}' needs a valid serial port name.", source.name));
                }
                if !matches!(*baud_rate, 4800 | 9600 | 19200 | 38400 | 57600 | 115200 | 230400) {
                    return Err(format!("Source '{}' has an unsupported baud rate.", source.name));
                }
            }
            crate::nmea::SourceTransport::Tcp { address } => {
                if address.trim().is_empty() || address.chars().count() > 255 || !address.contains(':') {
                    return Err(format!("Source '{}' needs a TCP host:port address.", source.name));
                }
            }
            crate::nmea::SourceTransport::Udp { bind_address } => {
                if bind_address.parse::<std::net::SocketAddr>().is_err() {
                    return Err(format!("Source '{}' needs a valid UDP bind address.", source.name));
                }
            }
        }
    }
    for (group, source_id) in &s.nmea_arbitration.preferred_sources {
        if !crate::nmea::DataGroup::is_valid_key(group) {
            return Err(format!("Unknown NMEA data group '{group}'."));
        }
        if !source_id.is_empty() && !source_ids.contains(source_id) {
            return Err(format!("Preferred source '{source_id}' for '{group}' does not exist."));
        }
    }
    match (v.reference_lat, v.reference_lon) {
        (None, None) => {}
        (Some(lat), Some(lon)) if lat.is_finite() && (-90.0..=90.0).contains(&lat) && lon.is_finite() && (-180.0..=180.0).contains(&lon) => {}
        _ => return Err("Reference position needs both a valid latitude and longitude.".into()),
    }
    Ok(())
}

fn valid_imo(imo: &str) -> bool {
    let digits: Vec<u32> = imo.bytes().map(|b| (b - b'0') as u32).collect();
    digits[..6].iter().zip([7, 6, 5, 4, 3, 2]).map(|(d, w)| d * w).sum::<u32>() % 10 == digits[6]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_vessel_identifiers_and_reference_position() {
        let mut settings = Settings::default();
        settings.vessel.imo = "9074729".into();
        settings.vessel.mmsi = "123456789".into();
        settings.vessel.reference_lat = Some(-41.2);
        settings.vessel.reference_lon = Some(173.3);
        assert!(validate(&settings).is_ok());

        settings.vessel.imo = "1234568".into();
        assert!(validate(&settings).is_err());
        settings.vessel.imo.clear();
        settings.vessel.reference_lon = None;
        assert!(validate(&settings).is_err());
    }

    #[test]
    fn settings_round_trip_preserves_future_fields() {
        let mut settings = Settings::default();
        settings.extra.insert("futureSection".into(), serde_json::json!({"enabled": true}));
        settings.vessel.extra.insert("hullType".into(), serde_json::json!("sloop"));
        let encoded = serde_json::to_string(&settings).unwrap();
        let decoded: Settings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.extra["futureSection"]["enabled"], true);
        assert_eq!(decoded.vessel.extra["hullType"], "sloop");
    }

    #[test]
    fn validates_saved_nmea_sources() {
        let mut settings = Settings::default();
        settings.nmea_sources.push(crate::nmea::SourceConfig {
            id: "gps-main".into(),
            name: "Main GPS".into(),
            enabled: true,
            use_for_navigation: true,
            transport: crate::nmea::SourceTransport::Serial { port: "COM4".into(), baud_rate: 4800 },
        });
        assert!(validate(&settings).is_ok());
        settings.nmea_sources.push(crate::nmea::SourceConfig {
            id: "gps-backup".into(),
            name: "Backup GPS".into(),
            enabled: true,
            use_for_navigation: true,
            transport: crate::nmea::SourceTransport::Serial { port: "COM5".into(), baud_rate: 4800 },
        });
        assert!(validate(&settings).is_ok());
        settings.nmea_arbitration.preferred_sources.insert("position".into(), "missing-source".into());
        assert!(validate(&settings).is_err());
        settings.nmea_arbitration.preferred_sources.clear();
        settings.nmea_sources.pop();
        settings.nmea_sources[0].transport = crate::nmea::SourceTransport::Serial { port: "COM4".into(), baud_rate: 4800 };
        settings.nmea_sources.push(crate::nmea::SourceConfig {
            id: "gps-second".into(),
            name: "Second GPS".into(),
            enabled: true,
            use_for_navigation: false,
            transport: crate::nmea::SourceTransport::Serial { port: "COM4".into(), baud_rate: 9600 },
        });
        assert!(validate(&settings).is_err());
        settings.nmea_sources.pop();
        settings.nmea_sources[0].transport = crate::nmea::SourceTransport::Serial { port: "COM4".into(), baud_rate: 1234 };
        assert!(validate(&settings).is_err());
    }
}
