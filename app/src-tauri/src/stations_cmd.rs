//! Weather stations and buoys: latest observations for the chart and one
//! station's recent history (see the `stations` crate).

use std::collections::HashMap;
use std::sync::Mutex;

/// (fetched at, observations) per source key; refetched after 10 min.
static CACHE: Mutex<Option<HashMap<String, (i64, Vec<stations::Obs>)>>> = Mutex::new(None);
const FRESH_S: i64 = 600;

fn cached(key: &str, fetch: impl FnOnce() -> stations::Result<Vec<stations::Obs>>) -> stations::Result<Vec<stations::Obs>> {
    let now = chrono::Utc::now().timestamp();
    if let Some((t, v)) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(key) {
        if now - t < FRESH_S {
            return Ok(v.clone());
        }
    }
    let v = fetch()?;
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key.to_string(), (now, v.clone()));
    Ok(v)
}

/// The latest observation of each station in `bbox` [w, s, e, n] from the chosen
/// sources ("metservice", "metar", "ndbc"), and any errors by source.
#[tauri::command]
pub async fn stations_latest(bbox: [f64; 4], sources: Vec<String>, refresh: Option<bool>) -> serde_json::Value {
    tauri::async_runtime::spawn_blocking(move || {
        if refresh.unwrap_or(false) {
            CACHE.lock().unwrap().take();
        }
        let mut obs = Vec::new();
        let mut errors = serde_json::Map::new();
        let now = chrono::Utc::now().timestamp();
        for s in &sources {
            let r = match s.as_str() {
                "metservice" => {
                    cached("metservice", || stations::wis2::fetch(stations::wis2::METSERVICE, stations::wis2::METSERVICE_SYNOP, now - 4 * 3600, None, 4))
                }
                "metar" => {
                    // a box a little larger than asked, rounded so small pans reuse it
                    let r = |x: f64, up: bool| if up { (x / 2.0).ceil() * 2.0 } else { (x / 2.0).floor() * 2.0 };
                    let b = [r(bbox[0], false), r(bbox[1].max(-90.0), false), r(bbox[2], true), r(bbox[3].min(90.0), true)];
                    cached(&format!("metar:{b:?}"), || stations::metar::fetch(b, 3))
                }
                "ndbc" => cached("ndbc", stations::ndbc::fetch_latest),
                _ => continue,
            };
            match r {
                Ok(v) => obs.extend(v.into_iter().filter(|o| stations::in_bbox(o.lat, o.lon, bbox))),
                Err(e) => {
                    errors.insert(s.clone(), e.to_string().into());
                }
            }
        }
        // drop stale reports (12 h)
        let obs: Vec<_> = stations::latest(obs).into_iter().filter(|o| now - o.t < 12 * 3600).collect();
        serde_json::json!({ "obs": obs, "errors": errors })
    })
    .await
    .unwrap_or_else(|e| serde_json::json!({ "obs": [], "errors": { "all": e.to_string() } }))
}

/// One station's observations over the last `hours`, oldest first.
#[tauri::command]
pub async fn station_history(id: String, lat: f64, lon: f64, hours: u32) -> Result<Vec<stations::Obs>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let hours = hours.clamp(1, 72);
        let since = chrono::Utc::now().timestamp() - hours as i64 * 3600;
        let (src, sid) = id.split_once(':').ok_or("bad station id")?;
        let mut v = match src {
            "metar" => stations::metar::history(sid, hours),
            "ndbc" => stations::ndbc::history(sid, lat, lon, hours),
            "wis2" => stations::wis2::fetch(stations::wis2::METSERVICE, stations::wis2::METSERVICE_SYNOP, since, Some(sid), 3),
            _ => return Err(format!("unknown source {src}")),
        }
        .map_err(|e| e.to_string())?;
        v.retain(|o| o.id == id && o.t >= since);
        v.sort_by_key(|o| o.t);
        v.dedup_by_key(|o| o.t);
        Ok(v)
    })
    .await
    .map_err(|e| e.to_string())?
}
