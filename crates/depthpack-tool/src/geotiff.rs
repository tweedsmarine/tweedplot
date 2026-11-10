//! A float GeoTIFF (LINZ's depth model tiles): its georeferencing and its values, read a
//! chunk (strip or tile) at a time with a small cache, so big files never sit in memory.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::num::NonZeroUsize;
use std::path::Path;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

pub struct GeoTiff {
    dec: Decoder<BufReader<File>>,
    pub width: u32,
    pub height: u32,
    /// Map position of the top-left corner of the top-left pixel.
    pub x0: f64,
    pub y0: f64,
    /// Pixel size (positive both ways: y goes down the image).
    pub sx: f64,
    pub sy: f64,
    pub epsg: Option<u32>,
    nodata: Option<f32>,
    chunk: (u32, u32),
    across: u32,
    /// Decoded chunks: the values and the row length they are stored with.
    cache: lru::LruCache<u32, (Vec<f32>, u32)>,
    /// Files with several samples a pixel (the tiff crate reads one only): read here.
    raw: Option<Raw>,
    /// Which sample (band) is the depth, from 0.
    pub band: usize,
    /// The bands' names, where GDAL wrote them.
    pub band_names: Vec<String>,
}

/// What reading a multi-band file's chunks ourselves takes.
struct Raw {
    file: File,
    big_endian: bool,
    offsets: Vec<u64>,
    counts: Vec<u64>,
    spp: usize,
    bytes: usize,
    float: bool,
    compression: u16,
    predictor: u16,
    planar: u16,
    tiled: bool,
}

impl GeoTiff {
    pub fn open(path: &Path) -> Result<GeoTiff, String> {
        let f = File::open(path).map_err(|e| e.to_string())?;
        let mut dec = Decoder::new(BufReader::new(f)).map_err(|e| e.to_string())?.with_limits(Limits::unlimited());
        let (width, height) = dec.dimensions().map_err(|e| e.to_string())?;
        // GeoTIFF georeferencing: pixel scale + tie point, or a transformation matrix
        let scale = dec.get_tag_f64_vec(Tag::from_u16_exhaustive(33550)).ok();
        let tie = dec.get_tag_f64_vec(Tag::from_u16_exhaustive(33922)).ok();
        let matrix = dec.get_tag_f64_vec(Tag::from_u16_exhaustive(34264)).ok();
        let (mut x0, mut y0, sx, sy) = match (scale, tie, matrix) {
            (Some(s), Some(t), _) if s.len() >= 2 && t.len() >= 6 => (t[3] - t[0] * s[0], t[4] + t[1] * s[1], s[0], s[1]),
            (_, _, Some(m)) if m.len() >= 16 && m[1] == 0.0 && m[4] == 0.0 => (m[3], m[7], m[0], -m[5]),
            _ => return Err("no georeferencing (not a GeoTIFF?)".into()),
        };
        // GeoKeys: the projection (3072 ProjectedCSTypeGeoKey, 2048 geographic) and
        // whether pixels are points (1025 GTRasterTypeGeoKey = 2)
        let mut epsg = None;
        if let Ok(k) = dec.get_tag_u16_vec(Tag::from_u16_exhaustive(34735)) {
            for e in k.chunks(4).skip(1) {
                if e.len() == 4 && e[1] == 0 {
                    match e[0] {
                        3072 | 2048 if e[3] != 32767 => epsg = Some(e[3] as u32),
                        1025 if e[3] == 2 => {
                            x0 -= sx / 2.0;
                            y0 += sy / 2.0;
                        }
                        _ => {}
                    }
                }
            }
        }
        let nodata = dec.get_tag_ascii_string(Tag::from_u16_exhaustive(42113)).ok().and_then(|s| s.trim().trim_end_matches('\0').parse().ok());
        let chunk = dec.chunk_dimensions();
        let across = width.div_ceil(chunk.0.max(1));
        // several samples a pixel (LINZ's Taranaki model has three 32-bit floats): GDAL's band
        // names say which is the depth; read those files' chunks here
        let spp = dec.find_tag_unsigned::<u16>(Tag::SamplesPerPixel).ok().flatten().unwrap_or(1) as usize;
        let band_names = gdal_band_names(&dec.get_tag_ascii_string(Tag::from_u16_exhaustive(42112)).unwrap_or_default(), spp);
        let band = pick_band(&band_names);
        let raw = if spp > 1 { Some(Raw::new(path, &mut dec, spp)?) } else { None };
        Ok(GeoTiff {
            dec,
            width,
            height,
            x0,
            y0,
            sx,
            sy,
            epsg,
            nodata,
            chunk,
            across,
            cache: lru::LruCache::new(NonZeroUsize::new(96).unwrap()),
            raw,
            band,
            band_names,
        })
    }

    /// Map bounds [min x, min y, max x, max y].
    pub fn bounds(&self) -> [f64; 4] {
        [self.x0, self.y0 - self.sy * self.height as f64, self.x0 + self.sx * self.width as f64, self.y0]
    }

    fn chunk(&mut self, idx: u32) -> Option<&(Vec<f32>, u32)> {
        if !self.cache.contains(&idx) {
            let (cw, ch) = self.chunk;
            let v = if let Some(r) = &mut self.raw {
                // tiles keep their padding; a strip is as wide as the image
                let rows = if r.tiled { ch } else { ch.min(self.height.saturating_sub((idx % self.height.div_ceil(ch)) * ch)) };
                match r.read(idx, cw, rows, self.band) {
                    Ok(v) => (v, cw),
                    Err(e) => {
                        eprintln!("  chunk {idx}: {e}");
                        (Vec::new(), cw)
                    }
                }
            } else {
                let w = self.dec.chunk_data_dimensions(idx).0;
                let v = match self.dec.read_chunk(idx) {
                    Ok(DecodingResult::F32(v)) => v,
                    Ok(DecodingResult::F64(v)) => v.into_iter().map(|x| x as f32).collect(),
                    Ok(DecodingResult::I16(v)) => v.into_iter().map(f32::from).collect(),
                    Ok(DecodingResult::I32(v)) => v.into_iter().map(|x| x as f32).collect(),
                    Ok(DecodingResult::U16(v)) => v.into_iter().map(f32::from).collect(),
                    Ok(_) => return None,
                    Err(e) => {
                        eprintln!("  chunk {idx}: {e}");
                        Vec::new()
                    }
                };
                (v, w)
            };
            self.cache.put(idx, v);
        }
        self.cache.get(&idx)
    }

    /// The value at map position (x, y) (the pixel it falls in), None outside or no data.
    pub fn value_at(&mut self, x: f64, y: f64) -> Option<f32> {
        let col = ((x - self.x0) / self.sx).floor();
        let row = ((self.y0 - y) / self.sy).floor();
        if col < 0.0 || row < 0.0 || col >= self.width as f64 || row >= self.height as f64 {
            return None;
        }
        let (col, row) = (col as u32, row as u32);
        let (cw, ch) = self.chunk;
        let idx = (row / ch) * self.across + col / cw;
        let nodata = self.nodata;
        // the row length the chunk is stored with (the tiff crate crops edge tiles' padding)
        let (c, data_w) = self.chunk(idx)?;
        let v = *c.get(((row % ch) * data_w + col % cw) as usize)?;
        let bad = !v.is_finite() || nodata.is_some_and(|n| (v - n).abs() <= n.abs() * 1e-6 + 1e-6) || !(-12_000.0..=9_000.0).contains(&v);
        (!bad).then_some(v)
    }
}

impl Raw {
    fn new(path: &Path, dec: &mut Decoder<BufReader<File>>, spp: usize) -> Result<Raw, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut magic = [0u8; 2];
        file.read_exact(&mut magic).map_err(|e| e.to_string())?;
        let tiled = dec.find_tag(Tag::TileOffsets).ok().flatten().is_some();
        let (ot, ct) = if tiled { (Tag::TileOffsets, Tag::TileByteCounts) } else { (Tag::StripOffsets, Tag::StripByteCounts) };
        let offsets = dec.get_tag_u64_vec(ot).map_err(|e| e.to_string())?;
        let counts = dec.get_tag_u64_vec(ct).map_err(|e| e.to_string())?;
        let bits = dec.get_tag_u32_vec(Tag::BitsPerSample).map_err(|e| e.to_string())?;
        if bits.iter().any(|b| *b != bits[0]) || !matches!(bits[0], 16 | 32 | 64) {
            return Err(format!("{bits:?} bits per sample isn't supported"));
        }
        // one value per sample: the first says it
        let format = dec.get_tag_u16_vec(Tag::SampleFormat).ok().and_then(|v| v.first().copied()).unwrap_or(1);
        let mut u = |t: Tag, d: u16| dec.find_tag_unsigned::<u16>(t).ok().flatten().unwrap_or(d);
        Ok(Raw {
            file,
            big_endian: &magic == b"MM",
            offsets,
            counts,
            spp,
            bytes: bits[0] as usize / 8,
            float: format == 3,
            compression: u(Tag::Compression, 1),
            predictor: u(Tag::Predictor, 1),
            planar: u(Tag::PlanarConfiguration, 1),
            tiled,
        })
    }

    /// Band `band` of chunk `idx` (w × rows values; planar files keep each band's chunks after
    /// the previous band's).
    fn read(&mut self, idx: u32, w: u32, rows: u32, band: usize) -> Result<Vec<f32>, String> {
        let planar = self.planar == 2;
        let per_plane = if planar { self.offsets.len() / self.spp } else { self.offsets.len() };
        let i = if planar { band * per_plane + idx as usize } else { idx as usize };
        let (off, n) = (*self.offsets.get(i).ok_or("chunk out of range")?, *self.counts.get(i).ok_or("chunk out of range")?);
        let mut buf = vec![0u8; n as usize];
        self.file.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
        self.file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        let spp = if planar { 1 } else { self.spp };
        let row_bytes = w as usize * spp * self.bytes;
        let want = row_bytes * rows as usize;
        let mut data = match self.compression {
            1 => buf,
            5 => weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8).decode(&buf).map_err(|e| format!("LZW: {e}"))?,
            8 | 32946 => {
                let mut out = Vec::with_capacity(want);
                flate2::read::ZlibDecoder::new(&buf[..]).read_to_end(&mut out).map_err(|e| format!("deflate: {e}"))?;
                out
            }
            c => return Err(format!("compression {c} isn't supported for multi-band files")),
        };
        if data.len() < want {
            return Err(format!("chunk {idx} is short ({} of {want} bytes)", data.len()));
        }
        data.truncate(want);
        let b = self.bytes;
        let mut out = Vec::with_capacity((w * rows) as usize);
        for row in data.chunks_mut(row_bytes) {
            match self.predictor {
                1 => {}
                // floating point: bytes differenced along the row (stride = samples), then
                // stored as planes of each value's bytes, most significant first
                3 => {
                    for k in spp..row.len() {
                        row[k] = row[k].wrapping_add(row[k - spp]);
                    }
                    let nv = row.len() / b;
                    let planes = row.to_vec();
                    for v in 0..nv {
                        for j in 0..b {
                            // back into the file's byte order
                            let src = planes[j * nv + v];
                            let dst = if self.big_endian { j } else { b - 1 - j };
                            row[v * b + dst] = src;
                        }
                    }
                }
                // integer differencing of whole samples
                2 if !self.float => {
                    let nv = row.len() / b;
                    for v in spp..nv {
                        let get = |r: &[u8], k: usize| -> u64 {
                            let mut x = 0u64;
                            for j in 0..b {
                                let byte = if self.big_endian { r[k * b + j] } else { r[k * b + b - 1 - j] };
                                x = (x << 8) | byte as u64;
                            }
                            x
                        };
                        let s = get(row, v).wrapping_add(get(row, v - spp));
                        for j in 0..b {
                            let byte = (s >> (8 * (b - 1 - j))) as u8;
                            if self.big_endian {
                                row[v * b + j] = byte;
                            } else {
                                row[v * b + b - 1 - j] = byte;
                            }
                        }
                    }
                }
                p => return Err(format!("predictor {p} isn't supported here")),
            }
            let band_in_row = if planar { 0 } else { band };
            for x in 0..w as usize {
                let k = (x * spp + band_in_row) * b;
                let s = &row[k..k + b];
                let v = match (self.float, b) {
                    (true, 4) => f32::from_bits(if self.big_endian {
                        u32::from_be_bytes([s[0], s[1], s[2], s[3]])
                    } else {
                        u32::from_le_bytes([s[0], s[1], s[2], s[3]])
                    }),
                    (true, 8) => {
                        let a = [s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]];
                        (if self.big_endian { f64::from_be_bytes(a) } else { f64::from_le_bytes(a) }) as f32
                    }
                    (false, 2) => (if self.big_endian { i16::from_be_bytes([s[0], s[1]]) } else { i16::from_le_bytes([s[0], s[1]]) }) as f32,
                    (false, 4) => {
                        (if self.big_endian { i32::from_be_bytes([s[0], s[1], s[2], s[3]]) } else { i32::from_le_bytes([s[0], s[1], s[2], s[3]]) }) as f32
                    }
                    _ => f32::NAN,
                };
                out.push(v);
            }
        }
        Ok(out)
    }
}

/// The band names in GDAL's metadata (`<Item name="DESCRIPTION" sample="0" role="description">`),
/// one per band ("" where none).
pub fn gdal_band_names(xml: &str, bands: usize) -> Vec<String> {
    let mut names = vec![String::new(); bands];
    for item in xml.split("<Item").skip(1) {
        let head = item.split('>').next().unwrap_or("");
        if !head.contains("role=\"description\"") && !head.contains("name=\"DESCRIPTION\"") {
            continue;
        }
        let sample = head.split("sample=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|s| s.parse::<usize>().ok());
        let text = item.split('>').nth(1).and_then(|s| s.split('<').next()).unwrap_or("").trim().to_string();
        if let Some(i) = sample.filter(|i| *i < bands) {
            names[i] = text;
        }
    }
    names
}

/// The depth band: one named for depth or elevation, else the first.
pub fn pick_band(names: &[String]) -> usize {
    let has = |n: &String, w: &[&str]| w.iter().any(|w| n.to_ascii_lowercase().contains(w));
    names
        .iter()
        .position(|n| has(n, &["depth", "elevation", "height", "z_value", "bathy"]) && !has(n, &["uncert", "std", "density", "count", "tvu"]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdal_band_names_and_the_depth_band() {
        let xml = r#"<GDALMetadata><Item name="DESCRIPTION" sample="0" role="description">Uncertainty</Item><Item name="DESCRIPTION" sample="1" role="description">Depth</Item><Item name="STATISTICS_MAXIMUM" sample="1">2.4</Item></GDALMetadata>"#;
        let n = gdal_band_names(xml, 3);
        assert_eq!(n, vec!["Uncertainty".to_string(), "Depth".into(), "".into()]);
        assert_eq!(pick_band(&n), 1);
        assert_eq!(pick_band(&gdal_band_names("", 3)), 0);
    }
}
