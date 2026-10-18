//! Synoptic observations from a WIS2 node's OGC API (wis2box / pygeoapi), here
//! MetService's: one feature per observed value (wind speed, direction,
//! pressure …) with the station, time, value and units.

use crate::{get_text, Obs, Result, MS_TO_KN};
use serde_json::Value;
use std::collections::BTreeMap;

pub const METSERVICE: &str = "https://wis2.metservice.com/oapi";
pub const METSERVICE_SYNOP: &str = "urn:wmo:md:nz-metservice:surface-based-observations.synop";

/// Station names and positions (`collections/stations`), by WIGOS id. Empty if the node has none.
pub fn stations(base: &str) -> BTreeMap<String, (String, f64, f64)> {
    let mut m = BTreeMap::new();
    let Ok(text) = get_text(&format!("{base}/collections/stations/items?f=json&limit=2000")) else { return m };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return m };
    for f in v["features"].as_array().into_iter().flatten() {
        let p = &f["properties"];
        let Some(id) = p["wigos_station_identifier"].as_str().or(f["id"].as_str()) else { continue };
        let c = &f["geometry"]["coordinates"];
        let (Some(lon), Some(lat)) = (c[0].as_f64(), c[1].as_f64()) else { continue };
        let name = p["name"].as_str().or(p["station_name"].as_str()).unwrap_or(id).to_string();
        m.insert(id.to_string(), (title_case(&name), lat, lon));
    }
    m
}

fn title_case(s: &str) -> String {
    if s.chars().any(|c| c.is_lowercase()) {
        return s.to_string();
    }
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every observation in `coll` since `since` (Unix s), optionally for one station; follows `next` links (at most `pages`).
pub fn fetch(base: &str, coll: &str, since: i64, station: Option<&str>, pages: usize) -> Result<Vec<Obs>> {
    let start = chrono::DateTime::from_timestamp(since, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ");
    let mut url = format!("{base}/collections/{coll}/items?f=json&limit=10000&datetime={start}/..");
    if let Some(s) = station {
        url.push_str(&format!("&wigos_station_identifier={s}"));
    }
    let names = stations(base);
    let mut feats = Vec::new();
    for _ in 0..pages.max(1) {
        let text = get_text(&url)?;
        let v: Value = serde_json::from_str(&text).map_err(|e| crate::Error::Format(format!("WIS2 JSON: {e}")))?;
        feats.extend(v["features"].as_array().cloned().unwrap_or_default());
        let next = v["links"].as_array().and_then(|l| l.iter().find(|x| x["rel"] == "next")).and_then(|x| x["href"].as_str()).map(str::to_string);
        match next {
            Some(n) if !n.is_empty() => url = n,
            _ => break,
        }
    }
    Ok(group(&feats, &names))
}

/// The quantity a feature holds, from its `name` (text or a BUFR descriptor).
fn quantity(name: &str, fxxyyy: Option<&str>) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    let code = fxxyyy.unwrap_or(if n.chars().all(|c| c.is_ascii_digit()) { name } else { "" });
    Some(match (n.as_str(), code) {
        (_, "011001") | ("wind_direction", _) => "wdir",
        (_, "011002") | ("wind_speed", _) => "wspd",
        (_, "011041") | ("maximum_wind_gust_speed", _) | ("wind_gust", _) => "gust",
        (_, "010051") | ("pressure_reduced_to_mean_sea_level", _) => "msl",
        (_, "010004") | ("non_coordinate_pressure", _) | ("pressure", _) => "pres",
        (_, "012101") | (_, "012001") | ("air_temperature", _) => "temp",
        (_, "012103") | (_, "012003") | ("dewpoint_temperature", _) => "dew",
        (_, "020001") | ("horizontal_visibility", _) => "vis",
        _ => return None,
    })
}

/// Group per-value features into one observation per station and time.
pub fn group(feats: &[Value], names: &BTreeMap<String, (String, f64, f64)>) -> Vec<Obs> {
    let mut by: BTreeMap<(String, i64), Obs> = BTreeMap::new();
    for f in feats {
        let p = &f["properties"];
        let Some(st) = p["wigos_station_identifier"].as_str().or(p["station_identifier"].as_str()) else { continue };
        let Some(t) = p["phenomenonTime"].as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s.split('/').next_back().unwrap_or(s)).ok()).map(|d| d.timestamp()) else { continue };
        let Some(name) = p["name"].as_str() else { continue };
        let Some(q) = quantity(name, p["fxxyyy"].as_str()) else { continue };
        let Some(mut val) = p["value"].as_f64().or_else(|| p["value"].as_str().and_then(|s| s.parse().ok())) else { continue };
        let units = p["units"].as_str().unwrap_or("");
        let c = &f["geometry"]["coordinates"];
        let o = by.entry((st.to_string(), t)).or_insert_with(|| {
            let (nm, la, lo) = names.get(st).cloned().unwrap_or_else(|| (st.rsplit('-').next().unwrap_or(st).to_string(), c[1].as_f64().unwrap_or(f64::NAN), c[0].as_f64().unwrap_or(f64::NAN)));
            Obs { id: format!("wis2:{st}"), source: "metservice", name: nm, lat: la, lon: lo, t, kind: "station", ..Default::default() }
        });
        if o.lat.is_nan() {
            if let (Some(lo), Some(la)) = (c[0].as_f64(), c[1].as_f64()) {
                o.lat = la;
                o.lon = lo;
            }
        }
        match q {
            "wdir" => o.wind_dir = Some(val),
            "wspd" | "gust" => {
                if units.eq_ignore_ascii_case("m/s") || units == "m s-1" || units.is_empty() {
                    val *= MS_TO_KN;
                } else if units.eq_ignore_ascii_case("km/h") {
                    val /= 1.852;
                }
                if q == "wspd" {
                    o.wind_kn = Some(val);
                } else {
                    // the strongest of the gust periods reported
                    o.gust_kn = Some(o.gust_kn.map_or(val, |g| g.max(val)));
                }
            }
            "msl" | "pres" => {
                let hpa = if units == "Pa" || val > 2000.0 { val / 100.0 } else { val };
                if q == "msl" || o.pressure_hpa.is_none() {
                    o.pressure_hpa = Some(hpa);
                }
            }
            "temp" | "dew" => {
                let cdeg = if units == "K" || val > 150.0 { val - 273.15 } else { val };
                if q == "temp" {
                    o.temp_c = Some(cdeg);
                } else {
                    o.dewpoint_c = Some(cdeg);
                }
            }
            "vis" => o.vis_km = Some(if units == "m" || val > 100.0 { val / 1000.0 } else { val }),
            _ => {}
        }
    }
    by.into_values().filter(|o| !o.lat.is_nan()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn groups_wis2box_features() {
        let f = |st: &str, name: &str, v: f64, u: &str, t: &str| json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [172.67, -40.55, 2.0]},
            "properties": {"wigos_station_identifier": st, "phenomenonTime": t, "resultTime": t, "name": name, "value": v, "units": u}});
        let feats = vec![
            f("0-20000-0-93292", "wind_direction", 250.0, "deg", "2026-10-19T09:00:00Z"),
            f("0-20000-0-93292", "wind_speed", 10.0, "m/s", "2026-10-19T09:00:00Z"),
            f("0-20000-0-93292", "maximum_wind_gust_speed", 14.0, "m/s", "2026-10-19T09:00:00Z"),
            f("0-20000-0-93292", "pressure_reduced_to_mean_sea_level", 101250.0, "Pa", "2026-10-19T09:00:00Z"),
            f("0-20000-0-93292", "air_temperature", 287.15, "K", "2026-10-19T09:00:00Z"),
            f("0-20000-0-93292", "wind_speed", 8.0, "m/s", "2026-10-19T08:00:00Z"),
            // a node that names values by BUFR descriptor and the station differently
            json!({"geometry": {"coordinates": [174.0, -41.0]}, "properties": {"station_identifier": "0-20000-0-93420", "phenomenonTime": "2026-10-19T09:00:00Z", "name": "011002", "value": 5.0, "units": "m/s"}}),
        ];
        let mut names = BTreeMap::new();
        names.insert("0-20000-0-93292".to_string(), ("FAREWELL SPIT AWS".to_string(), -40.55, 173.0));
        let o = group(&feats, &names);
        assert_eq!(o.len(), 3);
        let latest = crate::latest(o.clone());
        let fs = latest.iter().find(|x| x.id == "wis2:0-20000-0-93292").unwrap();
        assert_eq!((fs.name.as_str(), fs.t, fs.wind_dir), ("FAREWELL SPIT AWS", 1792400400, Some(250.0)));
        assert!((fs.wind_kn.unwrap() - 19.44).abs() < 0.01 && (fs.gust_kn.unwrap() - 27.2).abs() < 0.05);
        assert!((fs.pressure_hpa.unwrap() - 1012.5).abs() < 1e-9 && (fs.temp_c.unwrap() - 14.0).abs() < 1e-9);
        let other = latest.iter().find(|x| x.id == "wis2:0-20000-0-93420").unwrap();
        assert!((other.wind_kn.unwrap() - 9.72).abs() < 0.01);
        assert_eq!(title_case("FAREWELL SPIT AWS"), "Farewell Spit Aws");
        assert!(crate::in_bbox(-41.0, -179.5, [170.0, -45.0, 185.0, -35.0]));
    }
}
