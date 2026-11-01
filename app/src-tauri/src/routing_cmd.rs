//! Polars and the weather routing (route_compute).

use super::*;

/// Your own polars: `RACENAV_POLAR_DIR`, else `C:\dev\racenav-data\polars` when
/// racenav-data exists, else `<app data>\polars` (made on first use).
pub(crate) fn user_polar_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_POLAR_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    let d = if dev.is_dir() { dev.join("polars") } else { app.path().app_data_dir().map(|p| p.join("polars")).unwrap_or_else(|_| PathBuf::from("polars")) };
    let _ = std::fs::create_dir_all(&d);
    d
}

/// The polars shipped with TweedPlot (read only).
pub(crate) fn bundled_polar_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let candidates = [
        app.path().resource_dir().ok().map(|d| d.join("resources").join("polars")),
        app.path().resource_dir().ok().map(|d| d.join("polars")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("polars")),
    ];
    candidates.into_iter().flatten().find(|d| d.is_dir())
}

/// Polar folders, yours first: a polar of yours with the same name as a shipped one is used instead.
pub(crate) fn polar_dirs(app: &tauri::AppHandle) -> Vec<(PathBuf, &'static str)> {
    let mut v = vec![(user_polar_dir(app), "yours")];
    if let Some(b) = bundled_polar_dir(app) {
        v.push((b, "shipped"));
    }
    v.retain(|d| d.0.is_dir());
    v
}

/// Boat speed from a polar file (cached) at a TWA and TWS, kn.
#[tauri::command]
pub(crate) fn polar_speed(path: String, twa: f64, tws: f64) -> Option<f64> {
    static CACHE: Mutex<Option<(String, Arc<polar::Polar>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    if c.as_ref().map(|x| x.0 != path).unwrap_or(true) {
        *c = Some((path.clone(), Arc::new(polar::Polar::from_path(&path).ok()?)));
    }
    let p = c.as_ref()?.1.clone();
    let b = p.bsp(twa.abs(), tws);
    b.is_finite().then_some((b * 100.0).round() / 100.0)
}

#[tauri::command]
pub(crate) fn polar_files(app: tauri::AppHandle) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for (dir, source) in polar_dirs(&app) {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "csv" | "pol" | "txt") {
                continue;
            }
            // only files that actually parse as polars
            if polar::Polar::from_path(&p).is_err() {
                continue;
            }
            let name = p.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            if out.iter().any(|o| o["name"].as_str() == Some(name.as_str())) {
                continue;
            }
            out.push(serde_json::json!({ "path": p.to_string_lossy(), "name": name, "dir": dir, "source": source }));
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

/// Check a polar file's text and copy it into your polar folder. Returns the saved path.
#[tauri::command]
pub(crate) fn polar_import(app: tauri::AppHandle, name: String, text: String) -> Result<String, String> {
    let stem: String = std::path::Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() || " ._-".contains(c) { c } else { '_' })
        .collect();
    let stem = stem.trim().to_string();
    if stem.is_empty() {
        return Err("The file has no usable name.".into());
    }
    polar::Polar::parse(&stem, &text).map_err(|e| format!("Not a polar TweedPlot can read ({e}). See Manual > Reference > Polars for the format."))?;
    let ext = std::path::Path::new(&name).extension().and_then(|x| x.to_str()).unwrap_or("pol").to_ascii_lowercase();
    let ext = if matches!(ext.as_str(), "csv" | "pol" | "txt") { ext } else { "pol".into() };
    let dest = user_polar_dir(&app).join(format!("{stem}.{ext}"));
    std::fs::write(&dest, text).map_err(|e| format!("Could not save {}: {e}", dest.display()))?;
    Ok(dest.to_string_lossy().into_owned())
}

/// Open your polar folder in Explorer.
#[tauri::command]
pub(crate) fn polar_open_folder(app: tauri::AppHandle) -> Result<String, String> {
    let d = user_polar_dir(&app);
    #[cfg(windows)]
    std::process::Command::new("explorer").arg(&d).spawn().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(&d).spawn().map_err(|e| e.to_string())?;
    #[cfg(all(unix, not(target_os = "macos")))]
    std::process::Command::new("xdg-open").arg(&d).spawn().map_err(|e| e.to_string())?;
    Ok(d.to_string_lossy().into_owned())
}

#[derive(serde::Deserialize)]
pub(crate) struct RouteRequest {
    /// [lon, lat]
    start: [f64; 2],
    marks: Vec<[f64; 2]>,
    /// Unix seconds UTC
    t0: i64,
    grib_path: String,
    polar_path: String,
    safety_depth: f64,
    /// Rocks, wrecks, obstructions and marine farms to keep off.
    #[serde(default)]
    avoid: routing::Avoid,
    #[serde(default)]
    params: routing::Params,
    /// Per mark (same order as `marks`): rounding "port" | "stbd" | "close" | "" and radius NM (0 = default).
    #[serde(default)]
    mark_opts: Vec<MarkOpt>,
    /// Keep off the library's active barriers and barrier marks.
    #[serde(default = "yes")]
    use_barriers: bool,
}

pub(crate) fn yes() -> bool {
    true
}

#[derive(serde::Deserialize, Default, Clone)]
#[serde(default)]
pub(crate) struct MarkOpt {
    rounding: String,
    radius_nm: f64,
}

/// Route from start through the marks with one loaded GRIB, a polar and the chart.
#[tauri::command]
pub(crate) async fn route_compute(req: RouteRequest) -> Result<serde_json::Value, String> {
    let grib = GRIBS.lock().unwrap().get(&req.grib_path).cloned().ok_or("Load the GRIB first (Weather panel).")?;
    tauri::async_runtime::spawn_blocking(move || {
        let polar = polar::Polar::from_path(&req.polar_path).map_err(|e| format!("Polar: {e}"))?;
        let start = nav_core::LatLon::new(req.start[1], req.start[0]);
        let marks: Vec<nav_core::LatLon> = req.marks.iter().map(|m| nav_core::LatLon::new(m[1], m[0])).collect();
        if marks.is_empty() {
            return Err("Add at least one mark.".to_string());
        }
        let specs: Vec<routing::MarkSpec> = marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let o = req.mark_opts.get(i).cloned().unwrap_or_default();
                routing::MarkSpec { pos: *m, rounding: routing::Rounding::parse(&o.rounding), radius_nm: (o.radius_nm > 0.0).then_some(o.radius_nm) }
            })
            .collect();
        // barriers: active lines and zones, and barrier marks that aren't on this course
        let mut barriers = routing::Barriers::default();
        let mut barrier_count = 0;
        if req.use_barriers {
            if let Ok(l) = lib() {
                for b in l.barriers().unwrap_or_default().into_iter().filter(|b| b.active && b.points.len() >= 2) {
                    barrier_count += 1;
                    if b.kind == "zone" && b.points.len() >= 3 {
                        barriers.zones.push(b.points)
                    } else {
                        barriers.lines.push(b.points)
                    }
                }
                let on_course = |m: &library::Mark| {
                    let p = nav_core::LatLon::new(m.lat, m.lon);
                    marks.iter().chain(std::iter::once(&start)).any(|c| c.distance_to(&p) < 0.01)
                };
                for m in l.marks().unwrap_or_default().into_iter().filter(|m| m.barrier_nm > 0.0 && !on_course(m)) {
                    barrier_count += 1;
                    barriers.discs.push(([m.lon, m.lat], m.barrier_nm));
                }
            }
        }
        // waves for the wave-height limit: this GRIB, else a loaded wave GRIB covering the start
        let wave_grib: Option<Arc<grib::GribFile>> = if req.params.max_hs > 0.0 && grib.times(grib::Param::WaveHeight).is_empty() {
            let g = GRIBS.lock().unwrap();
            g.values()
                .filter(|f| {
                    f.times(grib::Param::WaveHeight).first().is_some_and(|&a| a <= req.t0)
                        && f.times(grib::Param::WaveHeight).last().is_some_and(|&b| b >= req.t0)
                })
                .max_by_key(|f| f.model().1)
                .cloned()
        } else {
            None
        };
        let clock = std::time::Instant::now();
        let raster = charts().map(|store| {
            let mut b = [start.lon, start.lat, start.lon, start.lat];
            for m in &marks {
                b = [b[0].min(m.lon), b[1].min(m.lat), b[2].max(m.lon), b[3].max(m.lat)];
            }
            // room to go round headlands: a quarter of the span, at least 0.1 degree
            let pad = ((b[2] - b[0]).max(b[3] - b[1]) * 0.25).clamp(0.1, 2.0);
            routing::Raster::from_chart_with(&store, [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad], req.safety_depth, 50.0, 16_000_000, req.avoid)
        });
        let raster_ms = clock.elapsed().as_millis();
        let r = match &wave_grib {
            Some(w) => {
                let src = routing::WithWaves { wind: grib.as_ref(), waves: w.as_ref() };
                routing::route_marks(start, req.t0, &specs, &polar, &src, raster.as_ref(), &barriers, &req.params)
            }
            None => routing::route_marks(start, req.t0, &specs, &polar, grib.as_ref(), raster.as_ref(), &barriers, &req.params),
        };
        let mut v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
        v["barriers"] = barrier_count.into();
        v["waves_from"] = match (&wave_grib, req.params.max_hs > 0.0) {
            (Some(w), _) => w.model().0.into(),
            (None, true) if !grib.times(grib::Param::WaveHeight).is_empty() => "the same GRIB".into(),
            (None, true) => serde_json::Value::String("none: no wave GRIB loaded, the wave limit was not applied".into()),
            _ => serde_json::Value::Null,
        };
        v["obstacles"] = serde_json::json!({
            "cell_m": raster.as_ref().map(|r| r.cell_size_m().round()),
            "build_ms": raster_ms,
            "charts": raster.as_ref().is_some_and(|r| r.any_blocked()),
        });
        Ok(v)
    })
    .await
    .map_err(|e| e.to_string())?
}
