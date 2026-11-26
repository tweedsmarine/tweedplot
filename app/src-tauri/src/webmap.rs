//! Online map layers: OpenStreetMap (a base map that replaces the bundled land) and
//! OpenSeaMap (seamarks over the charts), or tile servers of the user's own.
//!
//! These are the only map requests that can leave the computer, and they all pass through
//! `load`, which does nothing at all unless the user has turned online maps on (Chart ›
//! Layers). Off is the default, and every start begins off until the page says otherwise.
//! Tiles are fetched here, not by the WebView, so that the requests carry a User-Agent that
//! names TweedPlot (the OpenStreetMap tile policy asks for one), are cached on disk, and can
//! be switched off in one place.

use serde::Deserialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::http::{header, Response, StatusCode};

const OSM_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
const SEA_URL: &str = "https://tiles.openseamap.org/seamark/{z}/{x}/{y}.png";
const FRESH: Duration = Duration::from_secs(14 * 24 * 3600);
const MAX_CACHE_BYTES: u64 = 500_000_000;
const WORKERS: usize = 4;
/// Tiles waiting for a worker: beyond this the oldest (long off the screen while panning) are dropped.
const MAX_QUEUE: usize = 192;

static ENABLED: AtomicBool = AtomicBool::new(false);
static CACHED_ONLY: AtomicBool = AtomicBool::new(false);
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
static CONFIG: RwLock<Urls> = RwLock::new(Urls { osm: String::new(), sea: String::new() });
/// Per server (OpenStreetMap, OpenSeaMap): one refusing doesn't stop the other.
static BACKOFF_UNTIL: Mutex<[Option<Instant>; 2]> = Mutex::new([None, None]);
/// No way through to the internet (a request failed before any server answered: a boat network
/// with no connection out, say): until this time no tile server is asked, the saved tiles are used.
static OFFLINE_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
const OFFLINE_RETRY: Duration = Duration::from_secs(120);

fn offline() -> bool {
    OFFLINE_UNTIL.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| Instant::now() < t)
}
/// Requests made to a tile server since the program started (tests check it stays 0 while off).
static FETCHES: AtomicUsize = AtomicUsize::new(0);
static WRITES: AtomicUsize = AtomicUsize::new(0);
static QUEUE: OnceLock<(Mutex<VecDeque<(String, tauri::UriSchemeResponder)>>, Condvar)> = OnceLock::new();

struct Urls {
    osm: String,
    sea: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Osm,
    Sea,
}

impl Kind {
    /// The cache folder's prefix. OpenStreetMap tiles are kept with their sea already made
    /// transparent ("osmk"), so a cached tile is served as it is.
    fn tag(self) -> &'static str {
        match self {
            Kind::Osm => "osmk",
            Kind::Sea => "osea",
        }
    }
    fn slot(self) -> usize {
        match self {
            Kind::Osm => 0,
            Kind::Sea => 1,
        }
    }
    fn max_zoom(self) -> u8 {
        match self {
            Kind::Osm => 19,
            Kind::Sea => 18,
        }
    }
}

enum Tile {
    Image(Vec<u8>),
    Blank,
    /// Nothing to show because the server could not be reached: answered with an error so the map asks again later.
    Failed,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebMapConfig {
    pub enabled: bool,
    pub cached_only: bool,
    pub osm_url: String,
    pub sea_url: String,
}

/// Where tiles are kept. Called once at start; the folder is made when the first tile is saved.
pub fn init(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

/// A tile server address needs http(s) and the three place-holders.
fn valid_template(t: &str) -> bool {
    (t.starts_with("https://") || t.starts_with("http://"))
        && ["{z}", "{x}", "{y}"].iter().all(|p| t.contains(p))
        && !t.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn fill(template: &str, z: u8, x: u32, y: u32) -> String {
    template.replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
}

/// `/osm/<z>/<x>/<y>.png` or `/osea/<z>/<x>/<y>.png`, with a tile that exists at that zoom.
fn parse_path(path: &str) -> Option<(Kind, u8, u32, u32)> {
    let p = path.trim_start_matches('/');
    let (kind, rest) = if let Some(r) = p.strip_prefix("osm/") {
        (Kind::Osm, r)
    } else {
        let r = p.strip_prefix("osea/")?;
        (Kind::Sea, r)
    };
    let rest = rest.strip_suffix(".png").unwrap_or(rest);
    let mut it = rest.split('/');
    let z: u8 = it.next()?.parse().ok()?;
    let x: u32 = it.next()?.parse().ok()?;
    let y: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || z > kind.max_zoom() {
        return None;
    }
    let n = 1u64 << z;
    (u64::from(x) < n && u64::from(y) < n).then_some((kind, z, x, y))
}

pub fn is_web_path(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    p.starts_with("osm/") || p.starts_with("osea/")
}

fn template(kind: Kind) -> String {
    let c = CONFIG.read().unwrap_or_else(|e| e.into_inner());
    let (custom, default) = match kind {
        Kind::Osm => (&c.osm, OSM_URL),
        Kind::Sea => (&c.sea, SEA_URL),
    };
    if custom.is_empty() {
        default.to_string()
    } else {
        custom.clone()
    }
}

/// FNV-1a: a stable short name for a tile server address, so that changing the address
/// doesn't show the old server's tiles.
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", h & 0xffff_ffff)
}

fn cache_file(dir: &Path, kind: Kind, template: &str, z: u8, x: u32, y: u32) -> PathBuf {
    dir.join(format!("{}-{}", kind.tag(), short_hash(template))).join(z.to_string()).join(x.to_string()).join(format!("{y}.tile"))
}

fn image_type(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if b.starts_with(&[0xFF, 0xD8]) {
        Some("image/jpeg")
    } else {
        None
    }
}

fn client() -> Option<&'static reqwest::blocking::Client> {
    static C: OnceLock<Option<reqwest::blocking::Client>> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::blocking::Client::builder()
            // the OSM tile policy asks for a name of the app's own and optionally a contact URL or email:
            // the URL only (the site carries a contact address), so no personal address goes to every tile server
            .user_agent(concat!("TweedPlot/", env!("CARGO_PKG_VERSION"), " (marine chart plotter; +https://tweedsmarine.nz)"))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .build()
            .ok()
    })
    .as_ref()
}

fn back_off(kind: Kind, secs: u64) {
    BACKOFF_UNTIL.lock().unwrap_or_else(|e| e.into_inner())[kind.slot()] = Some(Instant::now() + Duration::from_secs(secs));
}

fn backing_off(kind: Kind) -> bool {
    BACKOFF_UNTIL.lock().unwrap_or_else(|e| e.into_inner())[kind.slot()].is_some_and(|t| Instant::now() < t)
}

/// The one place that talks to a tile server. Ok(None): the server has no tile there.
fn fetch(kind: Kind, url: &str) -> Result<Option<Vec<u8>>, String> {
    if !ENABLED.load(Ordering::SeqCst) || CACHED_ONLY.load(Ordering::SeqCst) {
        return Err("online maps are off".into());
    }
    let c = client().ok_or("no HTTP client")?;
    FETCHES.fetch_add(1, Ordering::SeqCst);
    let r = c.get(url).send().map_err(|e| {
        // nothing answered: no internet here. Stop asking for a while rather than wait on every tile
        *OFFLINE_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + OFFLINE_RETRY);
        e.to_string()
    })?;
    *OFFLINE_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let status = r.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        // blocked or overloaded (403, 429, 5xx): leave the server alone for a while
        back_off(kind, if status.as_u16() == 403 || status.as_u16() == 429 { 600 } else { 60 });
        return Err(format!("tile server answered {status}"));
    }
    let bytes = r.bytes().map_err(|e| e.to_string())?;
    if bytes.len() > 2_000_000 || image_type(&bytes).is_none() {
        return Err("not a PNG or JPEG tile".into());
    }
    Ok(Some(bytes.to_vec()))
}

fn save(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let tmp = path.with_extension("part");
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    if WRITES.fetch_add(1, Ordering::Relaxed) % 2000 == 1999 {
        // walking the whole cache takes a while: not on a tile worker
        if let Some(dir) = CACHE_DIR.get().cloned() {
            std::thread::spawn(move || prune(&dir));
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<(SystemTime, u64, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        match e.metadata() {
            Ok(m) if m.is_dir() => walk(&p, out),
            Ok(m) => out.push((m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), p)),
            Err(_) => {}
        }
    }
}

/// Keep the cache under its limit, deleting the oldest tiles first.
fn prune(dir: &Path) {
    let mut files = Vec::new();
    walk(dir, &mut files);
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= MAX_CACHE_BYTES {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= MAX_CACHE_BYTES * 9 / 10 {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// OpenStreetMap tiles with their sea made transparent, so the charts and depth colours show through
/// and only OSM's land is drawn over them. Tiles that are not plain 8-bit PNGs are passed through.
fn key_water(bytes: &[u8]) -> Option<Vec<u8>> {
    const WATER: [i32; 3] = [170, 211, 223];
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let step = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => return None,
    };
    let (w, h) = (info.width as usize, info.height as usize);
    let water: Vec<bool> = buf[..info.buffer_size()].chunks_exact(step).map(|px| (0..3).all(|i| (px[i] as i32 - WATER[i]).abs() <= 10)).collect();
    // Water reaching the tile edge is taken to be sea and made transparent. Enclosed water (lakes, ponds,
    // wide rivers) stays opaque: it is OSM's own and must not expose the depth shading under the land.
    let mut sea = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for x in 0..w {
        stack.extend([x, (h - 1) * w + x]);
    }
    for y in 0..h {
        stack.extend([y * w, y * w + w - 1]);
    }
    while let Some(i) = stack.pop() {
        if !water[i] || sea[i] {
            continue;
        }
        sea[i] = true;
        let (x, y) = (i % w, i / w);
        if x > 0 {
            stack.push(i - 1);
        }
        if x + 1 < w {
            stack.push(i + 1);
        }
        if y > 0 {
            stack.push(i - w);
        }
        if y + 1 < h {
            stack.push(i + w);
        }
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for (i, px) in buf[..info.buffer_size()].chunks_exact(step).enumerate() {
        let a = if sea[i] {
            0
        } else if step == 4 {
            px[3]
        } else {
            255
        };
        rgba.extend_from_slice(&[px[0], px[1], px[2], a]);
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, info.width, info.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().ok()?.write_image_data(&rgba).ok()?;
    Some(out)
}

/// A tile as it is served and cached: OpenStreetMap's with the sea made transparent.
fn prepare(kind: Kind, bytes: Vec<u8>) -> Vec<u8> {
    match kind {
        Kind::Osm => key_water(&bytes).unwrap_or(bytes),
        Kind::Sea => bytes,
    }
}

fn load(kind: Kind, z: u8, x: u32, y: u32) -> Tile {
    if !ENABLED.load(Ordering::SeqCst) {
        return Tile::Blank;
    }
    let template = template(kind);
    let file = CACHE_DIR.get().map(|d| cache_file(d, kind, &template, z, x, y));
    let cached = file.as_ref().and_then(|f| {
        let age = std::fs::metadata(f).ok()?.modified().ok()?.elapsed().unwrap_or_default();
        Some((std::fs::read(f).ok()?, age))
    });
    let tile_of = |b: Vec<u8>| if b.is_empty() { Tile::Blank } else { Tile::Image(b) };
    if let Some((bytes, age)) = &cached {
        if *age < FRESH {
            return tile_of(bytes.clone());
        }
    }
    let stale = || cached.clone().map_or(Tile::Failed, |(b, _)| tile_of(b));
    if CACHED_ONLY.load(Ordering::SeqCst) {
        return cached.map_or(Tile::Blank, |(b, _)| tile_of(b));
    }
    if backing_off(kind) || offline() {
        return stale();
    }
    match fetch(kind, &fill(&template, z, x, y)) {
        Ok(Some(bytes)) => {
            let bytes = prepare(kind, bytes);
            if let Some(f) = &file {
                save(f, &bytes);
            }
            Tile::Image(bytes)
        }
        Ok(None) => {
            // remembered, so the server isn't asked again for the same empty tile
            if let Some(f) = &file {
                save(f, &[]);
            }
            Tile::Blank
        }
        Err(e) => {
            if !backing_off(kind) {
                back_off(kind, 20);
            }
            eprintln!("web map tile: {e}");
            stale()
        }
    }
}

fn response(tile: Tile) -> Response<Vec<u8>> {
    let (ct, body) = match tile {
        Tile::Image(b) => (image_type(&b).unwrap_or("image/png"), b),
        Tile::Blank => ("image/png", crate::clear_png().to_vec()),
        Tile::Failed => {
            return Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::CACHE_CONTROL, "no-store")
                .body(Vec::new())
                .expect("valid response");
        }
    };
    Response::builder()
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_TYPE, ct)
        .body(body)
        .expect("valid response")
}

/// Answer a `chart://…/osm/…` or `/osea/…` request. Off: answered at once, nothing queued.
pub fn serve(path: String, responder: tauri::UriSchemeResponder) {
    if !ENABLED.load(Ordering::SeqCst) {
        responder.respond(response(Tile::Blank));
        return;
    }
    let Some(_) = parse_path(&path) else {
        responder.respond(Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()).expect("valid response"));
        return;
    };
    let q = QUEUE.get_or_init(|| {
        for i in 0..WORKERS {
            let _ = std::thread::Builder::new().name(format!("webmap-{i}")).spawn(move || loop {
                let (path, responder) = {
                    let q = QUEUE.get().expect("queue");
                    let mut jobs = q.0.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        // newest first: the tiles on screen now matter most
                        if let Some(j) = jobs.pop_back() {
                            break j;
                        }
                        jobs = q.1.wait(jobs).unwrap_or_else(|e| e.into_inner());
                    }
                };
                let tile = std::panic::catch_unwind(|| match parse_path(&path) {
                    Some((k, z, x, y)) => load(k, z, x, y),
                    None => Tile::Blank,
                })
                .unwrap_or(Tile::Blank);
                responder.respond(response(tile));
            });
        }
        (Mutex::new(VecDeque::new()), Condvar::new())
    });
    let dropped = {
        let mut jobs = q.0.lock().unwrap_or_else(|e| e.into_inner());
        jobs.push_back((path, responder));
        let over = jobs.len().saturating_sub(MAX_QUEUE);
        jobs.drain(..over).collect::<Vec<_>>()
    };
    q.1.notify_one();
    // answered "not now" (not cached as blank), so the map asks again if they come back into view
    for (_, r) in dropped {
        r.respond(response(Tile::Failed));
    }
}

/// Turn online maps on or off and set the tile servers ("" = the standard one).
#[tauri::command]
pub fn webmap_set(cfg: WebMapConfig) -> Result<(), String> {
    for (name, u) in [("OpenStreetMap", &cfg.osm_url), ("OpenSeaMap", &cfg.sea_url)] {
        if !u.is_empty() && !valid_template(u) {
            return Err(format!("The {name} tile server address must start with http:// or https:// and contain {{z}}, {{x}} and {{y}}"));
        }
    }
    {
        let mut c = CONFIG.write().unwrap_or_else(|e| e.into_inner());
        c.osm = cfg.osm_url;
        c.sea = cfg.sea_url;
    }
    CACHED_ONLY.store(cfg.cached_only, Ordering::SeqCst);
    ENABLED.store(cfg.enabled, Ordering::SeqCst);
    if cfg.enabled {
        if let Some(dir) = CACHE_DIR.get().cloned() {
            std::thread::spawn(move || prune(&dir));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn webmap_status() -> serde_json::Value {
    let mut files = Vec::new();
    if let Some(d) = CACHE_DIR.get() {
        walk(d, &mut files);
    }
    serde_json::json!({
        "enabled": ENABLED.load(Ordering::SeqCst),
        // no internet: only saved tiles are shown, the servers asked again every two minutes
        "offline": offline(),
        "tiles": files.len(),
        "bytes": files.iter().map(|f| f.1).sum::<u64>(),
    })
}

#[tauri::command]
pub fn webmap_clear_cache() -> Result<(), String> {
    match CACHE_DIR.get() {
        Some(d) if d.exists() => std::fs::remove_dir_all(d).map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(parse_path("/osm/5/3/2.png"), Some((Kind::Osm, 5, 3, 2)));
        assert_eq!(parse_path("/osea/18/0/262143.png"), Some((Kind::Sea, 18, 0, 262143)));
        assert_eq!(parse_path("/osea/19/0/0.png"), None, "OpenSeaMap stops at zoom 18");
        assert_eq!(parse_path("/osm/3/8/0.png"), None, "x off the world");
        assert_eq!(parse_path("/osm/3/1/1/1.png"), None);
        assert_eq!(parse_path("/5/3/2.pbf"), None);
        assert!(is_web_path("/osm/1/0/0.png") && is_web_path("/osea/1/0/0.png") && !is_web_path("/hires/1/0/0.png"));
    }

    #[test]
    fn templates() {
        assert!(valid_template(OSM_URL) && valid_template(SEA_URL));
        assert!(valid_template("http://tiles.local:8080/{z}/{x}/{y}.png?key=abc"));
        assert!(!valid_template("file:///c:/tiles/{z}/{x}/{y}.png"));
        assert!(!valid_template("https://example.org/{z}/{x}.png"));
        assert!(!valid_template("https://example.org/{z}/{x}/{y}.png and more"));
        assert_eq!(fill(OSM_URL, 5, 3, 2), "https://tile.openstreetmap.org/5/3/2.png");
    }

    /// A server that refuses is left alone for a while; the other one isn't.
    #[test]
    fn back_off_is_per_server() {
        back_off(Kind::Osm, 600);
        assert!(backing_off(Kind::Osm));
        assert!(!backing_off(Kind::Sea));
        BACKOFF_UNTIL.lock().unwrap_or_else(|e| e.into_inner())[Kind::Osm.slot()] = None;
    }

    /// Off sends no request, and with "cached only" a cached tile is served without one.
    #[test]
    fn off_makes_no_requests() {
        ENABLED.store(false, Ordering::SeqCst);
        let before = FETCHES.load(Ordering::SeqCst);
        assert!(matches!(load(Kind::Osm, 5, 3, 2), Tile::Blank));
        assert!(fetch(Kind::Osm, "https://tile.openstreetmap.org/5/3/2.png").is_err());
        assert_eq!(FETCHES.load(Ordering::SeqCst), before);

        let dir = std::env::temp_dir().join(format!("tweedplot-webmap-test-{}", std::process::id()));
        init(dir.clone());
        {
            let mut c = CONFIG.write().unwrap_or_else(|e| e.into_inner());
            c.osm = "http://127.0.0.1:9/{z}/{x}/{y}.png".into();
        }
        let f = cache_file(&dir, Kind::Osm, &template(Kind::Osm), 5, 3, 2);
        save(&f, &[0x89, b'P', b'N', b'G', 1, 2, 3]);
        ENABLED.store(true, Ordering::SeqCst);
        CACHED_ONLY.store(true, Ordering::SeqCst);
        assert!(matches!(load(Kind::Osm, 5, 3, 2), Tile::Image(b) if b.len() == 7));
        assert!(matches!(load(Kind::Osm, 5, 3, 3), Tile::Blank), "not cached: blank, no request");
        assert_eq!(FETCHES.load(Ordering::SeqCst), before);
        ENABLED.store(false, Ordering::SeqCst);
        CACHED_ONLY.store(false, Ordering::SeqCst);
        CONFIG.write().unwrap_or_else(|e| e.into_inner()).osm.clear();
        let _ = std::fs::remove_dir_all(dir);
    }
}
