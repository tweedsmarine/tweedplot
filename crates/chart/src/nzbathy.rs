//! NIWA's New Zealand regional bathymetry (2016, 250 m, CC BY): imported from the
//! GeoTIFF NIWA publishes (`nzbathy_2016.tif`, in a zip or loose), kept as a compact tiled
//! store with coarser levels, and drawn as shaded depth images with contours for the chart
//! (`<z>/<x>/<y>` 256-pixel PNG tiles).
//!
//! The source is a float32 grid of heights (m, negative down) in WGS 84 / Mercator 41
//! (EPSG:3994: ellipsoidal Mercator, true scale at 41° S, central meridian 100° E), one
//! LZW strip a row. The store keeps heights as half-metres in 16 bits, in 256 × 256
//! tiles of raw deflate, with each coarser level averaging 2 × 2 cells of the one below.
//!
//! A compilation of old and new surveys, not chart datum: not for navigation.

#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rayon::prelude::*;

/// The store's file name in its folder.
pub const FILE: &str = "niwa-bathy-2016.tpg";
/// The first zoom the depths are drawn at.
pub const MIN_ZOOM: u8 = 3;

const MAGIC: &[u8; 4] = b"NZB1";
const TILE: usize = 256;
const NODATA: i16 = i16::MIN;
/// Stored units per metre.
const UNITS: f32 = 2.0;
const A: f64 = 6_378_137.0;
const E2: f64 = 0.006_694_379_990_141_317;
const LAT_TS: f64 = -41.0;
const LON0: f64 = 100.0;
const EPSG: u16 = 3994;

/// What went wrong reading or writing.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

fn bad<T>(m: impl Into<String>) -> Result<T> {
    Err(Error(m.into()))
}

// ---------- the projection ----------------------------------------------------------------

/// Scale of the projection on the ellipsoid at its standard parallel.
fn k0() -> f64 {
    let p = LAT_TS.to_radians();
    p.cos() / (1.0 - E2 * p.sin().powi(2)).sqrt()
}

/// Longitude and latitude (degrees) to the grid's Mercator metres.
fn forward(lon: f64, lat: f64) -> (f64, f64) {
    let e = E2.sqrt();
    let phi = lat.to_radians();
    let psi = phi.tan().asinh() - e * (e * phi.sin()).atanh();
    let dl = (lon - LON0).rem_euclid(360.0).to_radians();
    (A * k0() * dl, A * k0() * psi)
}

/// The grid's Mercator metres to longitude (degrees, may pass 180) and latitude.
fn inverse(x: f64, y: f64) -> (f64, f64) {
    let e = E2.sqrt();
    let psi = y / (A * k0());
    let mut phi = 2.0 * psi.exp().atan() - std::f64::consts::FRAC_PI_2;
    for _ in 0..8 {
        let s = e * phi.sin();
        phi = 2.0 * (psi.exp() * ((1.0 + s) / (1.0 - s)).powf(e / 2.0)).atan() - std::f64::consts::FRAC_PI_2;
    }
    (LON0 + (x / (A * k0())).to_degrees(), phi.to_degrees())
}

/// Grid metres on the ground at a latitude: the grid's scale there.
fn scale(lat: f64) -> f64 {
    let p = lat.to_radians();
    k0() * (1.0 - E2 * p.sin().powi(2)).sqrt() / p.cos()
}

// ---------- reading the GeoTIFF -----------------------------------------------------------

struct Tiff {
    file: File,
    width: usize,
    height: usize,
    rows_per_strip: usize,
    compression: u16,
    offsets: Vec<u64>,
    counts: Vec<u64>,
    /// Pixel size (m) and the north-west corner of the first pixel.
    cell: f64,
    x0: f64,
    y0: f64,
    nodata: Option<f64>,
}

fn rd16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn rd32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn rd64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

impl Tiff {
    fn open(path: &Path) -> Result<Tiff> {
        let mut file = File::open(path)?;
        let mut h = [0u8; 8];
        file.read_exact(&mut h)?;
        if &h[..4] != b"II*\0" {
            return bad("not a classic little-endian GeoTIFF");
        }
        file.seek(SeekFrom::Start(rd32(&h[4..]) as u64))?;
        let mut n = [0u8; 2];
        file.read_exact(&mut n)?;
        let mut ifd = vec![0u8; rd16(&n) as usize * 12];
        file.read_exact(&mut ifd)?;
        let size = |t: u16| match t {
            1 | 2 | 6 | 7 => 1usize,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            _ => 8,
        };
        // an entry's values: inline when they fit in 4 bytes, else read from their offset
        let mut tags = std::collections::HashMap::new();
        for e in ifd.chunks_exact(12) {
            let (tag, typ, cnt) = (rd16(e), rd16(&e[2..]), rd32(&e[4..]) as usize);
            let total = cnt * size(typ);
            let data = if total <= 4 {
                e[8..8 + total].to_vec()
            } else {
                let mut d = vec![0u8; total];
                file.seek(SeekFrom::Start(rd32(&e[8..]) as u64))?;
                file.read_exact(&mut d)?;
                d
            };
            tags.insert(tag, (typ, data));
        }
        let uint = |tag: u16| -> Option<Vec<u64>> {
            let (typ, d) = tags.get(&tag)?;
            Some(match typ {
                3 => d.chunks_exact(2).map(|c| rd16(c) as u64).collect(),
                4 => d.chunks_exact(4).map(|c| rd32(c) as u64).collect(),
                16 => d.chunks_exact(8).map(rd64).collect(),
                _ => return None,
            })
        };
        let one = |tag: u16| uint(tag).and_then(|v| v.first().copied());
        let doubles = |tag: u16| -> Option<Vec<f64>> {
            let (typ, d) = tags.get(&tag)?;
            (*typ == 12).then(|| d.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect())
        };
        let (width, height) = (one(256).ok_or(Error("no width".into()))? as usize, one(257).ok_or(Error("no height".into()))? as usize);
        if tags.contains_key(&322) {
            return bad("a tiled GeoTIFF: expected strips");
        }
        if one(258) != Some(32) || one(339) != Some(3) || one(277).unwrap_or(1) != 1 {
            return bad("expected one band of 32-bit floats");
        }
        let compression = one(259).unwrap_or(1) as u16;
        if !matches!(compression, 1 | 5 | 8 | 32946) {
            return bad(format!("unsupported TIFF compression {compression}"));
        }
        if one(317).unwrap_or(1) != 1 {
            return bad("unsupported TIFF predictor");
        }
        let offsets = uint(273).ok_or(Error("no strip offsets".into()))?;
        let counts = uint(279).ok_or(Error("no strip sizes".into()))?;
        let rows_per_strip = (one(278).unwrap_or(height as u64) as usize).clamp(1, height);
        if offsets.len() != counts.len() || offsets.len() < height.div_ceil(rows_per_strip) {
            return bad("the strip tables don't match the image");
        }
        // the projection: EPSG:3994 in the GeoKey directory
        let keys = tags.get(&34735).map(|(_, d)| d.chunks_exact(2).map(rd16).collect::<Vec<u16>>()).unwrap_or_default();
        let epsg = keys.get(4..).and_then(|k| k.chunks_exact(4).find(|e| e[0] == 3072 && e[1] == 0)).map(|e| e[3]);
        if epsg != Some(EPSG) {
            return bad(format!("the grid is in EPSG:{}, expected EPSG:{EPSG} (WGS 84 / Mercator 41)", epsg.map_or("unknown".into(), |e| e.to_string())));
        }
        let (scale, tie) = (doubles(33550).unwrap_or_default(), doubles(33922).unwrap_or_default());
        if scale.len() < 2 || tie.len() < 6 || (scale[0] - scale[1]).abs() > 1e-6 {
            return bad("no usable georeferencing (square pixels and a tie point)");
        }
        let nodata = tags.get(&42113).and_then(|(_, d)| String::from_utf8_lossy(d).trim_matches(['\0', ' ']).parse::<f64>().ok());
        Ok(Tiff {
            file,
            width,
            height,
            rows_per_strip,
            compression,
            offsets,
            counts,
            cell: scale[0],
            x0: tie[3] - tie[0] * scale[0],
            y0: tie[4] + tie[1] * scale[1],
            nodata,
        })
    }

    /// One strip's pixels (`rows` rows of the image), as float32 little-endian bytes.
    fn decode(&self, raw: &[u8], rows: usize) -> Result<Vec<u8>> {
        let want = rows * self.width * 4;
        let mut out = Vec::with_capacity(want);
        match self.compression {
            1 => out.extend_from_slice(raw),
            5 => lzw(raw, &mut out)?,
            _ => {
                flate2::read::ZlibDecoder::new(raw).read_to_end(&mut out)?;
            }
        }
        if out.len() < want {
            return bad("a strip is shorter than the image says");
        }
        out.truncate(want);
        Ok(out)
    }
}

/// TIFF's LZW (most significant bit first, "early change", codes of 9 to 12 bits).
fn lzw(src: &[u8], out: &mut Vec<u8>) -> Result<()> {
    const CLEAR: u32 = 256;
    const END: u32 = 257;
    let mut prefix = [0u16; 4096];
    let mut suffix = [0u8; 4096];
    let mut len = [1u16; 4096];
    let (mut next, mut width) = (258u32, 9u32);
    let (mut bits, mut nbits, mut pos) = (0u32, 0u32, 0usize);
    let mut old: Option<(u32, u8)> = None; // the last code and the first byte of its string
    let emit = |code: u32, out: &mut Vec<u8>, prefix: &[u16; 4096], suffix: &[u8; 4096], len: &[u16; 4096]| {
        let l = len[code as usize] as usize;
        let start = out.len();
        out.resize(start + l, 0);
        let (mut c, mut i) = (code as usize, start + l - 1);
        while c >= 258 {
            out[i] = suffix[c];
            i -= 1;
            c = prefix[c] as usize;
        }
        out[i] = c as u8;
        out[start]
    };
    loop {
        while nbits < width {
            let Some(&b) = src.get(pos) else { return Ok(()) };
            bits = (bits << 8) | b as u32;
            pos += 1;
            nbits += 8;
        }
        let code = (bits >> (nbits - width)) & ((1 << width) - 1);
        nbits -= width;
        bits &= (1 << nbits) - 1;
        if code == END {
            return Ok(());
        }
        if code == CLEAR {
            (next, width, old) = (258, 9, None);
            continue;
        }
        let Some((o, o_first)) = old else {
            if code >= CLEAR {
                return bad("LZW data starts with a table code");
            }
            out.push(code as u8);
            old = Some((code, code as u8));
            continue;
        };
        let first;
        if code < next {
            first = emit(code, out, &prefix, &suffix, &len);
            if next < 4096 {
                (prefix[next as usize], suffix[next as usize], len[next as usize]) = (o as u16, first, len[o as usize] + 1);
            }
        } else if code == next && next < 4096 {
            first = o_first;
            (prefix[next as usize], suffix[next as usize], len[next as usize]) = (o as u16, first, len[o as usize] + 1);
            emit(code, out, &prefix, &suffix, &len);
        } else {
            // Some encoders write the final END code at a width one step off; the data is complete by then.
            if src.len() - pos <= 1 {
                return Ok(());
            }
            return bad("bad LZW code");
        }
        next += 1;
        if next + 1 >= (1 << width) && width < 12 {
            width += 1;
        }
        old = Some((code, first));
    }
}

// ---------- the store ---------------------------------------------------------------------

/// A level of the store: cells across and down, tiles across and down, each tile's place.
struct Level {
    w: usize,
    h: usize,
    tx: usize,
    index: Vec<(u64, u32)>,
}

fn level_sizes(w: usize, h: usize) -> Vec<(usize, usize)> {
    let mut v = vec![(w, h)];
    while v.last().is_some_and(|&(w, h)| w.max(h) > TILE) {
        let (w, h) = *v.last().unwrap();
        v.push((w.div_ceil(2), h.div_ceil(2)));
    }
    v
}

fn deflate(cells: &[i16]) -> Vec<u8> {
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
    let bytes: Vec<u8> = cells.iter().flat_map(|v| v.to_le_bytes()).collect();
    let _ = enc.write_all(&bytes);
    enc.finish().unwrap_or_default()
}

/// What an import made.
pub struct Summary {
    pub width: usize,
    pub height: usize,
    pub bytes: u64,
    pub area: [f64; 4],
}

/// Import NIWA's grid: `src` is the zip NIWA publishes or its `.tif`; the store is written
/// into `dir`. `progress(stage, done, total)` is called as it goes; `cancel` stops it.
pub fn import(src: &Path, dir: &Path, progress: &dyn Fn(&str, u64, u64), cancel: &AtomicBool) -> Result<Summary> {
    std::fs::create_dir_all(dir)?;
    let is_zip = src.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip"));
    let tmp = dir.join(".niwa-import.tif");
    let tif_path: PathBuf = if is_zip {
        let mut zip = zip::ZipArchive::new(File::open(src)?).map_err(|e| Error(e.to_string()))?;
        let name = (0..zip.len())
            .filter_map(|i| zip.name_for_index(i).and_then(|n| n.ok()).map(|n| (i, n.to_string())))
            .find(|(_, n)| n.to_ascii_lowercase().ends_with(".tif"))
            .ok_or(Error("no .tif in the zip".into()))?;
        let mut entry = zip.by_index(name.0).map_err(|e| Error(e.to_string()))?;
        let total = entry.size();
        let mut out = File::create(&tmp)?;
        let (mut buf, mut done) = (vec![0u8; 1 << 20], 0u64);
        loop {
            if cancel.load(Ordering::SeqCst) {
                drop(out);
                let _ = std::fs::remove_file(&tmp);
                return bad("cancelled");
            }
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            progress("NIWA bathymetry: unpacking", done, total);
        }
        tmp.clone()
    } else {
        src.to_path_buf()
    };
    let r = build(&tif_path, dir, progress, cancel);
    if is_zip {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn build(tif: &Path, dir: &Path, progress: &dyn Fn(&str, u64, u64), cancel: &AtomicBool) -> Result<Summary> {
    let t = Tiff::open(tif)?;
    let (w, h) = (t.width, t.height);
    if w.checked_mul(h).is_none_or(|n| n > 1_000_000_000) {
        return bad("the grid is too big");
    }
    let mut grid = vec![NODATA; w * h];
    let strips = h.div_ceil(t.rows_per_strip);
    // strips are read in batches and unpacked on all cores
    let batch = 256usize.div_ceil(t.rows_per_strip).max(1);
    let mut file = t.file.try_clone()?;
    for s0 in (0..strips).step_by(batch) {
        if cancel.load(Ordering::SeqCst) {
            return bad("cancelled");
        }
        let s1 = (s0 + batch).min(strips);
        let mut raw = Vec::with_capacity(s1 - s0);
        for s in s0..s1 {
            let mut b = vec![0u8; t.counts[s] as usize];
            file.seek(SeekFrom::Start(t.offsets[s]))?;
            file.read_exact(&mut b)?;
            raw.push(b);
        }
        let decoded: Vec<Result<Vec<u8>>> =
            raw.par_iter().enumerate().map(|(i, b)| t.decode(b, t.rows_per_strip.min(h - (s0 + i) * t.rows_per_strip))).collect();
        for (i, d) in decoded.into_iter().enumerate() {
            let d = d?;
            let row0 = (s0 + i) * t.rows_per_strip;
            for (c, v) in d.chunks_exact(4).enumerate() {
                let v = f32::from_le_bytes([v[0], v[1], v[2], v[3]]);
                grid[row0 * w + c] = store_value(v, t.nodata);
            }
        }
        progress("NIWA bathymetry: reading the grid", s1 as u64, strips as u64);
    }
    // coarser levels: 2 × 2 cells averaged
    let sizes = level_sizes(w, h);
    let mut levels_data: Vec<Vec<i16>> = vec![grid];
    for l in 1..sizes.len() {
        let (pw, ph) = sizes[l - 1];
        let (cw, ch) = sizes[l];
        let prev = &levels_data[l - 1];
        let mut cur = vec![NODATA; cw * ch];
        cur.par_chunks_mut(cw).enumerate().for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let (mut sum, mut n) = (0i32, 0i32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (px, py) = (x * 2 + dx, y * 2 + dy);
                    if px < pw && py < ph {
                        let v = prev[py * pw + px];
                        if v != NODATA {
                            sum += v as i32;
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    *out = ((sum as f32 / n as f32).round() as i32).clamp(-32767, 32767) as i16;
                }
            }
        });
        levels_data.push(cur);
    }
    // the file: every level's tiles in turn, then the index
    let part = dir.join(format!("{FILE}.part"));
    let mut out = std::io::BufWriter::new(File::create(&part)?);
    let mut pos = 0u64;
    let mut levels = Vec::new();
    let total_tiles: usize = sizes.iter().map(|&(w, h)| w.div_ceil(TILE) * h.div_ceil(TILE)).sum();
    let mut written = 0usize;
    for (l, &(lw, lh)) in sizes.iter().enumerate() {
        let (tx, ty) = (lw.div_ceil(TILE), lh.div_ceil(TILE));
        let data = &levels_data[l];
        let mut index = Vec::with_capacity(tx * ty);
        for j in 0..ty {
            if cancel.load(Ordering::SeqCst) {
                drop(out);
                let _ = std::fs::remove_file(&part);
                return bad("cancelled");
            }
            let blobs: Vec<Vec<u8>> = (0..tx)
                .into_par_iter()
                .map(|i| {
                    let mut cells = vec![NODATA; TILE * TILE];
                    let mut any = false;
                    for y in 0..TILE.min(lh - j * TILE) {
                        let from = (j * TILE + y) * lw + i * TILE;
                        let n = TILE.min(lw - i * TILE);
                        cells[y * TILE..y * TILE + n].copy_from_slice(&data[from..from + n]);
                        any |= data[from..from + n].iter().any(|&v| v != NODATA);
                    }
                    if any {
                        deflate(&cells)
                    } else {
                        Vec::new()
                    }
                })
                .collect();
            for b in blobs {
                index.push((pos, b.len() as u32));
                out.write_all(&b)?;
                pos += b.len() as u64;
                written += 1;
            }
            progress("NIWA bathymetry: writing", written as u64, total_tiles as u64);
        }
        levels.push(Level { w: lw, h: lh, tx, index });
    }
    let mut idx = Vec::new();
    for v in [w as u32, h as u32] {
        idx.extend_from_slice(&v.to_le_bytes());
    }
    for v in [t.x0, t.y0, t.cell] {
        idx.extend_from_slice(&v.to_le_bytes());
    }
    idx.extend_from_slice(&(levels.len() as u32).to_le_bytes());
    for lv in &levels {
        for v in lv.index.iter() {
            idx.extend_from_slice(&v.0.to_le_bytes());
            idx.extend_from_slice(&v.1.to_le_bytes());
        }
    }
    out.write_all(&idx)?;
    out.write_all(&(idx.len() as u64).to_le_bytes())?;
    out.write_all(MAGIC)?;
    out.flush()?;
    drop(out);
    let dest = dir.join(FILE);
    let _ = std::fs::remove_file(&dest);
    std::fs::rename(&part, &dest)?;
    let bytes = std::fs::metadata(&dest)?.len();
    let r = Grid { x0: t.x0, y0: t.y0, cell: t.cell, w, h };
    Ok(Summary { width: w, height: h, bytes, area: r.area() })
}

/// A height as stored: half-metres, or NODATA.
fn store_value(v: f32, nodata: Option<f64>) -> i16 {
    if !v.is_finite() || nodata.is_some_and(|n| ((v as f64) - n).abs() <= n.abs() * 1e-6) || !(-16000.0..=16000.0).contains(&v) {
        return NODATA;
    }
    (v * UNITS).round().clamp(-32767.0, 32767.0) as i16
}

/// Where the grid is: cell size and north-west corner in the projection, cells across and down.
struct Grid {
    x0: f64,
    y0: f64,
    cell: f64,
    w: usize,
    h: usize,
}

impl Grid {
    /// [west, south, east, north] in longitude (east may pass 180) and latitude.
    fn area(&self) -> [f64; 4] {
        let (x1, y1) = (self.x0 + self.w as f64 * self.cell, self.y0 - self.h as f64 * self.cell);
        let (w, n) = inverse(self.x0, self.y0);
        let (e, s) = inverse(x1, y1);
        [w, s, e, n]
    }
}

/// The imported grid, and caches of decoded tiles and drawn images.
pub struct NzBathy {
    file: Mutex<File>,
    grid: Grid,
    levels: Vec<Level>,
    bytes: u64,
    tiles: Mutex<lru::LruCache<(usize, usize, usize), Option<Arc<Vec<i16>>>>>,
    images: Mutex<lru::LruCache<(u8, u32, u32), Option<Arc<Vec<u8>>>>>,
}

impl NzBathy {
    /// The store in `dir`, if there is one (quick: only its index is read).
    pub fn open(dir: &Path) -> Option<NzBathy> {
        let path = dir.join(FILE);
        match Self::read(&path) {
            Ok(n) => Some(n),
            Err(e) => {
                if path.exists() {
                    eprintln!("NIWA bathymetry: {}: {e}", path.display());
                }
                None
            }
        }
    }

    fn read(path: &Path) -> Result<NzBathy> {
        let mut f = File::open(path)?;
        let bytes = f.metadata()?.len();
        if bytes < 12 {
            return bad("too short");
        }
        let mut tail = [0u8; 12];
        f.seek(SeekFrom::End(-12))?;
        f.read_exact(&mut tail)?;
        if &tail[8..] != MAGIC {
            return bad("not a NIWA bathymetry store");
        }
        let len = rd64(&tail) as usize;
        if len as u64 + 12 > bytes {
            return bad("bad index");
        }
        let mut idx = vec![0u8; len];
        f.seek(SeekFrom::End(-12 - len as i64))?;
        f.read_exact(&mut idx)?;
        if idx.len() < 36 {
            return bad("bad index");
        }
        let (w, h) = (rd32(&idx) as usize, rd32(&idx[4..]) as usize);
        let fl = |o: usize| f64::from_le_bytes(idx[o..o + 8].try_into().unwrap());
        let grid = Grid { x0: fl(8), y0: fl(16), cell: fl(24), w, h };
        let nl = rd32(&idx[32..]) as usize;
        let sizes = level_sizes(w, h);
        if nl != sizes.len() {
            return bad("bad levels");
        }
        let mut p = 36;
        let mut levels = Vec::new();
        for &(lw, lh) in &sizes {
            let (tx, ty) = (lw.div_ceil(TILE), lh.div_ceil(TILE));
            let end = p + tx * ty * 12;
            if end > idx.len() {
                return bad("bad index");
            }
            let index = idx[p..end].chunks_exact(12).map(|c| (rd64(c), rd32(&c[8..]))).collect();
            levels.push(Level { w: lw, h: lh, tx, index });
            p = end;
        }
        Ok(NzBathy {
            file: Mutex::new(f),
            grid,
            levels,
            bytes,
            tiles: Mutex::new(lru::LruCache::new(NonZeroUsize::new(512).unwrap())),
            images: Mutex::new(lru::LruCache::new(NonZeroUsize::new(512).unwrap())),
        })
    }

    /// Size of the store's file.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// [w, s, e, n] in longitude (east may pass 180) and latitude.
    pub fn area(&self) -> [f64; 4] {
        self.grid.area()
    }

    fn tile(&self, l: usize, i: usize, j: usize) -> Option<Arc<Vec<i16>>> {
        let key = (l, i, j);
        if let Some(t) = self.tiles.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return t.clone();
        }
        let lv = &self.levels[l];
        let (pos, len) = lv.index[j * lv.tx + i];
        let t = (len > 0)
            .then(|| {
                let mut raw = vec![0u8; len as usize];
                {
                    let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
                    f.seek(SeekFrom::Start(pos)).ok()?;
                    f.read_exact(&mut raw).ok()?;
                }
                let mut bytes = Vec::with_capacity(TILE * TILE * 2);
                flate2::read::DeflateDecoder::new(&raw[..]).read_to_end(&mut bytes).ok()?;
                (bytes.len() == TILE * TILE * 2).then(|| Arc::new(bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect::<Vec<i16>>()))
            })
            .flatten();
        self.tiles.lock().unwrap_or_else(|e| e.into_inner()).put(key, t.clone());
        t
    }

    fn cell(&self, l: usize, x: i64, y: i64) -> Option<f32> {
        let lv = &self.levels[l];
        if x < 0 || y < 0 || x >= lv.w as i64 || y >= lv.h as i64 {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        let t = self.tile(l, x / TILE, y / TILE)?;
        let v = t[(y % TILE) * TILE + x % TILE];
        (v != NODATA).then(|| v as f32 / UNITS)
    }

    /// Bilinear height at grid metres (nearest where a neighbour is missing).
    fn sample(&self, l: usize, gx: f64, gy: f64) -> Option<f32> {
        let size = self.grid.cell * (1u64 << l) as f64;
        let fx = (gx - self.grid.x0) / size - 0.5;
        let fy = (self.grid.y0 - gy) / size - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = ((fx - x0) as f32, (fy - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let v = [self.cell(l, x0, y0), self.cell(l, x0 + 1, y0), self.cell(l, x0, y0 + 1), self.cell(l, x0 + 1, y0 + 1)];
        match v {
            [Some(a), Some(b), Some(c), Some(d)] => Some((a * (1.0 - ax) + b * ax) * (1.0 - ay) + (c * (1.0 - ax) + d * ax) * ay),
            _ => {
                let k = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().enumerate().min_by(|a, b| {
                    let d = |(dx, dy): (f32, f32)| (ax - dx).powi(2) + (ay - dy).powi(2);
                    d(*a.1).total_cmp(&d(*b.1))
                })?;
                v[k.0].or_else(|| v.iter().flatten().next().copied())
            }
        }
    }

    /// Height (m, negative down) at a place, from the finest level.
    pub fn height_at(&self, lon: f64, lat: f64) -> Option<f32> {
        let (x, y) = forward(lon, lat);
        self.sample(0, x, y)
    }

    /// The image for map tile `z/x/y` (256 pixels, PNG), or None where there is no data.
    pub fn tile_png(&self, z: u8, x: u32, y: u32) -> Option<Arc<Vec<u8>>> {
        if z < MIN_ZOOM {
            return None;
        }
        if let Some(t) = self.images.lock().unwrap_or_else(|e| e.into_inner()).get(&(z, x, y)) {
            return t.clone();
        }
        let png = self.render(z, x, y).map(Arc::new);
        self.images.lock().unwrap_or_else(|e| e.into_inner()).put((z, x, y), png.clone());
        png
    }

    fn render(&self, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
        const N: usize = 256;
        const G: usize = N + 2; // a pixel of border all round, for the shading
        let n = (1u64 << z) as f64;
        let lon_at = |px: f64| (x as f64 + px / N as f64) / n * 360.0 - 180.0;
        let lat_at = |py: f64| {
            let t = std::f64::consts::PI * (1.0 - 2.0 * (y as f64 + py / N as f64) / n);
            t.sinh().atan().to_degrees()
        };
        // ground size of a pixel here, and the coarsest level at least that fine
        let lat_mid = lat_at(N as f64 / 2.0);
        let res = 40_075_016.686 * lat_mid.to_radians().cos() / (N as f64 * n);
        let cell_ground = self.grid.cell / scale(lat_mid);
        let l = (0..self.levels.len()).rev().find(|&l| cell_ground * (1u64 << l) as f64 <= res * 1.01).unwrap_or(0);
        let mut heights = vec![f32::NAN; G * G];
        let mut any = false;
        for gy in 0..G {
            let lat = lat_at(gy as f64 - 0.5);
            for gx in 0..G {
                let (mx, my) = forward(lon_at(gx as f64 - 0.5), lat);
                if let Some(v) = self.sample(l, mx, my) {
                    heights[gy * G + gx] = v;
                    any |= v <= 0.0;
                }
            }
        }
        any.then(|| crate::hires::shade(&heights, G, res, contour_step(z), colour, 5.0))
    }
}

/// Contour interval (m) at a zoom.
fn contour_step(z: u8) -> f32 {
    match z {
        12.. => 50.0,
        10 | 11 => 100.0,
        8 | 9 => 200.0,
        6 | 7 => 500.0,
        _ => 1000.0,
    }
}

/// Depth colour for a height (negative below the datum): pale in the shallows, dark in the deep.
fn colour(h: f32) -> [f32; 3] {
    const STOPS: &[(f32, [f32; 3])] = &[
        (0.0, [214.0, 238.0, 244.0]),
        (20.0, [190.0, 226.0, 240.0]),
        (50.0, [166.0, 210.0, 234.0]),
        (100.0, [140.0, 192.0, 226.0]),
        (200.0, [112.0, 168.0, 214.0]),
        (500.0, [84.0, 138.0, 194.0]),
        (1000.0, [62.0, 108.0, 170.0]),
        (2000.0, [44.0, 80.0, 140.0]),
        (4000.0, [30.0, 56.0, 108.0]),
        (6000.0, [20.0, 38.0, 84.0]),
        (10000.0, [12.0, 24.0, 60.0]),
    ];
    let d = -h;
    if d <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for w in STOPS.windows(2) {
        if d <= w[1].0 {
            let t = (d - w[0].0) / (w[1].0 - w[0].0);
            return [0, 1, 2].map(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * t);
        }
    }
    STOPS[STOPS.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TIFF's LZW as an encoder writes it (a clear code first, "early change").
    fn lzw_encode(src: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut acc, mut nacc) = (0u32, 0u32);
        let mut put = |code: u32, width: u32, out: &mut Vec<u8>| {
            acc = (acc << width) | code;
            nacc += width;
            while nacc >= 8 {
                out.push((acc >> (nacc - 8)) as u8);
                nacc -= 8;
            }
            acc &= (1 << nacc) - 1;
        };
        let mut table: std::collections::HashMap<(u32, u8), u32> = std::collections::HashMap::new();
        let (mut next, mut width) = (258u32, 9u32);
        put(256, width, &mut out);
        let mut cur: Option<u32> = None;
        for &b in src {
            let Some(c) = cur else {
                cur = Some(b as u32);
                continue;
            };
            if let Some(&code) = table.get(&(c, b)) {
                cur = Some(code);
                continue;
            }
            put(c, width, &mut out);
            table.insert((c, b), next);
            next += 1;
            if next + 1 > (1 << width) && width < 12 {
                width += 1;
            }
            if next >= 4094 {
                put(256, width, &mut out);
                table.clear();
                (next, width) = (258, 9);
            }
            cur = Some(b as u32);
        }
        if let Some(c) = cur {
            put(c, width, &mut out);
        }
        put(257, width, &mut out);
        if nacc > 0 {
            out.push((acc << (8 - nacc)) as u8);
        }
        out
    }

    #[test]
    fn lzw_round_trip() {
        let mut data: Vec<u8> = Vec::new();
        for i in 0..40_000u32 {
            data.push(((i * i / 7) % 251) as u8);
            data.push((i % 13) as u8);
        }
        let mut back = Vec::new();
        lzw(&lzw_encode(&data), &mut back).unwrap();
        assert_eq!(back, data);
        let flat = vec![7u8; 5000];
        let mut back = Vec::new();
        lzw(&lzw_encode(&flat), &mut back).unwrap();
        assert_eq!(back, flat);
    }

    #[test]
    fn projection_round_trips_and_covers_nz() {
        for (lon, lat) in [(174.0, -41.0), (-176.5, -44.0), (166.0, -47.0), (180.0, -30.0)] {
            let (x, y) = forward(lon, lat);
            let (l2, p2) = inverse(x, y);
            assert!((((l2 - lon) + 180.0).rem_euclid(360.0) - 180.0).abs() < 1e-7 && (p2 - lat).abs() < 1e-7, "{lon} {lat} -> {l2} {p2}");
        }
        // true scale at 41° S, more ground per grid metre towards the equator
        assert!((scale(-41.0) - 1.0).abs() < 1e-9);
        // NIWA's grid: its corners are in the south-west Pacific
        let g = Grid { x0: 4_795_705.6, y0: -2_067_835.8, cell: 250.0, w: 12116, h: 15391 };
        let a = g.area();
        assert!(a[0] > 150.0 && a[0] < 165.0 && a[2] > 185.0 && a[2] < 200.0 && a[1] < -55.0 && a[3] > -30.0, "{a:?}");
    }

    #[test]
    fn stored_values() {
        assert_eq!(store_value(-100.25, None), -201);
        assert_eq!(store_value(f32::NAN, None), NODATA);
        assert_eq!(store_value(-3.4e38, Some(-3.4e38)), NODATA);
        assert_eq!(store_value(30000.0, None), NODATA);
    }

    #[test]
    fn colours_darken_with_depth() {
        assert!(colour(-10.0)[0] > colour(-3000.0)[0]);
        assert_eq!(colour(5.0), colour(0.0));
    }

    /// NZBATHY_ZIP=<NIWA's zip or tif> cargo test --release -p chart --lib real_grid -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_grid() {
        let src = PathBuf::from(std::env::var("NZBATHY_ZIP").expect("NZBATHY_ZIP"));
        let dir = std::env::temp_dir().join("nzbathy-test");
        let _ = std::fs::remove_dir_all(&dir);
        let t = std::time::Instant::now();
        let s = import(&src, &dir, &|_, _, _| {}, &AtomicBool::new(false)).unwrap();
        println!("imported {}x{} in {:?}, {} MB, area {:?}", s.width, s.height, t.elapsed(), s.bytes / 1_000_000, s.area);
        let nz = NzBathy::open(&dir).unwrap();
        for (name, lon, lat) in [
            ("Cook Strait", 174.5, -41.3),
            ("Hikurangi trench", 178.5, -40.5),
            ("Kermadec trench", -176.5, -31.0),
            ("Mt Taranaki land", 174.06, -39.3),
            ("Pacific", -160.0, -41.0),
        ] {
            println!("{name}: {:?}", nz.height_at(lon, lat));
        }
        assert!(nz.height_at(174.5, -41.3).is_some_and(|h| h < 0.0 && h > -500.0));
        assert!(nz.height_at(-176.5, -31.0).is_some_and(|h| h < -3000.0));
        assert!(nz.height_at(174.06, -39.3).is_some_and(|h| h > 0.0));
        // Cook Strait at zoom 8 and 12
        for (z, x, y) in [(8u8, 252u32, 154u32), (5, 31, 19), (12, 4038, 2463)] {
            let png = nz.tile_png(z, x, y);
            println!("tile {z}/{x}/{y}: {:?} bytes", png.as_ref().map(|p| p.len()));
            if let Some(p) = png {
                std::fs::write(std::env::temp_dir().join(format!("nzb-{z}-{x}-{y}.png")), p.as_ref()).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
