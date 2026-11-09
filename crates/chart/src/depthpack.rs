//! Survey depth packages (`.tpdepth`): LINZ's published depth models (multibeam and
//! bathymetric lidar surveys, CC BY 4.0) gridded onto the chart's own tiles, for TweedPlot
//! to draw like the high-resolution depths. Made by `tweedplot-depthpack` from the LINZ Data
//! Service and handed out as files (and, later, downloads named in a manifest).
//!
//! ## The file
//! An SQLite database:
//! - `meta (key, value)`: `format` = "tweedplot-depth", `version`, `edition` (e.g. "2026-11"),
//!   `tier` ("national" or "region"), `region`, `region_name`, `min_zoom`, `max_zoom`,
//!   `datum`, `source`, `licence`, `attribution`, `created`.
//! - `tiles (z, x, y, data)`: Web Mercator tiles as the map's (x from the west, y from the
//!   north), [`N`] × [`N`] cells each; cell (i, j) of tile (z, x, y) covers map pixels
//!   x·N + i … +1 at zoom z. `data` is [`encode`]d heights.
//! - `surveys (id, title, year, res, epsg, datum, accuracy, published, outline)`: the LINZ
//!   layers that went in (id = LDS layer id), outline as a GeoJSON geometry in longitude and
//!   latitude.
//!
//! Heights are metres, positive up, to the surveys' datum (LINZ: approximate Lowest
//! Astronomical Tide, near chart datum): a depth of 30 m is -30. Each coarser zoom keeps
//! the **shallowest** of the four cells below it (shoal-biased, as LINZ grids its surveys),
//! so a small shoal never disappears as you zoom out.
//!
//! The national package carries every survey at the coarser zooms; a regional one the
//! finer zooms of one area. TweedPlot reads every package in its folder and uses the finest
//! data it has at each place.

use std::collections::HashMap;
use std::io::{Read, Write as _};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Cells along a tile's side.
pub const N: usize = 256;
/// The file's format name, in `meta`.
pub const FORMAT: &str = "tweedplot-depth";
/// The format version this code writes and reads.
pub const VERSION: u32 = 1;
/// The marker for "no data" in an encoded tile.
const NODATA: i32 = i32::MIN;

/// One tile of heights (row by row from the north-west), NaN where there is no data.
pub type Cells = Vec<f32>;

// ---------------------------------------------------------------- tile coding

/// Heights to the stored form: centimetres as i32 (no data: [`NODATA`]), each row as the
/// difference from the cell before it, the four bytes of every value in four planes, then
/// zlib. Depth grids are smooth, so this packs to a small part of the f32s.
pub fn encode(cells: &[f32]) -> Vec<u8> {
    debug_assert_eq!(cells.len(), N * N);
    let q: Vec<i32> = cells.iter().map(|&h| if h.is_finite() { (h as f64 * 100.0).round().clamp(-2e9, 2e9) as i32 } else { NODATA }).collect();
    let mut d = vec![0i32; q.len()];
    for r in 0..N {
        let mut prev = 0i32;
        for c in 0..N {
            let v = q[r * N + c];
            d[r * N + c] = v.wrapping_sub(prev);
            prev = v;
        }
    }
    let mut planes = vec![0u8; d.len() * 4];
    for (i, v) in d.iter().enumerate() {
        let b = v.to_le_bytes();
        for k in 0..4 {
            planes[k * d.len() + i] = b[k];
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&planes).expect("in memory");
    z.finish().expect("in memory")
}

/// The stored form back to heights.
pub fn decode(data: &[u8]) -> Option<Cells> {
    let mut planes = Vec::with_capacity(N * N * 4);
    flate2::read::ZlibDecoder::new(data).take((N * N * 4) as u64 + 1).read_to_end(&mut planes).ok()?;
    if planes.len() != N * N * 4 {
        return None;
    }
    let n = N * N;
    let mut out = vec![f32::NAN; n];
    for r in 0..N {
        let mut prev = 0i32;
        for c in 0..N {
            let i = r * N + c;
            let d = i32::from_le_bytes([planes[i], planes[n + i], planes[2 * n + i], planes[3 * n + i]]);
            let v = prev.wrapping_add(d);
            prev = v;
            out[i] = if v == NODATA { f32::NAN } else { v as f32 / 100.0 };
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- tile maths

/// Map pixel (at zoom z, N per tile) of a longitude and latitude.
pub fn to_pixel(lon: f64, lat: f64, z: u8) -> (f64, f64) {
    let n = (N as f64) * (1u64 << z) as f64;
    let x = (lon + 180.0) / 360.0 * n;
    let s = lat.to_radians().sin().clamp(-0.9999999, 0.9999999);
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * n;
    (x, y)
}

/// Longitude and latitude of a map pixel position at zoom z.
pub fn from_pixel(px: f64, py: f64, z: u8) -> (f64, f64) {
    let n = (N as f64) * (1u64 << z) as f64;
    let lon = px / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * py / n)).sinh().atan().to_degrees();
    (lon, lat)
}

/// Ground size (m) of a cell at zoom z and latitude.
pub fn cell_size(z: u8, lat: f64) -> f64 {
    40_075_016.686 * lat.to_radians().cos() / ((N as f64) * (1u64 << z) as f64)
}

/// The zoom to grid a survey of `res` metres at `lat`: the finest whose cells are at least
/// 0.6 × `res` (at most 18). Cells a little finer than the survey's own, never coarser, so
/// nothing of it is lost (coarser zooms are then made shoal-biased from it).
pub fn zoom_for(res: f64, lat: f64) -> u8 {
    let mut z = 0u8;
    while z < 18 && cell_size(z + 1, lat) >= res * 0.6 {
        z += 1;
    }
    z
}

// ---------------------------------------------------------------- surveys and metadata

/// A survey (LINZ depth model) in a package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Survey {
    /// LINZ Data Service layer id.
    pub id: i64,
    pub title: String,
    /// Year surveyed (from the title), 0 if unknown.
    pub year: i32,
    /// Grid spacing of the published model, metres.
    pub res: f64,
    pub epsg: u32,
    pub datum: String,
    /// What LINZ says of its accuracy, e.g. "vertical ±0.5 m, horizontal ±1 m (95%)".
    pub accuracy: String,
    /// When LINZ published this version (ISO date).
    pub published: String,
    /// The area it covers: a GeoJSON geometry in longitude and latitude.
    pub outline: serde_json::Value,
}

/// What a package says about itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PackInfo {
    pub file: String,
    pub edition: String,
    pub tier: String,
    pub region: String,
    pub region_name: String,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub datum: String,
    pub attribution: String,
    pub created: String,
    pub surveys: usize,
    pub tiles: i64,
    pub bytes: u64,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS tiles (z INTEGER NOT NULL, x INTEGER NOT NULL, y INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (z, x, y)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS surveys (id INTEGER PRIMARY KEY, title TEXT NOT NULL, year INTEGER NOT NULL, res REAL NOT NULL, epsg INTEGER NOT NULL,
  datum TEXT NOT NULL, accuracy TEXT NOT NULL, published TEXT NOT NULL, outline TEXT NOT NULL);
";

// ---------------------------------------------------------------- writing

/// Builds a package: tiles painted in, the coarser zooms made, metadata set.
pub struct Writer {
    pub db: Connection,
}

impl Writer {
    /// A new (or continued) package file.
    pub fn create(path: &Path) -> rusqlite::Result<Writer> {
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        db.execute_batch(SCHEMA)?;
        db.execute("INSERT OR REPLACE INTO meta VALUES ('format', ?1), ('version', ?2)", params![FORMAT, VERSION.to_string()])?;
        Ok(Writer { db })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.db.execute("INSERT OR REPLACE INTO meta VALUES (?1, ?2)", params![key, value]).map(|_| ())
    }

    pub fn add_survey(&self, s: &Survey) -> rusqlite::Result<()> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO surveys VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![s.id, s.title, s.year, s.res, s.epsg, s.datum, s.accuracy, s.published, s.outline.to_string()],
            )
            .map(|_| ())
    }

    pub fn tile(&self, z: u8, x: u32, y: u32) -> Option<Cells> {
        let b: Option<Vec<u8>> =
            self.db.query_row("SELECT data FROM tiles WHERE z = ?1 AND x = ?2 AND y = ?3", params![z, x, y], |r| r.get(0)).optional().ok()?;
        decode(&b?)
    }

    /// Put cells into a tile: where `cells` has data it replaces what is there (paint the
    /// surveys coarsest and oldest first, so the finest and newest end on top).
    pub fn paint(&self, z: u8, x: u32, y: u32, cells: &[f32]) -> rusqlite::Result<()> {
        if cells.iter().all(|h| h.is_nan()) {
            return Ok(());
        }
        let mut cur = self.tile(z, x, y).unwrap_or_else(|| vec![f32::NAN; N * N]);
        for (c, &v) in cur.iter_mut().zip(cells) {
            if !v.is_nan() {
                *c = v;
            }
        }
        self.put(z, x, y, &cur)
    }

    fn put(&self, z: u8, x: u32, y: u32, cells: &[f32]) -> rusqlite::Result<()> {
        self.db.execute("INSERT OR REPLACE INTO tiles VALUES (?1, ?2, ?3, ?4)", params![z, x, y, encode(cells)]).map(|_| ())
    }

    /// Make zooms `max_zoom - 1` down to `min_zoom` from the zoom above: each cell the
    /// shallowest of its four (no data only where all four have none). A coarse survey
    /// painted at a coarser zoom stays where the finer ones don't reach: finer data wins.
    pub fn build_pyramid(&self, min_zoom: u8, max_zoom: u8) -> rusqlite::Result<()> {
        for z in (min_zoom..max_zoom).rev() {
            let children: Vec<(u32, u32)> = {
                let mut st = self.db.prepare("SELECT x, y FROM tiles WHERE z = ?1")?;
                let rows = st.query_map([z + 1], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            let mut parents: Vec<(u32, u32)> = children.iter().map(|&(x, y)| (x / 2, y / 2)).collect();
            parents.sort_unstable();
            parents.dedup();
            let tx = self.db.unchecked_transaction()?;
            for (px, py) in parents {
                let mut out = vec![f32::NAN; N * N];
                for (dx, dy) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)] {
                    let Some(c) = self.tile(z + 1, px * 2 + dx, py * 2 + dy) else { continue };
                    let (ox, oy) = (dx as usize * N / 2, dy as usize * N / 2);
                    for j in 0..N / 2 {
                        for i in 0..N / 2 {
                            let v = [c[(2 * j) * N + 2 * i], c[(2 * j) * N + 2 * i + 1], c[(2 * j + 1) * N + 2 * i], c[(2 * j + 1) * N + 2 * i + 1]]
                                .into_iter()
                                .filter(|v| !v.is_nan())
                                .fold(f32::NAN, f32::max);
                            out[(oy + j) * N + ox + i] = v;
                        }
                    }
                }
                // painted here already (a coarser survey): it fills in where the finer
                // surveys below have nothing
                if let Some(old) = self.tile(z, px, py) {
                    for (o, v) in out.iter_mut().zip(old) {
                        if o.is_nan() {
                            *o = v;
                        }
                    }
                }
                self.put(z, px, py, &out)?;
            }
            tx.commit()?;
        }
        Ok(())
    }

    /// Finish: zoom range in the metadata, the file compacted.
    pub fn finish(self) -> rusqlite::Result<()> {
        let (lo, hi): (Option<i64>, Option<i64>) = self.db.query_row("SELECT min(z), max(z) FROM tiles", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        self.set_meta("min_zoom", &lo.unwrap_or(0).to_string())?;
        self.set_meta("max_zoom", &hi.unwrap_or(0).to_string())?;
        self.db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE; VACUUM;")?;
        Ok(())
    }
}

/// Copy part of a package into a new one: zooms `zooms`, tiles whose middle is inside
/// `area` ([w, s, e, n]; None for all), the surveys that reach `area`, and `meta`.
pub fn extract(from: &Path, to: &Path, zooms: std::ops::RangeInclusive<u8>, area: Option<[f64; 4]>, meta: &[(&str, &str)]) -> rusqlite::Result<usize> {
    let keep = |lon: f64, lat: f64| area.is_none_or(|a| lon >= a[0] && lon < a[2] && lat >= a[1] && lat < a[3]);
    extract_where(from, to, zooms, &keep, meta)
}

/// [`extract`] with any test of a tile's middle (longitude, latitude). The surveys kept are
/// those that reach the area of the tiles kept.
pub fn extract_where(
    from: &Path,
    to: &Path,
    zooms: std::ops::RangeInclusive<u8>,
    keep: &dyn Fn(f64, f64) -> bool,
    meta: &[(&str, &str)],
) -> rusqlite::Result<usize> {
    let _ = std::fs::remove_file(to);
    let w = Writer::create(to)?;
    w.db.execute("ATTACH DATABASE ?1 AS src", [from.to_string_lossy()])?;
    let tx = w.db.unchecked_transaction()?;
    let mut n = 0;
    let mut kept = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    {
        let mut st = w.db.prepare("SELECT z, x, y, data FROM src.tiles WHERE z BETWEEN ?1 AND ?2")?;
        let mut ins = w.db.prepare("INSERT OR REPLACE INTO tiles VALUES (?1, ?2, ?3, ?4)")?;
        let mut rows = st.query(params![zooms.start(), zooms.end()])?;
        while let Some(r) = rows.next()? {
            let (z, x, y): (u8, u32, u32) = (r.get(0)?, r.get(1)?, r.get(2)?);
            let (lon, lat) = from_pixel((x as f64 + 0.5) * N as f64, (y as f64 + 0.5) * N as f64, z);
            if !keep(lon, lat) {
                continue;
            }
            // the area the kept tiles cover
            let (w, nn) = from_pixel(x as f64 * N as f64, y as f64 * N as f64, z);
            let (e, ss) = from_pixel((x + 1) as f64 * N as f64, (y + 1) as f64 * N as f64, z);
            kept = [kept[0].min(w), kept[1].min(ss), kept[2].max(e), kept[3].max(nn)];
            let data: Vec<u8> = r.get(3)?;
            ins.execute(params![z, x, y, data])?;
            n += 1;
        }
    }
    // the surveys that reach the area
    {
        let mut st = w.db.prepare("SELECT id, title, year, res, epsg, datum, accuracy, published, outline FROM src.surveys")?;
        let rows: Vec<Survey> = st
            .query_map([], |r| {
                Ok(Survey {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    year: r.get(2)?,
                    res: r.get(3)?,
                    epsg: r.get(4)?,
                    datum: r.get(5)?,
                    accuracy: r.get(6)?,
                    published: r.get(7)?,
                    outline: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
                })
            })?
            .collect::<Result<_, _>>()?;
        for s in rows {
            let a = kept;
            if n > 0 && bbox_of(&s.outline).is_none_or(|b| b[0] < a[2] && b[2] > a[0] && b[1] < a[3] && b[3] > a[1]) {
                w.add_survey(&s)?;
            }
        }
    }
    tx.commit()?;
    w.db.execute("DETACH DATABASE src", [])?;
    for (k, v) in meta {
        w.set_meta(k, v)?;
    }
    w.finish()?;
    Ok(n)
}

/// [w, s, e, n] of a GeoJSON geometry's coordinates.
pub fn bbox_of(g: &serde_json::Value) -> Option<[f64; 4]> {
    fn walk(v: &serde_json::Value, b: &mut [f64; 4]) {
        if let Some(a) = v.as_array() {
            if a.len() >= 2 && a[0].is_number() && a[1].is_number() {
                let (x, y) = (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0));
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            } else {
                for c in a {
                    walk(c, b);
                }
            }
        }
    }
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    walk(g.get("coordinates")?, &mut b);
    (b[0] <= b[2]).then_some(b)
}

// ---------------------------------------------------------------- reading

/// What one package file says about itself.
pub fn read_info(path: &Path) -> Result<PackInfo, String> {
    Pack::open(path).map(|p| p.info)
}

/// The LINZ layer ids of the surveys in one package file.
pub fn survey_ids(path: &Path) -> rusqlite::Result<Vec<i64>> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut st = db.prepare("SELECT id FROM surveys ORDER BY id")?;
    let v = st.query_map([], |r| r.get(0))?.collect();
    v
}

struct Pack {
    db: Mutex<Connection>,
    info: PackInfo,
}

impl Pack {
    fn open(path: &Path) -> Result<Pack, String> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| e.to_string())?;
        let meta: HashMap<String, String> = {
            let mut st = db.prepare("SELECT key, value FROM meta").map_err(|e| format!("not a depth package ({e})"))?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
            rows.flatten().collect()
        };
        if meta.get("format").map(String::as_str) != Some(FORMAT) {
            return Err("not a TweedPlot depth package".into());
        }
        let version: u32 = meta.get("version").and_then(|v| v.parse().ok()).unwrap_or(0);
        if version > VERSION {
            return Err(format!("made by a newer TweedPlot (format {version}); update TweedPlot to use it"));
        }
        let get = |k: &str| meta.get(k).cloned().unwrap_or_default();
        let surveys: i64 = db.query_row("SELECT count(*) FROM surveys", [], |r| r.get(0)).unwrap_or(0);
        let tiles: i64 = db.query_row("SELECT count(*) FROM tiles", [], |r| r.get(0)).unwrap_or(0);
        let info = PackInfo {
            file: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            edition: get("edition"),
            tier: get("tier"),
            region: get("region"),
            region_name: get("region_name"),
            min_zoom: get("min_zoom").parse().unwrap_or(0),
            max_zoom: get("max_zoom").parse().unwrap_or(0),
            datum: get("datum"),
            attribution: get("attribution"),
            created: get("created"),
            surveys: surveys as usize,
            tiles,
            bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        };
        Ok(Pack { db: Mutex::new(db), info })
    }

    fn tile(&self, z: u8, x: u32, y: u32) -> Option<Cells> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let b: Vec<u8> = db.query_row("SELECT data FROM tiles WHERE z = ?1 AND x = ?2 AND y = ?3", params![z, x, y], |r| r.get(0)).optional().ok()??;
        decode(&b)
    }
}

type TileKey = (usize, u8, u32, u32);

/// Every package in a folder, and caches of decoded tiles and drawn images.
pub struct DepthPacks {
    packs: Vec<Pack>,
    /// Packs finest first (by their finest zoom).
    order: Vec<usize>,
    tiles: Mutex<lru::LruCache<TileKey, Option<Arc<Cells>>>>,
    images: Mutex<lru::LruCache<(u8, u32, u32), Option<Arc<Vec<u8>>>>>,
    /// Files that couldn't be read: (file, why).
    pub problems: Vec<(String, String)>,
}

/// The zoom from which the depths are drawn.
pub const MIN_ZOOM: u8 = 8;

impl DepthPacks {
    /// Every `.tpdepth` file in `dir`.
    pub fn open(dir: &Path) -> DepthPacks {
        let mut packs = Vec::new();
        let mut problems = Vec::new();
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tpdepth"))).collect())
            .unwrap_or_default();
        paths.sort();
        for p in paths {
            match Pack::open(&p) {
                Ok(k) => packs.push(k),
                Err(e) => problems.push((p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), e)),
            }
        }
        let mut order: Vec<usize> = (0..packs.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse((packs[i].info.max_zoom, packs[i].info.edition.clone())));
        DepthPacks {
            packs,
            order,
            tiles: Mutex::new(lru::LruCache::new(NonZeroUsize::new(512).unwrap())),
            images: Mutex::new(lru::LruCache::new(NonZeroUsize::new(512).unwrap())),
            problems,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }

    /// The packages, as they describe themselves.
    pub fn infos(&self) -> Vec<PackInfo> {
        self.packs.iter().map(|p| p.info.clone()).collect()
    }

    /// Every survey in the packages (each once: from the newest edition that has it).
    pub fn surveys(&self) -> Vec<Survey> {
        let mut by: HashMap<i64, (String, Survey)> = HashMap::new();
        for p in &self.packs {
            let db = p.db.lock().unwrap_or_else(|e| e.into_inner());
            let Ok(mut st) = db.prepare("SELECT id, title, year, res, epsg, datum, accuracy, published, outline FROM surveys") else { continue };
            let rows = st.query_map([], |r| {
                Ok(Survey {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    year: r.get(2)?,
                    res: r.get(3)?,
                    epsg: r.get(4)?,
                    datum: r.get(5)?,
                    accuracy: r.get(6)?,
                    published: r.get(7)?,
                    outline: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
                })
            });
            for s in rows.into_iter().flatten().flatten() {
                if by.get(&s.id).is_none_or(|(ed, _)| *ed < p.info.edition) {
                    by.insert(s.id, (p.info.edition.clone(), s));
                }
            }
        }
        let mut out: Vec<Survey> = by.into_values().map(|(_, s)| s).collect();
        out.sort_by(|a, b| a.title.cmp(&b.title));
        out
    }

    fn tile(&self, pi: usize, z: u8, x: u32, y: u32) -> Option<Arc<Cells>> {
        let key = (pi, z, x, y);
        if let Some(t) = self.tiles.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return t.clone();
        }
        let t = self.packs[pi].tile(z, x, y).map(Arc::new);
        self.tiles.lock().unwrap_or_else(|e| e.into_inner()).put(key, t.clone());
        t
    }

    /// The height at map pixel (px, py) of zoom z from package `pi`, read at its zoom
    /// `level` (bilinear; nearest where a neighbour has no data).
    fn sample(&self, pi: usize, level: u8, z: u8, px: f64, py: f64) -> Option<f32> {
        let k = 2f64.powi(level as i32 - z as i32);
        let (fx, fy) = (px * k - 0.5, py * k - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = ((fx - x0) as f32, (fy - y0) as f32);
        let max = (N as f64) * (1u64 << level) as f64;
        let get = |cx: f64, cy: f64| -> Option<f32> {
            if cx < 0.0 || cy < 0.0 || cx >= max || cy >= max {
                return None;
            }
            let (cx, cy) = (cx as u64, cy as u64);
            let t = self.tile(pi, level, (cx / N as u64) as u32, (cy / N as u64) as u32)?;
            let v = t[(cy % N as u64) as usize * N + (cx % N as u64) as usize];
            (!v.is_nan()).then_some(v)
        };
        let v = [get(x0, y0), get(x0 + 1.0, y0), get(x0, y0 + 1.0), get(x0 + 1.0, y0 + 1.0)];
        match v {
            [Some(a), Some(b), Some(c), Some(d)] => Some((a * (1.0 - ax) + b * ax) * (1.0 - ay) + (c * (1.0 - ax) + d * ax) * ay),
            _ => {
                let near = [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
                let mut best: Option<(f32, f32)> = None;
                for (i, (dx, dy)) in near.iter().enumerate() {
                    if let Some(h) = v[i] {
                        let d = (ax - dx).powi(2) + (ay - dy).powi(2);
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, h));
                        }
                    }
                }
                // only next to data: a cell's width, not further
                best.filter(|(d, _)| *d <= 0.6).map(|(_, h)| h)
            }
        }
    }

    /// The height at map pixel (px, py) of zoom z: from the finest package that has data
    /// there, read at the zoom nearest z it holds.
    fn height(&self, z: u8, px: f64, py: f64) -> Option<f32> {
        for &pi in &self.order {
            let info = &self.packs[pi].info;
            if z + 2 < info.min_zoom {
                continue; // far coarser than this package goes: a coarser one has it
            }
            // the nearest zoom it holds, else coarser ones (a coarse survey only goes up to
            // the zoom of its own grid)
            let top = z.clamp(info.min_zoom, info.max_zoom);
            for level in (info.min_zoom..=top).rev().take(6) {
                if let Some(h) = self.sample(pi, level, z, px, py) {
                    return Some(h);
                }
            }
        }
        None
    }

    /// Height (m to the survey datum, negative below) at a place.
    pub fn height_at(&self, lon: f64, lat: f64) -> Option<f32> {
        let z = self.packs.iter().map(|p| p.info.max_zoom).max()?;
        let (px, py) = to_pixel(lon, lat, z);
        self.height(z, px, py)
    }

    /// The image for map tile z/x/y (256 pixels, PNG), or None where there is no data.
    pub fn tile_png(&self, z: u8, x: u32, y: u32) -> Option<Arc<Vec<u8>>> {
        if z < MIN_ZOOM || self.packs.is_empty() {
            return None;
        }
        if let Some(t) = self.images.lock().unwrap_or_else(|e| e.into_inner()).get(&(z, x, y)) {
            return t.clone();
        }
        let img = self.render(z, x, y).map(Arc::new);
        self.images.lock().unwrap_or_else(|e| e.into_inner()).put((z, x, y), img.clone());
        img
    }

    fn render(&self, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
        const G: usize = N + 2;
        let mut h = vec![f32::NAN; G * G];
        let mut any = false;
        for gy in 0..G {
            let py = (y as f64) * N as f64 + gy as f64 - 1.0 + 0.5;
            for gx in 0..G {
                let px = (x as f64) * N as f64 + gx as f64 - 1.0 + 0.5;
                if let Some(v) = self.height(z, px, py) {
                    h[gy * G + gx] = v;
                    any |= v <= 0.0;
                }
            }
        }
        if !any {
            return None;
        }
        let (_, lat) = from_pixel((x as f64 + 0.5) * N as f64, (y as f64 + 0.5) * N as f64, z);
        Some(crate::hires::shade(&h, G, cell_size(z, lat), crate::hires::contour_step(z), crate::hires::colour, 3.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tpdepth-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tiles_round_trip() {
        let mut c = vec![f32::NAN; N * N];
        for (i, v) in c.iter_mut().enumerate().skip(100) {
            *v = -((i % 700) as f32) * 0.37 + 2.0;
        }
        c[5000] = -2999.99;
        let e = encode(&c);
        let d = decode(&e).unwrap();
        for (a, b) in c.iter().zip(&d) {
            assert!((a.is_nan() && b.is_nan()) || (a - b).abs() <= 0.005, "{a} {b}");
        }
        // a smooth seabed packs small
        let smooth: Vec<f32> = (0..N * N).map(|i| -20.0 - (i / N) as f32 * 0.05 - (i % N) as f32 * 0.02).collect();
        assert!(encode(&smooth).len() < 20_000, "{}", encode(&smooth).len());
    }

    #[test]
    fn pixels_and_zooms() {
        let (px, py) = to_pixel(173.28, -41.27, 12);
        let (lon, lat) = from_pixel(px, py, 12);
        assert!((lon - 173.28).abs() < 1e-9 && (lat + 41.27).abs() < 1e-9);
        // at 41°S: 5 m → zoom 15 (3.6 m cells), 2 m → 16 (1.8 m), 1 m → 17 (0.9 m), 20 m → 13
        assert_eq!(zoom_for(5.0, -41.0), 15);
        assert_eq!(zoom_for(2.0, -41.0), 16);
        assert_eq!(zoom_for(1.0, -41.0), 17);
        assert_eq!(zoom_for(20.0, -41.0), 13);
    }

    /// A package built from painted cells: the pyramid keeps the shallowest, the reader
    /// finds heights at every zoom and draws images; extract splits it.
    #[test]
    fn build_read_extract() {
        let d = tmp("brx");
        let master = d.join("master.tpdepth");
        let w = Writer::create(&master).unwrap();
        w.set_meta("edition", "2026-11").unwrap();
        // a sloping seabed with a 2 m shoal, at zoom 15 round Nelson
        let z = 15u8;
        let (cx, cy) = to_pixel(173.2, -41.2, z);
        let (tx, ty) = ((cx / N as f64) as u32, (cy / N as f64) as u32);
        for dy in 0..2u32 {
            for dx in 0..2u32 {
                let mut c = vec![f32::NAN; N * N];
                for j in 0..N {
                    for i in 0..N {
                        c[j * N + i] = -30.0 + (i as f32) * 0.05;
                    }
                }
                if dx == 0 && dy == 0 {
                    c[10 * N + 10] = -2.0;
                }
                w.paint(z, tx + dx, ty + dy, &c).unwrap();
            }
        }
        w.add_survey(&Survey {
            id: 122423,
            title: "Tasman Bay".into(),
            year: 2022,
            res: 5.0,
            epsg: 32759,
            datum: "Approximate LAT".into(),
            outline: serde_json::json!({"type":"Polygon","coordinates":[[[172.1,-41.3],[174.1,-41.3],[174.1,-40.4],[172.1,-41.3]]]}),
            ..Default::default()
        })
        .unwrap();
        w.build_pyramid(8, z).unwrap();
        // the shoal survives to zoom 8
        let t8 = w.tile(8, tx >> 7, ty >> 7).unwrap();
        assert!(t8.iter().any(|v| *v == -2.0), "shoal kept");
        w.finish().unwrap();

        // national (≤ 13) and a regional (14–15) package
        let nat = d.join("nz.tpdepth");
        let n1 = extract(&master, &nat, 8..=13, None, &[("tier", "national")]).unwrap();
        let reg = d.join("nelson.tpdepth");
        let n2 = extract(&master, &reg, 14..=15, Some([172.0, -42.0, 175.0, -40.0]), &[("tier", "region"), ("region", "nelson")]).unwrap();
        assert!(n1 > 0 && n2 > 0);
        let none = extract(&master, &d.join("far.tpdepth"), 14..=15, Some([165.0, -48.0, 168.0, -45.0]), &[]).unwrap();
        assert_eq!(none, 0);
        std::fs::remove_file(d.join("far.tpdepth")).unwrap();
        std::fs::remove_file(&master).unwrap();

        let p = DepthPacks::open(&d);
        assert_eq!(p.infos().len(), 2, "{:?}", p.problems);
        assert_eq!(p.surveys().len(), 1);
        // the depth in the middle of the painted area, from the regional package
        let (lon, lat) = from_pixel(cx + 300.0, cy + 100.0, z);
        let h = p.height_at(lon, lat).unwrap();
        assert!(h < -10.0 && h > -30.0, "{h}");
        // drawn at a coarse zoom (national) and a fine one (regional), not where there's nothing
        let (x10, y10) = ((cx / N as f64 / 32.0) as u32, (cy / N as f64 / 32.0) as u32);
        assert!(p.tile_png(10, x10, y10).is_some());
        assert!(p.tile_png(16, (tx + 1) * 2, ty * 2 + 1).is_some());
        assert!(p.tile_png(16, 0, 0).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn not_a_package() {
        let d = tmp("bad");
        std::fs::write(d.join("x.tpdepth"), b"hello").unwrap();
        let p = DepthPacks::open(&d);
        assert!(p.is_empty());
        assert_eq!(p.problems.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
