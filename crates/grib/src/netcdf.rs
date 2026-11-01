//! A small reader for NetCDF-3 files (classic and 64-bit offset), the format
//! ERDDAP and THREDDS' NetCDF Subset Service send when asked for "netcdf".
//! Enough to pull gridded variables out (sea temperature, salinity, currents),
//! unpacked: `scale_factor`, `add_offset`, `_FillValue` and `missing_value`
//! are applied, missing values come back as NaN.
//!
//! The format: "CDF", a version byte, the record count, then the dimension,
//! global attribute and variable lists, then the data. Fixed-size variables are
//! stored whole at their offset; variables whose first dimension is the record
//! (unlimited) dimension are interleaved one record at a time.

use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum NcError {
    #[error("not a NetCDF-3 file: {0}")]
    Format(String),
    #[error("no variable {0} in the file")]
    NoVar(String),
}

#[derive(Debug, Clone)]
pub struct Dim {
    pub name: String,
    /// Length; for the record dimension, the number of records.
    pub len: usize,
    pub record: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Text(String),
    Numbers(Vec<f64>),
}

impl AttrValue {
    pub fn num(&self) -> Option<f64> {
        match self {
            AttrValue::Numbers(v) => v.first().copied(),
            AttrValue::Text(t) => t.trim().parse().ok(),
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            AttrValue::Text(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Var {
    pub name: String,
    pub dims: Vec<usize>,
    pub attrs: HashMap<String, AttrValue>,
    nc_type: u32,
    vsize: u64,
    begin: u64,
}

impl Var {
    pub fn attr_num(&self, k: &str) -> Option<f64> {
        self.attrs.get(k).and_then(|a| a.num())
    }
    pub fn attr_text(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).and_then(|a| a.text())
    }
}

pub struct NcFile<'a> {
    bytes: &'a [u8],
    pub dims: Vec<Dim>,
    pub attrs: HashMap<String, AttrValue>,
    pub vars: Vec<Var>,
    recsize: u64,
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn need(&self, n: usize) -> Result<(), NcError> {
        if self.p + n > self.b.len() {
            Err(NcError::Format("header runs past the end".into()))
        } else {
            Ok(())
        }
    }
    fn u32(&mut self) -> Result<u32, NcError> {
        self.need(4)?;
        let v = u32::from_be_bytes(self.b[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        Ok(v)
    }
    fn u64(&mut self) -> Result<u64, NcError> {
        self.need(8)?;
        let v = u64::from_be_bytes(self.b[self.p..self.p + 8].try_into().unwrap());
        self.p += 8;
        Ok(v)
    }
    fn bytes(&mut self, n: usize) -> Result<&[u8], NcError> {
        self.need(n)?;
        let s = &self.b[self.p..self.p + n];
        self.p += n.div_ceil(4) * 4; // padded to 4 bytes
        if self.p > self.b.len() {
            self.p = self.b.len();
        }
        Ok(s)
    }
    fn name(&mut self) -> Result<String, NcError> {
        let n = self.u32()? as usize;
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }
}

fn type_size(t: u32) -> Result<usize, NcError> {
    Ok(match t {
        1 | 2 => 1,
        3 => 2,
        4 | 5 => 4,
        6 => 8,
        _ => return Err(NcError::Format(format!("data type {t}"))),
    })
}

fn decode(t: u32, b: &[u8]) -> f64 {
    match t {
        1 => b[0] as i8 as f64,
        2 => b[0] as f64,
        3 => i16::from_be_bytes([b[0], b[1]]) as f64,
        4 => i32::from_be_bytes(b[..4].try_into().unwrap()) as f64,
        5 => f32::from_be_bytes(b[..4].try_into().unwrap()) as f64,
        6 => f64::from_be_bytes(b[..8].try_into().unwrap()),
        _ => f64::NAN,
    }
}

fn attrs(r: &mut Rd) -> Result<HashMap<String, AttrValue>, NcError> {
    let mut out = HashMap::new();
    let tag = r.u32()?;
    let n = r.u32()?;
    if tag == 0 {
        return Ok(out);
    }
    if tag != 0x0C {
        return Err(NcError::Format("attribute list".into()));
    }
    for _ in 0..n {
        let name = r.name()?;
        let t = r.u32()?;
        let count = r.u32()? as usize;
        let size = type_size(t)?;
        let raw = r.bytes(count * size)?;
        let v = if t == 2 {
            AttrValue::Text(String::from_utf8_lossy(raw).trim_end_matches('\0').to_string())
        } else {
            AttrValue::Numbers(raw.chunks(size).map(|c| decode(t, c)).collect())
        };
        out.insert(name, v);
    }
    Ok(out)
}

impl<'a> NcFile<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NcError> {
        if bytes.len() < 8 || &bytes[..3] != b"CDF" {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(160)]).replace('\n', " ");
            return Err(NcError::Format(if bytes.starts_with(b"\x89HDF") { "NetCDF-4 (HDF5), not NetCDF-3".into() } else { head }));
        }
        let version = bytes[3];
        if version != 1 && version != 2 {
            return Err(NcError::Format(format!("version {version}")));
        }
        let mut r = Rd { b: bytes, p: 4 };
        let numrecs = r.u32()?;
        // dimensions
        let mut dims = Vec::new();
        let tag = r.u32()?;
        let n = r.u32()?;
        if tag == 0x0A {
            for _ in 0..n {
                let name = r.name()?;
                let len = r.u32()? as usize;
                dims.push(Dim { name, len, record: len == 0 });
            }
        } else if tag != 0 {
            return Err(NcError::Format("dimension list".into()));
        }
        let gatts = attrs(&mut r)?;
        // variables
        let mut vars = Vec::new();
        let tag = r.u32()?;
        let n = r.u32()?;
        if tag == 0x0B {
            for _ in 0..n {
                let name = r.name()?;
                let nd = r.u32()? as usize;
                let mut vd = Vec::with_capacity(nd);
                for _ in 0..nd {
                    vd.push(r.u32()? as usize);
                }
                let va = attrs(&mut r)?;
                let nc_type = r.u32()?;
                let vsize = r.u32()? as u64;
                let begin = if version == 1 { r.u32()? as u64 } else { r.u64()? };
                vars.push(Var { name, dims: vd, attrs: va, nc_type, vsize, begin });
            }
        } else if tag != 0 {
            return Err(NcError::Format("variable list".into()));
        }
        let is_rec = |v: &Var| v.dims.first().is_some_and(|&d| dims.get(d).is_some_and(|x: &Dim| x.record));
        // the record count: from the header, or (streamed files) from the file length
        let rec_vars: Vec<&Var> = vars.iter().filter(|v| is_rec(v)).collect();
        let mut recsize: u64 = rec_vars.iter().map(|v| v.vsize).sum();
        // a single record variable of a small type isn't padded
        if rec_vars.len() == 1 {
            let v = rec_vars[0];
            if let Ok(sz) = type_size(v.nc_type) {
                let n: usize = v.dims[1..].iter().map(|&d| dims[d].len).product();
                recsize = (n * sz) as u64;
            }
        }
        let nrec = if numrecs == u32::MAX {
            let first = rec_vars.iter().map(|v| v.begin).min().unwrap_or(0);
            (bytes.len() as u64).saturating_sub(first).checked_div(recsize).unwrap_or(0) as usize
        } else {
            numrecs as usize
        };
        for d in dims.iter_mut().filter(|d| d.record) {
            d.len = nrec;
        }
        Ok(NcFile { bytes, dims, attrs: gatts, vars, recsize })
    }

    pub fn var(&self, name: &str) -> Option<&Var> {
        self.vars.iter().find(|v| v.name == name)
    }

    /// The lengths of a variable's dimensions.
    pub fn shape(&self, v: &Var) -> Vec<usize> {
        v.dims.iter().map(|&d| self.dims[d].len).collect()
    }

    pub fn dim_names(&self, v: &Var) -> Vec<&str> {
        v.dims.iter().map(|&d| self.dims[d].name.as_str()).collect()
    }

    /// All of a variable's values, row-major (last dimension fastest), unpacked
    /// with NaN for missing.
    pub fn read(&self, name: &str) -> Result<Vec<f64>, NcError> {
        let v = self.var(name).ok_or_else(|| NcError::NoVar(name.into()))?;
        let size = type_size(v.nc_type)?;
        let shape = self.shape(v);
        let record = v.dims.first().is_some_and(|&d| self.dims[d].record);
        let per: usize = if record { shape[1..].iter().product() } else { shape.iter().product() };
        let nrec = if record { shape[0] } else { 1 };
        let fill = v.attr_num("_FillValue");
        let missing = v.attr_num("missing_value");
        let scale = v.attr_num("scale_factor").unwrap_or(1.0);
        let offset = v.attr_num("add_offset").unwrap_or(0.0);
        let (vmin, vmax) = match v.attrs.get("valid_range") {
            Some(AttrValue::Numbers(r)) if r.len() == 2 => (Some(r[0]), Some(r[1])),
            _ => (v.attr_num("valid_min"), v.attr_num("valid_max")),
        };
        let mut out = Vec::with_capacity(per * nrec);
        for rec in 0..nrec {
            let start = v.begin as usize + if record { rec * self.recsize as usize } else { 0 };
            let end = start + per * size;
            if end > self.bytes.len() {
                return Err(NcError::Format(format!("{name}: data runs past the end of the file")));
            }
            for c in self.bytes[start..end].chunks(size) {
                let raw = decode(v.nc_type, c);
                let bad = fill.is_some_and(|f| raw == f)
                    || missing.is_some_and(|m| raw == m)
                    || vmin.is_some_and(|m| raw < m)
                    || vmax.is_some_and(|m| raw > m)
                    || (v.nc_type == 5 && raw.abs() > 1e30)
                    || (v.nc_type == 6 && raw.abs() > 1e300);
                out.push(if bad || !raw.is_finite() { f64::NAN } else { raw * scale + offset });
            }
        }
        Ok(out)
    }
}

/// Seconds since 1970 for a CF time `value` in `units` ("hours since 2000-01-01 00:00:00").
pub fn cf_time(value: f64, units: &str) -> Option<i64> {
    let u = units.trim();
    let (unit, since) = u.split_once(" since ")?;
    let mul = match unit.trim().to_ascii_lowercase().as_str() {
        "seconds" | "second" | "secs" | "sec" | "s" => 1.0,
        "minutes" | "minute" | "mins" | "min" => 60.0,
        "hours" | "hour" | "hrs" | "hr" | "h" => 3600.0,
        "days" | "day" | "d" => 86400.0,
        _ => return None,
    };
    let base = parse_date(since.trim())?;
    Some(base + (value * mul).round() as i64)
}

/// "2000-01-01 00:00:00", "1970-01-01T00:00:00Z", "1981-01-01": seconds since 1970 (UTC).
fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim_end_matches('Z').replace('T', " ");
    let mut parts = s.split_whitespace();
    let date = parts.next()?;
    let time = parts.next().unwrap_or("0:0:0");
    let d: Vec<i64> = date.split('-').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<f64> = time.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    if d.len() != 3 {
        return None;
    }
    let days = days_from_civil(d[0], d[1], d[2]);
    let secs = t.first().copied().unwrap_or(0.0) * 3600.0 + t.get(1).copied().unwrap_or(0.0) * 60.0 + t.get(2).copied().unwrap_or(0.0);
    Some(days * 86400 + secs.round() as i64)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// For tests (here and in other crates).
#[doc(hidden)]
pub mod testdata {
    /// A NetCDF-3 classic file: time (record, 2) × lat (3) × lon (2), `sst`
    /// packed as shorts with scale/offset and a fill value, plus `u` as floats.
    pub fn sample() -> Vec<u8> {
        let mut h: Vec<u8> = b"CDF\x01".to_vec();
        let be = |v: u32| v.to_be_bytes();
        let name = |s: &str| {
            let mut b = be(s.len() as u32).to_vec();
            b.extend(s.as_bytes());
            while b.len() % 4 != 0 {
                b.push(0);
            }
            b
        };
        h.extend(be(2)); // numrecs
        h.extend(be(0x0A));
        h.extend(be(3));
        for (n, l) in [("time", 0u32), ("lat", 3), ("lon", 2)] {
            h.extend(name(n));
            h.extend(be(l));
        }
        h.extend([0u8; 8]); // no global attributes
        h.extend(be(0x0B));
        h.extend(be(4));
        // the variable entries; offsets filled in after
        let text_attr = |k: &str, v: &str| {
            let mut b = name(k);
            b.extend(be(2));
            b.extend(be(v.len() as u32));
            b.extend(v.as_bytes());
            while b.len() % 4 != 0 {
                b.push(0);
            }
            b
        };
        let mut entries: Vec<(Vec<u8>, usize)> = Vec::new(); // entry, where its begin goes
        let mut push_var = |n: &str, dims: &[u32], atts: Vec<Vec<u8>>, t: u32, vsize: u32| {
            let mut b = name(n);
            b.extend(be(dims.len() as u32));
            for d in dims {
                b.extend(be(*d));
            }
            if atts.is_empty() {
                b.extend([0u8; 8]);
            } else {
                b.extend(be(0x0C));
                b.extend(be(atts.len() as u32));
                for a in atts {
                    b.extend(a);
                }
            }
            b.extend(be(t));
            b.extend(be(vsize));
            let at = b.len();
            b.extend(be(0));
            entries.push((b, at));
        };
        push_var("time", &[0], vec![text_attr("units", "hours since 2026-10-18 00:00:00")], 6, 8);
        push_var("lat", &[1], vec![], 5, 12);
        push_var("lon", &[2], vec![], 5, 8);
        let mut sst_atts = vec![text_attr("units", "degC")];
        for (k, t, v) in [("scale_factor", 5u32, 0.01f32.to_be_bytes().to_vec()), ("add_offset", 5, 15.0f32.to_be_bytes().to_vec())] {
            let mut b = name(k);
            b.extend(be(t));
            b.extend(be(1));
            b.extend(v);
            sst_atts.push(b);
        }
        let mut f = name("_FillValue");
        f.extend(be(3));
        f.extend(be(1));
        f.extend((-30000i16).to_be_bytes());
        f.extend([0, 0]);
        sst_atts.push(f);
        push_var("sst", &[0, 1, 2], sst_atts, 3, 12);
        let header_len = h.len() + entries.iter().map(|e| e.0.len()).sum::<usize>();
        // fixed variables: lat then lon, then records of [time, sst]
        let lat_at = header_len;
        let lon_at = lat_at + 12;
        let rec_at = lon_at + 8;
        let begins = [rec_at, lat_at, lon_at, rec_at + 8];
        for (k, (mut e, at)) in entries.into_iter().enumerate() {
            e[at..at + 4].copy_from_slice(&be(begins[k] as u32));
            h.extend(e);
        }
        for v in [-41.0f32, -40.9, -40.8] {
            h.extend(v.to_be_bytes());
        }
        for v in [173.0f32, 173.1] {
            h.extend(v.to_be_bytes());
        }
        for rec in 0..2 {
            h.extend((rec as f64 * 3.0).to_be_bytes());
            for k in 0..6i16 {
                let raw: i16 = if rec == 1 && k == 5 { -30000 } else { k * 10 + rec * 100 };
                h.extend(raw.to_be_bytes());
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::testdata::sample;
    use super::*;

    #[test]
    fn reads_records_packing_and_fill() {
        let b = sample();
        let nc = NcFile::parse(&b).unwrap();
        assert_eq!(nc.dims[0].len, 2);
        let sst = nc.read("sst").unwrap();
        assert_eq!(sst.len(), 12);
        assert!((sst[0] - 15.0).abs() < 1e-6 && (sst[1] - 15.1).abs() < 1e-6);
        assert!((sst[6] - 16.0).abs() < 1e-6);
        assert!(sst[11].is_nan());
        let t = nc.read("time").unwrap();
        let units = nc.var("time").unwrap().attr_text("units").unwrap();
        assert_eq!(cf_time(t[1], units), Some(cf_time(0.0, units).unwrap() + 3 * 3600));
        assert_eq!(cf_time(0.0, "seconds since 1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(cf_time(1.0, "days since 2026-10-18"), Some(1_792_281_600 + 86_400));
        assert_eq!(nc.read("lat").unwrap()[2] as f32, -40.8);
    }
}
