//! Writing RGBA images as tiled, deflate-compressed GeoTIFFs with overviews: the format
//! downloaded satellite images are kept in, readable by TweedPlot, QGIS, GDAL and OpenCPN's
//! GeoTIFF tools.

use std::io::Write;
use std::path::Path;

use crate::geo::{Affine, Crs};
use crate::{err, Result};

const TILE: usize = 256;

/// Where an image is: its EPSG code and a north-up affine (no rotation).
#[derive(Clone, Copy, Debug)]
pub struct GeoRef {
    pub epsg: u32,
    pub aff: Affine,
}

/// Write `rgba` (w × h × 4) as a GeoTIFF at `path` (via a temporary file, so a half-written
/// file is never left under the real name).
pub fn write_geotiff(path: &Path, w: usize, h: usize, rgba: &[u8], geo: &GeoRef, software: &str) -> Result<()> {
    if rgba.len() != w * h * 4 || w == 0 || h == 0 {
        return err("image size does not match its pixels");
    }
    if geo.aff.c != 0.0 || geo.aff.e != 0.0 {
        return err("only north-up images can be written");
    }
    if Crs::from_epsg(geo.epsg).is_none() {
        return err(format!("EPSG:{} is not supported", geo.epsg));
    }
    let bytes = encode(w, h, rgba, geo, software)?;
    let tmp = path.with_extension("tif.part");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The GeoTIFF's bytes.
pub fn encode(w: usize, h: usize, rgba: &[u8], geo: &GeoRef, software: &str) -> Result<Vec<u8>> {
    encode_with(w, h, rgba, Some(geo), software)
}

/// A plain tiled TIFF with overviews and no georeferencing (an image waiting to be calibrated,
/// or placed by its world file): fast to draw at any zoom, however large the original.
pub fn write_tiff(path: &Path, w: usize, h: usize, rgba: &[u8], software: &str) -> Result<()> {
    if rgba.len() != w * h * 4 || w == 0 || h == 0 {
        return err("image size does not match its pixels");
    }
    let bytes = encode_with(w, h, rgba, None, software)?;
    let tmp = path.with_extension("tif.part");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn encode_with(w: usize, h: usize, rgba: &[u8], geo: Option<&GeoRef>, software: &str) -> Result<Vec<u8>> {
    let mut levels: Vec<(usize, usize, Vec<u8>)> = vec![(w, h, rgba.to_vec())];
    while {
        let (lw, lh, _) = levels.last().expect("a level");
        lw.max(lh) > &TILE
    } {
        let (lw, lh, px) = levels.last().expect("a level");
        let next = half(px, *lw, *lh);
        levels.push(next);
    }
    let mut buf: Vec<u8> = b"II*\0\0\0\0\0".to_vec();
    let mut next_ptr = 4usize;
    for (li, (lw, lh, px)) in levels.iter().enumerate() {
        let (nx, ny) = (lw.div_ceil(TILE), lh.div_ceil(TILE));
        let mut offsets = Vec::with_capacity(nx * ny);
        let mut counts = Vec::with_capacity(nx * ny);
        let mut tile = vec![0u8; TILE * TILE * 4];
        for ty in 0..ny {
            for tx in 0..nx {
                tile.fill(0);
                for row in 0..TILE {
                    let y = ty * TILE + row;
                    if y >= *lh {
                        break;
                    }
                    let x0 = tx * TILE;
                    let n = (TILE).min(lw - x0);
                    let s = (y * lw + x0) * 4;
                    tile[row * TILE * 4..row * TILE * 4 + n * 4].copy_from_slice(&px[s..s + n * 4]);
                }
                // horizontal predictor, then deflate
                for row in tile.chunks_mut(TILE * 4) {
                    for i in (4..row.len()).rev() {
                        row[i] = row[i].wrapping_sub(row[i - 4]);
                    }
                }
                let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
                z.write_all(&tile)?;
                let data = z.finish()?;
                offsets.push(buf.len() as u32);
                counts.push(data.len() as u32);
                buf.extend_from_slice(&data);
            }
        }
        let mut e = Entries::default();
        e.long(254, &[if li == 0 { 0 } else { 1 }]);
        e.long(256, &[*lw as u32]);
        e.long(257, &[*lh as u32]);
        e.short(258, &[8, 8, 8, 8]);
        e.short(259, &[8]);
        e.short(262, &[2]);
        e.short(277, &[4]);
        e.short(284, &[1]);
        if li == 0 {
            let mut s = software.as_bytes().to_vec();
            s.push(0);
            e.raw(305, 2, s.len() as u32, s);
        }
        e.short(317, &[2]);
        e.short(322, &[TILE as u16]);
        e.short(323, &[TILE as u16]);
        e.long(324, &offsets);
        e.long(325, &counts);
        e.short(338, &[2]);
        if let (0, Some(geo)) = (li, geo) {
            let a = geo.aff;
            e.double(33550, &[a.b, -a.f, 0.0]);
            e.double(33922, &[0.0, 0.0, 0.0, a.a, a.d, 0.0]);
            let geographic = matches!(Crs::from_epsg(geo.epsg), Some(Crs::Geographic));
            let (model, key) = if geographic { (2, 2048) } else { (1, 3072) };
            e.short(34735, &[1, 1, 0, 3, 1024, 0, 1, model, 1025, 0, 1, 1, key, 0, 1, geo.epsg as u16]);
        }
        let ifd = e.write(&mut buf);
        buf[next_ptr..next_ptr + 4].copy_from_slice(&(ifd as u32).to_le_bytes());
        next_ptr = buf.len() - 4;
    }
    if buf.len() > u32::MAX as usize {
        return err("image too large for a classic TIFF");
    }
    Ok(buf)
}

/// Half size, averaging 2 × 2 pixels (weighted by their alpha).
fn half(px: &[u8], w: usize, h: usize) -> (usize, usize, Vec<u8>) {
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 4];
            let mut n = 0;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = (x * 2 + dx, y * 2 + dy);
                if sx < w && sy < h {
                    let s = &px[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
                    let a = s[3] as u32;
                    acc[0] += s[0] as u32 * a;
                    acc[1] += s[1] as u32 * a;
                    acc[2] += s[2] as u32 * a;
                    acc[3] += a;
                    n += 1;
                }
            }
            let o = (y * nw + x) * 4;
            if let (Some(r), Some(g), Some(b)) = (acc[0].checked_div(acc[3]), acc[1].checked_div(acc[3]), acc[2].checked_div(acc[3])) {
                out[o] = r as u8;
                out[o + 1] = g as u8;
                out[o + 2] = b as u8;
                out[o + 3] = (acc[3] / n) as u8;
            }
        }
    }
    (nw, nh, out)
}

#[derive(Default)]
struct Entries(Vec<(u16, u16, u32, Vec<u8>)>);

impl Entries {
    fn raw(&mut self, tag: u16, typ: u16, count: u32, data: Vec<u8>) {
        self.0.push((tag, typ, count, data));
    }
    fn short(&mut self, tag: u16, v: &[u16]) {
        self.raw(tag, 3, v.len() as u32, v.iter().flat_map(|x| x.to_le_bytes()).collect());
    }
    fn long(&mut self, tag: u16, v: &[u32]) {
        self.raw(tag, 4, v.len() as u32, v.iter().flat_map(|x| x.to_le_bytes()).collect());
    }
    fn double(&mut self, tag: u16, v: &[f64]) {
        self.raw(tag, 12, v.len() as u32, v.iter().flat_map(|x| x.to_le_bytes()).collect());
    }
    /// Out-of-line values first, then the directory; returns the directory's offset. The
    /// directory ends with a zero "next" pointer.
    fn write(mut self, buf: &mut Vec<u8>) -> usize {
        self.0.sort_by_key(|e| e.0);
        let mut placed = Vec::new();
        for (tag, typ, count, data) in &self.0 {
            if data.len() > 4 {
                if buf.len() % 2 == 1 {
                    buf.push(0);
                }
                placed.push((*tag, *typ, *count, (buf.len() as u32).to_le_bytes().to_vec()));
                buf.extend_from_slice(data);
            } else {
                let mut v = data.clone();
                v.resize(4, 0);
                placed.push((*tag, *typ, *count, v));
            }
        }
        if buf.len() % 2 == 1 {
            buf.push(0);
        }
        let at = buf.len();
        buf.extend_from_slice(&(placed.len() as u16).to_le_bytes());
        for (tag, typ, count, v) in placed {
            buf.extend_from_slice(&tag.to_le_bytes());
            buf.extend_from_slice(&typ.to_le_bytes());
            buf.extend_from_slice(&count.to_le_bytes());
            buf.extend_from_slice(&v);
        }
        buf.extend_from_slice(&[0, 0, 0, 0]);
        at
    }
}
