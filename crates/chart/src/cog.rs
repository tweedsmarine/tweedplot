//! Reading tiled GeoTIFFs (cloud-optimised GeoTIFFs as LINZ publishes its elevation and
//! coastal models): the full-resolution image and its overviews, tile by tile, from a file
//! on disk. Tiles compressed with LERC (optionally inside deflate) or not compressed;
//! classic TIFF and BigTIFF; the georeferencing from ModelTiepoint and ModelPixelScale;
//! "no data" from GDAL's tag.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::lerc::{self, Raster};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Format(String),
    Lerc(lerc::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "GeoTIFF: {e}"),
            Error::Format(s) => write!(f, "GeoTIFF: {s}"),
            Error::Lerc(e) => write!(f, "GeoTIFF tile: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

fn bad(s: impl Into<String>) -> Error {
    Error::Format(s.into())
}

/// One resolution of the image.
#[derive(Debug, Clone)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub tile_w: usize,
    pub tile_h: usize,
    offsets: Vec<u64>,
    pub(crate) counts: Vec<u64>,
    compression: u16,
    /// LERC's extra compression (LercParameters): 0 none, 1 deflate, 2 zstd.
    lerc_extra: u32,
    sample_format: u16,
    bits: u16,
    /// Ground size of a pixel (x, y) in the image's units.
    pub pixel: (f64, f64),
}

impl Level {
    pub fn tiles_x(&self) -> usize {
        self.width.div_ceil(self.tile_w)
    }
    pub fn tiles_y(&self) -> usize {
        self.height.div_ceil(self.tile_h)
    }
}

/// A tiled GeoTIFF on disk.
#[derive(Debug)]
pub struct Cog {
    pub path: PathBuf,
    /// Full resolution first, then the overviews, coarser and coarser.
    pub levels: Vec<Level>,
    /// The image's top-left corner (x, y) in its coordinate system.
    pub origin: (f64, f64),
    pub nodata: Option<f64>,
    file: Mutex<File>,
}

#[derive(Clone, Copy)]
struct Order {
    le: bool,
    big: bool,
}

impl Order {
    fn u16(&self, b: &[u8]) -> u16 {
        let a = [b[0], b[1]];
        if self.le {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        }
    }
    fn u32(&self, b: &[u8]) -> u32 {
        let a = [b[0], b[1], b[2], b[3]];
        if self.le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        }
    }
    fn u64(&self, b: &[u8]) -> u64 {
        let a: [u8; 8] = b[..8].try_into().unwrap();
        if self.le {
            u64::from_le_bytes(a)
        } else {
            u64::from_be_bytes(a)
        }
    }
    fn f64(&self, b: &[u8]) -> f64 {
        f64::from_bits(self.u64(b))
    }
}

/// A tag's values, as numbers (or text for ASCII).
#[derive(Debug, Clone)]
enum Val {
    Nums(Vec<f64>),
    Text(String),
}

impl Val {
    fn nums(&self) -> &[f64] {
        match self {
            Val::Nums(v) => v,
            Val::Text(_) => &[],
        }
    }
    fn first(&self) -> Option<f64> {
        self.nums().first().copied()
    }
}

fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 | 16 | 17 | 18 => 8,
        _ => 0,
    }
}

fn read_at(f: &mut File, off: u64, len: usize) -> Result<Vec<u8>, Error> {
    f.seek(SeekFrom::Start(off))?;
    let mut v = vec![0u8; len];
    f.read_exact(&mut v)?;
    Ok(v)
}

/// The tags of the IFD at `off`, and the next IFD's offset.
fn read_ifd(f: &mut File, o: Order, off: u64) -> Result<(Vec<(u16, Val)>, u64), Error> {
    let (count, entry, head) = if o.big {
        (read_at(f, off, 8).map(|b| o.u64(&b))?, 20usize, 8u64)
    } else {
        (read_at(f, off, 2).map(|b| o.u16(&b) as u64)?, 12usize, 2u64)
    };
    if count > 4096 {
        return Err(bad("too many tags"));
    }
    let raw = read_at(
        f,
        off + head,
        count as usize * entry + if o.big { 8 } else { 4 },
    )?;
    let mut tags = Vec::new();
    for i in 0..count as usize {
        let e = &raw[i * entry..(i + 1) * entry];
        let tag = o.u16(&e[0..2]);
        let ty = o.u16(&e[2..4]);
        let (n, inline, voff) = if o.big {
            (o.u64(&e[4..12]), &e[12..20], o.u64(&e[12..20]))
        } else {
            (o.u32(&e[4..8]) as u64, &e[8..12], o.u32(&e[8..12]) as u64)
        };
        let size = type_size(ty);
        if size == 0 || n > 50_000_000 {
            continue;
        }
        let len = size * n as usize;
        let bytes = if len <= inline.len() {
            inline[..len].to_vec()
        } else {
            read_at(f, voff, len)?
        };
        let val = match ty {
            2 => Val::Text(
                String::from_utf8_lossy(&bytes)
                    .trim_end_matches('\0')
                    .trim()
                    .to_string(),
            ),
            _ => Val::Nums(
                (0..n as usize)
                    .map(|k| {
                        let b = &bytes[k * size..(k + 1) * size];
                        match ty {
                            1 | 7 => b[0] as f64,
                            6 => b[0] as i8 as f64,
                            3 => o.u16(b) as f64,
                            8 => o.u16(b) as i16 as f64,
                            4 | 13 => o.u32(b) as f64,
                            9 => o.u32(b) as i32 as f64,
                            5 => o.u32(&b[0..4]) as f64 / o.u32(&b[4..8]).max(1) as f64,
                            10 => {
                                o.u32(&b[0..4]) as i32 as f64
                                    / (o.u32(&b[4..8]) as i32).max(1) as f64
                            }
                            11 => f32::from_bits(o.u32(b)) as f64,
                            12 => o.f64(b),
                            16 | 18 => o.u64(b) as f64,
                            17 => o.u64(b) as i64 as f64,
                            _ => 0.0,
                        }
                    })
                    .collect(),
            ),
        };
        tags.push((tag, val));
    }
    let tail = &raw[count as usize * entry..];
    let next = if o.big {
        o.u64(tail)
    } else {
        o.u32(tail) as u64
    };
    Ok((tags, next))
}

impl Cog {
    pub fn open(path: &Path) -> Result<Cog, Error> {
        let mut f = File::open(path)?;
        let head = read_at(&mut f, 0, 16)?;
        let le = match &head[0..2] {
            b"II" => true,
            b"MM" => false,
            _ => return Err(bad("not a TIFF")),
        };
        let mut o = Order { le, big: false };
        let mut ifd = match o.u16(&head[2..4]) {
            42 => o.u32(&head[4..8]) as u64,
            43 => {
                o.big = true;
                o.u64(&head[8..16])
            }
            _ => return Err(bad("not a TIFF")),
        };
        let mut levels: Vec<Level> = Vec::new();
        let (mut origin, mut base_pixel, mut nodata) = (None, None, None);
        let mut seen = 0;
        while ifd != 0 && seen < 64 {
            seen += 1;
            let (tags, next) = read_ifd(&mut f, o, ifd)?;
            ifd = next;
            let get = |t: u16| tags.iter().find(|(k, _)| *k == t).map(|(_, v)| v);
            let subfile = get(254).and_then(Val::first).unwrap_or(0.0) as u32;
            // masks (GDAL's internal nodata masks) are skipped
            if subfile & 4 != 0 {
                continue;
            }
            let num = |t: u16| get(t).and_then(Val::first);
            let (Some(w), Some(h)) = (num(256), num(257)) else {
                continue;
            };
            let (Some(tw), Some(th)) = (num(322), num(323)) else {
                return Err(bad("not tiled (strips aren't supported)"));
            };
            let offsets: Vec<u64> = get(324)
                .map(|v| v.nums().iter().map(|&x| x as u64).collect())
                .unwrap_or_default();
            let counts: Vec<u64> = get(325)
                .map(|v| v.nums().iter().map(|&x| x as u64).collect())
                .unwrap_or_default();
            let spp = num(277).unwrap_or(1.0) as usize;
            if spp != 1 {
                return Err(bad("only single-band images are read"));
            }
            if levels.is_empty() {
                if let (Some(s), Some(tp)) = (get(33550), get(33922)) {
                    let (s, tp) = (s.nums(), tp.nums());
                    if s.len() >= 2 && tp.len() >= 6 {
                        base_pixel = Some((s[0], s[1]));
                        origin = Some((tp[3] - tp[0] * s[0], tp[4] + tp[1] * s[1]));
                    }
                }
                if let Some(Val::Text(t)) = get(42113) {
                    nodata = t.parse::<f64>().ok();
                }
            }
            let (bw, bh) = levels
                .first()
                .map_or((w, h), |l| (l.width as f64, l.height as f64));
            let bp = base_pixel.unwrap_or((1.0, 1.0));
            let lerc_params = get(50674).map(|v| v.nums().to_vec()).unwrap_or_default();
            levels.push(Level {
                width: w as usize,
                height: h as usize,
                tile_w: tw as usize,
                tile_h: th as usize,
                offsets,
                counts,
                compression: num(259).unwrap_or(1.0) as u16,
                lerc_extra: lerc_params.get(1).copied().unwrap_or(0.0) as u32,
                sample_format: num(339).unwrap_or(1.0) as u16,
                bits: num(258).unwrap_or(32.0) as u16,
                pixel: (bp.0 * bw / w, bp.1 * bh / h),
            });
        }
        if levels.is_empty() {
            return Err(bad("no image"));
        }
        // coarsest last
        levels[1..].sort_by_key(|l| std::cmp::Reverse(l.width));
        let origin =
            origin.ok_or_else(|| bad("no georeferencing (ModelTiepoint/ModelPixelScale)"))?;
        Ok(Cog {
            path: path.to_path_buf(),
            levels,
            origin,
            nodata,
            file: Mutex::new(f),
        })
    }

    /// [min x, min y, max x, max y] in the image's coordinate system.
    pub fn bounds(&self) -> [f64; 4] {
        let l = &self.levels[0];
        let (x0, y1) = self.origin;
        [
            x0,
            y1 - l.height as f64 * l.pixel.1,
            x0 + l.width as f64 * l.pixel.0,
            y1,
        ]
    }

    /// Tile (`tx`, `ty`) of `level`, decoded; "no data" pixels are invalid.
    pub fn tile(&self, level: usize, tx: usize, ty: usize) -> Result<Raster, Error> {
        let l = self.levels.get(level).ok_or_else(|| bad("no such level"))?;
        let i = ty * l.tiles_x() + tx;
        let (Some(&off), Some(&len)) = (l.offsets.get(i), l.counts.get(i)) else {
            return Err(bad("no such tile"));
        };
        let empty = || Raster {
            width: l.tile_w,
            height: l.tile_h,
            depth: 1,
            data: vec![0.0; l.tile_w * l.tile_h],
            valid: Some(vec![false; l.tile_w * l.tile_h]),
        };
        if len == 0 {
            return Ok(empty()); // a sparse COG leaves empty tiles out
        }
        let bytes = {
            let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
            read_at(&mut f, off, len as usize)?
        };
        let mut r = match l.compression {
            34887 => {
                let blob = match l.lerc_extra {
                    0 => bytes,
                    1 => {
                        let mut out = Vec::new();
                        flate2::read::ZlibDecoder::new(&bytes[..]).read_to_end(&mut out)?;
                        out
                    }
                    n => {
                        return Err(bad(format!(
                            "LERC with extra compression {n} (zstd) isn't supported"
                        )))
                    }
                };
                lerc::decode(&blob).map_err(Error::Lerc)?
            }
            1 if l.sample_format == 3 && l.bits == 32 => {
                let n = l.tile_w * l.tile_h;
                let data: Vec<f32> = bytes
                    .chunks(4)
                    .take(n)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap_or([0; 4])))
                    .collect();
                Raster {
                    width: l.tile_w,
                    height: l.tile_h,
                    depth: 1,
                    data,
                    valid: None,
                }
            }
            c => return Err(bad(format!("compression {c} isn't supported"))),
        };
        if let Some(nd) = self.nodata {
            let nd = nd as f32;
            if r.data.iter().any(|v| *v == nd) {
                let mut valid = r.valid.take().unwrap_or_else(|| vec![true; r.data.len()]);
                for (k, v) in r.data.iter().enumerate() {
                    if *v == nd {
                        valid[k] = false;
                    }
                }
                r.valid = Some(valid);
            }
        }
        Ok(r)
    }

    /// The value at (`x`, `y`) in the image's coordinates, at full resolution (nearest pixel).
    pub fn value_at(&self, x: f64, y: f64) -> Option<f32> {
        let l = &self.levels[0];
        let px = ((x - self.origin.0) / l.pixel.0).floor();
        let py = ((self.origin.1 - y) / l.pixel.1).floor();
        if px < 0.0 || py < 0.0 || px >= l.width as f64 || py >= l.height as f64 {
            return None;
        }
        let (px, py) = (px as usize, py as usize);
        let t = self.tile(0, px / l.tile_w, py / l.tile_h).ok()?;
        t.get(px % l.tile_w, py % l.tile_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dem(x: usize, y: usize) -> Option<f32> {
        let (xf, yf) = (x as f32, y as f32);
        let v = -20.0 + 0.12 * xf + 0.03 * yf + 1.5 * (xf / 9.0).sin() * (yf / 13.0).cos();
        (v <= 1.0).then_some(v)
    }

    #[test]
    fn reads_a_lerc_cog() {
        let c = Cog::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/coastal_dem.tiff"))
            .unwrap();
        assert_eq!(c.levels.len(), 3);
        assert_eq!(
            (c.levels[0].width, c.levels[1].width, c.levels[2].width),
            (256, 128, 64)
        );
        assert_eq!(c.levels[2].pixel, (4.0, 4.0));
        assert_eq!(c.bounds(), [1741600.0, 5672144.0, 1741856.0, 5672400.0]);
        assert_eq!(c.nodata, Some(-9999.0));
        // every pixel of the full image, within LERC's error, nodata where there was land
        for ty in 0..2 {
            for tx in 0..2 {
                let t = c.tile(0, tx, ty).unwrap();
                for y in 0..128 {
                    for x in 0..128 {
                        let (gx, gy) = (tx * 128 + x, ty * 128 + y);
                        match (t.get(x, y), dem(gx, gy)) {
                            (Some(a), Some(b)) => {
                                assert!((a - b).abs() < 0.0011, "{gx},{gy}: {a} v {b}")
                            }
                            (None, None) => {}
                            (a, b) => panic!("{gx},{gy}: {a:?} v {b:?}"),
                        }
                    }
                }
            }
        }
        // an overview: every 4th pixel
        let t = c.tile(2, 0, 0).unwrap();
        let (a, b) = (t.get(5, 7), dem(20, 28));
        assert!(
            a.zip(b).is_some_and(|(a, b)| (a - b).abs() < 0.0011),
            "{a:?} {b:?}"
        );
        // by position: 10.5 m east, 3.2 m south of the corner
        let v = c.value_at(1741610.5, 5672396.8).unwrap();
        assert!((v - dem(10, 3).unwrap()).abs() < 0.0011);
        assert!(c.value_at(1741500.0, 5672300.0).is_none());
    }
}
