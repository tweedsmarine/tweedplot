use marine_nmea::{Framer, Observation, Sentence};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Emitter;
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceTransport {
    Serial { port: String, baud_rate: u32 },
    Tcp { address: String },
    Udp { bind_address: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport: SourceTransport,
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
}

static SOURCES: Mutex<Vec<watch::Sender<bool>>> = Mutex::new(Vec::new());
static STATUSES: Mutex<BTreeMap<String, SourceStatus>> = Mutex::new(BTreeMap::new());
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn emit_status(app: &tauri::AppHandle, status: SourceStatus) {
    if let Err(error) = app.emit("nmea-status", status) {
        eprintln!("Could not publish NMEA source status: {error}");
    }
}

fn update_status(
    app: &tauri::AppHandle,
    id: &str,
    generation: u64,
    state: Option<&str>,
    detail: Option<String>,
    valid: bool,
    sentence: Option<&Sentence>,
) {
    let mut statuses = STATUSES.lock().unwrap();
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
        status.valid_sentences += 1;
        status.state = "receiving".into();
        status.last_seen = Some(chrono::Utc::now().to_rfc3339());
    }
    if let Some(sentence) = sentence {
        status.last_sentence = Some(sentence.formatter.clone());
    }
    let status = status.clone();
    drop(statuses);
    if let Err(error) = app.emit("nmea-status", status) {
        eprintln!("Could not publish NMEA source status: {error}");
    }
}

fn invalid_sentence(
    app: &tauri::AppHandle,
    id: &str,
    generation: u64,
    error: &marine_nmea::ParseError,
) {
    let mut statuses = STATUSES.lock().unwrap();
    let Some(status) = statuses.get_mut(id) else {
        return;
    };
    if status.generation != generation {
        return;
    }
    status.invalid_sentences += 1;
    status.detail = error.to_string();
    let status = status.clone();
    drop(statuses);
    if let Err(error) = app.emit("nmea-status", status) {
        eprintln!("Could not publish NMEA source status: {error}");
    }
}

fn consume_bytes(
    app: &tauri::AppHandle,
    source: &SourceConfig,
    generation: u64,
    framer: &mut Framer,
    observation: &mut Observation,
    bytes: &[u8],
) {
    for parsed in framer.push(bytes) {
        match parsed {
            Ok(sentence) => {
                observation.apply(&sentence);
                update_status(
                    app,
                    &source.id,
                    generation,
                    None,
                    None,
                    true,
                    Some(&sentence),
                );
                if let Err(error) = app.emit(
                    "nmea-observation",
                    serde_json::json!({
                        "sourceId": source.id,
                        "sourceName": source.name,
                        "sentence": sentence,
                        "observation": observation,
                    }),
                ) {
                    eprintln!("Could not publish NMEA observation: {error}");
                }
            }
            Err(error) => invalid_sentence(app, &source.id, generation, &error),
        }
    }
}

fn start_sources(app: &tauri::AppHandle, sources: &[SourceConfig]) {
    let mut running = SOURCES.lock().unwrap();
    for stop in running.drain(..) {
        let _ = stop.send(true);
    }

    let enabled = sources.iter().filter(|source| source.enabled);
    let mut statuses = STATUSES.lock().unwrap();
    statuses.clear();
    let mut initial_statuses = Vec::new();
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
        };
        statuses.insert(source.id.clone(), status.clone());
        initial_statuses.push(status);
        let app = app.clone();
        let source = source.clone();
        tauri::async_runtime::spawn(run_source(app, source, generation, stop_rx));
    }
    drop(statuses);
    drop(running);
    for status in initial_statuses {
        emit_status(app, status);
    }
}

async fn run_source(
    app: tauri::AppHandle,
    source: SourceConfig,
    generation: u64,
    mut stop: watch::Receiver<bool>,
) {
    match &source.transport {
        SourceTransport::Serial { port, baud_rate } => {
            run_serial(
                &app,
                &source,
                generation,
                port.clone(),
                *baud_rate,
                &mut stop,
            )
            .await;
        }
        SourceTransport::Tcp { address } => {
            run_tcp(&app, &source, generation, address.clone(), &mut stop).await;
        }
        SourceTransport::Udp { bind_address } => {
            run_udp(&app, &source, generation, bind_address.clone(), &mut stop).await;
        }
    }
    if *stop.borrow() {
        update_status(
            &app,
            &source.id,
            generation,
            Some("stopped"),
            Some("Source stopped".into()),
            false,
            None,
        );
    }
}

async fn wait_or_stop(stop: &mut watch::Receiver<bool>, duration: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        changed = stop.changed() => changed.is_err() || *stop.borrow(),
    }
}

async fn run_tcp(
    app: &tauri::AppHandle,
    source: &SourceConfig,
    generation: u64,
    address: String,
    stop: &mut watch::Receiver<bool>,
) {
    let mut observation = Observation::default();
    loop {
        if *stop.borrow() {
            break;
        }
        update_status(
            app,
            &source.id,
            generation,
            Some("connecting"),
            Some(format!("Connecting to {address}")),
            false,
            None,
        );
        let stream = tokio::select! {
            _ = stop.changed() => break,
            result = tokio::net::TcpStream::connect(&address) => result,
        };
        match stream {
            Ok(mut stream) => {
                let mut framer = Framer::default();
                update_status(
                    app,
                    &source.id,
                    generation,
                    Some("connected"),
                    Some(format!("Connected to {address}; waiting for NMEA data")),
                    false,
                    None,
                );
                let mut buf = [0u8; 2048];
                loop {
                    let read = tokio::select! {
                        _ = stop.changed() => return,
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
                            consume_bytes(
                                app,
                                source,
                                generation,
                                &mut framer,
                                &mut observation,
                                &buf[..n],
                            );
                        }
                        Err(error) => {
                            update_status(
                                app,
                                &source.id,
                                generation,
                                Some("error"),
                                Some(format!("TCP read failed: {error}; retrying")),
                                false,
                                None,
                            );
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                update_status(
                    app,
                    &source.id,
                    generation,
                    Some("error"),
                    Some(format!("Could not connect to {address}: {error}")),
                    false,
                    None,
                );
            }
        }
        if wait_or_stop(stop, Duration::from_secs(2)).await {
            break;
        }
    }
}

async fn run_udp(
    app: &tauri::AppHandle,
    source: &SourceConfig,
    generation: u64,
    bind_address: String,
    stop: &mut watch::Receiver<bool>,
) {
    let socket = match tokio::net::UdpSocket::bind(&bind_address).await {
        Ok(socket) => socket,
        Err(error) => {
            update_status(
                app,
                &source.id,
                generation,
                Some("error"),
                Some(format!("Could not bind UDP {bind_address}: {error}")),
                false,
                None,
            );
            return;
        }
    };
    update_status(
        app,
        &source.id,
        generation,
        Some("connected"),
        Some(format!(
            "Listening on UDP {bind_address}; waiting for NMEA data"
        )),
        false,
        None,
    );
    let mut framer = Framer::default();
    let mut observation = Observation::default();
    let mut buf = [0u8; 65536];
    loop {
        let received = tokio::select! {
            _ = stop.changed() => break,
            result = socket.recv_from(&mut buf) => result,
        };
        match received {
            Ok((n, _peer)) => consume_bytes(
                app,
                source,
                generation,
                &mut framer,
                &mut observation,
                &buf[..n],
            ),
            Err(error) => {
                update_status(
                    app,
                    &source.id,
                    generation,
                    Some("error"),
                    Some(format!("UDP receive failed: {error}")),
                    false,
                    None,
                );
            }
        }
    }
}

enum SerialMessage {
    Opened,
    Data(Vec<u8>),
    Failed(String),
}

async fn run_serial(
    app: &tauri::AppHandle,
    source: &SourceConfig,
    generation: u64,
    port_name: String,
    baud_rate: u32,
    stop: &mut watch::Receiver<bool>,
) {
    let mut observation = Observation::default();
    loop {
        if *stop.borrow() {
            break;
        }
        update_status(
            app,
            &source.id,
            generation,
            Some("connecting"),
            Some(format!("Opening {port_name} at {baud_rate} baud")),
            false,
            None,
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let worker_port = port_name.clone();
        let worker = tokio::task::spawn_blocking(move || {
            let mut port = match serialport::new(&worker_port, baud_rate)
                .timeout(Duration::from_millis(250))
                .open()
            {
                Ok(port) => port,
                Err(error) => {
                    let _ = tx.send(SerialMessage::Failed(format!(
                        "Could not open {worker_port}: {error}"
                    )));
                    return;
                }
            };
            let _ = tx.send(SerialMessage::Opened);
            let mut buf = [0u8; 2048];
            while !worker_cancelled.load(Ordering::Relaxed) {
                match port.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        if tx.send(SerialMessage::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(error) => {
                        let _ = tx.send(SerialMessage::Failed(format!(
                            "Read failed on {worker_port}: {error}"
                        )));
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
        if let Err(error) = worker.await {
            update_status(
                app,
                &source.id,
                generation,
                Some("error"),
                Some(format!("Serial reader failed: {error}")),
                false,
                None,
            );
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
    let statuses = STATUSES
        .lock()
        .map_err(|error| format!("NMEA status lock failed: {error}"))?;
    Ok(serde_json::json!({
        "sources": settings.nmea_sources,
        "statuses": statuses.values().collect::<Vec<_>>(),
    }))
}

#[tauri::command]
pub fn nmea_sources_set(app: tauri::AppHandle, sources: Vec<SourceConfig>) -> Result<(), String> {
    let mut settings = crate::settings::settings_get(app.clone())?;
    settings.nmea_sources = sources;
    crate::settings::settings_save(app.clone(), settings.clone())?;
    start_sources(&app, &settings.nmea_sources);
    Ok(())
}

pub fn start(app: &tauri::AppHandle) {
    match crate::settings::settings_get(app.clone()) {
        Ok(settings) => start_sources(app, &settings.nmea_sources),
        Err(error) => eprintln!("Could not load saved NMEA sources: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_configuration_uses_explicit_transport_kinds() {
        let source: SourceConfig = serde_json::from_value(serde_json::json!({
            "id": "gps-main",
            "name": "Main GPS",
            "enabled": true,
            "transport": { "type": "serial", "port": "COM4", "baudRate": 4800 }
        }))
        .unwrap();
        assert_eq!(
            source.transport,
            SourceTransport::Serial {
                port: "COM4".into(),
                baud_rate: 4800
            }
        );
    }
}
