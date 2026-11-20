//! Sentinel-2 satellite images from Element 84's Earth Search catalogue (STAC) on AWS: free,
//! no account, Copernicus licence (free use including commercial; credit "Contains
//! modified Copernicus Sentinel data <year>").
//!
//! A search lists the scenes over an area (date, cloud cover); a download copies just the
//! area from the scene's true-colour image (a cloud-optimised GeoTIFF, 10 m) with HTTP range
//! requests, at the finest resolution that fits the size limit, into a local GeoTIFF.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::geo::Affine;
use crate::render::Raster;
use crate::source::{client, HttpSource, Source};
use crate::tiff::Tiff;
use crate::tiffw::GeoRef;
use crate::{err, Error, Result};

pub const SEARCH_URL: &str = "https://earth-search.aws.element84.com/v1/search";
/// Collection 1 (reprocessed, consistent), then the original L2A collection.
pub const COLLECTIONS: [&str; 2] = ["sentinel-2-c1-l2a", "sentinel-2-l2a"];

/// A scene the search found.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub id: String,
    pub collection: String,
    /// ISO date and time (UTC).
    pub datetime: String,
    pub cloud: Option<f64>,
    pub platform: String,
    pub tile: String,
    /// The true-colour image (cloud-optimised GeoTIFF).
    pub visual: String,
    pub thumbnail: Option<String>,
    pub bbox: [f64; 4],
    /// The share of the asked-for area inside the scene's outline (0–1, from the bounding box).
    pub cover: f64,
}

/// The search request body.
pub fn search_body(collection: &str, bbox: [f64; 4], from: &str, to: &str, max_cloud: f64, limit: usize) -> Value {
    json!({
        "collections": [collection],
        "bbox": bbox,
        "datetime": format!("{from}T00:00:00Z/{to}T23:59:59Z"),
        "limit": limit,
        "query": { "eo:cloud_cover": { "lte": max_cloud } },
        "sortby": [{ "field": "properties.eo:cloud_cover", "direction": "asc" }],
    })
}

/// S3 addresses as https (the buckets are public).
fn https(href: &str) -> Option<String> {
    if href.starts_with("https://") {
        return Some(href.to_string());
    }
    let rest = href.strip_prefix("s3://")?;
    let (bucket, key) = rest.split_once('/')?;
    Some(format!("https://{bucket}.s3.us-west-2.amazonaws.com/{key}"))
}

fn asset_href(item: &Value, key: &str) -> Option<String> {
    let a = item.get("assets")?.get(key)?;
    let h = a.get("href").and_then(Value::as_str)?;
    https(h).or_else(|| ["https", "download"].iter().find_map(|k| a.pointer(&format!("/alternate/{k}/href")).and_then(Value::as_str).and_then(https)))
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let area = (a[2] - a[0]) * (a[3] - a[1]);
    if area <= 0.0 {
        0.0
    } else {
        (w * h / area).clamp(0.0, 1.0)
    }
}

/// The scenes in a search response.
pub fn parse_search(resp: &Value, collection: &str, area: [f64; 4]) -> Vec<Scene> {
    let mut out = Vec::new();
    for f in resp.get("features").and_then(Value::as_array).into_iter().flatten() {
        let Some(visual) = asset_href(f, "visual") else { continue };
        let p = f.get("properties").cloned().unwrap_or(Value::Null);
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let bbox =
            f.get("bbox").and_then(Value::as_array).filter(|b| b.len() >= 4).map(|b| [0, 1, 2, 3].map(|i| b[i].as_f64().unwrap_or(0.0))).unwrap_or([0.0; 4]);
        let tile = [
            s("grid:code"),
            s("s2:mgrs_tile"),
            format!("{}{}{}", p.get("mgrs:utm_zone").map(|v| v.to_string()).unwrap_or_default(), s("mgrs:latitude_band"), s("mgrs:grid_square")),
        ]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or_default()
        .trim_start_matches("MGRS-")
        .to_string();
        out.push(Scene {
            id: f.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            collection: collection.to_string(),
            datetime: s("datetime"),
            cloud: p.get("eo:cloud_cover").and_then(Value::as_f64),
            platform: s("platform"),
            tile,
            visual,
            thumbnail: asset_href(f, "thumbnail"),
            bbox,
            cover: overlap(area, bbox),
        });
    }
    out
}

/// Search Earth Search for scenes over `bbox` [west, south, east, north] between two dates
/// (YYYY-MM-DD), at most `max_cloud` % cloud. Best first: most of the area, least cloud, newest.
pub fn search(bbox: [f64; 4], from: &str, to: &str, max_cloud: f64) -> Result<Vec<Scene>> {
    if !(bbox[0] < bbox[2] && bbox[1] < bbox[3]) {
        return err("the area is empty");
    }
    let c = client()?;
    let mut last = String::new();
    for coll in COLLECTIONS {
        let mut body = search_body(coll, bbox, from, to, max_cloud, 60);
        // some STAC servers refuse sortby: ask again without it
        for attempt in 0..2 {
            if attempt == 1 {
                body.as_object_mut().map(|o| o.remove("sortby"));
            }
            let r = match c.post(SEARCH_URL).header("Content-Type", "application/json").body(body.to_string()).send() {
                Ok(r) => r,
                Err(e) => return err(format!("could not reach Earth Search: {e}")),
            };
            let status = r.status();
            let text = r.text().unwrap_or_default();
            if !status.is_success() {
                last = format!("Earth Search: {status} {}", text.chars().take(200).collect::<String>());
                continue;
            }
            let v: Value = serde_json::from_str(&text).map_err(|e| Error(format!("Earth Search reply: {e}")))?;
            let mut scenes = parse_search(&v, coll, bbox);
            sort_scenes(&mut scenes);
            return Ok(scenes);
        }
    }
    err(last)
}

pub fn sort_scenes(s: &mut [Scene]) {
    s.sort_by(|a, b| {
        let cov = |x: &Scene| (x.cover * 10.0).round();
        cov(b).total_cmp(&cov(a)).then(a.cloud.unwrap_or(100.0).total_cmp(&b.cloud.unwrap_or(100.0))).then(b.datetime.cmp(&a.datetime))
    });
}

/// Only Earth Search's own image buckets are fetched from.
pub fn allowed_url(url: &str) -> bool {
    let host = crate::source::host_of(url);
    url.starts_with("https://")
        && (host.ends_with(".s3.us-west-2.amazonaws.com") || host == "s3.us-west-2.amazonaws.com" || host == "earth-search.aws.element84.com")
}

/// A thumbnail (JPEG/PNG bytes) for the scene list.
pub fn thumbnail(url: &str) -> Result<Vec<u8>> {
    if !allowed_url(url) {
        return err("not an Earth Search address");
    }
    let r = client()?.get(url).send().map_err(|e| Error(format!("thumbnail: {e}")))?;
    if !r.status().is_success() {
        return err(format!("thumbnail: {}", r.status()));
    }
    let b = r.bytes().map_err(|e| Error(format!("thumbnail: {e}")))?;
    if b.len() > 4 * 1024 * 1024 {
        return err("thumbnail too large");
    }
    Ok(b.to_vec())
}

/// Progress of a download: blocks done of total, and a stop flag.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub stop: AtomicBool,
}

/// The cut-out: pixels (RGBA), size, georeferencing, and the ground resolution.
pub struct Cut {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub geo: GeoRef,
    pub res_m: f64,
}

/// Copy `bbox` out of a (local or remote) GeoTIFF at the finest level whose cut-out is at most
/// `max_px` on each side. `workers` blocks are fetched at once.
pub fn cut(tiff: &Tiff, bbox: [f64; 4], max_px: usize, workers: usize, progress: &Progress) -> Result<Cut> {
    let Some((crs, aff)) = tiff.geo else { return err("the image has no georeferencing") };
    let Some(epsg) = tiff.epsg else { return err("the image has no EPSG code") };
    if aff.c != 0.0 || aff.e != 0.0 {
        return err("rotated images are not supported");
    }
    // the area's outline in the image's pixels
    let (mut c0, mut r0, mut c1, mut r1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let n = 16;
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let lon = bbox[0] + (bbox[2] - bbox[0]) * t;
        let lat = bbox[1] + (bbox[3] - bbox[1]) * t;
        for (lo, la) in [(lon, bbox[1]), (lon, bbox[3]), (bbox[0], lat), (bbox[2], lat)] {
            if let Some((c, r)) = tiff.to_pixel(lo, la) {
                c0 = c0.min(c);
                r0 = r0.min(r);
                c1 = c1.max(c);
                r1 = r1.max(r);
            }
        }
    }
    let (w0, h0) = tiff.size();
    let (c0, r0) = (c0.floor().max(0.0) as usize, r0.floor().max(0.0) as usize);
    let (c1, r1) = ((c1.ceil().max(0.0) as usize).min(w0), (r1.ceil().max(0.0) as usize).min(h0));
    if c1 <= c0 || r1 <= r0 {
        return err("the area is outside this scene");
    }
    // finest level that fits
    let mut level = tiff.levels() - 1;
    for l in 0..tiff.levels() {
        let (lw, lh) = tiff.level_size(l);
        let (sx, sy) = (w0 as f64 / lw as f64, h0 as f64 / lh as f64);
        if ((c1 - c0) as f64 / sx).ceil() as usize <= max_px && ((r1 - r0) as f64 / sy).ceil() as usize <= max_px {
            level = l;
            break;
        }
    }
    let (lw, lh) = tiff.level_size(level);
    let (sx, sy) = (w0 as f64 / lw as f64, h0 as f64 / lh as f64);
    let (lc0, lr0) = ((c0 as f64 / sx).floor() as usize, (r0 as f64 / sy).floor() as usize);
    let (lc1, lr1) = (((c1 as f64 / sx).ceil() as usize).min(lw), ((r1 as f64 / sy).ceil() as usize).min(lh));
    let (w, h) = (lc1 - lc0, lr1 - lr0);
    if w == 0 || h == 0 {
        return err("the area is too small");
    }
    let (bw, bh) = tiff.block_size(level);
    let blocks: Vec<(usize, usize)> = (lr0 / bh..lr1.div_ceil(bh)).flat_map(|by| (lc0 / bw..lc1.div_ceil(bw)).map(move |bx| (bx, by))).collect();
    progress.total.store(blocks.len(), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let next = AtomicUsize::new(0);
    let out = std::sync::Mutex::new(vec![0u8; w * h * 4]);
    let failure: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..workers.max(1) {
            s.spawn(|| loop {
                if progress.stop.load(Ordering::Relaxed) || failure.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(bx, by)) = blocks.get(i) else { return };
                match tiff.block_bytes(level, bx, by) {
                    Ok(Some(b)) => {
                        let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
                        for y in 0..b.h {
                            let gy = by * bh + y;
                            if gy < lr0 || gy >= lr1 {
                                continue;
                            }
                            for x in 0..b.w {
                                let gx = bx * bw + x;
                                if gx < lc0 || gx >= lc1 {
                                    continue;
                                }
                                let d = ((gy - lr0) * w + (gx - lc0)) * 4;
                                let px = block_px(&b, x, y);
                                o[d..d + 4].copy_from_slice(&px);
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.to_string());
                        return;
                    }
                }
                progress.done.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    if let Some(e) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return err(e);
    }
    if progress.stop.load(Ordering::Relaxed) {
        return err("stopped");
    }
    let geo =
        GeoRef { epsg, aff: Affine { a: aff.a + aff.b * sx * lc0 as f64, b: aff.b * sx, c: 0.0, d: aff.d + aff.f * sy * lr0 as f64, e: 0.0, f: aff.f * sy } };
    let res_m = crate::render::resolution_of(&crs, &geo.aff, w, h);
    let _ = crs;
    Ok(Cut { w, h, rgba: out.into_inner().unwrap_or_else(|e| e.into_inner()), geo, res_m })
}

fn block_px(b: &crate::render::Block, x: usize, y: usize) -> [u8; 4] {
    let i = y * b.w + x;
    match &b.palette {
        Some(p) => p.get(b.data[i] as usize).copied().unwrap_or([0; 4]),
        None => [b.data[i * 4], b.data[i * 4 + 1], b.data[i * 4 + 2], b.data[i * 4 + 3]],
    }
}

/// Download `bbox` from a scene into a GeoTIFF at `path`.
pub fn download(scene: &Scene, bbox: [f64; 4], max_px: usize, path: &std::path::Path, progress: &Progress) -> Result<Cut> {
    if !allowed_url(&scene.visual) {
        return err("not an Earth Search image address");
    }
    let src: Arc<dyn Source> = Arc::new(HttpSource::new(&scene.visual)?);
    let tiff = Tiff::open(src)?;
    let cut = cut(&tiff, bbox, max_px, 6, progress)?;
    if !cut.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0) {
        return err("the scene has no image over this area");
    }
    crate::tiffw::write_geotiff(path, cut.w, cut.h, &cut.rgba, &cut.geo, &format!("TweedPlot; Contains modified Copernicus Sentinel data; {}", scene.id))?;
    Ok(cut)
}

/// The credit the Copernicus licence asks for.
pub fn credit(datetime: &str) -> String {
    let year = datetime.get(..4).unwrap_or("");
    format!("Contains modified Copernicus Sentinel data {year}").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_search_reply() {
        let v: Value = serde_json::from_str(
            r#"{"type":"FeatureCollection","features":[
            {"id":"S2B_T59GPN_20251203T222539_L2A","bbox":[172.9,-41.6,174.2,-40.6],
             "properties":{"datetime":"2025-12-03T22:30:01.024000Z","eo:cloud_cover":3.2,"platform":"sentinel-2b","grid:code":"MGRS-59GPN"},
             "assets":{"visual":{"href":"s3://e84-earth-search-sentinel-data/sentinel-2-c1-l2a/59/G/PN/2025/12/S2B_T59GPN_20251203T222539_L2A/TCI.tif"},
                       "thumbnail":{"href":"https://e84-earth-search-sentinel-data.s3.us-west-2.amazonaws.com/x/L2A_PVI.jpg"}}},
            {"id":"no-visual","bbox":[0,0,1,1],"properties":{},"assets":{}}]}"#,
        )
        .unwrap();
        let s = parse_search(&v, "sentinel-2-c1-l2a", [173.2, -41.3, 173.3, -41.2]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].tile, "59GPN");
        assert_eq!(s[0].cloud, Some(3.2));
        assert!(s[0].visual.starts_with("https://e84-earth-search-sentinel-data.s3.us-west-2.amazonaws.com/sentinel-2-c1-l2a/"));
        assert!(allowed_url(&s[0].visual));
        assert!((s[0].cover - 1.0).abs() < 1e-9);
        assert!(!allowed_url("https://example.com/x.tif"));
        assert_eq!(credit(&s[0].datetime), "Contains modified Copernicus Sentinel data 2025");
    }
}
