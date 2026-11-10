//! The LINZ Data Service: the list of depth models (catalogue API, no key needed), their
//! export and download (needs an API key), and the survey index (WFS, needs the key).

use chart::depthpack::Survey;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const API: &str = "https://data.linz.govt.nz/services/api/v1.x";
/// NZ Bathymetric Surface Model Index: the survey outlines.
const INDEX_LAYER: i64 = 122710;

/// A depth model as listed: the survey and the version LINZ publishes now.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Model {
    pub survey: Survey,
    /// LDS version id: a new one means new data.
    pub version: i64,
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent("tweedplot-depthpack (TweedPlot chart plotter)")
        .timeout(Duration::from_secs(600))
        .build()
        .expect("http client")
}

fn get_json(c: &reqwest::blocking::Client, url: &str, key: Option<&str>) -> Result<Value, String> {
    let mut rq = c.get(url);
    if let Some(k) = key {
        rq = rq.header("Authorization", format!("key {k}"));
    }
    let r = rq.send().map_err(|e| format!("{url}: {e}"))?;
    let status = r.status();
    let text = r.text().map_err(|e| format!("{url}: {e}"))?;
    if !status.is_success() {
        return Err(format!("{url}: {status}: {}", text.chars().take(300).collect::<String>()));
    }
    serde_json::from_str(&text).map_err(|e| format!("{url}: {e}"))
}

/// The year in a title such as "Tasman Bay … Depth Model (2022)".
pub fn year_of(title: &str) -> i32 {
    title.rsplit('(').next().and_then(|s| s.trim_end_matches(')').trim().parse().ok()).unwrap_or(0)
}

/// The sentences of a description that say something about a word ("datum", "accuracy").
fn sentences_with(desc: &str, words: &[&str]) -> String {
    desc.split(['\n', '.']).map(str::trim).filter(|s| words.iter().any(|w| s.to_ascii_lowercase().contains(w))).collect::<Vec<_>>().join("; ")
}

/// A layer's details as a model.
pub fn model_of(j: &Value) -> Option<Model> {
    let id = j["id"].as_i64()?;
    let title = j["title"].as_str()?.trim().to_string();
    let desc = j["description"].as_str().unwrap_or("");
    let epsg = j["data"]["crs"]["id"].as_str().and_then(|s| s.strip_prefix("EPSG:")).and_then(|s| s.parse().ok()).unwrap_or(0);
    let datum = {
        let s = sentences_with(desc, &["vertical datum"]);
        let s = s.trim_start_matches("Vertical datum is based on ").trim_start_matches("Vertical datum is ").to_string();
        if s.is_empty() {
            "Approximate Lowest Astronomical Tide".into()
        } else {
            s
        }
    };
    Some(Model {
        survey: Survey {
            id,
            year: year_of(&title),
            title,
            res: j["data"]["raster_resolution"].as_f64().unwrap_or(0.0),
            epsg,
            datum,
            accuracy: sentences_with(desc, &["accuracy"]),
            published: j["published_at"].as_str().unwrap_or("").chars().take(10).collect(),
            outline: j["data"]["extent"].clone(),
        },
        version: j["published_version"]
            .as_str()
            .and_then(|u| u.trim_end_matches('/').rsplit('/').next())
            .and_then(|s| s.parse().ok())
            .or_else(|| j["version"]["id"].as_i64())
            .unwrap_or(0),
    })
}

/// List LINZ's depth models (grid layers titled "… Depth Model …") into D/catalogue.json.
pub fn catalogue(dir: &Path, key: Option<&str>) -> Result<Vec<Model>, String> {
    let c = client();
    let mut ids = std::collections::BTreeSet::new();
    for q in ["Depth Model", "Bathymetric LiDAR", "Multibeam"] {
        let mut url = format!("{API}/data/?q={}&kind=grid&page_size=100", q.replace(' ', "%20"));
        loop {
            let j = get_json(&c, &url, key)?;
            let items = j["results"].as_array().cloned().or_else(|| j.as_array().cloned()).unwrap_or_default();
            for it in items {
                if it["type"] == "layer" && it["title"].as_str().is_some_and(|t| t.to_ascii_lowercase().contains("depth model")) {
                    if let Some(id) = it["id"].as_i64() {
                        ids.insert(id);
                    }
                }
            }
            match j["next"].as_str() {
                Some(n) if !n.is_empty() => url = n.to_string(),
                _ => break,
            }
        }
    }
    let mut out = Vec::new();
    for id in ids {
        let j = get_json(&c, &format!("{API}/layers/{id}/"), key)?;
        if j["license"]["title"].as_str().is_some_and(|l| !l.contains("Creative Commons Attribution 4.0")) {
            eprintln!("skipped {id}: licence {}", j["license"]["title"]);
            continue;
        }
        match model_of(&j) {
            Some(m) => {
                println!("{:>7}  {:>4} m  EPSG:{}  {}", m.survey.id, m.survey.res, m.survey.epsg, m.survey.title);
                out.push(m);
            }
            None => eprintln!("skipped {id}: no details"),
        }
    }
    std::fs::write(dir.join("catalogue.json"), serde_json::to_string_pretty(&out).unwrap()).map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn load_catalogue(dir: &Path) -> Result<Vec<Model>, String> {
    let t = std::fs::read_to_string(dir.join("catalogue.json")).map_err(|_| "no catalogue.json: run `catalogue` first".to_string())?;
    serde_json::from_str(&t).map_err(|e| format!("catalogue.json: {e}"))
}

/// Export every model not yet downloaded (or with a newer version) and unzip it into
/// D/src/<id>/; then the survey index into D/src/index.geojson.
pub fn export_all(dir: &Path, key: &str, only: Option<&[i64]>) -> Result<(), String> {
    let cat = match load_catalogue(dir) {
        Ok(c) => c,
        Err(_) => catalogue(dir, Some(key))?,
    };
    let c = client();
    let mut failed = Vec::new();
    for m in cat.iter().filter(|m| only.is_none_or(|o| o.contains(&m.survey.id))) {
        let sdir = dir.join("src").join(m.survey.id.to_string());
        let stamp = sdir.join("version.txt");
        if std::fs::read_to_string(&stamp).ok().and_then(|s| s.trim().parse::<i64>().ok()) == Some(m.version) {
            println!("{}: up to date", m.survey.title);
            continue;
        }
        println!("{}: exporting…", m.survey.title);
        match export_one(&c, key, m, &sdir) {
            Ok(n) => {
                std::fs::write(&stamp, m.version.to_string()).map_err(|e| e.to_string())?;
                println!("{}: {n} GeoTIFF files", m.survey.title);
            }
            Err(e) => {
                eprintln!("{}: FAILED: {e}", m.survey.title);
                failed.push(m.survey.id);
            }
        }
    }
    match index(&c, key) {
        Ok(g) => std::fs::write(dir.join("src").join("index.geojson"), g.to_string()).map_err(|e| e.to_string())?,
        Err(e) => eprintln!("survey index: {e} (outlines will be the models' extents)"),
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("{} exports failed: {failed:?} (run export again to retry them)", failed.len()))
    }
}

/// One model: an LDS export (GeoTIFF, in its own projection), waited for, downloaded and
/// unzipped. The number of GeoTIFFs.
fn export_one(c: &reqwest::blocking::Client, key: &str, m: &Model, sdir: &Path) -> Result<usize, String> {
    let post = |epsg: &str| -> Result<(reqwest::StatusCode, Value), String> {
        let body = json!({
            "name": format!("tweedplot-depthpack {}", m.survey.id),
            "crs": epsg,
            "items": [{ "item": format!("{API}/layers/{}/", m.survey.id) }],
            "formats": { "grid": "image/tiff;subtype=geotiff" },
        });
        let r = c.post(format!("{API}/exports/")).header("Authorization", format!("key {key}")).json(&body).send().map_err(|e| e.to_string())?;
        let status = r.status();
        let j: Value = r.json().map_err(|e| format!("export reply: {e}"))?;
        Ok((status, j))
    };
    let mut crs = format!("EPSG:{}", m.survey.epsg);
    let (mut status, mut j) = post(&crs)?;
    // LINZ sometimes lists a layer in one UTM zone but exports it only in another (Shipping
    // Lane 6: listed 32760, exported 32759): take the CRS it suggests that TweedPlot reads
    if !status.is_success() {
        if let Some(alt) = alternative_crs(&j) {
            if alt != crs {
                println!("  LINZ won't export it in {crs}: using {alt}");
                crs = alt;
                (status, j) = post(&crs)?;
            }
        }
    }
    if !status.is_success() {
        return Err(format!("export refused ({status}): {}", refusal(&j)));
    }
    let url = j["url"].as_str().ok_or("export reply without url")?.to_string();
    // wait for it: LINZ makes the file, which can take minutes for a big survey
    let started = std::time::Instant::now();
    let download = loop {
        std::thread::sleep(Duration::from_secs(10));
        let s = get_json(c, &url, Some(key))?;
        match s["state"].as_str() {
            Some("complete") => break s["download_url"].as_str().ok_or("complete without download_url")?.to_string(),
            Some("error") | Some("cancelled") | Some("gone") => return Err(format!("export {}: {}", s["state"], s["error"])),
            _ => {
                if let Some(p) = s["progress"].as_f64() {
                    print!("\r  {:.0}%   ", p * 100.0);
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
            }
        }
        if started.elapsed() > Duration::from_secs(6 * 3600) {
            return Err("export still not ready after 6 hours".into());
        }
    };
    println!();
    let _ = std::fs::remove_dir_all(sdir);
    std::fs::create_dir_all(sdir).map_err(|e| e.to_string())?;
    let zip_path = sdir.join("export.zip");
    let mut r = c.get(&download).header("Authorization", format!("key {key}")).send().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("download: {}", r.status()));
    }
    let mut f = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    std::io::copy(&mut r, &mut f).map_err(|e| format!("download: {e}"))?;
    drop(f);
    let n = unzip_tiffs(&zip_path, sdir)?;
    let _ = std::fs::remove_file(&zip_path);
    // the coordinate system it came in (the build's fallback when a file doesn't say)
    std::fs::write(sdir.join("crs.txt"), crs.trim_start_matches("EPSG:")).map_err(|e| e.to_string())?;
    Ok(n)
}

/// From a refused export, a coordinate system LINZ suggests that the build can read: a WGS 84
/// UTM zone, else NZTM2000, else plain longitude and latitude. None unless the refusal is
/// about the coordinate system.
pub fn alternative_crs(j: &Value) -> Option<String> {
    let about_crs = j["items"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|i| i["invalid_reasons"].as_array().into_iter().flatten().any(|r| r.as_str().is_some_and(|r| r.contains("crs"))));
    if !about_crs {
        return None;
    }
    let ids: Vec<&str> = j["suggested_crs"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_str()).collect();
    let code = |id: &&str| id.strip_prefix("EPSG:").and_then(|c| c.parse::<u32>().ok());
    ids.iter()
        .find(|id| code(id).is_some_and(|c| (32701..=32760).contains(&c) || (32601..=32660).contains(&c)))
        .or_else(|| ids.iter().find(|id| **id == "EPSG:2193"))
        .or_else(|| ids.iter().find(|id| **id == "EPSG:4326"))
        .map(|s| s.to_string())
}

/// A refusal in a line: the reasons, not LINZ's whole list of coordinate systems.
fn refusal(j: &Value) -> String {
    let mut reasons: Vec<String> = j["invalid_reasons"].as_array().into_iter().flatten().filter_map(|r| r.as_str().map(String::from)).collect();
    for i in j["items"].as_array().into_iter().flatten() {
        reasons.extend(i["invalid_reasons"].as_array().into_iter().flatten().filter_map(|r| r.as_str().map(String::from)));
    }
    if reasons.is_empty() {
        j.to_string().chars().take(400).collect()
    } else {
        reasons.join(", ")
    }
}

/// The GeoTIFFs in an export zip, flat into `to`.
pub fn unzip_tiffs(zip_path: &Path, to: &Path) -> Result<usize, String> {
    let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut n = 0;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let name = e.name().map_err(|e| e.to_string())?.rsplit('/').next().unwrap_or("").to_string();
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".tif") || lower.ends_with(".tiff")) || name.contains("..") {
            continue;
        }
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        std::fs::write(to.join(&name), buf).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// The survey index (WFS, GeoJSON in longitude and latitude).
fn index(c: &reqwest::blocking::Client, key: &str) -> Result<Value, String> {
    let url = format!(
        "https://data.linz.govt.nz/services;key={key}/wfs?service=WFS&version=2.0.0&request=GetFeature&typeNames=layer-{INDEX_LAYER}&outputFormat=application/json&srsName=EPSG:4326"
    );
    let r = c.get(&url).send().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("WFS: {}", r.status()));
    }
    let mut g: Value = r.json().map_err(|e| format!("WFS: {e}"))?;
    swap_if_lat_first(&mut g);
    Ok(g)
}

/// WFS 2.0 in EPSG:4326 may give latitude first: New Zealand's longitudes are over 90°,
/// so if the first number of the first point isn't, swap every pair.
pub fn swap_if_lat_first(g: &mut Value) {
    fn first(v: &Value) -> Option<(f64, f64)> {
        let a = v.as_array()?;
        if a.len() >= 2 && a[0].is_number() {
            return Some((a[0].as_f64()?, a[1].as_f64()?));
        }
        a.iter().find_map(first)
    }
    fn swap(v: &mut Value) {
        if let Some(a) = v.as_array_mut() {
            if a.len() >= 2 && a[0].is_number() && a[1].is_number() {
                a.swap(0, 1);
            } else {
                a.iter_mut().for_each(swap);
            }
        }
    }
    let feats = g["features"].as_array().cloned().unwrap_or_default();
    let Some((a, _)) = feats.iter().find_map(|f| first(&f["geometry"]["coordinates"])) else { return };
    if a.abs() <= 90.0 {
        if let Some(fs) = g["features"].as_array_mut() {
            for f in fs {
                swap(&mut f["geometry"]["coordinates"]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_reads_as_a_model() {
        let j = json!({
            "id": 122423, "title": "Tasman Bay  Te Tai-o-Aorere Multibeam 5m Depth Model (2022)",
            "description": "This layer contains depth information.\nVertical accuracy is +/- 0.5m (95%)\nHorizontal accuracy is +/- 1m (95%)\nVertical datum is Approximate Lowest Astronomical Tide.\n",
            "published_at": "2025-03-02T01:02:03Z", "published_version": "https://data.linz.govt.nz/services/api/v1.x/layers/122423/versions/414395/",
            "data": { "crs": { "id": "EPSG:32759" }, "raster_resolution": 5, "extent": { "type": "Polygon", "coordinates": [[[172.1, -41.3], [174.1, -41.3], [174.1, -40.4], [172.1, -41.3]]] } }
        });
        let m = model_of(&j).unwrap();
        assert_eq!(m.survey.year, 2022);
        assert_eq!(m.survey.epsg, 32759);
        assert_eq!(m.survey.res, 5.0);
        assert_eq!(m.survey.datum, "Approximate Lowest Astronomical Tide");
        assert!(m.survey.accuracy.contains("Vertical accuracy is +/- 0") && m.survey.accuracy.contains("Horizontal"), "{}", m.survey.accuracy);
        assert_eq!(m.version, 414395);
        assert_eq!(m.survey.published, "2025-03-02");
    }

    #[test]
    fn a_crs_refusal_gives_one_the_build_reads() {
        // as LINZ refused Shipping Lane 6 (116674) in EPSG:32760
        let j = json!({ "invalid_reasons": [], "is_valid": false,
            "items": [{ "invalid_reasons": ["invalid-for-given-crs"], "is_valid": false }],
            "suggested_crs": [{ "id": "EPSG:4326" }, { "id": "EPSG:2134" }, { "id": "EPSG:2193" }, { "id": "EPSG:27200" }, { "id": "EPSG:32359" }, { "id": "EPSG:32759" }, { "id": "EPSG:3857" }] });
        assert_eq!(alternative_crs(&j).as_deref(), Some("EPSG:32759"));
        assert_eq!(refusal(&j), "invalid-for-given-crs");
        // without a UTM zone: NZTM
        let j2 = json!({ "items": [{ "invalid_reasons": ["invalid-for-given-crs"] }], "suggested_crs": [{ "id": "EPSG:4326" }, { "id": "EPSG:2193" }] });
        assert_eq!(alternative_crs(&j2).as_deref(), Some("EPSG:2193"));
        // some other refusal: no retry
        let j3 = json!({ "items": [{ "invalid_reasons": ["too-big"] }], "suggested_crs": [{ "id": "EPSG:32759" }] });
        assert_eq!(alternative_crs(&j3), None);
    }

    #[test]
    fn latitude_first_is_turned_round() {
        let mut g = json!({"features": [{"geometry": {"type": "Polygon", "coordinates": [[[-41.0, 173.0], [-41.5, 173.5]]]}}]});
        swap_if_lat_first(&mut g);
        assert_eq!(g["features"][0]["geometry"]["coordinates"][0][0], json!([173.0, -41.0]));
        swap_if_lat_first(&mut g); // already right: left alone
        assert_eq!(g["features"][0]["geometry"]["coordinates"][0][0], json!([173.0, -41.0]));
    }
}
