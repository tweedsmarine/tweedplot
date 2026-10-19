//! Chart data downloads that aren't weather: the GSHHG world coastline and
//! ETOPO 2022 bathymetry for an area.
//!
//! - **GSHHG** (Wessel & Smith, LGPL 3): the shapefile zip from its home at the
//!   University of Hawaiʻi, or NOAA NCEI's copy. The chart crate builds its
//!   coastline file from the zip.
//! - **ETOPO 2022** (NOAA NCEI; "may be used and redistributed for free"): the
//!   15 arc-second grid through NOAA CoastWatch's ERDDAP server, which cuts any
//!   area and thins it (stride 2 = 30″, 4 = 60″). Fetched as DAP2 binary
//!   (`.dods`): a short text header, then big-endian arrays.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{client, Error, Result};

pub const GSHHG_FILE: &str = "gshhg-shp-2.3.7.zip";
const GSHHG_URLS: [&str; 2] = [
    "https://www.soest.hawaii.edu/pwessel/gshhg/gshhg-shp-2.3.7.zip",
    "https://www.ngdc.noaa.gov/mgg/shorelines/data/gshhg/latest/gshhg-shp-2.3.7.zip",
];
/// Rough size, for the dialog.
pub const GSHHG_BYTES: u64 = 150_000_000;

/// Base of the ERDDAP griddap dataset (override with `RACENAV_ETOPO_URL` for tests).
fn etopo_base() -> String {
    std::env::var("RACENAV_ETOPO_URL").unwrap_or_else(|_| "https://coastwatch.pfeg.noaa.gov/erddap/griddap/ETOPO_2022_v1_15s".into())
}

/// Most grid points fetched for one area (about 24 MB at 4 bytes a point).
pub const MAX_POINTS: u64 = 6_000_000;

/// Download a large file with progress (`done`, `total` bytes; total 0 if unknown).
fn fetch_to(url: &str, out: &Path, progress: &(dyn Fn(u64, u64) + Sync), cancel: &AtomicBool) -> Result<()> {
    let c = reqwest::blocking::Client::builder()
        .user_agent(concat!("TweedPlot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(None::<std::time::Duration>)
        .build()
        .map_err(|e| Error::Http(e.to_string()))?;
    let mut r = c.get(url).send().map_err(|e| Error::Http(e.to_string()))?;
    if !r.status().is_success() {
        return Err(Error::Http(format!("HTTP {} from {url}", r.status().as_u16())));
    }
    let total = r.content_length().unwrap_or(0);
    let part = out.with_extension("part");
    let mut f = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(f);
            let _ = std::fs::remove_file(&part);
            return Err(Error::Cancelled);
        }
        let n = r.read(&mut buf).map_err(|e| Error::Http(e.to_string()))?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    f.flush()?;
    drop(f);
    std::fs::rename(&part, out)?;
    Ok(())
}

/// Download the GSHHG shapefile zip into `dir` (tries each source in turn).
pub fn gshhg(dir: &Path, progress: &(dyn Fn(u64, u64) + Sync), cancel: &AtomicBool) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let out = dir.join(GSHHG_FILE);
    let mut last = String::new();
    for url in GSHHG_URLS {
        match fetch_to(url, &out, progress, cancel) {
            Ok(()) => {
                // a zip starts "PK"
                let mut head = [0u8; 2];
                std::fs::File::open(&out)?.read_exact(&mut head)?;
                if &head == b"PK" {
                    return Ok(out);
                }
                let _ = std::fs::remove_file(&out);
                last = format!("{url} did not send a zip file");
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => last = e.to_string(),
        }
    }
    Err(Error::Http(last))
}

/// A height grid (metres, positive up; NaN = missing), rows south to north.
#[derive(Debug, Clone)]
pub struct HeightGrid {
    pub west: f64,
    pub south: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub nx: usize,
    pub ny: usize,
    pub z: Vec<f32>,
}

/// ETOPO points (and the stride) for an area at `arcsec` spacing (15, 30 or 60),
/// thinned further if needed to stay under [`MAX_POINTS`].
pub fn etopo_plan(area: [f64; 4], arcsec: u32) -> (u64, u32) {
    let mut stride = (arcsec / 15).max(1);
    loop {
        let step = 15.0 * stride as f64 / 3600.0;
        let nx = ((area[2] - area[0]) / step).ceil() as u64 + 1;
        let ny = ((area[3] - area[1]) / step).ceil() as u64 + 1;
        if nx * ny <= MAX_POINTS || stride >= 64 {
            return (nx * ny, stride);
        }
        stride *= 2;
    }
}

fn etopo_url(w: f64, s: f64, e: f64, n: f64, stride: u32) -> String {
    format!("{}.dods?z%5B({s:.5}):{stride}:({n:.5})%5D%5B({w:.5}):{stride}:({e:.5})%5D", etopo_base())
}

/// Download ETOPO 2022 for `area` [w, s, e, n] (east may pass 180) at `arcsec`.
pub fn etopo(area: [f64; 4], arcsec: u32, progress: &(dyn Fn(u64, u64) + Sync), cancel: &AtomicBool) -> Result<HeightGrid> {
    let [w, s, e, n] = area;
    if !(n > s) || !(e > w) || s < -90.0 || n > 90.0 {
        return Err(Error::Bad("the area is not valid".into()));
    }
    let (_, stride) = etopo_plan(area, arcsec);
    // the dataset runs -180..180: an area across the date line is two requests
    let w = (w + 180.0).rem_euclid(360.0) - 180.0;
    let e = w + (area[2] - area[0]);
    let parts: Vec<(f64, f64)> = if e > 180.0 { vec![(w, 179.999), (-180.0, e - 360.0)] } else { vec![(w, e)] };
    let c = client()?;
    let mut grids = Vec::new();
    for (pw, pe) in parts {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let url = etopo_url(pw, s, pe, n, stride);
        let r = c.get(&url).send().map_err(|e| Error::Http(e.to_string()))?;
        if !r.status().is_success() {
            let code = r.status().as_u16();
            let body = r.text().unwrap_or_default();
            return Err(Error::Http(format!("ETOPO server answered HTTP {code}: {}", body.chars().take(200).collect::<String>())));
        }
        let bytes = r.bytes().map_err(|e| Error::Http(e.to_string()))?;
        progress(bytes.len() as u64, 0);
        grids.push(parse_dods(&bytes)?);
    }
    let mut g = grids.remove(0);
    if let Some(east) = grids.pop() {
        g = join_east(g, east)?;
    }
    Ok(g)
}

/// Put a grid that starts at -180 to the east of one that ends at 180.
fn join_east(a: HeightGrid, b: HeightGrid) -> Result<HeightGrid> {
    if a.ny != b.ny {
        return Err(Error::Bad("the two halves across the date line differ in size".into()));
    }
    let nx = a.nx + b.nx;
    let mut z = Vec::with_capacity(nx * a.ny);
    for j in 0..a.ny {
        z.extend_from_slice(&a.z[j * a.nx..(j + 1) * a.nx]);
        z.extend_from_slice(&b.z[j * b.nx..(j + 1) * b.nx]);
    }
    Ok(HeightGrid { nx, z, ..a })
}

/// Parse an ERDDAP DAP2 binary response for the grid `z[latitude][longitude]`.
pub fn parse_dods(b: &[u8]) -> Result<HeightGrid> {
    let marker = b"\nData:\n";
    let at = b.windows(marker.len()).position(|w| w == marker).ok_or_else(|| Error::Bad("not a DAP response".into()))?;
    let dds = String::from_utf8_lossy(&b[..at]);
    let ty = ["Float32", "Float64", "Int16", "Int32", "UInt16"]
        .into_iter()
        .find(|t| dds.contains(&format!("{t} z[")))
        .ok_or_else(|| Error::Bad(format!("unexpected DAP header: {}", dds.chars().take(200).collect::<String>())))?;
    let mut p = at + marker.len();
    let u32be = |b: &[u8], p: usize| -> Result<u32> {
        b.get(p..p + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]])).ok_or_else(|| Error::Bad("DAP data cut short".into()))
    };
    // z: count twice, then values (XDR: 16-bit types take 4 bytes)
    let n = u32be(b, p)? as usize;
    p += 8;
    let width = if ty == "Float64" { 8 } else { 4 };
    if b.len() < p + n * width {
        return Err(Error::Bad("DAP data cut short".into()));
    }
    let mut z = Vec::with_capacity(n);
    for k in 0..n {
        let o = p + k * width;
        let v = match ty {
            "Float32" => f32::from_be_bytes(b[o..o + 4].try_into().unwrap()),
            "Float64" => f64::from_be_bytes(b[o..o + 8].try_into().unwrap()) as f32,
            "Int16" | "Int32" => i32::from_be_bytes(b[o..o + 4].try_into().unwrap()) as f32,
            _ => u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) as f32,
        };
        z.push(if v <= -99_000.0 || !v.is_finite() { f32::NAN } else { v });
    }
    p += n * width;
    let map = |p: &mut usize| -> Result<Vec<f64>> {
        let m = u32be(b, *p)? as usize;
        *p += 8;
        let v: Option<Vec<f64>> = (0..m).map(|k| b.get(*p + k * 8..*p + k * 8 + 8).map(|x| f64::from_be_bytes(x.try_into().unwrap()))).collect();
        *p += m * 8;
        v.ok_or_else(|| Error::Bad("DAP data cut short".into()))
    };
    let lat = map(&mut p)?;
    let lon = map(&mut p)?;
    let (ny, nx) = (lat.len(), lon.len());
    if ny * nx != n || ny < 2 || nx < 2 {
        return Err(Error::Bad(format!("DAP grid {ny}×{nx} does not match {n} values")));
    }
    // rows south to north
    if lat[0] > lat[ny - 1] {
        let mut flipped = Vec::with_capacity(n);
        for j in (0..ny).rev() {
            flipped.extend_from_slice(&z[j * nx..(j + 1) * nx]);
        }
        z = flipped;
    }
    let (south, dlat) = (lat[0].min(lat[ny - 1]), (lat[ny - 1] - lat[0]).abs() / (ny - 1) as f64);
    let dlon = (lon[nx - 1] - lon[0]) / (nx - 1) as f64;
    Ok(HeightGrid { west: lon[0], south, dlon, dlat, nx, ny, z })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DAP2 response as ERDDAP writes it, for a 3 × 4 grid (latitudes descending).
    fn dods() -> Vec<u8> {
        let mut v = b"Dataset {\n  GRID {\n    ARRAY:\n      Float32 z[latitude = 3][longitude = 4];\n    MAPS:\n      Float64 latitude[latitude = 3];\n      Float64 longitude[longitude = 4];\n  } z;\n} ETOPO_2022_v1_15s;\n\nData:\n".to_vec();
        let push_n = |v: &mut Vec<u8>, n: u32| {
            v.extend_from_slice(&n.to_be_bytes());
            v.extend_from_slice(&n.to_be_bytes());
        };
        push_n(&mut v, 12);
        for j in 0..3 {
            for i in 0..4 {
                let z = if j == 0 && i == 0 { -99999.0 } else { -(10.0 * j as f32 + i as f32) };
                v.extend_from_slice(&z.to_be_bytes());
            }
        }
        push_n(&mut v, 3);
        for la in [-41.0f64, -41.25, -41.5] {
            v.extend_from_slice(&la.to_be_bytes());
        }
        push_n(&mut v, 4);
        for lo in [173.0f64, 173.25, 173.5, 173.75] {
            v.extend_from_slice(&lo.to_be_bytes());
        }
        v
    }

    #[test]
    fn reads_erddap_dap2() {
        let g = parse_dods(&dods()).unwrap();
        assert_eq!((g.nx, g.ny), (4, 3));
        assert_eq!((g.west, g.south, g.dlon, g.dlat), (173.0, -41.5, 0.25, 0.25));
        // flipped to south first: the first row is the -41.5 row (j = 2 in the file)
        assert_eq!(g.z[0], -20.0);
        assert_eq!(g.z[3], -23.0);
        assert!(g.z[8].is_nan(), "fill value");
        assert_eq!(g.z[11], -3.0);
    }

    #[test]
    fn plans_and_urls() {
        assert_eq!(etopo_plan([173.0, -42.0, 175.0, -40.0], 15), (481 * 481, 1));
        assert_eq!(etopo_plan([173.0, -42.0, 175.0, -40.0], 60).1, 4);
        // a big area at 15″ is thinned to stay under the limit
        let (n, stride) = etopo_plan([160.0, -50.0, 180.0, -30.0], 15);
        assert!(n <= MAX_POINTS && stride == 2, "{n} {stride}");
        let u = etopo_url(173.0, -42.0, 175.0, -40.0, 2);
        assert!(u.ends_with("ETOPO_2022_v1_15s.dods?z%5B(-42.00000):2:(-40.00000)%5D%5B(173.00000):2:(175.00000)%5D"), "{u}");
    }

    #[test]
    fn joins_across_the_date_line() {
        let a = HeightGrid { west: 179.5, south: -40.0, dlon: 0.25, dlat: 0.25, nx: 2, ny: 2, z: vec![1.0, 2.0, 3.0, 4.0] };
        let b = HeightGrid { west: -180.0, south: -40.0, dlon: 0.25, dlat: 0.25, nx: 1, ny: 2, z: vec![9.0, 8.0] };
        let g = join_east(a, b).unwrap();
        assert_eq!(g.nx, 3);
        assert_eq!(g.z, vec![1.0, 2.0, 9.0, 3.0, 4.0, 8.0]);
        assert_eq!(g.west, 179.5);
    }
}
