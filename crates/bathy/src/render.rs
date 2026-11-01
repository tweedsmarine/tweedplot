//! Drawing own soundings as map tiles (`z/x/y`, 256-pixel PNG, Web Mercator): depth colours
//! (a rainbow, or chart blues) over a depth range chosen to suit the area, lit from the
//! north-west, with contours. Gaps between soundings up to [`Style::fill_m`] are filled from
//! the nearest soundings around them; nothing further is made up. Above 0 m is left clear.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{cells_across, to_world, Store, MAX_LEVEL, MIN_LEVEL, TILE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Palette {
    /// Red (shallow) through yellow, green, cyan and blue to violet (deep).
    Rainbow,
    /// Pale (shallow) to dark blue (deep), as charts.
    Blues,
    /// Night: one grey ramp, a smooth fade. The night colours invert the page, so dark (shallow)
    /// shows brightest and deep fades to black.
    Mono,
    /// Night: as Mono in six bands, which stay apart at low brightness.
    Bands,
}

/// How the bathymetry is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub palette: Palette,
    /// The colour range (m, positive down): shallower than `shallow` takes the first colour,
    /// deeper than `deep` the last.
    pub shallow: f32,
    pub deep: f32,
    pub contours: bool,
    pub shade: bool,
    /// Fill gaps between soundings.
    pub fill: bool,
    /// How far from a sounding gaps are filled (m).
    pub fill_m: f32,
    /// Mark the soundings the checks left out (magenta).
    pub rejected: bool,
    /// A depth of interest (m) between `shallow` and `deep`: the colours run from `shallow` to
    /// it over the first half of the palette and from it to `deep` over the second, so ground
    /// near it stands out, and it gets a bold contour (the three pointers, as the sea layers).
    pub target: Option<f32>,
    /// Run the palette the other way (deep to shallow).
    pub reverse: bool,
}

/// The largest fill (cells of the level drawn): beyond it the filled ring would cost more
/// to read than the tile itself.
pub const MAX_FILL_CELLS: i64 = 192;

impl Default for Style {
    fn default() -> Self {
        Style { palette: Palette::Rainbow, shallow: 0.0, deep: 500.0, contours: true, shade: true, fill: true, fill_m: 4.0, rejected: false, target: None, reverse: false }
    }
}

/// Contour interval (m) at a zoom.
fn contour_step(z: u8) -> f32 {
    match z {
        17.. => 1.0,
        16 => 2.0,
        15 => 5.0,
        13 | 14 => 10.0,
        11 | 12 => 20.0,
        9 | 10 => 50.0,
        _ => 100.0,
    }
}

const RAINBOW: &[(f32, [f32; 3])] = &[
    (0.0, [196.0, 32.0, 38.0]),
    (0.14, [236.0, 112.0, 36.0]),
    (0.28, [244.0, 206.0, 52.0]),
    (0.42, [120.0, 200.0, 80.0]),
    (0.56, [40.0, 186.0, 170.0]),
    (0.70, [40.0, 130.0, 216.0]),
    (0.84, [70.0, 62.0, 196.0]),
    (1.0, [92.0, 28.0, 128.0]),
];
const BLUES: &[(f32, [f32; 3])] = &[
    (0.0, [205.0, 240.0, 236.0]),
    (0.2, [150.0, 210.0, 234.0]),
    (0.45, [96.0, 164.0, 220.0]),
    (0.7, [58.0, 110.0, 184.0]),
    (1.0, [26.0, 52.0, 120.0]),
];
const MONO: &[(f32, [f32; 3])] = &[(0.0, [20.0, 20.0, 20.0]), (1.0, [235.0, 235.0, 235.0])];
const BANDS: &[(f32, [f32; 3])] = &[
    (0.0, [20.0, 20.0, 20.0]),
    (0.1666, [20.0, 20.0, 20.0]),
    (0.1667, [65.0, 65.0, 65.0]),
    (0.3333, [65.0, 65.0, 65.0]),
    (0.3334, [110.0, 110.0, 110.0]),
    (0.5, [110.0, 110.0, 110.0]),
    (0.5001, [150.0, 150.0, 150.0]),
    (0.6666, [150.0, 150.0, 150.0]),
    (0.6667, [190.0, 190.0, 190.0]),
    (0.8333, [190.0, 190.0, 190.0]),
    (0.8334, [235.0, 235.0, 235.0]),
    (1.0, [235.0, 235.0, 235.0]),
];

/// Colour of a depth (m, positive down) in a style.
pub fn colour(depth: f32, s: &Style) -> [f32; 3] {
    let stops = match s.palette {
        Palette::Rainbow => RAINBOW,
        Palette::Blues => BLUES,
        Palette::Mono => MONO,
        Palette::Bands => BANDS,
    };
    let span = (s.deep - s.shallow).max(0.1);
    let t = match s.target.filter(|tg| *tg > s.shallow && *tg < s.deep) {
        Some(tg) if depth <= tg => 0.5 * ((depth - s.shallow) / (tg - s.shallow)).clamp(0.0, 1.0),
        Some(tg) => 0.5 + 0.5 * ((depth - tg) / (s.deep - tg)).clamp(0.0, 1.0),
        None => ((depth - s.shallow) / span).clamp(0.0, 1.0),
    };
    let t = if s.reverse { 1.0 - t } else { t };
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let f = (t - w[0].0) / (w[1].0 - w[0].0);
            return [0, 1, 2].map(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * f);
        }
    }
    stops[stops.len() - 1].1
}

impl Store {
    /// The image for map tile `z/x/y`, or None where there are no soundings.
    pub fn tile_png(&self, z: u8, x: u32, y: u32) -> Option<Arc<Vec<u8>>> {
        if !(MIN_LEVEL..=24).contains(&z) || x >= 1 << z || y >= 1 << z {
            return None;
        }
        if let Some(t) = self.images.lock().unwrap_or_else(|e| e.into_inner()).get(&(z, x, y)) {
            return t.clone();
        }
        let png = self.render(z, x, y).map(Arc::new);
        self.images.lock().unwrap_or_else(|e| e.into_inner()).put((z, x, y), png.clone());
        png
    }

    /// Heights (m, positive up; NaN where none) for a `g` × `g` block of pixels of map tile
    /// z/x/y starting one pixel above and left of it, and the ground size of a pixel (m).
    /// Gaps are filled up to `fill_m` metres from soundings (0: none).
    pub fn heights(&self, z: u8, x: u32, y: u32, g: usize, fill_m: f32) -> Option<(Vec<f32>, f64)> {
        let level = z.min(MAX_LEVEL);
        let nz = TILE as f64 * (1u64 << z) as f64;
        let nl = cells_across(level) as f64;
        let web_lat = |py: f64| (std::f64::consts::PI * (1.0 - 2.0 * py / nz)).sinh().atan().to_degrees();
        // where each pixel's centre falls, in cells of the level
        let fx: Vec<f64> = (0..g).map(|i| (x as f64 * TILE as f64 + i as f64 - 0.5) / nz * nl).collect();
        let fy: Vec<f64> = (0..g).map(|j| to_world(0.0, web_lat(y as f64 * TILE as f64 + j as f64 - 0.5)).1 * nl).collect();
        let lat_mid = web_lat((y as f64 + 0.5) * TILE as f64);
        let cell_m = 40_075_016.686 * lat_mid.to_radians().cos() / nl;
        let r_fill = if fill_m > 0.0 { ((fill_m as f64 / cell_m).round() as i64).clamp(0, MAX_FILL_CELLS) } else { 0 };
        let r = r_fill.max(1);
        let max = nl as i64 - 1;
        let cx0 = (fx[0].floor() as i64 - r - 1).clamp(0, max);
        let cx1 = (fx[g - 1].floor() as i64 + r + 1).clamp(0, max);
        let cy0 = (fy[0].floor() as i64 - r - 1).clamp(0, max);
        let cy1 = (fy[g - 1].floor() as i64 + r + 1).clamp(0, max);
        let (w, h) = ((cx1 - cx0 + 1) as usize, (cy1 - cy0 + 1) as usize);
        let mut grid = vec![f32::NAN; w * h];
        let mut any = false;
        for ty in (cy0 >> 8)..=(cy1 >> 8) {
            for tx in (cx0 >> 8)..=(cx1 >> 8) {
                let m = self.means(level, ty as u32, tx as u32);
                if m.is_empty() {
                    continue;
                }
                let (ax, bx) = ((tx << 8).max(cx0), ((tx << 8) + 255).min(cx1));
                let (ay, by) = ((ty << 8).max(cy0), ((ty << 8) + 255).min(cy1));
                for cy in ay..=by {
                    let src = &m[((cy - (ty << 8)) as usize) * TILE..][..TILE];
                    let dst = &mut grid[(cy - cy0) as usize * w..][..w];
                    for cx in ax..=bx {
                        let v = src[(cx - (tx << 8)) as usize];
                        if v.is_finite() {
                            dst[(cx - cx0) as usize] = v;
                            any = true;
                        }
                    }
                }
            }
        }
        if !any {
            return None;
        }
        if r_fill > 0 {
            grid = fill_gaps(&grid, w, h, r_fill as usize);
        }
        let at = |cx: i64, cy: i64| -> f32 {
            if cx < cx0 || cy < cy0 || cx > cx1 || cy > cy1 {
                f32::NAN
            } else {
                grid[(cy - cy0) as usize * w + (cx - cx0) as usize]
            }
        };
        let mut out = vec![f32::NAN; g * g];
        let smooth = z > MAX_LEVEL;
        for j in 0..g {
            for i in 0..g {
                let (px, py) = (fx[i], fy[j]);
                out[j * g + i] = if smooth {
                    // between cell centres
                    let (sx, sy) = (px - 0.5, py - 0.5);
                    let (x0, y0) = (sx.floor() as i64, sy.floor() as i64);
                    let (ax, ay) = ((sx - x0 as f64) as f32, (sy - y0 as f64) as f32);
                    let v = [at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1)];
                    if v.iter().all(|v| v.is_finite()) {
                        (v[0] * (1.0 - ax) + v[1] * ax) * (1.0 - ay) + (v[2] * (1.0 - ax) + v[3] * ax) * ay
                    } else {
                        at(px.floor() as i64, py.floor() as i64)
                    }
                } else {
                    at(px.floor() as i64, py.floor() as i64)
                };
            }
        }
        let res = 40_075_016.686 * lat_mid.to_radians().cos() / nz;
        Some((out, res))
    }

    fn render(&self, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
        let style = self.style();
        const G: usize = TILE + 2;
        let mut rgba = None;
        if let Some((h, res)) = self.heights(z, x, y, G, if style.fill { style.fill_m } else { 0.0 }) {
            if !h.iter().all(|v| !v.is_finite() || *v > 0.0) {
                rgba = Some(shade(&h, G, res, contour_step(z), &style));
            }
        }
        if style.rejected {
            if let Some(mask) = self.left_out_mask(z, x, y) {
                let buf = rgba.get_or_insert_with(|| vec![0u8; TILE * TILE * 4]);
                // zoomed out a cell is a pixel: 2 × 2 so a line of them shows
                let grow = z <= MAX_LEVEL;
                for j in 0..TILE {
                    for i in 0..TILE {
                        let on = mask[j * TILE + i] || (grow && ((i > 0 && mask[j * TILE + i - 1]) || (j > 0 && mask[(j - 1) * TILE + i]) || (i > 0 && j > 0 && mask[(j - 1) * TILE + i - 1])));
                        if on {
                            let o = (j * TILE + i) * 4;
                            buf[o..o + 4].copy_from_slice(&[220, 0, 200, 255]);
                        }
                    }
                }
            }
        }
        rgba.map(|b| png_of(&b))
    }

    /// Which pixels of map tile z/x/y have a left-out sounding in them (at the level drawn
    /// at this zoom, as the seabed), or None if none do.
    fn left_out_mask(&self, z: u8, x: u32, y: u32) -> Option<Vec<bool>> {
        let level = z.min(MAX_LEVEL);
        let nz = TILE as f64 * (1u64 << z) as f64;
        let nl = cells_across(level) as f64;
        let web_lat = |py: f64| (std::f64::consts::PI * (1.0 - 2.0 * py / nz)).sinh().atan().to_degrees();
        let cx: Vec<i64> = (0..TILE).map(|i| ((x as f64 * TILE as f64 + i as f64 + 0.5) / nz * nl).floor() as i64).collect();
        let cy: Vec<i64> = (0..TILE).map(|j| (to_world(0.0, web_lat(y as f64 * TILE as f64 + j as f64 + 0.5)).1 * nl).floor() as i64).collect();
        let mut mask = vec![false; TILE * TILE];
        let mut any = false;
        let mut tiles: std::collections::HashMap<(i64, i64), Arc<Vec<f32>>> = std::collections::HashMap::new();
        for j in 0..TILE {
            for i in 0..TILE {
                let (gx, gy) = (cx[i], cy[j]);
                if gx < 0 || gy < 0 {
                    continue;
                }
                let t = tiles.entry((gy >> 8, gx >> 8)).or_insert_with(|| self.means_in(crate::LEFT_OUT, level, (gy >> 8) as u32, (gx >> 8) as u32));
                if t.is_empty() {
                    continue;
                }
                if t[((gy & 255) as usize) * TILE + (gx & 255) as usize].is_finite() {
                    mask[j * TILE + i] = true;
                    any = true;
                }
            }
        }
        any.then_some(mask)
    }
}

/// Fill the gaps (NaN) in a `w` × `h` grid up to `r` cells from data: each gap takes the mean
/// of the soundings in the smallest square around it (1, 2, 4 … `r` cells out) holding at
/// least three, so the nearest soundings count, not distant ones. Summed-area tables make
/// each pass cost the same whatever its size.
pub fn fill_gaps(grid: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let stride = w + 1;
    let mut sum = vec![0f64; stride * (h + 1)];
    let mut cnt = vec![0u32; stride * (h + 1)];
    for j in 0..h {
        let (mut rs, mut rc) = (0f64, 0u32);
        for i in 0..w {
            let v = grid[j * w + i];
            if v.is_finite() {
                rs += v as f64;
                rc += 1;
            }
            sum[(j + 1) * stride + i + 1] = sum[j * stride + i + 1] + rs;
            cnt[(j + 1) * stride + i + 1] = cnt[j * stride + i + 1] + rc;
        }
    }
    let mut out = grid.to_vec();
    let mut todo: Vec<usize> = (0..w * h).filter(|&k| !grid[k].is_finite()).collect();
    let mut rad = 1usize;
    loop {
        let rr = rad.min(r);
        todo.retain(|&k| {
            let (i, j) = (k % w, k / w);
            let (x0, x1) = (i.saturating_sub(rr), (i + rr + 1).min(w));
            let (y0, y1) = (j.saturating_sub(rr), (j + rr + 1).min(h));
            let n = cnt[y1 * stride + x1] + cnt[y0 * stride + x0] - cnt[y0 * stride + x1] - cnt[y1 * stride + x0];
            if n >= 3 {
                let sm = sum[y1 * stride + x1] + sum[y0 * stride + x0] - sum[y0 * stride + x1] - sum[y1 * stride + x0];
                out[k] = (sm / n as f64) as f32;
                false
            } else {
                true
            }
        });
        if rr >= r || todo.is_empty() {
            break;
        }
        rad *= 2;
    }
    out
}

/// The image: depth colours, lit from the north-west, contours every `step` m; clear above
/// 0 m and where there's no data. `h` is `g` × `g` with a border of one pixel.
fn shade(h: &[f32], g: usize, res: f64, step: f32, s: &Style) -> Vec<u8> {
    let n = g - 2;
    let mut rgba = vec![0u8; n * n * 4];
    let at = |x: usize, y: usize| h[y * g + x];
    let (az, alt) = (315f32.to_radians(), 45f32.to_radians());
    let (lx, ly, lz) = (alt.cos() * az.sin(), alt.cos() * az.cos(), alt.sin());
    let exaggerate = 3.0f32;
    for y in 0..n {
        for x in 0..n {
            let c = at(x + 1, y + 1);
            if !c.is_finite() || c > 0.0 {
                continue;
            }
            let pick = |xx: usize, yy: usize| {
                let v = at(xx, yy);
                if v.is_finite() {
                    v
                } else {
                    c
                }
            };
            let mut col = colour(-c, s);
            if s.shade {
                let dzdx = (pick(x + 2, y + 1) - pick(x, y + 1)) / (2.0 * res as f32) * exaggerate;
                let dzdy = (pick(x + 1, y) - pick(x + 1, y + 2)) / (2.0 * res as f32) * exaggerate;
                let norm = (dzdx * dzdx + dzdy * dzdy + 1.0).sqrt();
                let lit = ((-dzdx * lx - dzdy * ly + lz) / norm).clamp(0.0, 1.0);
                let k = (0.55 + 0.45 * lit / lz.max(0.01)).min(1.15);
                col = col.map(|v| (v * k).min(255.0));
            }
            // the target depth: a bold line (two pixels) where the seabed crosses it
            if let Some(tg) = s.target.filter(|tg| *tg > s.shallow && *tg < s.deep) {
                let side = |v: f32| -v > tg;
                let me = side(c);
                let cross = [(x + 2, y + 1), (x + 1, y + 2), (x, y + 1), (x + 1, y)].iter().any(|&(xx, yy)| {
                    let v = at(xx, yy);
                    v.is_finite() && side(v) != me
                });
                if cross {
                    let o = (y * n + x) * 4;
                    rgba[o..o + 4].copy_from_slice(&[15, 18, 28, 255]);
                    continue;
                }
            }
            if s.contours {
                let band = |v: f32| (v / step).floor();
                let (r, d) = (at(x + 2, y + 1), at(x + 1, y + 2));
                if (r.is_finite() && band(c) != band(r)) || (d.is_finite() && band(c) != band(d)) {
                    col = [col[0] * 0.4 + 20.0 * 0.6, col[1] * 0.4 + 30.0 * 0.6, col[2] * 0.4 + 45.0 * 0.6];
                }
            }
            let o = (y * n + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[col[0] as u8, col[1] as u8, col[2] as u8, 235]);
        }
    }
    rgba
}

/// A 256-pixel RGBA image as PNG.
fn png_of(rgba: &[u8]) -> Vec<u8> {
    let n = TILE;
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, n as u32, n as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        if let Ok(mut w) = enc.write_header() {
            let _ = w.write_image_data(rgba);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Store, SRC_OWN};
    use std::sync::atomic::AtomicBool;

    fn web_tile(lon: f64, lat: f64, z: u8) -> (u32, u32) {
        let n = (1u64 << z) as f64;
        let r = lat.to_radians();
        (((lon + 180.0) / 360.0 * n) as u32, ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32)
    }

    #[test]
    fn draws_soundings_where_they_are() {
        let dir = std::env::temp_dir().join(format!("bathy-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "t", 0).unwrap();
        // a 300 m × 300 m patch sloping from 20 m to 50 m, every 1.5 m
        for i in 0..200 {
            for j in 0..200 {
                let (lon, lat) = (173.2 + i as f64 * 1.5 / 84_000.0, -41.2 + j as f64 * 1.5 / 111_000.0);
                w.add(lon, lat, -20.0 - i as f32 * 0.15, 1, SRC_OWN).unwrap();
            }
        }
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        s.changed();
        for z in [10u8, 14, 16, 18] {
            let (tx, ty) = web_tile(173.2018, -41.1987, z);
            let (h, res) = s.heights(z, tx, ty, 258, 0.0).unwrap();
            // the pixel over the patch's centre has its depth (35 m there), within a cell
            let n = (1u64 << z) as f64 * 256.0;
            let px = ((173.2018 + 180.0) / 360.0 * n - tx as f64 * 256.0) as usize + 1;
            let r = (-41.1987f64).to_radians();
            let py = ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n - ty as f64 * 256.0) as usize + 1;
            let v = h[py * 258 + px];
            // the slope is 0.1 m per metre east: allow for where in the pixel the centre falls
            assert!((v + 35.0).abs() < 1.5 + 0.1 * res as f32, "z{z}: {v} (pixel {res:.1} m)");
            let png = s.tile_png(z, tx, ty).expect("an image");
            assert_eq!(&png[1..4], b"PNG");
        }
        // a tile far away has nothing
        let (tx, ty) = web_tile(174.5, -41.2, 14);
        assert!(s.tile_png(14, tx, ty).is_none());
        assert!(s.tile_png(4, 0, 0).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn marks_rejected_soundings() {
        let dir = std::env::temp_dir().join(format!("bathy-rej-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut w = s.writer("own", "t", 0).unwrap();
        for i in 0..50 {
            w.reject(173.2 + i as f64 * 0.0001, -41.2, -70.0, 1, SRC_OWN);
        }
        assert_eq!(w.rejected(), 50);
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        s.changed();
        assert_eq!(s.info().unwrap().rejected, 50);
        assert_eq!(s.info().unwrap().cells, 0);
        let (tx, ty) = web_tile(173.2025, -41.2, 15);
        assert!(s.tile_png(15, tx, ty).is_none()); // not shown unless asked for
        s.set_style(Style { rejected: true, ..Style::default() });
        let png = s.tile_png(15, tx, ty).expect("marks");
        let d = png::Decoder::new(std::io::Cursor::new(png.as_ref().clone()));
        let mut r = d.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        r.next_frame(&mut buf).unwrap();
        let magenta = buf.chunks(4).filter(|p| p == &[220, 0, 200, 255]).count();
        assert!(magenta > 50, "{magenta}");
        // zoomed right out too
        let (tx, ty) = web_tile(173.2025, -41.2, 8);
        assert!(s.tile_png(8, tx, ty).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gaps_fill_from_the_nearest_soundings() {
        // two lines of soundings 10 cells apart: -10 m and -30 m
        let (w, h) = (40, 40);
        let mut g = vec![f32::NAN; w * h];
        for j in 0..h {
            g[j * w + 10] = -10.0;
            g[j * w + 20] = -30.0;
        }
        let f = fill_gaps(&g, w, h, 2);
        assert_eq!(f[5 * w + 10], -10.0); // data stays as it is
        assert!((f[5 * w + 11] + 10.0).abs() < 1e-4); // next to the first line: its depth only
        assert!(f[5 * w + 15].is_nan()); // five cells from both: beyond 2
        let f = fill_gaps(&g, w, h, 8);
        assert!((f[5 * w + 11] + 10.0).abs() < 1e-4); // still the nearest line, not the mean
        assert!((f[5 * w + 15] + 20.0).abs() < 1e-4); // midway: both lines equally
        assert!(f[5 * w + 35].is_nan()); // 15 cells out: beyond 8
    }

    #[test]
    fn a_target_splits_the_palette() {
        let s = Style { shallow: 0.0, deep: 1000.0, target: Some(200.0), ..Style::default() };
        // 200 m is the middle of the palette; 100 m a quarter; 600 m three quarters
        let mid = colour(200.0, &Style { target: None, shallow: 0.0, deep: 400.0, ..Style::default() });
        assert_eq!(colour(200.0, &s), mid);
        assert_eq!(colour(100.0, &s), colour(100.0, &Style { target: None, shallow: 0.0, deep: 400.0, ..Style::default() }));
        assert_eq!(colour(600.0, &s), colour(300.0, &Style { target: None, shallow: 0.0, deep: 400.0, ..Style::default() }));
        // a target outside the range is ignored
        let out = Style { target: Some(2000.0), ..s.clone() };
        assert_eq!(colour(500.0, &out), colour(500.0, &Style { target: None, ..s }));
    }

    #[test]
    fn colours() {
        let s = Style::default();
        assert_eq!(colour(-5.0, &s), RAINBOW[0].1);
        assert_eq!(colour(9999.0, &s), RAINBOW[RAINBOW.len() - 1].1);
        let b = Style { palette: Palette::Blues, ..Style::default() };
        assert_eq!(colour(0.0, &b), BLUES[0].1);
    }
}

#[cfg(test)]
mod real {
    use crate::Store;
    use std::sync::atomic::AtomicBool;

    /// `RACENAV_TEST_PBG=/path/file.tzpbg cargo test -p bathy --release real_pbg -- --ignored --nocapture`
    /// Imports the file into a scratch store and draws an overview PNG of each zoom 8 tile with data.
    #[test]
    #[ignore]
    fn real_pbg() {
        let p = std::path::PathBuf::from(std::env::var("RACENAV_TEST_PBG").unwrap());
        let out = std::env::var("RACENAV_TEST_OUT").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("bathy-real"));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let s = Store::open(&out.join("bathy.sqlite")).unwrap();
        let t = std::time::Instant::now();
        let mut last = String::new();
        let n = s
            .import_pbg(
                &p,
                &mut |st, d, tot| {
                    if st != last {
                        println!("{st} {d}/{tot} at {:?}", t.elapsed());
                        last = st.to_string();
                    }
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        println!("imported {n} cells in {:?}", t.elapsed());
        let info = s.info().unwrap();
        println!("{:?}", (info.cells, info.tiles, info.bounds, info.bytes));
        let b = info.bounds.unwrap();
        // the main area's colour range
        if let Some(r) = s.depth_range(b) {
            println!("depth range {r:?}");
            s.set_style(crate::Style { deep: r.1, shallow: r.0, ..crate::Style::default() });
        }
        let t = std::time::Instant::now();
        let mut drawn = 0;
        for z in [8u8, 12] {
            let n = (1u64 << z) as f64;
            let tx = |lon: f64| ((lon + 180.0) / 360.0 * n) as u32;
            let ty = |lat: f64| {
                let r = lat.to_radians();
                ((1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32
            };
            for x in tx(b[0].max(160.0))..=tx(b[2]) {
                for y in ty(b[3].min(-30.0))..=ty(b[1]) {
                    if let Some(png) = s.tile_png(z, x, y) {
                        drawn += 1;
                        if z == 8 {
                            std::fs::write(out.join(format!("z{z}_{x}_{y}.png")), png.as_ref()).unwrap();
                        }
                    }
                }
            }
            println!("z{z}: {drawn} tiles drawn so far, {:?}", t.elapsed());
        }
        assert!(drawn > 0);
    }
}
