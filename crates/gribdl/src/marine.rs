//! Marine boundaries and fishing rules for New Zealand waters, downloaded for the
//! chart crate to build its layers from (see `chart::marine`).
//!
//! - **Maritime boundaries** (LINZ, Crown copyright: "Sourced from Land Information New
//!   Zealand data. Crown Copyright Reserved."): the shapefile zips of the territorial
//!   sea, contiguous zone, EEZ, the boundaries agreed with Australia and the outlying
//!   islands, from LINZ's maritime boundary downloads page. Saved as they come into
//!   `<dir>/boundaries`.
//! - **Fishing rules** (Ministry for Primary Industries, CC BY 4.0: "Source: Ministry for
//!   Primary Industries"): commercial fishing regulations, customary areas, voluntary
//!   closures, and the fisheries management and statistical areas, from MPI's ArcGIS map
//!   feature services (`FeatureServer/<layer>/query` on geospatial.mpi.govt.nz; the older
//!   maps.mpi.govt.nz is no longer reachable from New Zealand). Saved per service as
//!   `<dir>/mpi/<name>.json`: `{key, service, fetched, layers: [{id, name, features}]}`,
//!   where `features` is a GeoJSON FeatureCollection in longitude and latitude. For the
//!   regulations, the "all regulations" layer comes with its areas and the category
//!   layers (trawl prohibitions, over 46 m, set net...) with just their members'
//!   attributes, to say which categories each regulation is in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use crate::{client, Error, Result};

/// A request error with its causes (reqwest's own message just says "error sending request").
pub(crate) fn why(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(c) = src {
        s.push_str(&format!(": {c}"));
        src = c.source();
    }
    s
}

/// LINZ's maritime boundary zips (shapefiles; the "raw" text versions aren't needed).
pub const BOUNDARY_ZIPS: &[&str] = &[
    "ShpNZ.zip",
    "24CZ.zip",
    "eez_website.zip",
    "ShpAUS-NZ.zip",
    "ShpCHA.zip",
    "ShpAUC.zip",
    "ShpANT.zip",
    "ShpBOU.zip",
    "ShpCAM.zip",
    "ShpTKI.zip",
    "ShpRAO.zip",
    "ShpMAC.zip",
    "ShpCCI.zip",
    "ShpESP.zip",
    "ShpSNA.zip",
];

fn linz_base() -> String {
    std::env::var("RACENAV_LINZ_MB_URL").unwrap_or_else(|_| "https://www.linz.govt.nz/sites/default/files".into())
}

fn mpi_base() -> String {
    std::env::var("RACENAV_MPI_URL").unwrap_or_else(|_| "https://geospatial.mpi.govt.nz/portal/rest/services/MARINE".into())
}

/// Download LINZ's maritime boundary zips into `dir/boundaries`. A zip that fails is
/// skipped (and named in the result); it's an error only when none could be had.
pub fn boundaries(dir: &Path, progress: &(dyn Fn(u64, u64) + Sync), cancel: &AtomicBool) -> Result<(usize, Vec<String>)> {
    let out = dir.join("boundaries");
    std::fs::create_dir_all(&out)?;
    let c = client()?;
    let (mut got, mut failed) = (0, Vec::new());
    for (i, name) in BOUNDARY_ZIPS.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        progress(i as u64, BOUNDARY_ZIPS.len() as u64);
        let url = format!("{}/{name}", linz_base());
        let r = c.get(&url).send().map_err(|e| Error::Http(why(&e))).and_then(|r| {
            if !r.status().is_success() {
                return Err(Error::Http(format!("HTTP {} from {url}", r.status().as_u16())));
            }
            r.bytes().map_err(|e| Error::Http(why(&e)))
        });
        match r {
            Ok(b) if b.starts_with(b"PK") => {
                let part = out.join(format!("{name}.part"));
                std::fs::write(&part, &b)?;
                std::fs::rename(&part, out.join(name))?;
                got += 1;
            }
            Ok(_) => failed.push(format!("{name}: not a zip")),
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    progress(BOUNDARY_ZIPS.len() as u64, BOUNDARY_ZIPS.len() as u64);
    if got == 0 {
        return Err(Error::Http(format!("no maritime boundaries downloaded ({})", failed.join("; "))));
    }
    Ok((got, failed))
}

/// MPI services fetched: (file name, kind of data, service name, the layers wanted: all
/// when empty).
pub const MPI_SERVICES: &[(&str, &str, &str, &[i64])] = &[
    ("regs", "regs", "CommercialFishing", &[]),
    // mātaitai, s 186 temporary closures, taiāpure (not the coordinate points or rohe moana)
    ("customary", "customary", "Marine_Customary_Restrictions", &[1, 6, 8]),
    ("voluntary", "voluntary", "OtherFisheriesManagementAreas", &[1]),
    ("areas_fma", "areas", "Marine_FMAs", &[7]),
    ("areas_stat", "areas", "StatisticalAreas", &[2]),
];

/// Features asked for in one query (their areas can be big).
const BATCH: usize = 50;

fn get_json(c: &reqwest::blocking::Client, url: &str) -> Result<Value> {
    let r = c.get(url).send().map_err(|e| Error::Http(why(&e)))?;
    if !r.status().is_success() {
        return Err(Error::Http(format!("HTTP {} from {url}", r.status().as_u16())));
    }
    let b = r.bytes().map_err(|e| Error::Http(format!("{url}: {e}")))?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| Error::Http(format!("{url}: {e}")))?;
    if let Some(e) = v.get("error") {
        return Err(Error::Http(format!("{url}: {}", e.get("message").and_then(Value::as_str).unwrap_or("error"))));
    }
    Ok(v)
}

/// The leaf layers of a map service: (id, name).
fn service_layers(c: &reqwest::blocking::Client, base: &str) -> Result<Vec<(i64, String)>> {
    let v = get_json(c, &format!("{base}/FeatureServer?f=json"))?;
    let mut out = Vec::new();
    for l in v["layers"].as_array().into_iter().flatten() {
        let group = l["subLayerIds"].as_array().is_some_and(|s| !s.is_empty());
        if let (Some(id), false) = (l["id"].as_i64(), group) {
            out.push((id, l["name"].as_str().unwrap_or("").to_string()));
        }
    }
    Ok(out)
}

fn object_ids(c: &reqwest::blocking::Client, layer_url: &str) -> Result<Vec<i64>> {
    let v = get_json(c, &format!("{layer_url}/query?where=1%3D1&returnIdsOnly=true&f=json"))?;
    let mut ids: Vec<i64> = v["objectIds"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect();
    ids.sort_unstable();
    Ok(ids)
}

/// Esri JSON polygon rings to a GeoJSON geometry: clockwise rings are outer
/// boundaries, anticlockwise ones holes in the outer ring before them.
pub fn esri_to_geojson(g: &Value) -> Value {
    let pts = |r: &Value| -> Vec<[f64; 2]> {
        r.as_array().map(|a| a.iter().filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect()).unwrap_or_default()
    };
    if let Some(rings) = g["rings"].as_array() {
        let mut polys: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
        for r in rings.iter().map(pts).filter(|r| r.len() >= 4) {
            let area: f64 = r.windows(2).map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1]).sum();
            if area < 0.0 || polys.is_empty() {
                polys.push(vec![r]);
            } else {
                polys.last_mut().unwrap().push(r);
            }
        }
        return json!({"type": "MultiPolygon", "coordinates": polys});
    }
    if let Some(paths) = g["paths"].as_array() {
        return json!({"type": "MultiLineString", "coordinates": paths.iter().map(pts).collect::<Vec<_>>()});
    }
    Value::Null
}

/// Every feature of a layer as GeoJSON (in batches by object id); with
/// `geometry` false, just the attributes.
fn layer_features(c: &reqwest::blocking::Client, layer_url: &str, geometry: bool, cancel: &AtomicBool) -> Result<Vec<Value>> {
    let ids = object_ids(c, layer_url)?;
    let mut out = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(if geometry { BATCH } else { 500 }) {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let list = chunk.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        if geometry {
            let q = format!("{layer_url}/query?objectIds={list}&outFields=*&returnGeometry=true&outSR=4326&geometryPrecision=7");
            match get_json(c, &format!("{q}&f=geojson")) {
                Ok(v) if v["features"].is_array() && v["type"] == "FeatureCollection" => out.extend(v["features"].as_array().cloned().unwrap_or_default()),
                // older servers: Esri JSON
                _ => {
                    let v = get_json(c, &format!("{q}&f=json"))?;
                    for f in v["features"].as_array().into_iter().flatten() {
                        out.push(json!({"type": "Feature", "properties": f["attributes"], "geometry": esri_to_geojson(&f["geometry"])}));
                    }
                }
            }
        } else {
            let v = get_json(c, &format!("{layer_url}/query?objectIds={list}&outFields=*&returnGeometry=false&f=json"))?;
            out.extend(v["features"].as_array().into_iter().flatten().map(|f| f["attributes"].clone()));
        }
    }
    Ok(out)
}

/// What one MPI download fetched.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct MpiSummary {
    pub files: Vec<String>,
    /// Areas downloaded, over all services.
    pub features: usize,
    /// Services that failed, with why.
    pub failed: Vec<String>,
}

/// Download MPI's fishing rules into `dir/mpi`. `progress(stage, done, total)` is
/// told the service and how many of its layers are done. A service that fails keeps its
/// last download; it's an error only when nothing could be had.
pub fn mpi(dir: &Path, progress: &(dyn Fn(&str, u64, u64) + Sync), cancel: &AtomicBool) -> Result<MpiSummary> {
    let out_dir = dir.join("mpi");
    std::fs::create_dir_all(&out_dir)?;
    let c = client()?;
    let mut sum = MpiSummary::default();
    for (file, key, service, wanted) in MPI_SERVICES {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match mpi_service(&c, file, key, service, wanted, &out_dir, progress, cancel) {
            Ok((path, n)) => {
                sum.files.push(path.to_string_lossy().into_owned());
                sum.features += n;
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => sum.failed.push(format!("{service}: {e}")),
        }
    }
    if sum.files.is_empty() {
        return Err(Error::Http(format!("no fishing rules downloaded ({})", sum.failed.join("; "))));
    }
    Ok(sum)
}

fn mpi_service(
    c: &reqwest::blocking::Client,
    file: &str,
    key: &str,
    service: &str,
    wanted: &[i64],
    out_dir: &Path,
    progress: &(dyn Fn(&str, u64, u64) + Sync),
    cancel: &AtomicBool,
) -> Result<(PathBuf, usize)> {
    let base = format!("{}/{service}", mpi_base());
    let layers: Vec<(i64, String)> = service_layers(c, &base)?.into_iter().filter(|(id, _)| wanted.is_empty() || wanted.contains(id)).collect();
    // the regulations: the "all regulations" layer with its areas, the others just their members
    let all_layer = (key == "regs").then(|| layers.iter().find(|(id, _)| *id == 0).map(|l| l.0)).flatten();
    let mut out_layers = Vec::new();
    let mut n = 0;
    for (k, (id, name)) in layers.iter().enumerate() {
        progress(service, k as u64, layers.len() as u64);
        let url = format!("{base}/FeatureServer/{id}");
        let geometry = all_layer.is_none_or(|a| a == *id);
        let feats = layer_features(c, &url, geometry, cancel)?;
        if geometry {
            n += feats.len();
            out_layers.push(json!({"id": id, "name": name, "features": {"type": "FeatureCollection", "features": feats}}));
        } else {
            out_layers.push(json!({"id": id, "name": name, "members": feats}));
        }
    }
    progress(service, layers.len() as u64, layers.len() as u64);
    let doc = json!({"key": key, "service": service, "fetched": chrono::Utc::now().to_rfc3339(), "layers": out_layers});
    let path = out_dir.join(format!("{file}.json"));
    let part = out_dir.join(format!("{file}.json.part"));
    std::fs::write(&part, doc.to_string())?;
    std::fs::rename(&part, &path)?;
    Ok((path, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// A tiny HTTP server answering from `routes` (the first whose key the request path
    /// contains), for as long as the test runs.
    fn serve(routes: Vec<(&'static str, String)>) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap() <= 2 {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = routes.iter().find(|(k, _)| path.contains(k)).map(|(_, b)| b.clone());
                let (status, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", "{}".into()),
                };
                let mut s = s;
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn esri_rings_become_polygons_with_holes() {
        let g = json!({"rings": [
            [[0.0, 0.0], [0.0, 4.0], [4.0, 4.0], [4.0, 0.0], [0.0, 0.0]],   // clockwise: outer
            [[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0], [1.0, 1.0]],   // anticlockwise: hole
            [[10.0, 0.0], [10.0, 1.0], [11.0, 1.0], [11.0, 0.0], [10.0, 0.0]]
        ]});
        let v = esri_to_geojson(&g);
        assert_eq!(v["type"], "MultiPolygon");
        let c = v["coordinates"].as_array().unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].as_array().unwrap().len(), 2);
    }

    #[test]
    fn downloads_regulations_and_customary_areas() {
        let square = |x: f64| json!({"type": "Polygon", "coordinates": [[[x, -41.0], [x + 0.1, -41.0], [x + 0.1, -40.9], [x, -40.9], [x, -41.0]]]});
        let regs_layers = json!({"layers": [
            {"id": 0, "name": "All Commercial Fishing Regulations", "subLayerIds": null},
            {"id": 17, "name": "Trawl prohibitions", "subLayerIds": null},
            {"id": 30, "name": "A group", "subLayerIds": [17]}
        ]});
        let all = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {"OBJECTID": 1, "Name": "A", "Identifier": "R1", "BottomTrawl": 1}, "geometry": square(173.0)},
            {"type": "Feature", "properties": {"OBJECTID": 2, "Name": "B", "Identifier": "R2", "SetNet": 1}, "geometry": square(174.0)}
        ]});
        let members = json!({"features": [{"attributes": {"OBJECTID": 7, "Name": "A", "Identifier": "R1"}}]});
        // customary: a server that only does Esri JSON
        let cust_layers = json!({"layers": [{"id": 1, "name": "Mataitai Reserves"}, {"id": 3, "name": "Mataitai Reserve Coordinates"}]});
        let cust = json!({"features": [{"attributes": {"OBJECTID": 5, "Name": "M"}, "geometry": {"rings": [[[173.0, -41.0], [173.0, -40.9], [173.1, -40.9], [173.0, -41.0]]]}}]});
        let base = serve(vec![
            ("CommercialFishing/FeatureServer?f=json", regs_layers.to_string()),
            ("CommercialFishing/FeatureServer/0/query?where", json!({"objectIds": [2, 1]}).to_string()),
            ("CommercialFishing/FeatureServer/0/query?objectIds=1,2", all.to_string()),
            ("CommercialFishing/FeatureServer/17/query?where", json!({"objectIds": [7]}).to_string()),
            ("CommercialFishing/FeatureServer/17/query?objectIds=7", members.to_string()),
            ("Customary_Restrictions/FeatureServer?f=json", cust_layers.to_string()),
            ("Customary_Restrictions/FeatureServer/1/query?where", json!({"objectIds": [5]}).to_string()),
            (
                "Customary_Restrictions/FeatureServer/1/query?objectIds=5&outFields=*&returnGeometry=true&outSR=4326&geometryPrecision=7&f=geojson",
                json!({"error": {"message": "no geojson"}}).to_string(),
            ),
            (
                "Customary_Restrictions/FeatureServer/1/query?objectIds=5&outFields=*&returnGeometry=true&outSR=4326&geometryPrecision=7&f=json",
                cust.to_string(),
            ),
        ]);
        std::env::set_var("RACENAV_MPI_URL", &base);
        let dir = std::env::temp_dir().join(format!("tp_mpi_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sum = mpi(&dir, &|_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(sum.files.len(), 2, "{sum:?}");
        assert_eq!(sum.features, 3);
        assert_eq!(sum.failed.len(), 3, "the services the fake server doesn't have");
        let regs: Value = serde_json::from_slice(&std::fs::read(dir.join("mpi/regs.json")).unwrap()).unwrap();
        assert_eq!(regs["key"], "regs");
        let ls = regs["layers"].as_array().unwrap();
        assert_eq!(ls.len(), 2, "the group layer is skipped");
        assert_eq!(ls[0]["features"]["features"].as_array().unwrap().len(), 2);
        assert_eq!(ls[1]["members"][0]["Identifier"], "R1");
        let cust: Value = serde_json::from_slice(&std::fs::read(dir.join("mpi/customary.json")).unwrap()).unwrap();
        assert_eq!(cust["layers"][0]["features"]["features"][0]["geometry"]["type"], "MultiPolygon");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
