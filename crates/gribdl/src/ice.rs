//! Sea ice concentration from the University of Bremen: the ASI algorithm on the
//! AMSR2 microwave radiometer (JAXA's GCOM-W "Shizuku"), daily, on a 3.125 km polar
//! stereographic grid. Microwaves see through cloud and polar night.
//!
//! One GeoTIFF a day for each hemisphere (about 1 MB), 8-bit: 0–100 = per cent of the
//! cell covered by ice, 120 = land. The day's file is on the server early the next day
//! (UTC), with a first version the same afternoon.
//!
//! `https://data.seaice.uni-bremen.de/amsr2/asi_daygrid_swath/s3125/2026/nov/Antarctic3125/asi-AMSR2-s3125-20261118-v5.4.tif`
//! (north: `n3125/…/Arctic3125/asi-AMSR2-n3125-….tif`)
//!
//! The grids are EPSG:3976 (south) and EPSG:3413 (north). Each day is resampled
//! (nearest cell, so the ice edge stays sharp) onto a regular latitude/longitude grid
//! over the area asked for, and written as GRIB2 "ice cover" (10/2/0, a proportion
//! 0–1) so it loads, shades, reads out and steps through time like the other ocean data.
//!
//! Citation asked for by Bremen: Spreen, G., L. Kaleschke and G. Heygster (2008),
//! Sea ice remote sensing using AMSR-E 89 GHz channels, J. Geophys. Res. 113, C02S03.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use grib::{Field, Grid, Param};

use crate::{client, get_retry, Error, Progress, Request, Result};

pub const BREMEN: &str = "https://data.seaice.uni-bremen.de/amsr2/asi_daygrid_swath";

/// The grid spacing written (degrees of latitude): about the data's 3.125 km.
const STEP_LAT: f64 = 0.025;
/// Most points along a side of the grid written.
const MAX_SIDE: usize = 1600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pole {
    South,
    North,
}

/// The hemisphere whose grid covers an area. Each grid reaches about 50° from its pole
/// (40° in its corners): an area must be wholly in one of them.
pub fn pole_for(area: [f64; 4]) -> Result<Pole> {
    let (s, n) = (area[1].min(area[3]), area[1].max(area[3]));
    if n <= -45.0 {
        Ok(Pole::South)
    } else if s >= 45.0 {
        Ok(Pole::North)
    } else {
        Err(Error::Bad("sea ice data covers the polar seas: choose an area wholly south of 45°S (or north of 45°N)".into()))
    }
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// The day's file. `day` is unix seconds of any time that day (UTC).
pub fn url(pole: Pole, day: i64) -> String {
    let d = chrono::DateTime::from_timestamp(day, 0).unwrap_or_default();
    let (y, m, ymd) = (d.format("%Y"), MONTHS[d.format("%m").to_string().parse::<usize>().unwrap_or(1).clamp(1, 12) - 1], d.format("%Y%m%d"));
    match pole {
        Pole::South => format!("{BREMEN}/s3125/{y}/{m}/Antarctic3125/asi-AMSR2-s3125-{ymd}-v5.4.tif"),
        Pole::North => format!("{BREMEN}/n3125/{y}/{m}/Arctic3125/asi-AMSR2-n3125-{ymd}-v5.4.tif"),
    }
}

/// Polar stereographic on WGS84 (EPSG "variant B": a latitude of true scale).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolarStereo {
    pub south: bool,
    /// Latitude of true scale, degrees (negative in the south).
    pub lat_ts: f64,
    /// Longitude straight down from the pole (towards +y in the north, -y in the south), degrees.
    pub lon0: f64,
}

impl PolarStereo {
    pub fn from_epsg(code: u32) -> Option<PolarStereo> {
        Some(match code {
            3976 => PolarStereo { south: true, lat_ts: -70.0, lon0: 0.0 },   // NSIDC sea ice south
            3412 => PolarStereo { south: true, lat_ts: -70.0, lon0: 0.0 },   // the same on Hughes 1980 (metres apart)
            3031 => PolarStereo { south: true, lat_ts: -71.0, lon0: 0.0 },   // Antarctic polar stereographic
            9354 => PolarStereo { south: true, lat_ts: -65.0, lon0: 0.0 },   // IBCSO
            3413 => PolarStereo { south: false, lat_ts: 70.0, lon0: -45.0 }, // NSIDC sea ice north
            3995 => PolarStereo { south: false, lat_ts: 71.0, lon0: 0.0 },   // Arctic polar stereographic
            3996 => PolarStereo { south: false, lat_ts: 75.0, lon0: 0.0 },   // IBCAO
            _ => return None,
        })
    }

    /// Metres east and north of the pole for a position (Snyder, Map Projections, 21-33 to 21-40).
    pub fn forward(&self, lon: f64, lat: f64) -> (f64, f64) {
        const A: f64 = 6_378_137.0;
        const F: f64 = 1.0 / 298.257_223_563;
        let e = (F * (2.0 - F)).sqrt();
        // the south pole is the north pole with latitude and longitude turned round
        let s = if self.south { -1.0 } else { 1.0 };
        let (phi, lam, lam0, phic) = ((s * lat).to_radians(), (s * lon).to_radians(), (s * self.lon0).to_radians(), (s * self.lat_ts).to_radians());
        let t = |p: f64| (std::f64::consts::FRAC_PI_4 - p / 2.0).tan() / ((1.0 - e * p.sin()) / (1.0 + e * p.sin())).powf(e / 2.0);
        let m = |p: f64| p.cos() / (1.0 - e * e * p.sin().powi(2)).sqrt();
        let rho = A * m(phic) * t(phi) / t(phic);
        let (x, y) = (rho * (lam - lam0).sin(), -rho * (lam - lam0).cos());
        (s * x, s * y)
    }
}

/// A one-band 8-bit GeoTIFF read whole: what Bremen's files are.
#[derive(Debug)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    /// The outer corner of the top-left pixel (metres) and the pixel size.
    pub x0: f64,
    pub y0: f64,
    pub px: f64,
    pub py: f64,
    pub epsg: Option<u32>,
    pub data: Vec<u8>,
}

impl Raster {
    /// The value of the cell a projected point falls in.
    pub fn at(&self, x: f64, y: f64) -> Option<u8> {
        let c = ((x - self.x0) / self.px).floor();
        let r = ((self.y0 - y) / self.py).floor();
        if c < 0.0 || r < 0.0 || c >= self.width as f64 || r >= self.height as f64 {
            return None;
        }
        Some(self.data[r as usize * self.width + c as usize])
    }
}

/// Read a stripped, 8-bit, single-band GeoTIFF (no compression or deflate, no predictor).
pub fn read_tiff(b: &[u8]) -> Result<Raster> {
    let bad = |s: &str| Error::Bad(format!("sea ice file: {s}"));
    if b.len() < 8 {
        return Err(bad("too short"));
    }
    let le = match &b[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(bad("not a TIFF")),
    };
    let u16_at = |o: usize| -> Result<u16> {
        let s: [u8; 2] = b.get(o..o + 2).ok_or_else(|| bad("cut short"))?.try_into().unwrap();
        Ok(if le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
    };
    let u32_at = |o: usize| -> Result<u32> {
        let s: [u8; 4] = b.get(o..o + 4).ok_or_else(|| bad("cut short"))?.try_into().unwrap();
        Ok(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    };
    let f64_at = |o: usize| -> Result<f64> {
        let s: [u8; 8] = b.get(o..o + 8).ok_or_else(|| bad("cut short"))?.try_into().unwrap();
        Ok(if le { f64::from_le_bytes(s) } else { f64::from_be_bytes(s) })
    };
    if u16_at(2)? != 42 {
        return Err(bad("not a classic TIFF"));
    }
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    // tag -> numbers (integers and doubles, all as f64)
    let mut tags: std::collections::HashMap<u16, Vec<f64>> = std::collections::HashMap::new();
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        let (tag, typ, cnt) = (u16_at(e)?, u16_at(e + 2)?, u32_at(e + 4)? as usize);
        let size = match typ {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            12 | 16 => 8,
            _ => continue,
        };
        if typ == 2 || cnt > 1_000_000 {
            continue; // text: not needed
        }
        let at = if size * cnt <= 4 { e + 8 } else { u32_at(e + 8)? as usize };
        let mut v = Vec::with_capacity(cnt);
        for k in 0..cnt {
            v.push(match typ {
                1 | 7 => *b.get(at + k).ok_or_else(|| bad("cut short"))? as f64,
                3 => u16_at(at + 2 * k)? as f64,
                4 => u32_at(at + 4 * k)? as f64,
                12 => f64_at(at + 8 * k)?,
                _ => f64::NAN,
            });
        }
        tags.insert(tag, v);
    }
    let one = |t: u16| tags.get(&t).and_then(|v| v.first().copied());
    let (w, h) = (one(256).ok_or_else(|| bad("no width"))? as usize, one(257).ok_or_else(|| bad("no height"))? as usize);
    if one(258).unwrap_or(1.0) != 8.0 || one(277).unwrap_or(1.0) != 1.0 {
        return Err(bad("not one 8-bit band"));
    }
    if tags.contains_key(&322) {
        return Err(bad("tiled TIFFs are not read"));
    }
    if one(317).unwrap_or(1.0) != 1.0 {
        return Err(bad("predictor not supported"));
    }
    let compression = one(259).unwrap_or(1.0) as u32;
    let offsets = tags.get(&273).ok_or_else(|| bad("no strips"))?;
    let counts = tags.get(&279).ok_or_else(|| bad("no strip sizes"))?;
    let mut data = Vec::with_capacity(w * h);
    for (&o, &c) in offsets.iter().zip(counts) {
        let raw = b.get(o as usize..(o + c) as usize).ok_or_else(|| bad("strip outside the file"))?;
        match compression {
            1 => data.extend_from_slice(raw),
            8 | 32946 => {
                flate2::read::ZlibDecoder::new(raw).read_to_end(&mut data).map_err(|e| bad(&format!("deflate: {e}")))?;
            }
            c => return Err(bad(&format!("compression {c} not supported"))),
        }
    }
    if data.len() < w * h {
        return Err(bad("fewer pixels than the size says"));
    }
    data.truncate(w * h);
    let scale = tags.get(&33550).ok_or_else(|| bad("no pixel size"))?;
    let tie = tags.get(&33922).ok_or_else(|| bad("no tie point"))?;
    if scale.len() < 2 || tie.len() < 6 {
        return Err(bad("pixel size or tie point incomplete"));
    }
    let (px, py) = (scale[0], scale[1]);
    let (x0, y0) = (tie[3] - tie[0] * px, tie[4] + tie[1] * py);
    // GeoKeys: ProjectedCSTypeGeoKey (3072)
    let epsg = tags.get(&34735).and_then(|k| {
        let n = *k.get(3)? as usize;
        (0..n).find_map(|i| {
            let e = 4 + i * 4;
            if *k.get(e)? as u16 == 3072 && *k.get(e + 1)? == 0.0 {
                Some(*k.get(e + 3)? as u32)
            } else {
                None
            }
        })
    });
    Ok(Raster { width: w, height: h, x0, y0, px, py, epsg, data })
}

/// The day's ice on a regular latitude/longitude grid over `area` [w, s, e, n] (east
/// may pass 180): proportion 0–1 of each cell, land and cells off the grid missing.
pub fn resample(r: &Raster, proj: &PolarStereo, area: [f64; 4], valid_time: i64) -> Result<Field> {
    let [w, s, e, n] = area;
    let e = if e <= w { e + 360.0 } else { e };
    let mid = ((s + n) / 2.0).to_radians().cos().max(0.05);
    let mut dlat = STEP_LAT;
    let mut dlon = STEP_LAT / mid;
    // thin a big area
    let k = (((n - s) / dlat).max((e - w) / dlon) / MAX_SIDE as f64).max(1.0);
    dlat *= k;
    dlon *= k;
    let round = |v: f64| (v * 1e4).round() / 1e4;
    let (dlat, dlon) = (round(dlat), round(dlon));
    let nj = ((n - s) / dlat).floor() as usize + 1;
    let ni = ((e - w) / dlon).floor() as usize + 1;
    if ni < 2 || nj < 2 {
        return Err(Error::Bad("the area is too small for sea ice data".into()));
    }
    let mut values = vec![f32::NAN; ni * nj];
    let mut any = false;
    for j in 0..nj {
        let lat = s + j as f64 * dlat;
        for i in 0..ni {
            let (x, y) = proj.forward(w + i as f64 * dlon, lat);
            if let Some(v) = r.at(x, y) {
                // 0–100 %: 120 is land, anything else unknown
                if v <= 100 {
                    values[j * ni + i] = v as f32 / 100.0;
                    any = true;
                }
            }
        }
    }
    if !any {
        return Err(Error::Bad("no sea ice data over the area (all land, or off the polar grid)".into()));
    }
    let grid = Grid::regular(ni, nj, s, nav_core::wrap_lon(w), dlat, dlon);
    Ok(Field {
        param: Param::SeaIce,
        discipline: 10,
        category: 2,
        number: 0,
        surface_type: 1,
        surface_value: 0.0,
        centre: grib::write::CENTRE_LOCAL,
        ref_time: valid_time,
        valid_time,
        member: None,
        grid,
        values,
        local: None,
    })
}

/// Download the latest `days` daily files over the area (missing days skipped); returns the file written.
pub fn download(req: &Request, now: i64, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let pole = pole_for(req.area)?;
    let days = (req.hours / 24).clamp(1, 14) as usize;
    let c = client()?;
    let mut fields = Vec::new();
    let mut bytes = 0u64;
    let today = now - now.rem_euclid(86_400);
    progress(0, days, 0);
    // today's file is often not there yet, and the odd day is missing: look a week further back
    for k in 0..(days as i64 + 7) {
        if fields.len() >= days {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let day = today - k * 86_400;
        let data = match get_retry(&c, &url(pole, day), cancel) {
            Ok(d) => d,
            Err(Error::Http(m)) if m.starts_with("not on the server yet") => continue,
            Err(e) => return Err(e),
        };
        bytes += data.len() as u64;
        let r = read_tiff(&data)?;
        let code = r.epsg.unwrap_or(if pole == Pole::South { 3976 } else { 3413 });
        let proj = PolarStereo::from_epsg(code).ok_or_else(|| Error::Bad(format!("sea ice file: projection EPSG:{code} not known")))?;
        // a daily grid of all the day's passes: its time is the middle of the day
        fields.push(resample(&r, &proj, req.area, day + 12 * 3600)?);
        progress(fields.len(), days, bytes);
    }
    if fields.is_empty() {
        return Err(Error::Bad("no sea ice files on the University of Bremen's server for the last week".into()));
    }
    fields.sort_by_key(|f| f.valid_time);
    let r = fields[0].valid_time;
    for f in fields.iter_mut() {
        f.ref_time = r;
    }
    let ymd = |t: i64| chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().format("%Y%m%d").to_string();
    let path = dir.join(crate::ocean::name("SEAICE_AMSR2", &format!("{}-{}", ymd(r), ymd(fields[fields.len() - 1].valid_time)), req.area));
    crate::ocean::write(&fields, grib::write::PROCESS_ASI_ICE, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projections_match_proj() {
        // reference values from PROJ (pyproj 3.7)
        let s = PolarStereo::from_epsg(3976).unwrap();
        for (lon, lat, x, y) in [
            (170.0, -77.85, 229377.439, -1300864.1),
            (-60.6, -62.98, -2596472.18, 1463036.884),
            (-45.0, -60.0, -2349829.162, 2349829.162),
            (120.0, -50.0, 3906639.648, -2255499.453),
        ] {
            let (a, b) = s.forward(lon, lat);
            assert!((a - x).abs() < 0.01 && (b - y).abs() < 0.01, "3976 {lon} {lat}: {a} {b}");
        }
        let n = PolarStereo::from_epsg(3413).unwrap();
        for (lon, lat, x, y) in [(-45.0, 70.0, 0.0, -2187927.649), (10.0, 78.0, 1068590.763, -748235.307), (-150.0, 72.0, -1898469.898, 508693.476)] {
            let (a, b) = n.forward(lon, lat);
            assert!((a - x).abs() < 0.01 && (b - y).abs() < 0.01, "3413 {lon} {lat}: {a} {b}");
        }
    }

    /// A small file laid out like Bremen's: little-endian, deflated strips, GeoKeys with EPSG:3976.
    fn sample_tiff(w: usize, h: usize, px: f64, x0: f64, y0: f64, pix: &[u8], deflate: bool) -> Vec<u8> {
        use std::io::Write;
        let rps = 3usize;
        let mut strips: Vec<Vec<u8>> = Vec::new();
        for r in (0..h).step_by(rps) {
            let rows = &pix[r * w..((r + rps).min(h)) * w];
            strips.push(if deflate {
                let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                z.write_all(rows).unwrap();
                z.finish().unwrap()
            } else {
                rows.to_vec()
            });
        }
        let geokeys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 3976];
        // layout: header, data, arrays, IFD
        let mut b = b"II*\0\0\0\0\0".to_vec();
        let mut offs = Vec::new();
        for s in &strips {
            offs.push(b.len() as u32);
            b.extend_from_slice(s);
        }
        let arr_u32 = |b: &mut Vec<u8>, v: &[u32]| {
            let o = b.len() as u32;
            for x in v {
                b.extend_from_slice(&x.to_le_bytes());
            }
            o
        };
        let o_offs = arr_u32(&mut b, &offs);
        let o_cnts = arr_u32(&mut b, &strips.iter().map(|s| s.len() as u32).collect::<Vec<_>>());
        let o_scale = b.len() as u32;
        for v in [px, px, 0.0] {
            b.extend_from_slice(&f64::to_le_bytes(v));
        }
        let o_tie = b.len() as u32;
        for v in [0.0, 0.0, 0.0, x0, y0, 0.0] {
            b.extend_from_slice(&f64::to_le_bytes(v));
        }
        let o_keys = b.len() as u32;
        for v in geokeys {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let ifd = b.len() as u32;
        b[4..8].copy_from_slice(&ifd.to_le_bytes());
        let ns = strips.len() as u32;
        let ents: Vec<(u16, u16, u32, u32)> = vec![
            (256, 4, 1, w as u32),
            (257, 4, 1, h as u32),
            (258, 3, 1, 8),
            (259, 3, 1, if deflate { 8 } else { 1 }),
            (262, 3, 1, 1),
            (273, 4, ns, if ns == 1 { offs[0] } else { o_offs }),
            (277, 3, 1, 1),
            (278, 4, 1, rps as u32),
            (279, 4, ns, if ns == 1 { strips[0].len() as u32 } else { o_cnts }),
            (33550, 12, 3, o_scale),
            (33922, 12, 6, o_tie),
            (34735, 3, 16, o_keys),
        ];
        b.extend_from_slice(&(ents.len() as u16).to_le_bytes());
        for (t, ty, c, v) in ents {
            b.extend_from_slice(&t.to_le_bytes());
            b.extend_from_slice(&ty.to_le_bytes());
            b.extend_from_slice(&c.to_le_bytes());
            if ty == 3 && c == 1 {
                b.extend_from_slice(&(v as u16).to_le_bytes());
                b.extend_from_slice(&[0, 0]);
            } else {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b.extend_from_slice(&[0; 4]);
        b
    }

    #[test]
    fn reads_bremen_layout_and_resamples() {
        // a 40 × 40 piece of the southern grid, 25 km cells, round the Ross Sea:
        // ice 80 % in the south half, open water north, land in one corner
        let (w, h, px) = (40usize, 40usize, 25_000.0);
        let (x0, y0) = (-250_000.0, -800_000.0);
        let mut pix = vec![0u8; w * h];
        for r in 0..h {
            for c in 0..w {
                pix[r * w + c] = if r < 5 && c < 5 {
                    120
                } else if r >= 20 {
                    80
                } else {
                    0
                };
            }
        }
        for deflate in [true, false] {
            let r = read_tiff(&sample_tiff(w, h, px, x0, y0, &pix, deflate)).unwrap();
            assert_eq!((r.width, r.height, r.epsg), (40, 40, Some(3976)));
            assert_eq!((r.x0, r.y0, r.px), (x0, y0, px));
            assert_eq!(r.data, pix);
            // cell (row 30, column 10): ice
            assert_eq!(r.at(x0 + 10.5 * px, y0 - 30.5 * px), Some(80));
            assert_eq!(r.at(x0 - 1.0, y0), None);
        }
        let r = read_tiff(&sample_tiff(w, h, px, x0, y0, &pix, true)).unwrap();
        let p = PolarStereo::from_epsg(3976).unwrap();
        // the piece spans about 81–90°S near 180°; ask across the date line
        let f = resample(&r, &p, [170.0, -85.0, 190.0, -80.0], 1_800_000_000).unwrap();
        assert_eq!(f.param, Param::SeaIce);
        assert!(f.grid.ni >= 2 && f.grid.nj >= 2);
        // every value is missing, 0, or 0.8
        assert!(f.values.iter().all(|v| v.is_nan() || *v == 0.0 || (*v - 0.8).abs() < 1e-6));
        // check the sample against the source at a point
        let (lat, lon) = (-84.0, 179.0);
        let (x, y) = p.forward(lon, lat);
        let want = r.at(x, y).map(|v| v as f64 / 100.0);
        let got = f.grid.sample(&f.values, lat, lon);
        if let (Some(a), Some(b)) = (want, got) {
            assert!((a - b).abs() < 0.41, "{a} vs {b}");
        }
    }

    #[test]
    fn writes_a_grib_the_app_reads() {
        let (w, h, px) = (40usize, 40usize, 25_000.0);
        let pix: Vec<u8> = (0..w * h).map(|i| if i % 7 == 0 { 120 } else { (i % 101) as u8 }).collect();
        let r = read_tiff(&sample_tiff(w, h, px, -250_000.0, -800_000.0, &pix, true)).unwrap();
        let p = PolarStereo::from_epsg(3976).unwrap();
        let day = 1_795_003_200;
        let fields = vec![resample(&r, &p, [170.0, -85.0, 190.0, -80.0], day - 86_400).unwrap(), resample(&r, &p, [170.0, -85.0, 190.0, -80.0], day).unwrap()];
        let path = std::env::temp_dir().join(format!("seaice_test_{}.grb2", std::process::id()));
        crate::ocean::write(&fields, grib::write::PROCESS_ASI_ICE, &path).unwrap();
        let g = grib::GribFile::open(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(g.model().0, "AMSR2 sea ice (Bremen)");
        assert!(g.fields.iter().all(|f| f.param == Param::SeaIce));
        assert_eq!(g.fields.len(), 2);
        // values survive packing to two decimals
        for (a, b) in g.fields[1].values.iter().zip(&fields[1].values) {
            assert!(a.is_nan() == b.is_nan() && (a.is_nan() || (a - b).abs() < 0.006), "{a} {b}");
        }
    }

    #[test]
    fn urls_and_poles() {
        // 18 Nov 2026 12:00 UTC
        let t = 1_795_003_200;
        assert_eq!(url(Pole::South, t), format!("{BREMEN}/s3125/2026/nov/Antarctic3125/asi-AMSR2-s3125-20261118-v5.4.tif"));
        assert_eq!(url(Pole::North, t), format!("{BREMEN}/n3125/2026/nov/Arctic3125/asi-AMSR2-n3125-20261118-v5.4.tif"));
        assert_eq!(pole_for([160.0, -78.0, 200.0, -66.0]).unwrap(), Pole::South);
        assert_eq!(pole_for([-30.0, 70.0, 30.0, 82.0]).unwrap(), Pole::North);
        assert!(pole_for([170.0, -48.0, 180.0, -40.0]).is_err());
    }
}
