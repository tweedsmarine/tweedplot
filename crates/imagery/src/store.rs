//! The user's images: a folder of image files (copied in on import, or downloaded) and
//! `images.json` listing them with how they are shown. An image can also be catalogued where
//! it is (`linked`): a chart folder another program already uses, or an external drive. Then
//! the entry holds the file's full path, TweedPlot never copies, moves or deletes it, and the
//! image is reported `missing` while the file can't be found (a drive not plugged in).
//! Recently drawn images are kept open (a KAP is read into memory whole).
//!
//! Besides KAP charts and GeoTIFFs: MBTiles (ready-made tiles, passed through), and pictures
//! (PNG, JPEG, plain TIFF) placed by a world file or by a calibration (control points,
//! calib.rs). A PNG or JPEG is always kept as a tiled TIFF in the images folder.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use lru::LruCache;
use serde::{Deserialize, Serialize};

use crate::calib::{Calib, Calibrated, FitReport};
use crate::geo::{Affine, Crs};
use crate::kap::Kap;
use crate::mbtiles::MbTiles;
use crate::render::{native_zoom, Raster};
use crate::source::FileSource;
use crate::tiff::Tiff;
use crate::{err, Error, Result};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// "kap", "geotiff" or "mbtiles".
    pub kind: String,
    /// File name inside the images folder, or the file's full path when `linked`.
    pub file: String,
    /// Catalogued where it is rather than copied in: TweedPlot reads the file there and never
    /// deletes it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub linked: bool,
    /// A linked file that can't be found now (worked out when the list is read, not saved).
    #[serde(default, skip_deserializing, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
    /// Where it came from: "KAP chart", "GeoTIFF", "Sentinel-2".
    pub source: String,
    /// The image's date (the scene's date for satellite images, the edition for charts).
    #[serde(default)]
    pub date: String,
    /// Credit to show on the map while it is visible.
    #[serde(default)]
    pub credit: String,
    pub bounds: [f64; 4],
    pub width: usize,
    pub height: usize,
    /// Ground metres per pixel.
    pub res_m: f64,
    /// The zoom with one image pixel to a screen pixel.
    pub native_zoom: u8,
    /// For KAP charts: how well the chart's reference points fit (RMS, pixels).
    #[serde(default)]
    pub fit_rms_px: Option<f64>,
    #[serde(default)]
    pub scale: Option<u64>,
    pub visible: bool,
    pub opacity: f64,
    /// "under" (over the depth shading, under the chart's lines and land) or "over".
    pub placement: String,
    pub added: String,
    /// Placed by a world file: its EPSG code (0 while the coordinate system isn't known) and
    /// the affine [a, b, c, d, e, f] from pixel corners to map coordinates.
    #[serde(default)]
    pub georef: Option<WorldGeo>,
    /// The calibration in use (control points), if the image has been calibrated.
    #[serde(default)]
    pub calibration: Option<Calib>,
    /// A calibration being worked on and not yet saved.
    #[serde(default)]
    pub draft: Option<Calib>,
    /// MBTiles: the first zoom with tiles.
    #[serde(default)]
    pub min_zoom: Option<u8>,
    /// How well the calibration's points agree (RMS, ground metres).
    #[serde(default)]
    pub calib_rms_m: Option<f64>,
    /// For the page: "ready", "calibrate" (no position yet) or "crs" (the world file's
    /// coordinate system is needed). Worked out when listed.
    #[serde(default, skip_deserializing)]
    pub state: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WorldGeo {
    pub epsg: u32,
    pub aff: [f64; 6],
}

impl WorldGeo {
    fn affine(&self) -> Affine {
        let a = self.aff;
        Affine { a: a[0], b: a[1], c: a[2], d: a[3], e: a[4], f: a[5] }
    }
}

/// An opened image: drawn from its pixels, or ready-made tiles.
#[derive(Clone)]
enum Opened {
    Raster(Arc<dyn Raster>),
    Tiles(Arc<MbTiles>),
}

/// What the page may change.
#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub opacity: Option<f64>,
    pub placement: Option<String>,
}

/// How an image comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum How {
    /// Copied into the images folder (from a USB stick, a download…).
    Copy,
    /// Catalogued and read where it is (another program's chart folder, an external drive).
    InPlace,
}

/// What an import of several files did.
#[derive(Serialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub added: Vec<Entry>,
    /// Files already in the list (catalogued from the same place before).
    pub already: Vec<String>,
    /// Files that can't be shown, with why.
    pub errors: Vec<String>,
    /// TIFFs found in a folder that aren't georeferenced and have no world file (plain
    /// pictures), skipped quietly: import them one by one to calibrate them.
    pub not_geo: usize,
    /// A folder had more image files than are looked at in one go.
    pub truncated: bool,
}

/// How many images are kept open for drawing (the rest are opened again when next shown).
const KEEP_OPEN: usize = 32;
/// The most image files catalogued from one folder at a time.
pub const MAX_FOLDER_FILES: usize = 5000;

pub struct Store {
    dir: PathBuf,
    entries: RwLock<Vec<Entry>>,
    /// Recently drawn images, by id ("<id>:draft" and "<id>:base" while calibrating).
    open: Mutex<LruCache<String, Opened>>,
}

fn now() -> String {
    // seconds since 1970 are enough for ordering; the page shows dates from `date`
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    s.to_string()
}

fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64;
    format!("{:x}{:x}", t & 0xffff_ffff_ffff, N.fetch_add(1, Ordering::Relaxed) & 0xff)
}

/// The image's own pixels and georeferencing, before any calibration: a KAP, or a TIFF with
/// its GeoKeys or world file. `true` with it: the image has no position of its own (a plain
/// picture), so a calibration works in its pixels.
fn open_base(e: &Entry, path: &Path) -> Result<(Arc<dyn Raster>, bool)> {
    match e.kind.as_str() {
        "kap" => Ok((Arc::new(Kap::open(path)?), false)),
        "geotiff" => {
            let mut t = Tiff::open(Arc::new(FileSource::open(path)?))?;
            match e.georef {
                Some(g) if g.epsg != 0 => {
                    let Some(crs) = Crs::from_epsg(g.epsg) else { return err(format!("EPSG:{} is not supported", g.epsg)) };
                    t.set_geo(crs, g.affine());
                }
                Some(_) => t.clear_geo(),
                None => {}
            }
            let plain = t.geo.is_none();
            Ok((Arc::new(t), plain))
        }
        k => err(format!("unknown image kind {k}")),
    }
}

/// The image as drawn: with its calibration (or the draft being worked on), MBTiles as tiles.
fn open_entry(e: &Entry, path: &Path, draft: bool) -> Result<Opened> {
    if e.kind == "mbtiles" {
        return Ok(Opened::Tiles(Arc::new(MbTiles::open(path)?)));
    }
    let (base, plain) = open_base(e, path)?;
    let cal = if draft { e.draft.as_ref().or(e.calibration.as_ref()) } else { e.calibration.as_ref() };
    match cal {
        Some(c) => Ok(Opened::Raster(Arc::new(Calibrated::wrap(base, plain, c)?))),
        None if plain => err("not placed yet: calibrate it"),
        None => Ok(Opened::Raster(base)),
    }
}

fn state_of(e: &Entry) -> &'static str {
    if e.kind == "mbtiles" || e.kind == "kap" || e.calibration.is_some() {
        return "ready";
    }
    match e.georef {
        Some(g) if g.epsg == 0 => "crs",
        Some(_) => "ready",
        // a TIFF that had no georeferencing of its own is marked by an empty bounds
        None if e.bounds == [0.0; 4] => "calibrate",
        None => "ready",
    }
}

/// Opens an image file by its kind.
pub fn open_raster(kind: &str, path: &Path) -> Result<Arc<dyn Raster>> {
    Ok(match kind {
        "kap" => Arc::new(Kap::open(path)?),
        "geotiff" => {
            let t = Tiff::open(Arc::new(FileSource::open(path)?))?;
            if t.geo.is_none() {
                return err("this TIFF has no georeferencing (it is not a GeoTIFF)");
            }
            Arc::new(t)
        }
        k => return err(format!("unknown image kind {k}")),
    })
}

/// The kind of an image file, from its first bytes.
pub fn sniff(path: &Path) -> Result<&'static str> {
    let mut head = [0u8; 16];
    {
        use std::io::Read;
        let mut f = std::fs::File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let _ = f.read(&mut head)?;
    }
    if crate::mbtiles::is_sqlite(&head) {
        return Ok("mbtiles");
    }
    if &head[..2] == b"II" || &head[..2] == b"MM" {
        return Ok("geotiff");
    }
    if head.starts_with(&[0x89, b'P', b'N', b'G']) || head.starts_with(&[0xff, 0xd8]) {
        return Ok("picture");
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "kap" || head.is_ascii() {
        return Ok("kap");
    }
    err("not a KAP chart, GeoTIFF, MBTiles file, or PNG or JPEG picture")
}

/// Whether a file name looks like a KAP chart, a TIFF or MBTiles (for looking through
/// folders; PNG and JPEG pictures are imported one by one).
pub fn looks_like_image(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "kap" | "tif" | "tiff" | "mbtiles")
}

/// The KAP charts and TIFFs in a folder and the folders inside it (sorted, at most `max`;
/// true when there were more). Hidden folders and `skip` (the images folder) are left out.
pub fn find_images(folder: &Path, skip: Option<&Path>, max: usize) -> Result<(Vec<PathBuf>, bool)> {
    if !folder.is_dir() {
        return err(format!("{} is not a folder", folder.display()));
    }
    let skip = skip.and_then(|p| p.canonicalize().ok());
    let mut out = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    let mut seen = HashSet::new();
    while let Some(d) = stack.pop() {
        // a folder linked into itself is looked at once
        let Ok(canon) = d.canonicalize() else { continue };
        if !seen.insert(canon.clone()) || skip.as_ref() == Some(&canon) {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut subdirs = Vec::new();
        for ent in rd.flatten() {
            let p = ent.path();
            let hidden = ent.file_name().to_string_lossy().starts_with('.');
            let Ok(ft) = ent.file_type() else { continue };
            let is_dir = ft.is_dir() || (ft.is_symlink() && p.is_dir());
            if is_dir {
                if !hidden {
                    subdirs.push(p);
                }
            } else if !hidden && looks_like_image(&p) {
                out.push(p);
                if out.len() > max {
                    out.sort();
                    out.truncate(max);
                    return Ok((out, true));
                }
            }
        }
        subdirs.sort();
        stack.extend(subdirs.into_iter().rev());
    }
    out.sort();
    Ok((out, false))
}

/// A path to compare against others: canonical where the file exists.
fn same_file_key(p: &Path) -> PathBuf {
    let c = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    // Windows paths compare without regard to case, and with or without the \\?\ prefix
    // canonical paths have (a file that can't be found now isn't made canonical)
    if cfg!(windows) {
        let t = c.to_string_lossy().to_lowercase();
        PathBuf::from(t.strip_prefix(r"\\?\").unwrap_or(&t))
    } else {
        c
    }
}

impl Store {
    pub fn open(dir: &Path) -> Store {
        let entries = std::fs::read(dir.join("images.json")).ok().and_then(|b| serde_json::from_slice::<Vec<Entry>>(&b).ok()).unwrap_or_default();
        Store { dir: dir.to_path_buf(), entries: RwLock::new(entries), open: Mutex::new(LruCache::new(NonZeroUsize::new(KEEP_OPEN).expect("not zero"))) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where an image's file is.
    pub fn path_of(&self, e: &Entry) -> PathBuf {
        if e.linked {
            PathBuf::from(&e.file)
        } else {
            self.dir.join(&e.file)
        }
    }

    /// The images, with linked files that can't be found now marked `missing`. A missing
    /// image's open file is let go, so a drive that was pulled out isn't held on to.
    pub fn list(&self) -> Vec<Entry> {
        let mut l = self.entries.read().unwrap_or_else(|e| e.into_inner()).clone();
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        for e in l.iter_mut() {
            e.missing = e.linked && !Path::new(&e.file).is_file();
            if e.missing {
                for k in [e.id.clone(), format!("{}:draft", e.id), format!("{}:base", e.id)] {
                    open.pop(&k);
                }
            }
            e.state = state_of(e).to_string();
        }
        l
    }

    fn entry(&self, id: &str) -> Result<Entry> {
        self.list().into_iter().find(|e| e.id == id).ok_or_else(|| Error("no such image".into()))
    }

    fn forget(&self, id: &str) {
        let mut o = self.open.lock().unwrap_or_else(|e| e.into_inner());
        for k in [id.to_string(), format!("{id}:draft"), format!("{id}:base")] {
            o.pop(&k);
        }
    }

    /// The image opened (kept among the recently drawn): "" as drawn, "draft" with the
    /// calibration being worked on, "base" its own pixels for the calibration's image pane.
    fn opened(&self, id: &str, how: &str) -> Option<Opened> {
        let key = if how.is_empty() { id.to_string() } else { format!("{id}:{how}") };
        if let Some(o) = self.open.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return Some(o.clone());
        }
        let e = self.entries.read().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.id == id).cloned()?;
        let path = self.path_of(&e);
        if e.linked && !path.is_file() {
            // a drive that isn't plugged in: nothing to draw, and no message on every tile
            return None;
        }
        let o = match how {
            "base" => open_base(&e, &path).map(|(r, _)| Opened::Raster(r)),
            "draft" => open_entry(&e, &path, true),
            _ => open_entry(&e, &path, false),
        };
        match o {
            Ok(o) => {
                self.open.lock().unwrap_or_else(|e| e.into_inner()).put(key, o.clone());
                Some(o)
            }
            Err(err) => {
                if how.is_empty() && state_of(&e) == "ready" {
                    eprintln!("image {}: {err}", path.display());
                }
                None
            }
        }
    }

    /// Let go of every open image file (before a drive is ejected: Windows won't eject a
    /// drive with files open on it). They are opened again when next drawn.
    pub fn close_all(&self) {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn save(&self, list: &[Entry]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join("images.json.part");
        std::fs::write(&tmp, serde_json::to_vec_pretty(list).map_err(|e| Error(e.to_string()))?)?;
        std::fs::rename(&tmp, self.dir.join("images.json"))?;
        Ok(())
    }

    /// The drawable image for an id (opened on first use); None for MBTiles, images not placed
    /// yet, and linked files that can't be found now.
    pub fn raster(&self, id: &str) -> Option<Arc<dyn Raster>> {
        match self.opened(id, "")? {
            Opened::Raster(r) => Some(r),
            Opened::Tiles(_) => None,
        }
    }

    /// A file name in the folder that isn't taken, keeping the original name where it can.
    pub fn free_name(&self, wanted: &str) -> String {
        let clean: String = wanted.chars().map(|c| if c.is_alphanumeric() || "._- ".contains(c) { c } else { '_' }).collect();
        let clean = if clean.trim().is_empty() { "image".to_string() } else { clean.trim().to_string() };
        let (stem, ext) = match clean.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (clean.clone(), String::new()),
        };
        let mut name = clean.clone();
        let mut n = 2;
        while self.dir.join(&name).exists() {
            name = format!("{stem} ({n}){ext}");
            n += 1;
        }
        name
    }

    /// The linked image already catalogued from this file, if any.
    fn linked_to(&self, path: &Path) -> Option<Entry> {
        let key = same_file_key(path);
        self.entries.read().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.linked && same_file_key(Path::new(&e.file)) == key).cloned()
    }

    /// Add an image: a KAP chart, GeoTIFF or MBTiles copied into the folder or catalogued where
    /// it is; or a picture (PNG, JPEG, a TIFF without georeferencing) placed by its world file
    /// or waiting to be calibrated.
    pub fn import(&self, path: &Path, how: How) -> Result<Entry> {
        self.import_one(path, how, true, false)
    }

    fn import_one(&self, path: &Path, how: How, keep_open: bool, from_folder: bool) -> Result<Entry> {
        let kind = sniff(path)?;
        if how == How::InPlace && kind != "picture" && self.linked_to(path).is_some() {
            return err("already in the list");
        }
        match kind {
            "mbtiles" => return self.import_mbtiles(path, how),
            // a PNG or JPEG is always copied in, as a tiled TIFF
            "picture" => {
                let (w, h, rgba) = crate::world::decode_picture(path)?;
                std::fs::create_dir_all(&self.dir)?;
                let fname = self.free_name(&format!("{}.tif", stem(path)));
                crate::tiffw::write_tiff(&self.dir.join(&fname), w, h, &rgba, "TweedPlot")?;
                drop(rgba);
                return self.add_picture(path, &fname, false, w, h);
            }
            "geotiff" => {
                let t = Tiff::open(Arc::new(FileSource::open(path)?))?;
                if t.geo.is_none() {
                    // a plain picture: placed by a world file, else (one by one) to calibrate
                    let has_world = crate::world::find_world_file(path).is_some();
                    if from_folder && !has_world {
                        return err("this TIFF has no georeferencing");
                    }
                    let (w, h) = t.size();
                    let file = self.bring_in(path, how)?;
                    return self.add_picture(path, &file, how == How::InPlace, w, h);
                }
            }
            _ => {}
        }
        // open it where it is first, so a file that can't be shown is never copied; a KAP is
        // read once, for both its header and drawing
        let (raster, name, date, scale, fit, source): (Arc<dyn Raster>, _, _, _, _, _) = if kind == "kap" {
            let k = Kap::open(path)?;
            let name = match (k.number.is_empty(), k.name.is_empty()) {
                (false, false) => format!("{} {}", k.number, k.name),
                (true, false) => k.name.clone(),
                _ => stem(path),
            };
            let (date, scale, fit) = (k.edition.clone(), k.scale, Some(k.fit_rms_px));
            (Arc::new(k), name, date, scale, fit, "KAP chart")
        } else {
            (open_raster(kind, path)?, stem(path), String::new(), None, None, "GeoTIFF")
        };
        let file = self.bring_in(path, how)?;
        let e = self.add_entry(&file, how == How::InPlace, kind, &name, source, &date, "", scale, fit, raster.as_ref())?;
        if keep_open {
            self.open.lock().unwrap_or_else(|e| e.into_inner()).put(e.id.clone(), Opened::Raster(raster));
        }
        if kind == "kap" {
            let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
            place_by_scale(&mut list, &e.id);
            self.save(&list)?;
        }
        Ok(e)
    }

    /// The file as the entry holds it: copied into the folder (its name there), or its full path.
    fn bring_in(&self, path: &Path, how: How) -> Result<String> {
        Ok(match how {
            How::Copy => {
                std::fs::create_dir_all(&self.dir)?;
                let fname = self.free_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into()));
                std::fs::copy(path, self.dir.join(&fname)).map_err(|e| Error(format!("copying {}: {e}", path.display())))?;
                fname
            }
            How::InPlace => {
                // the full path, so it is found again whatever folder TweedPlot runs from
                let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
                full.to_string_lossy().into_owned()
            }
        })
    }

    fn import_mbtiles(&self, path: &Path, how: How) -> Result<Entry> {
        let m = MbTiles::open(path)?;
        let file = self.bring_in(path, how)?;
        let name = if m.name.trim().is_empty() { stem(path) } else { m.name.trim().to_string() };
        let e = Entry {
            id: new_id(),
            name,
            kind: "mbtiles".into(),
            file,
            linked: how == How::InPlace,
            missing: false,
            source: "MBTiles".into(),
            date: String::new(),
            credit: m.attribution.clone(),
            bounds: m.bounds,
            width: 0,
            height: 0,
            res_m: m.resolution_m(),
            native_zoom: m.max_zoom,
            fit_rms_px: None,
            scale: None,
            visible: true,
            opacity: 1.0,
            placement: "under".into(),
            added: now(),
            georef: None,
            calibration: None,
            draft: None,
            min_zoom: Some(m.min_zoom),
            calib_rms_m: None,
            state: String::new(),
        };
        self.push(e)
    }

    /// A picture (as a TIFF without georeferencing, in the folder or linked): placed by its
    /// world file when it has one (its coordinate system from a .prj or .aux.xml, else asked),
    /// else to calibrate.
    fn add_picture(&self, orig: &Path, file: &str, linked: bool, w: usize, h: usize) -> Result<Entry> {
        let georef = match crate::world::find_world_file(orig) {
            Some(wf) => {
                let aff = crate::world::read_world_file(&wf)?;
                let epsg = crate::world::find_crs(orig).unwrap_or(0);
                Some(WorldGeo { epsg, aff: [aff.a, aff.b, aff.c, aff.d, aff.e, aff.f] })
            }
            None => None,
        };
        let mut e = Entry {
            id: new_id(),
            name: stem(orig),
            kind: "geotiff".into(),
            file: file.to_string(),
            linked,
            missing: false,
            source: "Picture".into(),
            date: String::new(),
            credit: String::new(),
            bounds: [0.0; 4],
            width: w,
            height: h,
            res_m: 0.0,
            native_zoom: 0,
            fit_rms_px: None,
            scale: None,
            visible: true,
            opacity: 0.85,
            placement: "under".into(),
            added: now(),
            georef,
            calibration: None,
            draft: None,
            min_zoom: None,
            calib_rms_m: None,
            state: String::new(),
        };
        self.measure(&mut e);
        self.push(e)
    }

    /// Where an image now is (bounds, resolution, zooms), after it is placed or calibrated.
    fn measure(&self, e: &mut Entry) {
        if let Ok(Opened::Raster(r)) = open_entry(e, &self.path_of(e), false) {
            e.bounds = r.bounds();
            e.res_m = r.resolution_m();
            e.native_zoom = native_zoom(r.as_ref());
        } else if e.kind != "mbtiles" && e.kind != "kap" {
            e.bounds = [0.0; 4];
        }
    }

    fn push(&self, e: Entry) -> Result<Entry> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        list.push(e.clone());
        self.save(&list)?;
        let mut e = e;
        e.state = state_of(&e).to_string();
        Ok(e)
    }

    /// Add several files, reporting each that can't be shown and going on with the rest.
    /// `from_folder`: the files were found by looking through a folder, so TIFFs that are
    /// plain pictures are counted, not reported one by one.
    pub fn import_many(&self, paths: &[PathBuf], how: How, from_folder: bool) -> Report {
        let mut r = Report::default();
        // a few files are kept open to draw at once; a whole chart folder is opened as shown
        let keep_open = paths.len() <= 8;
        for p in paths {
            let label = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
            match self.import_one(p, how, keep_open, from_folder) {
                Ok(e) => r.added.push(e),
                Err(e) if e.0 == "already in the list" => r.already.push(label),
                Err(e) if from_folder && e.0.contains("no georeferencing") => r.not_geo += 1,
                Err(e) => r.errors.push(format!("{label}: {e}")),
            }
        }
        r
    }

    /// The coordinate system of an image's world file (when the files beside it didn't say).
    pub fn set_crs(&self, id: &str, epsg: u32) -> Result<Entry> {
        if Crs::from_epsg(epsg).is_none() {
            return err(format!("EPSG:{epsg} is not supported"));
        }
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let Some(e) = list.iter_mut().find(|e| e.id == id) else { return err("no such image") };
        let Some(g) = e.georef.as_mut() else { return err("this image has no world file") };
        g.epsg = epsg;
        self.measure(e);
        let mut out = e.clone();
        self.save(&list)?;
        drop(list);
        self.forget(id);
        out.state = state_of(&out).to_string();
        Ok(out)
    }

    // ---------- calibration ----------

    /// What the calibration needs to show the image: its size, the flat zoom of its pixel
    /// tiles, whether it has a position of its own, and the saved and draft calibrations.
    pub fn calib_info(&self, id: &str) -> Result<serde_json::Value> {
        let e = self.entry(id)?;
        if e.kind == "mbtiles" {
            return err("MBTiles are ready-made map tiles: they can't be calibrated");
        }
        let (base, plain) = open_base(&e, &self.path_of(&e))?;
        let (w, h) = base.size();
        Ok(serde_json::json!({
            "id": e.id, "name": e.name, "width": w, "height": h,
            "pixelZoom": crate::render::pixel_zoom(base.as_ref()),
            "plain": plain,
            "calibration": e.calibration, "draft": e.draft,
        }))
    }

    /// Fit a calibration being worked on; it is kept as the image's draft (drawn by the
    /// draft tiles) until saved or discarded.
    pub fn calib_fit(&self, id: &str, c: &Calib) -> Result<FitReport> {
        let e = self.entry(id)?;
        let (base, plain) = open_base(&e, &self.path_of(&e))?;
        let (_, report) = crate::calib::fit(base.as_ref(), plain, c);
        {
            let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
            if let Some(x) = list.iter_mut().find(|x| x.id == id) {
                x.draft = Some(c.clone());
            }
            self.save(&list)?;
        }
        self.open.lock().unwrap_or_else(|e| e.into_inner()).pop(&format!("{id}:draft"));
        Ok(report)
    }

    /// Use the draft: it becomes the image's calibration.
    pub fn calib_save(&self, id: &str) -> Result<Entry> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let Some(e) = list.iter_mut().find(|e| e.id == id) else { return err("no such image") };
        let Some(d) = e.draft.take() else { return err("nothing to save") };
        let (base, plain) = open_base(e, &self.path_of(e))?;
        let (warp, report) = crate::calib::fit(base.as_ref(), plain, &d);
        if warp.is_none() {
            e.draft = Some(d);
            return err(report.note);
        }
        let points = d.points.iter().any(|p| p.px.is_some() && p.ll.is_some());
        let moved = d.nudge != crate::calib::Nudge::default();
        e.calibration = (points || moved).then_some(d);
        e.calib_rms_m = if e.calibration.is_some() { report.rms_m } else { None };
        self.measure(e);
        let mut out = e.clone();
        self.save(&list)?;
        drop(list);
        self.forget(id);
        out.state = state_of(&out).to_string();
        Ok(out)
    }

    /// Drop the draft (the saved calibration, if any, stays).
    pub fn calib_discard(&self, id: &str) -> Result<()> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = list.iter_mut().find(|e| e.id == id) {
            e.draft = None;
        }
        self.save(&list)?;
        drop(list);
        self.forget(id);
        Ok(())
    }

    /// Remove an image's calibration: back to its own georeferencing (or to calibrate).
    pub fn calib_clear(&self, id: &str) -> Result<Entry> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let Some(e) = list.iter_mut().find(|e| e.id == id) else { return err("no such image") };
        e.calibration = None;
        e.draft = None;
        e.calib_rms_m = None;
        self.measure(e);
        let mut out = e.clone();
        self.save(&list)?;
        drop(list);
        self.forget(id);
        out.state = state_of(&out).to_string();
        Ok(out)
    }

    /// Put the raster charts in scale order (the most detailed on top), each in one of the
    /// places charts already have in the list: satellite images and GeoTIFFs stay where they are.
    pub fn order_by_scale(&self) -> Result<()> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        order_by_scale(&mut list);
        self.save(&list)
    }

    /// Add a file already in the folder.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &self,
        file: &str,
        kind: &str,
        name: &str,
        source: &str,
        date: &str,
        credit: &str,
        scale: Option<u64>,
        fit: Option<f64>,
        raster: &dyn Raster,
    ) -> Result<Entry> {
        self.add_entry(file, false, kind, name, source, date, credit, scale, fit, raster)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_entry(
        &self,
        file: &str,
        linked: bool,
        kind: &str,
        name: &str,
        source: &str,
        date: &str,
        credit: &str,
        scale: Option<u64>,
        fit: Option<f64>,
        raster: &dyn Raster,
    ) -> Result<Entry> {
        let (width, height) = raster.size();
        let e = Entry {
            id: new_id(),
            name: name.to_string(),
            kind: kind.to_string(),
            file: file.to_string(),
            linked,
            missing: false,
            source: source.to_string(),
            date: date.to_string(),
            credit: credit.to_string(),
            bounds: raster.bounds(),
            width,
            height,
            res_m: raster.resolution_m(),
            native_zoom: native_zoom(raster),
            fit_rms_px: fit,
            scale,
            visible: true,
            opacity: if kind == "kap" { 1.0 } else { 0.85 },
            placement: "under".to_string(),
            added: now(),
            georef: None,
            calibration: None,
            draft: None,
            min_zoom: None,
            calib_rms_m: None,
            state: String::new(),
        };
        self.push(e)
    }

    pub fn update(&self, id: &str, p: Patch) -> Result<Entry> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let Some(e) = list.iter_mut().find(|e| e.id == id) else { return err("no such image") };
        if let Some(n) = p.name.filter(|n| !n.trim().is_empty()) {
            e.name = n.trim().to_string();
        }
        if let Some(v) = p.visible {
            e.visible = v;
        }
        if let Some(o) = p.opacity {
            e.opacity = o.clamp(0.05, 1.0);
        }
        if let Some(pl) = p.placement.filter(|p| p == "under" || p == "over") {
            e.placement = pl;
        }
        let out = e.clone();
        self.save(&list)?;
        Ok(out)
    }

    /// Move an image up or down the list (later images draw on top).
    pub fn reorder(&self, ids: &[String]) -> Result<()> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<Entry> = ids.iter().filter_map(|id| list.iter().find(|e| &e.id == id).cloned()).collect();
        for e in list.iter() {
            if !ids.contains(&e.id) {
                out.push(e.clone());
            }
        }
        *list = out;
        self.save(&list)
    }

    /// Remove an image, deleting TweedPlot's copy. A linked image's file is left alone.
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let Some(i) = list.iter().position(|e| e.id == id) else { return Ok(()) };
        let e = list.remove(i);
        self.save(&list)?;
        drop(list);
        self.forget(id);
        if !e.linked {
            let _ = std::fs::remove_file(self.dir.join(&e.file));
        }
        Ok(())
    }

    /// A map tile of an image (PNG), or None.
    pub fn tile_png(&self, id: &str, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
        self.tile(id, "", z, x, y).map(|t| t.0)
    }

    /// A map tile and its type: "" the image as drawn, "draft" through the calibration being
    /// worked on, "px" the picture itself flat (the calibration's image pane).
    pub fn tile(&self, id: &str, how: &str, z: u8, x: u32, y: u32) -> Option<(Vec<u8>, &'static str)> {
        if how == "px" {
            let Opened::Raster(r) = self.opened(id, "base")? else { return None };
            return crate::render::render_pixel_tile(r.as_ref(), z, x, y).map(|b| (b, "image/png"));
        }
        match self.opened(id, how)? {
            Opened::Tiles(m) => m.tile(z, x, y).map(|b| (b, m.mime())),
            Opened::Raster(r) => crate::render::render_tile(r.as_ref(), z, x, y).map(|b| (b, "image/png")),
        }
    }
}

/// Ground metres per pixel: the order charts go in (coarser below, finer above). A KAP's
/// pixel size follows its scale, and is there even when the header has no scale.
fn detail(e: &Entry) -> f64 {
    if e.res_m.is_finite() && e.res_m > 0.0 {
        e.res_m
    } else {
        e.scale.map_or(f64::MAX, |s| s as f64 / 4000.0)
    }
}

/// Raster charts sorted coarsest first (drawn underneath) within the positions charts hold.
pub fn order_by_scale(list: &mut [Entry]) {
    let slots: Vec<usize> = (0..list.len()).filter(|&i| list[i].kind == "kap").collect();
    let mut charts: Vec<Entry> = slots.iter().map(|&i| list[i].clone()).collect();
    // stable: equal scales keep their order
    charts.sort_by(|a, b| detail(b).total_cmp(&detail(a)));
    for (slot, e) in slots.into_iter().zip(charts) {
        list[slot] = e;
    }
}

/// Move a newly added chart to its place by scale: just above the highest chart that is as
/// coarse or coarser, else just below the lowest chart that is finer, else where it is (on top).
pub fn place_by_scale(list: &mut Vec<Entry>, id: &str) {
    let Some(i) = list.iter().position(|e| e.id == id) else { return };
    let e = list.remove(i);
    let d = detail(&e);
    let at = if let Some(j) = list.iter().rposition(|x| x.kind == "kap" && detail(x) >= d) {
        j + 1
    } else if let Some(j) = list.iter().position(|x| x.kind == "kap" && detail(x) < d) {
        j
    } else {
        list.len()
    };
    list.insert(at, e);
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Image".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, kind: &str, res: f64) -> Entry {
        Entry {
            id: id.into(),
            name: id.into(),
            kind: kind.into(),
            file: String::new(),
            linked: false,
            missing: false,
            source: String::new(),
            date: String::new(),
            credit: String::new(),
            bounds: [0.0; 4],
            width: 1,
            height: 1,
            res_m: res,
            native_zoom: 0,
            fit_rms_px: None,
            scale: None,
            visible: true,
            opacity: 1.0,
            placement: "under".into(),
            added: String::new(),
            georef: None,
            calibration: None,
            draft: None,
            min_zoom: None,
            calib_rms_m: None,
            state: String::new(),
        }
    }
    fn ids(l: &[Entry]) -> Vec<&str> {
        l.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn charts_go_in_scale_order() {
        // bottom of the list first: coastal (50 m), satellite, harbour (2 m)
        let mut l = vec![e("coastal", "kap", 50.0), e("sat", "geotiff", 10.0), e("harbour", "kap", 2.0)];
        l.push(e("approach", "kap", 10.0));
        place_by_scale(&mut l, "approach");
        assert_eq!(ids(&l), ["coastal", "approach", "sat", "harbour"]);
        l.push(e("ocean", "kap", 500.0));
        place_by_scale(&mut l, "ocean");
        assert_eq!(ids(&l), ["ocean", "coastal", "approach", "sat", "harbour"]);
        l.push(e("berth", "kap", 0.5));
        place_by_scale(&mut l, "berth");
        assert_eq!(ids(&l), ["ocean", "coastal", "approach", "sat", "harbour", "berth"]);
        // a muddled list is put back in order, the satellite image staying in its place
        let mut m = vec![e("harbour", "kap", 2.0), e("sat", "geotiff", 10.0), e("ocean", "kap", 500.0), e("coastal", "kap", 50.0)];
        order_by_scale(&mut m);
        assert_eq!(ids(&m), ["ocean", "sat", "coastal", "harbour"]);
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;
    use crate::calib::Point;
    use crate::geo::{web_mercator, web_mercator_inv};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tp-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A 64 × 48 picture, opaque, a colour gradient.
    fn picture(path: &Path) {
        let (w, h) = (64u32, 48u32);
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend([(x * 4) as u8, (y * 5) as u8, 128, 255]);
            }
        }
        std::fs::write(path, crate::render::encode_png(&px, w, h)).unwrap();
    }

    #[test]
    fn world_file_places_a_picture_and_asks_for_its_crs() {
        let d = tmp("wld");
        let src = d.join("in");
        std::fs::create_dir_all(&src).unwrap();
        let img = src.join("bay.png");
        picture(&img);
        // NZTM, 10 m pixels, near Nelson
        std::fs::write(src.join("bay.pgw"), "10\n0\n0\n-10\n1623005\n5431995\n").unwrap();
        let s = Store::open(&d.join("images"));
        let e = s.import(&img, How::Copy).unwrap();
        assert_eq!(e.state, "crs");
        assert!(s.tile("x", "", 10, 0, 0).is_none());
        let e = s.set_crs(&e.id, 2193).unwrap();
        assert_eq!(e.state, "ready");
        assert!(e.bounds[0] > 173.2 && e.bounds[2] < 173.4 && e.bounds[1] > -41.4 && e.bounds[3] < -41.2, "{:?}", e.bounds);
        assert!((e.res_m - 10.0).abs() < 0.5, "{}", e.res_m);
        // it draws: a position inside it maps to a pixel inside the picture
        let r = s.raster(&e.id).unwrap();
        let (c, row) = r.to_pixel((e.bounds[0] + e.bounds[2]) / 2.0, (e.bounds[1] + e.bounds[3]) / 2.0).unwrap();
        assert!((0.0..64.0).contains(&c) && (0.0..48.0).contains(&row), "{c} {row}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_plain_picture_waits_then_is_calibrated() {
        let d = tmp("cal");
        let img = d.join("sketch.png");
        picture(&img);
        let s = Store::open(&d.join("images"));
        let e = s.import(&img, How::Copy).unwrap();
        assert_eq!(e.state, "calibrate");
        assert!(s.raster(&e.id).is_none());
        let info = s.calib_info(&e.id).unwrap();
        assert_eq!(info["plain"], true);
        assert_eq!(info["width"], 64);
        // the picture's own tiles show in the image pane
        assert!(s.tile(&e.id, "px", 0, 0, 0).is_some());
        // two points: 1 pixel = 2 mercator metres, north up
        let (x0, y0) = web_mercator(173.25, -41.25);
        let ll = |c: f64, r: f64| {
            let (lon, lat) = web_mercator_inv(x0 + 2.0 * c, y0 - 2.0 * r);
            Some([lon, lat])
        };
        let c = Calib {
            points: vec![
                Point { px: Some([0.0, 0.0]), ll: ll(0.0, 0.0), off: false, from: "typed".into() },
                Point { px: Some([64.0, 48.0]), ll: ll(64.0, 48.0), off: false, from: "chart".into() },
            ],
            ..Default::default()
        };
        let r = s.calib_fit(&e.id, &c).unwrap();
        assert_eq!(r.model, "similarity");
        assert!(r.rms_m.unwrap() < 0.01);
        // the draft draws before it is saved; the image itself doesn't yet
        let n = 1u32 << 16;
        let tx = ((173.2505 + 180.0) / 360.0 * n as f64) as u32;
        let lat = (-41.2502f64).to_radians();
        let ty = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n as f64) as u32;
        assert!(s.tile(&e.id, "draft", 16, tx, ty).is_some());
        assert!(s.tile(&e.id, "", 16, tx, ty).is_none());
        let e = s.calib_save(&e.id).unwrap();
        assert_eq!(e.state, "ready");
        assert!(e.draft.is_none() && e.calibration.is_some());
        assert!(s.tile(&e.id, "", 16, tx, ty).is_some());
        // taken off again, it waits to be calibrated
        let e = s.calib_clear(&e.id).unwrap();
        assert_eq!(e.state, "calibrate");
        let _ = std::fs::remove_dir_all(&d);
    }
}
