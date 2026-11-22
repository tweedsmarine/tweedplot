//! Copernicus Marine Service data, read straight from its cloud store.
//!
//! Copernicus Marine publishes every dataset as "ARCO" Zarr stores (Analysis
//! Ready, Cloud Optimised) on CloudFerro's object storage, open to anonymous
//! reading over HTTPS. The official Python toolbox reads exactly these; here
//! they are read in Rust, so no Python and no toolbox:
//!
//! 1. The STAC catalogue gives each product's datasets and, for each dataset
//!    version, where its stores are:
//!    `https://s3.waw3-1.cloudferro.com/mdl-metadata/metadata/{PRODUCT}/product.stac.json`,
//!    then `{dataset}_{version}/dataset.stac.json` (assets `timeChunked`, `geoChunked`).
//!    The newest released version is used (two versions are published side by
//!    side for a few weeks when a product is updated).
//! 2. A store's consolidated metadata (`.zmetadata`, JSON) gives each array's
//!    shape, chunking, type, compression and fill value.
//! 3. Chunks are fetched by index (`thetao/2633.49.0.2`) and decompressed
//!    (Blosc with LZ4, see `blosc`). A chunk that is all land or empty is not
//!    stored, and the server answers 403 for it.
//!
//! Two stores hold the same data chunked two ways: `timeChunked` (one time,
//! a big area per chunk: good for many places at a few times) and `geoChunked`
//! (many times, a small area per chunk). Each request takes whichever needs
//! fewer bytes.
//!
//! Datasets are global and regular, so an area across 180° is read as one
//! window whose columns wrap round: the grid runs on past 180° with no seam.
//!
//! Data: E.U. Copernicus Marine Service Information; free and open
//! (Copernicus Marine Service licence), with that credit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use grib::netcdf::cf_time;
use grib::{Field, Grid, Param};
use serde_json::Value;

use crate::ocean::{name, stride_for, width, write};
use crate::{client, Error, Model, Progress, Request, Result};

pub const STAC_ROOT: &str = "https://s3.waw3-1.cloudferro.com/mdl-metadata/metadata";

/// Copernicus Marine's global physics model (Mercator 1/12°) and biogeochemistry
/// model (1/4°), and its gap-free ocean colour (4 km).
pub const PHY: &str = "GLOBAL_ANALYSISFORECAST_PHY_001_024";
pub const BGC: &str = "GLOBAL_ANALYSISFORECAST_BGC_001_028";
pub const OC: &str = "OCEANCOLOUR_GLO_BGC_L4_NRT_009_102";

fn bad(why: impl Into<String>) -> Error {
    Error::Bad(why.into())
}

// ---------------------------------------------------------------- catalogue

/// Where a dataset's stores are.
#[derive(Debug, Clone, PartialEq)]
pub struct Stores {
    /// The dataset id with its version, e.g. "cmems_mod_glo_phy-cur_anfc_0.083deg_PT6H-i_202406".
    pub id: String,
    pub time: String,
    pub geo: Option<String>,
}

/// The versions of `dataset` listed in a product's STAC collection, newest first:
/// links "{dataset}_{version}/dataset.stac.json".
pub fn versions(product_json: &Value, dataset: &str) -> Vec<String> {
    let mut v: Vec<String> = product_json["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["rel"] == "item")
        .filter_map(|l| l["href"].as_str())
        .filter_map(|h| h.strip_suffix("/dataset.stac.json"))
        .filter(|h| h.strip_prefix(dataset).and_then(|r| r.strip_prefix('_')).is_some_and(|ver| !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit())))
        .map(str::to_string)
        .collect();
    v.sort();
    v.reverse();
    v
}

fn iso_time(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.timestamp())
}

/// A dataset version is in service: released, not in preparation, not retired.
pub fn in_service(item: &Value, now: i64) -> bool {
    let p = &item["properties"];
    if p["admp_in_preparation"].as_bool() == Some(true) {
        return false;
    }
    if iso_time(&p["admp_released_date"]).is_some_and(|t| t > now) {
        return false;
    }
    !iso_time(&p["admp_retired_date"]).is_some_and(|t| t <= now)
}

pub fn stores_of(item: &Value, id: &str) -> Result<Stores> {
    let href = |k: &str| item["assets"][k]["href"].as_str().map(str::to_string);
    Ok(Stores { id: id.to_string(), time: href("timeChunked").ok_or_else(|| bad(format!("{id}: no time-chunked store")))?, geo: href("geoChunked") })
}

fn get_json(c: &reqwest::blocking::Client, url: &str, cancel: &AtomicBool) -> Result<Value> {
    let b = get(c, url, cancel)?.ok_or_else(|| Error::Http(format!("Copernicus Marine: not found ({url})")))?;
    serde_json::from_slice(&b).map_err(|e| bad(format!("Copernicus Marine: {e} ({url})")))
}

/// The stores of the newest version of a dataset that is in service.
pub fn resolve(c: &reqwest::blocking::Client, product: &str, dataset: &str, now: i64, cancel: &AtomicBool) -> Result<Stores> {
    let prod = get_json(c, &format!("{STAC_ROOT}/{product}/product.stac.json"), cancel)?;
    let vs = versions(&prod, dataset);
    for id in &vs {
        let item = get_json(c, &format!("{STAC_ROOT}/{product}/{id}/dataset.stac.json"), cancel)?;
        if in_service(&item, now) {
            return stores_of(&item, id);
        }
    }
    Err(bad(format!("Copernicus Marine: no dataset {dataset} in service in {product}")))
}

// ---------------------------------------------------------------- HTTP

/// GET with retries: `None` when the object isn't there (403 or 404: the
/// store keeps no chunk where all is land or empty, and lists nothing).
fn get(c: &reqwest::blocking::Client, url: &str, cancel: &AtomicBool) -> Result<Option<Vec<u8>>> {
    let mut wait = 1;
    for attempt in 0..5 {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match c.get(url).send() {
            Ok(r) if r.status().is_success() => return r.bytes().map(|b| Some(b.to_vec())).map_err(|e| Error::Http(e.to_string())),
            Ok(r) if matches!(r.status().as_u16(), 403 | 404) => return Ok(None),
            Ok(r) if attempt == 4 => return Err(Error::Http(format!("Copernicus Marine answered HTTP {}", r.status().as_u16()))),
            Err(e) if attempt == 4 => return Err(Error::Http(format!("Copernicus Marine: {e}"))),
            _ => {}
        }
        std::thread::sleep(Duration::from_secs(wait));
        wait *= 2;
    }
    unreachable!()
}

// ---------------------------------------------------------------- Zarr

#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    pub shape: Vec<usize>,
    pub chunks: Vec<usize>,
    /// "f4", "f8", "i1", "i2", "i4", "u1", "u2" (little-endian, or one byte)
    pub dtype: String,
    pub compressor: Option<String>,
    pub fill: Option<f64>,
    pub scale: f64,
    pub offset: f64,
    pub dims: Vec<String>,
    pub sep: String,
    pub units: String,
}

impl Array {
    fn item(&self) -> usize {
        self.dtype[1..].parse().unwrap_or(4)
    }
    fn chunk_len(&self) -> usize {
        self.chunks.iter().product()
    }
    pub fn chunk_bytes(&self) -> usize {
        self.chunk_len() * self.item()
    }
}

/// A store's arrays, from its consolidated metadata.
#[derive(Debug, Clone)]
pub struct Zarr {
    pub base: String,
    pub arrays: HashMap<String, Array>,
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) if s == "NaN" => Some(f64::NAN),
        _ => None,
    }
}

impl Zarr {
    pub fn from_metadata(base: &str, zmetadata: &Value) -> Result<Zarr> {
        let m = zmetadata["metadata"].as_object().ok_or_else(|| bad("Zarr: no consolidated metadata"))?;
        let mut arrays = HashMap::new();
        for (k, a) in m {
            let Some(name) = k.strip_suffix("/.zarray") else { continue };
            let at = m.get(&format!("{name}/.zattrs")).cloned().unwrap_or(Value::Null);
            let list = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect::<Vec<_>>()).unwrap_or_default();
            let dtype = a["dtype"].as_str().unwrap_or("").to_string();
            if dtype.starts_with('>') {
                return Err(bad(format!("Zarr: big-endian array {name}")));
            }
            let dtype = dtype.trim_start_matches(['<', '|']).to_string();
            // _FillValue (CF) wins over the Zarr fill value: both mean "no data"
            let fill = num(&at["_FillValue"]).or_else(|| num(&a["fill_value"])).filter(|f| !f.is_nan());
            arrays.insert(
                name.to_string(),
                Array {
                    shape: list(&a["shape"]),
                    chunks: list(&a["chunks"]),
                    dtype,
                    compressor: a["compressor"]["id"].as_str().map(str::to_string),
                    fill,
                    scale: num(&at["scale_factor"]).unwrap_or(1.0),
                    offset: num(&at["add_offset"]).unwrap_or(0.0),
                    dims: at["_ARRAY_DIMENSIONS"].as_array().map(|d| d.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                    sep: a["dimension_separator"].as_str().unwrap_or(".").to_string(),
                    units: at["units"].as_str().unwrap_or("").to_string(),
                },
            );
        }
        Ok(Zarr { base: base.trim_end_matches('/').to_string(), arrays })
    }

    pub fn open(c: &reqwest::blocking::Client, base: &str, cancel: &AtomicBool) -> Result<Zarr> {
        Zarr::from_metadata(base, &get_json(c, &format!("{}/.zmetadata", base.trim_end_matches('/')), cancel)?)
    }

    pub fn array(&self, name: &str) -> Result<&Array> {
        self.arrays.get(name).ok_or_else(|| bad(format!("Copernicus Marine: no variable {name}")))
    }

    pub fn chunk_url(&self, name: &str, idx: &[usize]) -> String {
        let sep = self.arrays.get(name).map(|a| a.sep.as_str()).unwrap_or(".");
        let key: Vec<String> = idx.iter().map(|i| i.to_string()).collect();
        format!("{}/{name}/{}", self.base, key.join(sep))
    }

    /// A whole one-dimensional array (a coordinate).
    pub fn read_1d(&self, c: &reqwest::blocking::Client, name: &str, cancel: &AtomicBool) -> Result<Vec<f64>> {
        let a = self.array(name)?;
        let n = *a.shape.first().ok_or_else(|| bad(format!("Zarr: {name} has no shape")))?;
        let per = a.chunks.first().copied().unwrap_or(n).max(1);
        let mut out = Vec::with_capacity(n);
        for k in 0..n.div_ceil(per) {
            let vals = match get(c, &self.chunk_url(name, &[k]), cancel)? {
                Some(b) => decode(a, &b)?,
                None => vec![f32::NAN; per],
            };
            out.extend(vals.iter().take(n - k * per).map(|&v| v as f64));
        }
        Ok(out)
    }
}

/// Decode one chunk into values (no data = NaN, scale and offset applied).
pub fn decode(a: &Array, bytes: &[u8]) -> Result<Vec<f32>> {
    let raw = match a.compressor.as_deref() {
        None => bytes.to_vec(),
        Some("blosc") => crate::blosc::decompress(bytes)?,
        Some("zlib") | Some("gzip") => {
            use std::io::Read;
            let mut out = Vec::new();
            if a.compressor.as_deref() == Some("zlib") {
                flate2::read::ZlibDecoder::new(bytes).read_to_end(&mut out)?;
            } else {
                flate2::read::GzDecoder::new(bytes).read_to_end(&mut out)?;
            }
            out
        }
        Some(other) => return Err(bad(format!("Zarr: compressor {other} is not supported"))),
    };
    let n = a.chunk_len();
    let item = a.item();
    if raw.len() < n * item {
        return Err(bad("Zarr: a chunk is shorter than it should be"));
    }
    let b = |i: usize| &raw[i * item..(i + 1) * item];
    let get_f = |i: usize| -> f64 {
        let s = b(i);
        match a.dtype.as_str() {
            "f4" => f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
            "f8" => f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]),
            "i1" => s[0] as i8 as f64,
            "u1" => s[0] as f64,
            "i2" => i16::from_le_bytes([s[0], s[1]]) as f64,
            "u2" => u16::from_le_bytes([s[0], s[1]]) as f64,
            "i4" => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
            "u4" => u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
            _ => f64::NAN,
        }
    };
    if !matches!(a.dtype.as_str(), "f4" | "f8" | "i1" | "u1" | "i2" | "u2" | "i4" | "u4") {
        return Err(bad(format!("Zarr: type {} is not supported", a.dtype)));
    }
    Ok((0..n)
        .map(|i| {
            let v = get_f(i);
            let empty = !v.is_finite() || a.fill.is_some_and(|f| v == f || (f.abs() > 1e30 && (v - f).abs() <= f.abs() * 1e-6));
            if empty {
                f32::NAN
            } else {
                (v * a.scale + a.offset) as f32
            }
        })
        .collect())
}

// ---------------------------------------------------------------- windows

/// The columns of an area on a global, regular longitude axis: the first
/// column's index and how many, every `k`th (wrapping round past 180°).
pub fn lon_window(lons: &[f64], area: [f64; 4], k: usize) -> Result<(usize, usize, f64, f64)> {
    let n = lons.len();
    if n < 2 {
        return Err(bad("no longitudes"));
    }
    let d = (lons[n - 1] - lons[0]) / (n - 1) as f64;
    if (n as f64 * d - 360.0).abs() > 2.0 * d {
        return Err(bad("the dataset is not global in longitude"));
    }
    let wid = width(area).min(360.0 - d);
    let w = area[0];
    // the first column at or east of the west edge
    let off = ((w - lons[0]).rem_euclid(360.0) / d - 1e-6).ceil() as usize % n;
    let count = ((wid - ((lons[0] + off as f64 * d - w).rem_euclid(360.0))) / d + 1e-6).floor() as usize + 1;
    let k = k.max(1);
    let cols = count.div_ceil(k).max(1);
    let lon0 = lons[0] + off as f64 * d;
    // the west edge as asked (e.g. 175 for an area 175°E to 170°W), not wrapped differently
    let lon0 = lon0 + ((w - lon0) / 360.0).round() * 360.0;
    Ok((off, cols, lon0, d * k as f64))
}

/// The rows of an area, south to north, every `k`th: the source indices.
pub fn lat_rows(lats: &[f64], s: f64, n: f64, k: usize) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..lats.len()).filter(|&j| lats[j] >= s - 1e-9 && lats[j] <= n + 1e-9).collect();
    rows.sort_by(|&a, &b| lats[a].partial_cmp(&lats[b]).unwrap_or(std::cmp::Ordering::Equal));
    rows.into_iter().step_by(k.max(1)).collect()
}

/// What to read from one variable: times (indices), a level (index, when
/// the variable has one), the rows and columns.
pub struct Window<'a> {
    pub times: &'a [usize],
    pub level: Option<usize>,
    pub rows: &'a [usize],
    /// first column, number of columns, step (columns wrap round)
    pub col0: usize,
    pub ncols: usize,
    pub k: usize,
    pub nlon: usize,
}

impl Window<'_> {
    fn col(&self, m: usize) -> usize {
        (self.col0 + m * self.k) % self.nlon
    }
}

/// The chunks a window needs: (time chunk, lat chunk, lon chunk), with
/// the level chunk fixed.
fn chunks_for(a: &Array, w: &Window) -> Vec<Vec<usize>> {
    let nd = a.shape.len();
    let (ct, cy, cx) = (a.chunks[0], a.chunks[nd - 2], a.chunks[nd - 1]);
    let mut ts: Vec<usize> = w.times.iter().map(|t| t / ct).collect();
    ts.dedup();
    let mut ys: Vec<usize> = w.rows.iter().map(|j| j / cy).collect();
    ys.sort();
    ys.dedup();
    let mut xs: Vec<usize> = (0..w.ncols).map(|m| w.col(m) / cx).collect();
    xs.sort();
    xs.dedup();
    let mut out = Vec::new();
    for &t in &ts {
        for &y in &ys {
            for &x in &xs {
                let mut idx = vec![t];
                if nd == 4 {
                    idx.push(w.level.unwrap_or(0) / a.chunks[1]);
                }
                idx.extend([y, x]);
                out.push(idx);
            }
        }
    }
    out
}

/// Bytes a window would fetch from this store (before compression).
pub fn window_cost(a: &Array, w: &Window) -> usize {
    chunks_for(a, w).len() * a.chunk_bytes()
}

/// Read a window of one variable: one grid of values (rows south to north)
/// per time asked. `got` is told the bytes of each chunk as it comes.
pub fn read_window(c: &reqwest::blocking::Client, z: &Zarr, var: &str, w: &Window, cancel: &AtomicBool, got: &mut dyn FnMut(usize)) -> Result<Vec<Vec<f32>>> {
    let a = z.array(var)?;
    let nd = a.shape.len();
    if !(nd == 3 || nd == 4) {
        return Err(bad(format!("Copernicus Marine: {var} is not a map over time")));
    }
    let (ct, cy, cx) = (a.chunks[0], a.chunks[nd - 2], a.chunks[nd - 1]);
    let cl = if nd == 4 { a.chunks[1] } else { 1 };
    let nj = w.rows.len();
    let mut out = vec![vec![f32::NAN; nj * w.ncols]; w.times.len()];
    let jobs = chunks_for(a, w);
    // a few at a time, in parallel
    for batch in jobs.chunks(6) {
        let fetched: Vec<Result<(Vec<usize>, Option<Vec<f32>>, usize)>> = std::thread::scope(|s| {
            let hs: Vec<_> = batch
                .iter()
                .map(|idx| {
                    let url = z.chunk_url(var, idx);
                    s.spawn(move || -> Result<(Vec<usize>, Option<Vec<f32>>, usize)> {
                        match get(c, &url, cancel)? {
                            Some(b) => Ok((idx.clone(), Some(decode(a, &b)?), b.len())),
                            None => Ok((idx.clone(), None, 0)),
                        }
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(bad("a download thread failed")))).collect()
        });
        for r in fetched {
            let (idx, vals, n) = r?;
            got(n);
            let Some(vals) = vals else { continue };
            let (t_c, y_c, x_c) = (idx[0], idx[nd - 2], idx[nd - 1]);
            let lev_in = w.level.map(|l| l % cl).unwrap_or(0);
            for (ti, &t) in w.times.iter().enumerate() {
                if t / ct != t_c {
                    continue;
                }
                let tt = t % ct;
                let base = if nd == 4 { (tt * cl + lev_in) * cy * cx } else { tt * cy * cx };
                for (jj, &j) in w.rows.iter().enumerate() {
                    if j / cy != y_c {
                        continue;
                    }
                    let row = base + (j % cy) * cx;
                    for m in 0..w.ncols {
                        let i = w.col(m);
                        if i / cx != x_c {
                            continue;
                        }
                        out[ti][jj * w.ncols + m] = vals[row + i % cx];
                    }
                }
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- what is offered

/// One variable to read, and what it becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Var {
    pub dataset: &'static str,
    pub name: &'static str,
    pub param: Param,
    pub category: u8,
    pub number: u8,
    /// °C to K
    pub kelvin: bool,
    /// at the chosen depth (else the top level)
    pub at_depth: bool,
    /// at every depth down to `STACK_DEPTH`
    pub stack: bool,
}

const fn v(dataset: &'static str, name: &'static str, param: Param, category: u8, number: u8, kelvin: bool, at_depth: bool) -> Var {
    Var { dataset, name, param, category, number, kelvin, at_depth, stack: false }
}

const fn stack(dataset: &'static str, name: &'static str, param: Param, category: u8, number: u8, kelvin: bool) -> Var {
    Var { dataset, name, param, category, number, kelvin, at_depth: true, stack: true }
}

/// The depth stack goes down to here (m): the seasonal thermocline and below it, where
/// longlines and midwater trawls fish (31 of the model's 50 levels, 0.5 to 454 m).
pub const STACK_DEPTH: f64 = 500.0;

/// The levels of the stack: indices of the elevation axis (negative down) within `STACK_DEPTH`.
pub fn stack_levels(elev: &[f64]) -> Vec<usize> {
    let mut l: Vec<usize> = (0..elev.len()).filter(|&i| elev[i].is_finite() && elev[i].abs() <= STACK_DEPTH).collect();
    l.sort_by(|&a, &b| elev[a].abs().partial_cmp(&elev[b].abs()).unwrap_or(std::cmp::Ordering::Equal));
    l
}

pub const PHY_T: &str = "cmems_mod_glo_phy-thetao_anfc_0.083deg_PT6H-i";
pub const PHY_S: &str = "cmems_mod_glo_phy-so_anfc_0.083deg_PT6H-i";
pub const PHY_UV: &str = "cmems_mod_glo_phy-cur_anfc_0.083deg_PT6H-i";
pub const PHY_DAY: &str = "cmems_mod_glo_phy_anfc_0.083deg_P1D-m";
pub const PHY_TOTAL: &str = "cmems_mod_glo_phy_anfc_merged-uv_PT1H-i";
pub const BGC_PFT: &str = "cmems_mod_glo_bgc-pft_anfc_0.25deg_P1D-m";
pub const BGC_BIO: &str = "cmems_mod_glo_bgc-bio_anfc_0.25deg_P1D-m";
pub const BGC_CAR: &str = "cmems_mod_glo_bgc-car_anfc_0.25deg_P1D-m";
pub const OC_CHL: &str = "cmems_obs-oc_glo_bgc-plankton_nrt_l4-gapfree-multi-4km_P1D";

/// The variables behind a model's parameter ids.
pub fn vars_for(model: Model, params: &[String]) -> Vec<Var> {
    let has = |p: &str| params.iter().any(|x| x == p);
    let mut out = Vec::new();
    match model {
        Model::CmemsPhy => {
            if has("sst") {
                out.push(v(PHY_T, "thetao", Param::SeaTemp, 3, 0, true, false));
            }
            if has("sss") {
                out.push(v(PHY_S, "so", Param::Salinity, 4, 21, false, false));
            }
            if has("current") {
                out.push(v(PHY_UV, "uo", Param::CurrentU, 1, 2, false, false));
                out.push(v(PHY_UV, "vo", Param::CurrentV, 1, 3, false, false));
            }
            if has("total") {
                out.push(v(PHY_TOTAL, "utotal", Param::CurrentU, 1, 2, false, false));
                out.push(v(PHY_TOTAL, "vtotal", Param::CurrentV, 1, 3, false, false));
            }
            if has("mld") {
                out.push(v(PHY_DAY, "mlotst", Param::MixedLayer, 4, 193, false, false));
            }
            if has("ssh") {
                out.push(v(PHY_DAY, "zos", Param::SeaLevel, 3, 1, false, false));
            }
            if has("stack") {
                out.push(stack(PHY_T, "thetao", Param::SubTemp, 3, 0, true));
                out.push(stack(PHY_S, "so", Param::SubSalinity, 4, 21, false));
                out.push(stack(PHY_UV, "uo", Param::SubCurrentU, 1, 192, false));
                out.push(stack(PHY_UV, "vo", Param::SubCurrentV, 1, 193, false));
            } else if has("depth") {
                out.push(v(PHY_T, "thetao", Param::SubTemp, 3, 0, true, true));
                out.push(v(PHY_S, "so", Param::SubSalinity, 4, 21, false, true));
                out.push(v(PHY_UV, "uo", Param::SubCurrentU, 1, 192, false, true));
                out.push(v(PHY_UV, "vo", Param::SubCurrentV, 1, 193, false, true));
            }
        }
        Model::CmemsBgc => {
            if has("chl") {
                out.push(v(BGC_PFT, "chl", Param::Chlorophyll, 4, 192, false, false));
            }
            if has("phyc") {
                out.push(v(BGC_PFT, "phyc", Param::Phytoplankton, 4, 194, false, false));
            }
            if has("nppv") {
                out.push(v(BGC_BIO, "nppv", Param::PrimaryProduction, 4, 195, false, false));
            }
            if has("o2") {
                out.push(v(BGC_BIO, "o2", Param::Oxygen, 4, 196, false, false));
            }
            if has("ph") {
                out.push(v(BGC_CAR, "ph", Param::Ph, 4, 197, false, false));
            }
        }
        Model::CmemsChl if has("chl") => out.push(v(OC_CHL, "CHL", Param::Chlorophyll, 4, 192, false, false)),
        _ => {}
    }
    out
}

pub fn product_of(dataset: &str) -> &'static str {
    if dataset.starts_with("cmems_mod_glo_bgc") {
        BGC
    } else if dataset.starts_with("cmems_obs-oc") {
        OC
    } else {
        PHY
    }
}

fn process_of(model: Model) -> u8 {
    match model {
        Model::CmemsBgc => grib::write::PROCESS_CMEMS_BGC,
        Model::CmemsChl => grib::write::PROCESS_CMEMS_OC,
        _ => grib::write::PROCESS_CMEMS_PHY,
    }
}

/// The source spacing of a model, degrees.
pub fn res_of(model: Model) -> f64 {
    match model {
        Model::CmemsBgc => 0.25,
        Model::CmemsChl => 1.0 / 24.0,
        _ => 1.0 / 12.0,
    }
}

/// The times to take: from `from` to `to`, no closer than `every` seconds
/// (indices into `times`, in order).
pub fn pick_times(times: &[i64], from: i64, to: i64, every: i64) -> Vec<usize> {
    let mut out = Vec::new();
    let mut last = i64::MIN;
    for (i, &t) in times.iter().enumerate() {
        if t >= from && t <= to && (last == i64::MIN || t - last >= every) {
            out.push(i);
            last = t;
        }
    }
    out
}

/// The level nearest a depth (m, positive down) on an elevation axis (negative down).
pub fn nearest_level(elev: &[f64], depth: f64) -> Option<usize> {
    (0..elev.len()).filter(|&i| elev[i].is_finite()).min_by(|&a, &b| (elev[a] + depth).abs().partial_cmp(&(elev[b] + depth).abs()).unwrap_or(std::cmp::Ordering::Equal))
}

/// An opened dataset: its axes, in both chunkings.
struct Opened {
    time: Zarr,
    geo: Option<Zarr>,
    times: Vec<i64>,
    lats: Vec<f64>,
    lons: Vec<f64>,
    elev: Vec<f64>,
}

fn open(c: &reqwest::blocking::Client, dataset: &str, now: i64, cancel: &AtomicBool) -> Result<Opened> {
    let st = resolve(c, product_of(dataset), dataset, now, cancel)?;
    let time = Zarr::open(c, &st.time, cancel)?;
    let geo = match &st.geo {
        Some(g) => Zarr::open(c, g, cancel).ok(),
        None => None,
    };
    let units = time.array("time")?.units.clone();
    let times = time.read_1d(c, "time", cancel)?.into_iter().map(|t| cf_time(t, &units).ok_or_else(|| bad(format!("time units “{units}”")))).collect::<Result<Vec<_>>>()?;
    let lats = time.read_1d(c, "latitude", cancel)?;
    let lons = time.read_1d(c, "longitude", cancel)?;
    let elev = if time.arrays.contains_key("elevation") { time.read_1d(c, "elevation", cancel)? } else { Vec::new() };
    Ok(Opened { time, geo, times, lats, lons, elev })
}

/// The times wanted from a dataset for a request.
fn times_for(model: Model, req: &Request, times: &[i64], now: i64) -> Vec<usize> {
    let day = 86_400;
    match model {
        // observations: the latest days
        Model::CmemsChl => {
            let days = (req.hours / 24).clamp(1, 14) as usize;
            let n = times.len();
            (n.saturating_sub(days)..n).collect()
        }
        // models: from the latest time at or before now (the analysis) to the length asked
        _ => {
            let start = times.iter().copied().filter(|&t| t <= now).max().unwrap_or(now);
            let step = if times.len() > 1 { times[1] - times[0] } else { day };
            let every = if step >= day { day } else { (req.every.max(1) as i64 * 3600).max(step) };
            // daily means are stamped at the start or middle of their day
            let from = if step >= day { start - day / 2 } else { start };
            pick_times(times, from, now + req.hours as i64 * 3600, every)
        }
    }
}

/// Download a Copernicus Marine request into `dir`; returns the file written.
pub fn download(req: &Request, now: i64, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let c = client()?;
    let vars = vars_for(req.model, &req.params);
    if vars.is_empty() {
        return Err(bad("choose at least one parameter"));
    }
    let mut datasets: Vec<&str> = vars.iter().map(|v| v.dataset).collect();
    datasets.dedup();
    datasets.sort();
    datasets.dedup();
    let mut total = vars.len();
    let mut bytes = 0u64;
    progress(0, total, 0);
    let mut fields: Vec<Field> = Vec::new();
    let mut done = 0;
    for ds in datasets {
        let o = open(&c, ds, now, cancel)?;
        let tix = times_for(req.model, req, &o.times, now);
        if tix.is_empty() {
            continue;
        }
        let ds_vars: Vec<&Var> = vars.iter().filter(|v| v.dataset == ds).collect();
        let res = (o.lons[o.lons.len() - 1] - o.lons[0]) / (o.lons.len() - 1) as f64;
        let nlev = if vars.iter().any(|v| v.stack) { stack_levels(&o.elev).len().max(1) } else { 1 };
        let k = stride_for(req.area, res, tix.len() * vars.len() * nlev);
        let rows = lat_rows(&o.lats, req.area[1].min(req.area[3]), req.area[1].max(req.area[3]), k);
        if rows.len() < 2 {
            return Err(bad("the area is too small for this dataset's grid"));
        }
        let (col0, ncols, lon0, dlon) = lon_window(&o.lons, req.area, k)?;
        let dlat = (o.lats[rows[1]] - o.lats[rows[0]]).abs();
        let grid = Grid::regular(ncols, rows.len(), o.lats[rows[0]], lon0, dlat, dlon);
        let surface = nearest_level(&o.elev, 0.0);
        let deep = nearest_level(&o.elev, if req.depth > 0.0 { req.depth } else { 50.0 });
        let jobs: Vec<(&Var, Option<usize>)> = ds_vars
            .iter()
            .flat_map(|var| {
                let levels: Vec<Option<usize>> = if o.elev.is_empty() {
                    vec![None]
                } else if var.stack {
                    stack_levels(&o.elev).into_iter().map(Some).collect()
                } else if var.at_depth {
                    vec![deep]
                } else {
                    vec![surface]
                };
                levels.into_iter().map(move |l| (*var, l))
            })
            .collect();
        total += jobs.len() - ds_vars.len();
        for (var, level) in jobs {
            let w = Window { times: &tix, level, rows: &rows, col0, ncols, k, nlon: o.lons.len() };
            // the cheaper chunking
            let a_t = o.time.array(var.name)?;
            let z = match &o.geo {
                Some(g) if g.array(var.name).is_ok_and(|a_g| a_g.dims == a_t.dims && window_cost(a_g, &w) < window_cost(a_t, &w)) => g,
                _ => &o.time,
            };
            let grids = read_window(&c, z, var.name, &w, cancel, &mut |n| {
                bytes += n as u64;
                progress(done, total, bytes);
            })?;
            let depth = level.and_then(|l| o.elev.get(l)).map(|e| e.abs()).unwrap_or(0.0);
            for (gi, &ti) in tix.iter().enumerate() {
                let mut values = grids[gi].clone();
                if values.iter().all(|x| x.is_nan()) {
                    continue;
                }
                if var.kelvin {
                    values.iter_mut().for_each(|x| *x += 273.15);
                }
                fields.push(Field {
                    param: var.param,
                    discipline: 10,
                    category: var.category,
                    number: var.number,
                    surface_type: if var.at_depth { 160 } else { 1 },
                    surface_value: if var.at_depth { (depth * 10.0).round() / 10.0 } else { 0.0 },
                    centre: grib::write::CENTRE_LOCAL,
                    ref_time: o.times[tix[0]],
                    valid_time: o.times[ti],
                    member: None,
                    grid,
                    values,
                    local: None,
                });
            }
            done += 1;
            progress(done, total, bytes);
        }
    }
    if fields.is_empty() {
        return Err(bad("no sea values in the area (all land?)"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    fields.sort_by_key(|f| (f.param, f.valid_time));
    let r = fields.iter().map(|f| f.valid_time).min().unwrap_or(now);
    for f in fields.iter_mut() {
        f.ref_time = r;
    }
    let last = fields.iter().map(|f| f.valid_time).max().unwrap_or(r);
    let when = chrono::DateTime::from_timestamp(r, 0).unwrap_or_default();
    let (prefix, span) = match req.model {
        Model::CmemsChl => ("CMEMS_CHL", format!("{}-{}", when.format("%Y%m%d"), chrono::DateTime::from_timestamp(last, 0).unwrap_or_default().format("%Y%m%d"))),
        Model::CmemsBgc => ("CMEMS_BGC", format!("{}_f000-f{:03}", when.format("%Y%m%d_%Hz"), (last - r) / 3600)),
        _ => ("CMEMS_PHY", format!("{}_f000-f{:03}", when.format("%Y%m%d_%Hz"), (last - r) / 3600)),
    };
    let path = dir.join(name(prefix, &span, req.area));
    write(&fields, process_of(req.model), &path)?;
    Ok(path)
}

/// How many chunks of `c` points cover indices `a..=b` on an axis of `n` (wrapping past the end).
fn chunk_span(a: i64, b: i64, c: f64, n: i64) -> f64 {
    let c = c as i64;
    let count = |a: i64, b: i64| (b.min(n - 1) / c - a.max(0) / c + 1) as f64;
    if b >= n {
        count(a, n - 1) + count(0, b - n)
    } else {
        count(a, b)
    }
}

/// What a chunk compresses to, roughly (a quarter at the surface, much less at depth).
const COMPRESSED: f64 = 0.3;

/// Roughly how much a request downloads, bytes: for each variable, the
/// cheaper of the two chunkings, at about a third of the raw size after compression.
/// (Chunk shapes and grids as published in November 2026; the download itself reads them.)
pub fn download_estimate(req: &Request) -> u64 {
    let res = res_of(req.model);
    // index ranges: latitude from the south edge of the grid, longitude from 180°W
    let lat0 = if req.model == Model::CmemsChl { -90.0 } else { -80.0 };
    let (s, n) = (req.area[1].min(req.area[3]).max(lat0), req.area[1].max(req.area[3]));
    let (j0, j1) = (((s - lat0) / res).floor() as i64, ((n - lat0) / res).floor() as i64);
    let nlon = (360.0 / res).round() as i64;
    let i0 = ((nav_core::wrap_lon(req.area[0]) + 180.0) / res).floor() as i64;
    let i1 = i0 + (width(req.area) / res).ceil() as i64;
    let mut total = 0.0;
    for var in vars_for(req.model, &req.params) {
        // (time-chunked lat, lon), (geo-chunked times, lat, lon), the dataset's step (h)
        let (tc, gc, step): ((f64, f64), Option<(f64, f64, f64)>, f64) = match var.dataset {
            PHY_TOTAL => ((512.0, 2048.0), Some((4272.0, 16.0, 8.0)), 1.0),
            PHY_DAY => ((512.0, 2048.0), None, 24.0),
            BGC_PFT | BGC_BIO | BGC_CAR => ((681.0, 1440.0), Some((155.0, 32.0, 64.0)), 24.0),
            OC_CHL => ((512.0, 1024.0), Some((5.0, 256.0, 256.0)), 24.0),
            _ => ((512.0, 2048.0), Some((526.0, 32.0, 32.0)), 6.0),
        };
        let nt = if req.model == Model::CmemsChl {
            (req.hours / 24).clamp(1, 14) as f64
        } else {
            (req.hours as f64 / (req.every as f64).max(step)).floor() + 1.0
        };
        let tiles = |cy: f64, cx: f64| chunk_span(j0, j1, cy, i64::MAX) * chunk_span(i0, i1.min(i0 + nlon - 1), cx, nlon);
        let by_time = nt * tiles(tc.0, tc.1) * tc.0 * tc.1 * 4.0;
        // the times asked span one time chunk, or two where they straddle a boundary
        let by_geo = gc.map(|(ct, cy, cx)| (nt * (req.every as f64).max(step) / step / ct + 1.0).floor().max(1.0) * tiles(cy, cx) * ct * cy * cx * 4.0).unwrap_or(f64::MAX);
        let levels = if var.stack { 31.0 } else { 1.0 };
        total += by_time.min(by_geo) * COMPRESSED * levels;
    }
    total as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_newest_first() {
        let p = json!({"links": [
            {"rel": "item", "href": "cmems_obs-oc_glo_bgc-plankton_nrt_l3-multi-4km_P1D_202411/dataset.stac.json"},
            {"rel": "item", "href": "cmems_obs-oc_glo_bgc-plankton_nrt_l3-multi-4km_P1D_202611/dataset.stac.json"},
            {"rel": "item", "href": "cmems_obs-oc_glo_bgc-plankton_nrt_l3-multi-4km_P1M_202411/dataset.stac.json"},
            {"rel": "item", "href": "cmems_obs-oc_glo_bgc-plankton_nrt_l3-olci-4km_P1D_202511/dataset.stac.json"},
            {"rel": "parent", "href": "../catalog.stac.json"}
        ]});
        assert_eq!(
            versions(&p, "cmems_obs-oc_glo_bgc-plankton_nrt_l3-multi-4km_P1D"),
            vec!["cmems_obs-oc_glo_bgc-plankton_nrt_l3-multi-4km_P1D_202611", "cmems_obs-oc_glo_bgc-plankton_nrt_l3-multi-4km_P1D_202411"]
        );
    }

    #[test]
    fn service_dates() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-22T08:00:00Z").unwrap().timestamp();
        let old = json!({"properties": {"admp_retired_date": "2027-01-12T13:00:00.000Z", "admp_in_preparation": false, "admp_released_date": "2024-11-26T13:00:00.000Z"}});
        let new = json!({"properties": {"admp_retired_date": null, "admp_in_preparation": false, "admp_released_date": "2026-11-24T13:00:00.000Z"}});
        let gone = json!({"properties": {"admp_retired_date": "2026-01-12T13:00:00.000Z"}});
        assert!(in_service(&old, now));
        assert!(!in_service(&new, now));
        assert!(!in_service(&gone, now));
    }

    fn meta() -> Value {
        json!({"metadata": {
            "thetao/.zarray": {"chunks": [1, 1, 512, 2048], "compressor": {"blocksize": 0, "clevel": 5, "cname": "lz4", "id": "blosc", "shuffle": 1}, "dtype": "<f4", "fill_value": 9.969209968386869e36, "filters": null, "order": "C", "shape": [6581, 50, 2041, 4320], "zarr_format": 2},
            "thetao/.zattrs": {"_ARRAY_DIMENSIONS": ["time", "elevation", "latitude", "longitude"], "units": "degrees_C"},
            "time/.zarray": {"chunks": [2633], "compressor": {"id": "blosc"}, "dtype": "<f4", "fill_value": "NaN", "shape": [6581], "zarr_format": 2},
            "time/.zattrs": {"_ARRAY_DIMENSIONS": ["time"], "units": "hours since 1950-01-01"},
            "CHL_uncertainty/.zarray": {"chunks": [1, 512, 1024], "compressor": null, "dtype": "<i2", "fill_value": -32768, "shape": [10, 4320, 8640], "zarr_format": 2},
            "CHL_uncertainty/.zattrs": {"_ARRAY_DIMENSIONS": ["time", "latitude", "longitude"], "scale_factor": 0.01}
        }})
    }

    #[test]
    fn metadata() {
        let z = Zarr::from_metadata("https://x/timeChunked.zarr/", &meta()).unwrap();
        let a = z.array("thetao").unwrap();
        assert_eq!(a.chunks, vec![1, 1, 512, 2048]);
        assert_eq!(a.dtype, "f4");
        assert_eq!(a.compressor.as_deref(), Some("blosc"));
        assert_eq!(a.units, "degrees_C");
        assert!(a.fill.unwrap() > 9e36);
        assert_eq!(z.array("time").unwrap().fill, None);
        assert_eq!(z.chunk_url("thetao", &[6000, 49, 0, 2]), "https://x/timeChunked.zarr/thetao/6000.49.0.2");
    }

    #[test]
    fn decode_types() {
        let z = Zarr::from_metadata("https://x", &meta()).unwrap();
        let mut a = z.array("CHL_uncertainty").unwrap().clone();
        a.chunks = vec![1, 1, 3];
        let raw: Vec<u8> = [250i16, -32768, 1000].iter().flat_map(|v| v.to_le_bytes()).collect();
        let v = decode(&a, &raw).unwrap();
        assert!((v[0] - 2.5).abs() < 1e-6 && v[1].is_nan() && (v[2] - 10.0).abs() < 1e-5);
        // the netCDF default fill of a float is "no data"
        let mut t = z.array("thetao").unwrap().clone();
        t.compressor = None;
        t.chunks = vec![1, 1, 1, 2];
        let raw: Vec<u8> = [9.969_21e36_f32, 18.5].iter().flat_map(|v| v.to_le_bytes()).collect();
        let v = decode(&t, &raw).unwrap();
        assert!(v[0].is_nan() && v[1] == 18.5);
    }

    fn axis(start: f64, step: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| start + i as f64 * step).collect()
    }

    #[test]
    fn columns_and_rows() {
        let lons = axis(-180.0, 1.0 / 12.0, 4320);
        // 172°E to 175°E: 37 columns from index (172 + 180) × 12
        let (c0, n, lon0, d) = lon_window(&lons, [172.0, -42.0, 175.0, -40.0], 1).unwrap();
        assert_eq!((c0, n), (352 * 12, 37));
        assert!((lon0 - 172.0).abs() < 1e-9 && (d - 1.0 / 12.0).abs() < 1e-12);
        // across 180°: 175°E to 170°W (190) runs on past the end of the axis, unbroken
        let (c0, n, lon0, _) = lon_window(&lons, [175.0, -42.0, 190.0, -40.0], 1).unwrap();
        assert_eq!((c0, n), (355 * 12, 181));
        assert!((lon0 - 175.0).abs() < 1e-9);
        let w = Window { times: &[0], level: None, rows: &[0], col0: c0, ncols: n, k: 1, nlon: 4320 };
        assert_eq!(w.col(60), 0); // 180° is the axis' first column
        assert_eq!(w.col(180), 120);
        // given as -185 to -170 the same
        let (c0b, nb, lon0b, _) = lon_window(&lons, [-185.0, -42.0, -170.0, -40.0], 1).unwrap();
        assert_eq!((c0b, nb), (c0, n));
        assert!((lon0b + 185.0).abs() < 1e-9);
        // every second column
        let (_, n2, _, d2) = lon_window(&lons, [172.0, -42.0, 175.0, -40.0], 2).unwrap();
        assert_eq!(n2, 19);
        assert!((d2 - 1.0 / 6.0).abs() < 1e-12);
        // rows, from an axis north to south
        let lats: Vec<f64> = axis(89.979_166, -1.0 / 24.0, 4320);
        let r = lat_rows(&lats, -42.0, -40.0, 1);
        assert!(lats[r[0]] >= -42.0 && lats[r[0]] < -41.95);
        assert!(lats[r[r.len() - 1]] <= -40.0 && lats[r[r.len() - 1]] > -40.05);
        assert!(lats[r[1]] > lats[r[0]]);
    }

    #[test]
    fn chunk_choice() {
        let mk = |chunks: Vec<usize>| Array { shape: vec![6581, 50, 2041, 4320], chunks, dtype: "f4".into(), compressor: None, fill: None, scale: 1.0, offset: 0.0, dims: vec![], sep: ".".into(), units: String::new() };
        let rows: Vec<usize> = (1440..1476).collect();
        let times: Vec<usize> = (6500..6540).collect();
        let w = Window { times: &times, level: Some(49), rows: &rows, col0: 4224, ncols: 37, k: 1, nlon: 4320 };
        let t = mk(vec![1, 1, 512, 2048]);
        let g = mk(vec![526, 1, 32, 32]);
        // 40 times of a small area: the geo chunks (a few, holding all the times) are cheaper
        assert_eq!(chunks_for(&t, &w).len(), 40);
        assert!(window_cost(&g, &w) < window_cost(&t, &w));
        // one time of a wide area: the time chunks
        let wide: Vec<usize> = (1000..1500).collect();
        let w1 = Window { times: &times[..1], level: Some(49), rows: &wide, col0: 3000, ncols: 1000, k: 1, nlon: 4320 };
        assert!(window_cost(&t, &w1) < window_cost(&g, &w1));
    }

    #[test]
    fn times_and_levels() {
        let h = 3600;
        let times: Vec<i64> = (0..40).map(|i| i * 6 * h).collect();
        assert_eq!(pick_times(&times, 12 * h, 48 * h, 12 * h), vec![2, 4, 6, 8]);
        assert_eq!(pick_times(&times, 12 * h, 30 * h, 6 * h), vec![2, 3, 4, 5]);
        let elev = [-5727.9, -100.0, -47.2, -40.3, -2.6, -0.494];
        assert_eq!(nearest_level(&elev, 0.0), Some(5));
        assert_eq!(nearest_level(&elev, 50.0), Some(2));
        assert_eq!(nearest_level(&elev, 42.0), Some(3));
        // the stack: shallowest first, nothing below 500 m
        assert_eq!(stack_levels(&elev), vec![5, 4, 3, 2, 1]);
    }

    #[test]
    fn download_size() {
        let req = |model, params: &[&str], hours, every| Request { model, area: [172.0, -42.0, 176.0, -38.0], params: params.iter().map(|s| s.to_string()).collect(), hours, every, depth: 50.0 };
        // 5 days of 6-hourly surface temperature round Cook Strait: the geo chunks, tens of MB at most
        let b = download_estimate(&req(Model::CmemsPhy, &["sst"], 120, 6));
        assert!(b > 1_000_000 && b < 60_000_000, "{b}");
        // one day of chlorophyll at 4 km: a chunk or two
        let c = download_estimate(&req(Model::CmemsChl, &["chl"], 24, 24));
        assert!(c > 100_000 && c < 10_000_000, "{c}");
    }

    #[test]
    fn what_is_read() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let v = vars_for(Model::CmemsPhy, &p(&["sst", "current", "depth"]));
        assert_eq!(v.iter().map(|x| x.name).collect::<Vec<_>>(), vec!["thetao", "uo", "vo", "thetao", "so", "uo", "vo"]);
        assert!(v[3].at_depth && v[3].param == Param::SubTemp);
        let s = vars_for(Model::CmemsPhy, &p(&["stack", "depth"]));
        assert_eq!(s.iter().map(|x| x.param).collect::<Vec<_>>(), vec![Param::SubTemp, Param::SubSalinity, Param::SubCurrentU, Param::SubCurrentV]);
        assert!(s.iter().all(|x| x.stack));
        assert_eq!(product_of(BGC_BIO), BGC);
        assert_eq!(product_of(OC_CHL), OC);
        assert_eq!(product_of(PHY_TOTAL), PHY);
    }
}
