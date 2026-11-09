//! A float GeoTIFF (LINZ's depth model tiles): its georeferencing and its values, read a
//! chunk (strip or tile) at a time with a small cache, so big files never sit in memory.

use std::fs::File;
use std::io::BufReader;
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
    cache: lru::LruCache<u32, Vec<f32>>,
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
        Ok(GeoTiff { dec, width, height, x0, y0, sx, sy, epsg, nodata, chunk, across, cache: lru::LruCache::new(NonZeroUsize::new(96).unwrap()) })
    }

    /// Map bounds [min x, min y, max x, max y].
    pub fn bounds(&self) -> [f64; 4] {
        [self.x0, self.y0 - self.sy * self.height as f64, self.x0 + self.sx * self.width as f64, self.y0]
    }

    fn chunk(&mut self, idx: u32) -> Option<&Vec<f32>> {
        if !self.cache.contains(&idx) {
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
        // the chunk's data width (the decoder crops the padding of edge tiles)
        let data_w = self.dec.chunk_data_dimensions(idx).0;
        let nodata = self.nodata;
        let c = self.chunk(idx)?;
        let v = *c.get(((row % ch) * data_w + col % cw) as usize)?;
        let bad = !v.is_finite() || nodata.is_some_and(|n| (v - n).abs() <= n.abs() * 1e-6 + 1e-6) || !(-12_000.0..=9_000.0).contains(&v);
        (!bad).then_some(v)
    }
}
