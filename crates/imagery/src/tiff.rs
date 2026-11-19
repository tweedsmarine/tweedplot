//! Reading 8-bit images from TIFF and GeoTIFF files, tiled or in strips, on disk or on a web
//! server: RGB, RGBA, grey and palette images; no compression, LZW, deflate or JPEG; the
//! horizontal predictor; overviews (reduced-resolution copies, as cloud-optimised GeoTIFFs
//! carry); and the georeferencing (GeoKeys with ModelPixelScale and ModelTiepoint, or
//! ModelTransformation). Classic TIFF and BigTIFF.

use std::io::Read;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;

use crate::geo::{Affine, Crs};
use crate::render::{Block, Raster};
use crate::source::Source;
use crate::{err, Error, Result};

#[derive(Clone, Debug)]
struct Entry {
    tag: u16,
    typ: u16,
    count: u64,
    /// The value itself when it fits in the entry, else empty.
    inline: Vec<u8>,
    offset: u64,
}

/// One image in the file: the full resolution or an overview.
#[derive(Clone, Debug)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub block_w: usize,
    pub block_h: usize,
    /// Tiles (always whole) rather than strips (the last one shorter).
    tiled: bool,
    offsets: Vec<u64>,
    counts: Vec<u64>,
    compression: u16,
    predictor: u16,
    photometric: u16,
    spp: usize,
    alpha: bool,
    palette: Option<Arc<Vec<[u8; 4]>>>,
    jpeg_tables: Option<Vec<u8>>,
}

impl Level {
    fn blocks_x(&self) -> usize {
        self.width.div_ceil(self.block_w)
    }
    fn blocks_y(&self) -> usize {
        self.height.div_ceil(self.block_h)
    }
}

pub struct Tiff {
    src: Arc<dyn Source>,
    pub levels: Vec<Level>,
    /// The full-resolution image's georeferencing, when the file has it.
    pub geo: Option<(Crs, Affine)>,
    /// The EPSG code of the CRS, when the GeoKeys give one.
    pub epsg: Option<u32>,
    /// GDAL's "no data" value: pixels with every colour band equal to it are transparent.
    pub nodata: Option<u8>,
    cache: Mutex<LruCache<(usize, usize, usize), Option<Arc<Block>>>>,
    bounds: [f64; 4],
    res_m: f64,
}

struct Reader<'a> {
    src: &'a dyn Source,
    le: bool,
    big: bool,
}

impl Reader<'_> {
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
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[..8]);
        if self.le {
            u64::from_le_bytes(a)
        } else {
            u64::from_be_bytes(a)
        }
    }
    fn f64(&self, b: &[u8]) -> f64 {
        f64::from_bits(self.u64(b))
    }

    fn read_ifd(&self, off: u64) -> Result<(Vec<Entry>, u64)> {
        let (cnt_len, ent_len, next_len) = if self.big { (8, 20, 8) } else { (2, 12, 4) };
        let c = self.src.read_at(off, cnt_len)?;
        if c.len() < cnt_len {
            return err("TIFF: directory past the end of the file");
        }
        let n = if self.big { self.u64(&c) } else { self.u16(&c) as u64 };
        if n == 0 || n > 4000 {
            return err("TIFF: bad directory");
        }
        let body = self.src.read_at(off + cnt_len as u64, n as usize * ent_len + next_len)?;
        if body.len() < n as usize * ent_len + next_len {
            return err("TIFF: directory cut short");
        }
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            let e = &body[i * ent_len..(i + 1) * ent_len];
            let tag = self.u16(&e[0..]);
            let typ = self.u16(&e[2..]);
            let (count, val) = if self.big { (self.u64(&e[4..]), &e[12..20]) } else { (self.u32(&e[4..]) as u64, &e[8..12]) };
            let size = type_size(typ).saturating_mul(count);
            let (inline, offset) = if size <= val.len() as u64 {
                (val[..size as usize].to_vec(), 0)
            } else {
                (Vec::new(), if self.big { self.u64(val) } else { self.u32(val) as u64 })
            };
            out.push(Entry { tag, typ, count, inline, offset });
        }
        let nb = &body[n as usize * ent_len..];
        let next = if self.big { self.u64(nb) } else { self.u32(nb) as u64 };
        Ok((out, next))
    }

    fn bytes(&self, e: &Entry) -> Result<Vec<u8>> {
        if !e.inline.is_empty() || e.count == 0 {
            return Ok(e.inline.clone());
        }
        let size = type_size(e.typ).saturating_mul(e.count);
        if size > 256 * 1024 * 1024 {
            return err("TIFF: tag value too large");
        }
        let b = self.src.read_at(e.offset, size as usize)?;
        if (b.len() as u64) < size {
            return err("TIFF: tag value past the end of the file");
        }
        Ok(b)
    }

    /// An integer tag's values.
    fn ints(&self, e: &Entry) -> Result<Vec<u64>> {
        let b = self.bytes(e)?;
        let n = e.count as usize;
        Ok(match e.typ {
            1 | 7 => b.iter().map(|&v| v as u64).collect(),
            3 => (0..n).map(|i| self.u16(&b[i * 2..]) as u64).collect(),
            4 | 13 => (0..n).map(|i| self.u32(&b[i * 4..]) as u64).collect(),
            16 | 18 => (0..n).map(|i| self.u64(&b[i * 8..])).collect(),
            _ => return err(format!("TIFF: tag {} is not an integer", e.tag)),
        })
    }

    fn doubles(&self, e: &Entry) -> Result<Vec<f64>> {
        let b = self.bytes(e)?;
        let n = e.count as usize;
        Ok(match e.typ {
            12 => (0..n).map(|i| self.f64(&b[i * 8..])).collect(),
            11 => (0..n).map(|i| f32::from_bits(self.u32(&b[i * 4..])) as f64).collect(),
            _ => self.ints(e)?.into_iter().map(|v| v as f64).collect(),
        })
    }
}

fn type_size(t: u16) -> u64 {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 | 16 | 17 | 18 => 8,
        _ => 1,
    }
}

fn find(es: &[Entry], tag: u16) -> Option<&Entry> {
    es.iter().find(|e| e.tag == tag)
}

impl Tiff {
    pub fn open(src: Arc<dyn Source>) -> Result<Tiff> {
        let h = src.read_at(0, 16)?;
        if h.len() < 8 {
            return err("not a TIFF file");
        }
        let le = match &h[0..2] {
            b"II" => true,
            b"MM" => false,
            _ => return err("not a TIFF file"),
        };
        let mut r = Reader { src: &*src, le, big: false };
        let magic = r.u16(&h[2..]);
        let first = match magic {
            42 => r.u32(&h[4..]) as u64,
            43 => {
                if h.len() < 16 {
                    return err("BigTIFF header cut short");
                }
                r.big = true;
                r.u64(&h[8..])
            }
            _ => return err("not a TIFF file"),
        };
        let mut levels: Vec<Level> = Vec::new();
        let mut geo = None;
        let mut epsg = None;
        let mut nodata = None;
        let mut off = first;
        let mut seen = 0;
        while off != 0 && seen < 32 {
            seen += 1;
            let (es, next) = r.read_ifd(off)?;
            off = next;
            let subfile = find(&es, 254).map(|e| r.ints(e)).transpose()?.and_then(|v| v.first().copied()).unwrap_or(0);
            // masks (bit 2) are skipped; reduced-resolution copies (bit 0) after the first image
            if subfile & 4 != 0 || (levels.is_empty() && subfile & 1 != 0) {
                continue;
            }
            let level = match read_level(&r, &es) {
                Ok(l) => l,
                Err(e) if levels.is_empty() => return Err(e),
                Err(_) => continue,
            };
            if levels.is_empty() {
                (geo, epsg) = read_geo(&r, &es)?;
                if let Some(e) = find(&es, 42113) {
                    let s = String::from_utf8_lossy(&r.bytes(e)?).trim_matches(char::from(0)).trim().to_string();
                    nodata = s.parse::<f64>().ok().filter(|v| (0.0..=255.0).contains(v) && v.fract() == 0.0).map(|v| v as u8);
                }
            } else {
                if level.spp != levels[0].spp || level.width >= levels[0].width {
                    continue;
                }
            }
            levels.push(level);
        }
        if levels.is_empty() {
            return err("TIFF: no image");
        }
        levels[1..].sort_by_key(|l| std::cmp::Reverse(l.width));
        let mut t =
            Tiff { src, levels, geo, epsg, nodata, cache: Mutex::new(LruCache::new(NonZeroUsize::new(48).expect("non-zero"))), bounds: [0.0; 4], res_m: 0.0 };
        if let Some((crs, aff)) = t.geo {
            t.bounds = crate::render::bounds_of(&crs, &aff, t.levels[0].width, t.levels[0].height);
            t.res_m = crate::render::resolution_of(&crs, &aff, t.levels[0].width, t.levels[0].height);
        }
        Ok(t)
    }

    /// Raw block bytes, decompressed and un-predicted, as stored (samples per pixel × 8 bits).
    fn decode(&self, level: usize, bx: usize, by: usize) -> Result<Option<Block>> {
        let l = &self.levels[level];
        let (nx, ny) = (l.blocks_x(), l.blocks_y());
        if bx >= nx || by >= ny {
            return Ok(None);
        }
        let i = by * nx + bx;
        let (Some(&off), Some(&cnt)) = (l.offsets.get(i), l.counts.get(i)) else { return Ok(None) };
        if off == 0 || cnt == 0 {
            // a sparse COG leaves empty blocks out
            return Ok(None);
        }
        if cnt > 64 * 1024 * 1024 {
            return err("TIFF: block too large");
        }
        let raw = self.src.read_at(off, cnt as usize)?;
        // strips: the last is shorter; tiles are always whole
        let rows = if l.tiled { l.block_h } else { l.block_h.min(l.height - by * l.block_h) };
        let (w, h) = (l.block_w, rows);
        let n = w * h * l.spp;
        let mut px = match l.compression {
            1 => raw,
            5 => {
                let mut d = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
                let mut out = Vec::with_capacity(n);
                let r = d.into_stream(&mut out).decode_all(&raw[..]);
                if let Err(e) = r.status {
                    if out.len() < n {
                        return err(format!("TIFF LZW: {e}"));
                    }
                }
                out
            }
            8 | 32946 => {
                let mut out = Vec::with_capacity(n);
                flate2::read::ZlibDecoder::new(&raw[..]).read_to_end(&mut out).map_err(|e| Error(format!("TIFF deflate: {e}")))?;
                out
            }
            7 => return self.jpeg(l, &raw, w, h).map(Some),
            c => return err(format!("TIFF: compression {c} is not supported")),
        };
        if px.len() < n {
            px.resize(n, 0);
        }
        if l.predictor == 2 {
            let s = l.spp;
            for row in px[..n].chunks_mut(w * s) {
                for i in s..row.len() {
                    row[i] = row[i].wrapping_add(row[i - s]);
                }
            }
        }
        Ok(Some(self.to_block(l, &px[..n], w, h)))
    }

    fn jpeg(&self, l: &Level, raw: &[u8], w: usize, h: usize) -> Result<Block> {
        use zune_core::colorspace::ColorSpace;
        // abbreviated JPEG: the tables from JPEGTables, then the block without its SOI
        let mut stream;
        let data: &[u8] = match &l.jpeg_tables {
            Some(t) if t.len() > 4 && raw.len() > 2 => {
                stream = t[..t.len() - 2].to_vec();
                stream.extend_from_slice(&raw[2..]);
                &stream
            }
            _ => raw,
        };
        let mut opts = zune_core::options::DecoderOptions::default().set_strict_mode(false);
        if l.photometric == 2 {
            // RGB inside the JPEG: no YCbCr conversion
            opts = opts.jpeg_set_out_colorspace(ColorSpace::YCbCr);
        } else if l.spp == 1 {
            opts = opts.jpeg_set_out_colorspace(ColorSpace::Luma);
        } else {
            opts = opts.jpeg_set_out_colorspace(ColorSpace::RGB);
        }
        let mut d = zune_jpeg::JpegDecoder::new_with_options(data, opts);
        let px = d.decode().map_err(|e| Error(format!("TIFF JPEG: {e:?}")))?;
        let (jw, jh) = d.dimensions().unwrap_or((w, h));
        let comps = if l.spp == 1 { 1 } else { 3 };
        let mut out = vec![0u8; w * h * comps];
        for row in 0..h.min(jh) {
            let n = w.min(jw) * comps;
            out[row * w * comps..row * w * comps + n].copy_from_slice(&px[row * jw * comps..row * jw * comps + n]);
        }
        let mut lv = l.clone();
        lv.spp = comps;
        lv.alpha = false;
        Ok(self.to_block(&lv, &out, w, h))
    }

    fn to_block(&self, l: &Level, px: &[u8], w: usize, h: usize) -> Block {
        if let Some(p) = &l.palette {
            if l.spp == 1 {
                return Block { w, h, data: px.to_vec(), palette: Some(p.clone()) };
            }
        }
        let mut out = vec![0u8; w * h * 4];
        let nd = self.nodata;
        for (i, o) in out.chunks_exact_mut(4).enumerate() {
            let s = &px[i * l.spp..(i + 1) * l.spp];
            let rgba = match l.spp {
                1 | 2 => {
                    let g = if l.photometric == 0 { 255 - s[0] } else { s[0] };
                    [g, g, g, if l.spp == 2 { s[1] } else { 255 }]
                }
                3 => [s[0], s[1], s[2], 255],
                _ => [s[0], s[1], s[2], if l.alpha { s[3] } else { 255 }],
            };
            let blank = nd.is_some_and(|v| s[..l.spp.min(3)].iter().all(|&b| b == v));
            o.copy_from_slice(&if blank { [0, 0, 0, 0] } else { rgba });
        }
        Block { w, h, data: out, palette: None }
    }
}

fn read_level(r: &Reader, es: &[Entry]) -> Result<Level> {
    let one = |tag: u16| -> Result<Option<u64>> { Ok(find(es, tag).map(|e| r.ints(e)).transpose()?.and_then(|v| v.first().copied())) };
    let width = one(256)?.ok_or_else(|| Error("TIFF: no width".into()))? as usize;
    let height = one(257)?.ok_or_else(|| Error("TIFF: no height".into()))? as usize;
    let spp = one(277)?.unwrap_or(1) as usize;
    let bits = find(es, 258).map(|e| r.ints(e)).transpose()?.unwrap_or_else(|| vec![1]);
    if bits.iter().any(|&b| b != 8) {
        return err(format!("TIFF: only 8-bit images can be shown (this one has {} bits per sample)", bits.first().unwrap_or(&0)));
    }
    if one(284)?.unwrap_or(1) != 1 && spp > 1 {
        return err("TIFF: separate colour planes are not supported");
    }
    if one(339)?.is_some_and(|f| f != 1) {
        return err("TIFF: only unsigned 8-bit samples can be shown");
    }
    let compression = one(259)?.unwrap_or(1) as u16;
    let predictor = one(317)?.unwrap_or(1) as u16;
    let photometric = one(262)?.unwrap_or(if spp >= 3 { 2 } else { 1 }) as u16;
    let tiled = one(322)?.is_some();
    let (block_w, block_h, offsets, counts) = if let Some(tw) = one(322)? {
        let th = one(323)?.unwrap_or(tw);
        let o = find(es, 324).ok_or_else(|| Error("TIFF: no tile offsets".into()))?;
        let c = find(es, 325).ok_or_else(|| Error("TIFF: no tile sizes".into()))?;
        (tw as usize, th as usize, r.ints(o)?, r.ints(c)?)
    } else {
        let rps = one(278)?.unwrap_or(height as u64).min(height as u64) as usize;
        let o = find(es, 273).ok_or_else(|| Error("TIFF: no strip offsets".into()))?;
        let c = find(es, 279).ok_or_else(|| Error("TIFF: no strip sizes".into()))?;
        (width, rps.max(1), r.ints(o)?, r.ints(c)?)
    };
    if block_w == 0 || block_h == 0 || width == 0 || height == 0 {
        return err("TIFF: empty image");
    }
    let extra = find(es, 338).map(|e| r.ints(e)).transpose()?.unwrap_or_default();
    let alpha = spp >= 4 && extra.first().is_some_and(|&v| v == 1 || v == 2);
    let palette = if photometric == 3 {
        let e = find(es, 320).ok_or_else(|| Error("TIFF: palette image without a colour map".into()))?;
        let v = r.ints(e)?;
        let n = v.len() / 3;
        Some(Arc::new((0..n).map(|i| [(v[i] >> 8) as u8, (v[n + i] >> 8) as u8, (v[2 * n + i] >> 8) as u8, 255]).collect()))
    } else {
        None
    };
    if !matches!(photometric, 0..=3 | 6) {
        return err(format!("TIFF: colour model {photometric} is not supported"));
    }
    let jpeg_tables = find(es, 347).map(|e| r.bytes(e)).transpose()?;
    Ok(Level { width, height, block_w, block_h, tiled, offsets, counts, compression, predictor, photometric, spp, alpha, palette, jpeg_tables })
}

fn read_geo(r: &Reader, es: &[Entry]) -> Result<(Option<(Crs, Affine)>, Option<u32>)> {
    let keys = match find(es, 34735) {
        Some(e) => r.ints(e)?,
        None => return Ok((None, None)),
    };
    let mut model = 0;
    let mut raster_type = 1;
    let mut projected = None;
    let mut geographic = None;
    if keys.len() >= 4 {
        let n = keys[3] as usize;
        for k in 0..n {
            let Some(e) = keys.get(4 + k * 4..8 + k * 4) else { break };
            if e[1] != 0 {
                continue;
            }
            match e[0] {
                1024 => model = e[3],
                1025 => raster_type = e[3],
                2048 => geographic = Some(e[3] as u32),
                3072 => projected = Some(e[3] as u32),
                _ => {}
            }
        }
    }
    let epsg = match (model, projected, geographic) {
        (2, _, Some(g)) => Some(g),
        (_, Some(p), _) if p != 32767 => Some(p),
        (_, None, Some(g)) => Some(g),
        _ => None,
    };
    let Some(code) = epsg else { return err("GeoTIFF: no EPSG code for its projection (only EPSG-coded projections are supported)") };
    let Some(crs) = Crs::from_epsg(code) else { return err(format!("GeoTIFF: projection EPSG:{code} is not supported yet")) };
    let mut aff = if let Some(e) = find(es, 34264) {
        let m = r.doubles(e)?;
        if m.len() < 16 {
            return err("GeoTIFF: bad ModelTransformation");
        }
        Affine { a: m[3], b: m[0], c: m[1], d: m[7], e: m[4], f: m[5] }
    } else {
        let s = find(es, 33550).map(|e| r.doubles(e)).transpose()?.ok_or_else(|| Error("GeoTIFF: no pixel scale".into()))?;
        let t = find(es, 33922).map(|e| r.doubles(e)).transpose()?.ok_or_else(|| Error("GeoTIFF: no tie point".into()))?;
        if s.len() < 2 || t.len() < 6 {
            return err("GeoTIFF: bad pixel scale or tie point");
        }
        Affine { a: t[3] - t[0] * s[0], b: s[0], c: 0.0, d: t[4] + t[1] * s[1], e: 0.0, f: -s[1] }
    };
    if raster_type == 2 {
        // PixelIsPoint: the coordinates are the pixel centres
        aff.a -= 0.5 * (aff.b + aff.c);
        aff.d -= 0.5 * (aff.e + aff.f);
    }
    Ok((Some((crs, aff)), Some(code)))
}

impl Raster for Tiff {
    fn size(&self) -> (usize, usize) {
        (self.levels[0].width, self.levels[0].height)
    }
    fn levels(&self) -> usize {
        self.levels.len()
    }
    fn level_size(&self, level: usize) -> (usize, usize) {
        (self.levels[level].width, self.levels[level].height)
    }
    fn block_size(&self, level: usize) -> (usize, usize) {
        (self.levels[level].block_w, self.levels[level].block_h)
    }
    fn block(&self, level: usize, bx: usize, by: usize) -> Option<Arc<Block>> {
        if let Some(b) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&(level, bx, by)) {
            return b.clone();
        }
        let b = match self.decode(level, bx, by) {
            Ok(b) => b.map(Arc::new),
            Err(e) => {
                eprintln!("image block {level}/{bx}/{by}: {e}");
                None
            }
        };
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).put((level, bx, by), b.clone());
        b
    }
    fn to_pixel(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let (crs, aff) = self.geo.as_ref()?;
        let (x, y) = crs.forward(lon, lat);
        aff.invert(x, y)
    }
    fn bounds(&self) -> [f64; 4] {
        self.bounds
    }
    fn resolution_m(&self) -> f64 {
        self.res_m
    }
}

impl Tiff {
    /// Every block of a level, decoded (for copying a window out of a remote image).
    pub fn block_bytes(&self, level: usize, bx: usize, by: usize) -> Result<Option<Block>> {
        self.decode(level, bx, by)
    }
}
