//! Drawing an image as the map's web-mercator tiles (256 px PNG): the right overview for
//! the zoom, the image's projection followed through a 16-pixel grid, bilinear sampling.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::Arc;

use crate::geo::{distance_m, web_mercator_inv, Affine, Crs, R};

/// A block of pixels: RGBA (4 bytes a pixel), or palette indices (1 byte) with the palette.
pub struct Block {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
    pub palette: Option<Arc<Vec<[u8; 4]>>>,
}

impl Block {
    fn px(&self, x: usize, y: usize) -> [u8; 4] {
        if x >= self.w || y >= self.h {
            return [0; 4];
        }
        let i = y * self.w + x;
        match &self.palette {
            Some(p) => p.get(self.data[i] as usize).copied().unwrap_or([0; 4]),
            None => {
                let s = &self.data[i * 4..i * 4 + 4];
                [s[0], s[1], s[2], s[3]]
            }
        }
    }
}

/// An image that can be drawn on the chart.
pub trait Raster: Send + Sync {
    /// Full-resolution size (width, height) in pixels.
    fn size(&self) -> (usize, usize);
    /// Resolutions: 0 is the full image, then reduced copies.
    fn levels(&self) -> usize;
    fn level_size(&self, level: usize) -> (usize, usize);
    fn block_size(&self, level: usize) -> (usize, usize);
    fn block(&self, level: usize, bx: usize, by: usize) -> Option<Arc<Block>>;
    /// Full-resolution pixel coordinates (column, row; 0,0 the top-left corner) of a WGS84 point.
    fn to_pixel(&self, lon: f64, lat: f64) -> Option<(f64, f64)>;
    /// The WGS84 position of a full-resolution pixel position, when the image has a georeference.
    fn to_lonlat(&self, _col: f64, _row: f64) -> Option<(f64, f64)> {
        None
    }
    /// Whether a full-resolution pixel position is inside the image's charted area (a KAP's border).
    fn inside(&self, _col: f64, _row: f64) -> bool {
        true
    }
    /// [west, south, east, north] in degrees.
    fn bounds(&self) -> [f64; 4];
    /// Ground metres per full-resolution pixel.
    fn resolution_m(&self) -> f64;
}

/// An image's [west, south, east, north], from its edges.
pub fn bounds_of(crs: &Crs, aff: &Affine, w: usize, h: usize) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let n = 24;
    for i in 0..=n {
        let t = i as f64 / n as f64;
        for (c, r) in [(t * w as f64, 0.0), (t * w as f64, h as f64), (0.0, t * h as f64), (w as f64, t * h as f64)] {
            let (x, y) = aff.apply(c, r);
            let (lon, lat) = crs.inverse(x, y);
            if lon.is_finite() && lat.is_finite() {
                b = [b[0].min(lon), b[1].min(lat), b[2].max(lon), b[3].max(lat)];
            }
        }
    }
    b
}

/// Ground metres per pixel at the image's centre.
pub fn resolution_of(crs: &Crs, aff: &Affine, w: usize, h: usize) -> f64 {
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let ll = |c: f64, r: f64| {
        let (x, y) = aff.apply(c, r);
        crs.inverse(x, y)
    };
    let p = ll(cx, cy);
    let dx = distance_m(p, ll(cx + 1.0, cy));
    let dy = distance_m(p, ll(cx, cy + 1.0));
    (dx * dy).sqrt()
}

/// The zoom at which one tile pixel is about one image pixel (at the image's latitude):
/// the map can zoom further, but there is nothing more to see.
pub fn native_zoom(r: &dyn Raster) -> u8 {
    let b = r.bounds();
    let lat = ((b[1] + b[3]) / 2.0).to_radians();
    let res = r.resolution_m().max(0.01);
    let z = (2.0 * PI * R * lat.cos() / (256.0 * res)).log2();
    z.ceil().clamp(0.0, 22.0) as u8
}

const GRID: usize = 16;
const T: usize = 256;

/// A web-mercator tile of the image as PNG, or None where the image has nothing there.
pub fn render_tile(r: &dyn Raster, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
    let rgba = render_rgba(r, z, x, y)?;
    Some(encode_png(&rgba, T as u32, T as u32))
}

/// The tile's pixels (RGBA, 256 × 256), or None where the image has nothing there.
pub fn render_rgba(r: &dyn Raster, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
    let n = (1u64 << z) as f64;
    let size = 2.0 * PI * R / n;
    let (x0, y0) = (-PI * R + x as f64 * size, PI * R - y as f64 * size);
    let (w, s) = web_mercator_inv(x0, y0 - size);
    let (e, nn) = web_mercator_inv(x0 + size, y0);
    let b = r.bounds();
    if e < b[0] || w > b[2] || nn < b[1] || s > b[3] {
        return None;
    }
    // the overview whose pixels are no smaller than the tile's
    let lat_c = (s + nn) / 2.0;
    let tile_m = size / T as f64 * lat_c.to_radians().cos();
    let (w0, h0) = r.size();
    let res = r.resolution_m().max(1e-6);
    let mut level = 0;
    for l in 1..r.levels() {
        let (lw, _) = r.level_size(l);
        if res * (w0 as f64 / lw as f64) <= tile_m * 1.1 {
            level = l;
        }
    }
    let (lw, lh) = r.level_size(level);
    let (sx, sy) = (w0 as f64 / lw as f64, h0 as f64 / lh as f64);
    let (bw, bh) = r.block_size(level);

    // full-resolution pixel positions on a grid across the tile
    let g = T / GRID + 1;
    let step = size / T as f64;
    let mut grid = vec![None; g * g];
    for j in 0..g {
        for i in 0..g {
            let (lon, lat) = web_mercator_inv(x0 + (i * GRID) as f64 * step, y0 - (j * GRID) as f64 * step);
            grid[j * g + i] = r.to_pixel(lon, lat);
        }
    }
    let mut blocks: HashMap<(usize, usize), Option<Arc<Block>>> = HashMap::new();
    let mut fetch = |px: i64, py: i64| -> [u8; 4] {
        if px < 0 || py < 0 || px as usize >= lw || py as usize >= lh {
            return [0; 4];
        }
        let (px, py) = (px as usize, py as usize);
        let key = (px / bw, py / bh);
        let blk = blocks.entry(key).or_insert_with(|| r.block(level, key.0, key.1));
        match blk {
            Some(b) => b.px(px - key.0 * bw, py - key.1 * bh),
            None => [0; 4],
        }
    };
    let mut out = vec![0u8; T * T * 4];
    let mut any = false;
    for oy in 0..T {
        let (gj, fy) = (oy / GRID, (oy % GRID) as f64 + 0.5);
        for ox in 0..T {
            let (gi, fx) = (ox / GRID, (ox % GRID) as f64 + 0.5);
            let corners = [grid[gj * g + gi], grid[gj * g + gi + 1], grid[(gj + 1) * g + gi], grid[(gj + 1) * g + gi + 1]];
            let (u, v) = (fx / GRID as f64, fy / GRID as f64);
            let p = if let [Some(a), Some(b), Some(c), Some(d)] = corners {
                let top = (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
                let bot = (c.0 + (d.0 - c.0) * u, c.1 + (d.1 - c.1) * u);
                Some((top.0 + (bot.0 - top.0) * v, top.1 + (bot.1 - top.1) * v))
            } else {
                let (lon, lat) = web_mercator_inv(x0 + (ox as f64 + 0.5) * step, y0 - (oy as f64 + 0.5) * step);
                r.to_pixel(lon, lat)
            };
            let Some((fc, fr)) = p else { continue };
            if fc < 0.0 || fr < 0.0 || fc > w0 as f64 || fr > h0 as f64 || !r.inside(fc, fr) {
                continue;
            }
            // bilinear between the level's pixel centres
            let (lc, lr) = (fc / sx - 0.5, fr / sy - 0.5);
            let (ix, iy) = (lc.floor() as i64, lr.floor() as i64);
            let (tx, ty) = (lc - ix as f64, lr - iy as f64);
            let q = [fetch(ix, iy), fetch(ix + 1, iy), fetch(ix, iy + 1), fetch(ix + 1, iy + 1)];
            let wts = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
            let mut acc = [0.0f64; 4];
            for (c, wt) in q.iter().zip(wts) {
                let a = c[3] as f64 * wt;
                acc[0] += c[0] as f64 * a;
                acc[1] += c[1] as f64 * a;
                acc[2] += c[2] as f64 * a;
                acc[3] += a;
            }
            if acc[3] < 0.5 {
                continue;
            }
            let o = (oy * T + ox) * 4;
            out[o] = (acc[0] / acc[3]).round() as u8;
            out[o + 1] = (acc[1] / acc[3]).round() as u8;
            out[o + 2] = (acc[2] / acc[3]).round() as u8;
            out[o + 3] = acc[3].round().min(255.0) as u8;
            any = true;
        }
    }
    any.then_some(out)
}

pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(rgba).expect("png data");
    }
    out
}

/// Web-mercator tile numbers covering [west, south, east, north] at zoom z.
pub fn tile_range(b: [f64; 4], z: u8) -> (u32, u32, u32, u32) {
    let n = 1u32 << z;
    let tx = |lon: f64| (((lon + 180.0) / 360.0 * n as f64).floor() as i64).clamp(0, n as i64 - 1) as u32;
    let ty = |lat: f64| {
        let p = lat.clamp(-85.05, 85.05).to_radians();
        (((1.0 - (p.tan() + 1.0 / p.cos()).ln() / PI) / 2.0 * n as f64).floor() as i64).clamp(0, n as i64 - 1) as u32
    };
    (tx(b[0]), ty(b[3]), tx(b[2]), ty(b[1]))
}

/// The zoom at which a picture shown flat (its pixels as the map's, for calibration) is at
/// full resolution: the picture fills the top-left of a square world 256·2^zoom pixels across.
pub fn pixel_zoom(r: &dyn Raster) -> u8 {
    let (w, h) = r.size();
    let m = w.max(h).max(1) as f64 / T as f64;
    m.log2().ceil().clamp(0.0, 22.0) as u8
}

/// A 256 px tile of the picture itself (not on the chart): at the flat world's zoom `z` each
/// tile pixel covers 2^(pixel_zoom − z) image pixels. For the calibration's image pane.
pub fn render_pixel_tile(r: &dyn Raster, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
    let top = pixel_zoom(r);
    let (w0, h0) = r.size();
    let s = 2f64.powi(top as i32 - z as i32); // image pixels a tile pixel
    let (x0, y0) = (x as f64 * T as f64 * s, y as f64 * T as f64 * s);
    if x0 >= w0 as f64 || y0 >= h0 as f64 {
        return None;
    }
    let mut level = 0;
    for l in 1..r.levels() {
        let (lw, _) = r.level_size(l);
        if w0 as f64 / lw as f64 <= s * 1.01 {
            level = l;
        }
    }
    let (lw, lh) = r.level_size(level);
    let (sx, sy) = (w0 as f64 / lw as f64, h0 as f64 / lh as f64);
    let (bw, bh) = r.block_size(level);
    let mut blocks: HashMap<(usize, usize), Option<Arc<Block>>> = HashMap::new();
    let mut fetch = |px: i64, py: i64| -> [u8; 4] {
        if px < 0 || py < 0 || px as usize >= lw || py as usize >= lh {
            return [0; 4];
        }
        let (px, py) = (px as usize, py as usize);
        let key = (px / bw, py / bh);
        let blk = blocks.entry(key).or_insert_with(|| r.block(level, key.0, key.1));
        match blk {
            Some(b) => b.px(px - key.0 * bw, py - key.1 * bh),
            None => [0; 4],
        }
    };
    let mut out = vec![0u8; T * T * 4];
    for oy in 0..T {
        let fr = y0 + (oy as f64 + 0.5) * s;
        if fr > h0 as f64 {
            break;
        }
        for ox in 0..T {
            let fc = x0 + (ox as f64 + 0.5) * s;
            if fc > w0 as f64 {
                break;
            }
            let (lc, lr) = (fc / sx - 0.5, fr / sy - 0.5);
            let (ix, iy) = (lc.floor() as i64, lr.floor() as i64);
            let (tx, ty) = (lc - ix as f64, lr - iy as f64);
            // enlarged far past full resolution: sharp pixels, so a point can be put on one
            let c = if s < 0.25 {
                fetch(lc.round() as i64, lr.round() as i64)
            } else {
                let q = [fetch(ix, iy), fetch(ix + 1, iy), fetch(ix, iy + 1), fetch(ix + 1, iy + 1)];
                let wts = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
                let mut acc = [0.0f64; 4];
                for (c, wt) in q.iter().zip(wts) {
                    for k in 0..4 {
                        acc[k] += c[k] as f64 * wt;
                    }
                }
                [acc[0].round() as u8, acc[1].round() as u8, acc[2].round() as u8, acc[3].round() as u8]
            };
            let o = (oy * T + ox) * 4;
            // a picture without transparency shows its own pixels; transparent parts stay clear
            out[o..o + 4].copy_from_slice(&c);
        }
    }
    Some(encode_png(&out, T as u32, T as u32))
}
