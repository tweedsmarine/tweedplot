//! The user's images: a folder of image files (copied in on import, or downloaded) and
//! `images.json` listing them with how they are shown. An image can also be catalogued where
//! it is (`linked`): a chart folder another program already uses, or an external drive. Then
//! the entry holds the file's full path, TweedPlot never copies, moves or deletes it, and the
//! image is reported `missing` while the file can't be found (a drive not plugged in).
//! Recently drawn images are kept open (a KAP is read into memory whole).

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use lru::LruCache;
use serde::{Deserialize, Serialize};

use crate::kap::Kap;
use crate::render::{native_zoom, Raster};
use crate::source::FileSource;
use crate::tiff::Tiff;
use crate::{err, Error, Result};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// "kap" or "geotiff".
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
    /// TIFFs found in a folder that aren't georeferenced (plain pictures), skipped quietly.
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
    open: Mutex<LruCache<String, Arc<dyn Raster>>>,
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

/// Opens an image file by its kind.
pub fn open_raster(kind: &str, path: &Path) -> Result<Arc<dyn Raster>> {
    Ok(match kind {
        "kap" => Arc::new(Kap::open(path)?),
        "geotiff" => {
            let t = Tiff::open(Arc::new(FileSource::open(path)?))?;
            if t.geo.is_none() {
                return err("this TIFF has no georeferencing (it is not a GeoTIFF): calibrating plain images comes later");
            }
            Arc::new(t)
        }
        k => return err(format!("unknown image kind {k}")),
    })
}

/// The kind of an image file, from its first bytes.
pub fn sniff(path: &Path) -> Result<&'static str> {
    let mut head = [0u8; 4];
    {
        use std::io::Read;
        let mut f = std::fs::File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let _ = f.read(&mut head)?;
    }
    if &head[..2] == b"II" || &head[..2] == b"MM" {
        return Ok("geotiff");
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "kap" || head.is_ascii() {
        return Ok("kap");
    }
    err("not a KAP chart or a GeoTIFF")
}

/// Whether a file name looks like a KAP chart or a TIFF (for looking through folders).
pub fn looks_like_image(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "kap" | "tif" | "tiff")
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
                open.pop(&e.id);
            }
        }
        l
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

    /// The drawable image for an id (opened on first use).
    pub fn raster(&self, id: &str) -> Option<Arc<dyn Raster>> {
        if let Some(r) = self.open.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            return Some(r.clone());
        }
        let e = self.entries.read().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.id == id).cloned()?;
        let path = self.path_of(&e);
        if e.linked && !path.is_file() {
            // a drive that isn't plugged in: nothing to draw, and no message on every tile
            return None;
        }
        match open_raster(&e.kind, &path) {
            Ok(r) => {
                self.open.lock().unwrap_or_else(|e| e.into_inner()).put(id.to_string(), r.clone());
                Some(r)
            }
            Err(err) => {
                eprintln!("image {}: {err}", path.display());
                None
            }
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

    /// Add a KAP chart or GeoTIFF: copied into the folder, or catalogued where it is.
    pub fn import(&self, path: &Path, how: How) -> Result<Entry> {
        self.import_one(path, how, true)
    }

    fn import_one(&self, path: &Path, how: How, keep_open: bool) -> Result<Entry> {
        let kind = sniff(path)?;
        if how == How::InPlace && self.linked_to(path).is_some() {
            return err("already in the list");
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
        let file = match how {
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
        };
        let e = self.add_entry(&file, how == How::InPlace, kind, &name, source, &date, "", scale, fit, raster.as_ref())?;
        if keep_open {
            self.open.lock().unwrap_or_else(|e| e.into_inner()).put(e.id.clone(), raster);
        }
        if kind == "kap" {
            let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
            place_by_scale(&mut list, &e.id);
            self.save(&list)?;
        }
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
            match self.import_one(p, how, keep_open) {
                Ok(e) => r.added.push(e),
                Err(e) if e.0 == "already in the list" => r.already.push(label),
                Err(e) if from_folder && e.0.contains("no georeferencing") => r.not_geo += 1,
                Err(e) => r.errors.push(format!("{label}: {e}")),
            }
        }
        r
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
        };
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        list.push(e.clone());
        self.save(&list)?;
        Ok(e)
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
        self.open.lock().unwrap_or_else(|e| e.into_inner()).pop(id);
        if !e.linked {
            let _ = std::fs::remove_file(self.dir.join(&e.file));
        }
        Ok(())
    }

    /// A map tile of an image (PNG), or None.
    pub fn tile_png(&self, id: &str, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
        let r = self.raster(id)?;
        crate::render::render_tile(r.as_ref(), z, x, y)
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
