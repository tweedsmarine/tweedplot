//! Ocean data: sea temperature, salinity and currents, downloaded as NetCDF
//! cut to the area on the server and written as GRIB2, so they load, shade,
//! read out and animate like any weather GRIB.
//!
//! * **MUR SST** (NASA JPL, via NOAA's ERDDAP): daily, about 1 km, gap-free
//!   analysis of sea surface temperature from all the satellites. Free to use
//!   and redistribute (PO.DAAC policy). A day or two old when published.
//!
//!   `https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.nc?analysed_sst[last-2:1:last][(S):k:(N)][(W):k:(E)]`
//!
//! * **HYCOM ESPC-D-V02** (US Navy global ocean model, 1/12°, about 9 km):
//!   hourly surface temperature, salinity and currents, and 3-hourly
//!   temperature and salinity at depth, from the latest analysis out to an
//!   8-day forecast, via the NetCDF Subset Service (FMRC "best" series).
//!   Public release, unlimited distribution, no guarantee of service.
//!
//!   `https://ncss.hycom.org/thredds/ncss/FMRC_ESPC-D-V02_ice/FMRC_ESPC-D-V02_ice_best.ncd?var=sst&var=sss&…&accept=netcdf`
//!
//! * **Himawari-9 AHI** (JMA satellite, ACSPO L3C by NOAA STAR, via CoastWatch
//!   THREDDS): hourly sea surface (sub-skin) temperature at 0.02° (about 2 km)
//!   where the sky is clear, two or three hours after the hour. One file an
//!   hour, cut to the area by NCSS; only "best quality" (level 5) pixels kept.
//!   GHRSST data: free and open.
//!
//!   `https://coastwatch.noaa.gov/thredds/ncss/grid/gridH09AHINRTL3CWW00/2026/291/20261018030000-STAR-L3C_GHRSST-SSTsubskin-AHI_H09-ACSPO_V2.90-v02.0-fv01.0.nc?var=sea_surface_temperature&var=quality_level&north=…&accept=netcdf`
//!
//! * **Chlorophyll-a** (NOAA CoastWatch, VIIRS on S-NPP and NOAA-20, gap-filled
//!   by DINEOF): daily, 0.083° (about 9 km), near real time (about three days
//!   old). Free to use and redistribute.
//!
//!   `https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPN20VIIRSDINEOFDaily.nc?chlor_a[last-2:1:last][0][(N):1:(S)][(W):1:(E)]`

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use grib::netcdf::{cf_time, NcFile};
use grib::{Field, Grid, Param};

use crate::{client, get_retry, Error, Model, Progress, Request, Result};

/// HYCOM's subset server is slow (half a minute or more a request): a long timeout.
fn slow_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("TweedPlot/", env!("CARGO_PKG_VERSION"), " (marine chart plotter; +https://tweedsmarine.nz)"))
        .timeout(std::time::Duration::from_secs(420))
        .build()
        .map_err(|e| Error::Http(e.to_string()))
}

pub const ERDDAP: &str = "https://coastwatch.pfeg.noaa.gov/erddap/griddap";
pub const HYCOM_NCSS: &str = "https://ncss.hycom.org/thredds/ncss";
pub const COASTWATCH: &str = "https://coastwatch.noaa.gov";
const HIMAWARI_DS: &str = "gridH09AHINRTL3CWW00";
const CHL_DS: &str = "noaacwNPPN20VIIRSDINEOFDaily";
/// Most points along each side of a Himawari request (0.02°: a 16° box comes every 2nd).
const HIM_MAX_SIDE: f64 = 800.0;
/// The quality level kept (5 = clear sky, "recommended for operational use").
pub const HIM_MIN_QUALITY: f64 = 5.0;

/// Most points along each side of a MUR request (it is 0.01°: a 12° box comes
/// down at 1 km × 10, every 10th point).
const MUR_MAX_SIDE: f64 = 1200.0;

fn area_ok(a: [f64; 4]) -> Result<[f64; 4]> {
    let [w, s, e, n] = a;
    if !(n > s) || !(-90.0..=90.0).contains(&s) || !(-90.0..=90.0).contains(&n) {
        return Err(Error::Bad("the area is not valid".into()));
    }
    // both servers want west < east in -180..180: an area across the date line isn't offered yet
    let w2 = (w + 180.0).rem_euclid(360.0) - 180.0;
    let mut e2 = (e + 180.0).rem_euclid(360.0) - 180.0;
    if e2 <= w2 {
        if (e - w - 360.0).abs() < 1e-6 || e - w >= 359.0 {
            e2 = 180.0;
        } else {
            return Err(Error::Bad("an area across the date line (180°) isn't available for ocean data yet: split it at 180°".into()));
        }
    }
    Ok([w2, s, e2, n])
}

/// Every how many MUR points to take so the request stays a sensible size.
pub fn mur_stride(area: [f64; 4]) -> usize {
    let side = ((area[2] - area[0]).abs().max((area[3] - area[1]).abs()) / 0.01).max(1.0);
    (side / MUR_MAX_SIDE).ceil().max(1.0) as usize
}

/// The ERDDAP request for the last `days` daily analyses.
pub fn mur_url(area: [f64; 4], days: u32) -> Result<String> {
    let [w, s, e, n] = area_ok(area)?;
    let k = mur_stride(area);
    let days = days.clamp(1, 14);
    Ok(format!("{ERDDAP}/jplMURSST41.nc?analysed_sst%5Blast-{}:1:last%5D%5B({s:.3}):{k}:({n:.3})%5D%5B({w:.3}):{k}:({e:.3})%5D", days - 1))
}

/// The ERDDAP request for the last `days` daily chlorophyll analyses (latitude
/// runs north to south in this dataset).
pub fn chl_url(area: [f64; 4], days: u32) -> Result<String> {
    let [w, s, e, n] = area_ok(area)?;
    let days = days.clamp(1, 14);
    Ok(format!("{COASTWATCH}/erddap/griddap/{CHL_DS}.nc?chlor_a%5Blast-{}:1:last%5D%5B0%5D%5B({n:.3}):1:({s:.3})%5D%5B({w:.3}):1:({e:.3})%5D", days - 1))
}

/// The THREDDS catalogue of one day's Himawari files.
pub fn himawari_catalog_url(t: i64) -> String {
    let d = chrono::DateTime::from_timestamp(t, 0).unwrap_or_default();
    format!("{COASTWATCH}/thredds/catalog/{HIMAWARI_DS}/{}/catalog.xml", d.format("%Y/%j"))
}

/// The hourly files a catalogue lists: (hour as Unix time, file name).
pub fn himawari_files(catalog: &str) -> Vec<(i64, String)> {
    let mut out = Vec::new();
    for part in catalog.split("name=\"").skip(1) {
        let Some(name) = part.split('"').next() else { continue };
        if !name.ends_with(".nc") || !name.contains("AHI_H09") || name.len() < 14 {
            continue;
        }
        if let Ok(t) = chrono::NaiveDateTime::parse_from_str(&name[..14], "%Y%m%d%H%M%S") {
            out.push((t.and_utc().timestamp(), name.to_string()));
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn him_stride(area: [f64; 4]) -> usize {
    let side = ((area[2] - area[0]).abs().max((area[3] - area[1]).abs()) / 0.02).max(1.0);
    (side / HIM_MAX_SIDE).ceil().max(1.0) as usize
}

/// The NCSS request for one hourly Himawari file, cut to the area.
pub fn himawari_url(file: &str, area: [f64; 4]) -> Result<String> {
    let [w, s, e, n] = area_ok(area)?;
    let t = chrono::NaiveDateTime::parse_from_str(file.get(..14).unwrap_or(""), "%Y%m%d%H%M%S").map_err(|_| Error::Bad(format!("not a Himawari file: {file}")))?;
    Ok(format!(
        "{COASTWATCH}/thredds/ncss/grid/{HIMAWARI_DS}/{}/{file}?var=sea_surface_temperature&var=quality_level&north={n:.3}&west={w:.3}&east={e:.3}&south={s:.3}&horizStride={}&accept=netcdf",
        t.format("%Y/%j"),
        him_stride(area)
    ))
}

/// Each hour shows the latest clear sighting of every point within the hours
/// downloaded: a gap under cloud keeps the last value seen there (fields in time order).
pub fn fill_forward(fields: &mut [Field]) {
    for k in 1..fields.len() {
        let (a, b) = fields.split_at_mut(k);
        let prev = &a[k - 1];
        let f = &mut b[0];
        if prev.grid != f.grid || prev.param != f.param {
            continue;
        }
        for (v, p) in f.values.iter_mut().zip(&prev.values) {
            if v.is_nan() {
                *v = *p;
            }
        }
    }
}

/// HYCOM: which aggregation, which variables.
fn hycom_parts(params: &[String]) -> (Vec<&'static str>, bool) {
    let has = |p: &str| params.iter().any(|x| x == p);
    let mut surf = Vec::new();
    if has("sst") {
        surf.push("sst");
    }
    if has("sss") {
        surf.push("sss");
    }
    if has("current") {
        surf.extend(["ssu", "ssv"]);
    }
    (surf, has("depth"))
}

/// HYCOM's separate hourly water surface elevation dataset.
fn hycom_ssh(params: &[String]) -> bool {
    params.iter().any(|x| x == "ssh")
}

fn iso(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// The NCSS request for one HYCOM aggregation ("ice": the hourly 2-D surface
/// fields; "ts3z": 3-hourly temperature and salinity at depths).
pub fn hycom_url(dataset: &str, vars: &[&str], area: [f64; 4], t0: i64, hours: u32, every: u32, depth: Option<f64>) -> Result<String> {
    let [w, s, e, n] = area_ok(area)?;
    let native = if dataset == "ice" || dataset == "ssh" { 1 } else { 3 };
    let stride = (every.max(native) / native).max(1);
    let vars: String = vars.iter().map(|v| format!("var={v}&")).collect();
    let vert = depth.map(|d| format!("&vertCoord={d}")).unwrap_or_default();
    Ok(format!(
        "{HYCOM_NCSS}/FMRC_ESPC-D-V02_{dataset}/FMRC_ESPC-D-V02_{dataset}_best.ncd?{vars}north={n:.3}&west={w:.3}&east={e:.3}&south={s:.3}&horizStride=1&time_start={}&time_end={}&timeStride={stride}{vert}&accept=netcdf",
        iso(t0),
        iso(t0 + hours as i64 * 3600)
    ))
}

/// What a variable becomes in GRIB2 terms.
struct Kind {
    param: Param,
    category: u8,
    number: u8,
    surface_type: u8,
    kelvin: bool,
}

fn kind_of(var: &str, depth: Option<f64>) -> Option<Kind> {
    let sub = depth.is_some();
    Some(match var {
        "analysed_sst" | "sst" => Kind { param: Param::SeaTemp, category: 3, number: 0, surface_type: 1, kelvin: true },
        "sss" => Kind { param: Param::Salinity, category: 4, number: 21, surface_type: 1, kelvin: false },
        "water_temp" if sub => Kind { param: Param::SubTemp, category: 3, number: 0, surface_type: 160, kelvin: true },
        "water_temp" => Kind { param: Param::SeaTemp, category: 3, number: 0, surface_type: 1, kelvin: true },
        "salinity" if sub => Kind { param: Param::SubSalinity, category: 4, number: 21, surface_type: 160, kelvin: false },
        "salinity" => Kind { param: Param::Salinity, category: 4, number: 21, surface_type: 1, kelvin: false },
        "ssu" | "water_u" => Kind { param: Param::CurrentU, category: 1, number: 2, surface_type: 1, kelvin: false },
        "ssv" | "water_v" => Kind { param: Param::CurrentV, category: 1, number: 3, surface_type: 1, kelvin: false },
        "sea_surface_temperature" => Kind { param: Param::SkinTemp, category: 3, number: 192, surface_type: 1, kelvin: true },
        "chlor_a" => Kind { param: Param::Chlorophyll, category: 4, number: 192, surface_type: 1, kelvin: false },
        "surf_el" => Kind { param: Param::SeaLevel, category: 3, number: 1, surface_type: 1, kelvin: false },
        _ => return None,
    })
}

/// The fields of a NetCDF-3 grid file: every variable `kind_of` knows, every
/// time (depth: the first level, which is the one asked for).
pub fn fields_from_netcdf(bytes: &[u8], centre: u16, depth: Option<f64>) -> Result<Vec<Field>> {
    let nc = NcFile::parse(bytes).map_err(|e| Error::Bad(e.to_string()))?;
    let find = |names: &[&str]| nc.vars.iter().find(|v| names.contains(&v.name.as_str())).map(|v| v.name.clone());
    let lat_n = find(&["lat", "latitude"]).ok_or_else(|| Error::Bad("no latitude in the file".into()))?;
    let lon_n = find(&["lon", "longitude"]).ok_or_else(|| Error::Bad("no longitude in the file".into()))?;
    let time_n = find(&["time"]).ok_or_else(|| Error::Bad("no time in the file".into()))?;
    let rd = |n: &str| nc.read(n).map_err(|e| Error::Bad(e.to_string()));
    let lats = rd(&lat_n)?;
    let lons = rd(&lon_n)?;
    let times_raw = rd(&time_n)?;
    let units = nc.var(&time_n).and_then(|v| v.attr_text("units")).unwrap_or("seconds since 1970-01-01").to_string();
    let times: Vec<i64> = times_raw.iter().map(|&t| cf_time(t, &units).ok_or_else(|| Error::Bad(format!("time units “{units}”")))).collect::<Result<_>>()?;
    if lats.len() < 2 || lons.len() < 2 || times.is_empty() {
        return Err(Error::Bad("the file has no grid in it (is the area at sea?)".into()));
    }
    let dlat = (lats[lats.len() - 1] - lats[0]) / (lats.len() - 1) as f64;
    let dlon = (lons[lons.len() - 1] - lons[0]) / (lons.len() - 1) as f64;
    let north_first = dlat < 0.0;
    let (ni, nj) = (lons.len(), lats.len());
    let lat0 = lats.iter().cloned().fold(f64::MAX, f64::min);
    let grid = Grid::regular(ni, nj, lat0, (lons[0] + 180.0).rem_euclid(360.0) - 180.0, dlat.abs(), dlon.abs());
    let ref_time = times[0];
    // satellite temperatures: only the pixels of the quality kept
    let quality = if nc.var("quality_level").is_some() { Some(rd("quality_level")?) } else { None };
    let mut out = Vec::new();
    for v in &nc.vars {
        let Some(k) = kind_of(&v.name, depth) else { continue };
        let names = nc.dim_names(v);
        let shape = nc.shape(v);
        // [time, (depth,) lat, lon]
        if names.len() < 3 || shape[names.len() - 1] != ni || shape[names.len() - 2] != nj {
            continue;
        }
        let vals = rd(&v.name)?;
        let units = v.attr_text("units").unwrap_or("").to_ascii_lowercase();
        let to_k = k.kelvin && !(units == "k" || units.contains("kelvin"));
        let per_level = ni * nj;
        let levels: usize = shape[1..names.len() - 2].iter().product::<usize>().max(1);
        for (ti, &t) in times.iter().enumerate().take(shape[0]) {
            let base = ti * levels * per_level; // first level
            let mut values = vec![f32::NAN; per_level];
            for j in 0..nj {
                let src_row = if north_first { nj - 1 - j } else { j };
                for i in 0..ni {
                    let x = vals[base + src_row * ni + i];
                    let good = match (&quality, k.param) {
                        (Some(q), Param::SkinTemp) => q.get(base + src_row * ni + i).is_some_and(|&q| q >= HIM_MIN_QUALITY),
                        _ => true,
                    };
                    if x.is_finite() && good {
                        values[j * ni + i] = (if to_k { x + 273.15 } else { x }) as f32;
                    }
                }
            }
            if values.iter().all(|x| x.is_nan()) {
                continue;
            }
            out.push(Field {
                param: k.param,
                discipline: 10,
                category: k.category,
                number: k.number,
                surface_type: k.surface_type,
                surface_value: if k.surface_type == 160 { depth.unwrap_or(0.0) } else { 0.0 },
                centre,
                ref_time,
                valid_time: t,
                member: None,
                grid,
                values,
                local: None,
            });
        }
    }
    if out.is_empty() {
        return Err(Error::Bad("no sea values in the area (all land, or all cloud?)".into()));
    }
    Ok(out)
}

/// e.g. "MUR_SST_20261015-20261017_42S-40S_172E-175E.grb2", "HYCOM_ESPC_20261019_06z_f000-f192_…grb2"
fn name(prefix: &str, when: &str, area: [f64; 4]) -> String {
    let lat = |v: f64| format!("{}{}", v.abs().round(), if v < 0.0 { "S" } else { "N" });
    let lon = |v: f64| {
        let v = (v + 180.0).rem_euclid(360.0) - 180.0;
        format!("{}{}", v.abs().round(), if v < 0.0 { "W" } else { "E" })
    };
    format!("{prefix}_{when}_{}-{}_{}-{}.grb2", lat(area[1]), lat(area[3]), lon(area[0]), lon(area[2]))
}

fn ymd(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().format("%Y%m%d").to_string()
}

fn write(fields: &[Field], process: u8, path: &Path) -> Result<()> {
    let part = path.with_extension("grb2.part");
    let mut bytes = Vec::new();
    for f in fields {
        bytes.extend(grib::write::encode(f, process));
    }
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)?;
    Ok(())
}

/// Download an ocean request into `dir`; returns the file written.
pub fn download(req: &Request, now: i64, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let c = client()?;
    match req.model {
        Model::MurSst => {
            let days = (req.hours / 24).clamp(1, 14);
            progress(0, 1, 0);
            let url = mur_url(req.area, days)?;
            let data = get_retry(&c, &url, cancel)?;
            progress(1, 1, data.len() as u64);
            let fields = fields_from_netcdf(&data, grib::write::CENTRE_NASA, None)?;
            let (t0, t1) = (fields.iter().map(|f| f.valid_time).min().unwrap(), fields.iter().map(|f| f.valid_time).max().unwrap());
            let path = dir.join(name("MUR_SST", &format!("{}-{}", ymd(t0), ymd(t1)), req.area));
            write(&fields, grib::write::PROCESS_MUR_SST, &path)?;
            Ok(path)
        }
        Model::HycomEspc => {
            let (surf, deep) = hycom_parts(&req.params);
            let ssh = hycom_ssh(&req.params);
            if surf.is_empty() && !deep && !ssh {
                return Err(Error::Bad("choose at least one parameter".into()));
            }
            // from the last whole 3 hours (the analysis) to the forecast length asked
            let t0 = now - now.rem_euclid(3 * 3600) - 3 * 3600;
            let hours = req.hours.min(192);
            // HYCOM asks for no more than a day per request (and its server takes
            // half a minute or more for each): one request per day and dataset
            let days = hours.div_ceil(24).max(1);
            let slow = slow_client()?;
            let mut jobs: Vec<(&str, Vec<&str>, i64, u32, u32, Option<f64>)> = Vec::new();
            for d in 0..days {
                let start = t0 + d as i64 * 86_400;
                let len = (hours - d * 24).min(24);
                if !surf.is_empty() {
                    jobs.push(("ice", surf.clone(), start, len, req.every, None));
                }
                if ssh {
                    jobs.push(("ssh", vec!["surf_el"], start, len, req.every, None));
                }
                if deep {
                    let dep = if req.depth > 0.0 { req.depth } else { 50.0 };
                    jobs.push(("ts3z", vec!["water_temp", "salinity"], start, len, req.every.max(3), Some(dep)));
                }
            }
            let total = jobs.len();
            let mut fields = Vec::new();
            let mut bytes = 0u64;
            progress(0, total, 0);
            for (k, (ds, vars, start, len, every, dep)) in jobs.iter().enumerate() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                // the day's last step only (the next day's request starts on the next one)
                let span = if *len >= 24 { 24 - every.max(&if *ds == "ice" || *ds == "ssh" { 1 } else { 3 }) } else { *len };
                let url = hycom_url(ds, vars, req.area, *start, span, *every, *dep)?;
                let data = get_retry(&slow, &url, cancel)?;
                bytes += data.len() as u64;
                fields.extend(fields_from_netcdf(&data, grib::write::CENTRE_FNMOC, *dep)?);
                progress(k + 1, total, bytes);
            }
            // no time twice (requests meeting at a day boundary)
            fields.sort_by_key(|f| (f.param, f.valid_time));
            fields.dedup_by_key(|f| (f.param, f.valid_time));
            // one reference time for the whole file: the first analysis
            let r = fields.iter().map(|f| f.ref_time).min().unwrap_or(t0);
            for f in fields.iter_mut() {
                f.ref_time = r;
            }
            let last = fields.iter().map(|f| f.valid_time).max().unwrap_or(r);
            let when = chrono::DateTime::from_timestamp(r, 0).unwrap_or_default().format("%Y%m%d_%Hz").to_string();
            let path = dir.join(name("HYCOM_ESPC", &format!("{when}_f000-f{:03}", (last - r) / 3600), req.area));
            write(&fields, grib::write::PROCESS_HYCOM_ESPC, &path)?;
            Ok(path)
        }
        Model::ChlViirs => {
            let days = (req.hours / 24).clamp(1, 14);
            progress(0, 1, 0);
            let data = get_retry(&c, &chl_url(req.area, days)?, cancel)?;
            progress(1, 1, data.len() as u64);
            let fields = fields_from_netcdf(&data, grib::write::CENTRE_NESDIS, None)?;
            let (t0, t1) = (fields.iter().map(|f| f.valid_time).min().unwrap(), fields.iter().map(|f| f.valid_time).max().unwrap());
            let path = dir.join(name("CHL_VIIRS", &format!("{}-{}", ymd(t0), ymd(t1)), req.area));
            write(&fields, grib::write::PROCESS_VIIRS_CHL, &path)?;
            Ok(path)
        }
        Model::Himawari => {
            let hours = req.hours.clamp(1, 72) as i64;
            let every = req.every.clamp(1, 6) as i64;
            let from = now - hours * 3600;
            // the days' catalogues: which hours are on the server
            let mut files = Vec::new();
            let mut day = from - from.rem_euclid(86_400);
            while day <= now {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                if let Ok(cat) = get_retry(&c, &himawari_catalog_url(day), cancel) {
                    files.extend(himawari_files(&String::from_utf8_lossy(&cat)));
                }
                day += 86_400;
            }
            files.retain(|(t, _)| *t >= from && *t <= now && (t / 3600) % every == 0);
            if files.is_empty() {
                return Err(Error::Bad("no Himawari hours on the server for that time yet (they come two or three hours after the hour)".into()));
            }
            let total = files.len();
            progress(0, total, 0);
            let mut fields = Vec::new();
            let mut bytes = 0u64;
            let mut cloudy = 0;
            for (k, (_, file)) in files.iter().enumerate() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                let data = get_retry(&c, &himawari_url(file, req.area)?, cancel)?;
                bytes += data.len() as u64;
                match fields_from_netcdf(&data, grib::write::CENTRE_NESDIS, None) {
                    Ok(f) => fields.extend(f),
                    Err(Error::Bad(_)) => cloudy += 1, // all cloud this hour
                    Err(e) => return Err(e),
                }
                progress(k + 1, total, bytes);
            }
            if fields.is_empty() {
                return Err(Error::Bad(format!("all {cloudy} hours were cloudy over the area: no clear-sky sea temperature")));
            }
            fields.sort_by_key(|f| f.valid_time);
            fill_forward(&mut fields);
            let r = fields[0].valid_time;
            for f in fields.iter_mut() {
                f.ref_time = r;
            }
            let last = fields.iter().map(|f| f.valid_time).max().unwrap_or(r);
            let when = chrono::DateTime::from_timestamp(r, 0).unwrap_or_default().format("%Y%m%d_%Hz").to_string();
            let path = dir.join(name("HIMAWARI_SST", &format!("{when}_{:02}h", (last - r) / 3600 + 1), req.area));
            write(&fields, grib::write::PROCESS_HIMAWARI_SST, &path)?;
            Ok(path)
        }
        _ => Err(Error::Bad("not an ocean model".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let u = mur_url([172.0, -42.0, 175.0, -40.0], 3).unwrap();
        assert_eq!(u, "https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.nc?analysed_sst%5Blast-2:1:last%5D%5B(-42.000):1:(-40.000)%5D%5B(172.000):1:(175.000)%5D");
        // a big area is thinned
        assert_eq!(mur_stride([160.0, -50.0, 180.0, -30.0]), 2);
        let h = hycom_url("ice", &["sst", "sss"], [172.0, -42.0, 175.0, -40.0], 1_792_281_600, 48, 3, None).unwrap();
        assert!(h.starts_with("https://ncss.hycom.org/thredds/ncss/FMRC_ESPC-D-V02_ice/FMRC_ESPC-D-V02_ice_best.ncd?var=sst&var=sss&north=-40.000&west=172.000&east=175.000&south=-42.000"), "{h}");
        assert!(h.contains("time_start=2026-10-18T00:00:00Z&time_end=2026-10-20T00:00:00Z&timeStride=3&accept=netcdf"), "{h}");
        let d = hycom_url("ts3z", &["water_temp"], [172.0, -42.0, 175.0, -40.0], 0, 24, 6, Some(50.0)).unwrap();
        assert!(d.contains("timeStride=2&vertCoord=50&accept=netcdf"), "{d}");
        assert!(area_ok([170.0, -45.0, 190.0, -30.0]).is_err());
    }

    /// Real NCSS output from HYCOM (FMRC_ESPC-D-V02_ice, 20 Oct 2026 00Z and 03Z,
    /// 41.2–40.8°S 173.0–173.5°E): every variable, the times and the grid come through.
    #[test]
    fn reads_real_hycom_output() {
        let b = crate::tests::b64(include_str!("../tests/hycom_ice_nelson.b64"));
        let f = fields_from_netcdf(&b, grib::write::CENTRE_FNMOC, None).unwrap();
        let count = |p: Param| f.iter().filter(|x| x.param == p).count();
        assert_eq!((count(Param::SeaTemp), count(Param::Salinity), count(Param::CurrentU), count(Param::CurrentV)), (2, 2, 2, 2));
        assert_eq!(f[0].valid_time, 1_792_281_600 + 2 * 86_400); // 2026-10-20 00Z
        let g = &f[0].grid;
        assert_eq!((g.ni, g.nj), (8, 11));
        assert!((g.dlat - 0.04).abs() < 1e-4 && (g.dlon - 0.08).abs() < 1e-4, "{g:?}");
        assert!((g.lat0 + 41.2).abs() < 1e-4 && (g.lon0 - 172.96).abs() < 1e-3, "{g:?}");
        // the row at 41.0°S (j = 5): land then 16.36 °C
        let sst = f.iter().find(|x| x.param == Param::SeaTemp).unwrap();
        assert!(sst.values[5 * 8].is_nan());
        assert!((sst.values[5 * 8 + 1] as f64 - (16.361 + 273.15)).abs() < 0.01, "{}", sst.values[5 * 8 + 1]);
        let mut bytes = Vec::new();
        for x in &f {
            bytes.extend(grib::write::encode(x, grib::write::PROCESS_HYCOM_ESPC));
        }
        let gf = grib::GribFile::from_bytes(&bytes);
        assert!(gf.current(-41.0, 173.3, f[0].valid_time).is_some());
    }

    /// Real Himawari-9 NCSS output (18 Oct 2026 03Z, 41.0–40.8°S 173.0–173.3°E):
    /// sub-skin temperature, land and lower-quality pixels left out.
    #[test]
    fn reads_real_himawari_output() {
        let b = crate::tests::b64(include_str!("../tests/himawari_nelson.b64"));
        let f = fields_from_netcdf(&b, grib::write::CENTRE_NESDIS, None).unwrap();
        assert_eq!(f.len(), 1);
        let x = &f[0];
        assert_eq!(x.param, Param::SkinTemp);
        assert_eq!(x.valid_time, 1_792_292_400); // 2026-10-18 03Z
        assert_eq!((x.grid.ni, x.grid.nj), (16, 11));
        assert!((x.grid.dlat - 0.02).abs() < 1e-4 && (x.grid.lat0 + 41.01).abs() < 1e-3, "{:?}", x.grid);
        let ok: Vec<f32> = x.values.iter().cloned().filter(|v| v.is_finite()).collect();
        assert!(ok.len() > 100 && ok.len() < 176, "{}", ok.len());
        // all plausible spring Tasman Bay water (12–18 °C)
        assert!(ok.iter().all(|v| (285.0..291.5).contains(v)), "{ok:?}");
        let mut bytes = Vec::new();
        bytes.extend(grib::write::encode(x, grib::write::PROCESS_HIMAWARI_SST));
        let g = grib::GribFile::from_bytes(&bytes);
        assert_eq!(g.model().0, "Himawari-9 SST hourly");
        assert_eq!(g.times(Param::SkinTemp).len(), 1);
    }

    /// Real chlorophyll output from CoastWatch ERDDAP (15 and 16 Oct 2026).
    #[test]
    fn reads_real_chlorophyll_output() {
        let b = crate::tests::b64(include_str!("../tests/chl_nelson.b64"));
        let f = fields_from_netcdf(&b, grib::write::CENTRE_NESDIS, None).unwrap();
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|x| x.param == Param::Chlorophyll));
        assert_eq!(f[1].valid_time - f[0].valid_time, 86_400);
        let g = &f[0].grid;
        assert_eq!((g.ni, g.nj), (5, 5));
        assert!((g.dlat - 0.08333).abs() < 1e-3, "{g:?}");
        let ok: Vec<f32> = f[0].values.iter().cloned().filter(|v| v.is_finite()).collect();
        assert!(!ok.is_empty() && ok.iter().all(|v| (0.01..50.0).contains(v)), "{ok:?}");
        let mut bytes = Vec::new();
        for x in &f {
            bytes.extend(grib::write::encode(x, grib::write::PROCESS_VIIRS_CHL));
        }
        let gf = grib::GribFile::from_bytes(&bytes);
        assert!(gf.skipped.is_empty(), "{:?}", gf.skipped);
        assert_eq!(gf.times(Param::Chlorophyll).len(), 2);
    }

    #[test]
    fn himawari_catalogue_and_urls() {
        let cat = r#"<dataset name="20261019000000-STAR-L3C_GHRSST-SSTsubskin-AHI_H09-ACSPO_V2.90-v02.0-fv01.0.nc" ID="x"/><dataset name="20261019010000-STAR-L3C_GHRSST-SSTsubskin-AHI_H09-ACSPO_V2.90-v02.0-fv01.0.nc"/><dataset name="latest.xml"/>"#;
        let f = himawari_files(cat);
        assert_eq!(f.len(), 2);
        assert_eq!(f[1].0, 1_792_368_000 + 3600);
        assert_eq!(himawari_catalog_url(1_792_368_000), "https://coastwatch.noaa.gov/thredds/catalog/gridH09AHINRTL3CWW00/2026/292/catalog.xml");
        let u = himawari_url(&f[0].1, [172.5, -41.5, 174.5, -40.5]).unwrap();
        assert!(u.starts_with("https://coastwatch.noaa.gov/thredds/ncss/grid/gridH09AHINRTL3CWW00/2026/292/20261019000000-"), "{u}");
        assert!(u.ends_with("?var=sea_surface_temperature&var=quality_level&north=-40.500&west=172.500&east=174.500&south=-41.500&horizStride=1&accept=netcdf"), "{u}");
        let c = chl_url([172.5, -41.5, 174.5, -40.5], 3).unwrap();
        assert_eq!(c, "https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPN20VIIRSDINEOFDaily.nc?chlor_a%5Blast-2:1:last%5D%5B0%5D%5B(-40.500):1:(-41.500)%5D%5B(172.500):1:(174.500)%5D");
    }

    #[test]
    fn cloud_gaps_keep_the_last_clear_value() {
        let b = crate::tests::b64(include_str!("../tests/himawari_nelson.b64"));
        let a = fields_from_netcdf(&b, grib::write::CENTRE_NESDIS, None).unwrap().remove(0);
        let mut later = a.clone();
        later.valid_time += 3600;
        later.values.iter_mut().take(20).for_each(|v| *v = f32::NAN);
        let mut fs = vec![a.clone(), later];
        fill_forward(&mut fs);
        for i in 0..20 {
            assert!(fs[1].values[i].is_nan() == a.values[i].is_nan());
        }
    }

    #[test]
    fn netcdf_to_grib_round_trip() {
        let b = grib::netcdf_sample_for_tests();
        let fields = fields_from_netcdf(&b, grib::write::CENTRE_FNMOC, None).unwrap();
        assert_eq!(fields.len(), 2);
        let mut bytes = Vec::new();
        for f in &fields {
            bytes.extend(grib::write::encode(f, grib::write::PROCESS_HYCOM_ESPC));
        }
        let g = grib::GribFile::from_bytes(&bytes);
        assert!(g.skipped.is_empty(), "{:?}", g.skipped);
        assert_eq!(g.model().0, "HYCOM ESPC ocean");
        let t = g.times(Param::SeaTemp);
        assert_eq!(t.len(), 2);
        // sample: lat -41.0..-40.8, lon 173.0..173.1; first value 15.0 °C at (-41, 173)
        let v = g.value(Param::SeaTemp, -41.0, 173.0, t[0]).unwrap() - 273.15;
        assert!((v - 15.0).abs() < 0.01, "{v}");
        let v = g.value(Param::SeaTemp, -40.8, 173.1, t[0]).unwrap() - 273.15;
        assert!((v - 15.5).abs() < 0.01, "{v}");
        // a day after the last analysis it still stands (held)
        assert!(g.value(Param::SeaTemp, -41.0, 173.0, t[1] + 86_400).is_some());
    }
}
