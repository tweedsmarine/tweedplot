//! Survey depths: TweedPlot depth packages (`.tpdepth`, made by `tweedplot-depthpack` from
//! LINZ's published multibeam and lidar depth models, CC BY 4.0) in `<data>\depth`. Drawn as
//! images like the high-resolution depths; the survey outlines and the depth at a place for
//! the chart. Packages come as files (a USB stick, a shared folder: Import), one national and
//! any number of regional ones; importing a newer edition of a package replaces the older.

use chart::depthpack::{self, DepthPacks};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tauri::{Emitter, Manager};

static PACKS: RwLock<Option<Arc<DepthPacks>>> = RwLock::new(None);

pub fn packs() -> Option<Arc<DepthPacks>> {
    PACKS.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `RACENAV_DEPTH_DIR`, else `C:\dev\tweedplot-data\depth` when the dev data folder exists,
/// else `<app data>\depth`.
pub fn dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_DEPTH_DIR") {
        return PathBuf::from(d);
    }
    let dev = crate::dev_root();
    if dev.is_dir() {
        return dev.join("depth");
    }
    app.path().app_data_dir().map(|p| p.join("depth")).unwrap_or_else(|_| PathBuf::from("depth"))
}

/// (Re)open the packages (quick: their metadata only).
pub fn open(app: &tauri::AppHandle) {
    let p = DepthPacks::open(&dir(app));
    for (f, e) in &p.problems {
        eprintln!("survey depths: {f}: {e}");
    }
    *PACKS.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(p));
}

/// The image for /depthpack/z/x/y.png.
pub fn tile_png(z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
    packs()?.tile_png(z, x, y).map(|t| t.as_ref().clone())
}

/// The packages here, the editions and anything that couldn't be read.
#[tauri::command]
pub fn depthpack_status(app: tauri::AppHandle) -> Value {
    let p = packs();
    let infos = p.as_ref().map(|p| p.infos()).unwrap_or_default();
    let problems: Vec<Value> = p.as_ref().map(|p| p.problems.iter().map(|(f, e)| json!({ "file": f, "error": e })).collect()).unwrap_or_default();
    let surveys = p.as_ref().map_or(0, |p| p.surveys().len());
    json!({ "dir": dir(&app), "packages": infos, "problems": problems, "surveys": surveys })
}

/// The survey outlines, as GeoJSON features with their details.
#[tauri::command]
pub async fn depthpack_surveys() -> Value {
    let list = tauri::async_runtime::spawn_blocking(|| packs().map(|p| p.surveys()).unwrap_or_default()).await.unwrap_or_default();
    let features: Vec<Value> = list
        .into_iter()
        .filter(|s| s.outline.is_object())
        .map(|s| {
            json!({ "type": "Feature", "geometry": s.outline,
                "properties": { "id": s.id, "title": s.title, "year": s.year, "res": s.res, "datum": s.datum, "accuracy": s.accuracy, "published": s.published } })
        })
        .collect();
    json!({ "type": "FeatureCollection", "features": features })
}

/// Height (m to the survey datum, approximate LAT; negative below) at a place.
#[tauri::command]
pub async fn depthpack_height(lon: f64, lat: f64) -> Option<f32> {
    tauri::async_runtime::spawn_blocking(move || packs()?.height_at(lon, lat)).await.ok().flatten()
}

/// How an import is getting on: the file it is on (1-based) of `count`, what it is doing
/// with it ("checking" the checksum, "copying"), and the bytes done of all there are to read.
pub struct Progress<'a> {
    pub file: &'a str,
    pub index: usize,
    pub count: usize,
    pub stage: &'static str,
    pub done: u64,
    pub total: u64,
}

/// Read `path` in large blocks, telling `each` how many bytes each block was.
fn read_blocks(path: &Path, mut each: impl FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 4 << 20];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        each(&buf[..n])?;
    }
}

/// Copy packages in: `files` (from the picker), each checked (readable, and against the
/// manifest.json beside it when there is one). A package replaces another of the same tier
/// and region with an older edition; one older than the one here is skipped. `progress` is
/// told as it goes (files can be hundreds of MB, from a USB stick).
pub fn import_files(to: &Path, files: &[PathBuf], progress: &mut dyn FnMut(Progress)) -> Result<Value, String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let mut added = Vec::new();
    let mut skipped = Vec::new();
    // the manifest beside the files, if any, and what there is to read: each file once to
    // copy, and once more to check where the manifest has its checksum
    let manifest_of = |f: &Path| -> Option<Value> {
        f.parent().map(|p| p.join("manifest.json")).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok())
    };
    let entry_for = |m: &Option<Value>, name: &str| -> Option<String> {
        m.as_ref()?["packages"].as_array()?.iter().find(|p| p["file"] == name)?["sha256"].as_str().map(String::from)
    };
    let names: Vec<String> = files.iter().map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect();
    let manifests: Vec<Option<Value>> = files.iter().map(|f| manifest_of(f)).collect();
    let size = |f: &Path| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
    let total: u64 = files.iter().enumerate().map(|(i, f)| size(f) * if entry_for(&manifests[i], &names[i]).is_some() { 2 } else { 1 }).sum();
    let mut done = 0u64;
    let count = files.len();
    for (k, f) in files.iter().enumerate() {
        let name = names[k].clone();
        let len = size(f);
        let want_sum = entry_for(&manifests[k], &name);
        let mut report = |stage: &'static str, done: u64| progress(Progress { file: &name, index: k + 1, count, stage, done, total });
        let skip_bytes = len * if want_sum.is_some() { 2 } else { 1 };
        report("checking", done);
        let info = match depthpack::read_info(f) {
            Ok(i) => i,
            Err(e) => {
                skipped.push(json!({ "file": name, "why": e }));
                done += skip_bytes;
                continue;
            }
        };
        // its checksum against the manifest's
        if let Some(sum) = want_sum {
            let mut h = Sha256::new();
            let mut d = done;
            read_blocks(f, |b| {
                h.update(b);
                d += b.len() as u64;
                report("checking", d);
                Ok(())
            })?;
            done += len;
            let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if got != sum {
                skipped.push(json!({ "file": name, "why": "damaged: its checksum differs from the manifest (copy it again)" }));
                done += len;
                continue;
            }
        }
        // the same package already here
        let mut older = Vec::new();
        let mut newer = None;
        if let Ok(rd) = std::fs::read_dir(to) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tpdepth")) {
                    continue;
                }
                let Ok(have) = depthpack::read_info(&p) else { continue };
                if have.tier == info.tier && have.region == info.region {
                    if have.edition > info.edition {
                        newer = Some(have.edition);
                    } else {
                        older.push(p);
                    }
                }
            }
        }
        if let Some(ed) = newer {
            skipped.push(json!({ "file": name, "why": format!("edition {} is older than the {ed} here", info.edition) }));
            done += len;
            continue;
        }
        let dest = to.join(&name);
        let tmp = to.join(format!("{name}.part"));
        {
            let mut out = std::fs::File::create(&tmp).map_err(|e| format!("{name}: {e}"))?;
            let mut d = done;
            let r = read_blocks(f, |b| {
                std::io::Write::write_all(&mut out, b).map_err(|e| format!("{name}: {e}"))?;
                d += b.len() as u64;
                report("copying", d);
                Ok(())
            });
            if let Err(e) = r {
                drop(out);
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
        done += len;
        for p in older {
            if p != dest {
                let _ = std::fs::remove_file(p);
            }
        }
        std::fs::rename(&tmp, &dest).map_err(|e| format!("{name}: {e}"))?;
        added.push(json!({ "file": name, "region": info.region_name, "edition": info.edition }));
    }
    Ok(json!({ "added": added, "skipped": skipped }))
}

/// Import packages: pick files, or (`folder`) a folder (every .tpdepth in it, checked
/// against its manifest.json).
#[tauri::command]
pub async fn depthpack_import(app: tauri::AppHandle, folder: bool) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let a = app.clone();
    let files: Option<Vec<PathBuf>> = tauri::async_runtime::spawn_blocking(move || {
        if folder {
            let d = a.dialog().file().set_title("Choose the folder with the depth packages (.tpdepth)").blocking_pick_folder()?;
            let d = d.into_path().ok()?;
            let mut v: Vec<PathBuf> =
                std::fs::read_dir(&d).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tpdepth"))).collect();
            v.sort();
            Some(v)
        } else {
            let f = a.dialog().file().set_title("Choose depth packages").add_filter("TweedPlot depth package", &["tpdepth"]).blocking_pick_files()?;
            Some(f.into_iter().filter_map(|p| p.into_path().ok()).collect())
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(files) = files else { return Ok(json!({ "cancelled": true })) };
    if files.is_empty() {
        return Err("no depth packages (.tpdepth) there".into());
    }
    let to = dir(&app);
    // the page shows the import from here (it can take minutes from a USB stick)
    let total: u64 = files.iter().map(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)).sum();
    let _ = app.emit("depthpack-import", json!({ "stage": "start", "count": files.len(), "bytes": total }));
    *PACKS.write().unwrap_or_else(|e| e.into_inner()) = None; // files being replaced are open
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut last_file = 0;
        import_files(&to, &files, &mut |p| {
            // a few times a second, and at each new file
            if p.index != last_file || last.elapsed() >= std::time::Duration::from_millis(200) {
                last = std::time::Instant::now();
                last_file = p.index;
                let _ = a.emit(
                    "depthpack-import",
                    json!({ "stage": p.stage, "file": p.file, "index": p.index, "count": p.count, "done": p.done, "total": p.total }),
                );
            }
        })
    })
    .await
    .map_err(|e| e.to_string());
    open(&app);
    let _ = app.emit("depthpack-import", json!({ "stage": "end" }));
    r?
}

/// Delete a package by file name, or all with "".
#[tauri::command]
pub fn depthpack_delete(app: tauri::AppHandle, file: String) -> Result<(), String> {
    if file.contains(['/', '\\']) || file.contains("..") {
        return Err("bad name".into());
    }
    *PACKS.write().unwrap_or_else(|e| e.into_inner()) = None;
    let mut err = None;
    if let Ok(rd) = std::fs::read_dir(dir(&app)) {
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tpdepth")) && (file.is_empty() || name == file) {
                if let Err(e) = std::fs::remove_file(&p) {
                    err = Some(e.to_string());
                }
            }
        }
    }
    open(&app);
    err.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chart::depthpack::{Writer, N};

    fn pack(dir: &Path, name: &str, edition: &str, region: &str) -> PathBuf {
        let p = dir.join(name);
        let w = Writer::create(&p).unwrap();
        w.set_meta("edition", edition).unwrap();
        w.set_meta("tier", "region").unwrap();
        w.set_meta("region", region).unwrap();
        w.paint(10, 1000, 600, &vec![-12.0; N * N]).unwrap();
        w.finish().unwrap();
        p
    }

    #[test]
    fn import_replaces_older_and_skips_older() {
        let base = std::env::temp_dir().join(format!("tp-dpimp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (src, to) = (base.join("usb"), base.join("depth"));
        std::fs::create_dir_all(&src).unwrap();
        let a = pack(&src, "region-cook-strait-2026-05.tpdepth", "2026-05", "cook-strait");
        let mut seen = Vec::new();
        let r = import_files(&to, &[a], &mut |p| seen.push((p.stage, p.done, p.total))).unwrap();
        assert_eq!(r["added"].as_array().unwrap().len(), 1);
        // told it was checking, then copying, ending with every byte done
        assert_eq!(seen.first().map(|s| s.0), Some("checking"));
        let last = *seen.last().unwrap();
        assert_eq!((last.0, last.1), ("copying", last.2));
        // the newer edition replaces it
        let b = pack(&src, "region-cook-strait-2026-11.tpdepth", "2026-11", "cook-strait");
        import_files(&to, std::slice::from_ref(&b), &mut |_| {}).unwrap();
        let names: Vec<String> = std::fs::read_dir(&to).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["region-cook-strait-2026-11.tpdepth"]);
        // the older one again: skipped
        let a2 = pack(&src, "old-copy.tpdepth", "2026-05", "cook-strait");
        let r = import_files(&to, &[a2], &mut |_| {}).unwrap();
        assert_eq!(r["skipped"].as_array().unwrap().len(), 1);
        // a damaged one (checksum in the manifest differs): skipped
        std::fs::write(src.join("manifest.json"), json!({ "packages": [{ "file": "region-cook-strait-2026-11.tpdepth", "sha256": "00" }] }).to_string())
            .unwrap();
        let r = import_files(&to, &[b], &mut |_| {}).unwrap();
        assert!(r["skipped"][0]["why"].as_str().unwrap().contains("damaged"));
        // not a package
        std::fs::write(src.join("x.tpdepth"), b"nope").unwrap();
        let r = import_files(&to, &[src.join("x.tpdepth")], &mut |_| {}).unwrap();
        assert_eq!(r["skipped"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }
}
