//! The library: marks, routes, routings, barriers and tracks.
//!
//! Kept in one SQLite database, `<dir>/library.gpkg` (a GeoPackage: GIS software opens
//! it too), see [`store`]. GPX and other files written by the app go to `<dir>/export`.
//! Libraries kept as JSON files by earlier versions are moved in once
//! ([`Library::convert_json`]); the files are kept in `<dir>/json-backup-<date>`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod density;
mod events;
pub mod formats;
mod gpx;
mod manage;
mod trips;
pub use events::Event;
mod overlay;
pub mod protect;
pub use manage::{Also, Counts, ImportInfo, Scope, TrashItem};
pub use overlay::Overlay;
pub use trips::Trip;
pub mod store;
pub mod tracks;
pub mod tzd;
pub use gpx::{parse_gpx, GpxData, GpxRoute, GpxTrack, GpxWaypoint};
pub use tracks::{Recorder, Track, TrackPoint, TrackStats, TrackSummary};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("GPX: {0}")]
    Gpx(String),
    #[error("TimeZero: {0}")]
    Tz(String),
    #[error("library database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("library database: {0}")]
    Store(String),
    #[error("stopped")]
    Cancelled,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    InUse(String),
}
pub type Result<T> = std::result::Result<T, Error>;

/// A named position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mark {
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// "mark" (a racing mark or buoy), "waypoint", "start" or "finish" (line ends), "danger", or a
    /// buoy drawn with its IALA A symbol: "port", "starboard", "north", "east", "south", "west",
    /// "isolated", "safewater", "special".
    pub kind: String,
    pub notes: String,
    /// Groups marks, e.g. "Coastal Classic 2026" or "RNZYS".
    pub group: String,
    /// When above 0: the router keeps this far off (NM), unless the mark is on the course.
    pub barrier_nm: f64,
    /// Unix seconds.
    pub modified: i64,
    /// When the mark was made (Unix seconds; 0 unknown), e.g. from an imported file.
    #[serde(skip_serializing_if = "is_zero")]
    pub created: i64,
    /// `created` is only the day it was brought in from another plotter, in a batch
    /// (e.g. a fleet's SeaPlot marks moved into TimeZero): when it was made isn't known.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub date_unknown: bool,
    /// "#rrggbb" to draw it in (empty: by its kind).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub colour: String,
    /// The symbol it had where it came from, e.g. "tz:62" (TimeZero) or a GPX `<sym>`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub symbol: String,
    /// The conditions when it was made: depth (m), sea temperature (°C), SOG (kn), from the
    /// boat's track at that moment or from its name ("180 m").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sog: Option<f64>,
    /// Comma-separated tags: from words in its name ("foul", "rock", "hard", "caution" …),
    /// "event" for marks dropped with a plotter's event or quick-mark button.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tags: String,
    /// A kind's own details, as JSON: an anchor's swing radius, scope, bottom type and holding
    /// ({"radius_m": 45, "scope_m": 30, "bottom": "mud", "holding": 4, "dropped": 1794190000}).
    #[serde(skip_serializing_if = "is_empty_json")]
    pub data: serde_json::Value,
}

fn is_empty_json(v: &serde_json::Value) -> bool {
    v.is_null() || v.as_object().is_some_and(|o| o.is_empty())
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl Default for Mark {
    fn default() -> Self {
        Mark {
            id: String::new(),
            name: String::new(),
            lat: 0.0,
            lon: 0.0,
            kind: "mark".into(),
            notes: String::new(),
            group: String::new(),
            barrier_nm: 0.0,
            modified: 0,
            created: 0,
            date_unknown: false,
            colour: String::new(),
            symbol: String::new(),
            depth: None,
            temp: None,
            sog: None,
            tags: String::new(),
            data: serde_json::Value::Null,
        }
    }
}

/// A barrier the router avoids: a line it may not cross, or a zone it may not enter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Barrier {
    pub id: String,
    pub name: String,
    /// "line", "zone", or "clearing" (a zone of three points: a mark, a point along a clearing
    /// bearing from it, and the point 90° off on the side to keep off).
    pub kind: String,
    /// [lon, lat] points (a zone is closed implicitly).
    pub points: Vec<[f64; 2]>,
    /// Switched off barriers are kept but not used.
    pub active: bool,
    pub notes: String,
    pub modified: i64,
}

impl Default for Barrier {
    fn default() -> Self {
        Barrier { id: String::new(), name: String::new(), kind: "line".into(), points: Vec::new(), active: true, notes: String::new(), modified: 0 }
    }
}

/// One mark in a route.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct RouteMark {
    pub mark: String,
    /// "port", "stbd", "close" (as close as possible) or empty (enter the arrival circle).
    pub rounding: String,
    /// Arrival radius, or clearance when rounding, NM (0 = the routing's arrival radius).
    pub radius_nm: f64,
}

/// An ordered list of marks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Route {
    pub id: String,
    pub name: String,
    pub notes: String,
    /// Where the race or passage starts, [lon, lat], if the route keeps one.
    pub start: Option<[f64; 2]>,
    /// The library mark the start is at, when it was set from one.
    pub start_mark: Option<String>,
    pub marks: Vec<RouteMark>,
    pub modified: i64,
}

/// A saved routing: the inputs and the router's result, kept as JSON values
/// so the format can grow without breaking old files.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Routing {
    pub id: String,
    pub name: String,
    pub created: i64,
    pub route_id: String,
    pub route_name: String,
    /// start, marks, t0, grib_path, grib_name, polar_path, safety_depth, params …
    pub request: Value,
    pub result: Value,
}

/// What lists show without reading every result.
#[derive(Debug, Clone, Serialize)]
pub struct RoutingSummary {
    pub id: String,
    pub name: String,
    pub created: i64,
    pub route_name: String,
    pub t0: Option<i64>,
    pub status: Option<String>,
    pub arrival: Option<i64>,
    pub distance_nm: Option<f64>,
    pub grib_name: Option<String>,
    pub polar: Option<String>,
}

/// A plotter symbol used by marks (TimeZero's "tz:63"), how many, their colours, and what
/// it's been given to mean here.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolUse {
    pub symbol: String,
    pub count: usize,
    /// The mark type given to it ("" none yet).
    pub kind: String,
    pub label: String,
    /// The colours its marks have, commonest first, with how many.
    pub colours: Vec<(String, usize)>,
}

/// What an import did.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ImportReport {
    pub marks_added: usize,
    pub marks_matched: usize,
    pub routes_added: Vec<String>,
    /// Routes in the file that were already here.
    pub routes_matched: usize,
    pub tracks_added: Vec<String>,
    #[serde(default)]
    pub routings_added: usize,
    #[serde(default)]
    pub barriers_added: usize,
    #[serde(default)]
    pub events_added: usize,
    #[serde(default)]
    pub trips_added: usize,
    /// What the file held that wasn't brought in (TimeZero lines, areas, circles, tracks).
    #[serde(default)]
    pub not_imported: Vec<String>,
    /// Anything else worth saying about the import.
    #[serde(default)]
    pub notes: Vec<String>,
}

pub struct Library {
    dir: PathBuf,
    /// The database's file name in `dir` (library.gpkg, or another library opened by name).
    file: String,
    db: Mutex<Connection>,
    /// The one-off move from the JSON files is running (lists may be incomplete).
    converting: AtomicBool,
}

/// Two marks this close (NM) with the same name are the same mark.
const SAME_MARK_NM: f64 = 0.03;
const SAME_UNNAMED_NM: f64 = 0.002;
/// This many marks made in the same second were brought in from another plotter in one go:
/// their date is the day of that import, not when they were made.
const BULK_SAME_SECOND: usize = 20;

const MARK_COLS: &str = "id, name, lat, lon, kind, notes, grp, barrier_nm, modified, created, date_unknown, colour, symbol, depth, temp, sog, tags, data";
const TRACK_COLS: &str =
    "id, name, notes, colour, source, modified, points, start, \"end\", distance_nm, max_sog, west, south, east, north, has_depth, has_temp";

fn mark_row(r: &rusqlite::Row) -> rusqlite::Result<Mark> {
    Ok(Mark {
        id: r.get(0)?,
        name: r.get(1)?,
        lat: r.get(2)?,
        lon: r.get(3)?,
        kind: r.get(4)?,
        notes: r.get(5)?,
        group: r.get(6)?,
        barrier_nm: r.get(7)?,
        modified: r.get(8)?,
        created: r.get(9)?,
        date_unknown: r.get::<_, i64>(10)? != 0,
        colour: r.get(11)?,
        symbol: r.get(12)?,
        depth: r.get(13)?,
        temp: r.get(14)?,
        sog: r.get(15)?,
        tags: r.get(16)?,
        data: serde_json::from_str(&r.get::<_, String>(17)?).unwrap_or(serde_json::Value::Null),
    })
}

fn track_summary_row(r: &rusqlite::Row) -> rusqlite::Result<TrackSummary> {
    let bbox = match (r.get::<_, Option<f64>>(11)?, r.get::<_, Option<f64>>(12)?, r.get::<_, Option<f64>>(13)?, r.get::<_, Option<f64>>(14)?) {
        (Some(w), Some(s), Some(e), Some(n)) => Some([w, s, e, n]),
        _ => None,
    };
    Ok(TrackSummary {
        id: r.get(0)?,
        name: r.get(1)?,
        notes: r.get(2)?,
        colour: r.get(3)?,
        source: r.get(4)?,
        modified: r.get(5)?,
        stats: TrackStats { points: r.get::<_, i64>(6)? as usize, start: r.get(7)?, end: r.get(8)?, distance_nm: r.get(9)?, max_sog: r.get(10)?, bbox },
        has_depth: r.get::<_, i64>(15)? != 0,
        has_temp: r.get::<_, i64>(16)? != 0,
        // filled in from their own columns (a library file from before them has none)
        depth_ref: String::new(),
        depth_offset: None,
    })
}

/// What each track's depths are measured from (tracks with depth), by id. Empty for a library
/// file made before the columns (an overlay, read-only).
fn depth_refs(c: &Connection) -> std::collections::HashMap<String, (String, Option<f64>)> {
    let Ok(mut q) = c.prepare_cached("SELECT id, depth_ref, depth_offset FROM tracks WHERE has_depth = 1") else { return std::collections::HashMap::new() };
    q.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, Option<f64>>(2)?))))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

fn insert_mark(c: &Connection, m: &Mark, import_id: Option<i64>) -> Result<()> {
    c.prepare_cached(
        "INSERT INTO marks (geom, id, name, lat, lon, kind, notes, grp, barrier_nm, modified, created, date_unknown, colour, symbol, import_id, depth, temp, sog, tags, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(id) DO UPDATE SET geom = excluded.geom, name = excluded.name, lat = excluded.lat, lon = excluded.lon,
           kind = excluded.kind, notes = excluded.notes, grp = excluded.grp, barrier_nm = excluded.barrier_nm,
           modified = excluded.modified, created = excluded.created, date_unknown = excluded.date_unknown,
           colour = excluded.colour, symbol = excluded.symbol, depth = excluded.depth, temp = excluded.temp,
           sog = excluded.sog, tags = excluded.tags, data = excluded.data, deleted_at = NULL",
    )?
    .execute(params![
        store::gpkg_point(m.lon, m.lat),
        m.id,
        m.name,
        m.lat,
        m.lon,
        m.kind,
        m.notes,
        m.group,
        m.barrier_nm,
        m.modified,
        m.created,
        m.date_unknown as i64,
        m.colour,
        m.symbol,
        import_id,
        m.depth,
        m.temp,
        m.sog,
        m.tags,
        if is_empty_json(&m.data) { "{}".to_string() } else { m.data.to_string() }
    ])?;
    Ok(())
}

fn write_route(c: &Connection, r: &Route, import_id: Option<i64>) -> Result<()> {
    c.execute(
        "INSERT INTO routes (id, name, notes, start_lon, start_lat, start_mark, modified, import_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, notes = excluded.notes, start_lon = excluded.start_lon,
           start_lat = excluded.start_lat, start_mark = excluded.start_mark, modified = excluded.modified, deleted_at = NULL",
        params![r.id, r.name, r.notes, r.start.map(|s| s[0]), r.start.map(|s| s[1]), r.start_mark, r.modified, import_id],
    )?;
    c.execute("DELETE FROM route_marks WHERE route_id = ?1", [&r.id])?;
    let mut ins = c.prepare_cached("INSERT INTO route_marks (route_id, seq, mark_id, rounding, radius_nm) VALUES (?1, ?2, ?3, ?4, ?5)")?;
    for (k, m) in r.marks.iter().enumerate() {
        ins.execute(params![r.id, k as i64, m.mark, m.rounding, m.radius_nm])?;
    }
    Ok(())
}

fn write_track(c: &Connection, t: &Track, import_id: Option<i64>) -> Result<()> {
    let s = &t.stats;
    let (w, so, e, n) = match s.bbox {
        Some(b) => (Some(b[0]), Some(b[1]), Some(b[2]), Some(b[3])),
        None => (None, None, None, None),
    };
    let has = |f: fn(&TrackPoint) -> bool| t.segments.iter().flatten().any(f) as i64;
    c.prepare_cached(
        "INSERT INTO tracks (id, name, notes, colour, source, modified, points, start, \"end\", distance_nm, max_sog, west, south, east, north, has_depth, has_temp, import_id, depth_ref, depth_offset)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, notes = excluded.notes, colour = excluded.colour, source = excluded.source,
           modified = excluded.modified, points = excluded.points, start = excluded.start, \"end\" = excluded.\"end\",
           distance_nm = excluded.distance_nm, max_sog = excluded.max_sog, west = excluded.west, south = excluded.south,
           east = excluded.east, north = excluded.north, has_depth = excluded.has_depth, has_temp = excluded.has_temp,
           depth_ref = excluded.depth_ref, depth_offset = excluded.depth_offset, deleted_at = NULL",
    )?
    .execute(params![
        t.id,
        t.name,
        t.notes,
        t.colour,
        t.source,
        t.modified,
        s.points as i64,
        s.start,
        s.end,
        s.distance_nm,
        s.max_sog,
        w,
        so,
        e,
        n,
        has(|p| p.depth.is_some()),
        has(|p| p.temp.is_some()),
        import_id,
        t.depth_ref,
        t.depth_offset
    ])?;
    c.prepare_cached("INSERT INTO track_data (track_id, data) VALUES (?1, ?2) ON CONFLICT(track_id) DO UPDATE SET data = excluded.data")?
        .execute(params![t.id, store::encode_segments(&t.segments)])?;
    Ok(())
}

impl Library {
    /// Open the library in `dir` (made if missing): `<dir>/library.gpkg`, and `<dir>/export` for files written.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        Self::open_file(dir.into().join(store::FILE))
    }

    /// Open (or make) the library in the database file `path`: another boat's library, one
    /// for another fishery, a copy. Its folder holds its `export` and `backups` too.
    pub fn open_file(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| store::FILE.into());
        std::fs::create_dir_all(dir.join("export"))?;
        let c = store::open(&path)?;
        // the Trash keeps deleted things for a while
        let old = now() - store::TRASH_DAYS * 86_400;
        for t in ["marks", "routes", "routings", "barriers", "tracks", "trips", "events"] {
            c.execute(&format!("DELETE FROM {t} WHERE deleted_at IS NOT NULL AND deleted_at < ?1"), [old])?;
        }
        if store::meta(&c, "marks_reviewed")?.is_none() {
            review_marks(&c)?;
        }
        Ok(Library { dir, file, db: Mutex::new(c), converting: AtomicBool::new(false) })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The database file.
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.file)
    }

    /// The library's name: its file's name without .gpkg.
    pub fn name(&self) -> String {
        Path::new(&self.file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the one-off move from the JSON files is running.
    pub fn converting(&self) -> bool {
        self.converting.load(Ordering::Relaxed)
    }

    // ---------- marks ----------

    pub fn marks(&self) -> Result<Vec<Mark>> {
        let c = self.db();
        let mut q = c.prepare_cached(&format!("SELECT {MARK_COLS} FROM marks WHERE deleted_at IS NULL ORDER BY fid"))?;
        let v = q.query_map([], mark_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// The marks inside [west, south, east, north] (degrees; east < west crosses 180°), from
    /// the spatial index: what's in the chart view, however many marks the library has.
    pub fn marks_in(&self, west: f64, south: f64, east: f64, north: f64) -> Result<Vec<Mark>> {
        let c = self.db();
        let mut q = c.prepare_cached(&format!(
            "SELECT {MARK_COLS} FROM marks WHERE deleted_at IS NULL AND fid IN
             (SELECT fid FROM marks_rtree WHERE minx >= ?1 AND maxx <= ?3 AND miny >= ?2 AND maxy <= ?4) ORDER BY fid"
        ))?;
        let spans: Vec<(f64, f64)> = if east >= west { vec![(west, east)] } else { vec![(west, 180.0), (-180.0, east)] };
        let mut v = Vec::new();
        for (w, e) in spans {
            v.extend(q.query_map(params![w, south, e, north], mark_row)?.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        Ok(v)
    }

    /// Add or update a mark (matched by id; an empty id makes a new one).
    pub fn save_mark(&self, mut m: Mark) -> Result<Mark> {
        let c = self.db();
        m.modified = now();
        if m.name.trim().is_empty() {
            m.name = next_name(&c, "WP")?;
        }
        if m.id.is_empty() {
            m.id = new_id("m");
        }
        insert_mark(&c, &m, None)?;
        Ok(m)
    }

    /// Delete a mark (to the Trash). If routes use it, refuse unless `force`, which also
    /// takes it out of those routes.
    pub fn delete_mark(&self, id: &str, force: bool) -> Result<()> {
        if self.delete_marks(&[id.to_string()], force)? == 0 {
            return Err(Error::NotFound(id.into()));
        }
        Ok(())
    }

    /// Delete many marks at once (to the Trash). With `force`, marks used by routes are
    /// taken out of them; without it, nothing is deleted if any is in use.
    pub fn delete_marks(&self, ids: &[String], force: bool) -> Result<usize> {
        let mut c = self.db();
        let tx = c.transaction()?;
        tx.execute("CREATE TEMP TABLE IF NOT EXISTS gone (id TEXT PRIMARY KEY)", [])?;
        tx.execute("DELETE FROM gone", [])?;
        {
            let mut ins = tx.prepare_cached("INSERT OR IGNORE INTO gone (id) VALUES (?1)")?;
            for id in ids {
                ins.execute([id])?;
            }
        }
        let users: Vec<String> = {
            let mut q = tx.prepare(
                "SELECT DISTINCT r.name FROM routes r WHERE r.deleted_at IS NULL AND (r.start_mark IN (SELECT id FROM gone)
                 OR r.id IN (SELECT route_id FROM route_marks WHERE mark_id IN (SELECT id FROM gone)))",
            )?;
            let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            v
        };
        if !users.is_empty() && !force {
            let what = if ids.len() == 1 { "The mark is" } else { "Some of the marks are" };
            return Err(Error::InUse(format!("{what} used by: {}", users.join(", "))));
        }
        let t = now();
        // out of the routes they were in (a route's start keeps its position)
        tx.execute("UPDATE routes SET start_mark = NULL, modified = ?1 WHERE start_mark IN (SELECT id FROM gone)", [t])?;
        let touched: Vec<String> = {
            let mut q = tx.prepare("SELECT DISTINCT route_id FROM route_marks WHERE mark_id IN (SELECT id FROM gone)")?;
            let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            v
        };
        tx.execute("DELETE FROM route_marks WHERE mark_id IN (SELECT id FROM gone)", [])?;
        for r in &touched {
            // close the gaps in the order
            tx.execute(
                "UPDATE route_marks SET seq = (SELECT count(*) FROM route_marks x WHERE x.route_id = route_marks.route_id AND x.seq < route_marks.seq) + 100000 WHERE route_id = ?1",
                [r],
            )?;
            tx.execute("UPDATE route_marks SET seq = seq - 100000 WHERE route_id = ?1", [r])?;
            tx.execute("UPDATE routes SET modified = ?2 WHERE id = ?1", params![r, t])?;
        }
        let n = tx.execute("UPDATE marks SET deleted_at = ?1 WHERE deleted_at IS NULL AND id IN (SELECT id FROM gone)", [t])?;
        tx.execute("DELETE FROM gone", [])?;
        tx.commit()?;
        Ok(n)
    }

    /// Change many marks at once: any of group, type and colour (None leaves it as it is).
    /// Returns how many changed.
    pub fn update_marks(&self, ids: &[String], group: Option<&str>, kind: Option<&str>, colour: Option<&str>) -> Result<usize> {
        let mut c = self.db();
        let tx = c.transaction()?;
        let mut n = 0;
        {
            let mut q = tx.prepare(
                "UPDATE marks SET grp = coalesce(?2, grp), kind = coalesce(?3, kind), colour = coalesce(?4, colour), modified = ?5
                 WHERE id = ?1 AND deleted_at IS NULL",
            )?;
            let t = now();
            for id in ids {
                n += q.execute(params![id, group.map(str::trim), kind, colour, t])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Several tracks in one GPX file.
    pub fn export_tracks_gpx(&self, ids: &[String]) -> Result<String> {
        let mut out =
            String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"racenav\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n");
        for id in ids {
            out.push_str(&self.track(id)?.trk_xml());
        }
        out.push_str("</gpx>\n");
        Ok(out)
    }

    /// Bring everything from another library into this one: marks (matched by id, or by
    /// name and place as imports are), routes (with their marks; matched by name and marks),
    /// tracks (matched by their first and last fix), routings and barriers (by id). The
    /// other library isn't changed. `progress(step, done, total)`; `cancel` stops part way.
    pub fn merge_from(&self, other: &Path, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<ImportReport> {
        self.merge_from_scope(other, Scope::default(), progress, cancel)
    }

    /// [`Library::merge_from`] bringing in only the categories in `scope`, and the logs and
    /// trips (by id, and by name and start). Routes need their marks: with marks left out
    /// the marks the chosen routes use still come in.
    pub fn merge_from_scope(&self, other: &Path, scope: Scope, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<ImportReport> {
        if other.canonicalize().ok() == self.path().canonicalize().ok() {
            return Err(Error::Store("that's the library already open".into()));
        }
        progress("Reading the other library", 0, 0);
        let o = Library::open_file(other)?;
        let id = self.begin_import(&other.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "library")?;
        let mut rep = ImportReport::default();
        // marks: the same id is the same mark; else matched as an import matches
        let mut theirs = o.marks()?;
        let their_routes = if scope.routes { o.routes()? } else { Vec::new() };
        if !scope.marks {
            let used: std::collections::HashSet<&str> =
                their_routes.iter().flat_map(|r| r.marks.iter().map(|m| m.mark.as_str()).chain(r.start_mark.as_deref())).collect();
            theirs.retain(|m| used.contains(m.id.as_str()));
        }
        let mut ours = self.marks()?;
        let by_id: std::collections::HashSet<String> = ours.iter().map(|m| m.id.clone()).collect();
        const CELL: f64 = 0.01;
        let cell = |lat: f64, lon: f64| ((lat / CELL).floor() as i64, (lon / CELL).floor() as i64);
        let mut index: std::collections::HashMap<(String, (i64, i64)), Vec<usize>> = std::collections::HashMap::new();
        for (i, m) in ours.iter().enumerate() {
            index.entry((m.name.to_lowercase(), cell(m.lat, m.lon))).or_default().push(i);
        }
        let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let first_new = ours.len();
        progress("Marks", 0, theirs.len());
        for (k, m) in theirs.iter().enumerate() {
            if k % 2000 == 0 {
                progress("Marks", k, theirs.len());
            }
            if by_id.contains(&m.id) {
                rep.marks_matched += 1;
                map.insert(m.id.clone(), m.id.clone());
                continue;
            }
            let key = m.name.trim().to_lowercase();
            let near = if key.is_empty() { SAME_UNNAMED_NM } else { SAME_MARK_NM };
            let here = nav_core::LatLon::new(m.lat, m.lon);
            let (cy, cx) = cell(m.lat, m.lon);
            let mut found = None;
            'look: for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(v) = index.get(&(key.clone(), (cy + dy, cx + dx))) {
                        if let Some(&i) = v.iter().find(|&&i| nav_core::LatLon::new(ours[i].lat, ours[i].lon).distance_to(&here) < near) {
                            found = Some(ours[i].id.clone());
                            break 'look;
                        }
                    }
                }
            }
            match found {
                Some(x) => {
                    rep.marks_matched += 1;
                    map.insert(m.id.clone(), x);
                }
                None => {
                    rep.marks_added += 1;
                    map.insert(m.id.clone(), m.id.clone());
                    index.entry((key, (cy, cx))).or_default().push(ours.len());
                    ours.push(m.clone());
                }
            }
        }
        // routes, their marks given as ours
        let route_key = |name: &str, marks: &[RouteMark]| (name.to_lowercase(), marks.iter().map(|m| m.mark.clone()).collect::<Vec<_>>());
        let mut known: std::collections::HashSet<(String, Vec<String>)> = self.routes()?.iter().map(|r| route_key(&r.name, &r.marks)).collect();
        let route_ids: std::collections::HashSet<String> = self.routes()?.into_iter().map(|r| r.id).collect();
        let mut new_routes = Vec::new();
        for mut r in their_routes {
            for x in &mut r.marks {
                if let Some(to) = map.get(&x.mark) {
                    x.mark = to.clone();
                }
            }
            r.start_mark = r.start_mark.and_then(|s| map.get(&s).cloned());
            if route_ids.contains(&r.id) || !known.insert(route_key(&r.name, &r.marks)) {
                rep.routes_matched += 1;
                continue;
            }
            rep.routes_added.push(r.name.clone());
            new_routes.push(r);
        }
        {
            let mut c = self.db();
            let tx = c.transaction()?;
            for m in &ours[first_new..] {
                insert_mark(&tx, m, Some(id))?;
            }
            for r in &new_routes {
                write_route(&tx, r, Some(id))?;
            }
            tx.commit()?;
        }
        // routings and barriers: by id
        // ids in the Trash count as here: a routing or barrier deleted here stays deleted
        let ids_here = |table: &str| -> Result<std::collections::HashSet<String>> {
            let c = self.db();
            let mut q = c.prepare(&format!("SELECT id FROM {table}"))?;
            let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            Ok(v)
        };
        if scope.routings {
            let have_x = ids_here("routings")?;
            for s in o.routings()? {
                if !have_x.contains(&s.id) {
                    write_routing(&self.db(), &o.routing(&s.id)?)?;
                    rep.routings_added += 1;
                }
            }
            if rep.routings_added > 0 {
                rep.notes.push("Routings can't be taken out again by Undo an import: delete them from the Routings list".into());
            }
        }
        if scope.barriers {
            let have_b = ids_here("barriers")?;
            for b in o.barriers()? {
                if !have_b.contains(&b.id) {
                    // with the import's id, so Undo an import takes them out again
                    self.db().execute(
                        "INSERT INTO barriers (id, name, kind, points, active, notes, modified, import_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![b.id, b.name, b.kind, serde_json::to_string(&b.points)?, b.active as i64, b.notes, b.modified, id],
                    )?;
                    rep.barriers_added += 1;
                }
            }
        }
        // logs: by id (a deleted one stays deleted)
        if scope.events {
            let have: std::collections::HashSet<String> = {
                let c = self.db();
                let mut q = c.prepare("SELECT id FROM events")?;
                let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                v
            };
            let c = self.db();
            for e in o.events()? {
                if have.contains(&e.id) {
                    continue;
                }
                c.execute(
                    "INSERT INTO events (id, kind, name, at, lat, lon, depth, temp, sog, cog, notes, pair, data, import_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![e.id, e.kind, e.name, e.at, e.lat, e.lon, e.depth, e.temp, e.sog, e.cog, e.notes, e.pair, if e.data.is_null() { "{}".to_string() } else { e.data.to_string() }, id],
                )?;
                rep.events_added += 1;
            }
        }
        // trips: the same name and start is the same trip. One that was still on comes in ended
        // (only one trip can be on, and it's this library's).
        if scope.trips {
            let key = |name: &str, started: Option<i64>| (name.trim().to_lowercase(), started);
            let mut have: std::collections::HashSet<(String, Option<i64>)> = self.trips()?.iter().map(|t| key(&t.name, t.started)).collect();
            let c = self.db();
            for t in o.trips()? {
                if !have.insert(key(&t.name, t.started)) {
                    continue;
                }
                c.execute(
                    "INSERT INTO trips (name, number, fishery, crew, notes, started, ended) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    // ended: never left on (only this library's trip can be); with no times at all, ended at the merge
                    params![t.name, t.number, t.fishery, t.crew, t.notes, t.started, t.ended.or(t.started).or(Some(now()))],
                )?;
                rep.trips_added += 1;
            }
            if rep.trips_added > 0 {
                rep.notes.push("Trips can't be taken out again by Undo an import: delete them from the Trips list".into());
            }
        }
        // tracks: by first and last fix, a few hundred at a time
        let have: std::collections::HashSet<(i64, i64)> = self.tracks()?.into_iter().filter_map(|t| t.stats.start.zip(t.stats.end)).collect();
        let theirs = if scope.tracks { o.tracks()? } else { Vec::new() };
        let total = theirs.len();
        let mut batch = Vec::new();
        let mut same = 0;
        for (k, t) in theirs.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                rep.not_imported.push(format!("tracks from {} of {total} on (stopped part way: merge again to bring in the rest)", k + 1));
                break;
            }
            progress("Tracks", k + 1, total);
            if t.stats.start.zip(t.stats.end).is_some_and(|x| have.contains(&x)) {
                same += 1;
                continue;
            }
            batch.push(o.track(&t.id)?);
            if batch.len() >= 200 || k + 1 == total {
                let mut c = self.db();
                let tx = c.transaction()?;
                for t in batch.drain(..) {
                    write_track(&tx, &t, Some(id))?;
                    rep.tracks_added.push(t.name);
                }
                tx.commit()?;
            }
        }
        if !batch.is_empty() {
            let mut c = self.db();
            let tx = c.transaction()?;
            for t in batch.drain(..) {
                write_track(&tx, &t, Some(id))?;
                rep.tracks_added.push(t.name);
            }
            tx.commit()?;
        }
        if same > 0 {
            rep.notes.push(format!("{same} tracks were already here"));
        }
        self.finish_import(id, &rep)?;
        Ok(rep)
    }

    // ---------- plotter symbols ----------

    /// The TimeZero symbols the marks use, commonest first, with what each has been given to mean.
    pub fn symbols(&self) -> Result<Vec<SymbolUse>> {
        let c = self.db();
        let mut q = c.prepare(
            "SELECT m.symbol, count(*), coalesce(s.kind, ''), coalesce(s.label, '') FROM marks m LEFT JOIN tz_symbols s ON s.symbol = m.symbol
             WHERE m.deleted_at IS NULL AND m.symbol LIKE 'tz:%' GROUP BY m.symbol ORDER BY count(*) DESC",
        )?;
        let mut v = q
            .query_map([], |r| {
                Ok(SymbolUse { symbol: r.get(0)?, count: r.get::<_, i64>(1)? as usize, kind: r.get(2)?, label: r.get(3)?, colours: Vec::new() })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut q = c.prepare(
            "SELECT symbol, colour, count(*) FROM marks WHERE deleted_at IS NULL AND symbol LIKE 'tz:%' GROUP BY symbol, colour ORDER BY count(*) DESC",
        )?;
        let mut rows = q.query([])?;
        let at: std::collections::HashMap<String, usize> = v.iter().enumerate().map(|(i, s)| (s.symbol.clone(), i)).collect();
        while let Some(r) = rows.next()? {
            let sym: String = r.get(0)?;
            if let Some(&i) = at.get(&sym) {
                v[i].colours.push((r.get(1)?, r.get::<_, i64>(2)? as usize));
            }
        }
        Ok(v)
    }

    /// Give a plotter symbol a meaning: `kind` (a mark type: "fishing", "foul", "waypoint" …)
    /// and a label. Marks with it still as they were imported (or given the symbol's earlier
    /// type) take the new type; later imports do too. Returns how many marks changed.
    pub fn set_symbol(&self, symbol: &str, kind: &str, label: &str) -> Result<usize> {
        let c = self.db();
        let before: String = c.query_row("SELECT kind FROM tz_symbols WHERE symbol = ?1", [symbol], |r| r.get(0)).optional()?.unwrap_or_default();
        c.execute(
            "INSERT INTO tz_symbols (symbol, kind, label) VALUES (?1, ?2, ?3) ON CONFLICT(symbol) DO UPDATE SET kind = excluded.kind, label = excluded.label",
            params![symbol, kind, label.trim()],
        )?;
        let to = if kind.is_empty() { "waypoint" } else { kind };
        let n = c.execute(
            "UPDATE marks SET kind = ?2, modified = ?4 WHERE symbol = ?1 AND deleted_at IS NULL AND (kind = 'waypoint' OR (?3 <> '' AND kind = ?3))",
            params![symbol, to, before, now()],
        )?;
        Ok(n)
    }

    /// Every symbol meaning this library holds: (symbol, kind, label).
    pub fn symbol_table(&self) -> Result<Vec<(String, String, String)>> {
        let c = self.db();
        let mut q = c.prepare("SELECT symbol, kind, label FROM tz_symbols ORDER BY symbol")?;
        let v = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Bring this library's symbol meanings into line with `table` (the app's one table):
    /// each symbol that differs is set as `set_symbol` would. Returns how many marks changed.
    pub fn apply_symbol_table(&self, table: &[(String, String, String)]) -> Result<usize> {
        let have: std::collections::HashMap<String, (String, String)> = self.symbol_table()?.into_iter().map(|(s, k, l)| (s, (k, l))).collect();
        let mut n = 0;
        for (sym, kind, label) in table {
            if have.get(sym).is_some_and(|(k, l)| k == kind && l == label) {
                continue;
            }
            n += self.set_symbol(sym, kind, label)?;
        }
        Ok(n)
    }

    // ---------- barriers ----------

    pub fn barriers(&self) -> Result<Vec<Barrier>> {
        let c = self.db();
        let mut q = c.prepare_cached("SELECT id, name, kind, points, active, notes, modified FROM barriers WHERE deleted_at IS NULL ORDER BY rowid")?;
        let v = q
            .query_map([], |r| {
                Ok(Barrier {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    points: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                    active: r.get::<_, i64>(4)? != 0,
                    notes: r.get(5)?,
                    modified: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Add or update a barrier (an empty id makes a new one, named "Barrier n" when unnamed).
    pub fn save_barrier(&self, mut b: Barrier) -> Result<Barrier> {
        let names: Vec<String> = self.barriers()?.into_iter().map(|x| x.name).collect();
        b.modified = now();
        if b.name.trim().is_empty() {
            b.name = (1..).map(|k| format!("Barrier {k}")).find(|n| !names.contains(n)).unwrap();
        }
        if b.id.is_empty() {
            b.id = new_id("b");
        }
        self.db().execute(
            "INSERT INTO barriers (id, name, kind, points, active, notes, modified) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, kind = excluded.kind, points = excluded.points,
               active = excluded.active, notes = excluded.notes, modified = excluded.modified, deleted_at = NULL",
            params![b.id, b.name, b.kind, serde_json::to_string(&b.points)?, b.active as i64, b.notes, b.modified],
        )?;
        Ok(b)
    }

    pub fn delete_barrier(&self, id: &str) -> Result<()> {
        self.soft_delete("barriers", id)
    }

    fn soft_delete(&self, table: &str, id: &str) -> Result<()> {
        let n = self.db().execute(&format!("UPDATE {table} SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL"), params![id, now()])?;
        if n == 0 {
            return Err(Error::NotFound(id.into()));
        }
        Ok(())
    }

    // ---------- routes ----------

    pub fn routes(&self) -> Result<Vec<Route>> {
        let c = self.db();
        routes_where(&c, "1")
    }

    pub fn route(&self, id: &str) -> Result<Route> {
        let c = self.db();
        let mut q = c.prepare("SELECT 1 FROM routes WHERE id = ?1 AND deleted_at IS NULL")?;
        if !q.exists([id])? {
            return Err(Error::NotFound(id.into()));
        }
        routes_where(&c, &format!("r.id = '{}'", id.replace('\'', "''")))?.into_iter().next().ok_or_else(|| Error::NotFound(id.into()))
    }

    pub fn save_route(&self, mut r: Route) -> Result<Route> {
        if r.id.is_empty() {
            r.id = new_id("r");
        }
        if r.name.trim().is_empty() {
            r.name = "Route".into();
        }
        r.modified = now();
        let mut c = self.db();
        let tx = c.transaction()?;
        write_route(&tx, &r, None)?;
        tx.commit()?;
        Ok(r)
    }

    pub fn delete_route(&self, id: &str) -> Result<()> {
        self.soft_delete("routes", id)
    }

    // ---------- routings ----------

    pub fn routings(&self) -> Result<Vec<RoutingSummary>> {
        let c = self.db();
        let mut q = c.prepare_cached(
            "SELECT id, name, created, route_name, t0, status, arrival, distance_nm, grib_name, polar FROM routings
             WHERE deleted_at IS NULL ORDER BY created DESC",
        )?;
        let v = q
            .query_map([], |r| {
                Ok(RoutingSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created: r.get(2)?,
                    route_name: r.get(3)?,
                    t0: r.get(4)?,
                    status: r.get(5)?,
                    arrival: r.get(6)?,
                    distance_nm: r.get(7)?,
                    grib_name: r.get(8)?,
                    polar: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn routing(&self, id: &str) -> Result<Routing> {
        let c = self.db();
        c.query_row("SELECT id, name, created, route_id, route_name, request, result FROM routings WHERE id = ?1 AND deleted_at IS NULL", [id], |r| {
            Ok(Routing {
                id: r.get(0)?,
                name: r.get(1)?,
                created: r.get(2)?,
                route_id: r.get(3)?,
                route_name: r.get(4)?,
                request: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
                result: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(Value::Null),
            })
        })
        .optional()?
        .ok_or_else(|| Error::NotFound(id.into()))
    }

    pub fn save_routing(&self, mut r: Routing) -> Result<RoutingSummary> {
        if r.id.is_empty() {
            r.id = new_id("x");
        }
        if r.created == 0 {
            r.created = now();
        }
        if r.name.trim().is_empty() {
            r.name = if r.route_name.is_empty() { "Routing".into() } else { r.route_name.clone() };
        }
        write_routing(&self.db(), &r)?;
        let id = r.id.clone();
        self.routings()?.into_iter().find(|s| s.id == id).ok_or(Error::NotFound(id))
    }

    pub fn delete_routing(&self, id: &str) -> Result<()> {
        self.soft_delete("routings", id)
    }

    // ---------- GPX ----------

    /// GPX of the given marks (as waypoints) and routes; empty slices mean "all".
    pub fn export_gpx(&self, mark_ids: &[String], route_ids: &[String]) -> Result<String> {
        let marks = self.marks()?;
        let routes: Vec<Route> = self.routes()?.into_iter().filter(|r| route_ids.is_empty() || route_ids.contains(&r.id)).collect();
        let by_id: std::collections::HashMap<&str, &Mark> = marks.iter().map(|m| (m.id.as_str(), m)).collect();
        let wpts: Vec<&Mark> = if mark_ids.is_empty() && route_ids.is_empty() {
            marks.iter().collect()
        } else {
            // the chosen marks plus every mark the chosen routes use
            let mut want: std::collections::HashSet<&str> = mark_ids.iter().map(String::as_str).collect();
            for r in &routes {
                want.extend(r.marks.iter().map(|x| x.mark.as_str()));
                want.extend(r.start_mark.as_deref());
            }
            marks.iter().filter(|m| want.contains(m.id.as_str())).collect()
        };
        let mut out =
            String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"racenav\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n");
        for m in &wpts {
            out.push_str(&gpx::wpt("wpt", m, "  "));
        }
        for r in &routes {
            out.push_str(&format!("  <rte>\n    <name>{}</name>\n", gpx::esc(&r.name)));
            // the start leads the route: its library mark, else a plain point
            if let Some(m) = r.start_mark.as_deref().and_then(|id| by_id.get(id)) {
                out.push_str(&gpx::wpt("rtept", m, "    "));
            } else if let Some([lon, lat]) = r.start {
                let m = Mark { name: "Start".into(), lat, lon, kind: "start".into(), ..Default::default() };
                out.push_str(&gpx::wpt("rtept", &m, "    "));
            }
            for rm in &r.marks {
                if let Some(m) = by_id.get(rm.mark.as_str()) {
                    out.push_str(&gpx::wpt("rtept", m, "    "));
                }
            }
            out.push_str("  </rte>\n");
        }
        out.push_str("</gpx>\n");
        Ok(out)
    }

    /// Bring GPX waypoints and routes into the library. A waypoint with the
    /// same name as a library mark and within about 50 m is the same mark.
    pub fn import_gpx(&self, text: &str, group: &str) -> Result<ImportReport> {
        self.import_gpx_with(text, group, &mut |_, _, _| {})
    }

    /// [`import_gpx`](Self::import_gpx), telling `progress(step, done, total)` as it goes.
    pub fn import_gpx_with(&self, text: &str, group: &str, progress: &mut dyn FnMut(&str, usize, usize)) -> Result<ImportReport> {
        progress("Reading the file", 0, 0);
        let data = parse_gpx(text)?;
        progress("Matching marks with the library", 0, 0);
        let id = self.begin_import(group, "gpx")?;
        let rep = self.import_data(data, group, id)?;
        self.finish_import(id, &rep)?;
        Ok(rep)
    }

    /// Bring in a TimeZero `.tzd` / `.TzBackup` (or its WaypointRouteDatabase.xml): marks
    /// in their layers as groups, with symbol, colour and date, and routes. Matching is as GPX.
    pub fn import_tzd(&self, path: &std::path::Path, group: &str) -> Result<ImportReport> {
        self.import_tzd_with(path, group, &mut |_, _, _| {}, &AtomicBool::new(false))
    }

    /// [`import_tzd`](Self::import_tzd), telling `progress(step, done, total)` as it goes; when
    /// `cancel` is set it stops after the tracks read so far (what's in stays in).
    pub fn import_tzd_with(
        &self,
        path: &std::path::Path,
        group: &str,
        progress: &mut dyn FnMut(&str, usize, usize),
        cancel: &AtomicBool,
    ) -> Result<ImportReport> {
        progress("Reading marks and routes", 0, 0);
        let d = tzd::read_tzd(path)?;
        progress("Matching marks with the library", 0, 0);
        let id = self.begin_import(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "timezero")?;
        let mut rep = self.import_data(d.gpx, group, id)?;
        let n = |k: usize, one: &str, many: &str| if k == 1 { format!("1 {one}") } else { format!("{k} {many}") };
        if !d.shapes.is_empty() {
            let (added, matched) = self.import_tz_shapes(&d.shapes, id)?;
            let what = format!("{}, {} and {}", n(d.lines, "line", "lines"), n(d.areas, "area", "areas"), n(d.circles, "circle", "circles"));
            if added > 0 {
                let again = if matched > 0 { format!(", {matched} already here") } else { String::new() };
                rep.notes.push(format!(
                    "{what} came in as barriers ({added} new{again}), switched off: shown on the chart, not used for routing until ticked (Library > Barriers)"
                ));
            } else if matched > 0 {
                rep.notes.push(format!("{what}: already here as barriers"));
            }
        }
        let skipped = d.lines + d.areas + d.circles - d.shapes.len();
        if skipped > 0 {
            rep.not_imported.push(format!("{} with too few points", n(skipped, "line, area or circle", "lines, areas or circles")));
        }
        self.import_tz_tracks(path, group, id, &mut rep, progress, cancel)?;
        self.finish_import(id, &rep)?;
        Ok(rep)
    }

    /// TimeZero's lines, areas and circles as barriers, switched off (kept, shown, not used by
    /// routing until ticked). A circle becomes a zone of 48 sides. Its TimeZero id is kept, so
    /// importing again matches it. Returns (added, already here).
    fn import_tz_shapes(&self, shapes: &[tzd::TzShape], import_id: i64) -> Result<(usize, usize)> {
        let mut c = self.db();
        let tx = c.transaction()?;
        let mut names: std::collections::HashSet<String> = {
            let mut q = tx.prepare("SELECT name FROM barriers")?;
            let v = q.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?;
            v
        };
        let (mut added, mut matched) = (0, 0);
        let t = now();
        for sh in shapes {
            let kml = sh.id.starts_with("kml-");
            let id = if kml { sh.id.clone() } else { format!("tz-{}", sh.id) };
            let there: bool = tx.query_row("SELECT count(*) FROM barriers WHERE id = ?1", [&id], |r| r.get::<_, i64>(0))? > 0;
            if there {
                matched += 1;
                continue;
            }
            let (kind, points) = match sh.kind.as_str() {
                "circle" => {
                    let [lon, lat] = sh.points[0];
                    let r = sh.radius_m.unwrap_or(1852.0) / 1852.0 / 60.0; // degrees of latitude
                    let k = lat.to_radians().cos().max(0.01);
                    (
                        "zone",
                        (0..48)
                            .map(|i| {
                                let a = i as f64 * std::f64::consts::TAU / 48.0;
                                [lon + r * a.sin() / k, lat + r * a.cos()]
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                "area" => ("zone", sh.points.clone()),
                _ => ("line", sh.points.clone()),
            };
            let label = match sh.kind.as_str() {
                "circle" => "circle",
                "area" => "area",
                _ => "line",
            };
            let from = if kml { "KML" } else { "TimeZero" };
            let base = if sh.name.trim().is_empty() {
                format!("{} {label}", if sh.layer.is_empty() { from } else { sh.layer.as_str() })
            } else {
                sh.name.trim().to_string()
            };
            let name = if names.contains(&base) { (2..).map(|k| format!("{base} {k}")).find(|n| !names.contains(n)).unwrap() } else { base };
            names.insert(name.clone());
            let mut notes = format!(
                "From {from} ({label}{}{})",
                if sh.layer.is_empty() { String::new() } else { format!(", layer {}", sh.layer) },
                if sh.colour.is_empty() { String::new() } else { format!(", colour {}", sh.colour) }
            );
            if let Some(r) = sh.radius_m.filter(|_| sh.kind == "circle") {
                notes.push_str(&format!(", radius {:.0} m", r));
            }
            notes.push(')');
            tx.execute(
                "INSERT INTO barriers (id, name, kind, points, active, notes, modified, import_id) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7)",
                params![id, name, kind, serde_json::to_string(&points)?, notes, t, import_id],
            )?;
            added += 1;
        }
        tx.commit()?;
        Ok((added, matched))
    }

    /// Bring in a KML or KMZ (Google Earth, other plotters): marks, routes, tracks, and areas
    /// as barriers (switched off). Folders become groups.
    pub fn import_kml_with(&self, path: &std::path::Path, group: &str, progress: &mut dyn FnMut(&str, usize, usize)) -> Result<ImportReport> {
        progress("Reading the file", 0, 0);
        let (data, shapes) = formats::read_kml_file(path)?;
        progress("Matching marks with the library", 0, 0);
        let id = self.begin_import(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "kml")?;
        let mut rep = self.import_data(data, group, id)?;
        if !shapes.is_empty() {
            let (added, matched) = self.import_tz_shapes(&shapes, id)?;
            let n = shapes.len();
            rep.notes.push(format!(
                "{n} area{} came in as barriers ({added} new{}), switched off: shown on the chart, not used for routing until ticked (Library > Barriers)",
                if n == 1 { "" } else { "s" },
                if matched > 0 { format!(", {matched} already here") } else { String::new() }
            ));
        }
        self.finish_import(id, &rep)?;
        Ok(rep)
    }

    /// Bring in marks from a CSV file (a spreadsheet): see formats.rs for the columns.
    pub fn import_csv_with(&self, path: &std::path::Path, group: &str, progress: &mut dyn FnMut(&str, usize, usize)) -> Result<ImportReport> {
        progress("Reading the file", 0, 0);
        let bytes = std::fs::read(path)?;
        // a spreadsheet's CSV is often Windows-1252, not UTF-8: read it as Latin-1 then
        let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect());
        let (data, notes) = formats::parse_csv(&text)?;
        progress("Matching marks with the library", 0, 0);
        let id = self.begin_import(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "csv")?;
        let mut rep = self.import_data(data, group, id)?;
        rep.notes.extend(notes);
        self.finish_import(id, &rep)?;
        Ok(rep)
    }

    fn begin_import(&self, file: &str, kind: &str) -> Result<i64> {
        let c = self.db();
        c.execute("INSERT INTO imports (file, kind, at) VALUES (?1, ?2, ?3)", params![file, kind, now()])?;
        Ok(c.last_insert_rowid())
    }

    fn finish_import(&self, id: i64, rep: &ImportReport) -> Result<()> {
        self.db().execute("UPDATE imports SET report = ?2 WHERE id = ?1", params![id, serde_json::to_string(rep)?])?;
        Ok(())
    }

    /// TimeZero's tracks (Tracks.bin), thinned as the track recorder thins live fixes
    /// (TimeZero keeps one every 2 s). A track already here (same start and name) is skipped.
    /// Written a few hundred at a time, so the rest of the app isn't held up for long.
    fn import_tz_tracks(
        &self,
        path: &std::path::Path,
        group: &str,
        import_id: i64,
        rep: &mut ImportReport,
        progress: &mut dyn FnMut(&str, usize, usize),
        cancel: &AtomicBool,
    ) -> Result<()> {
        // a track already here: the same first and last fix (whatever it's called: the file
        // may have been renamed since)
        let have: std::collections::HashSet<(i64, i64)> = self.tracks()?.into_iter().filter_map(|t| t.stats.start.zip(t.stats.end)).collect();
        let (mut fixes, mut kept, mut same) = (0usize, 0usize, 0usize);
        let mut batch: Vec<Track> = Vec::new();
        // marks made at the boat (within 2 s and 30 m of a fix) take its depth, sea
        // temperature and SOG: years of fishing marks with the conditions they were made in
        let mut by_second: std::collections::HashMap<i64, Vec<(usize, f64, f64)>> = std::collections::HashMap::new();
        let marks: Vec<Mark> =
            self.marks()?.into_iter().filter(|m| m.created > 0 && !m.date_unknown && (m.depth.is_none() || m.temp.is_none() || m.sog.is_none())).collect();
        for (i, m) in marks.iter().enumerate() {
            by_second.entry(m.created).or_default().push((i, m.lat, m.lon));
        }
        // best fix for each mark: (seconds off, depth, temp, sog)
        let mut found: std::collections::HashMap<usize, (i64, Option<f32>, Option<f32>, Option<f32>)> = std::collections::HashMap::new();
        let stamp = now();
        let flush = |batch: &mut Vec<Track>, rep: &mut ImportReport| -> Result<()> {
            if batch.is_empty() {
                return Ok(());
            }
            let mut c = self.db();
            let tx = c.transaction()?;
            for t in batch.iter() {
                write_track(&tx, t, Some(import_id))?;
            }
            tx.commit()?;
            rep.tracks_added.extend(batch.drain(..).map(|t| t.name));
            Ok(())
        };
        let mut stopped_at = None;
        let r = tzd::read_tzd_tracks(path, |t| {
            if cancel.load(Ordering::Relaxed) {
                stopped_at = Some((t.n, t.of));
                return Err(Error::Cancelled);
            }
            progress("Tracks", t.n, t.of);
            fixes += t.points.len();
            if !by_second.is_empty() && t.mobile.eq_ignore_ascii_case("OwnShip") {
                for p in &t.points {
                    for dt in -2i64..=2 {
                        let Some(v) = by_second.get(&(p.t + dt)) else { continue };
                        for &(i, lat, lon) in v {
                            let d = nav_core::LatLon::new(lat, lon).distance_to(&nav_core::LatLon::new(p.lat, p.lon)) * 1852.0;
                            if d <= 30.0 && found.get(&i).is_none_or(|f| dt.abs() < f.0) {
                                found.insert(i, (dt.abs(), p.depth, p.temp, p.sog));
                            }
                        }
                    }
                }
            }
            let mut rec = Recorder::new(Track { source: "timezero".into(), colour: t.colour.clone(), depth_ref: "datum".into(), ..Default::default() });
            for p in &t.points {
                rec.push(TrackPoint {
                    t: Some(p.t),
                    lat: p.lat,
                    lon: p.lon,
                    sog: p.sog.map(f64::from),
                    cog: p.cog.map(f64::from),
                    depth: p.depth.map(f64::from),
                    temp: p.temp.map(f64::from),
                });
            }
            let mut tr = rec.track;
            if tr.segments.iter().map(Vec::len).sum::<usize>() < 2 {
                return Ok(());
            }
            // the earliest and latest fix, as the track's totals have them (a track can go back in time)
            let start = tr.segments.iter().flatten().filter_map(|p| p.t).min().unwrap_or(0);
            let end = tr.segments.iter().flatten().filter_map(|p| p.t).max().unwrap_or(0);
            let when = tracks::iso_time(start).get(..16).unwrap_or("").replace('T', " ");
            let who = if t.mobile.eq_ignore_ascii_case("OwnShip") || t.name.is_empty() { group.to_string() } else { t.name.trim().to_string() };
            tr.name = format!("{who} {when}").trim().to_string();
            if have.contains(&(start, end)) {
                same += 1;
                return Ok(());
            }
            kept += tr.segments.iter().map(Vec::len).sum::<usize>();
            tr.id = new_id("t");
            tr.modified = stamp;
            tr.update_stats();
            batch.push(tr);
            if batch.len() >= 200 {
                flush(&mut batch, rep)?;
            }
            Ok(())
        });
        let r = match r {
            Err(Error::Cancelled) => None,
            other => other?,
        };
        flush(&mut batch, rep)?;
        // lines kept among the tracks (brought in from MaxSea) come in as barriers, switched off
        if let Some(lines) = r.as_ref().map(|r| &r.lines).filter(|l| !l.is_empty()) {
            let shapes: Vec<tzd::TzShape> = lines
                .iter()
                .map(|l| {
                    let p0 = l.points[0];
                    tzd::TzShape {
                        id: format!("line-{:016x}", formats::fnv(&format!("{}|{:.5}|{:.5}|{}", l.name, p0.lat, p0.lon, l.points.len()))),
                        name: l.name.clone(),
                        kind: "line".into(),
                        points: l.points.iter().map(|p| [p.lon, p.lat]).collect(),
                        radius_m: None,
                        layer: "MaxSea".into(),
                        colour: l.colour.clone(),
                    }
                })
                .collect();
            let (added, matched) = self.import_tz_shapes(&shapes, import_id)?;
            rep.notes.push(format!(
                "{} line{} from the track file (brought in from MaxSea) came in as barriers ({added} new{}), switched off",
                shapes.len(),
                if shapes.len() == 1 { "" } else { "s" },
                if matched > 0 { format!(", {matched} already here") } else { String::new() }
            ));
        }
        if let Some((n, of)) = stopped_at {
            rep.not_imported.push(format!("tracks from {n} of {of} on (stopped part way: import the file again to bring in the rest)"));
        }
        progress("Filling in the conditions of marks made at the boat", 0, 0);
        if !found.is_empty() {
            let mut c = self.db();
            let tx = c.transaction()?;
            {
                let mut up = tx.prepare(
                    "UPDATE marks SET depth = coalesce(depth, ?2), temp = coalesce(temp, ?3), sog = coalesce(sog, ?4),
                     tags = CASE WHEN ',' || tags || ',' LIKE '%,at-boat,%' THEN tags WHEN tags = '' THEN 'at-boat' ELSE tags || ',at-boat' END
                     WHERE id = ?1",
                )?;
                let r1 = |v: Option<f32>, k: f64| v.map(|x| (x as f64 * k).round() / k);
                for (i, (_, d, t, s)) in &found {
                    up.execute(params![marks[*i].id, r1(*d, 10.0), r1(*t, 100.0), r1(*s, 100.0)])?;
                }
            }
            tx.commit()?;
            rep.notes.push(format!("{} marks were made at the boat: their depth, sea temperature and speed came from the track", found.len()));
        }
        if let Some(p) = r.as_ref().and_then(|r| r.problem.clone()) {
            let r = r.as_ref().unwrap();
            rep.not_imported.push(format!("tracks after {} of {} (TimeZero's Tracks.bin: {p})", r.read, r.count));
        }
        if let Some(r) = r.as_ref().filter(|r| !r.other.is_empty()) {
            let o: Vec<String> = r.other.iter().map(|(k, n)| format!("{n} {}{}", k.to_lowercase(), if *n == 1 { "" } else { "s" })).collect();
            rep.not_imported.push(format!("from the track file: {} (not yet)", o.join(", ")));
        }
        if r.is_some() && fixes > 0 {
            rep.notes.push(format!(
                "tracks: kept {kept} of {fixes} fixes (a fix every 10 s, sooner in turns){}",
                if same > 0 { format!("; {same} already here") } else { String::new() }
            ));
        }
        Ok(())
    }

    fn import_data(&self, data: GpxData, group: &str, import_id: i64) -> Result<ImportReport> {
        let mut rep = ImportReport::default();
        let mut marks = self.marks()?;
        let first_new = marks.len();
        // marks made in the same second by the hundred came in from another plotter in one
        // go (a fleet's SeaPlot marks into TimeZero): their date is that day, not when made
        let mut per_second: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
        for w in data.waypoints.iter().chain(data.routes.iter().flat_map(|r| r.points.iter())) {
            if let Some(t) = w.time {
                *per_second.entry(t).or_default() += 1;
            }
        }
        // Marks already here, by name and a grid of about a kilometre, so a big file (a
        // fisherman's tens of thousands of marks) matches in moments, not hours.
        const CELL: f64 = 0.01;
        let cell = |lat: f64, lon: f64| ((lat / CELL).floor() as i64, (lon / CELL).floor() as i64);
        let mut index: std::collections::HashMap<(String, (i64, i64)), Vec<usize>> = std::collections::HashMap::new();
        for (i, m) in marks.iter().enumerate() {
            index.entry((m.name.to_lowercase(), cell(m.lat, m.lon))).or_default().push(i);
        }
        let stamp = now();
        // what the user has said each TimeZero symbol means
        let symbol_kinds: std::collections::HashMap<String, String> = {
            let c = self.db();
            let mut q = c.prepare("SELECT symbol, kind FROM tz_symbols WHERE kind <> ''")?;
            let v = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            v
        };
        let mut find_or_add = |w: &GpxWaypoint, rep: &mut ImportReport| -> String {
            let here = nav_core::LatLon::new(w.lat, w.lon);
            let name = w.name.trim().to_string();
            let key = name.to_lowercase();
            let (cy, cx) = cell(w.lat, w.lon);
            // a named mark within about 50 m is the same mark; unnamed ones (a fisher's
            // marks are often a few boat lengths apart) only when within a few metres
            let near = if key.is_empty() { SAME_UNNAMED_NM } else { SAME_MARK_NM };
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(ids) = index.get(&(key.clone(), (cy + dy, cx + dx))) {
                        if let Some(&i) = ids.iter().find(|&&i| nav_core::LatLon::new(marks[i].lat, marks[i].lon).distance_to(&here) < near) {
                            rep.marks_matched += 1;
                            return marks[i].id.clone();
                        }
                    }
                }
            }
            let symbol = match w.tz_symbol {
                Some(n) => format!("tz:{n}"),
                None => w.sym.trim().to_string(),
            };
            let name_info = read_name(&name);
            let mut m = Mark {
                id: new_id("m"),
                // unnamed marks (most of a plotter's) stay unnamed
                name,
                lat: w.lat,
                lon: w.lon,
                kind: if w.sym.to_ascii_lowercase().contains("buoy") { "mark".into() } else { "waypoint".into() },
                notes: w.desc.clone(),
                group: if w.group.is_empty() { group.to_string() } else { w.group.clone() },
                barrier_nm: 0.0,
                modified: stamp,
                created: w.time.unwrap_or(0),
                date_unknown: w.time.is_some_and(|t| per_second.get(&t).copied().unwrap_or(0) >= BULK_SAME_SECOND),
                colour: w.colour.clone(),
                symbol,
                depth: name_info.depth,
                temp: None,
                sog: None,
                tags: {
                    let mut t = name_info.tags.clone();
                    if w.quick && !t.contains(&"event") {
                        t.push("event");
                    }
                    t.join(",")
                },
                data: serde_json::Value::Null,
            };
            if let Some(k) = symbol_kinds.get(&m.symbol) {
                m.kind = k.clone();
            }
            // what the name says (foul ground, a wreck) wins over the symbol
            if let Some(k) = name_info.kind {
                m.kind = k.into();
            }
            rep.marks_added += 1;
            let id = m.id.clone();
            index.entry((key, (cy, cx))).or_default().push(marks.len());
            marks.push(m);
            id
        };
        for w in &data.waypoints {
            find_or_add(w, &mut rep);
        }
        // a route with the same name and the same marks in the same order is already here
        let route_key = |name: &str, marks: &[RouteMark]| (name.to_lowercase(), marks.iter().map(|m| m.mark.clone()).collect::<Vec<_>>());
        let mut known: std::collections::HashSet<(String, Vec<String>)> = self.routes()?.iter().map(|r| route_key(&r.name, &r.marks)).collect();
        let mut new_routes = Vec::new();
        for (k, r) in data.routes.iter().enumerate() {
            let ids: Vec<RouteMark> = r.points.iter().map(|w| RouteMark { mark: find_or_add(w, &mut rep), ..Default::default() }).collect();
            let name = if r.name.trim().is_empty() { format!("Imported route {}", k + 1) } else { r.name.trim().to_string() };
            if !known.insert(route_key(&name, &ids)) {
                rep.routes_matched += 1;
                continue;
            }
            new_routes.push(Route { id: new_id("r"), name, notes: String::new(), start: None, start_mark: None, marks: ids, modified: stamp });
        }
        let mut new_tracks = Vec::new();
        for (k, t) in data.tracks.into_iter().enumerate() {
            let name = if t.name.trim().is_empty() {
                if group.is_empty() {
                    format!("Imported track {}", k + 1)
                } else {
                    format!("{group} {}", k + 1)
                }
            } else {
                t.name.trim().to_string()
            };
            let mut tr = Track { id: new_id("t"), name, notes: t.desc, source: "gpx".into(), segments: t.segments, modified: stamp, ..Default::default() };
            tr.update_stats();
            new_tracks.push(tr);
        }
        // everything new in one go: a crash part way leaves nothing half done
        let mut c = self.db();
        let tx = c.transaction()?;
        for m in &marks[first_new..] {
            insert_mark(&tx, m, Some(import_id))?;
        }
        for r in &new_routes {
            write_route(&tx, r, Some(import_id))?;
            rep.routes_added.push(r.name.clone());
        }
        for t in &new_tracks {
            write_track(&tx, t, Some(import_id))?;
            rep.tracks_added.push(t.name.clone());
        }
        tx.commit()?;
        Ok(rep)
    }

    // ---------- tracks ----------

    /// Every track, without points, newest first.
    pub fn tracks(&self) -> Result<Vec<TrackSummary>> {
        let c = self.db();
        let mut q = c.prepare_cached(&format!("SELECT {TRACK_COLS} FROM tracks WHERE deleted_at IS NULL ORDER BY coalesce(start, modified) DESC"))?;
        let mut v = q.query_map([], track_summary_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let refs = depth_refs(&c);
        for t in &mut v {
            if let Some((r, o)) = refs.get(&t.id) {
                (t.depth_ref, t.depth_offset) = (r.clone(), *o);
            }
        }
        Ok(v)
    }

    /// Say what the depths of tracks `ids` are measured from (see [`Track::depth_ref`]).
    /// Their modified time moves on, so soundings already built from them show as changed.
    pub fn set_depth_ref(&self, ids: &[String], kind: &str, offset: Option<f64>) -> Result<usize> {
        if !["", "surface", "transducer", "keel", "datum"].contains(&kind) {
            return Err(Error::Store(format!("unknown depth reference {kind:?}")));
        }
        let offset = if matches!(kind, "transducer" | "keel") { offset } else { None };
        let mut c = self.db();
        let tx = c.transaction()?;
        let mut n = 0;
        {
            let mut q = tx.prepare_cached("UPDATE tracks SET depth_ref = ?2, depth_offset = ?3, modified = ?4 WHERE id = ?1 AND deleted_at IS NULL")?;
            for id in ids {
                n += q.execute(params![id, kind, offset, now()])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Tracks with depths whose reference isn't known (of the newest import only, or all).
    pub fn depth_unknown(&self, newest_import_only: bool) -> Result<Vec<String>> {
        let c = self.db();
        let sql = if newest_import_only {
            "SELECT id FROM tracks WHERE has_depth = 1 AND depth_ref = '' AND deleted_at IS NULL AND import_id = (SELECT max(id) FROM imports)"
        } else {
            "SELECT id FROM tracks WHERE has_depth = 1 AND depth_ref = '' AND deleted_at IS NULL"
        };
        let mut q = c.prepare(sql)?;
        let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    pub fn track(&self, id: &str) -> Result<Track> {
        let c = self.db();
        let s = c
            .query_row(&format!("SELECT {TRACK_COLS} FROM tracks WHERE id = ?1 AND deleted_at IS NULL"), [id], track_summary_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(id.into()))?;
        let data: Vec<u8> = c.query_row("SELECT data FROM track_data WHERE track_id = ?1", [id], |r| r.get(0)).optional()?.unwrap_or_default();
        let segments = if data.is_empty() { Vec::new() } else { store::decode_segments(&data)? };
        let (depth_ref, depth_offset) = c
            .query_row("SELECT depth_ref, depth_offset FROM tracks WHERE id = ?1", [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<f64>>(1)?)))
            .unwrap_or_default();
        Ok(Track {
            id: s.id,
            name: s.name,
            notes: s.notes,
            colour: s.colour,
            source: s.source,
            modified: s.modified,
            stats: s.stats,
            segments,
            depth_ref,
            depth_offset,
        })
    }

    /// Save a track (new id if it has none); its totals are worked out again.
    pub fn save_track(&self, mut t: Track) -> Result<TrackSummary> {
        if t.id.is_empty() {
            t.id = new_id("t");
        }
        t.modified = now();
        t.update_stats();
        self.store_track(&t)?;
        Ok(t.summary())
    }

    /// A fresh track id (for a track that will be saved later).
    pub fn new_track_id(&self) -> String {
        new_id("t")
    }

    /// Save a track as it is (totals kept), e.g. a track being recorded.
    pub fn store_track(&self, t: &Track) -> Result<()> {
        let mut c = self.db();
        let tx = c.transaction()?;
        write_track(&tx, t, None)?;
        tx.commit()?;
        Ok(())
    }

    /// Change a track's name, notes and colour.
    pub fn update_track_info(&self, id: &str, name: &str, notes: &str, colour: &str) -> Result<TrackSummary> {
        let n = self.db().execute(
            "UPDATE tracks SET name = ?2, notes = ?3, colour = ?4, modified = ?5 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, name.trim(), notes, colour, now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound(id.into()));
        }
        let c = self.db();
        Ok(c.query_row(&format!("SELECT {TRACK_COLS} FROM tracks WHERE id = ?1"), [id], track_summary_row)?)
    }

    pub fn delete_track(&self, id: &str) -> Result<()> {
        self.soft_delete("tracks", id)
    }

    /// Write text into the export folder; returns the full path.
    pub fn write_export(&self, name: &str, text: &str) -> Result<PathBuf> {
        let mut base = safe(name);
        if base.is_empty() {
            base = "export".into();
        }
        if !base.to_ascii_lowercase().ends_with(".gpx") {
            base.push_str(".gpx");
        }
        let p = self.dir.join("export").join(base);
        std::fs::write(&p, text)?;
        Ok(p)
    }

    // ---------- backups ----------

    /// A copy of the whole library at `to` (compacted; safe while TweedPlot runs: it reads
    /// through its own connection, so nothing waits for it).
    pub fn backup_to(&self, to: &Path) -> Result<()> {
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = to.with_extension("gpkg.part");
        let _ = std::fs::remove_file(&tmp);
        let c = Connection::open_with_flags(self.path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        c.busy_timeout(std::time::Duration::from_secs(30))?;
        c.execute("VACUUM INTO ?1", [tmp.to_string_lossy()])?;
        drop(c);
        std::fs::rename(&tmp, to)?;
        Ok(())
    }

    /// The day's backup in `<dir>/backups` (library-YYYY-MM-DD.gpkg) if it hasn't been made,
    /// keeping one a day for the last 7 days and one a week for 4 weeks before that.
    pub fn backup_if_due(&self) -> Result<Option<PathBuf>> {
        self.backup_if_due_on(tracks::iso_time(now()).get(..10).unwrap_or(""))
    }

    /// As [`backup_if_due`], with today's date as the boat's clock has it ("2026-10-28"):
    /// in New Zealand the UTC date is yesterday's until midday.
    pub fn backup_if_due_on(&self, today: &str) -> Result<Option<PathBuf>> {
        let dir = self.dir.join("backups");
        let today = today.to_string();
        let stem = self.name();
        let to = dir.join(format!("{stem}-{today}.gpkg"));
        let made = if to.exists() {
            None
        } else {
            self.backup_to(&to)?;
            Some(to)
        };
        // thin the old ones
        let prefix = format!("{stem}-");
        let day = |name: &str| {
            name.strip_prefix(prefix.as_str())
                .and_then(|n| n.strip_suffix(".gpkg"))
                .filter(|d| d.len() == 10)
                .and_then(|d| tracks::parse_iso(&format!("{d}T00:00:00Z")))
                .map(|t| t / 86_400)
        };
        let mut all: Vec<(i64, PathBuf)> =
            std::fs::read_dir(&dir)?.flatten().filter_map(|e| day(&e.file_name().to_string_lossy()).map(|d| (d, e.path()))).collect();
        all.sort();
        let now_day = tracks::parse_iso(&format!("{today}T00:00:00Z")).map(|t| t / 86_400).unwrap_or(now() / 86_400);
        let mut weeks = std::collections::HashSet::new();
        for (d, p) in all.iter().rev() {
            let age = now_day - d;
            let keep = age < 7 || (age < 35 && weeks.insert(d / 7));
            if !keep {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(made)
    }

    // ---------- the one-off move from the JSON files ----------

    /// Whether the folder still has a library kept as JSON files (TweedPlot before Oct 2026).
    pub fn needs_conversion(&self) -> bool {
        if self.file != store::FILE {
            return false; // only the library in the app's own folder came from JSON files
        }
        let d = &self.dir;
        let any_json =
            |sub: &str| std::fs::read_dir(d.join(sub)).map(|r| r.flatten().any(|e| e.path().extension().is_some_and(|x| x == "json"))).unwrap_or(false);
        d.join("marks.json").exists() || d.join("barriers.json").exists() || any_json("routes") || any_json("routings") || any_json("tracks")
    }

    /// Move a JSON library into the database. The JSON files are then moved, untouched, to
    /// `<dir>/json-backup-<date>`. `progress(step, done, total)` is called as it goes.
    /// The database lock is let go between pieces, so the app keeps working meanwhile.
    pub fn convert_json(&self, mut progress: impl FnMut(&str, usize, usize)) -> Result<String> {
        if !self.needs_conversion() {
            return Ok(String::new());
        }
        self.converting.store(true, Ordering::Relaxed);
        let r = self.convert_json_inner(&mut progress);
        self.converting.store(false, Ordering::Relaxed);
        r
    }

    fn convert_json_inner(&self, progress: &mut impl FnMut(&str, usize, usize)) -> Result<String> {
        let d = self.dir.clone();
        let read = |p: &Path| -> Option<Vec<u8>> { std::fs::read(p).ok() };
        let files = |sub: &str| -> Vec<PathBuf> {
            let mut v: Vec<PathBuf> = std::fs::read_dir(d.join(sub))
                .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
                .unwrap_or_default();
            v.sort();
            v
        };
        let (mut nm, mut nr, mut nx, mut nb, mut nt, mut bad) = (0, 0, 0, 0, 0, 0);
        progress("marks", 0, 1);
        {
            let marks: Vec<Mark> = read(&d.join("marks.json")).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let barriers: Vec<Barrier> = read(&d.join("barriers.json")).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let mut c = self.db();
            let tx = c.transaction()?;
            for m in &marks {
                insert_mark(&tx, m, None)?;
                nm += 1;
            }
            for b in &barriers {
                tx.execute(
                    "INSERT OR REPLACE INTO barriers (id, name, kind, points, active, notes, modified) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![b.id, b.name, b.kind, serde_json::to_string(&b.points)?, b.active as i64, b.notes, b.modified],
                )?;
                nb += 1;
            }
            for p in files("routes") {
                match read(&p).and_then(|b| serde_json::from_slice::<Route>(&b).ok()) {
                    Some(r) => {
                        write_route(&tx, &r, None)?;
                        nr += 1;
                    }
                    None => bad += 1,
                }
            }
            for p in files("routings") {
                match read(&p).and_then(|b| serde_json::from_slice::<Routing>(&b).ok()) {
                    Some(r) => {
                        write_routing(&tx, &r)?;
                        nx += 1;
                    }
                    None => bad += 1,
                }
            }
            tx.commit()?;
        }
        let tracks = files("tracks");
        let total = tracks.len();
        for chunk in tracks.chunks(100) {
            // read and decode outside the lock; write in one go
            let parsed: Vec<Option<Track>> = chunk.iter().map(|p| read(p).and_then(|b| serde_json::from_slice::<Track>(&b).ok())).collect();
            let mut c = self.db();
            let tx = c.transaction()?;
            for t in parsed {
                match t {
                    Some(t) => {
                        write_track(&tx, &t, None)?;
                        nt += 1;
                    }
                    None => bad += 1,
                }
            }
            tx.commit()?;
            drop(c);
            progress("tracks", nt + bad, total);
        }
        // keep the JSON files, out of the way
        let backup = d.join(format!("json-backup-{}", tracks::iso_time(now()).get(..10).unwrap_or("old")));
        std::fs::create_dir_all(&backup)?;
        for name in ["marks.json", "barriers.json", "routes", "routings", "tracks"] {
            let p = d.join(name);
            if p.exists() {
                std::fs::rename(&p, backup.join(name))?;
            }
        }
        let summary = format!(
            "{nm} marks, {nr} routes, {nx} routings, {nb} barriers and {nt} tracks moved into the library database{}; the old files are in {}",
            if bad > 0 { format!(" ({bad} damaged files skipped)") } else { String::new() },
            backup.display()
        );
        store::set_meta(&self.db(), "converted_json", &summary)?;
        Ok(summary)
    }
}

// ---------- helpers ----------

/// Once, for marks brought in before TweedPlot read these things at import: what their
/// names say (depth, tags, foul ground, wrecks, hazards), and which dates are only the day
/// they were moved in from another plotter (20 or more made in the same second).
fn review_marks(c: &Connection) -> Result<()> {
    let tx = c.unchecked_transaction()?;
    tx.execute(
        "UPDATE marks SET date_unknown = 1 WHERE created > 0 AND created IN
         (SELECT created FROM marks WHERE created > 0 GROUP BY created HAVING count(*) >= ?1)",
        [BULK_SAME_SECOND as i64],
    )?;
    let rows: Vec<(String, String, String, Option<f64>, String, String)> = {
        let mut q = tx.prepare("SELECT id, name, kind, depth, tags, symbol FROM marks WHERE name <> '' AND deleted_at IS NULL")?;
        let v = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        v
    };
    {
        let mut up = tx.prepare("UPDATE marks SET depth = ?2, tags = ?3, kind = ?4 WHERE id = ?1")?;
        for (id, name, kind, depth, tags, symbol) in rows {
            let info = read_name(&name);
            if info.depth.is_none() && info.tags.is_empty() {
                continue;
            }
            let mut t: Vec<&str> = tags.split(',').filter(|x| !x.is_empty()).collect();
            for x in &info.tags {
                if !t.contains(x) {
                    t.push(x);
                }
            }
            // the kind only for marks still as imported (a waypoint with a plotter's symbol)
            let k = match info.kind {
                Some(k) if kind == "waypoint" && !symbol.is_empty() => k.to_string(),
                _ => kind,
            };
            up.execute(params![id, depth.or(info.depth), t.join(","), k])?;
        }
    }
    store::set_meta(&tx, "marks_reviewed", "1")?;
    tx.commit()?;
    Ok(())
}

/// What a fisher's mark name says about the place.
#[derive(Debug, Default, PartialEq)]
struct NameInfo {
    depth: Option<f64>,
    tags: Vec<&'static str>,
    kind: Option<&'static str>,
}

/// Depth from names like "180 m", "200MTS", "45.5 m" or "60 fm" (fathoms); words a skipper
/// writes on a mark: foul ground (foul, fast, snag, hang, ripped), rock, wreck, the bottom
/// (hard, rough, rolly, soft, mud, sand, shell, gravel, weed), tow; "?" unsure, "!" caution;
/// "Event 12" a mark dropped with the event button. A plain number ("6") is the mark's
/// number, not a depth.
fn read_name(name: &str) -> NameInfo {
    let mut out = NameInfo::default();
    let n = name.trim();
    if n.is_empty() {
        return out;
    }
    let low = n.to_ascii_lowercase();
    // depth: a number with a unit
    let num_end = low.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(low.len());
    if num_end > 0 {
        if let Ok(v) = low[..num_end].parse::<f64>() {
            let unit = low[num_end..].trim();
            let d = match unit {
                "m" | "mt" | "mts" | "mtrs" | "metres" | "meters" | "metre" | "meter" => Some(v),
                "fm" | "fms" | "fath" | "fathoms" => Some(v * 1.8288),
                _ => None,
            };
            if let Some(d) = d.filter(|d| *d > 0.0 && *d < 11_000.0) {
                out.depth = Some((d * 10.0).round() / 10.0);
                out.tags.push("depth");
            }
        }
    }
    let words: Vec<&str> = low.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    let tag = |t: &'static str, out: &mut NameInfo| {
        if !out.tags.contains(&t) {
            out.tags.push(t);
        }
    };
    if has(&["foul", "fast", "snag", "snagged", "hang", "hangs", "ripped", "torn", "caught"]) {
        tag("foul", &mut out);
        out.kind = Some("foul");
    }
    if has(&["rock", "rocks", "reef", "pinnacle"]) {
        tag("rock", &mut out);
        out.kind.get_or_insert("sym-rock");
    }
    if has(&["wreck", "wrk"]) {
        tag("wreck", &mut out);
        out.kind = Some("sym-wreck");
    }
    for (w, t) in [
        ("hard", "hard"),
        ("rough", "rough"),
        ("rolly", "rough"),
        ("soft", "soft"),
        ("mud", "mud"),
        ("sand", "sand"),
        ("shell", "shell"),
        ("gravel", "gravel"),
        ("weed", "weed"),
        ("tow", "tow"),
    ] {
        if words.contains(&w) {
            tag(t, &mut out);
        }
    }
    if n.contains('?') {
        tag("unsure", &mut out);
    }
    if n.contains('!') {
        tag("caution", &mut out);
        out.kind.get_or_insert("hazard");
    }
    if words.first() == Some(&"event") {
        tag("event", &mut out);
    }
    out
}

fn routes_where(c: &Connection, cond: &str) -> Result<Vec<Route>> {
    let mut q = c.prepare(&format!(
        "SELECT r.id, r.name, r.notes, r.start_lon, r.start_lat, r.start_mark, r.modified FROM routes r WHERE r.deleted_at IS NULL AND {cond} ORDER BY r.modified DESC"
    ))?;
    let mut routes = q
        .query_map([], |r| {
            let (lon, lat): (Option<f64>, Option<f64>) = (r.get(3)?, r.get(4)?);
            Ok(Route {
                id: r.get(0)?,
                name: r.get(1)?,
                notes: r.get(2)?,
                start: lon.zip(lat).map(|(a, b)| [a, b]),
                start_mark: r.get(5)?,
                marks: Vec::new(),
                modified: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let at: std::collections::HashMap<String, usize> = routes.iter().enumerate().map(|(i, r)| (r.id.clone(), i)).collect();
    let mut q = c.prepare_cached("SELECT route_id, mark_id, rounding, radius_nm FROM route_marks ORDER BY route_id, seq")?;
    let mut rows = q.query([])?;
    while let Some(r) = rows.next()? {
        let rid: String = r.get(0)?;
        if let Some(&i) = at.get(&rid) {
            routes[i].marks.push(RouteMark { mark: r.get(1)?, rounding: r.get(2)?, radius_nm: r.get(3)? });
        }
    }
    Ok(routes)
}

fn write_routing(c: &Connection, r: &Routing) -> Result<()> {
    let polar =
        r.request["polar_path"].as_str().map(|p| Path::new(&p.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    c.execute(
        "INSERT INTO routings (id, name, created, route_id, route_name, t0, status, arrival, distance_nm, grib_name, polar, request, result)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, created = excluded.created, route_id = excluded.route_id,
           route_name = excluded.route_name, t0 = excluded.t0, status = excluded.status, arrival = excluded.arrival,
           distance_nm = excluded.distance_nm, grib_name = excluded.grib_name, polar = excluded.polar,
           request = excluded.request, result = excluded.result, deleted_at = NULL",
        params![
            r.id,
            r.name,
            r.created,
            r.route_id,
            r.route_name,
            r.request["t0"].as_i64(),
            r.result["status"].as_str(),
            r.result["arrival"].as_i64(),
            r.result["distance_nm"].as_f64(),
            r.request["grib_name"].as_str(),
            polar,
            serde_json::to_string(&r.request)?,
            serde_json::to_string(&r.result)?
        ],
    )?;
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Unique enough for one person's library: time in ms plus a counter.
fn new_id(prefix: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{prefix}{ms:x}{:03x}", N.fetch_add(1, Ordering::Relaxed) % 4096)
}

/// "WP 1", "WP 2" … the first free one.
fn next_name(c: &Connection, stem: &str) -> Result<String> {
    let mut q = c.prepare("SELECT name FROM marks WHERE deleted_at IS NULL AND name LIKE ?1")?;
    let used: std::collections::HashSet<String> = q.query_map([format!("{stem} %")], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok((1..).map(|k| format!("{stem} {k}")).find(|n| !used.contains(n)).unwrap())
}

/// File-name safe version of an id or name.
fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_lib(tag: &str) -> Library {
        let d = std::env::temp_dir().join(format!("racenav-lib-{tag}-{}", new_id("")));
        Library::open(d).unwrap()
    }

    #[test]
    fn a_mark_keeps_its_own_details() {
        // an anchor's swing radius, bottom and holding (anchor.js) round-trip; a plain mark has none
        let lib = temp_lib("markdata");
        let a = lib
            .save_mark(Mark {
                name: "Anchor".into(),
                kind: "anchor".into(),
                group: "Anchorages".into(),
                lat: -41.25,
                lon: 173.27,
                data: serde_json::json!({ "radius_m": 45, "bottom": "mud", "holding": 4 }),
                ..Default::default()
            })
            .unwrap();
        let plain = lib.save_mark(Mark { name: "WP".into(), lat: -41.0, lon: 173.0, ..Default::default() }).unwrap();
        let marks = lib.marks().unwrap();
        let back = marks.iter().find(|m| m.id == a.id).unwrap();
        assert_eq!(back.data["radius_m"], 45);
        assert_eq!(back.data["bottom"], "mud");
        let p = marks.iter().find(|m| m.id == plain.id).unwrap();
        assert!(is_empty_json(&p.data));
        assert!(serde_json::to_value(p).unwrap().get("data").is_none());
    }

    #[test]
    fn daily_backup_is_a_whole_library() {
        let lib = temp_lib("backup");
        lib.save_mark(Mark { name: "A".into(), lat: -36.0, lon: 174.0, ..Default::default() }).unwrap();
        let p = lib.backup_if_due().unwrap().expect("first backup today");
        assert!(lib.backup_if_due().unwrap().is_none(), "one a day");
        // an old one beyond the weeks kept is cleared out
        let old = lib.dir().join("backups").join("library-2001-01-01.gpkg");
        std::fs::write(&old, b"x").unwrap();
        lib.backup_if_due().unwrap();
        assert!(!old.exists());
        let copy = Library::open(p.parent().unwrap().join("restore")).unwrap();
        drop(copy);
        let restored = Connection::open(&p).unwrap();
        let n: i64 = restored.query_row("SELECT count(*) FROM marks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn plotter_symbols_given_a_meaning() {
        let lib = temp_lib("sym");
        let mut g = String::from(r#"<?xml version="1.0"?><gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1">"#);
        for k in 0..5 {
            g.push_str(&format!(
                r#"<wpt lat="-41.{k}" lon="173.2"><extensions><MxTimeZeroSymbol>63</MxTimeZeroSymbol><color>-65536</color></extensions></wpt>"#
            ));
        }
        g.push_str(r#"<wpt lat="-41.9" lon="173.9"><name>FOUL</name><extensions><MxTimeZeroSymbol>63</MxTimeZeroSymbol></extensions></wpt>"#);
        g.push_str(r#"<wpt lat="-42" lon="174"><extensions><MxTimeZeroSymbol>2</MxTimeZeroSymbol></extensions></wpt></gpx>"#);
        lib.import_gpx(&g, "TZ").unwrap();
        let s = lib.symbols().unwrap();
        assert_eq!((s[0].symbol.as_str(), s[0].count), ("tz:63", 6));
        assert_eq!(s[0].colours[0], ("#ff0000".to_string(), 5));
        assert_eq!(lib.set_symbol("tz:63", "fishing", "Longline station").unwrap(), 5, "not the foul one");
        assert_eq!(lib.marks().unwrap().iter().filter(|m| m.kind == "fishing").count(), 5);
        assert_eq!(lib.marks().unwrap().iter().find(|m| m.name == "FOUL").unwrap().kind, "foul");
        // changed again: the ones it set follow
        assert_eq!(lib.set_symbol("tz:63", "anchorage", "").unwrap(), 5);
        // later imports take it too
        lib.import_gpx(r#"<?xml version="1.0"?><gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1"><wpt lat="-45" lon="170"><extensions><MxTimeZeroSymbol>63</MxTimeZeroSymbol></extensions></wpt></gpx>"#, "TZ").unwrap();
        assert_eq!(lib.marks().unwrap().iter().filter(|m| m.kind == "anchorage").count(), 6);
        assert_eq!(lib.symbols().unwrap()[0].kind, "anchorage");
    }

    #[test]
    fn names_say_depth_and_ground() {
        let d = |n: &str| read_name(n);
        assert_eq!(d("180 m").depth, Some(180.0));
        assert_eq!(d("200MTS").depth, Some(200.0));
        assert_eq!(d("45.5 m").depth, Some(45.5));
        assert_eq!(d("10 fm").depth, Some(18.3));
        assert_eq!(d("6").depth, None, "a mark's number");
        assert_eq!(d("13:43").depth, None);
        assert_eq!((d("FOUL").kind, d("FOUL").tags), (Some("foul"), vec!["foul"]));
        assert_eq!(d("hard bottom, rolly").tags, vec!["hard", "rough"]);
        assert_eq!(d("ROCK").kind, Some("sym-rock"));
        assert_eq!(d("?").tags, vec!["unsure"]);
        assert_eq!(d("Event 0035").tags, vec!["event"]);
        assert_eq!(d("fast 3 times!").kind, Some("foul"));
    }

    #[test]
    fn merging_another_boats_library() {
        let base = std::env::temp_dir().join(format!("racenav-merge-{}", new_id("")));
        let a = Library::open_file(base.join("ours.gpkg")).unwrap();
        let b = Library::open_file(base.join("other boat").join("theirs.gpkg")).unwrap();
        assert_eq!(b.name(), "theirs");
        // both have Bean Rock (same name, a few metres apart); theirs has a new mark, a route and a track
        a.save_mark(Mark { name: "Bean Rock".into(), lat: -36.8317, lon: 174.8303, ..Default::default() }).unwrap();
        let br = b.save_mark(Mark { name: "Bean Rock".into(), lat: -36.83171, lon: 174.83031, ..Default::default() }).unwrap();
        let hole = b.save_mark(Mark { name: "The Hole".into(), lat: -36.9, lon: 174.9, colour: "#ff0000".into(), ..Default::default() }).unwrap();
        b.save_route(Route {
            name: "Out and back".into(),
            marks: vec![RouteMark { mark: br.id.clone(), ..Default::default() }, RouteMark { mark: hole.id.clone(), ..Default::default() }],
            ..Default::default()
        })
        .unwrap();
        let p = |t: i64| TrackPoint { t: Some(t), lat: -36.8, lon: 174.8 + t as f64 * 1e-6, sog: Some(6.0), ..Default::default() };
        b.save_track(Track { name: "Trip".into(), segments: vec![vec![p(1000), p(1010), p(1020)]], ..Default::default() }).unwrap();
        let stop = AtomicBool::new(false);
        let r = a.merge_from(&b.path(), &mut |_, _, _| {}, &stop).unwrap();
        assert_eq!((r.marks_added, r.marks_matched, r.routes_added.len(), r.tracks_added.len()), (1, 1, 1, 1));
        let m = a.marks().unwrap();
        assert_eq!(m.len(), 2);
        let ours_br = m.iter().find(|x| x.name == "Bean Rock").unwrap();
        let route = &a.routes().unwrap()[0];
        assert_eq!(route.marks[0].mark, ours_br.id, "the route uses our Bean Rock");
        assert_eq!(m.iter().find(|x| x.name == "The Hole").unwrap().colour, "#ff0000");
        assert_eq!(a.track(&a.tracks().unwrap()[0].id).unwrap().segments[0].len(), 3);
        // merging again adds nothing
        let r2 = a.merge_from(&b.path(), &mut |_, _, _| {}, &stop).unwrap();
        assert_eq!((r2.marks_added, r2.routes_added.len(), r2.tracks_added.len()), (0, 0, 0));
        assert!(a.merge_from(&a.path(), &mut |_, _, _| {}, &stop).is_err(), "not into itself");
        // a library opened by file keeps its backups by its own name
        let made = b.backup_if_due().unwrap().unwrap();
        assert!(made.file_name().unwrap().to_string_lossy().starts_with("theirs-"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn changing_many_marks_at_once() {
        let lib = temp_lib("bulk-edit");
        let ids: Vec<String> = (0..3)
            .map(|k| {
                lib.save_mark(Mark { name: format!("M{k}"), lat: -36.0 - k as f64 * 0.1, lon: 174.0, group: "old".into(), ..Default::default() }).unwrap().id
            })
            .collect();
        assert_eq!(lib.update_marks(&ids[..2], Some("Tuna 2024"), Some("fishing"), None).unwrap(), 2);
        let m = lib.marks().unwrap();
        assert_eq!(m.iter().filter(|x| x.group == "Tuna 2024" && x.kind == "fishing").count(), 2);
        assert_eq!(m.iter().find(|x| x.name == "M2").unwrap().group, "old");
        lib.update_marks(&ids, None, None, Some("#00ff00")).unwrap();
        assert!(lib.marks().unwrap().iter().all(|x| x.colour == "#00ff00"));
    }

    #[test]
    fn converts_a_json_library() {
        let d = std::env::temp_dir().join(format!("racenav-conv-{}", new_id("")));
        std::fs::create_dir_all(d.join("routes")).unwrap();
        std::fs::create_dir_all(d.join("tracks")).unwrap();
        std::fs::create_dir_all(d.join("routings")).unwrap();
        std::fs::write(d.join("marks.json"), r##"[{"id":"m1","name":"Bean Rock","lat":-36.83,"lon":174.83,"kind":"mark","notes":"","group":"RNZYS","barrier_nm":0,"modified":5},
            {"id":"m2","name":"","lat":-36.9,"lon":174.9,"kind":"waypoint","notes":"","group":"","barrier_nm":0,"modified":6,"colour":"#ff0000","symbol":"tz:14"}]"##).unwrap();
        std::fs::write(
            d.join("barriers.json"),
            r#"[{"id":"b1","name":"Reef","kind":"zone","points":[[174,-36],[174.1,-36],[174.1,-35.9]],"active":true,"notes":"","modified":1}]"#,
        )
        .unwrap();
        std::fs::write(d.join("routes").join("r1.json"), r#"{"id":"r1","name":"Round","notes":"","start":[174.8,-36.8],"start_mark":null,"marks":[{"mark":"m1","rounding":"port","radius_nm":0.1},{"mark":"m2","rounding":"","radius_nm":0}],"modified":7}"#).unwrap();
        std::fs::write(d.join("routings").join("x1.json"), r#"{"id":"x1","name":"Run","created":9,"route_id":"r1","route_name":"Round","request":{"t0":100,"polar_path":"C:\\p\\boat.pol"},"result":{"status":"ok","distance_nm":12.5}}"#).unwrap();
        std::fs::write(d.join("tracks").join("t1.json"), r#"{"id":"t1","name":"Trip","notes":"","colour":"","source":"log","modified":3,
            "stats":{"points":2,"start":1000,"end":1010,"distance_nm":0.1,"max_sog":6.0,"bbox":[174,-36.1,174.1,-36]},"segments":[[[1000,-36.1,174.0,6.0,90.0],[1010,-36.0,174.1,6.0,90.0,12.5,16.25]]]}"#).unwrap();
        std::fs::write(d.join("tracks").join("broken.json"), "{not json").unwrap();
        let lib = Library::open(&d).unwrap();
        assert!(lib.needs_conversion());
        let mut steps = Vec::new();
        let s = lib.convert_json(|st, a, b| steps.push((st.to_string(), a, b))).unwrap();
        assert!(s.contains("2 marks") && s.contains("1 tracks") && s.contains("1 damaged"), "{s}");
        assert!(!lib.needs_conversion());
        assert!(steps.iter().any(|x| x.0 == "tracks"));
        let m = lib.marks().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!((m[1].colour.as_str(), m[1].symbol.as_str()), ("#ff0000", "tz:14"));
        let r = lib.routes().unwrap();
        assert_eq!(r[0].marks.iter().map(|x| x.mark.as_str()).collect::<Vec<_>>(), vec!["m1", "m2"]);
        assert_eq!(r[0].marks[0].rounding, "port");
        assert_eq!(r[0].start, Some([174.8, -36.8]));
        let x = lib.routings().unwrap();
        assert_eq!((x[0].t0, x[0].distance_nm, x[0].polar.as_deref()), (Some(100), Some(12.5), Some("boat")));
        assert_eq!(lib.routing("x1").unwrap().request["t0"], 100);
        assert_eq!(lib.barriers().unwrap()[0].points.len(), 3);
        let t = lib.track("t1").unwrap();
        assert_eq!(t.segments[0][1].depth, Some(12.5));
        assert_eq!(t.segments[0][1].temp, Some(16.25));
        assert_eq!(lib.tracks().unwrap()[0].stats.points, 2);
        // the JSON files are kept, out of the way
        let backup = std::fs::read_dir(&d).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("json-backup-")).unwrap().path();
        assert!(backup.join("marks.json").exists() && backup.join("tracks").join("t1.json").exists());
        assert!(!d.join("marks.json").exists());
        // opening again finds nothing to move
        drop(lib);
        assert!(!Library::open(&d).unwrap().needs_conversion());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn deleted_things_go_to_the_trash_and_stay_out_of_lists() {
        let lib = temp_lib("trash");
        let a = lib.save_mark(Mark { name: "A".into(), lat: -36.0, lon: 174.0, ..Default::default() }).unwrap();
        let b = lib.save_mark(Mark { name: "B".into(), lat: -36.1, lon: 174.1, ..Default::default() }).unwrap();
        let c = lib.save_mark(Mark { name: "C".into(), lat: -36.2, lon: 174.2, ..Default::default() }).unwrap();
        let r = lib
            .save_route(Route {
                name: "R".into(),
                start_mark: Some(a.id.clone()),
                marks: [&a, &b, &c].iter().map(|m| RouteMark { mark: m.id.clone(), ..Default::default() }).collect(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(lib.delete_marks(&[a.id.clone(), b.id.clone()], true).unwrap(), 2);
        assert_eq!(lib.marks().unwrap().len(), 1);
        let r2 = lib.route(&r.id).unwrap();
        assert_eq!(r2.marks.iter().map(|x| x.mark.clone()).collect::<Vec<_>>(), vec![c.id.clone()]);
        assert_eq!(r2.start_mark, None);
        // still in the database, marked deleted
        let n: i64 = lib.db().query_row("SELECT count(*) FROM marks WHERE deleted_at IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        // a new mark can reuse a deleted one's name
        assert_eq!(lib.save_mark(Mark { lat: -36.3, lon: 174.3, ..Default::default() }).unwrap().name, "WP 1");
        lib.delete_route(&r.id).unwrap();
        assert!(lib.routes().unwrap().is_empty());
        assert!(matches!(lib.route(&r.id), Err(Error::NotFound(_))));
        assert!(matches!(lib.delete_route(&r.id), Err(Error::NotFound(_))));
    }

    #[test]
    fn marks_brought_in_by_the_hundred_have_an_unknown_date() {
        let lib = temp_lib("bulk");
        let mut g = String::from(r#"<?xml version="1.0"?><gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1">"#);
        for k in 0..30 {
            g.push_str(&format!(r#"<wpt lat="-41.{k:03}" lon="173.2"><time>2016-12-02T02:18:07Z</time></wpt>"#));
        }
        g.push_str(r#"<wpt lat="-41.5" lon="173.5"><name>Own</name><time>2017-03-01T05:00:00Z</time></wpt></gpx>"#);
        let r = lib.import_gpx(&g, "SeaPlot").unwrap();
        assert_eq!(r.marks_added, 31);
        let m = lib.marks().unwrap();
        assert_eq!(m.iter().filter(|m| m.date_unknown).count(), 30);
        assert!(!m.iter().find(|m| m.name == "Own").unwrap().date_unknown);
        let imports: i64 = lib.db().query_row("SELECT count(*) FROM marks WHERE import_id IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(imports, 31);
    }

    #[test]
    fn barriers_round_trip() {
        let dir = std::env::temp_dir().join(format!("racenav-bar-{}", std::process::id()));
        let lib = Library::open(&dir).unwrap();
        let b = lib.save_barrier(Barrier { kind: "zone".into(), points: vec![[174.0, -36.0], [174.1, -36.0], [174.1, -35.9]], ..Default::default() }).unwrap();
        assert_eq!(b.name, "Barrier 1");
        assert!(b.active);
        let mut b2 = b.clone();
        b2.active = false;
        lib.save_barrier(b2).unwrap();
        let all = lib.barriers().unwrap();
        assert_eq!(all.len(), 1);
        assert!(!all[0].active);
        lib.delete_barrier(&b.id).unwrap();
        assert!(lib.barriers().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn marks_routes_and_delete_rules() {
        let lib = temp_lib("a");
        let a = lib.save_mark(Mark { name: "Poor Knights".into(), lat: -35.465, lon: 174.80, ..Default::default() }).unwrap();
        let b = lib.save_mark(Mark { lat: -35.252, lon: 174.10, ..Default::default() }).unwrap();
        assert_eq!(b.name, "WP 1");
        let r = lib
            .save_route(Route {
                name: "Coastal Classic".into(),
                marks: vec![RouteMark { mark: a.id.clone(), ..Default::default() }, RouteMark { mark: b.id.clone(), ..Default::default() }],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(lib.routes().unwrap().len(), 1);
        // a route that starts at a library mark
        let s = lib.save_mark(Mark { name: "Bean Rock".into(), lat: -36.83, lon: 174.83, ..Default::default() }).unwrap();
        lib.save_route(Route {
            name: "From Bean Rock".into(),
            start: Some([174.83, -36.83]),
            start_mark: Some(s.id.clone()),
            marks: vec![RouteMark { mark: b.id.clone(), ..Default::default() }],
            ..Default::default()
        })
        .unwrap();
        assert!(lib.export_gpx(&[], &[]).unwrap().contains("<rtept lat=\"-36.830000\" lon=\"174.830000\">\n      <name>Bean Rock</name>"));
        assert!(matches!(lib.delete_mark(&s.id, false), Err(Error::InUse(_))));
        lib.delete_mark(&s.id, true).unwrap();
        let fr = lib.routes().unwrap().into_iter().find(|x| x.name == "From Bean Rock").unwrap();
        assert_eq!((fr.start_mark, fr.start), (None, Some([174.83, -36.83])));
        lib.delete_route(&fr.id).unwrap();
        // in use: refused, then forced
        assert!(matches!(lib.delete_mark(&a.id, false), Err(Error::InUse(_))));
        lib.delete_mark(&a.id, true).unwrap();
        assert_eq!(lib.route(&r.id).unwrap().marks.len(), 1);
        assert_eq!(lib.marks().unwrap().len(), 1);
        // rename keeps the id
        let mut b2 = b.clone();
        b2.name = "Russell".into();
        lib.save_mark(b2).unwrap();
        assert_eq!(lib.marks().unwrap()[0].name, "Russell");
        lib.delete_route(&r.id).unwrap();
        assert!(lib.routes().unwrap().is_empty());
    }

    #[test]
    fn routings_round_trip() {
        let lib = temp_lib("b");
        let s = lib
            .save_routing(Routing {
                route_name: "GGR leg".into(),
                request: serde_json::json!({"t0": 1791806400, "grib_name": "UKMO", "polar_path": "C:\\x\\Morgane_median.csv"}),
                result: serde_json::json!({"status": "grib end", "distance_nm": 925.6}),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s.name, "GGR leg");
        assert_eq!(s.polar.as_deref(), Some("Morgane_median"));
        assert_eq!(s.status.as_deref(), Some("grib end"));
        let full = lib.routing(&s.id).unwrap();
        assert_eq!(full.result["distance_nm"], 925.6);
        lib.delete_routing(&s.id).unwrap();
        assert!(lib.routings().unwrap().is_empty());
    }

    #[test]
    fn tracks_import_list_edit_delete() {
        let lib = temp_lib("t");
        let g = r#"<gpx version="1.1"><trk><name>Race</name><trkseg>
            <trkpt lat="-41.0" lon="173.0"><time>2026-10-17T00:00:00Z</time></trkpt>
            <trkpt lat="-41.0" lon="173.02"><time>2026-10-17T00:10:00Z</time></trkpt></trkseg></trk>
            <trk><trkseg><trkpt lat="-40.0" lon="173.0"/></trkseg></trk></gpx>"#;
        let rep = lib.import_gpx(g, "Morgane").unwrap();
        assert_eq!(rep.tracks_added, vec!["Race".to_string(), "Morgane 2".to_string()]);
        let list = lib.tracks().unwrap();
        assert_eq!(list.len(), 2);
        let race = list.iter().find(|t| t.name == "Race").unwrap();
        assert_eq!(race.stats.points, 2);
        assert!((race.stats.distance_nm - 0.91).abs() < 0.02, "{}", race.stats.distance_nm);
        assert!((race.stats.max_sog.unwrap() - 5.4).abs() < 0.2);
        assert_eq!(race.source, "gpx");
        let s = lib.update_track_info(&race.id, "Race day 1", "light airs", "#ff0000").unwrap();
        assert_eq!((s.name.as_str(), s.colour.as_str()), ("Race day 1", "#ff0000"));
        let full = lib.track(&race.id).unwrap();
        assert!(full.to_gpx().contains("<name>Race day 1</name>"));
        lib.delete_track(&race.id).unwrap();
        assert_eq!(lib.tracks().unwrap().len(), 1);
    }

    #[test]
    fn gpx_export_import_round_trip() {
        let lib = temp_lib("c");
        let a = lib.save_mark(Mark { name: "Bean Rock".into(), lat: -36.8317, lon: 174.8303, ..Default::default() }).unwrap();
        let b = lib.save_mark(Mark { name: "Tiri & Channel <N>".into(), lat: -36.60, lon: 174.90, ..Default::default() }).unwrap();
        lib.save_route(Route {
            name: "Round the Bay".into(),
            marks: vec![
                RouteMark { mark: a.id.clone(), ..Default::default() },
                RouteMark { mark: b.id, ..Default::default() },
                RouteMark { mark: a.id, ..Default::default() },
            ],
            ..Default::default()
        })
        .unwrap();
        let gpx = lib.export_gpx(&[], &[]).unwrap();
        assert!(gpx.contains("Tiri &amp; Channel &lt;N&gt;"));

        let other = temp_lib("d");
        let rep = other.import_gpx(&gpx, "RNZYS").unwrap();
        assert_eq!(rep.marks_added, 2);
        assert_eq!(rep.marks_matched, 3); // the route's three points reuse the two waypoints
        assert_eq!(rep.routes_added, vec!["Round the Bay".to_string()]);
        let r = &other.routes().unwrap()[0];
        assert_eq!(r.marks.len(), 3);
        assert_eq!(r.marks[0].mark, r.marks[2].mark);
        // importing again adds nothing new but the route
        let rep2 = other.import_gpx(&gpx, "").unwrap();
        assert_eq!(rep2.marks_added, 0);
    }
}

#[cfg(test)]
mod big {
    /// Timing on a big library (tools/testdata/make_library.py):
    /// `RACENAV_TEST_BIGLIB=<dir> cargo test --release -p library big_library -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn big_library() {
        let dir = std::env::var("RACENAV_TEST_BIGLIB").unwrap();
        let lib = super::Library::open(&dir).unwrap();
        let t = std::time::Instant::now();
        let m = lib.marks().unwrap();
        println!("marks(): {} in {:?}", m.len(), t.elapsed());
        let t = std::time::Instant::now();
        let mut one = m[0].clone();
        one.name.push('!');
        lib.save_mark(one).unwrap();
        println!("save_mark (rewrites marks.json): {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let tr = lib.tracks().unwrap();
        println!("tracks() list: {} in {:?}", tr.len(), t.elapsed());
        let t = std::time::Instant::now();
        let one = lib.track(&tr[0].id).unwrap();
        println!("track(): {} points in {:?}", one.stats.points, t.elapsed());
    }
}

#[cfg(test)]
mod big_gpx {
    /// `RACENAV_TEST_GPX=<file.gpx> cargo test --release -p library big_gpx_import -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn big_gpx_import() {
        let f = std::env::var("RACENAV_TEST_GPX").unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        let dir = std::env::temp_dir().join(format!("tp_biggpx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = super::Library::open(&dir).unwrap();
        let t = std::time::Instant::now();
        let d = super::parse_gpx(&text).unwrap();
        println!("parse: {} waypoints in {:?}", d.waypoints.len(), t.elapsed());
        let t = std::time::Instant::now();
        let r = lib.import_gpx(&text, "TimeZero").unwrap();
        println!("import: added {} matched {} in {:?}", r.marks_added, r.marks_matched, t.elapsed());
        let t = std::time::Instant::now();
        let r = lib.import_gpx(&text, "TimeZero").unwrap();
        println!("import again: added {} matched {} in {:?}", r.marks_added, r.marks_matched, t.elapsed());
        let t = std::time::Instant::now();
        let out = lib.export_gpx(&[], &[]).unwrap();
        println!("export: {} bytes in {:?}", out.len(), t.elapsed());
        let back = super::parse_gpx(&out).unwrap();
        let (a, b) = (&d.waypoints[0], &back.waypoints[0]);
        println!("round trip: {:?} {:?} {} / {:?} {:?} {}", a.time, a.tz_symbol, a.colour, b.time, b.tz_symbol, b.colour);
        assert_eq!((a.time, a.tz_symbol, &a.colour), (b.time, b.tz_symbol, &b.colour));
        if std::env::var("RACENAV_KEEP").is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            println!("kept {}", dir.display());
        }
    }
}

#[cfg(test)]
mod big_tzd {
    /// RACENAV_TEST_TZD=<a .tzd, .TzBackup or WaypointRouteDatabase.xml> cargo test --release -p library big_tzd -- --ignored --nocapture
    #[test]
    #[ignore]
    fn big_tzd_import() {
        let f = std::env::var("RACENAV_TEST_TZD").unwrap();
        let dir = std::env::temp_dir().join(format!("tp_bigtzd_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = super::Library::open(&dir).unwrap();
        let t = std::time::Instant::now();
        let d = super::tzd::read_tzd(std::path::Path::new(&f)).unwrap();
        println!(
            "read: {} marks ({} labels), {} routes, {} lines, {} areas, {} circles in {:?}",
            d.gpx.waypoints.len(),
            d.labels,
            d.gpx.routes.len(),
            d.lines,
            d.areas,
            d.circles,
            t.elapsed()
        );
        let t = std::time::Instant::now();
        let r = lib.import_tzd(std::path::Path::new(&f), "TimeZero").unwrap();
        println!(
            "import: added {} matched {}, {} routes, {} tracks, {:?} {:?} in {:?}",
            r.marks_added,
            r.marks_matched,
            r.routes_added.len(),
            r.tracks_added.len(),
            r.not_imported,
            r.notes,
            t.elapsed()
        );
        let r = lib.import_tzd(std::path::Path::new(&f), "TimeZero").unwrap();
        println!("tracks: {} added, {:?}", r.tracks_added.len(), r.notes);
        let t = std::time::Instant::now();
        let n = lib.tracks().unwrap().len();
        println!("tracks list: {n} in {:?}", t.elapsed());
        let r = lib.import_tzd(std::path::Path::new(&f), "TimeZero").unwrap();
        println!(
            "again: added {} matched {}, routes {} matched {}, tracks {} {:?}",
            r.marks_added,
            r.marks_matched,
            r.routes_added.len(),
            r.routes_matched,
            r.tracks_added.len(),
            r.notes
        );
        let groups: std::collections::BTreeSet<String> = lib.marks().unwrap().into_iter().map(|m| m.group).collect();
        println!("{} groups", groups.len());
        if std::env::var("RACENAV_KEEP").is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            println!("kept {}", dir.display());
        }
    }
}

#[cfg(test)]
mod scale {
    /// RACENAV_TEST_TZDS="a.tzd;b.tzd" RACENAV_TEST_DIR=<folder> cargo test --release -p library scale -- --ignored --nocapture
    /// Imports the files into one library (kept), then times what start-up and the drawers do.
    #[test]
    #[ignore]
    fn scale_library() {
        let dir = std::path::PathBuf::from(std::env::var("RACENAV_TEST_DIR").unwrap());
        let lib = super::Library::open(&dir).unwrap();
        for f in std::env::var("RACENAV_TEST_TZDS").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
            let t = std::time::Instant::now();
            let r = lib.import_tzd(std::path::Path::new(f), std::path::Path::new(f).file_stem().unwrap().to_str().unwrap()).unwrap();
            println!("import {f}: {} marks, {} tracks in {:?}", r.marks_added, r.tracks_added.len(), t.elapsed());
        }
        drop(lib);
        let t = std::time::Instant::now();
        let lib = super::Library::open(&dir).unwrap();
        println!("open: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let n = lib.tracks().unwrap();
        println!("tracks list: {} in {:?}", n.len(), t.elapsed());
        let t = std::time::Instant::now();
        let m = lib.marks().unwrap();
        println!("marks: {} in {:?}", m.len(), t.elapsed());
        let t = std::time::Instant::now();
        let big = n.iter().max_by_key(|s| s.stats.points).unwrap();
        let tr = lib.track(&big.id).unwrap();
        println!("biggest track: {} fixes in {:?}", tr.segments.iter().map(Vec::len).sum::<usize>(), t.elapsed());
        let t = std::time::Instant::now();
        for s in n.iter().take(50) {
            lib.track(&s.id).unwrap();
        }
        println!("50 tracks opened in {:?}", t.elapsed());
        println!("database: {:.0} MB", std::fs::metadata(lib.path()).unwrap().len() as f64 / 1e6);
    }
}

#[cfg(test)]
mod convert_big {
    /// RACENAV_TEST_DIR=<a JSON library> cargo test --release -p library convert_big -- --ignored --nocapture
    #[test]
    #[ignore]
    fn convert_big_json() {
        let dir = std::path::PathBuf::from(std::env::var("RACENAV_TEST_DIR").unwrap());
        let t = std::time::Instant::now();
        let lib = super::Library::open(&dir).unwrap();
        let mut last = 0;
        let s = lib
            .convert_json(|st, a, b| {
                if a >= last + 500 || a == b {
                    last = a;
                    println!("  {st} {a}/{b} at {:?}", t.elapsed());
                }
            })
            .unwrap();
        println!("{s}\nin {:?}", t.elapsed());
        let t = std::time::Instant::now();
        println!("tracks list: {} in {:?}", lib.tracks().unwrap().len(), t.elapsed());
    }
}

#[cfg(test)]
mod startup_cost {
    /// RACENAV_TEST_DIR=<a library folder> cargo test --release -p library startup_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn startup_cost() {
        let dir = std::path::PathBuf::from(std::env::var("RACENAV_TEST_DIR").unwrap());
        let t = std::time::Instant::now();
        let lib = super::Library::open(&dir).unwrap();
        println!("open {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let m = lib.marks().unwrap();
        println!("marks {} {:?}", m.len(), t.elapsed());
        let t = std::time::Instant::now();
        let j = serde_json::to_string(&m).unwrap();
        println!("marks as JSON {:.1} MB {:?}", j.len() as f64 / 1e6, t.elapsed());
        let t = std::time::Instant::now();
        let tr = lib.tracks().unwrap();
        let j = serde_json::to_string(&tr).unwrap();
        println!("tracks {} + JSON {:.1} MB {:?}", tr.len(), j.len() as f64 / 1e6, t.elapsed());
        let t = std::time::Instant::now();
        let r = lib.routes().unwrap();
        println!("routes {} {:?}", r.len(), t.elapsed());
        let t = std::time::Instant::now();
        let x = lib.routings().unwrap();
        println!("routings {} {:?}", x.len(), t.elapsed());
        let t = std::time::Instant::now();
        let b = lib.backup_if_due().unwrap();
        println!("backup {:?} {:?}", b.is_some(), t.elapsed());
    }
}

#[cfg(test)]
mod reimport {
    /// RACENAV_TEST_DIR=<library> RACENAV_TEST_TZDS="a;b" cargo test --release -p library reimport -- --ignored --nocapture
    #[test]
    #[ignore]
    fn reimport_fills_in() {
        let dir = std::path::PathBuf::from(std::env::var("RACENAV_TEST_DIR").unwrap());
        let t = std::time::Instant::now();
        let lib = super::Library::open(&dir).unwrap();
        println!("open (with migration) {:?}", t.elapsed());
        for f in std::env::var("RACENAV_TEST_TZDS").unwrap().split(';') {
            let t = std::time::Instant::now();
            let r = lib.import_tzd(std::path::Path::new(f), "x").unwrap();
            println!("{f}: added {} matched {} tracks {} {:?} in {:?}", r.marks_added, r.marks_matched, r.tracks_added.len(), r.notes, t.elapsed());
        }
        let m = lib.marks().unwrap();
        let n = |f: &dyn Fn(&super::Mark) -> bool| m.iter().filter(|x| f(x)).count();
        println!(
            "marks {}: depth {} temp {} sog {} at-boat {} date-unknown {}",
            m.len(),
            n(&|x| x.depth.is_some()),
            n(&|x| x.temp.is_some()),
            n(&|x| x.sog.is_some()),
            n(&|x| x.tags.contains("at-boat")),
            n(&|x| x.date_unknown)
        );
        let mut tags = std::collections::BTreeMap::new();
        for x in &m {
            for t in x.tags.split(',').filter(|t| !t.is_empty()) {
                *tags.entry(t.to_string()).or_insert(0) += 1;
            }
        }
        println!("tags {tags:?}");
        let t = std::time::Instant::now();
        let v = lib.marks_in(172.5, -42.0, 174.5, -40.0).unwrap();
        println!("marks in a 2° box: {} in {:?}", v.len(), t.elapsed());
        let v = lib.marks_in(179.0, -45.0, -179.0, -43.0).unwrap();
        println!("marks across 180°: {}", v.len());
        if let Some(x) = m.iter().find(|x| x.tags.contains("at-boat") && x.temp.is_some()) {
            println!("e.g. {:?}", x);
        }
    }
}

#[cfg(test)]
mod cancel_check {
    #[test]
    #[ignore]
    fn cancel_part_way() {
        let f = std::env::var("RACENAV_TEST_TZD").unwrap();
        let dir = std::env::temp_dir().join(format!("tp_cancel_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = super::Library::open(&dir).unwrap();
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut steps = std::collections::BTreeSet::new();
        let r = lib
            .import_tzd_with(
                std::path::Path::new(&f),
                "t",
                &mut |s, d, _| {
                    steps.insert(s.to_string());
                    if s == "Tracks" && d >= 100 {
                        stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                },
                &stop,
            )
            .unwrap();
        println!("steps {steps:?}\ntracks {} not imported {:?}", r.tracks_added.len(), r.not_imported);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
