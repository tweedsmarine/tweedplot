//! METAR (and the buoy / C-MAN reports in the same feed) from
//! aviationweather.gov's data API, JSON format.

use crate::{get_text, in_bbox, Obs, Result};
use serde_json::Value;

const BASE: &str = "https://aviationweather.gov/api/data/metar";

/// Reports in `bbox` [w, s, e, n] over the last `hours`.
pub fn fetch(bbox: [f64; 4], hours: u32) -> Result<Vec<Obs>> {
    // the API takes minLat,minLon,maxLat,maxLon; split at the date line
    let mut boxes = vec![];
    if bbox[2] > 180.0 {
        boxes.push([bbox[0], bbox[1], 180.0, bbox[3]]);
        boxes.push([-180.0, bbox[1], bbox[2] - 360.0, bbox[3]]);
    } else {
        boxes.push(bbox);
    }
    let mut out = Vec::new();
    for b in boxes {
        let url = format!("{BASE}?bbox={:.3},{:.3},{:.3},{:.3}&format=json&hours={hours}", b[1], b[0], b[3], b[2]);
        out.extend(parse(&get_text(&url)?)?.into_iter().filter(|o| in_bbox(o.lat, o.lon, bbox)));
    }
    Ok(out)
}

/// One station's reports over the last `hours`.
pub fn history(icao: &str, hours: u32) -> Result<Vec<Obs>> {
    let url = format!("{BASE}?ids={icao}&format=json&hours={hours}");
    parse(&get_text(&url)?)
}

pub fn parse(text: &str) -> Result<Vec<Obs>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| crate::Error::Format(format!("METAR JSON: {e}")))?;
    let mut out = Vec::new();
    for m in v.as_array().into_iter().flatten() {
        let (Some(id), Some(lat), Some(lon), Some(t)) = (m["icaoId"].as_str(), m["lat"].as_f64(), m["lon"].as_f64(), m["obsTime"].as_i64()) else { continue };
        let f = |k: &str| m[k].as_f64();
        let vis = match &m["visib"] {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim_end_matches('+').parse::<f64>().ok(),
            _ => None,
        };
        let kind = match m["metarType"].as_str() {
            Some("BUOY") => "buoy",
            Some("CMAN") => "station",
            Some("SYNOP") => "station",
            _ => "airport",
        };
        out.push(Obs {
            id: format!("metar:{id}"),
            source: "metar",
            name: m["name"].as_str().map(|s| s.split(',').next().unwrap_or(s).trim().to_string()).unwrap_or_else(|| id.to_string()),
            lat,
            lon,
            t,
            wind_dir: m["wdir"].as_f64(), // "VRB" stays None
            wind_kn: f("wspd"),
            gust_kn: f("wgst"),
            pressure_hpa: f("slp").or(f("altim")),
            temp_c: f("temp"),
            dewpoint_c: f("dewp"),
            vis_km: vis.map(|sm| sm * 1.609),
            raw: m["rawOb"].as_str().map(str::to_string),
            kind,
            ..Default::default()
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metar_json() {
        let j = r#"[{"metar_id":1,"icaoId":"NZNS","receiptTime":"2026-10-19 09:03:12","obsTime":1792400400,"reportTime":"2026-10-19 09:00:00","temp":14,"dewp":8,"wdir":200,"wspd":15,"wgst":25,"visib":"6+","altim":1012,"slp":null,"qcField":2,"metarType":"METAR","rawOb":"METAR NZNS 190900Z AUTO 20015G25KT 9999 NCD 14/08 Q1012","lat":-41.298,"lon":173.221,"elev":5,"name":"Nelson Intl, SI, NZ","clouds":[]},
                   {"icaoId":"NZWN","obsTime":1792400400,"temp":13,"wdir":"VRB","wspd":3,"wgst":null,"visib":10,"altim":1011,"lat":-41.327,"lon":174.805,"name":"Wellington Intl, NI, NZ"}]"#;
        let o = parse(j).unwrap();
        assert_eq!(o.len(), 2);
        assert_eq!((o[0].id.as_str(), o[0].name.as_str(), o[0].wind_dir, o[0].wind_kn, o[0].gust_kn, o[0].pressure_hpa), ("metar:NZNS", "Nelson Intl", Some(200.0), Some(15.0), Some(25.0), Some(1012.0)));
        assert!((o[0].vis_km.unwrap() - 9.65).abs() < 0.01);
        assert_eq!(o[1].wind_dir, None);
        assert!(parse("").unwrap().is_empty());
    }
}
