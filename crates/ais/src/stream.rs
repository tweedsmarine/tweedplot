//! Online AIS from shore stations through [aisstream.io](https://aisstream.io):
//! a WebSocket stream of decoded AIS messages for the areas asked for. A free
//! API key comes from signing in at aisstream.io with a GitHub account.
//!
//! Coverage depends on volunteer receiving stations near the area.

use crate::{Kind, Update};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

pub const URL: &str = "wss://stream.aisstream.io/v0/stream";

/// What to ask for: the API key and the areas, each [[lat, lon], [lat, lon]].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub key: String,
    pub boxes: Vec<[[f64; 2]; 2]>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Status { connected: bool, detail: String },
    Update(Update),
}

fn subscription(r: &Request) -> String {
    json!({
        "APIKey": r.key,
        "BoundingBoxes": r.boxes,
        "FilterMessageTypes": [
            "PositionReport", "StandardClassBPositionReport", "ExtendedClassBPositionReport", "ShipStaticData",
            "StaticDataReport", "AidsToNavigationReport", "StandardSearchAndRescueAircraftReport", "BaseStationReport"
        ]
    })
    .to_string()
}

/// Run until `tx` closes: (re)connect while a key and an area are set, send
/// each decoded message, resubscribe when the request changes.
pub async fn run(mut req: watch::Receiver<Request>, tx: mpsc::Sender<Event>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let r = req.borrow_and_update().clone();
        if r.key.trim().is_empty() || r.boxes.is_empty() {
            if req.changed().await.is_err() {
                return;
            }
            continue;
        }
        match tokio_tungstenite::connect_async(URL).await {
            Ok((mut ws, _)) => {
                if ws.send(Message::text(subscription(&r))).await.is_err() {
                    continue;
                }
                let mut said_up = false;
                let mut last_sub = tokio::time::Instant::now();
                loop {
                    tokio::select! {
                        m = ws.next() => {
                            let text = match m {
                                Some(Ok(Message::Text(t))) => t.to_string(),
                                Some(Ok(Message::Binary(b))) => String::from_utf8_lossy(&b).into_owned(),
                                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                                Some(Ok(Message::Close(c))) => {
                                    let why = c.map(|c| c.reason.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "closed by aisstream.io".into());
                                    if tx.send(Event::Status { connected: false, detail: why }).await.is_err() { return; }
                                    break;
                                }
                                Some(Err(e)) => {
                                    if tx.send(Event::Status { connected: false, detail: e.to_string() }).await.is_err() { return; }
                                    break;
                                }
                                None => break,
                            };
                            if let Some(err) = error_of(&text) {
                                if tx.send(Event::Status { connected: false, detail: err }).await.is_err() { return; }
                                break;
                            }
                            if !said_up {
                                said_up = true;
                                backoff = Duration::from_secs(2);
                                if tx.send(Event::Status { connected: true, detail: "aisstream.io".into() }).await.is_err() { return; }
                            }
                            if let Some(u) = parse(&text) {
                                if tx.send(Event::Update(u)).await.is_err() { return; }
                            }
                        }
                        ch = req.changed() => {
                            if ch.is_err() { return; }
                            let n = req.borrow_and_update().clone();
                            if n.key != r.key || n.key.trim().is_empty() || n.boxes.is_empty() {
                                let _ = ws.close(None).await;
                                break; // reconnect with the new key (or idle)
                            }
                            // at most one subscription update a second
                            let wait = Duration::from_millis(1100).saturating_sub(last_sub.elapsed());
                            tokio::time::sleep(wait).await;
                            let n = req.borrow_and_update().clone();
                            last_sub = tokio::time::Instant::now();
                            if ws.send(Message::text(subscription(&n))).await.is_err() { break; }
                        }
                    }
                }
            }
            Err(e) => {
                if tx.send(Event::Status { connected: false, detail: format!("aisstream.io: {e}") }).await.is_err() {
                    return;
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

fn error_of(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    v.get("error").and_then(Value::as_str).map(|e| format!("aisstream.io: {e}"))
}

fn clean(s: Option<&str>) -> Option<String> {
    let t = s?.trim_end_matches('@').trim().trim_end_matches('@').trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// Decode one aisstream.io message.
pub fn parse(text: &str) -> Option<Update> {
    let v: Value = serde_json::from_str(text).ok()?;
    let ty = v.get("MessageType")?.as_str()?;
    let m = v.get("Message")?.get(ty)?;
    let meta = v.get("MetaData");
    let mmsi = m
        .get("UserID")
        .and_then(Value::as_u64)
        .or_else(|| meta.and_then(|x| x.get("MMSI")).and_then(Value::as_u64))? as u32;
    let f = |k: &str| m.get(k).and_then(Value::as_f64);
    let mut u = Update { mmsi, source: "aisstream", ..Default::default() };
    let pos = || Some((f("Latitude")?, f("Longitude")?));
    let motion = |u: &mut Update| {
        u.pos = pos();
        u.cog = f("Cog").filter(|c| (0.0..360.0).contains(c));
        u.sog = f("Sog").filter(|s| *s < 102.2);
        u.heading = f("TrueHeading").filter(|h| (0.0..360.0).contains(h));
    };
    let dims = |u: &mut Update, d: Option<&Value>| {
        if let Some(d) = d {
            let g = |k: &str| d.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let (l, b) = (g("A") + g("B"), g("C") + g("D"));
            if l > 0.0 {
                u.length = Some(l);
                // A: the antenna's distance from the bow; C: from the port side
                u.from_bow = Some(g("A"));
            }
            if b > 0.0 {
                u.beam = Some(b);
                u.from_centre = Some(g("C") - b / 2.0);
            }
        }
    };
    match ty {
        "PositionReport" => {
            motion(&mut u);
            u.status = m.get("NavigationalStatus").and_then(Value::as_u64).map(|s| s as u8);
            u.class = Some("A");
        }
        "StandardClassBPositionReport" => {
            motion(&mut u);
            u.class = Some("B");
        }
        "ExtendedClassBPositionReport" => {
            motion(&mut u);
            u.class = Some("B");
            u.name = clean(m.get("Name").and_then(Value::as_str));
            u.ship_type = m.get("Type").and_then(Value::as_u64).filter(|t| *t > 0).map(|t| t as u16);
            dims(&mut u, m.get("Dimension"));
        }
        "ShipStaticData" => {
            u.class = Some("A");
            u.name = clean(m.get("Name").and_then(Value::as_str));
            u.callsign = clean(m.get("CallSign").and_then(Value::as_str));
            u.ship_type = m.get("Type").and_then(Value::as_u64).filter(|t| *t > 0).map(|t| t as u16);
            u.destination = clean(m.get("Destination").and_then(Value::as_str));
            u.draught = f("MaximumStaticDraught").filter(|d| *d > 0.0);
            dims(&mut u, m.get("Dimension"));
        }
        "StaticDataReport" => {
            u.class = Some("B");
            if let Some(a) = m.get("ReportA").filter(|a| a.get("Valid").and_then(Value::as_bool).unwrap_or(false)) {
                u.name = clean(a.get("Name").and_then(Value::as_str));
            }
            if let Some(b) = m.get("ReportB").filter(|b| b.get("Valid").and_then(Value::as_bool).unwrap_or(false)) {
                u.ship_type = b.get("ShipType").and_then(Value::as_u64).filter(|t| *t > 0).map(|t| t as u16);
                u.callsign = clean(b.get("CallSign").and_then(Value::as_str));
                dims(&mut u, b.get("Dimension"));
            }
        }
        "AidsToNavigationReport" => {
            u.kind = Some(Kind::Aton);
            u.pos = pos();
            let name = format!("{}{}", m.get("Name").and_then(Value::as_str).unwrap_or(""), m.get("NameExtension").and_then(Value::as_str).unwrap_or(""));
            u.name = clean(Some(&name));
            u.aton_type = m.get("Type").and_then(Value::as_u64).map(|t| t as u8);
            u.virtual_aton = m.get("VirtualAtoN").and_then(Value::as_bool);
            u.off_position = m.get("OffPosition").and_then(Value::as_bool);
        }
        "StandardSearchAndRescueAircraftReport" => {
            u.kind = Some(Kind::Aircraft);
            u.pos = pos();
            u.cog = f("Cog").filter(|c| (0.0..360.0).contains(c));
            u.sog = f("Sog").filter(|s| *s < 1022.0);
            u.altitude = f("Altitude").filter(|a| *a < 4095.0);
        }
        "BaseStationReport" => {
            u.kind = Some(Kind::Base);
            u.pos = pos();
        }
        _ => return None,
    }
    if u.name.is_none() {
        u.name = clean(meta.and_then(|x| x.get("ShipName")).and_then(Value::as_str));
    }
    Some(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_position_static_and_atons() {
        let p = r#"{"Message":{"PositionReport":{"Cog":308,"CommunicationState":81982,"Latitude":-41.2575,"Longitude":173.2801,"MessageID":1,"NavigationalStatus":0,"PositionAccuracy":true,"Raim":true,"RateOfTurn":0,"RepeatIndicator":0,"Sog":11.3,"Spare":0,"SpecialManoeuvreIndicator":0,"Timestamp":31,"TrueHeading":307,"UserID":512000123,"Valid":true}},"MessageType":"PositionReport","MetaData":{"MMSI":512000123,"MMSI_String":512000123,"ShipName":"KAITAKI     ","latitude":-41.2575,"longitude":173.2801,"time_utc":"2026-10-19 09:12:01.1 +0000 UTC"}}"#;
        let u = parse(p).unwrap();
        assert_eq!((u.mmsi, u.pos, u.cog, u.sog, u.heading, u.status, u.class), (512000123, Some((-41.2575, 173.2801)), Some(308.0), Some(11.3), Some(307.0), Some(0), Some("A")));
        assert_eq!(u.name.as_deref(), Some("KAITAKI"));
        let s = r#"{"Message":{"ShipStaticData":{"AisVersion":2,"CallSign":"ZMA1234","Destination":"NELSON@@@@","Dimension":{"A":20,"B":10,"C":4,"D":4},"Dte":false,"Eta":{"Day":19,"Hour":12,"Minute":0,"Month":10},"FixType":1,"ImoNumber":0,"MaximumStaticDraught":3.2,"MessageID":5,"Name":"AMALTAL APOLLO@@@@@@","RepeatIndicator":0,"Spare":false,"Type":30,"UserID":512000123,"Valid":true}},"MessageType":"ShipStaticData","MetaData":{"MMSI":512000123}}"#;
        let u = parse(s).unwrap();
        assert_eq!((u.name.as_deref(), u.callsign.as_deref(), u.destination.as_deref(), u.ship_type, u.length, u.beam, u.draught), (Some("AMALTAL APOLLO"), Some("ZMA1234"), Some("NELSON"), Some(30), Some(30.0), Some(8.0), Some(3.2)));
        assert_eq!((u.from_bow, u.from_centre), (Some(20.0), Some(0.0)));
        let a = r#"{"Message":{"AidsToNavigationReport":{"AssignedMode":false,"AtoN":0,"Dimension":{"A":0,"B":0,"C":0,"D":0},"Fixtype":7,"Latitude":-41.2,"Longitude":173.3,"MessageID":21,"Name":"NELSON WAVE BUOY","NameExtension":"","OffPosition":false,"PositionAccuracy":false,"Raim":false,"RepeatIndicator":0,"Spare":false,"Timestamp":60,"Type":1,"UserID":995121001,"Valid":true,"VirtualAtoN":true}},"MessageType":"AidsToNavigationReport","MetaData":{"MMSI":995121001}}"#;
        let u = parse(a).unwrap();
        assert_eq!((u.kind, u.virtual_aton, u.name.as_deref()), (Some(Kind::Aton), Some(true), Some("NELSON WAVE BUOY")));
        // not available values dropped
        let na = p.replace("\"Cog\":308", "\"Cog\":360").replace("\"TrueHeading\":307", "\"TrueHeading\":511").replace("\"Sog\":11.3", "\"Sog\":102.3");
        let u = parse(&na).unwrap();
        assert_eq!((u.cog, u.heading, u.sog), (None, None, None));
        assert!(parse(r#"{"error":"Api Key Is Not Valid"}"#).is_none());
        assert_eq!(error_of(r#"{"error":"Api Key Is Not Valid"}"#).as_deref(), Some("aisstream.io: Api Key Is Not Valid"));
        assert!(subscription(&Request { key: "k".into(), boxes: vec![[[-42.0, 172.0], [-40.0, 175.0]]] }).contains("[[[-42.0,172.0],[-40.0,175.0]]]"));
    }
}
