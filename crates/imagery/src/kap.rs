//! BSB/KAP raster charts (BSB versions 1 to 3, not encrypted): the text header (name,
//! size, palette, reference points, border, datum shift) and the run-length coded rows.
//!
//! The chart is placed by fitting its reference points (REF) in the chart's projection:
//! exactly for mercator and transverse mercator charts, by a least-squares quadratic for
//! others. The fit's residual is kept as a check on the chart's own georeferencing. The
//! datum shift (DTM, seconds, added to the chart's positions to give WGS84) is applied.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use lru::LruCache;

use crate::geo::{distance_m, least_squares, tm_forward, tm_inverse, web_mercator, web_mercator_inv, Tm};
use crate::render::{Block, Raster};
use crate::{err, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Proj {
    Mercator,
    Tm(f64),
    /// Anything else (polyconic, Lambert…): a quadratic in mercator coordinates.
    Other,
}

/// A reference point: pixel (x, y) and its position in the chart's datum.
#[derive(Clone, Copy, Debug)]
pub struct Ref {
    pub x: f64,
    pub y: f64,
    pub lat: f64,
    pub lon: f64,
}

pub struct Kap {
    pub name: String,
    pub number: String,
    pub edition: String,
    pub scale: Option<u64>,
    pub datum: String,
    pub projection: String,
    pub width: usize,
    pub height: usize,
    data: Vec<u8>,
    depth: u8,
    rows: Vec<usize>,
    palette: Arc<Vec<[u8; 4]>>,
    proj: Proj,
    /// Longitude the chart's longitudes are unwrapped round (charts over 180°).
    lon_c: f64,
    /// DTM: seconds of latitude and longitude added to chart positions to give WGS84.
    pub dtm: (f64, f64),
    pub refs: Vec<Ref>,
    /// pixel = Σ cx·term(u, v), with u, v the projected position, centred and scaled.
    fit_x: Vec<f64>,
    fit_y: Vec<f64>,
    /// The reverse fit: (u, v) from the pixel, for the corners and the resolution.
    inv_u: Vec<f64>,
    inv_v: Vec<f64>,
    norm: (f64, f64, f64),
    pnorm: (f64, f64, f64),
    quadratic: bool,
    /// RMS of the reference points' residuals, pixels.
    pub fit_rms_px: f64,
    /// The border (PLY) in pixels.
    border: Vec<(f64, f64)>,
    bounds: [f64; 4],
    res_m: f64,
    cache: Mutex<LruCache<usize, Arc<Block>>>,
}

fn terms(u: f64, v: f64, quadratic: bool) -> Vec<f64> {
    if quadratic {
        vec![1.0, u, v, u * u, u * v, v * v]
    } else {
        vec![1.0, u, v]
    }
}

fn eval(c: &[f64], u: f64, v: f64, quadratic: bool) -> f64 {
    terms(u, v, quadratic).iter().zip(c).map(|(t, k)| t * k).sum()
}

/// `KEY=value` inside a header line's fields: up to the next `,XX=` or the end.
fn field<'a>(rest: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{key}=");
    let mut start = None;
    let b = rest.as_bytes();
    let mut i = 0;
    while let Some(p) = rest[i..].find(&pat) {
        let at = i + p;
        if at == 0 || b[at - 1] == b',' || b[at - 1] == b'/' {
            start = Some(at + pat.len());
            break;
        }
        i = at + 1;
    }
    let s = start?;
    let tail = &rest[s..];
    let tb = tail.as_bytes();
    let mut end = tail.len();
    for j in 0..tb.len() {
        if tb[j] == b',' && tb.len() > j + 3 && tb[j + 1].is_ascii_uppercase() && tb[j + 2].is_ascii_uppercase() && tb[j + 3] == b'=' {
            end = j;
            break;
        }
    }
    Some(tail[..end].trim())
}

fn nums(s: &str) -> Vec<f64> {
    s.split(',').filter_map(|v| v.trim().parse::<f64>().ok()).collect()
}

impl Kap {
    pub fn open(path: &Path) -> Result<Kap> {
        let data = std::fs::read(path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
        Kap::from_bytes(data)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Kap> {
        let Some(hdr_end) = data.iter().position(|&b| b == 0x1A) else { return err("KAP: no end to the header (not a KAP file?)") };
        let text = String::from_utf8_lossy(&data[..hdr_end]).into_owned();
        // logical lines: a line starting with spaces continues the one before
        let mut lines: Vec<String> = Vec::new();
        for raw in text.split(['\n', '\r']) {
            if raw.is_empty() {
                continue;
            }
            if raw.starts_with(' ') && !lines.is_empty() {
                let l = lines.last_mut().expect("a line");
                if !l.ends_with(',') {
                    l.push(',');
                }
                l.push_str(raw.trim());
            } else if !raw.starts_with('!') {
                lines.push(raw.trim().to_string());
            }
        }
        let (mut name, mut number, mut edition, mut datum, mut projection) = (String::new(), String::new(), String::new(), String::new(), String::new());
        let (mut width, mut height) = (0usize, 0usize);
        let mut scale = None;
        let mut pp = None;
        let mut dtm = (0.0, 0.0);
        let mut refs = Vec::new();
        let mut ply = Vec::new();
        let mut rgb: Vec<(usize, [u8; 4])> = Vec::new();
        let mut day: Vec<(usize, [u8; 4])> = Vec::new();
        for l in &lines {
            let Some((tag, rest)) = l.split_once('/') else { continue };
            match tag {
                "BSB" | "NOS" => {
                    if let Some(v) = field(rest, "NA") {
                        name = v.to_string();
                    }
                    if let Some(v) = field(rest, "NU") {
                        number = v.to_string();
                    }
                    if let Some(v) = field(rest, "RA") {
                        let n = nums(v);
                        if n.len() >= 2 {
                            (width, height) = (n[0] as usize, n[1] as usize);
                        }
                    }
                }
                "KNP" => {
                    if let Some(v) = field(rest, "SC") {
                        scale = v.parse::<f64>().ok().map(|s| s as u64);
                    }
                    if let Some(v) = field(rest, "GD") {
                        datum = v.to_string();
                    }
                    if let Some(v) = field(rest, "PR") {
                        projection = v.to_string();
                    }
                    if let Some(v) = field(rest, "PP") {
                        pp = v.parse::<f64>().ok();
                    }
                }
                "CED" => {
                    if let Some(v) = field(rest, "ED") {
                        edition = v.to_string();
                    }
                }
                "DTM" => {
                    let n = nums(rest);
                    if n.len() >= 2 {
                        dtm = (n[0], n[1]);
                    }
                }
                "REF" => {
                    let n = nums(rest);
                    if n.len() >= 5 {
                        refs.push(Ref { x: n[1], y: n[2], lat: n[3], lon: n[4] });
                    }
                }
                "PLY" => {
                    let n = nums(rest);
                    if n.len() >= 3 {
                        ply.push((n[1], n[2]));
                    }
                }
                "RGB" | "DAY" => {
                    let n = nums(rest);
                    if n.len() >= 4 {
                        let c = [n[1] as u8, n[2] as u8, n[3] as u8, 255];
                        if tag == "RGB" {
                            rgb.push((n[0] as usize, c));
                        } else {
                            day.push((n[0] as usize, c));
                        }
                    }
                }
                _ => {}
            }
        }
        if width == 0 || height == 0 {
            return err("KAP: no image size (RA) in the header");
        }
        if width > 100_000 || height > 100_000 {
            return err("KAP: image size out of range");
        }
        if refs.len() < 3 {
            return err(format!("KAP: {} reference points (REF); at least 3 are needed to place the chart", refs.len()));
        }
        let colours = if rgb.is_empty() { day } else { rgb };
        if colours.is_empty() {
            return err("KAP: no colour palette (RGB) in the header");
        }
        let mut palette = vec![[0u8; 4]; 256];
        for (i, c) in colours {
            if i < 256 {
                palette[i] = c;
            }
        }
        palette[0] = [0, 0, 0, 0];

        // the bitmap: 0x1A, 0x00, the bits-per-pixel byte, then the rows
        let mut p = hdr_end + 1;
        if data.get(p) == Some(&0) {
            p += 1;
        }
        let depth = *data.get(p).ok_or_else(|| crate::Error("KAP: no image data".into()))?;
        if !(1..=7).contains(&depth) {
            return err(format!("KAP: bad colour depth {depth}"));
        }
        let rows_start = p + 1;
        let rows = row_index(&data, rows_start, height)
            .or_else(|| scan_rows(&data, rows_start, height))
            .ok_or_else(|| crate::Error("KAP: image rows not found".into()))?;

        let up = projection.to_ascii_uppercase();
        let proj = if up.starts_with("MERCATOR") {
            Proj::Mercator
        } else if up.contains("TRANSVERSE") {
            Proj::Tm(pp.unwrap_or_else(|| refs.iter().map(|r| r.lon).sum::<f64>() / refs.len() as f64))
        } else {
            Proj::Other
        };
        let lon_c = refs[0].lon;
        let mut kap = Kap {
            name,
            number,
            edition,
            scale,
            datum,
            projection,
            width,
            height,
            data,
            depth,
            rows,
            palette: Arc::new(palette),
            proj,
            lon_c,
            dtm,
            refs,
            fit_x: Vec::new(),
            fit_y: Vec::new(),
            inv_u: Vec::new(),
            inv_v: Vec::new(),
            norm: (0.0, 0.0, 1.0),
            pnorm: (0.0, 0.0, 1.0),
            quadratic: false,
            fit_rms_px: 0.0,
            border: Vec::new(),
            bounds: [0.0; 4],
            res_m: 0.0,
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(4096).expect("non-zero"))),
        };
        kap.fit()?;
        kap.border = ply.iter().filter_map(|&(lat, lon)| kap.chart_to_pixel(lon, lat)).collect();
        if kap.border.len() < 3 {
            kap.border.clear();
        }
        kap.bounds = kap.compute_bounds(&ply);
        kap.res_m = kap.compute_res();
        Ok(kap)
    }

    fn unwrap_lon(&self, lon: f64) -> f64 {
        let mut l = lon;
        while l - self.lon_c > 180.0 {
            l -= 360.0;
        }
        while l - self.lon_c < -180.0 {
            l += 360.0;
        }
        l
    }

    /// Chart-datum longitude and latitude to projected (u, v) before normalising.
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        let lon = self.unwrap_lon(lon);
        match self.proj {
            Proj::Mercator | Proj::Other => web_mercator(lon, lat),
            Proj::Tm(lon0) => tm_forward(&Tm { lon0, k0: 1.0, false_e: 0.0, false_n: 0.0 }, lon, lat),
        }
    }

    fn unproject(&self, u: f64, v: f64) -> (f64, f64) {
        match self.proj {
            Proj::Mercator | Proj::Other => web_mercator_inv(u, v),
            Proj::Tm(lon0) => tm_inverse(&Tm { lon0, k0: 1.0, false_e: 0.0, false_n: 0.0 }, u, v),
        }
    }

    fn fit(&mut self) -> Result<()> {
        let pts: Vec<(f64, f64, f64, f64)> = self
            .refs
            .iter()
            .map(|r| {
                let (u, v) = self.project(r.lon, r.lat);
                (u, v, r.x, r.y)
            })
            .collect();
        let n = pts.len() as f64;
        let (mu, mv) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
        let s = pts.iter().map(|p| (p.0 - mu).abs().max((p.1 - mv).abs())).fold(0.0, f64::max).max(1.0);
        self.norm = (mu, mv, s);
        let (mx, my) = (pts.iter().map(|p| p.2).sum::<f64>() / n, pts.iter().map(|p| p.3).sum::<f64>() / n);
        let ps = pts.iter().map(|p| (p.2 - mx).abs().max((p.3 - my).abs())).fold(0.0, f64::max).max(1.0);
        self.pnorm = (mx, my, ps);
        let try_fit = |quadratic: bool| -> Option<(Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, f64)> {
            let rows: Vec<Vec<f64>> = pts.iter().map(|p| terms((p.0 - mu) / s, (p.1 - mv) / s, quadratic)).collect();
            let fx = least_squares(&rows, &pts.iter().map(|p| p.2).collect::<Vec<_>>())?;
            let fy = least_squares(&rows, &pts.iter().map(|p| p.3).collect::<Vec<_>>())?;
            let prow: Vec<Vec<f64>> = pts.iter().map(|p| terms((p.2 - mx) / ps, (p.3 - my) / ps, quadratic)).collect();
            let iu = least_squares(&prow, &pts.iter().map(|p| (p.0 - mu) / s).collect::<Vec<_>>())?;
            let iv = least_squares(&prow, &pts.iter().map(|p| (p.1 - mv) / s).collect::<Vec<_>>())?;
            let sq: f64 = pts
                .iter()
                .map(|p| {
                    let (u, v) = ((p.0 - mu) / s, (p.1 - mv) / s);
                    (eval(&fx, u, v, quadratic) - p.2).powi(2) + (eval(&fy, u, v, quadratic) - p.3).powi(2)
                })
                .sum();
            Some((fx, fy, iu, iv, (sq / n).sqrt()))
        };
        let affine = try_fit(false);
        let want_quad = pts.len() >= 6 && (self.proj == Proj::Other || affine.as_ref().is_none_or(|a| a.4 > 2.0));
        let chosen = if want_quad {
            match try_fit(true) {
                Some(q) if affine.as_ref().is_none_or(|a| q.4 < a.4) => Some((q, true)),
                _ => affine.map(|a| (a, false)),
            }
        } else {
            affine.map(|a| (a, false))
        };
        let Some(((fx, fy, iu, iv, rms), quad)) = chosen else { return err("KAP: the reference points don't place the chart (all in a line?)") };
        self.fit_x = fx;
        self.fit_y = fy;
        self.inv_u = iu;
        self.inv_v = iv;
        self.quadratic = quad;
        self.fit_rms_px = rms;
        Ok(())
    }

    /// A chart-datum position to pixels.
    fn chart_to_pixel(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let (u, v) = self.project(lon, lat);
        let (u, v) = ((u - self.norm.0) / self.norm.2, (v - self.norm.1) / self.norm.2);
        let p = (eval(&self.fit_x, u, v, self.quadratic), eval(&self.fit_y, u, v, self.quadratic));
        (p.0.is_finite() && p.1.is_finite()).then_some(p)
    }

    /// Pixels to a WGS84 position (approximate: the reverse fit).
    pub fn pixel_to_lonlat(&self, x: f64, y: f64) -> (f64, f64) {
        let (a, b) = ((x - self.pnorm.0) / self.pnorm.2, (y - self.pnorm.1) / self.pnorm.2);
        let u = eval(&self.inv_u, a, b, self.quadratic) * self.norm.2 + self.norm.0;
        let v = eval(&self.inv_v, a, b, self.quadratic) * self.norm.2 + self.norm.1;
        let (lon, lat) = self.unproject(u, v);
        (wrap(lon + self.dtm.1 / 3600.0), lat + self.dtm.0 / 3600.0)
    }

    fn compute_bounds(&self, ply: &[(f64, f64)]) -> [f64; 4] {
        let mut pts: Vec<(f64, f64)> = if ply.len() >= 3 {
            ply.iter().map(|&(lat, lon)| (self.unwrap_lon(lon) + self.dtm.1 / 3600.0, lat + self.dtm.0 / 3600.0)).collect()
        } else {
            Vec::new()
        };
        if pts.is_empty() {
            let (w, h) = (self.width as f64, self.height as f64);
            for i in 0..=8 {
                let t = i as f64 / 8.0;
                for (x, y) in [(t * w, 0.0), (t * w, h), (0.0, t * h), (w, t * h)] {
                    let (lon, lat) = self.pixel_to_lonlat(x, y);
                    pts.push((self.unwrap_lon(lon), lat));
                }
            }
        }
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for (lon, lat) in pts {
            b = [b[0].min(lon), b[1].min(lat), b[2].max(lon), b[3].max(lat)];
        }
        b
    }

    fn compute_res(&self) -> f64 {
        let (cx, cy) = (self.width as f64 / 2.0, self.height as f64 / 2.0);
        let p = self.pixel_to_lonlat(cx, cy);
        let dx = distance_m(p, self.pixel_to_lonlat(cx + 10.0, cy)) / 10.0;
        let dy = distance_m(p, self.pixel_to_lonlat(cx, cy + 10.0)) / 10.0;
        (dx * dy).sqrt()
    }

    /// One row's palette indices.
    fn decode_row(&self, row: usize) -> Vec<u8> {
        let mut out = vec![0u8; self.width];
        let Some(&start) = self.rows.get(row) else { return out };
        let d = &self.data;
        let mut p = start;
        // the row number (7 bits a byte, high bit: more)
        while p < d.len() && d[p] & 0x80 != 0 {
            p += 1;
        }
        p += 1;
        let shift = 7 - self.depth as u32;
        let vmask: u8 = (((1u16 << self.depth) - 1) << shift) as u8;
        let cmask: u8 = ((1u16 << shift) - 1) as u8;
        let mut x = 0usize;
        while p < d.len() && x < self.width {
            let b = d[p];
            p += 1;
            if b == 0 {
                break;
            }
            let v = (b & vmask) >> shift;
            let mut run = (b & cmask) as usize;
            let mut c = b;
            while c & 0x80 != 0 && p < d.len() {
                c = d[p];
                p += 1;
                run = run * 128 + (c & 0x7f) as usize;
            }
            let n = (run + 1).min(self.width - x);
            out[x..x + n].fill(v);
            x += n;
        }
        out
    }

    /// The palette (index 0 is transparent).
    pub fn palette(&self) -> &[[u8; 4]] {
        &self.palette
    }

    /// The whole image as palette indices (for tests and conversion).
    pub fn indices(&self) -> Vec<u8> {
        (0..self.height).flat_map(|r| self.decode_row(r)).collect()
    }

    /// Whether the chart's border (PLY) was given.
    pub fn has_border(&self) -> bool {
        !self.border.is_empty()
    }
}

fn wrap(lon: f64) -> f64 {
    if lon > 180.0 {
        lon - 360.0
    } else if lon < -180.0 {
        lon + 360.0
    } else {
        lon
    }
}

/// The row index at the end of the file: its offset in the last 4 bytes (big-endian), then
/// one 4-byte offset a row.
fn row_index(d: &[u8], rows_start: usize, height: usize) -> Option<Vec<usize>> {
    if d.len() < 4 + height * 4 {
        return None;
    }
    let be = |p: usize| u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]) as usize;
    let at = be(d.len() - 4);
    if at < rows_start || at + height * 4 > d.len() - 4 {
        return None;
    }
    let rows: Vec<usize> = (0..height).map(|i| be(at + i * 4)).collect();
    let ok = rows.windows(2).all(|w| w[0] < w[1]) && rows.first().is_some_and(|&r| r >= rows_start) && rows.last().is_some_and(|&r| r < at);
    ok.then_some(rows)
}

/// No usable index: walk the rows (each ends with a 0 byte; row numbers and runs never contain one).
fn scan_rows(d: &[u8], rows_start: usize, height: usize) -> Option<Vec<usize>> {
    let mut rows = Vec::with_capacity(height);
    let mut p = rows_start;
    while rows.len() < height && p < d.len() {
        rows.push(p);
        while p < d.len() && d[p] & 0x80 != 0 {
            p += 1;
        }
        p += 1;
        while p < d.len() && d[p] != 0 {
            p += 1;
        }
        p += 1;
    }
    (rows.len() == height).then_some(rows)
}

fn point_in(poly: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl Raster for Kap {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    fn levels(&self) -> usize {
        1
    }
    fn level_size(&self, _level: usize) -> (usize, usize) {
        (self.width, self.height)
    }
    fn block_size(&self, _level: usize) -> (usize, usize) {
        (self.width, 1)
    }
    fn block(&self, _level: usize, _bx: usize, by: usize) -> Option<Arc<Block>> {
        if by >= self.height {
            return None;
        }
        if let Some(b) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&by) {
            return Some(b.clone());
        }
        let b = Arc::new(Block { w: self.width, h: 1, data: self.decode_row(by), palette: Some(self.palette.clone()) });
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).put(by, b.clone());
        Some(b)
    }
    fn to_pixel(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        // WGS84 to the chart's datum, then the fit
        self.chart_to_pixel(lon - self.dtm.1 / 3600.0, lat - self.dtm.0 / 3600.0)
    }
    fn inside(&self, col: f64, row: f64) -> bool {
        self.border.is_empty() || point_in(&self.border, col, row)
    }
    fn bounds(&self) -> [f64; 4] {
        self.bounds
    }
    fn resolution_m(&self) -> f64 {
        self.res_m
    }
}
