//! Own soundings: the boat's own depth soundings gridded into a seabed, drawn as depth
//! images for the chart.
//!
//! ## The grid
//! TimeZero's layout, so its files import without resampling: WGS84 **ellipsoidal**
//! Mercator (EPSG:3395; not the spherical Web Mercator of the chart tiles), a quadtree of
//! 256 × 256-cell tiles, level 16 the finest (cells 360°/2²⁴ wide: about 2.4 m at the
//! equator, 1.8 m at 41° S). Levels [`MIN_LEVEL`]..16 are kept; each coarser level merges
//! 2 × 2 cells of the one below. Tiles are numbered from the top-left (as map tiles); TimeZero
//! numbers its rows from the south.
//!
//! ## A cell
//! Not the soundings themselves but their running statistics ([`Cell`]): count, mean,
//! sum of squared differences (for the spread), the newest time and which kinds of source
//! went in. Adding a sounding, another boat's grid or an imported file is the same merge,
//! and gives the same answer in any order.
//!
//! Heights are metres, positive up (a depth of 30 m is -30), as recorded: own soundings
//! are below the surface or the transducer, not reduced to chart datum. Not for navigation.

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::io::{Read, Write as _};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use rusqlite::{params, Connection, OptionalExtension};

pub mod check;
pub mod live;
mod import;
mod render;
pub mod share;
pub use import::{clean_depths, decode_pbg_blob, pbg_level16, pbg_made, PbgTile};
pub use render::{Palette, Style};

/// The finest level (TimeZero's).
pub const MAX_LEVEL: u8 = 16;
/// The coarsest level kept (a tile is about 600 km across).
pub const MIN_LEVEL: u8 = 3;
/// The coarsest level stores made before 30 Oct 2026 have (see [`Store::ensure_levels`]).
const OLD_MIN_LEVEL: u8 = 6;
/// Cells along a tile's side.
pub const TILE: usize = 256;

/// Kinds of source, as bits of [`Cell::src`].
pub const SRC_OWN: u8 = 1;
pub const SRC_PBG: u8 = 2;
pub const SRC_XYZ: u8 = 4;
pub const SRC_OTHER: u8 = 8;
/// Why a left-out sounding was left out (more bits of [`Cell::src`], left-out cells only).
pub const WHY_SPIKE: u8 = 16;
pub const WHY_SHALLOW: u8 = 32;
pub const WHY_DEEP: u8 = 64;
pub const WHY_RUN: u8 = 128;
/// The source bits of [`Cell::src`] (without the reasons).
pub const SRC_BITS: u8 = 15;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("{0}")]
    Format(String),
    #[error("cancelled")]
    Cancelled,
}
pub type Result<T> = std::result::Result<T, Error>;

// ---------- ellipsoidal Mercator ---------------------------------------------------------

const E: f64 = 0.081_819_190_842_621_5; // WGS84 first eccentricity
const MAX_LAT: f64 = 85.0;

/// Mercator northing (radians of the equator) of a latitude, on the WGS84 ellipsoid.
pub fn merc_y(lat: f64) -> f64 {
    let p = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let s = p.sin();
    ((PI / 4.0 + p / 2.0).tan() * ((1.0 - E * s) / (1.0 + E * s)).powf(E / 2.0)).ln()
}

/// Latitude of a Mercator northing (inverse of [`merc_y`]).
pub fn merc_lat(y: f64) -> f64 {
    let t = (-y).exp();
    let mut p = PI / 2.0 - 2.0 * t.atan();
    for _ in 0..8 {
        let s = p.sin();
        p = PI / 2.0 - 2.0 * (t * ((1.0 - E * s) / (1.0 + E * s)).powf(E / 2.0)).atan();
    }
    p.to_degrees()
}

/// A place as fractions of the world (x east from 180° W, y down from the top), 0..1.
pub fn to_world(lon: f64, lat: f64) -> (f64, f64) {
    let lon = ((lon + 180.0).rem_euclid(360.0)) - 180.0;
    ((lon + 180.0) / 360.0, (1.0 - merc_y(lat) / PI) / 2.0)
}

/// The place at world fractions `(u, v)`.
pub fn from_world(u: f64, v: f64) -> (f64, f64) {
    (u * 360.0 - 180.0, merc_lat(PI * (1.0 - 2.0 * v)))
}

/// Cells across the world at a level.
pub fn cells_across(level: u8) -> u64 {
    (TILE as u64) << level
}

/// The cell (column, row from the top) a place is in at a level.
pub fn cell_of(lon: f64, lat: f64, level: u8) -> Option<(u32, u32)> {
    if !lon.is_finite() || !lat.is_finite() || lat.abs() > MAX_LAT {
        return None;
    }
    let (u, v) = to_world(lon, lat);
    let n = cells_across(level) as f64;
    let (cx, cy) = ((u * n).floor(), (v * n).floor());
    if cx < 0.0 || cy < 0.0 || cx >= n || cy >= n {
        return None;
    }
    Some((cx as u32, cy as u32))
}

/// The centre of a cell.
pub fn cell_centre(cx: u32, cy: u32, level: u8) -> (f64, f64) {
    let n = cells_across(level) as f64;
    from_world((cx as f64 + 0.5) / n, (cy as f64 + 0.5) / n)
}

/// Ground width of a cell (m) at a latitude.
pub fn cell_size_m(level: u8, lat: f64) -> f64 {
    40_075_016.686 * lat.to_radians().cos() / cells_across(level) as f64
}

// ---------- cells -------------------------------------------------------------------------

/// Running statistics of the soundings in a cell.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cell {
    /// Soundings merged (0 = empty).
    pub n: u32,
    /// Mean height (m, positive up).
    pub mean: f32,
    /// Sum of squared differences from the mean (m²): the spread is √(m2 / n).
    pub m2: f32,
    /// Newest sounding, hours since 1970 (0 = unknown).
    pub newest: u32,
    /// Kinds of source ([`SRC_OWN`] …).
    pub src: u8,
}

impl Cell {
    pub fn one(z: f32, hours: u32, src: u8) -> Cell {
        Cell { n: 1, mean: z, m2: 0.0, newest: hours, src }
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// Add another cell's soundings (Chan's parallel update).
    pub fn merge(&mut self, o: &Cell) {
        if o.n == 0 {
            return;
        }
        if self.n == 0 {
            *self = *o;
            return;
        }
        let (na, nb) = (self.n as f64, o.n as f64);
        let n = na + nb;
        let d = o.mean as f64 - self.mean as f64;
        self.mean = (self.mean as f64 + d * nb / n) as f32;
        self.m2 = (self.m2 as f64 + o.m2 as f64 + d * d * na * nb / n) as f32;
        self.n = self.n.saturating_add(o.n);
        self.newest = self.newest.max(o.newest);
        self.src |= o.src;
    }
    /// Standard deviation of the soundings (m).
    pub fn spread(&self) -> f32 {
        if self.n < 2 {
            0.0
        } else {
            (self.m2.max(0.0) / self.n as f32).sqrt()
        }
    }
}

/// A tile's cells, all 256 × 256 (empty ones n = 0), row by row from the top.
pub struct Dense(pub Vec<Cell>);

impl Dense {
    pub fn new() -> Dense {
        Dense(vec![Cell::default(); TILE * TILE])
    }
    pub fn count(&self) -> usize {
        self.0.iter().filter(|c| c.n > 0).count()
    }
}

impl Default for Dense {
    fn default() -> Self {
        Dense::new()
    }
}

const MAGIC: &[u8; 4] = b"TPB1";
const REC: usize = 19;

/// A tile as stored: "TPB1", the number of cells, then zlib of 19-byte records
/// (x u8, y u8, n u32, mean f32, m2 f32, newest u32, src u8; little-endian).
pub fn encode(d: &Dense) -> (usize, Vec<u8>) {
    let mut raw = Vec::with_capacity(4096);
    let mut count = 0usize;
    for (i, c) in d.0.iter().enumerate() {
        if c.n == 0 {
            continue;
        }
        count += 1;
        raw.push((i % TILE) as u8);
        raw.push((i / TILE) as u8);
        raw.extend_from_slice(&c.n.to_le_bytes());
        raw.extend_from_slice(&c.mean.to_le_bytes());
        raw.extend_from_slice(&c.m2.to_le_bytes());
        raw.extend_from_slice(&c.newest.to_le_bytes());
        raw.push(c.src);
    }
    let mut out = Vec::with_capacity(raw.len() / 2 + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    let mut z = flate2::write::ZlibEncoder::new(out, flate2::Compression::fast());
    let _ = z.write_all(&raw);
    (count, z.finish().unwrap_or_default())
}

/// Read a stored tile: calls `f(x, y, cell)` for each cell.
pub fn decode(b: &[u8], mut f: impl FnMut(usize, usize, Cell)) -> Result<()> {
    if b.len() < 8 || &b[..4] != MAGIC {
        return Err(Error::Format("not a bathymetry tile".into()));
    }
    let count = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let mut raw = Vec::with_capacity(count * REC);
    flate2::read::ZlibDecoder::new(&b[8..]).read_to_end(&mut raw)?;
    if raw.len() != count * REC {
        return Err(Error::Format("bathymetry tile cut short".into()));
    }
    let f32_at = |r: &[u8], o: usize| f32::from_le_bytes(r[o..o + 4].try_into().unwrap());
    let u32_at = |r: &[u8], o: usize| u32::from_le_bytes(r[o..o + 4].try_into().unwrap());
    for r in raw.chunks_exact(REC) {
        f(r[0] as usize, r[1] as usize, Cell { n: u32_at(r, 2), mean: f32_at(r, 6), m2: f32_at(r, 10), newest: u32_at(r, 14), src: r[18] });
    }
    Ok(())
}

// ---------- the store ---------------------------------------------------------------------

/// One import or build, as listed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceInfo {
    pub kind: String,
    pub name: String,
    pub bytes: i64,
    /// When it was added (Unix seconds).
    pub added: i64,
    /// Soundings (or grid cells) it brought.
    pub soundings: i64,
    /// Soundings it brought that the checks left out.
    pub rejected: i64,
}

/// What's in the store.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Info {
    /// Cells with soundings at the finest level.
    pub cells: i64,
    pub tiles: i64,
    /// [w, s, e, n] of the finest tiles, when there are any.
    pub bounds: Option<[f64; 4]>,
    pub bytes: u64,
    pub sources: Vec<SourceInfo>,
    /// Soundings left out by the checks (kept apart, to be shown).
    pub rejected: i64,
}

/// The area holding nearly all the soundings [w, s, e, n] (e may pass 180), from the
/// level-10 tiles weighted by their cells: the outermost 0.5 % each side (a stray fix, a
/// sounding at 0° 0°) is left out, and an area across 180° (the Chatham Rise) stays whole.
fn main_area(db: &Connection) -> Result<Option<[f64; 4]>> {
    const L: u8 = 10;
    let mut q = db.prepare("SELECT ty, tx, n FROM tiles WHERE l = ?1")?;
    let t: Vec<(i64, i64, i64)> = q.query_map([L], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
    if t.is_empty() {
        return Ok(None);
    }
    let n = 1i64 << L;
    let total: i64 = t.iter().map(|x| x.2.max(1)).sum();
    // (lowest, highest) tile index holding all but `cut` of the cells either side
    let range = |mut v: Vec<(i64, i64)>| -> (i64, i64) {
        v.sort();
        let cut = total / 200;
        let (mut acc, mut lo) = (0, v[0].0);
        for &(i, c) in &v {
            acc += c;
            if acc > cut {
                lo = i;
                break;
            }
        }
        let (mut acc, mut hi) = (0, v[v.len() - 1].0);
        for &(i, c) in v.iter().rev() {
            acc += c;
            if acc > cut {
                hi = i;
                break;
            }
        }
        (lo, hi)
    };
    let (y0, y1) = range(t.iter().map(|x| (x.0, x.2.max(1))).collect());
    let (x0, x1) = range(t.iter().map(|x| (x.1, x.2.max(1))).collect());
    // the same with longitudes counted from 0° (so 180° is in the middle)
    let (s0, s1) = range(t.iter().map(|x| ((x.1 + n / 2) % n, x.2.max(1))).collect());
    let (x0, x1) = if s1 - s0 < x1 - x0 { (s0 - n / 2, s1 - n / 2) } else { (x0, x1) };
    let nf = n as f64;
    let w = x0 as f64 / nf * 360.0 - 180.0;
    let e = (x1 + 1) as f64 / nf * 360.0 - 180.0;
    let (_, north) = from_world(0.0, y0 as f64 / nf);
    let (_, south) = from_world(0.0, (y1 + 1) as f64 / nf);
    let w2 = (w + 180.0).rem_euclid(360.0) - 180.0;
    Ok(Some([w2, south, w2 + (e - w), north]))
}

/// The sounding at a place.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Sounding {
    /// Mean height (m, positive up).
    pub z: f32,
    pub n: u32,
    /// Standard deviation (m).
    pub spread: f32,
    /// Newest sounding (Unix seconds, 0 = unknown).
    pub newest: i64,
    pub src: u8,
    /// Distance to the cell's centre (m): soundings are looked for up to two cells away.
    pub off_m: f64,
}

type Key = (u8, u32, u32);

/// How near the passed soundings giving a second opinion on a left-out one must be (m):
/// see [`Store::second_opinion`].
pub const AGREE_M: f64 = 50.0;
/// How near other days' left-out soundings vouching for one deeper than the chart must be (m).
pub const DEEP_AGREE_M: f64 = 200.0;

/// The table of the seabed's tiles.
pub(crate) const SEABED: &str = "tiles";
/// The table of the soundings the checks left out (the same levels, kept apart).
pub(crate) const LEFT_OUT: &str = "left_out";

/// The bathymetry file (SQLite) and caches for drawing.
pub struct Store {
    path: PathBuf,
    conn: Mutex<Connection>,
    /// Mean heights of tiles (NaN where empty; empty Vec for a tile with no data).
    means: Mutex<lru::LruCache<Key, Arc<Vec<f32>>>>,
    images: Mutex<lru::LruCache<(u8, u32, u32), Option<Arc<Vec<u8>>>>>,
    style: RwLock<Style>,
    /// Bumped whenever the data or the style change (the page reloads its tiles).
    generation: AtomicU64,
}

fn schema(c: &Connection) -> Result<()> {
    c.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS tiles (l INTEGER NOT NULL, ty INTEGER NOT NULL, tx INTEGER NOT NULL, n INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (l, ty, tx)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS rejected (ty INTEGER NOT NULL, tx INTEGER NOT NULL, n INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (ty, tx)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS left_out (l INTEGER NOT NULL, ty INTEGER NOT NULL, tx INTEGER NOT NULL, n INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (l, ty, tx)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS sources (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, bytes INTEGER NOT NULL DEFAULT 0, added INTEGER NOT NULL, soundings INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS seeded (track_id TEXT PRIMARY KEY, modified INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT);",
    )?;
    // stores made before the checks have no column for what they left out
    let _ = c.execute_batch("ALTER TABLE sources ADD COLUMN rejected INTEGER NOT NULL DEFAULT 0");
    c.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok(())
}

fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Unix seconds as the hours a [`Cell`] keeps.
pub fn hours(unix_s: i64) -> u32 {
    (unix_s.max(0) / 3600).min(u32::MAX as i64) as u32
}

impl Store {
    /// Open (or create) the store at `path`.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let conn = Connection::open(path)?;
        schema(&conn)?;
        Ok(Store {
            path: path.to_path_buf(),
            conn: Mutex::new(conn),
            means: Mutex::new(lru::LruCache::new(NonZeroUsize::new(192).unwrap())),
            images: Mutex::new(lru::LruCache::new(NonZeroUsize::new(768).unwrap())),
            style: RwLock::new(Style::default()),
            generation: AtomicU64::new(1),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes whenever what's drawn changes.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Forget the drawn images and cached tiles (after an import, or a style change).
    pub fn changed(&self) {
        self.means.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.images.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn style(&self) -> Style {
        self.style.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_style(&self, s: Style) {
        *self.style.write().unwrap_or_else(|e| e.into_inner()) = s;
        self.images.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn blob(&self, k: Key) -> Result<Option<Vec<u8>>> {
        self.blob_in(SEABED, k)
    }

    fn blob_in(&self, table: &str, k: Key) -> Result<Option<Vec<u8>>> {
        Ok(self.db().query_row(&format!("SELECT data FROM {table} WHERE l = ?1 AND ty = ?2 AND tx = ?3"), params![k.0, k.1, k.2], |r| r.get(0)).optional()?)
    }

    /// A tile's cells, when it has any.
    pub fn tile(&self, level: u8, ty: u32, tx: u32) -> Result<Option<Dense>> {
        let Some(b) = self.blob((level, ty, tx))? else { return Ok(None) };
        let mut d = Dense::new();
        decode(&b, |x, y, c| d.0[y * TILE + x] = c)?;
        Ok(Some(d))
    }

    /// A tile's mean heights (NaN where empty), cached; an empty Vec where there's no tile.
    pub(crate) fn means(&self, level: u8, ty: u32, tx: u32) -> Arc<Vec<f32>> {
        self.means_in(SEABED, level, ty, tx)
    }

    /// [`Store::means`] of the seabed ([`SEABED`]) or of what the checks left out ([`LEFT_OUT`]).
    pub(crate) fn means_in(&self, table: &'static str, level: u8, ty: u32, tx: u32) -> Arc<Vec<f32>> {
        let k = (level + if table == LEFT_OUT { 64 } else { 0 }, ty, tx);
        if let Some(m) = self.means.lock().unwrap_or_else(|e| e.into_inner()).get(&k) {
            return m.clone();
        }
        let mut v = Vec::new();
        if let Ok(Some(b)) = self.blob_in(table, (level, ty, tx)) {
            v = vec![f32::NAN; TILE * TILE];
            if decode(&b, |x, y, c| v[y * TILE + x] = c.mean).is_err() {
                v.clear();
            }
        }
        let m = Arc::new(v);
        self.means.lock().unwrap_or_else(|e| e.into_inner()).put(k, m.clone());
        m
    }

    /// The sounding at a place: the finest cell there, else the nearest within two cells.
    pub fn sounding_at(&self, lon: f64, lat: f64) -> Option<Sounding> {
        self.sounding_in(SEABED, lon, lat, 4.0)
    }

    /// The nearest sounding to a place within `radius_m` (for the cursor: a few pixels at
    /// the zoom shown). Beyond about 24 cells the search moves to a coarser level, whose
    /// cells merge the soundings in them.
    pub fn sounding_near(&self, lon: f64, lat: f64, radius_m: f64) -> Option<Sounding> {
        self.sounding_in(SEABED, lon, lat, radius_m)
    }

    /// A left-out sounding at a place (within two cells), with why in its `src` bits.
    pub fn left_out_at(&self, lon: f64, lat: f64) -> Option<Sounding> {
        self.sounding_in(LEFT_OUT, lon, lat, 4.0)
    }

    /// The nearest left-out sounding within `radius_m`, with why in its `src` bits.
    pub fn left_out_near(&self, lon: f64, lat: f64, radius_m: f64) -> Option<Sounding> {
        self.sounding_in(LEFT_OUT, lon, lat, radius_m)
    }

    fn sounding_in(&self, table: &str, lon: f64, lat: f64, radius_m: f64) -> Option<Sounding> {
        // the level to search: radius at most about 24 of its cells
        let mut level = MAX_LEVEL;
        let mut rc = (radius_m.max(0.0) / cell_size_m(MAX_LEVEL, lat).max(0.01)).max(1.0);
        while rc > 24.0 && level > MIN_LEVEL {
            level -= 1;
            rc /= 2.0;
        }
        let r = rc.ceil() as i64;
        let (cx, cy) = cell_of(lon, lat, level)?;
        let mut tiles: HashMap<(u32, u32), Option<Dense>> = HashMap::new();
        let mut best: Option<(i64, Cell, u32, u32)> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                let d2 = dx * dx + dy * dy;
                if d2 as f64 > (rc + 0.5) * (rc + 0.5) || best.as_ref().is_some_and(|b| d2 >= b.0) {
                    continue;
                }
                let (x, y) = (cx as i64 + dx, cy as i64 + dy);
                if x < 0 || y < 0 {
                    continue;
                }
                let (x, y) = (x as u32, y as u32);
                let t = tiles.entry((y >> 8, x >> 8)).or_insert_with(|| {
                    self.blob_in(table, (level, y >> 8, x >> 8)).ok().flatten().and_then(|b| {
                        let mut d = Dense::new();
                        decode(&b, |x, y, c| d.0[y * TILE + x] = c).ok()?;
                        Some(d)
                    })
                });
                let Some(t) = t else { continue };
                let c = t.0[(y as usize & 255) * TILE + (x as usize & 255)];
                if c.n > 0 {
                    best = Some((d2, c, x, y));
                }
            }
        }
        let (_, c, x, y) = best?;
        let (clon, clat) = cell_centre(x, y, level);
        let (dx, dy) = ((clon - lon) * lat.to_radians().cos() * 60.0 * 1852.0, (clat - lat) * 60.0 * 1852.0);
        Some(Sounding { z: c.mean, n: c.n, spread: c.spread(), newest: c.newest as i64 * 3600, src: c.src, off_m: (dx * dx + dy * dy).sqrt() })
    }

    pub fn info(&self) -> Result<Info> {
        let db = self.db();
        let (cells, tiles): (i64, i64) = db.query_row("SELECT coalesce(sum(n), 0), count(*) FROM tiles WHERE l = ?1", [MAX_LEVEL], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let bounds = main_area(&db)?;
        let mut q = db.prepare("SELECT kind, name, bytes, added, soundings, rejected FROM sources ORDER BY id")?;
        let sources = q
            .query_map([], |r| Ok(SourceInfo { kind: r.get(0)?, name: r.get(1)?, bytes: r.get(2)?, added: r.get(3)?, soundings: r.get(4)?, rejected: r.get(5)? }))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let rejected: i64 = db.query_row("SELECT coalesce(sum(n), 0) FROM left_out WHERE l = ?1", [MAX_LEVEL], |r| r.get(0))?;
        let bytes = ["", "-wal"].iter().filter_map(|s| std::fs::metadata(format!("{}{s}", self.path.display())).ok()).map(|m| m.len()).sum();
        Ok(Info { cells, tiles, bounds, bytes, sources, rejected })
    }

    /// Build the coarsest levels where a store made by an older TweedPlot stops at level 6
    /// (quick: a few tiles). Called on opening.
    pub fn ensure_levels(&self) -> Result<()> {
        // left-out soundings from the first checks (29 Oct 2026): finest level only, no levels
        let old: Vec<(u32, u32, i64, Vec<u8>)> = {
            let db = self.db();
            let mut q = db.prepare("SELECT ty, tx, n, data FROM rejected")?;
            let v = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<std::result::Result<_, _>>()?;
            v
        };
        if !old.is_empty() {
            let mut w = self.writer("", "", 0)?;
            let mut dirty = HashSet::new();
            for (ty, tx, n, b) in old {
                w.write_in(LEFT_OUT, (MAX_LEVEL, ty, tx), n as usize, &b)?;
                dirty.insert((ty, tx));
            }
            w.build_levels_in(LEFT_OUT, MAX_LEVEL, dirty, &mut |_, _, _| {}, &AtomicBool::new(false))?;
            w.conn.execute_batch("DELETE FROM rejected; COMMIT")?;
            self.changed();
        }
        let (old, new): (i64, i64) = self.db().query_row(
            "SELECT (SELECT count(*) FROM tiles WHERE l = ?1), (SELECT count(*) FROM tiles WHERE l = ?2)",
            params![OLD_MIN_LEVEL, MIN_LEVEL],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if old == 0 || new > 0 {
            return Ok(());
        }
        let keys: Vec<(u32, u32)> = {
            let db = self.db();
            let mut q = db.prepare("SELECT ty, tx FROM tiles WHERE l = ?1")?;
            let k = q.query_map([OLD_MIN_LEVEL], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
            k
        };
        let mut w = self.writer("", "", 0)?;
        w.build_levels(OLD_MIN_LEVEL, keys.into_iter().collect(), &mut |_, _, _| {}, &AtomicBool::new(false))?;
        w.conn.execute_batch("COMMIT")?;
        self.changed();
        Ok(())
    }

    /// A second opinion on what the checks left out: a left-out sounding whose own
    /// soundings around it (within [`AGREE_M`]; at least three
    /// 7 m cells that passed) agree with it, within 5 m or 5 %, goes back in. On a canyon wall or a
    /// shelf break covered by several tracks the chart's coarse bands were wrong, not the
    /// sounder; a sounder locked onto something else disagrees with the soundings beside it.
    ///
    /// With `chart`, also: a left-out sounding *deeper* than the chart (a drowned river
    /// channel or a canyon narrower than the chart's depth areas) goes back in when left-out
    /// soundings from at least two other days within [`DEEP_AGREE_M`] agree with it. A
    /// sounder's faults read shallow (fish, a scattering layer, a held depth), and a run
    /// from one trip can't vouch for itself; soundings too shallow need passed ones.
    ///
    /// Returns how many soundings went back in.
    pub fn second_opinion(&self, chart: Option<&check::Reference>, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<i64> {
        // every left-out cell, in memory (16 bytes each), and in buckets for the deep rule
        let mut cells: Vec<(i64, i64, Cell)> = Vec::new();
        {
            let db = self.db();
            let mut q = db.prepare("SELECT ty, tx, data FROM left_out WHERE l = ?1 ORDER BY ty, tx")?;
            let mut rows = q.query([MAX_LEVEL])?;
            while let Some(r) = rows.next()? {
                let (ty, tx, b): (i64, i64, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                decode(&b, |x, y, c| cells.push((tx * 256 + x as i64, ty * 256 + y as i64, c)))?;
            }
        }
        if cells.is_empty() {
            return Ok(0);
        }
        const B: i64 = 64; // bucket: 64 cells (about 115 m)
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        if chart.is_some() {
            for (i, c) in cells.iter().enumerate() {
                buckets.entry((c.0.div_euclid(B), c.1.div_euclid(B))).or_default().push(i);
            }
        }
        // the seabed one level up (cells of about 7 m), for agreement with passed soundings
        const UP: u8 = MAX_LEVEL - 2;
        let mut seabed: HashMap<(u32, u32), Option<Vec<f32>>> = HashMap::new();
        let mut w = self.writer("", "", 0)?;
        let agree = |a: f64, b: f64| (a - b).abs() <= (0.05 * b.abs()).max(5.0);
        let mut back: Vec<usize> = Vec::new();
        for (k, &(gx, gy, c)) in cells.iter().enumerate() {
            if k % 20_000 == 0 {
                if cancel.load(Ordering::Relaxed) {
                    let _ = w.conn.execute_batch("ROLLBACK");
                    return Err(Error::Cancelled);
                }
                progress("Second opinion", k, cells.len());
                if seabed.len() > 4000 {
                    seabed.clear();
                }
            }
            let (_, lat) = cell_centre(gx as u32, gy as u32, MAX_LEVEL);
            let cell_m = cell_size_m(MAX_LEVEL, lat).max(0.1);
            // 1. the passed soundings within about AGREE_M
            let (ux, uy) = (gx >> 2, gy >> 2);
            let r_up = (AGREE_M / (cell_m * 4.0)).ceil() as i64;
            let (mut sum, mut n) = (0f64, 0u32);
            for oy in -r_up..=r_up {
                for ox in -r_up..=r_up {
                    let (x, y) = (ux + ox, uy + oy);
                    if x < 0 || y < 0 {
                        continue;
                    }
                    let key = ((y >> 8) as u32, (x >> 8) as u32);
                    if let std::collections::hash_map::Entry::Vacant(e) = seabed.entry(key) {
                        let m = w.read_in(SEABED, (UP, key.0, key.1))?.map(|b| {
                            let mut v = vec![f32::NAN; TILE * TILE];
                            let _ = decode(&b, |x, y, c| v[y * TILE + x] = c.mean);
                            v
                        });
                        e.insert(m);
                    }
                    if let Some(Some(v)) = seabed.get(&key) {
                        let z = v[((y & 255) * TILE as i64 + (x & 255)) as usize];
                        if z.is_finite() {
                            sum += z as f64;
                            n += 1;
                        }
                    }
                }
            }
            if n >= 3 && agree(c.mean as f64, sum / n as f64) {
                back.push(k);
                continue;
            }
            // 0. left out only by the chart, and the chart rule (now) lets it in
            let (lon, _) = cell_centre(gx as u32, gy as u32, MAX_LEVEL);
            let chart_only = c.src & (WHY_SPIKE | WHY_RUN) == 0;
            if chart_only && chart.and_then(|r| r.at(lon, lat)).is_some_and(|rg| !check::off_chart(-(c.mean as f64), rg)) {
                back.push(k);
                continue;
            }
            // 2. deeper than the chart: other days' left-out soundings within DEEP_AGREE_M
            let Some(r) = chart else { continue };
            if !r.at(lon, lat).is_some_and(|(_, hi)| hi.is_finite() && -(c.mean as f64) > hi as f64) {
                continue;
            }
            let rc = DEEP_AGREE_M / cell_m;
            let rb = (rc / B as f64).ceil() as i64;
            let own = c.newest / 24;
            let mut days: Vec<u32> = Vec::new();
            'look: for by in (gy.div_euclid(B) - rb)..=(gy.div_euclid(B) + rb) {
                for bx in (gx.div_euclid(B) - rb)..=(gx.div_euclid(B) + rb) {
                    let Some(list) = buckets.get(&(bx, by)) else { continue };
                    for &j in list {
                        let (x, y, o) = cells[j];
                        let day = o.newest / 24;
                        if day == own || days.contains(&day) {
                            continue;
                        }
                        let (dx, dy) = ((x - gx) as f64, (y - gy) as f64);
                        if dx * dx + dy * dy <= rc * rc && agree(o.mean as f64, c.mean as f64) {
                            days.push(day);
                            if days.len() >= 2 {
                                break 'look;
                            }
                        }
                    }
                }
            }
            if days.len() >= 2 {
                back.push(k);
            }
        }
        progress("Second opinion", cells.len(), cells.len());
        if back.is_empty() {
            let _ = w.conn.execute_batch("ROLLBACK");
            return Ok(0);
        }
        // take them out of their left-out tiles and put them in the seabed
        let mut by_tile: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        let mut moved = 0i64;
        for &k in &back {
            let (gx, gy, mut c) = cells[k];
            c.src &= SRC_BITS;
            by_tile.entry(((gy >> 8) as u32, (gx >> 8) as u32)).or_default().push(((gy & 255) * TILE as i64 + (gx & 255)) as usize);
            w.add_cell(gx as u32, gy as u32, c)?;
            moved += c.n as i64;
        }
        for ((ty, tx), idx) in by_tile {
            let Some(b) = w.read_in(LEFT_OUT, (MAX_LEVEL, ty, tx))? else { continue };
            let mut left = Dense::new();
            decode(&b, |x, y, c| left.0[y * TILE + x] = c)?;
            for i in idx {
                left.0[i] = Cell::default();
            }
            let (n, blob) = encode(&left);
            w.write_in(LEFT_OUT, (MAX_LEVEL, ty, tx), n, &blob)?;
            w.left_dirty.insert((ty, tx));
        }
        // the tally of what went in isn't a new source
        w.soundings = 0;
        w.finish(progress, cancel)?;
        self.changed();
        Ok(moved)
    }

    /// A setting kept in the store (`meta` table).
    pub fn meta(&self, k: &str) -> Option<String> {
        self.db().query_row("SELECT v FROM meta WHERE k = ?1", [k], |r| r.get(0)).optional().ok().flatten().flatten()
    }

    pub fn set_meta(&self, k: &str, v: &str) -> Result<()> {
        self.db().execute("INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)", params![k, v])?;
        Ok(())
    }

    /// Add to a running source (one row per kind and name, e.g. live soundings per day):
    /// its soundings and left-out counts grow, its time is the latest.
    pub fn tally_source(&self, kind: &str, name: &str, soundings: i64, rejected: i64) -> Result<()> {
        let db = self.db();
        let n = db.execute(
            "UPDATE sources SET soundings = soundings + ?3, rejected = rejected + ?4, added = ?5 WHERE kind = ?1 AND name = ?2",
            params![kind, name, soundings, rejected, now_s()],
        )?;
        if n == 0 {
            db.execute("INSERT INTO sources (kind, name, bytes, added, soundings, rejected) VALUES (?1, ?2, 0, ?3, ?4, ?5)", params![kind, name, now_s(), soundings, rejected])?;
        }
        Ok(())
    }

    /// Mark tracks as added without adding them (their soundings came in live).
    pub fn mark_seeded(&self, ids: &[(String, i64)]) -> Result<()> {
        let db = self.db();
        for (id, m) in ids {
            db.execute("INSERT OR REPLACE INTO seeded (track_id, modified) VALUES (?1, ?2)", params![id, m])?;
        }
        Ok(())
    }

    /// The tracks already added, with their modified times.
    pub fn seeded(&self) -> Result<HashMap<String, i64>> {
        let db = self.db();
        let mut q = db.prepare("SELECT track_id, modified FROM seeded")?;
        let m = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<HashMap<_, _>, _>>()?;
        Ok(m)
    }

    /// Whether a file of this name and size was imported already.
    pub fn has_source(&self, kind: &str, name: &str, bytes: i64) -> bool {
        self.db()
            .query_row("SELECT 1 FROM sources WHERE kind = ?1 AND name = ?2 AND bytes = ?3", params![kind, name, bytes], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    /// Remove everything.
    pub fn clear(&self) -> Result<()> {
        self.db().execute_batch("DELETE FROM tiles; DELETE FROM rejected; DELETE FROM left_out; DELETE FROM sources; DELETE FROM seeded; DELETE FROM meta WHERE k = 'origin'; VACUUM;")?;
        self.changed();
        Ok(())
    }

    /// Depths (positive down, m) from the 2nd to the 98th percentile in an area [w, s, e, n],
    /// from a level with at most about 16 × 16 tiles over it: a colour range that fits the view.
    pub fn depth_range(&self, area: [f64; 4]) -> Option<(f32, f32)> {
        let [w, s, e, n] = area;
        let (u0, v0) = to_world(w.max(-180.0), n.min(MAX_LAT));
        let (u1, v1) = to_world(e.min(180.0 - 1e-9), s.max(-MAX_LAT));
        let mut level = MAX_LEVEL;
        while level > MIN_LEVEL && ((u1 - u0) * (1u64 << level) as f64 > 16.0 || (v1 - v0) * (1u64 << level) as f64 > 16.0) {
            level -= 1;
        }
        let nc = cells_across(level) as f64;
        let (cx0, cx1) = ((u0 * nc).floor() as u64, (u1 * nc).floor() as u64);
        let (cy0, cy1) = ((v0 * nc).floor() as u64, (v1 * nc).floor() as u64);
        let mut depths = Vec::new();
        for ty in (cy0 >> 8)..=(cy1 >> 8) {
            for tx in (cx0 >> 8)..=(cx1 >> 8) {
                let m = self.means(level, ty as u32, tx as u32);
                if m.is_empty() {
                    continue;
                }
                for y in 0..TILE as u64 {
                    let cy = (ty << 8) + y;
                    if cy < cy0 || cy > cy1 {
                        continue;
                    }
                    for x in 0..TILE as u64 {
                        let cx = (tx << 8) + x;
                        if cx < cx0 || cx > cx1 {
                            continue;
                        }
                        let z = m[(y * TILE as u64 + x) as usize];
                        if z.is_finite() && z < 0.0 {
                            depths.push(-z);
                        }
                    }
                }
            }
        }
        if depths.len() < 10 {
            return None;
        }
        depths.sort_by(f32::total_cmp);
        let at = |p: f64| depths[((depths.len() - 1) as f64 * p).round() as usize];
        let (a, b) = (at(0.02), at(0.98));
        Some((a, b.max(a + 1.0)))
    }

    /// A writer adding soundings or grids (one import or build); the store's drawing goes
    /// on meanwhile and sees the new data once [`Writer::finish`] returns.
    pub fn writer(&self, kind: &str, name: &str, bytes: i64) -> Result<Writer> {
        let conn = Connection::open(&self.path)?;
        schema(&conn)?;
        conn.execute_batch("BEGIN")?;
        Ok(Writer { conn, kind: kind.into(), name: name.into(), bytes, dirty: HashSet::new(), pending: HashMap::new(), pending_n: 0, soundings: 0, seeded: Vec::new(), since_commit: 0, acc: Acc::new(), rejected: HashMap::new(), rejected_n: 0, left_dirty: HashSet::new() })
    }
}

/// Adds soundings or whole tiles to a [`Store`] (see [`Store::writer`]).
pub struct Writer {
    conn: Connection,
    kind: String,
    name: String,
    bytes: i64,
    /// Finest tiles changed (ty, tx), for the coarser levels.
    dirty: HashSet<(u32, u32)>,
    pending: HashMap<(u32, u32), Vec<(u16, Cell)>>,
    pending_n: usize,
    soundings: i64,
    seeded: Vec<(String, i64)>,
    since_commit: usize,
    acc: Acc,
    rejected: HashMap<(u32, u32), Vec<(u16, Cell)>>,
    rejected_n: i64,
    /// Left-out tiles changed other than by [`Writer::reject`] (put back by a second opinion).
    left_dirty: HashSet<(u32, u32)>,
}

const PENDING_MAX: usize = 4_000_000;

/// One tile being built: its cells, and which are in use (so only those are visited).
struct Acc {
    d: Vec<Cell>,
    touched: Vec<u16>,
}

impl Acc {
    fn new() -> Acc {
        Acc { d: vec![Cell::default(); TILE * TILE], touched: Vec::with_capacity(4096) }
    }
    #[inline]
    fn add(&mut self, i: usize, c: &Cell) {
        if c.n == 0 {
            return;
        }
        let e = &mut self.d[i];
        if e.n == 0 {
            self.touched.push(i as u16);
        }
        e.merge(c);
    }
    /// Add a stored tile: as it is, or (`quadrant` = Some((row, col))) halved into that
    /// quarter of a tile one level coarser.
    fn add_blob(&mut self, b: &[u8], quadrant: Option<(usize, usize)>) -> Result<()> {
        match quadrant {
            None => decode(b, |x, y, c| self.add(y * TILE + x, &c)),
            Some((qy, qx)) => decode(b, |x, y, c| self.add((qy * 128 + y / 2) * TILE + qx * 128 + x / 2, &c)),
        }
    }
    /// The tile as stored ([`encode`]'s format), and empty again.
    fn take(&mut self) -> (usize, Vec<u8>) {
        self.touched.sort_unstable();
        let mut raw = Vec::with_capacity(self.touched.len() * REC);
        for &i in &self.touched {
            let c = std::mem::take(&mut self.d[i as usize]);
            raw.push((i as usize % TILE) as u8);
            raw.push((i as usize / TILE) as u8);
            raw.extend_from_slice(&c.n.to_le_bytes());
            raw.extend_from_slice(&c.mean.to_le_bytes());
            raw.extend_from_slice(&c.m2.to_le_bytes());
            raw.extend_from_slice(&c.newest.to_le_bytes());
            raw.push(c.src);
        }
        let count = self.touched.len();
        self.touched.clear();
        let mut out = Vec::with_capacity(raw.len() / 2 + 16);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(count as u32).to_le_bytes());
        let mut z = flate2::write::ZlibEncoder::new(out, flate2::Compression::fast());
        let _ = z.write_all(&raw);
        (count, z.finish().unwrap_or_default())
    }
}

impl Writer {
    fn read_blob(&self, k: Key) -> Result<Option<Vec<u8>>> {
        self.read_in(SEABED, k)
    }

    fn read_in(&self, table: &str, k: Key) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row(&format!("SELECT data FROM {table} WHERE l = ?1 AND ty = ?2 AND tx = ?3"), params![k.0, k.1, k.2], |r| r.get(0)).optional()?)
    }

    fn write_blob(&mut self, k: Key, n: usize, blob: &[u8]) -> Result<()> {
        self.write_in(SEABED, k, n, blob)
    }

    fn write_in(&mut self, table: &str, k: Key, n: usize, blob: &[u8]) -> Result<()> {
        if n == 0 {
            self.conn.execute(&format!("DELETE FROM {table} WHERE l = ?1 AND ty = ?2 AND tx = ?3"), params![k.0, k.1, k.2])?;
        } else {
            self.conn.execute(&format!("INSERT OR REPLACE INTO {table} (l, ty, tx, n, data) VALUES (?1, ?2, ?3, ?4, ?5)"), params![k.0, k.1, k.2, n as i64, blob])?;
        }
        self.since_commit += 1;
        if self.since_commit >= 4000 {
            self.conn.execute_batch("COMMIT; BEGIN")?;
            self.since_commit = 0;
        }
        Ok(())
    }

    /// One sounding: height `z` (m, positive up) at a place.
    pub fn add(&mut self, lon: f64, lat: f64, z: f32, hours: u32, src: u8) -> Result<()> {
        if !z.is_finite() {
            return Ok(());
        }
        let Some((cx, cy)) = cell_of(lon, lat, MAX_LEVEL) else { return Ok(()) };
        self.add_cell(cx, cy, Cell::one(z, hours, src))
    }

    /// Cell statistics at finest-level cell (cx, cy).
    pub fn add_cell(&mut self, cx: u32, cy: u32, c: Cell) -> Result<()> {
        let idx = ((cy & 255) * TILE as u32 + (cx & 255)) as u16;
        self.pending.entry((cy >> 8, cx >> 8)).or_default().push((idx, c));
        self.pending_n += 1;
        self.soundings += c.n as i64;
        if self.pending_n >= PENDING_MAX {
            self.flush()?;
        }
        Ok(())
    }

    /// A whole finest-level tile at once (an imported grid): merged with what's there.
    pub fn add_tile(&mut self, ty: u32, tx: u32, cells: &[(u8, u8, Cell)]) -> Result<()> {
        let pend = self.pending.remove(&(ty, tx)).unwrap_or_default();
        self.pending_n -= pend.len();
        if let Some(b) = self.read_blob((MAX_LEVEL, ty, tx))? {
            self.acc.add_blob(&b, None)?;
        }
        for (i, c) in &pend {
            self.acc.add(*i as usize, c);
        }
        for (x, y, c) in cells {
            self.acc.add(*y as usize * TILE + *x as usize, c);
            self.soundings += c.n as i64;
        }
        let (n, blob) = self.acc.take();
        self.write_blob((MAX_LEVEL, ty, tx), n, &blob)?;
        self.dirty.insert((ty, tx));
        Ok(())
    }

    /// Whole finest-level tiles (an imported grid), merged with what's there on all cores.
    pub fn add_tiles(&mut self, tiles: Vec<(u32, u32, Vec<(u8, u8, Cell)>)>) -> Result<()> {
        use rayon::prelude::*;
        let mut work = Vec::with_capacity(tiles.len());
        for (ty, tx, cells) in tiles {
            let pend = self.pending.remove(&(ty, tx)).unwrap_or_default();
            self.pending_n -= pend.len();
            let old = self.read_blob((MAX_LEVEL, ty, tx))?;
            work.push(((ty, tx), old, pend, cells));
        }
        let built: Vec<Result<((u32, u32), usize, Vec<u8>, i64)>> = work
            .into_par_iter()
            .map_init(Acc::new, |acc, (k, old, pend, cells)| {
                if let Some(b) = &old {
                    acc.add_blob(b, None)?;
                }
                for (i, c) in &pend {
                    acc.add(*i as usize, c);
                }
                let mut n = 0i64;
                for (x, y, c) in &cells {
                    acc.add(*y as usize * TILE + *x as usize, c);
                    n += c.n as i64;
                }
                let (count, blob) = acc.take();
                Ok((k, count, blob, n))
            })
            .collect();
        for r in built {
            let (k, count, blob, n) = r?;
            self.write_blob((MAX_LEVEL, k.0, k.1), count, &blob)?;
            self.dirty.insert(k);
            self.soundings += n;
        }
        Ok(())
    }

    fn merge_into(&mut self, k: (u32, u32), cells: &[(u16, Cell)]) -> Result<()> {
        if let Some(b) = self.read_blob((MAX_LEVEL, k.0, k.1))? {
            self.acc.add_blob(&b, None)?;
        }
        for (i, c) in cells {
            self.acc.add(*i as usize, c);
        }
        let (n, blob) = self.acc.take();
        self.write_blob((MAX_LEVEL, k.0, k.1), n, &blob)?;
        self.dirty.insert(k);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        let mut p: Vec<_> = std::mem::take(&mut self.pending).into_iter().collect();
        p.sort_by_key(|(k, _)| *k);
        for (k, cells) in p {
            self.merge_into(k, &cells)?;
        }
        self.pending_n = 0;
        Ok(())
    }

    /// A sounding left out by the checks: kept apart (not in the seabed), to be shown.
    pub fn reject(&mut self, lon: f64, lat: f64, z: f32, hours: u32, src: u8) {
        if !z.is_finite() {
            return;
        }
        let Some((cx, cy)) = cell_of(lon, lat, MAX_LEVEL) else { return };
        self.reject_cell(cx, cy, Cell::one(z, hours, src));
    }

    /// A finest-level cell left out by the checks.
    pub fn reject_cell(&mut self, cx: u32, cy: u32, c: Cell) {
        let idx = ((cy & 255) * TILE as u32 + (cx & 255)) as u16;
        self.rejected.entry((cy >> 8, cx >> 8)).or_default().push((idx, c));
        self.rejected_n += c.n as i64;
    }

    /// Soundings left out so far.
    pub fn rejected(&self) -> i64 {
        self.rejected_n
    }

    /// Write the left-out soundings (finest level); returns the tiles changed.
    fn flush_rejected(&mut self) -> Result<HashSet<(u32, u32)>> {
        let mut p: Vec<_> = std::mem::take(&mut self.rejected).into_iter().collect();
        p.sort_by_key(|(k, _)| *k);
        let mut dirty = HashSet::new();
        for ((ty, tx), cells) in p {
            if let Some(b) = self.read_in(LEFT_OUT, (MAX_LEVEL, ty, tx))? {
                self.acc.add_blob(&b, None)?;
            }
            for (i, c) in &cells {
                self.acc.add(*i as usize, c);
            }
            let (n, blob) = self.acc.take();
            self.write_in(LEFT_OUT, (MAX_LEVEL, ty, tx), n, &blob)?;
            dirty.insert((ty, tx));
        }
        Ok(dirty)
    }

    /// Note a track as added (so building again skips it).
    pub fn mark_seeded(&mut self, track_id: &str, modified: i64) {
        self.seeded.push((track_id.to_string(), modified));
    }

    /// Soundings added so far.
    pub fn soundings(&self) -> i64 {
        self.soundings
    }

    /// Rebuild the levels coarser than `from` over its tiles `dirty` (ty, tx), on all cores.
    fn build_levels(&mut self, from: u8, dirty: HashSet<(u32, u32)>, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<()> {
        self.build_levels_in(SEABED, from, dirty, progress, cancel)
    }

    fn build_levels_in(&mut self, table: &str, from: u8, mut dirty: HashSet<(u32, u32)>, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<()> {
        use rayon::prelude::*;
        for level in (MIN_LEVEL..from).rev() {
            let mut parents: Vec<(u32, u32)> = dirty.iter().map(|&(y, x)| (y >> 1, x >> 1)).collect::<HashSet<_>>().into_iter().collect();
            parents.sort();
            let total = parents.len();
            for (ci, chunk) in parents.chunks(1024).enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(Error::Cancelled);
                }
                progress(&format!("Coarser levels: level {level}"), ci * 1024, total);
                // read the children (one connection), merge them on all cores, write
                let mut work: Vec<((u32, u32), Vec<((usize, usize), Vec<u8>)>)> = Vec::with_capacity(chunk.len());
                for &(py, px) in chunk {
                    let mut kids = Vec::with_capacity(4);
                    for (cy, cx) in [(0u32, 0u32), (0, 1), (1, 0), (1, 1)] {
                        if let Some(b) = self.read_in(table, (level + 1, py * 2 + cy, px * 2 + cx))? {
                            kids.push(((cy as usize, cx as usize), b));
                        }
                    }
                    work.push(((py, px), kids));
                }
                let built: Vec<Result<((u32, u32), usize, Vec<u8>)>> = work
                    .into_par_iter()
                    .map_init(Acc::new, |acc, (k, kids)| {
                        for (q, b) in &kids {
                            acc.add_blob(b, Some(*q))?;
                        }
                        let (n, blob) = acc.take();
                        Ok((k, n, blob))
                    })
                    .collect();
                for r in built {
                    let (k, n, blob) = r?;
                    self.write_in(table, (level, k.0, k.1), n, &blob)?;
                }
            }
            dirty = parents.into_iter().collect();
        }
        Ok(())
    }

    /// Write what's left, rebuild the coarser levels over what changed, and commit.
    /// `progress(stage, done, total)`. The coarser tiles are built on all cores.
    pub fn finish(mut self, progress: &mut dyn FnMut(&str, usize, usize), cancel: &AtomicBool) -> Result<i64> {
        self.flush()?;
        let mut left = self.flush_rejected()?;
        left.extend(std::mem::take(&mut self.left_dirty));
        let dirty: HashSet<(u32, u32)> = std::mem::take(&mut self.dirty);
        self.build_levels(MAX_LEVEL, dirty, progress, cancel)?;
        self.build_levels_in(LEFT_OUT, MAX_LEVEL, left, &mut |_, _, _| {}, cancel)?;
        for (id, m) in std::mem::take(&mut self.seeded) {
            self.conn.execute("INSERT OR REPLACE INTO seeded (track_id, modified) VALUES (?1, ?2)", params![id, m])?;
        }
        if !self.kind.is_empty() && (self.soundings > 0 || self.rejected_n > 0) {
            self.conn.execute("INSERT INTO sources (kind, name, bytes, added, soundings, rejected) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![self.kind, self.name, self.bytes, now_s(), self.soundings, self.rejected_n])?;
        }
        self.conn.execute_batch("COMMIT")?;
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
        Ok(self.soundings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_round_trip_and_timezero_cells() {
        for lat in [-80.0, -41.27, -0.1, 0.0, 12.5, 60.0] {
            let y = merc_y(lat);
            assert!((merc_lat(y) - lat).abs() < 1e-10, "{lat}");
        }
        // TimeZero's XYZ export puts points on the cells' top-left corners: they are whole
        // cells in ellipsoidal Mercator (lines from Chris's timezero.xyz)
        for (lon, lat) in [(171.991846561432, -40.1582038082353), (171.99193239212, -40.1582202721742), (171.992018222809, -40.1583025918085)] {
            let (u, v) = to_world(lon, lat);
            let n = cells_across(16) as f64;
            assert!(((u * n) - (u * n).round()).abs() < 1e-4, "x {}", u * n);
            assert!(((v * n) - (v * n).round()).abs() < 1e-4, "y {}", v * n);
        }
        let (cx, cy) = cell_of(171.991846561432 + 1e-7, -40.1582038082353 - 1e-7, 16).unwrap();
        assert_eq!((cx, cy), (16404009, 10423809));
        let c = cell_centre(cx, cy, 16);
        assert!((c.0 - 171.99186).abs() < 1e-4 && (c.1 + 40.15821).abs() < 1e-4, "{c:?}");
        assert!((cell_size_m(16, -41.0) - 1.80).abs() < 0.01);
    }

    #[test]
    fn merging_cells_matches_all_at_once() {
        let v = [-30.0f32, -31.5, -29.0, -30.5, -33.0, -28.0];
        let mut a = Cell::default();
        for &z in &v[..2] {
            a.merge(&Cell::one(z, 10, SRC_OWN));
        }
        let mut b = Cell::default();
        for &z in &v[2..] {
            b.merge(&Cell::one(z, 20, SRC_PBG));
        }
        a.merge(&b);
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let var = v.iter().map(|z| (z - mean).powi(2)).sum::<f32>() / v.len() as f32;
        assert_eq!(a.n, 6);
        assert!((a.mean - mean).abs() < 1e-5);
        assert!((a.spread() - var.sqrt()).abs() < 1e-4);
        assert_eq!((a.newest, a.src), (20, SRC_OWN | SRC_PBG));
    }

    #[test]
    fn main_area_skips_strays_and_spans_180() {
        let dir = std::env::temp_dir().join(format!("bathy-area-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "t", 0).unwrap();
        // the Chatham Rise either side of 180°, and one stray fix at 0° 0°
        for i in 0..400 {
            w.add(179.0 + i as f64 * 0.005, -43.5, -400.0, 1, SRC_OWN).unwrap();
        }
        w.add(0.0, 0.0, -50.0, 1, SRC_OWN).unwrap();
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        let b = s.info().unwrap().bounds.unwrap();
        assert!(b[0] > 178.5 && b[0] < 179.1 && b[2] > 180.9 && b[2] < 181.5, "{b:?}");
        assert!(b[1] < -43.5 && b[3] > -43.5 && b[3] - b[1] < 1.0, "{b:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_stores_get_the_coarse_levels() {
        let dir = std::env::temp_dir().join(format!("bathy-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "t", 0).unwrap();
        for i in 0..50 {
            w.add(173.2 + i as f64 * 0.001, -41.2, -30.0, 1, SRC_OWN).unwrap();
        }
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        // as an older TweedPlot left it: nothing below level 6
        s.db().execute("DELETE FROM tiles WHERE l < 6", []).unwrap();
        s.ensure_levels().unwrap();
        let b: Vec<u8> = s.db().query_row("SELECT data FROM tiles WHERE l = 3", [], |r| r.get(0)).unwrap();
        let mut n = 0;
        decode(&b, |_, _, c| n += c.n).unwrap();
        assert_eq!(n, 50);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn left_out_from_the_first_checks_moves_over() {
        let dir = std::env::temp_dir().join(format!("bathy-lo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "t", 0).unwrap();
        for i in 0..30 {
            w.reject(173.2 + i as f64 * 0.001, -41.2, -70.0, 1, SRC_OWN);
        }
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        // as the first checks stored them: finest tiles in `rejected`, no levels
        s.db().execute_batch("INSERT INTO rejected (ty, tx, n, data) SELECT ty, tx, n, data FROM left_out WHERE l = 16; DELETE FROM left_out;").unwrap();
        assert_eq!(s.info().unwrap().rejected, 0);
        s.ensure_levels().unwrap();
        assert_eq!(s.info().unwrap().rejected, 30);
        let k: i64 = s.db().query_row("SELECT count(*) FROM left_out WHERE l = 3", [], |r| r.get(0)).unwrap();
        assert_eq!(k, 1);
        let r: i64 = s.db().query_row("SELECT count(*) FROM rejected", [], |r| r.get(0)).unwrap();
        assert_eq!(r, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_opinion_puts_back_what_neighbours_agree_with() {
        let dir = std::env::temp_dir().join(format!("bathy-2nd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "t", 0).unwrap();
        // a canyon wall at ~600 m sounded on one line, and two left-out soundings beside it:
        // one at 605 m (the chart was wrong) and one at 70 m (the sounder was)
        for i in 0..40 {
            w.add(173.2 + i as f64 * 0.00003, -41.2, -600.0, 1, SRC_OWN).unwrap();
        }
        w.reject(173.2 + 20.0 * 0.00003, -41.20002, -605.0, 1, SRC_OWN);
        w.reject(173.2 + 10.0 * 0.00003, -41.20002, -70.0, 1, SRC_OWN);
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(s.info().unwrap().rejected, 2);
        let back = s.second_opinion(None, &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(back, 1);
        let i = s.info().unwrap();
        assert_eq!(i.rejected, 1);
        assert_eq!(i.sources.len(), 1); // no source row for it
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_channel_deeper_than_the_chart_vouched_for_by_other_days() {
        let dir = std::env::temp_dir().join(format!("bathy-chan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        // charted 200–500 m; a narrow canyon at ~950 m crossed on three days, all left out,
        // and a shallow lock-on at 80 m on three days, left out too
        let chart = check::Reference::new(|b| {
            let mut g = check::RefGrid::new(b, check::REF_STEP);
            g.paint(1, 200.0, Some(500.0), &[vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]]);
            g
        });
        let mut w = s.writer("own", "t", 0).unwrap();
        for (k, day) in [10u32, 40, 90].iter().enumerate() {
            let h = day * 24;
            w.reject(173.2 + k as f64 * 0.00003, -41.2, -950.0 - k as f32, h, SRC_OWN | WHY_DEEP);
            w.reject(173.3 + k as f64 * 0.00003, -41.2, -80.0, h, SRC_OWN);
        }
        // one more deep one on a single day only: nobody else's
        w.reject(173.4, -41.2, -1000.0, 5 * 24, SRC_OWN | WHY_DEEP);
        w.reject(173.40003, -41.2, -1001.0, 5 * 24 + 1, SRC_OWN | WHY_DEEP);
        // and one the chart rule now lets in by itself (deeper than charted, not absurdly)
        w.reject(173.5, -41.2, -620.0, 7 * 24, SRC_OWN | WHY_DEEP);
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(s.info().unwrap().rejected, 9);
        let back = s.second_opinion(Some(&chart), &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(back, 4, "the canyon and the 620 m one");
        assert_eq!(s.info().unwrap().rejected, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn channel_crossings_far_apart_vouch_for_each_other() {
        let dir = std::env::temp_dir().join(format!("bathy-cross-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let chart = check::Reference::new(|b| {
            let mut g = check::RefGrid::new(b, check::REF_STEP);
            g.paint(1, 200.0, Some(500.0), &[vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]]);
            g
        });
        // a channel along 41.2° S crossed on three days, 100 m apart along it
        let mut w = s.writer("own", "t", 0).unwrap();
        for (k, day) in [3u32, 30, 300].iter().enumerate() {
            for j in 0..5 {
                w.reject(173.2 + k as f64 * 0.0012, -41.2 + j as f64 * 0.00002, -640.0 - k as f32 * 3.0, day * 24, SRC_OWN);
            }
        }
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        let back = s.second_opinion(Some(&chart), &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(back, 15);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cursor_snaps_to_the_nearest_sounding() {
        let dir = std::env::temp_dir().join(format!("bathy-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "t", 0).unwrap();
        w.add(173.2, -41.2, -300.0, 1, SRC_OWN).unwrap();
        w.add(173.2003, -41.2, -310.0, 1, SRC_OWN).unwrap(); // 25 m east
        w.reject(173.2, -41.2006, -80.0, 1, SRC_OWN | WHY_SHALLOW); // 67 m south
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        s.changed();
        // 10 m off: nothing within 4 m, the first within 15 m
        let p = (173.2 + 0.00012, -41.2);
        assert!(s.sounding_at(p.0, p.1).is_none());
        let n = s.sounding_near(p.0, p.1, 15.0).unwrap();
        assert!((n.z + 300.0).abs() < 1e-3 && n.off_m < 12.0, "{n:?}");
        // a wide snap (zoomed out) finds the left-out one too, at a coarser level
        assert!(s.left_out_near(173.2, -41.2, 20.0).is_none());
        let l = s.left_out_near(173.2, -41.2, 150.0).unwrap();
        assert!((l.z + 80.0).abs() < 1e-3 && l.src & WHY_SHALLOW != 0, "{l:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn running_sources_and_settings() {
        let dir = std::env::temp_dir().join(format!("bathy-tally-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        s.tally_source("Live", "2026-10-30", 100, 3).unwrap();
        s.tally_source("Live", "2026-10-30", 50, 1).unwrap();
        s.tally_source("Live", "2026-10-31", 7, 0).unwrap();
        let src = s.info().unwrap().sources;
        assert_eq!(src.len(), 2);
        assert_eq!((src[0].soundings, src[0].rejected), (150, 4));
        assert_eq!(s.meta("live"), None);
        s.set_meta("live", "1").unwrap();
        assert_eq!(s.meta("live").as_deref(), Some("1"));
        s.mark_seeded(&[("t9".into(), -1)]).unwrap();
        assert_eq!(s.seeded().unwrap().get("t9"), Some(&-1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tiles_round_trip() {
        let mut d = Dense::new();
        d.0[0] = Cell::one(-5.0, 1, SRC_OWN);
        d.0[255 * TILE + 254] = Cell { n: 7, mean: -1200.5, m2: 3.0, newest: 99, src: SRC_XYZ };
        let (n, b) = encode(&d);
        assert_eq!(n, 2);
        let mut got = Vec::new();
        decode(&b, |x, y, c| got.push((x, y, c))).unwrap();
        assert_eq!(got, vec![(0, 0, d.0[0]), (254, 255, d.0[255 * TILE + 254])]);
    }

    #[test]
    fn store_adds_and_builds_levels() {
        let dir = std::env::temp_dir().join(format!("bathy-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "test", 0).unwrap();
        // a line of soundings off Nelson, 20 m to 40 m, and one sounding twice
        for i in 0..200 {
            w.add(173.2 + i as f64 * 0.0001, -41.2, -20.0 - i as f32 * 0.1, 500_000, SRC_OWN).unwrap();
        }
        w.add(173.2, -41.2, -22.0, 500_001, SRC_OWN).unwrap();
        w.mark_seeded("t1", 5);
        assert_eq!(w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap(), 201);
        s.changed();
        let p = s.sounding_at(173.2, -41.2).unwrap();
        assert_eq!(p.n, 2);
        assert!((p.z + 21.0).abs() < 1e-4 && (p.spread - 1.0).abs() < 1e-4, "{p:?}");
        let info = s.info().unwrap();
        assert!(info.cells >= 190 && info.cells <= 201, "{}", info.cells);
        let b = info.bounds.unwrap();
        assert!(b[0] <= 173.2 && b[2] >= 173.22 && b[1] <= -41.2 && b[3] >= -41.2, "{b:?}");
        assert_eq!(info.sources.len(), 1);
        assert_eq!(s.seeded().unwrap().get("t1"), Some(&5));
        // every coarser level has the soundings, all 201 of them
        let db = s.db();
        for l in MIN_LEVEL..=MAX_LEVEL {
            let mut q = db.prepare("SELECT data FROM tiles WHERE l = ?1").unwrap();
            let blobs: Vec<Vec<u8>> = q.query_map([l], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
            let mut n = 0;
            for b in &blobs {
                decode(b, |_, _, c| n += c.n).unwrap();
            }
            assert_eq!(n, 201, "level {l}");
        }
        drop(db);
        let r = s.depth_range([173.19, -41.21, 173.23, -41.19]).unwrap();
        assert!(r.0 >= 20.0 && r.0 < 21.0 && r.1 > 39.0 && r.1 <= 40.0, "{r:?}");
        s.clear().unwrap();
        assert_eq!(s.info().unwrap().cells, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
