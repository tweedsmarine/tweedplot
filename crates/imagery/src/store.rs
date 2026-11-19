//! The user's images: a folder of image files (copied in on import, or downloaded) and
//! `images.json` listing them with how they are shown. Opened images are kept for drawing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

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
    /// File name inside the images folder.
    pub file: String,
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

pub struct Store {
    dir: PathBuf,
    entries: RwLock<Vec<Entry>>,
    open: Mutex<HashMap<String, Arc<dyn Raster>>>,
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

impl Store {
    pub fn open(dir: &Path) -> Store {
        let entries = std::fs::read(dir.join("images.json")).ok().and_then(|b| serde_json::from_slice::<Vec<Entry>>(&b).ok()).unwrap_or_default();
        Store { dir: dir.to_path_buf(), entries: RwLock::new(entries), open: Mutex::new(HashMap::new()) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn list(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(|e| e.into_inner()).clone()
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
        let e = self.list().into_iter().find(|e| e.id == id)?;
        match open_raster(&e.kind, &self.dir.join(&e.file)) {
            Ok(r) => {
                self.open.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), r.clone());
                Some(r)
            }
            Err(err) => {
                eprintln!("image {}: {err}", e.file);
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

    /// Copy a KAP chart or GeoTIFF into the folder and add it.
    pub fn import(&self, path: &Path) -> Result<Entry> {
        let kind = sniff(path)?;
        // open it where it is first, so a file that can't be shown is never copied
        let raster = open_raster(kind, path)?;
        std::fs::create_dir_all(&self.dir)?;
        let fname = self.free_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into()));
        std::fs::copy(path, self.dir.join(&fname)).map_err(|e| Error(format!("copying {}: {e}", path.display())))?;
        let (name, date, scale, fit, source) = if kind == "kap" {
            let k = Kap::open(&self.dir.join(&fname))?;
            let name = match (k.number.is_empty(), k.name.is_empty()) {
                (false, false) => format!("{} {}", k.number, k.name),
                (true, false) => k.name.clone(),
                _ => stem(path),
            };
            (name, k.edition.clone(), k.scale, Some(k.fit_rms_px), "KAP chart")
        } else {
            (stem(path), String::new(), None, None, "GeoTIFF")
        };
        let e = self.add(&fname, kind, &name, source, &date, "", scale, fit, raster.as_ref())?;
        self.open.lock().unwrap_or_else(|e| e.into_inner()).insert(e.id.clone(), raster);
        if kind == "kap" {
            let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
            place_by_scale(&mut list, &e.id);
            self.save(&list)?;
        }
        Ok(e)
    }

    /// Put the raster charts in scale order (the most detailed on top), each in one of the
    /// places charts already have in the list: satellite images and GeoTIFFs stay where they are.
    pub fn order_by_scale(&self) -> Result<()> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        order_by_scale(&mut list);
        self.save(&list)
    }

    /// Add a file already in the folder.
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
        let (width, height) = raster.size();
        let e = Entry {
            id: new_id(),
            name: name.to_string(),
            kind: kind.to_string(),
            file: file.to_string(),
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

    /// Remove an image and delete its file.
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut list = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let Some(i) = list.iter().position(|e| e.id == id) else { return Ok(()) };
        let e = list.remove(i);
        self.save(&list)?;
        self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let _ = std::fs::remove_file(self.dir.join(&e.file));
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
