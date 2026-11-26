//! The mark, route and track library: which library, its contents, the fishing log, trips,
//! Trash, export and merge, libraries alongside, and backups.

use super::*;

// ---------- mark and route library ----------

/// Library folder: `RACENAV_LIBRARY_DIR`, else `C:\dev\racenav-data\library` when
/// racenav-data exists, else `<app data>\library`.
pub(crate) fn library_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(d) = std::env::var("RACENAV_LIBRARY_DIR") {
        return PathBuf::from(d);
    }
    let dev = dev_root();
    if dev.is_dir() {
        return dev.join("library");
    }
    app.path().app_data_dir().map(|p| p.join("library")).unwrap_or_else(|_| PathBuf::from("library"))
}

pub(crate) fn lib() -> Result<Arc<library::Library>, String> {
    LIBRARY.read().unwrap_or_else(|e| e.into_inner()).clone().ok_or_else(|| "The mark library could not be opened.".to_string())
}

// ---------- which library: New, Open, Recent, Save a copy, Merge ----------
// The library in use is remembered in library.json in the app's data folder, with the
// ones used recently. The default is <library folder>\library.gpkg.

pub(crate) fn library_settings_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("library.json"))
}

pub(crate) fn library_settings(app: &tauri::AppHandle) -> serde_json::Value {
    library_settings_file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// The library file to open at start: the one last used, if it's still there.
pub(crate) fn library_start_file(app: &tauri::AppHandle) -> PathBuf {
    let v = library_settings(app);
    v["current"].as_str().map(PathBuf::from).filter(|p| p.is_file()).unwrap_or_else(|| library_dir(app).join("library.gpkg"))
}

pub(crate) fn library_remember(app: &tauri::AppHandle, path: &std::path::Path) {
    let v = library_settings(app);
    let mut recent: Vec<String> = v["recent"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let p = path.to_string_lossy().to_string();
    recent.retain(|x| x != &p);
    recent.insert(0, p.clone());
    recent.truncate(8);
    if let Some(f) = library_settings_file(app) {
        let _ = std::fs::write(f, serde_json::json!({ "current": p, "recent": recent }).to_string());
    }
}

// ---------- one TimeZero symbol table for every library ----------
// What each TimeZero symbol number means is how the skipper (or the fleet) works, not part
// of one library, so the app keeps one table (tz-symbols.json in the app's data folder) and
// every library opened is brought into line with it. The first time, the table is the library
// in use's own; meanings a library has that the table lacks are added to it.

pub(crate) fn symbol_table_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("tz-symbols.json"))
}

pub(crate) fn symbol_table_read(app: &tauri::AppHandle) -> Option<Vec<(String, String, String)>> {
    symbol_table_file(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok())
}

pub(crate) fn symbol_table_write(app: &tauri::AppHandle, t: &[(String, String, String)]) {
    if let (Some(f), Ok(j)) = (symbol_table_file(app), serde_json::to_string_pretty(t)) {
        let _ = std::fs::write(f, j);
    }
}

/// Bring the library in use into line with the app's symbol table; returns marks changed.
pub(crate) fn symbols_sync(app: &tauri::AppHandle) -> Result<usize, String> {
    let l = lib()?;
    let own = l.symbol_table().map_err(es)?;
    let mut table = symbol_table_read(app).unwrap_or_default();
    let before = table.len();
    for e in own {
        if !table.iter().any(|t| t.0 == e.0) {
            table.push(e);
        }
    }
    if table.len() != before || symbol_table_file(app).is_some_and(|f| !f.is_file()) {
        table.sort();
        symbol_table_write(app, &table);
    }
    l.apply_symbol_table(&table).map_err(es)
}

/// Make `path` the library in use (opening, or making, it).
pub(crate) fn library_switch(app: &tauri::AppHandle, path: &std::path::Path) -> Result<(), String> {
    if REC.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        return Err("The track recorder is on: stop recording before changing library.".into());
    }
    let l = library::Library::open_file(path).map_err(es)?;
    *LIBRARY.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(l));
    library_remember(app, path);
    if let Err(e) = symbols_sync(app) {
        eprintln!("symbol table: {e}");
    }
    library_changed(app);
    Ok(())
}

/// The library in use: its name, file, and the ones used recently.
#[tauri::command]
pub(crate) fn library_current(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let l = lib()?;
    let v = library_settings(&app);
    let recent: Vec<serde_json::Value> = v["recent"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .filter(|p| std::path::Path::new(p).is_file() && std::path::Path::new(p) != l.path())
                .map(|p| serde_json::json!({ "path": p, "name": std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() }))
                .collect()
        })
        .unwrap_or_default();
    Ok(serde_json::json!({ "name": l.name(), "path": l.path(), "dir": l.dir(), "recent": recent }))
}

/// Pick a library file: to open (`save` false) or to make or save a copy to (`save` true).
#[tauri::command]
pub(crate) async fn library_pick_db(
    app: tauri::AppHandle,
    save: bool,
    title: String,
    suggest: Option<String>,
    protected: Option<bool>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let protected = protected.unwrap_or(false);
    let ext = if protected { library::protect::EXT } else { "gpkg" };
    let dir = lib().ok().map(|l| l.dir().to_path_buf());
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut d = app.dialog().file().set_title(&title);
        d = if save {
            d.add_filter(if protected { "Protected TweedPlot library" } else { "TweedPlot library" }, &[ext])
        } else {
            d.add_filter("TweedPlot library", &["gpkg", library::protect::EXT])
        };
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
        if let Some(s) = suggest {
            d = d.set_file_name(&s);
        }
        if save {
            d.blocking_save_file()
        } else {
            d.blocking_pick_file()
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(p) = picked else { return Ok(None) };
    let mut p = p.into_path().map_err(|e| e.to_string())?;
    if save && p.extension().is_none_or(|x| !x.eq_ignore_ascii_case(ext)) {
        p.set_extension(ext);
    }
    Ok(Some(p.to_string_lossy().into_owned()))
}

/// Open a library (or make a new one at `path`) and use it.
#[tauri::command]
pub(crate) async fn library_use(app: tauri::AppHandle, path: String, new: bool) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if new && p.exists() {
        return Err(format!("{path} is already there: open it instead, or choose another name"));
    }
    if !new && !p.is_file() {
        return Err(format!("{path} isn't there"));
    }
    let h = app.clone();
    tauri::async_runtime::spawn_blocking(move || library_switch(&h, &p)).await.map_err(|e| e.to_string())?
}

/// A copy of the whole library in use, at `path` (it stays the one in use).
#[tauri::command]
pub(crate) async fn library_save_copy(path: String) -> Result<(), String> {
    let l = lib()?;
    tauri::async_runtime::spawn_blocking(move || l.backup_to(std::path::Path::new(&path)).map_err(es)).await.map_err(|e| e.to_string())?
}

/// Bring another library's marks, routes, tracks, logs, trips, routings and barriers into
/// the one in use: all of them, or the categories in `what`.
#[tauri::command]
pub(crate) async fn library_merge(
    app: tauri::AppHandle,
    path: String,
    job: Option<String>,
    what: Option<library::Scope>,
) -> Result<library::ImportReport, String> {
    let job = job.unwrap_or_default();
    let cancel = job_begin(&job);
    let l = lib()?;
    let h = app.clone();
    let j = job.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = job_progress(&h, &j);
        l.merge_from_scope(std::path::Path::new(&path), what.unwrap_or_default(), &mut progress, &cancel).map_err(es)
    })
    .await
    .map_err(|e| e.to_string());
    job_end(&job);
    let r = r??;
    let _ = symbols_sync(&app);
    library_changed(&app);
    Ok(r)
}

/// What merging `path` (the categories in `what`) would bring in, changing nothing: the preview
/// before a merge.
#[tauri::command]
pub(crate) async fn library_merge_preview(path: String, what: Option<library::Scope>) -> Result<library::ImportReport, String> {
    let l = lib()?;
    tauri::async_runtime::spawn_blocking(move || l.merge_preview(std::path::Path::new(&path), what.unwrap_or_default()).map_err(es))
        .await
        .map_err(|e| e.to_string())?
}

/// What a library file holds, to choose from before bringing it in: how many of each
/// category, and the own soundings.
#[tauri::command]
pub(crate) async fn library_file_contents(path: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = PathBuf::from(&path);
        let n = library::Library::peek(&p).map_err(es)?;
        let mut v = serde_json::to_value(&n).map_err(|e| e.to_string())?;
        v["soundings"] = serde_json::to_value(bathy::share::file_info(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(v)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The effort map of the tracks `ids` (library/density.rs): [lon, lat, minutes] per cell.
#[tauri::command]
pub(crate) async fn tracks_density(
    app: tauri::AppHandle,
    ids: Vec<String>,
    cell: f64,
    sog_min: f64,
    sog_max: f64,
    job: Option<String>,
) -> Result<Vec<[f64; 3]>, String> {
    let job = job.unwrap_or_default();
    let cancel = job_begin(&job);
    let l = lib()?;
    let h = app.clone();
    let j = job.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = job_progress(&h, &j);
        l.track_density(&ids, cell, sog_min, sog_max, &mut progress, &cancel).map_err(es)
    })
    .await
    .map_err(|e| e.to_string());
    job_end(&job);
    r?
}

// ---------- the fishing log ----------

#[tauri::command]
pub(crate) fn events_list() -> Result<Vec<library::Event>, String> {
    lib()?.events().map_err(es)
}
#[tauri::command]
pub(crate) fn event_save(app: tauri::AppHandle, event: library::Event) -> Result<library::Event, String> {
    let e = lib()?.save_event(event).map_err(es)?;
    let _ = app.emit("events-changed", ());
    Ok(e)
}
#[tauri::command]
pub(crate) fn events_delete(app: tauri::AppHandle, ids: Vec<String>) -> Result<usize, String> {
    let n = lib()?.delete_events(&ids).map_err(es)?;
    let _ = app.emit("events-changed", ());
    Ok(n)
}
/// Events as CSV in the export folder; returns the file.
#[tauri::command]
pub(crate) fn events_csv(name: String, ids: Vec<String>) -> Result<String, String> {
    lib()?.export_events_csv(&name, &ids).map(|p| p.to_string_lossy().into_owned()).map_err(es)
}

// ---------- trips ----------

#[tauri::command]
pub(crate) fn trips_list() -> Result<Vec<library::Trip>, String> {
    rec_flush();
    lib()?.trips().map_err(es)
}
/// Save a trip; `record` starts the track recorder (a new track) if it isn't on.
#[tauri::command]
pub(crate) fn trip_save(app: tauri::AppHandle, trip: library::Trip, record: bool) -> Result<library::Trip, String> {
    let t = lib()?.save_trip(trip).map_err(es)?;
    if record && t.ended.is_none() && REC.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        rec_flush();
        rec_begin(None)?;
    }
    library_changed(&app);
    Ok(t)
}
/// End a trip now; `stop_recording` stops the track recorder too.
#[tauri::command]
pub(crate) fn trip_end(app: tauri::AppHandle, id: i64, stop_recording: bool) -> Result<library::Trip, String> {
    if stop_recording {
        rec_stop();
    } else {
        rec_flush();
    }
    let t = lib()?.end_trip(id, None).map_err(es)?;
    library_changed(&app);
    Ok(t)
}
#[tauri::command]
pub(crate) fn trips_delete(app: tauri::AppHandle, ids: Vec<i64>) -> Result<usize, String> {
    let n = lib()?.delete_trips(&ids).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
/// The marks and tracks made in a trip: { marks: [ids], tracks: [ids] }.
#[tauri::command]
pub(crate) fn trip_contents(id: i64) -> Result<serde_json::Value, String> {
    let l = lib()?;
    let (marks, tracks) = l.trip_contents(id).map_err(es)?;
    let events = l.trip_events(id).map_err(es)?;
    Ok(serde_json::json!({ "marks": marks, "tracks": tracks, "events": events }))
}

// ---------- Trash, undoing an import, a part of the library as a file ----------

#[tauri::command]
pub(crate) fn library_trash() -> Result<Vec<library::TrashItem>, String> {
    lib()?.trash().map_err(es)
}
/// Bring things back from the Trash; `kind` is mark, route, track, routing or barrier.
#[tauri::command]
pub(crate) fn library_restore(app: tauri::AppHandle, kind: String, ids: Vec<String>) -> Result<usize, String> {
    let n = lib()?.restore(&kind, &ids).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
/// Delete things in the Trash for good (no ids: all of that kind).
#[tauri::command]
pub(crate) fn library_purge(kind: String, ids: Vec<String>) -> Result<usize, String> {
    lib()?.purge(&kind, &ids).map_err(es)
}
#[tauri::command]
pub(crate) fn library_imports() -> Result<Vec<library::ImportInfo>, String> {
    lib()?.imports().map_err(es)
}
#[tauri::command]
pub(crate) fn library_undo_import(app: tauri::AppHandle, id: i64) -> Result<library::Counts, String> {
    let n = lib()?.undo_import(id).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
/// The chosen marks, routes, tracks and logs as a library file of their own, with whole
/// categories of trips, routings and barriers, and the own soundings, as asked.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn library_export(
    app: tauri::AppHandle,
    path: String,
    marks: Vec<String>,
    routes: Vec<String>,
    tracks: Vec<String>,
    events: Option<Vec<String>>,
    notes: Option<bool>,
    passphrase: Option<String>,
    trips: Option<bool>,
    routings: Option<bool>,
    barriers: Option<bool>,
    soundings: Option<bool>,
    job: Option<String>,
) -> Result<serde_json::Value, String> {
    let events = events.unwrap_or_default();
    let also = library::Also { trips: trips.unwrap_or(false), routings: routings.unwrap_or(false), barriers: barriers.unwrap_or(false) };
    let want_soundings = soundings.unwrap_or(false);
    let job = job.unwrap_or_default();
    let cancel = job_begin(&job);
    let job_id = job.clone();
    let l = lib()?;
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = job_progress(&app, &job);
        let to = PathBuf::from(&path);
        let keep_notes = notes.unwrap_or(true);
        let pass = passphrase.filter(|p| !p.is_empty());
        // a protected file is written plain beside it first, then locked; the plain copy goes
        let target = if pass.is_some() { to.with_extension("gpkg.plain") } else { to.clone() };
        let r = (|| -> Result<serde_json::Value, String> {
            let n = l.export_library(&target, &marks, &routes, &tracks, &events, also, keep_notes).map_err(es)?;
            let cells = if want_soundings {
                pbathy::export_into(&target, &mut progress, &cancel).inspect_err(|_| {
                    let _ = std::fs::remove_file(&target);
                })?
            } else {
                0
            };
            if let Some(pass) = &pass {
                library::protect::protect_file(&target, &to, pass).map_err(es)?;
            }
            let mut v = serde_json::to_value(&n).map_err(|e| e.to_string())?;
            v["soundings"] = cells.into();
            Ok(v)
        })();
        if pass.is_some() {
            let _ = std::fs::remove_file(&target);
        }
        r
    })
    .await
    .map_err(|e| e.to_string());
    job_end(&job_id);
    r?
}

/// Unlock a protected library (.tplib) with its passphrase, as a plain library file:
/// `keep` puts it beside the protected one (to open as a library), else in the app's own
/// folder (to merge, or show alongside). Returns the plain file.
#[tauri::command]
pub(crate) async fn library_unlock(app: tauri::AppHandle, path: String, passphrase: String, keep: bool) -> Result<String, String> {
    let src = PathBuf::from(&path);
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "library".into());
    let dir = if keep {
        src.parent().map(PathBuf::from).unwrap_or_else(|| library_dir(&app))
    } else {
        app.path().app_data_dir().map_err(|e| e.to_string())?.join("unlocked")
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut to = dir.join(format!("{stem}.gpkg"));
    let mut k = 2;
    while keep && to.exists() {
        to = dir.join(format!("{stem} ({k}).gpkg"));
        k += 1;
    }
    let t = to.clone();
    tauri::async_runtime::spawn_blocking(move || library::protect::unprotect_file(&src, &t, &passphrase).map_err(es)).await.map_err(|e| e.to_string())??;
    Ok(to.to_string_lossy().into_owned())
}

/// Is this a protected library (needing a passphrase)?
#[tauri::command]
pub(crate) fn library_is_protected(path: String) -> bool {
    library::protect::is_protected(std::path::Path::new(&path))
}

// ---------- other libraries shown alongside, read-only ----------
pub(crate) static OVERLAYS: std::sync::LazyLock<Mutex<Vec<Arc<library::Overlay>>>> = std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

pub(crate) fn overlay(path: &str) -> Result<Arc<library::Overlay>, String> {
    OVERLAYS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|o| o.path() == std::path::Path::new(path))
        .cloned()
        .ok_or_else(|| format!("{path} isn't shown alongside"))
}

/// Show a library alongside (read-only); returns its name and what's in it.
#[tauri::command]
pub(crate) async fn overlay_add(path: String) -> Result<serde_json::Value, String> {
    let p = PathBuf::from(&path);
    if lib().is_ok_and(|l| l.path().canonicalize().ok() == p.canonicalize().ok()) {
        return Err("that's the library in use".into());
    }
    let o = match overlay(&path) {
        Ok(o) => o,
        Err(_) => {
            let o = Arc::new(tauri::async_runtime::spawn_blocking(move || library::Overlay::open(&p).map_err(es)).await.map_err(|e| e.to_string())??);
            OVERLAYS.lock().unwrap_or_else(|e| e.into_inner()).push(o.clone());
            o
        }
    };
    Ok(serde_json::json!({ "path": path, "name": o.name() }))
}
#[tauri::command]
pub(crate) fn overlay_remove(path: String) {
    OVERLAYS.lock().unwrap_or_else(|e| e.into_inner()).retain(|o| o.path() != std::path::Path::new(&path));
}
#[tauri::command]
pub(crate) async fn overlay_marks(path: String) -> Result<Vec<library::Mark>, String> {
    let o = overlay(&path)?;
    tauri::async_runtime::spawn_blocking(move || o.marks().map_err(es)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn overlay_tracks(path: String) -> Result<Vec<library::TrackSummary>, String> {
    let o = overlay(&path)?;
    tauri::async_runtime::spawn_blocking(move || o.tracks().map_err(es)).await.map_err(|e| e.to_string())?
}
/// One of an overlay's tracks, as `track_get` gives one of the library's.
#[tauri::command]
pub(crate) async fn overlay_track(path: String, id: String) -> Result<serde_json::Value, String> {
    let o = overlay(&path)?;
    let t = tauri::async_runtime::spawn_blocking(move || o.track(&id).map_err(es)).await.map_err(|e| e.to_string())??;
    let segs: Vec<Vec<serde_json::Value>> =
        t.segments.iter().map(|s| s.iter().map(|p| serde_json::json!([p.lon, p.lat, p.t, p.sog, p.cog, p.depth, p.temp])).collect()).collect();
    Ok(serde_json::json!({ "id": t.id, "name": t.name, "colour": t.colour, "stats": t.stats, "segments": segs }))
}

/// The library's own backups (newest first): file, date, size.
#[tauri::command]
pub(crate) fn library_backups() -> Result<Vec<serde_json::Value>, String> {
    let l = lib()?;
    let dir = l.dir().join("backups");
    let prefix = format!("{}-", l.name());
    let mut v: Vec<(String, PathBuf, u64)> = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    let day = n.strip_prefix(prefix.as_str())?.strip_suffix(".gpkg")?.to_string();
                    (day.len() == 10).then(|| (day, e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(v.into_iter().map(|(d, p, n)| serde_json::json!({ "day": d, "path": p, "bytes": n })).collect())
}

/// Where the library's own backups are written (made if missing), and open it in Explorer if `open`.
#[tauri::command]
pub(crate) fn library_backup_dir(open: bool) -> Result<String, String> {
    let d = lib()?.dir().join("backups");
    let _ = std::fs::create_dir_all(&d);
    if open {
        #[cfg(windows)]
        std::process::Command::new("explorer").arg(&d).spawn().map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        std::process::Command::new("open").arg(&d).spawn().map_err(|e| e.to_string())?;
        #[cfg(all(unix, not(target_os = "macos")))]
        std::process::Command::new("xdg-open").arg(&d).spawn().map_err(|e| e.to_string())?;
    }
    Ok(d.to_string_lossy().into_owned())
}

// ---------- Back up to…: a copy somewhere else (a USB stick, another disk) ----------
// The folder chosen is remembered (backup-to.json in app data), with when each library was
// last copied there, so the menu can offer "Back up to <folder>" and say how long ago.

pub(crate) fn backup_to_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("backup-to.json"))
}
pub(crate) fn backup_to_read(app: &tauri::AppHandle) -> serde_json::Value {
    backup_to_file(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}))
}

/// The folder last backed up to (if any) and when the library in use was last copied out.
#[tauri::command]
pub(crate) fn library_backup_to_info(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let l = lib()?;
    let v = backup_to_read(&app);
    let key = l.path().to_string_lossy().into_owned();
    let dir = v["dir"].as_str().map(str::to_string);
    let reachable = dir.as_deref().is_some_and(|d| std::path::Path::new(d).is_dir());
    Ok(serde_json::json!({ "dir": dir, "reachable": reachable, "last": v["last"][&key].as_i64() }))
}

/// Copy the library in use to `dir` (or a folder picked now) as "<name> YYYY-MM-DD HHMM.gpkg".
/// Returns the file written, or null if no folder was picked.
#[tauri::command]
pub(crate) async fn library_backup_to(app: tauri::AppHandle, dir: Option<String>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let l = lib()?;
    let dir = match dir {
        Some(d) => PathBuf::from(d),
        None => {
            let mut d = app.dialog().file().set_title("Back up the library to (a USB stick, another disk)");
            if let Some(prev) = backup_to_read(&app)["dir"].as_str() {
                d = d.set_directory(prev);
            }
            let Some(p) = d.blocking_pick_folder() else { return Ok(None) };
            p.into_path().map_err(|e| e.to_string())?
        }
    };
    if !dir.is_dir() {
        return Err(format!("{} isn't there (is the USB stick plugged in?)", dir.display()));
    }
    if dir.canonicalize().ok() == l.dir().canonicalize().ok() {
        return Err("that's the library's own folder: choose another disk, or a USB stick".into());
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M").to_string();
    let to = dir.join(format!("{} {stamp}.gpkg", l.name()));
    let t = to.clone();
    let ll = l.clone();
    tauri::async_runtime::spawn_blocking(move || ll.backup_to(&t).map_err(es)).await.map_err(|e| e.to_string())??;
    let mut v = backup_to_read(&app);
    v["dir"] = serde_json::json!(dir.to_string_lossy());
    v["last"][l.path().to_string_lossy().as_ref()] = serde_json::json!(chrono::Utc::now().timestamp());
    if let Some(f) = backup_to_file(&app) {
        let _ = std::fs::write(f, v.to_string());
    }
    Ok(Some(to.to_string_lossy().into_owned()))
}

/// Use a backup: copied (the backup stays as it is) beside the library as
/// "<name> restored <day>.gpkg", which becomes the library in use.
#[tauri::command]
pub(crate) async fn library_use_backup(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let l = lib()?;
    let src = PathBuf::from(&path);
    let day = src.file_stem().map(|s| s.to_string_lossy().chars().rev().take(10).collect::<String>().chars().rev().collect::<String>()).unwrap_or_default();
    let mut to = l.dir().join(format!("{} restored {day}.gpkg", l.name()));
    let mut k = 2;
    while to.exists() {
        to = l.dir().join(format!("{} restored {day} ({k}).gpkg", l.name()));
        k += 1;
    }
    std::fs::copy(&src, &to).map_err(|e| e.to_string())?;
    let h = app.clone();
    let t = to.clone();
    tauri::async_runtime::spawn_blocking(move || library_switch(&h, &t)).await.map_err(|e| e.to_string())??;
    Ok(to.to_string_lossy().into_owned())
}

/// Change many marks at once (group, type, colour; empty or missing: unchanged).
#[tauri::command]
pub(crate) fn marks_update(
    app: tauri::AppHandle,
    ids: Vec<String>,
    group: Option<String>,
    kind: Option<String>,
    colour: Option<String>,
) -> Result<usize, String> {
    let n = lib()?.update_marks(&ids, group.as_deref(), kind.as_deref().filter(|k| !k.is_empty()), colour.as_deref()).map_err(es)?;
    library_changed(&app);
    Ok(n)
}

/// Several tracks in one GPX file in the export folder; returns its path.
#[tauri::command]
pub(crate) fn tracks_export(name: String, ids: Vec<String>) -> Result<String, String> {
    rec_flush();
    let l = lib()?;
    let text = l.export_tracks_gpx(&ids).map_err(es)?;
    Ok(l.write_export(&name, &text).map_err(es)?.to_string_lossy().into_owned())
}

/// Say what the depths of tracks are measured below: "surface", "transducer" (offset: the
/// transducer's depth), "keel" (offset: the draft), "datum" (already reduced to chart datum).
#[tauri::command]
pub(crate) fn tracks_set_depth_ref(app: tauri::AppHandle, ids: Vec<String>, kind: String, offset: Option<f64>) -> Result<usize, String> {
    let n = lib()?.set_depth_ref(&ids, &kind, offset.filter(|o| o.is_finite())).map_err(es)?;
    library_changed(&app);
    Ok(n)
}

/// Tracks with depths that don't say what they're measured below: of the newest import, or all.
#[tauri::command]
pub(crate) fn tracks_depth_unknown(newest_import: bool) -> Result<Vec<String>, String> {
    lib()?.depth_unknown(newest_import).map_err(es)
}

pub(crate) fn es(e: library::Error) -> String {
    e.to_string()
}

#[tauri::command]
pub(crate) fn barriers_list() -> Result<Vec<library::Barrier>, String> {
    lib()?.barriers().map_err(es)
}

#[tauri::command]
pub(crate) fn barrier_save(barrier: library::Barrier) -> Result<library::Barrier, String> {
    lib()?.save_barrier(barrier).map_err(es)
}

#[tauri::command]
pub(crate) fn barrier_delete(id: String) -> Result<(), String> {
    lib()?.delete_barrier(&id).map_err(es)
}

/// Tell every window the library changed, so each reloads its marks, routes and routings.
pub(crate) fn library_changed(app: &tauri::AppHandle) {
    let _ = app.emit("library-changed", ());
}

#[tauri::command]
pub(crate) fn marks_list() -> Result<Vec<library::Mark>, String> {
    lib()?.marks().map_err(es)
}
#[tauri::command]
pub(crate) fn mark_save(app: tauri::AppHandle, mark: library::Mark) -> Result<library::Mark, String> {
    let m = lib()?.save_mark(mark).map_err(es)?;
    library_changed(&app);
    Ok(m)
}
#[tauri::command]
pub(crate) fn mark_delete(app: tauri::AppHandle, id: String, force: bool) -> Result<(), String> {
    lib()?.delete_mark(&id, force).map_err(es)?;
    library_changed(&app);
    Ok(())
}
/// The TimeZero symbols the marks use, with what each has been given to mean.
#[tauri::command]
pub(crate) fn symbols_list() -> Result<Vec<library::SymbolUse>, String> {
    lib()?.symbols().map_err(es)
}
/// Give a TimeZero symbol a meaning (a mark type and label); returns how many marks changed.
#[tauri::command]
pub(crate) fn symbol_set(app: tauri::AppHandle, symbol: String, kind: String, label: String) -> Result<usize, String> {
    let n = lib()?.set_symbol(&symbol, &kind, &label).map_err(es)?;
    // the one table every library uses
    let mut table = symbol_table_read(&app).unwrap_or_default();
    table.retain(|t| t.0 != symbol);
    table.push((symbol, kind, label.trim().to_string()));
    table.sort();
    symbol_table_write(&app, &table);
    library_changed(&app);
    Ok(n)
}
#[tauri::command]
pub(crate) fn marks_delete(app: tauri::AppHandle, ids: Vec<String>, force: bool) -> Result<usize, String> {
    let n = lib()?.delete_marks(&ids, force).map_err(es)?;
    library_changed(&app);
    Ok(n)
}
#[tauri::command]
pub(crate) fn routes_list() -> Result<Vec<library::Route>, String> {
    lib()?.routes().map_err(es)
}
#[tauri::command]
pub(crate) fn route_save(app: tauri::AppHandle, route: library::Route) -> Result<library::Route, String> {
    let r = lib()?.save_route(route).map_err(es)?;
    library_changed(&app);
    Ok(r)
}
#[tauri::command]
pub(crate) fn route_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    lib()?.delete_route(&id).map_err(es)?;
    library_changed(&app);
    Ok(())
}
#[tauri::command]
pub(crate) fn routings_list() -> Result<Vec<library::RoutingSummary>, String> {
    lib()?.routings().map_err(es)
}
#[tauri::command]
pub(crate) fn routing_get(id: String) -> Result<library::Routing, String> {
    lib()?.routing(&id).map_err(es)
}
#[tauri::command]
pub(crate) fn routing_save(app: tauri::AppHandle, routing: library::Routing) -> Result<library::RoutingSummary, String> {
    let r = lib()?.save_routing(routing).map_err(es)?;
    library_changed(&app);
    Ok(r)
}
#[tauri::command]
pub(crate) fn routing_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    lib()?.delete_routing(&id).map_err(es)?;
    library_changed(&app);
    Ok(())
}
/// GPX of marks and routes (empty lists = everything) written to the export folder.
#[tauri::command]
pub(crate) fn gpx_export(name: String, mark_ids: Vec<String>, route_ids: Vec<String>) -> Result<String, String> {
    let l = lib()?;
    let text = l.export_gpx(&mark_ids, &route_ids).map_err(es)?;
    Ok(l.write_export(&name, &text).map_err(es)?.to_string_lossy().into_owned())
}
/// Any GPX text the page made (e.g. a routing with times) written to the export folder.
#[tauri::command]
pub(crate) fn gpx_write(name: String, text: String) -> Result<String, String> {
    Ok(lib()?.write_export(&name, &text).map_err(es)?.to_string_lossy().into_owned())
}
/// Import marks, routes and tracks from a file picked in the system file dialog: GPX, or
/// TimeZero's .tzd / .TzBackup (or the WaypointRouteDatabase.xml inside one). The file is read
/// here, not passed through the page: a TimeZero file can be gigabytes.
#[tauri::command]
pub(crate) async fn library_pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("Import marks, routes and tracks")
        .add_filter("Marks, routes and tracks", &["gpx", "tzd", "TzBackup", "xml", "kml", "kmz", "csv"])
        .add_filter("GPX", &["gpx"])
        .add_filter("TimeZero", &["tzd", "TzBackup", "xml"])
        .add_filter("Google Earth (KML, KMZ)", &["kml", "kmz"])
        .add_filter("Spreadsheet (CSV)", &["csv", "txt"])
        .add_filter("All files", &["*"])
        .blocking_pick_file();
    let Some(p) = picked else { return Ok(None) };
    Ok(Some(p.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned()))
}

/// Import marks, routes and tracks from a file (picked with library_pick_file): GPX, or
/// TimeZero's .tzd / .TzBackup (or the WaypointRouteDatabase.xml inside one). The file is read
/// here, not passed through the page: a TimeZero file can be gigabytes.
#[tauri::command]
pub(crate) async fn library_import_path(app: tauri::AppHandle, path: String, job: Option<String>) -> Result<library::ImportReport, String> {
    let path = std::path::PathBuf::from(path);
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let job = job.unwrap_or_default();
    let cancel = job_begin(&job);
    let h = app.clone();
    let j = job.clone();
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<library::ImportReport, String> {
        let l = lib()?;
        let mut progress = job_progress(&h, &j);
        if ext == "gpx" {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            l.import_gpx_with(&text, &stem, &mut progress).map_err(es)
        } else if ext == "kml" || ext == "kmz" {
            l.import_kml_with(&path, &stem, &mut progress).map_err(es)
        } else if ext == "csv" || ext == "txt" {
            l.import_csv_with(&path, &stem, &mut progress).map_err(es)
        } else {
            l.import_tzd_with(&path, &stem, &mut progress, &cancel).map_err(es)
        }
    })
    .await
    .map_err(|e| e.to_string());
    job_end(&job);
    let r = r??;
    library_changed(&app);
    Ok(r)
}
