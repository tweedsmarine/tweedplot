//! Making the packages: every survey's GeoTIFFs gridded onto the chart tiles at the zoom of
//! its own grid (one master file), the coarser zooms made shoal-biased, then cut into the
//! national package and the regional full-resolution ones, with a manifest.

use crate::geotiff::GeoTiff;
use crate::lds::{load_catalogue, Model};
use crate::regions::{region_of, OFFSHORE, REGIONS};
use chart::depthpack::{self, extract_where, from_pixel, to_pixel, zoom_for, Survey, Writer, MIN_ZOOM, N};
use chart::shp::{lonlat_to_tm, tm_to_lonlat, Tm};
use rayon::prelude::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const ATTRIBUTION: &str = "Sourced from the LINZ Data Service and licensed for reuse under the CC BY 4.0 licence. Not for navigation.";
const DATUM: &str = "Approximate Lowest Astronomical Tide";

/// The GeoTIFFs of a survey's download.
fn tiffs(sdir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(sdir)
        .map(|rd| {
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tif") || x.eq_ignore_ascii_case("tiff"))).collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Survey outlines from LINZ's index, by the depth model's title (`lds_name`).
fn outlines(dir: &Path) -> HashMap<String, Value> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let mut by: HashMap<String, Vec<Value>> = HashMap::new();
    let Ok(t) = std::fs::read_to_string(dir.join("src").join("index.geojson")) else { return HashMap::new() };
    let Ok(g) = serde_json::from_str::<Value>(&t) else { return HashMap::new() };
    for f in g["features"].as_array().into_iter().flatten() {
        let Some(name) = f["properties"]["lds_name"].as_str().filter(|s| !s.is_empty()) else { continue };
        let geom = &f["geometry"];
        let polys: Vec<Value> = match geom["type"].as_str() {
            Some("Polygon") => vec![geom["coordinates"].clone()],
            Some("MultiPolygon") => geom["coordinates"].as_array().cloned().unwrap_or_default(),
            _ => continue,
        };
        by.entry(norm(name)).or_default().extend(polys);
    }
    by.into_iter().map(|(k, p)| (k, json!({ "type": "MultiPolygon", "coordinates": p }))).collect()
}

/// Paint one survey's GeoTIFFs into the master at zoom `z`. Cells: the value of the source
/// pixel each cell's middle falls in. Returns the number of tiles touched.
fn paint_survey(w: &Writer, s: &Survey, files: &[PathBuf], z: u8) -> Result<usize, String> {
    let mut touched = 0;
    for (k, path) in files.iter().enumerate() {
        let mut g = match GeoTiff::open(path) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("  {}: {e}", path.display());
                continue;
            }
        };
        let epsg = g.epsg.unwrap_or(s.epsg);
        let geographic = epsg == 4326 || epsg == 4167;
        let tm = if geographic { None } else { Some(Tm::from_epsg(epsg).ok_or(format!("{}: projection EPSG:{epsg} not known", path.display()))?) };
        let to_ll = |x: f64, y: f64| -> [f64; 2] { tm.map_or([x, y], |t| tm_to_lonlat([x, y], t)) };
        let to_map = |lon: f64, lat: f64| -> [f64; 2] { tm.map_or([lon, lat], |t| lonlat_to_tm([lon, lat], t)) };
        // the file's area in longitude and latitude (its edges, as they bend)
        let b = g.bounds();
        let mut ll = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for i in 0..=20 {
            let f = i as f64 / 20.0;
            for p in [[b[0] + (b[2] - b[0]) * f, b[1]], [b[0] + (b[2] - b[0]) * f, b[3]], [b[0], b[1] + (b[3] - b[1]) * f], [b[2], b[1] + (b[3] - b[1]) * f]] {
                let q = to_ll(p[0], p[1]);
                ll = [ll[0].min(q[0]), ll[1].min(q[1]), ll[2].max(q[0]), ll[3].max(q[1])];
            }
        }
        let (x0, y0) = to_pixel(ll[0], ll[3], z);
        let (x1, y1) = to_pixel(ll[2], ll[1], z);
        let (tx0, ty0, tx1, ty1) = ((x0 / N as f64) as u32, (y0 / N as f64) as u32, (x1 / N as f64) as u32, (y1 / N as f64) as u32);
        let tx = w.db.unchecked_transaction().map_err(|e| e.to_string())?;
        for ty in ty0..=ty1 {
            for txx in tx0..=tx1 {
                // where each cell's middle is in the file's projection (in parallel), then
                // the values (one reader)
                let pos: Vec<[f64; 2]> = (0..N * N)
                    .into_par_iter()
                    .map(|i| {
                        let (lon, lat) = from_pixel((txx as f64) * N as f64 + (i % N) as f64 + 0.5, (ty as f64) * N as f64 + (i / N) as f64 + 0.5, z);
                        to_map(lon, lat)
                    })
                    .collect();
                let cells: Vec<f32> = pos.iter().map(|p| g.value_at(p[0], p[1]).unwrap_or(f32::NAN)).collect();
                if cells.iter().any(|v| !v.is_nan()) {
                    w.paint(z, txx, ty, &cells).map_err(|e| e.to_string())?;
                    touched += 1;
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        print!("\r  {}/{} files, {touched} tiles   ", k + 1, files.len());
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
    println!();
    Ok(touched)
}

/// Today as YYYY-MM-DD (UTC).
pub fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| e.to_string())?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Build D/out: nz-national-<edition>.tpdepth, region-<id>-<edition>.tpdepth, manifest.json.
pub fn build(dir: &Path, edition: &str, national_max: u8, only: Option<&[i64]>) -> Result<(), String> {
    let cat: Vec<Model> = load_catalogue(dir)?.into_iter().filter(|m| only.is_none_or(|o| o.contains(&m.survey.id))).collect();
    let outl = outlines(dir);
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let work = dir.join("work");
    let out = dir.join("out");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let master = work.join("master.tpdepth");
    let _ = std::fs::remove_file(&master);
    let w = Writer::create(&master).map_err(|e| e.to_string())?;

    // coarsest and oldest first: the finer and newer are painted over them
    let mut todo: Vec<(Survey, Vec<PathBuf>)> = cat
        .iter()
        .filter_map(|m| {
            let files = tiffs(&dir.join("src").join(m.survey.id.to_string()));
            if files.is_empty() {
                eprintln!("{}: not downloaded (run export)", m.survey.title);
                return None;
            }
            let mut s = m.survey.clone();
            if let Some(o) = outl.get(&norm(&s.title)) {
                s.outline = o.clone();
            }
            Some((s, files))
        })
        .collect();
    todo.sort_by(|a, b| b.0.res.total_cmp(&a.0.res).then(a.0.year.cmp(&b.0.year)));
    if todo.is_empty() {
        return Err("nothing to build: no downloaded surveys".into());
    }
    let mut max_z = MIN_ZOOM;
    for (s, files) in &todo {
        let lat = depthpack::bbox_of(&s.outline).map_or(-41.0, |b| (b[1] + b[3]) / 2.0);
        let z = zoom_for(s.res.max(0.5), lat).max(MIN_ZOOM);
        max_z = max_z.max(z);
        println!("{} ({} m, zoom {z}):", s.title, s.res);
        paint_survey(&w, s, files, z)?;
        w.add_survey(s).map_err(|e| e.to_string())?;
    }
    println!("coarser zooms…");
    w.build_pyramid(MIN_ZOOM, max_z).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;

    // the packages
    let created = today();
    let common = |tier: &str, region: &str, name: &str| -> Vec<(String, String)> {
        vec![
            ("edition".into(), edition.into()),
            ("tier".into(), tier.into()),
            ("region".into(), region.into()),
            ("region_name".into(), name.into()),
            ("datum".into(), DATUM.into()),
            ("source".into(), "LINZ Data Service, Hydrographic & Maritime: Bathymetric Surfaces".into()),
            ("licence".into(), "CC BY 4.0".into()),
            ("attribution".into(), ATTRIBUTION.into()),
            ("created".into(), created.clone()),
        ]
    };
    let mut packages = Vec::new();
    let mut make = |file: String, zooms: std::ops::RangeInclusive<u8>, keep: &dyn Fn(f64, f64) -> bool, meta: Vec<(String, String)>| -> Result<(), String> {
        let path = out.join(&file);
        let m: Vec<(&str, &str)> = meta.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let n = extract_where(&master, &path, zooms, keep, &m).map_err(|e| e.to_string())?;
        if n == 0 {
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        let info = depthpack::read_info(&path)?;
        let surveys = depthpack::survey_ids(&path).map_err(|e| e.to_string())?;
        println!("{file}: {n} tiles, {:.1} MB", info.bytes as f64 / 1e6);
        packages.push(json!({
            "file": file, "tier": info.tier, "region": info.region, "region_name": info.region_name,
            "min_zoom": info.min_zoom, "max_zoom": info.max_zoom, "bytes": info.bytes, "sha256": sha256(&path)?, "surveys": surveys,
        }));
        Ok(())
    };
    make(format!("nz-national-{edition}.tpdepth"), MIN_ZOOM..=national_max, &|_, _| true, common("national", "nz", "New Zealand"))?;
    if max_z > national_max {
        for r in REGIONS.iter().chain(std::iter::once(&OFFSHORE)) {
            let id = r.id;
            make(format!("region-{id}-{edition}.tpdepth"), national_max + 1..=max_z, &move |lon, lat| region_of(lon, lat) == id, common("region", id, r.name))?;
        }
    }
    let manifest = json!({
        "format": "tweedplot-depth-manifest",
        "version": 1,
        "edition": edition,
        "created": created,
        "datum": DATUM,
        "licence": "CC BY 4.0",
        "attribution": ATTRIBUTION,
        "packages": packages,
        "surveys": todo.iter().map(|(s, _)| json!({ "id": s.id, "title": s.title, "year": s.year, "res": s.res, "published": s.published })).collect::<Vec<_>>(),
    });
    std::fs::write(out.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).map_err(|e| e.to_string())?;
    println!("done: {} packages and manifest.json in {}", manifest["packages"].as_array().map_or(0, Vec::len), out.display());
    Ok(())
}

/// Check D/out: every package in the manifest there, of the right size and checksum, and
/// readable; optionally the depth at a place and a tile drawn to D/out/<z>-<x>-<y>.png.
pub fn check(dir: &Path, at: Option<&str>, png: Option<&str>) -> Result<(), String> {
    let out = dir.join("out");
    let m: Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).map_err(|e| format!("manifest.json: {e}"))?).map_err(|e| e.to_string())?;
    let mut bad = 0;
    for p in m["packages"].as_array().into_iter().flatten() {
        let file = p["file"].as_str().unwrap_or("");
        let path = out.join(file);
        let ok = std::fs::metadata(&path).map(|x| x.len()).ok() == p["bytes"].as_u64() && sha256(&path).ok().as_deref() == p["sha256"].as_str();
        match (ok, depthpack::read_info(&path)) {
            (true, Ok(i)) => println!("ok   {file}: {} zooms {}–{}, {} surveys, {} tiles", i.region_name, i.min_zoom, i.max_zoom, i.surveys, i.tiles),
            (_, Err(e)) => {
                bad += 1;
                println!("BAD  {file}: {e}");
            }
            (false, _) => {
                bad += 1;
                println!("BAD  {file}: size or checksum differs from the manifest");
            }
        }
    }
    let packs = depthpack::DepthPacks::open(&out);
    if let Some(a) = at {
        let v: Vec<f64> = a.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() == 2 {
            match packs.height_at(v[0], v[1]) {
                Some(h) => println!("at {a}: {:.2} m ({})", h, if h <= 0.0 { format!("depth {:.2} m", -h) } else { "dries".into() }),
                None => println!("at {a}: no data"),
            }
        }
    }
    if let Some(t) = png {
        let v: Vec<u32> = t.split('/').filter_map(|x| x.parse().ok()).collect();
        if v.len() == 3 {
            match packs.tile_png(v[0] as u8, v[1], v[2]) {
                Some(b) => {
                    let f = out.join(format!("{}-{}-{}.png", v[0], v[1], v[2]));
                    std::fs::write(&f, b.as_slice()).map_err(|e| e.to_string())?;
                    println!("tile {t}: {}", f.display());
                }
                None => println!("tile {t}: no data"),
            }
        }
    }
    if bad > 0 {
        Err(format!("{bad} packages failed the check"))
    } else {
        Ok(())
    }
}
