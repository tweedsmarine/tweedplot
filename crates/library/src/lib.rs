//! The mark and route library.
//!
//! Everything is plain JSON in one folder, so it can be backed up, copied to
//! another PC or fixed by hand:
//!
//! ```text
//! <dir>/marks.json             every mark, shared by all routes
//! <dir>/barriers.json          barrier lines and zones the router avoids
//! <dir>/routes/<id>.json       one route: an ordered list of mark ids
//! <dir>/routings/<id>.json     one saved routing: its inputs and the result
//! <dir>/tracks/<id>.json       one track (logged or imported), see [`tracks`]
//! <dir>/export/*.gpx           GPX written by the app
//! ```
//!
//! Files are written to a temporary name and renamed, so a crash never leaves
//! half a file behind.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod gpx;
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
    /// "#rrggbb" to draw it in (empty: by its kind).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub colour: String,
    /// The symbol it had where it came from, e.g. "tz:62" (TimeZero) or a GPX `<sym>`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub symbol: String,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl Default for Mark {
    fn default() -> Self {
        Mark { id: String::new(), name: String::new(), lat: 0.0, lon: 0.0, kind: "mark".into(), notes: String::new(), group: String::new(), barrier_nm: 0.0, modified: 0, created: 0, colour: String::new(), symbol: String::new() }
    }
}

/// A barrier the router avoids: a line it may not cross, or a zone it may not enter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Barrier {
    pub id: String,
    pub name: String,
    /// "line" or "zone".
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

/// What an import did.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ImportReport {
    pub marks_added: usize,
    pub marks_matched: usize,
    pub routes_added: Vec<String>,
    pub tracks_added: Vec<String>,
    /// What the file held that wasn't brought in (TimeZero lines, areas, circles, tracks).
    #[serde(default)]
    pub not_imported: Vec<String>,
    /// Anything else worth saying about the import.
    #[serde(default)]
    pub notes: Vec<String>,
}

pub struct Library {
    dir: PathBuf,
}

/// Two marks this close (NM) with the same name are the same mark.
const SAME_MARK_NM: f64 = 0.03;
const SAME_UNNAMED_NM: f64 = 0.002;

impl Library {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        for sub in ["routes", "routings", "tracks", "export"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        Ok(Library { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    // ---------- marks ----------

    pub fn marks(&self) -> Result<Vec<Mark>> {
        let p = self.dir.join("marks.json");
        if !p.exists() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_slice(&std::fs::read(p)?)?)
    }

    fn write_marks(&self, marks: &[Mark]) -> Result<()> {
        write_json(&self.dir.join("marks.json"), &marks)
    }

    /// Add or update a mark (matched by id; an empty id makes a new one).
    pub fn save_mark(&self, mut m: Mark) -> Result<Mark> {
        let mut all = self.marks()?;
        m.modified = now();
        if m.name.trim().is_empty() {
            m.name = next_name(&all, "WP");
        }
        match all.iter_mut().find(|x| !m.id.is_empty() && x.id == m.id) {
            Some(x) => *x = m.clone(),
            None => {
                if m.id.is_empty() {
                    m.id = new_id("m");
                }
                all.push(m.clone());
            }
        }
        self.write_marks(&all)?;
        Ok(m)
    }

    /// Delete a mark. If routes use it, refuse unless `force`, which also
    /// takes it out of those routes.
    pub fn delete_mark(&self, id: &str, force: bool) -> Result<()> {
        let uses = |r: &Route| r.marks.iter().any(|x| x.mark == id) || r.start_mark.as_deref() == Some(id);
        let users: Vec<Route> = self.routes()?.into_iter().filter(|r| uses(r)).collect();
        if !users.is_empty() && !force {
            let names: Vec<&str> = users.iter().map(|r| r.name.as_str()).collect();
            return Err(Error::InUse(format!("The mark is used by: {}", names.join(", "))));
        }
        for mut r in users {
            r.marks.retain(|x| x.mark != id);
            if r.start_mark.as_deref() == Some(id) {
                r.start_mark = None; // the start keeps its position
            }
            self.save_route(r)?;
        }
        let mut all = self.marks()?;
        let n = all.len();
        all.retain(|m| m.id != id);
        if all.len() == n {
            return Err(Error::NotFound(id.into()));
        }
        self.write_marks(&all)
    }

    /// Delete many marks at once (in one write, not one per mark). With `force`, marks used by
    /// routes are taken out of them; without it, nothing is deleted if any is in use.
    pub fn delete_marks(&self, ids: &[String], force: bool) -> Result<usize> {
        let gone: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        let uses = |r: &Route| r.marks.iter().any(|x| gone.contains(x.mark.as_str())) || r.start_mark.as_deref().is_some_and(|m| gone.contains(m));
        let users: Vec<Route> = self.routes()?.into_iter().filter(|r| uses(r)).collect();
        if !users.is_empty() && !force {
            let names: Vec<&str> = users.iter().map(|r| r.name.as_str()).collect();
            return Err(Error::InUse(format!("Some of the marks are used by: {}", names.join(", "))));
        }
        for mut r in users {
            r.marks.retain(|x| !gone.contains(x.mark.as_str()));
            if r.start_mark.as_deref().is_some_and(|m| gone.contains(m)) {
                r.start_mark = None;
            }
            self.save_route(r)?;
        }
        let mut all = self.marks()?;
        let n = all.len();
        all.retain(|m| !gone.contains(m.id.as_str()));
        self.write_marks(&all)?;
        Ok(n - all.len())
    }

    // ---------- barriers ----------

    pub fn barriers(&self) -> Result<Vec<Barrier>> {
        let p = self.dir.join("barriers.json");
        if !p.exists() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_slice(&std::fs::read(p)?)?)
    }

    /// Add or update a barrier (an empty id makes a new one, named "Barrier n" when unnamed).
    pub fn save_barrier(&self, mut b: Barrier) -> Result<Barrier> {
        let mut all = self.barriers()?;
        b.modified = now();
        if b.name.trim().is_empty() {
            b.name = (1..).map(|k| format!("Barrier {k}")).find(|n| !all.iter().any(|x| &x.name == n)).unwrap();
        }
        match all.iter_mut().find(|x| !b.id.is_empty() && x.id == b.id) {
            Some(x) => *x = b.clone(),
            None => {
                if b.id.is_empty() {
                    b.id = new_id("b");
                }
                all.push(b.clone());
            }
        }
        write_json(&self.dir.join("barriers.json"), &all)?;
        Ok(b)
    }

    pub fn delete_barrier(&self, id: &str) -> Result<()> {
        let mut all = self.barriers()?;
        let n = all.len();
        all.retain(|b| b.id != id);
        if all.len() == n {
            return Err(Error::NotFound(id.into()));
        }
        write_json(&self.dir.join("barriers.json"), &all)
    }

    // ---------- routes ----------

    pub fn routes(&self) -> Result<Vec<Route>> {
        let mut v: Vec<Route> = read_dir_json(&self.dir.join("routes"))?;
        v.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(v)
    }

    pub fn route(&self, id: &str) -> Result<Route> {
        read_one(&self.dir.join("routes"), id)
    }

    pub fn save_route(&self, mut r: Route) -> Result<Route> {
        if r.id.is_empty() {
            r.id = new_id("r");
        }
        if r.name.trim().is_empty() {
            r.name = "Route".into();
        }
        r.modified = now();
        write_json(&self.dir.join("routes").join(format!("{}.json", safe(&r.id))), &r)?;
        Ok(r)
    }

    pub fn delete_route(&self, id: &str) -> Result<()> {
        remove(&self.dir.join("routes"), id)
    }

    // ---------- routings ----------

    pub fn routings(&self) -> Result<Vec<RoutingSummary>> {
        let all: Vec<Routing> = read_dir_json(&self.dir.join("routings"))?;
        let mut v: Vec<RoutingSummary> = all
            .into_iter()
            .map(|r| RoutingSummary {
                t0: r.request["t0"].as_i64(),
                status: r.result["status"].as_str().map(str::to_string),
                arrival: r.result["arrival"].as_i64(),
                distance_nm: r.result["distance_nm"].as_f64(),
                grib_name: r.request["grib_name"].as_str().map(str::to_string),
                polar: r.request["polar_path"].as_str().map(|p| {
                    Path::new(&p.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                }),
                id: r.id,
                name: r.name,
                created: r.created,
                route_name: r.route_name,
            })
            .collect();
        v.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(v)
    }

    pub fn routing(&self, id: &str) -> Result<Routing> {
        read_one(&self.dir.join("routings"), id)
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
        write_json(&self.dir.join("routings").join(format!("{}.json", safe(&r.id))), &r)?;
        let id = r.id.clone();
        self.routings()?.into_iter().find(|s| s.id == id).ok_or(Error::NotFound(id))
    }

    pub fn delete_routing(&self, id: &str) -> Result<()> {
        remove(&self.dir.join("routings"), id)
    }

    // ---------- GPX ----------

    /// GPX of the given marks (as waypoints) and routes; empty slices mean "all".
    pub fn export_gpx(&self, mark_ids: &[String], route_ids: &[String]) -> Result<String> {
        let marks = self.marks()?;
        let routes: Vec<Route> = self.routes()?.into_iter().filter(|r| route_ids.is_empty() || route_ids.contains(&r.id)).collect();
        let wpts: Vec<&Mark> = if mark_ids.is_empty() && route_ids.is_empty() {
            marks.iter().collect()
        } else {
            // the chosen marks plus every mark the chosen routes use
            marks
                .iter()
                .filter(|m| {
                    mark_ids.contains(&m.id)
                        || routes.iter().any(|r| r.marks.iter().any(|x| x.mark == m.id) || r.start_mark.as_deref() == Some(m.id.as_str()))
                })
                .collect()
        };
        let by_id = |id: &str| marks.iter().find(|m| m.id == id);
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"racenav\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n");
        for m in &wpts {
            out.push_str(&gpx::wpt("wpt", m, "  "));
        }
        for r in &routes {
            out.push_str(&format!("  <rte>\n    <name>{}</name>\n", gpx::esc(&r.name)));
            // the start leads the route: its library mark, else a plain point
            if let Some(m) = r.start_mark.as_deref().and_then(by_id) {
                out.push_str(&gpx::wpt("rtept", m, "    "));
            } else if let Some([lon, lat]) = r.start {
                let m = Mark { name: "Start".into(), lat, lon, kind: "start".into(), ..Default::default() };
                out.push_str(&gpx::wpt("rtept", &m, "    "));
            }
            for rm in &r.marks {
                if let Some(m) = by_id(&rm.mark) {
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
        self.import_data(parse_gpx(text)?, group)
    }

    /// Bring in a TimeZero `.tzd` / `.TzBackup` (or its WaypointRouteDatabase.xml): marks
    /// in their layers as groups, with symbol, colour and date, and routes. Matching is as GPX.
    pub fn import_tzd(&self, path: &std::path::Path, group: &str) -> Result<ImportReport> {
        let d = tzd::read_tzd(path)?;
        let mut rep = self.import_data(d.gpx, group)?;
        let n = |k: usize, one: &str, many: &str| if k == 1 { format!("1 {one}") } else { format!("{k} {many}") };
        if d.lines + d.areas + d.circles > 0 {
            rep.not_imported.push(format!("{}, {} and {} (not yet)", n(d.lines, "line", "lines"), n(d.areas, "area", "areas"), n(d.circles, "circle", "circles")));
        }
        self.import_tz_tracks(path, group, &mut rep)?;
        Ok(rep)
    }

    /// TimeZero's tracks (Tracks.bin), thinned as the track recorder thins live fixes
    /// (TimeZero keeps one every 2 s). A track already here (same start and name) is skipped.
    fn import_tz_tracks(&self, path: &std::path::Path, group: &str, rep: &mut ImportReport) -> Result<()> {
        let have: std::collections::HashSet<(i64, String)> =
            self.tracks()?.into_iter().filter_map(|t| t.stats.start.map(|s| (s, t.name))).collect();
        let (mut fixes, mut kept, mut same) = (0usize, 0usize, 0usize);
        let r = tzd::read_tzd_tracks(path, |t| {
            fixes += t.points.len();
            let mut rec = Recorder::new(Track { source: "timezero".into(), colour: t.colour.clone(), ..Default::default() });
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
            // the earliest fix, as the track's totals have it (a track can go back in time)
            let start = tr.segments.iter().flatten().filter_map(|p| p.t).min().unwrap_or(0);
            let when = tracks::iso_time(start).get(..16).unwrap_or("").replace('T', " ");
            let who = if t.mobile.eq_ignore_ascii_case("OwnShip") || t.name.is_empty() { group.to_string() } else { t.name.trim().to_string() };
            tr.name = format!("{who} {when}").trim().to_string();
            if have.contains(&(start, tr.name.clone())) {
                same += 1;
                return Ok(());
            }
            kept += tr.segments.iter().map(Vec::len).sum::<usize>();
            rep.tracks_added.push(self.save_track(tr)?.name);
            Ok(())
        })?;
        if let Some(p) = r.as_ref().and_then(|r| r.problem.clone()) {
            let r = r.as_ref().unwrap();
            rep.not_imported.push(format!("tracks after {} of {} (TimeZero's Tracks.bin: {p})", r.read, r.count));
        }
        if let Some(r) = r.as_ref().filter(|r| !r.other.is_empty()) {
            let o: Vec<String> = r.other.iter().map(|(k, n)| format!("{n} {}", k.to_lowercase())).collect();
            rep.not_imported.push(format!("from the track file: {} (not yet)", o.join(", ")));
        }
        if r.is_some() && fixes > 0 {
            rep.notes.push(format!("tracks: kept {kept} of {fixes} fixes (a fix every 10 s, sooner in turns){}", if same > 0 { format!("; {same} already here") } else { String::new() }));
        }
        Ok(())
    }

    fn import_data(&self, data: GpxData, group: &str) -> Result<ImportReport> {
        let mut rep = ImportReport::default();
        let mut marks = self.marks()?;
        // Marks already here, by name and a grid of about a kilometre, so a big file (a
        // fisherman's tens of thousands of marks) matches in moments, not hours.
        const CELL: f64 = 0.01;
        let cell = |lat: f64, lon: f64| ((lat / CELL).floor() as i64, (lon / CELL).floor() as i64);
        let mut index: std::collections::HashMap<(String, (i64, i64)), Vec<usize>> = std::collections::HashMap::new();
        for (i, m) in marks.iter().enumerate() {
            index.entry((m.name.to_lowercase(), cell(m.lat, m.lon))).or_default().push(i);
        }
        let stamp = now();
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
            let m = Mark {
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
                colour: w.colour.clone(),
                symbol,
            };
            rep.marks_added += 1;
            let id = m.id.clone();
            index.entry((key, (cy, cx))).or_default().push(marks.len());
            marks.push(m);
            id
        };
        for w in &data.waypoints {
            find_or_add(w, &mut rep);
        }
        let mut new_routes = Vec::new();
        for (k, r) in data.routes.iter().enumerate() {
            let ids: Vec<RouteMark> = r.points.iter().map(|w| RouteMark { mark: find_or_add(w, &mut rep), ..Default::default() }).collect();
            let name = if r.name.trim().is_empty() { format!("Imported route {}", k + 1) } else { r.name.trim().to_string() };
            new_routes.push(Route { id: String::new(), name, notes: String::new(), start: None, start_mark: None, marks: ids, modified: 0 });
        }
        self.write_marks(&marks)?;
        for r in new_routes {
            rep.routes_added.push(self.save_route(r)?.name);
        }
        for (k, t) in data.tracks.into_iter().enumerate() {
            let name = if t.name.trim().is_empty() {
                if group.is_empty() { format!("Imported track {}", k + 1) } else { format!("{group} {}", k + 1) }
            } else {
                t.name.trim().to_string()
            };
            let tr = Track { name, notes: t.desc, source: "gpx".into(), segments: t.segments, ..Default::default() };
            rep.tracks_added.push(self.save_track(tr)?.name);
        }
        Ok(rep)
    }

    // ---------- tracks ----------

    /// Every track, without points, newest first.
    pub fn tracks(&self) -> Result<Vec<TrackSummary>> {
        let mut v: Vec<TrackSummary> = read_dir_json::<Track>(&self.dir.join("tracks"))?.iter().map(Track::summary).collect();
        v.sort_by_key(|t| std::cmp::Reverse(t.stats.start.unwrap_or(t.modified)));
        Ok(v)
    }

    pub fn track(&self, id: &str) -> Result<Track> {
        read_one(&self.dir.join("tracks"), id)
    }

    /// Save a track (new id if it has none); its totals are worked out again.
    pub fn save_track(&self, mut t: Track) -> Result<TrackSummary> {
        if t.id.is_empty() {
            t.id = new_id("t");
        }
        t.modified = now();
        t.update_stats();
        write_json(&self.dir.join("tracks").join(format!("{}.json", safe(&t.id))), &t)?;
        Ok(t.summary())
    }

    /// A fresh track id (for a track that will be saved later).
    pub fn new_track_id(&self) -> String {
        new_id("t")
    }

    /// Save a track as it is (totals kept), e.g. a track being recorded.
    pub fn store_track(&self, t: &Track) -> Result<()> {
        write_json(&self.dir.join("tracks").join(format!("{}.json", safe(&t.id))), t)
    }

    /// Change a track's name, notes and colour.
    pub fn update_track_info(&self, id: &str, name: &str, notes: &str, colour: &str) -> Result<TrackSummary> {
        let mut t = self.track(id)?;
        t.name = name.trim().to_string();
        t.notes = notes.to_string();
        t.colour = colour.to_string();
        t.modified = now();
        self.store_track(&t)?;
        Ok(t.summary())
    }

    pub fn delete_track(&self, id: &str) -> Result<()> {
        let p = self.dir.join("tracks").join(format!("{}.json", safe(id)));
        if !p.exists() {
            return Err(Error::NotFound(id.to_string()));
        }
        std::fs::remove_file(p)?;
        Ok(())
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
}

// ---------- helpers ----------

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
fn next_name(marks: &[Mark], stem: &str) -> String {
    (1..).map(|k| format!("{stem} {k}")).find(|n| !marks.iter().any(|m| &m.name == n)).unwrap()
}

/// File-name safe version of an id or name.
fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect::<String>().trim().to_string()
}

fn write_json<T: Serialize + ?Sized>(p: &Path, v: &T) -> Result<()> {
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

fn read_dir_json<T: for<'de> Deserialize<'de>>(dir: &Path) -> Result<Vec<T>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        // a damaged file is skipped rather than hiding the rest
        if let Ok(x) = std::fs::read(&p).map_err(Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
            v.push(x);
        }
    }
    Ok(v)
}

fn read_one<T: for<'de> Deserialize<'de>>(dir: &Path, id: &str) -> Result<T> {
    let p = dir.join(format!("{}.json", safe(id)));
    if !p.exists() {
        return Err(Error::NotFound(id.into()));
    }
    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
}

fn remove(dir: &Path, id: &str) -> Result<()> {
    let p = dir.join(format!("{}.json", safe(id)));
    if !p.exists() {
        return Err(Error::NotFound(id.into()));
    }
    Ok(std::fs::remove_file(p)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_lib(tag: &str) -> Library {
        let d = std::env::temp_dir().join(format!("racenav-lib-{tag}-{}", new_id("")));
        Library::open(d).unwrap()
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
            .save_route(Route { name: "Coastal Classic".into(), marks: vec![RouteMark { mark: a.id.clone(), ..Default::default() }, RouteMark { mark: b.id.clone(), ..Default::default() }], ..Default::default() })
            .unwrap();
        assert_eq!(lib.routes().unwrap().len(), 1);
        // a route that starts at a library mark
        let s = lib.save_mark(Mark { name: "Bean Rock".into(), lat: -36.83, lon: 174.83, ..Default::default() }).unwrap();
        lib.save_route(Route { name: "From Bean Rock".into(), start: Some([174.83, -36.83]), start_mark: Some(s.id.clone()), marks: vec![RouteMark { mark: b.id.clone(), ..Default::default() }], ..Default::default() }).unwrap();
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
        lib.save_route(Route { name: "Round the Bay".into(), marks: vec![RouteMark { mark: a.id.clone(), ..Default::default() }, RouteMark { mark: b.id, ..Default::default() }, RouteMark { mark: a.id, ..Default::default() }], ..Default::default() }).unwrap();
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
        if std::env::var("RACENAV_KEEP").is_err() { let _ = std::fs::remove_dir_all(&dir); } else { println!("kept {}", dir.display()); }
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
        println!("read: {} marks ({} labels), {} routes, {} lines, {} areas, {} circles in {:?}",
            d.gpx.waypoints.len(), d.labels, d.gpx.routes.len(), d.lines, d.areas, d.circles, t.elapsed());
        let t = std::time::Instant::now();
        let r = lib.import_tzd(std::path::Path::new(&f), "TimeZero").unwrap();
        println!("import: added {} matched {}, {} routes, {} tracks, {:?} {:?} in {:?}", r.marks_added, r.marks_matched, r.routes_added.len(), r.tracks_added.len(), r.not_imported, r.notes, t.elapsed());
        let r = lib.import_tzd(std::path::Path::new(&f), "TimeZero").unwrap();
        println!("tracks: {} added, {:?}", r.tracks_added.len(), r.notes);
        let t = std::time::Instant::now();
        let n = lib.tracks().unwrap().len();
        println!("tracks list: {n} in {:?}", t.elapsed());
        let r = lib.import_tzd(std::path::Path::new(&f), "TimeZero").unwrap();
        println!("again: added {} matched {}, tracks {} {:?}", r.marks_added, r.marks_matched, r.tracks_added.len(), r.notes);
        let groups: std::collections::BTreeSet<String> = lib.marks().unwrap().into_iter().map(|m| m.group).collect();
        println!("{} groups", groups.len());
        if std::env::var("RACENAV_KEEP").is_err() { let _ = std::fs::remove_dir_all(&dir); } else { println!("kept {}", dir.display()); }
    }
}
