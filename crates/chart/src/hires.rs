//! High-resolution depths: LINZ's coastal elevation models (1 m, from aerial laser
//! surveys, down to about 25 m depth), as downloaded GeoTIFF tiles in NZTM2000, drawn as
//! shaded depth images with contours for the chart (`<z>/<x>/<y>` 256-pixel PNG tiles).
//!
//! Heights are above NZVD2016 (about mean sea level), not chart datum: the depths shown
//! are deeper than charted depths by roughly half the tidal range. Above 0 m is left
//! clear (the charts show the land and drying areas).
//!
//! The tiles' names give where they are: LINZ's 1:10 000 tiles of the Topo50 sheets
//! (`BQ31_10000_0401`: sheet BQ31, row 4, column 1), each 4800 m × 7200 m.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::cog::Cog;
use crate::lerc::Raster;
use crate::shp::{lonlat_to_nztm, nztm_to_lonlat};

const SHEET_W: f64 = 24_000.0;
const SHEET_H: f64 = 36_000.0;
const ORIGIN_LEFT: f64 = 988_000.0;
const ORIGIN_TOP: f64 = 6_234_000.0;
/// Letters of the Topo50 sheet rows (no I or O); row "AS" is the first.
const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const TILE_W: f64 = 4_800.0;
const TILE_H: f64 = 7_200.0;
/// The first zoom the depths are drawn at.
pub const MIN_ZOOM: u8 = 10;

/// NZTM bounds [min e, min n, max e, max n] of a 1:10 000 tile from its name.
pub fn tile_bounds(name: &str) -> Option<[f64; 4]> {
    let b = name.as_bytes();
    if b.len() < 15 || &name[4..11] != "_10000_" {
        return None;
    }
    let li = |c: u8| LETTERS.iter().position(|&l| l == c);
    let row = li(b[0])? * LETTERS.len() + li(b[1])?;
    let first = li(b'A')? * LETTERS.len() + li(b'S')?;
    let row = row.checked_sub(first)? as f64;
    let col: f64 = name[2..4].parse().ok()?;
    let (r, c): (f64, f64) = (name[11..13].parse().ok()?, name[13..15].parse().ok()?);
    if !(1.0..=5.0).contains(&r) || !(1.0..=5.0).contains(&c) {
        return None;
    }
    let left = ORIGIN_LEFT + col * SHEET_W + (c - 1.0) * TILE_W;
    let top = ORIGIN_TOP - row * SHEET_H - (r - 1.0) * TILE_H;
    Some([left, top - TILE_H, left + TILE_W, top])
}

/// The names of the 1:10 000 tiles covering an area [w, s, e, n] in longitude and latitude.
pub fn tile_names(area: [f64; 4]) -> Vec<String> {
    // the area's NZTM box, from its corners and edge midpoints
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for i in 0..=4 {
        for j in 0..=4 {
            let p = lonlat_to_nztm([
                area[0] + (area[2] - area[0]) * i as f64 / 4.0,
                area[1] + (area[3] - area[1]) * j as f64 / 4.0,
            ]);
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ];
        }
    }
    let mut out = Vec::new();
    let first = LETTERS.iter().position(|&l| l == b'A').unwrap() * LETTERS.len()
        + LETTERS.iter().position(|&l| l == b'S').unwrap();
    let c0 = ((b[0] - ORIGIN_LEFT) / TILE_W).floor().max(0.0) as i64;
    let c1 = ((b[2] - ORIGIN_LEFT) / TILE_W).floor() as i64;
    let r0 = ((ORIGIN_TOP - b[3]) / TILE_H).floor().max(0.0) as i64;
    let r1 = ((ORIGIN_TOP - b[1]) / TILE_H).floor() as i64;
    if (c1 - c0 + 1) * (r1 - r0 + 1) > 20_000 {
        return out;
    }
    for r in r0..=r1 {
        for c in c0..=c1 {
            let (sheet_row, tr) = ((r / 5) as usize + first, r % 5 + 1);
            let (sheet_col, tc) = (c / 5, c % 5 + 1);
            if sheet_row / LETTERS.len() >= LETTERS.len() || sheet_col > 99 {
                continue;
            }
            let l1 = LETTERS[sheet_row / LETTERS.len()] as char;
            let l2 = LETTERS[sheet_row % LETTERS.len()] as char;
            out.push(format!("{l1}{l2}{sheet_col:02}_10000_{tr:02}{tc:02}"));
        }
    }
    out
}

type BlockKey = (usize, usize, usize, usize);

/// The downloaded tiles, and caches of decoded blocks and drawn images.
pub struct Hires {
    cogs: Vec<Cog>,
    /// Each file's NZTM bounds.
    bounds: Vec<[f64; 4]>,
    blocks: Mutex<lru::LruCache<BlockKey, Arc<Raster>>>,
    images: Mutex<lru::LruCache<(u8, u32, u32), Arc<Vec<u8>>>>,
}

impl Hires {
    /// Every readable GeoTIFF in `dir`.
    pub fn open(dir: &Path) -> Hires {
        let mut cogs = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut paths: Vec<_> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().is_some_and(|x| {
                        x.eq_ignore_ascii_case("tiff") || x.eq_ignore_ascii_case("tif")
                    })
                })
                .collect();
            paths.sort();
            for p in paths {
                match Cog::open(&p) {
                    Ok(c) => cogs.push(c),
                    Err(e) => eprintln!("high-resolution depths: {}: {e}", p.display()),
                }
            }
        }
        let bounds = cogs.iter().map(Cog::bounds).collect();
        Hires {
            cogs,
            bounds,
            blocks: Mutex::new(lru::LruCache::new(NonZeroUsize::new(256).unwrap())),
            images: Mutex::new(lru::LruCache::new(NonZeroUsize::new(512).unwrap())),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cogs.is_empty()
    }

    /// Each tile: (file name, [w, s, e, n] in longitude and latitude).
    pub fn coverage(&self) -> Vec<(String, [f64; 4])> {
        self.cogs
            .iter()
            .zip(&self.bounds)
            .map(|(c, b)| {
                let sw = nztm_to_lonlat([b[0], b[1]]);
                let ne = nztm_to_lonlat([b[2], b[3]]);
                let nw = nztm_to_lonlat([b[0], b[3]]);
                let se = nztm_to_lonlat([b[2], b[1]]);
                let name = c
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (
                    name,
                    [
                        sw[0].min(nw[0]),
                        sw[1].min(se[1]),
                        ne[0].max(se[0]),
                        ne[1].max(nw[1]),
                    ],
                )
            })
            .collect()
    }

    fn block(&self, key: BlockKey) -> Option<Arc<Raster>> {
        if let Some(b) = self
            .blocks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return Some(b.clone());
        }
        let r = Arc::new(self.cogs[key.0].tile(key.1, key.2, key.3).ok()?);
        self.blocks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(key, r.clone());
        Some(r)
    }

    /// Height (above NZVD2016) at a place, from the finest data.
    pub fn height_at(&self, lon: f64, lat: f64) -> Option<f32> {
        let p = lonlat_to_nztm([lon, lat]);
        self.cogs
            .iter()
            .zip(&self.bounds)
            .find(|(_, b)| p[0] >= b[0] && p[0] < b[2] && p[1] >= b[1] && p[1] < b[3])
            .and_then(|(c, _)| c.value_at(p[0], p[1]))
    }

    /// The image for map tile `z/x/y` (256 pixels, PNG), or None where there is no data.
    pub fn tile_png(&self, z: u8, x: u32, y: u32) -> Option<Arc<Vec<u8>>> {
        if z < MIN_ZOOM || self.cogs.is_empty() {
            return None;
        }
        if let Some(t) = self
            .images
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(z, x, y))
        {
            return Some(t.clone());
        }
        let png = Arc::new(self.render(z, x, y)?);
        self.images
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put((z, x, y), png.clone());
        Some(png)
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
        // which files reach this tile
        let corners = [
            (0.0, 0.0),
            (N as f64, 0.0),
            (0.0, N as f64),
            (N as f64, N as f64),
            (N as f64 / 2.0, N as f64 / 2.0),
        ];
        let mut tb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for (px, py) in corners {
            let p = lonlat_to_nztm([lon_at(px), lat_at(py)]);
            tb = [
                tb[0].min(p[0]),
                tb[1].min(p[1]),
                tb[2].max(p[0]),
                tb[3].max(p[1]),
            ];
        }
        let pad = 20.0;
        let files: Vec<usize> = (0..self.cogs.len())
            .filter(|&i| {
                let b = self.bounds[i];
                b[0] <= tb[2] + pad
                    && b[2] >= tb[0] - pad
                    && b[1] <= tb[3] + pad
                    && b[3] >= tb[1] - pad
            })
            .collect();
        if files.is_empty() {
            return None;
        }
        // ground size of a pixel here, and the coarsest overview at least that fine
        let lat_mid = lat_at(N as f64 / 2.0);
        let res = 40_075_016.686 * lat_mid.to_radians().cos() / (N as f64 * n);
        let mut local: HashMap<BlockKey, Option<Arc<Raster>>> = HashMap::new();
        let mut heights = vec![f32::NAN; G * G];
        for gy in 0..G {
            let lat = lat_at(gy as f64 - 0.5);
            for gx in 0..G {
                let p = lonlat_to_nztm([lon_at(gx as f64 - 0.5), lat]);
                for &fi in &files {
                    let b = self.bounds[fi];
                    if p[0] < b[0] || p[0] >= b[2] || p[1] < b[1] || p[1] >= b[3] {
                        continue;
                    }
                    let c = &self.cogs[fi];
                    let li = c
                        .levels
                        .iter()
                        .rposition(|l| l.pixel.0 <= res * 1.01)
                        .unwrap_or(0);
                    if let Some(h) = self.sample(&mut local, fi, li, p) {
                        heights[gy * G + gx] = h;
                        break;
                    }
                }
            }
        }
        if heights.iter().all(|h| h.is_nan() || *h > 0.0) {
            return None;
        }
        Some(shade(&heights, G, res, contour_step(z)))
    }

    /// Bilinear height at NZTM point `p` from level `li` of file `fi` (nearest where a
    /// neighbour is missing).
    fn sample(
        &self,
        local: &mut HashMap<BlockKey, Option<Arc<Raster>>>,
        fi: usize,
        li: usize,
        p: [f64; 2],
    ) -> Option<f32> {
        let c = &self.cogs[fi];
        let l = &c.levels[li];
        let fx = (p[0] - c.origin.0) / l.pixel.0 - 0.5;
        let fy = (c.origin.1 - p[1]) / l.pixel.1 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = ((fx - x0) as f32, (fy - y0) as f32);
        let mut get = |ix: f64, iy: f64| -> Option<f32> {
            if ix < 0.0 || iy < 0.0 || ix >= l.width as f64 || iy >= l.height as f64 {
                return None;
            }
            let (ix, iy) = (ix as usize, iy as usize);
            let key = (fi, li, ix / l.tile_w, iy / l.tile_h);
            let b = local
                .entry(key)
                .or_insert_with(|| self.block(key))
                .as_ref()?;
            b.get(ix % l.tile_w, iy % l.tile_h)
        };
        let v = [
            get(x0, y0),
            get(x0 + 1.0, y0),
            get(x0, y0 + 1.0),
            get(x0 + 1.0, y0 + 1.0),
        ];
        match v {
            [Some(a), Some(b), Some(c2), Some(d)] => {
                Some((a * (1.0 - ax) + b * ax) * (1.0 - ay) + (c2 * (1.0 - ax) + d * ax) * ay)
            }
            _ => {
                let near = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
                let k = near.iter().enumerate().min_by(|a, b| {
                    let d = |(dx, dy): (f32, f32)| (ax - dx).powi(2) + (ay - dy).powi(2);
                    d(*a.1).total_cmp(&d(*b.1))
                })?;
                v[k.0].or_else(|| v.iter().flatten().next().copied())
            }
        }
    }
}

/// Contour interval (m) at a zoom.
fn contour_step(z: u8) -> f32 {
    match z {
        16.. => 1.0,
        14 | 15 => 2.0,
        12 | 13 => 5.0,
        _ => 10.0,
    }
}

/// Depth colour for a height (negative below the datum).
fn colour(h: f32) -> [f32; 3] {
    const STOPS: &[(f32, [f32; 3])] = &[
        (0.0, [205.0, 240.0, 236.0]),
        (2.0, [170.0, 222.0, 236.0]),
        (5.0, [135.0, 200.0, 232.0]),
        (10.0, [104.0, 172.0, 222.0]),
        (20.0, [78.0, 140.0, 204.0]),
        (40.0, [58.0, 108.0, 178.0]),
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

/// The image: depth colours, lit from the north-west, with contours every `step` m;
/// clear above 0 m and where there is no data. `h` is `g` × `g` with a border of one.
fn shade(h: &[f32], g: usize, res: f64, step: f32) -> Vec<u8> {
    let n = g - 2;
    let mut rgba = vec![0u8; n * n * 4];
    let at = |x: usize, y: usize| h[y * g + x];
    let (az, alt) = (315f32.to_radians(), 45f32.to_radians());
    let (lx, ly, lz) = (alt.cos() * az.sin(), alt.cos() * az.cos(), alt.sin());
    let exaggerate = 3.0f32;
    for y in 0..n {
        for x in 0..n {
            let c = at(x + 1, y + 1);
            if c.is_nan() || c > 0.0 {
                continue;
            }
            let pick = |xx: usize, yy: usize| {
                let v = at(xx, yy);
                if v.is_nan() {
                    c
                } else {
                    v
                }
            };
            let dzdx = (pick(x + 2, y + 1) - pick(x, y + 1)) / (2.0 * res as f32) * exaggerate;
            let dzdy = (pick(x + 1, y) - pick(x + 1, y + 2)) / (2.0 * res as f32) * exaggerate; // north is up
            let norm = (dzdx * dzdx + dzdy * dzdy + 1.0).sqrt();
            let lit = ((-dzdx * lx - dzdy * ly + lz) / norm).clamp(0.0, 1.0);
            let k = 0.55 + 0.45 * lit / lz.max(0.01);
            let mut col = colour(c).map(|v| (v * k.min(1.15)).min(255.0));
            // contour where the band changes to the right or below
            let band = |v: f32| (v / step).floor();
            let (r, d) = (pick(x + 2, y + 1), pick(x + 1, y + 2));
            if band(c) != band(r) || band(c) != band(d) {
                col = [
                    col[0] * 0.45 + 30.0 * 0.55,
                    col[1] * 0.45 + 70.0 * 0.55,
                    col[2] * 0.45 + 105.0 * 0.55,
                ];
            }
            let o = (y * n + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[col[0] as u8, col[1] as u8, col[2] as u8, 235]);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, n as u32, n as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        if let Ok(mut w) = enc.write_header() {
            let _ = w.write_image_data(&rgba);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_names_and_bounds() {
        // LINZ's own example: BH31_10000_0403 starts at (1741600, 5672400)
        let b = tile_bounds("BH31_10000_0403").unwrap();
        assert_eq!(b, [1741600.0, 5665200.0, 1746400.0, 5672400.0]);
        assert!(tile_bounds("BH31_50000_0403").is_none());
        assert!(tile_bounds("BH31_10000_0603").is_none());
        // the tile's centre names it back
        let c = nztm_to_lonlat([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]);
        assert_eq!(
            tile_names([c[0], c[1], c[0], c[1]]),
            vec!["BH31_10000_0403".to_string()]
        );
        // an area a few km across: a handful of tiles, all with good bounds
        let names = tile_names([173.25, -41.3, 173.32, -41.24]);
        assert!((2..=12).contains(&names.len()), "{names:?}");
        assert!(names.iter().all(|n| tile_bounds(n).is_some()));
    }

    #[test]
    fn draws_the_fixture() {
        let dir = std::env::temp_dir().join(format!("tp_hires_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/coastal_dem.tiff"),
            dir.join("BH31_10000_0403.tiff"),
        )
        .unwrap();
        let h = Hires::open(&dir);
        assert!(!h.is_empty());
        let cov = h.coverage();
        assert_eq!(cov.len(), 1);
        // a point in the tile: the height there
        let p = nztm_to_lonlat([1741600.0 + 10.5, 5672400.0 - 3.2]);
        let v = h.height_at(p[0], p[1]).unwrap();
        assert!(
            (v - (-20.0
                + 0.12 * 10.0
                + 0.03 * 3.0
                + 1.5 * (10f32 / 9.0).sin() * (3f32 / 13.0).cos()))
            .abs()
                < 0.002,
            "{v}"
        );
        // the map tile over it at zoom 17 draws; one far away doesn't
        let n = (1u64 << 17) as f64;
        let c = nztm_to_lonlat([1741700.0, 5672300.0]);
        let tx = ((c[0] + 180.0) / 360.0 * n) as u32;
        let ty = ((1.0
            - (c[1].to_radians().tan() + 1.0 / c[1].to_radians().cos()).ln()
                / std::f64::consts::PI)
            / 2.0
            * n) as u32;
        let png = h.tile_png(17, tx, ty).expect("an image");
        assert_eq!(&png[1..4], b"PNG");
        let d = png::Decoder::new(std::io::Cursor::new(png.as_ref().clone()));
        let mut r = d.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        r.next_frame(&mut buf).unwrap();
        let opaque = buf.chunks(4).filter(|p| p[3] > 0).count();
        assert!(opaque > 1000, "{opaque}");
        assert!(h.tile_png(17, tx + 50, ty).is_none());
        assert!(h.tile_png(8, tx >> 9, ty >> 9).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
