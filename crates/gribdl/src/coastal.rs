//! LINZ's coastal elevation models (Toitū Te Whenua LINZ, CC BY 4.0): 1 m GeoTIFF tiles
//! (LERC-compressed, NZTM2000) from aerial laser surveys reaching to about 25 m depth,
//! in the public `nz-coastal` bucket on AWS Open Data. The national collection
//! (`new-zealand/new-zealand/dem_1m/2193`) holds the best data for each 1:10 000 tile;
//! its `collection.json` lists the tiles there are, by name.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::{client, Error, Result};

pub const NATIONAL: &str = "new-zealand/new-zealand/dem_1m/2193";
/// How long the list of tiles is kept before it's fetched again.
const INDEX_DAYS: i64 = 7;

fn base() -> String {
    std::env::var("RACENAV_COASTAL_URL")
        .unwrap_or_else(|_| "https://nz-coastal.s3-ap-southeast-2.amazonaws.com".into())
}

fn get_json(c: &reqwest::blocking::Client, url: &str) -> Result<Value> {
    let r = c.get(url).send().map_err(|e| Error::Http(e.to_string()))?;
    if !r.status().is_success() {
        return Err(Error::Http(format!(
            "HTTP {} from {url}",
            r.status().as_u16()
        )));
    }
    let b = r.bytes().map_err(|e| Error::Http(e.to_string()))?;
    serde_json::from_slice(&b).map_err(|e| Error::Http(format!("{url}: {e}")))
}

/// Tile names → their GeoTIFF's path in the bucket, from a STAC collection.
fn collection_items(
    c: &reqwest::blocking::Client,
    path: &str,
    out: &mut BTreeMap<String, String>,
) -> Result<()> {
    let v = get_json(c, &format!("{}/{path}/collection.json", base()))?;
    for l in v["links"].as_array().into_iter().flatten() {
        if l["rel"] != "item" {
            continue;
        }
        let Some(href) = l["href"].as_str() else {
            continue;
        };
        let name = href.trim_start_matches("./").trim_end_matches(".json");
        if name.contains('/') || name.is_empty() {
            continue;
        }
        out.insert(name.to_string(), format!("{path}/{name}.tiff"));
    }
    Ok(())
}

/// The tiles there are (name → path in the bucket): the national collection, else every
/// 1 m collection in the catalogue (later surveys replacing earlier ones). Kept in
/// `dir/index.json` for a week.
pub fn available(dir: &Path) -> Result<BTreeMap<String, String>> {
    let cache = dir.join("index.json");
    if let Ok(b) = std::fs::read(&cache) {
        if let Ok(v) = serde_json::from_slice::<Value>(&b) {
            let fresh = v["fetched"]
                .as_i64()
                .is_some_and(|t| chrono::Utc::now().timestamp() - t < INDEX_DAYS * 86_400);
            if let (true, Some(m)) = (fresh, v["tiles"].as_object()) {
                return Ok(m
                    .iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect());
            }
        }
    }
    let c = client()?;
    let mut out = BTreeMap::new();
    if collection_items(&c, NATIONAL, &mut out).is_err() || out.is_empty() {
        let cat = get_json(&c, &format!("{}/catalog.json", base()))?;
        let mut paths: Vec<String> = cat["links"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|l| l["rel"] == "child")
            .filter_map(|l| l["href"].as_str())
            .filter(|h| h.contains("/dem_1m/"))
            .map(|h| {
                h.trim_start_matches("./")
                    .trim_end_matches("/collection.json")
                    .to_string()
            })
            .collect();
        paths.sort(); // survey names end in their years: later ones last, and win
        for p in paths {
            let _ = collection_items(&c, &p, &mut out);
        }
    }
    if out.is_empty() {
        return Err(Error::Http("no coastal elevation tiles listed".into()));
    }
    std::fs::create_dir_all(dir)?;
    let doc = serde_json::json!({ "fetched": chrono::Utc::now().timestamp(), "tiles": out });
    std::fs::write(&cache, doc.to_string())?;
    Ok(out)
}

/// Download `paths` (name → path in the bucket) into `dir` as `<name>.tiff`, skipping
/// ones already there. `progress(tile, of, bytes so far)`.
pub fn download(
    paths: &[(String, String)],
    dir: &Path,
    progress: &(dyn Fn(usize, usize, u64) + Sync),
    cancel: &AtomicBool,
) -> Result<usize> {
    std::fs::create_dir_all(dir)?;
    let c = reqwest::blocking::Client::builder()
        .user_agent(concat!("TweedPlot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(None::<std::time::Duration>)
        .build()
        .map_err(|e| Error::Http(e.to_string()))?;
    let mut done = 0;
    let mut bytes = 0u64;
    for (i, (name, path)) in paths.iter().enumerate() {
        let out = dir.join(format!("{name}.tiff"));
        if out.exists() {
            continue;
        }
        progress(i, paths.len(), bytes);
        let url = format!("{}/{path}", base());
        let mut r = c.get(&url).send().map_err(|e| Error::Http(e.to_string()))?;
        if !r.status().is_success() {
            return Err(Error::Http(format!(
                "HTTP {} from {url}",
                r.status().as_u16()
            )));
        }
        let part = out.with_extension("part");
        let mut f = std::fs::File::create(&part)?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(f);
                let _ = std::fs::remove_file(&part);
                return Err(Error::Cancelled);
            }
            let n = r.read(&mut buf).map_err(|e| Error::Http(e.to_string()))?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n])?;
            bytes += n as u64;
            progress(i, paths.len(), bytes);
        }
        f.flush()?;
        drop(f);
        // a TIFF starts II*\0 / II+\0 / MM\0*
        let mut head = [0u8; 4];
        std::fs::File::open(&part)?.read_exact(&mut head)?;
        if !(head.starts_with(b"II") || head.starts_with(b"MM")) {
            let _ = std::fs::remove_file(&part);
            return Err(Error::Bad(format!("{name}: not a GeoTIFF")));
        }
        std::fs::rename(&part, &out)?;
        done += 1;
    }
    progress(paths.len(), paths.len(), bytes);
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> String {
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
                let body = routes
                    .iter()
                    .find(|(k, _)| path.ends_with(k))
                    .map(|(_, b)| b.clone());
                let mut s = s;
                match body {
                    Some(b) => {
                        let _ = write!(
                            s,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            b.len()
                        );
                        let _ = s.write_all(&b);
                    }
                    None => {
                        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn lists_and_downloads_tiles() {
        let col = serde_json::json!({"links": [
            {"rel": "root", "href": "../../catalog.json"},
            {"rel": "item", "href": "./BQ31_10000_0401.json"},
            {"rel": "item", "href": "./BQ31_10000_0402.json"}
        ]});
        let base = serve(vec![
            ("dem_1m/2193/collection.json", col.to_string().into_bytes()),
            ("BQ31_10000_0401.tiff", b"II*\0fake tiff".to_vec()),
        ]);
        std::env::set_var("RACENAV_COASTAL_URL", &base);
        let dir = std::env::temp_dir().join(format!("tp_coastal_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let av = available(&dir).unwrap();
        assert_eq!(av.len(), 2);
        assert_eq!(
            av["BQ31_10000_0401"],
            format!("{NATIONAL}/BQ31_10000_0401.tiff")
        );
        assert!(dir.join("index.json").exists());
        let want = vec![("BQ31_10000_0401".to_string(), av["BQ31_10000_0401"].clone())];
        assert_eq!(
            download(&want, &dir, &|_, _, _| {}, &AtomicBool::new(false)).unwrap(),
            1
        );
        assert!(dir.join("BQ31_10000_0401.tiff").exists());
        // already there: nothing to do; a missing one is an error
        assert_eq!(
            download(&want, &dir, &|_, _, _| {}, &AtomicBool::new(false)).unwrap(),
            0
        );
        let missing = vec![("BQ31_10000_0402".to_string(), av["BQ31_10000_0402"].clone())];
        assert!(download(&missing, &dir, &|_, _, _| {}, &AtomicBool::new(false)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
