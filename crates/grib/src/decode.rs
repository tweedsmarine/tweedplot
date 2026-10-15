//! GRIB edition 2 decoder.
//!
//! Supported: grid template 3.0 (regular lat/lon); product templates 4.0, 4.1,
//! 4.8 and 4.11; data representation 5.0 (simple), 5.2 (complex) and 5.3
//! (complex with spatial differencing); bitmaps. The local-use section (2) is
//! kept as JSON when it parses as JSON (Meltemus puts model and run there).
//! Fields in other representations (JPEG 2000, PNG, CCSDS) are skipped and
//! reported, not fatal.

use crate::{Field, Grid, GribError, Param};

pub struct Decoded {
    pub fields: Vec<Field>,
    /// Messages or fields that could not be decoded, with the reason.
    pub skipped: Vec<String>,
}

pub fn decode(bytes: &[u8]) -> Decoded {
    decode_with_progress(bytes, &|_, _| {})
}

/// Decode every message, in parallel. `progress(done, total)` is called as
/// messages finish (from worker threads).
pub fn decode_with_progress(bytes: &[u8], progress: &(dyn Fn(usize, usize) + Sync)) -> Decoded {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Pass 1 (fast, sequential): find the messages. Meltemus writes the local
    // section only on the first message of each model block, so each message
    // inherits the last one seen.
    let mut msgs: Vec<(usize, usize, Option<serde_json::Value>)> = Vec::new();
    let mut skipped = Vec::new();
    let mut pos = 0;
    let mut local: Option<serde_json::Value> = None;
    while let Some(off) = find(&bytes[pos..], b"GRIB") {
        let start = pos + off;
        if start + 16 > bytes.len() {
            break;
        }
        let edition = bytes[start + 7];
        if edition == 1 {
            let len = u24(&bytes[start + 4..]) as usize;
            skipped.push(format!("GRIB1 message at byte {start} (GRIB1 not supported yet)"));
            pos = start + len.max(8);
            continue;
        }
        if edition != 2 {
            pos = start + 4;
            continue;
        }
        let len = u64be(&bytes[start + 8..]) as usize;
        if len < 20 || start + len > bytes.len() {
            skipped.push(format!("truncated message at byte {start}"));
            break;
        }
        msgs.push((start, len, local.clone()));
        if let Some(l) = local_section(&bytes[start..start + len]) {
            local = l;
        }
        pos = start + len;
    }

    // Pass 2 (parallel): decode.
    let total = msgs.len();
    let done = AtomicUsize::new(0);
    let results: Vec<(usize, Result<(Vec<Field>, Vec<String>), GribError>)> = msgs
        .into_par_iter()
        .map(|(start, len, inherited)| {
            let mut l = inherited;
            let r = message(&bytes[start..start + len], &mut l);
            progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            (start, r)
        })
        .collect();
    let mut fields = Vec::new();
    for (start, r) in results {
        match r {
            Ok((f, s)) => {
                fields.extend(f);
                skipped.extend(s);
            }
            Err(e) => skipped.push(format!("message at byte {start}: {e}")),
        }
    }
    Decoded { fields, skipped }
}

/// The local-use section of a message, if it has one: Some(parsed JSON or None).
fn local_section(m: &[u8]) -> Option<Option<serde_json::Value>> {
    let mut p = 16;
    while p + 5 <= m.len() {
        if &m[p..p + 4] == b"7777" {
            return None;
        }
        let slen = u32be(&m[p..]) as usize;
        let snum = m[p + 4];
        if slen < 5 || p + slen > m.len() || snum > 2 {
            return None; // sections are in order: past 2 means there is none
        }
        if snum == 2 {
            let s = &m[p..p + slen];
            return Some(std::str::from_utf8(&s[5..]).ok().and_then(|t| serde_json::from_str(t.trim_end_matches('\0')).ok()));
        }
        p += slen;
    }
    None
}

#[derive(Default, Clone)]
struct State {
    discipline: u8,
    ref_time: i64,
    centre: u16,
    local: Option<serde_json::Value>,
    grid: Option<Grid>,
    npoints: usize,
    product: Option<Product>,
    repr: Option<(u16, Vec<u8>)>,
    bitmap: Option<Vec<u8>>,
}

#[derive(Clone)]
struct Product {
    category: u8,
    number: u8,
    surface_type: u8,
    surface_value: f64,
    valid_time: i64,
    member: Option<u16>,
}

fn message(m: &[u8], local: &mut Option<serde_json::Value>) -> Result<(Vec<Field>, Vec<String>), GribError> {
    let mut st = State { discipline: m[6], local: local.clone(), ..Default::default() };
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    let mut p = 16;
    while p + 4 <= m.len() {
        if &m[p..p + 4] == b"7777" {
            break;
        }
        if p + 5 > m.len() {
            return Err(GribError::Format("section header past end".into()));
        }
        let slen = u32be(&m[p..]) as usize;
        let snum = m[p + 4];
        if slen < 5 || p + slen > m.len() {
            return Err(GribError::Format(format!("bad length for section {snum}")));
        }
        let s = &m[p..p + slen];
        match snum {
            1 => {
                st.centre = u16be(&s[5..]);
                st.ref_time = unix(u16be(&s[12..]) as i32, s[14], s[15], s[16], s[17], s[18]);
            }
            2 => {
                st.local = std::str::from_utf8(&s[5..]).ok().and_then(|t| serde_json::from_str(t.trim_end_matches('\0')).ok());
                local.clone_from(&st.local);
            }
            3 => {
                st.npoints = u32be(&s[6..]) as usize;
                let tmpl = u16be(&s[12..]);
                st.grid = if tmpl == 0 { Some(grid_latlon(s)?) } else { None };
                if tmpl != 0 {
                    skipped.push(format!("grid template 3.{tmpl} not supported"));
                }
            }
            4 => st.product = product(s, st.ref_time),
            5 => st.repr = Some((u16be(&s[9..]), s.to_vec())),
            6 => match s[5] {
                0 => st.bitmap = Some(s[6..].to_vec()),
                254 => {} // reuse the previous bitmap
                _ => st.bitmap = None,
            },
            7 => {
                let r = field(&st, s);
                match r {
                    Ok(Some(f)) => out.push(f),
                    Ok(None) => {}
                    Err(e) => skipped.push(e.to_string()),
                }
            }
            _ => {}
        }
        p += slen;
    }
    Ok((out, skipped))
}

fn field(st: &State, s7: &[u8]) -> Result<Option<Field>, GribError> {
    let (Some(grid), Some(prod), Some((tmpl, s5))) = (&st.grid, &st.product, &st.repr) else {
        return Ok(None);
    };
    let nvalues = u32be(&s5[5..]) as usize;
    let packed = match tmpl {
        0 => unpack_simple(s5, &s7[5..], nvalues)?,
        2 | 3 => unpack_complex(s5, &s7[5..], nvalues, *tmpl == 3)?,
        t => return Err(GribError::Unsupported(format!("data representation 5.{t}"))),
    };
    // Spread over the grid using the bitmap (missing points become NaN).
    let n = st.npoints;
    let raw: Vec<f32> = match &st.bitmap {
        Some(bm) => {
            let mut it = packed.into_iter();
            (0..n)
                .map(|i| if bm[i / 8] & (0x80 >> (i % 8)) != 0 { it.next().unwrap_or(f32::NAN) } else { f32::NAN })
                .collect()
        }
        None => {
            if packed.len() != n {
                return Err(GribError::Format(format!("{} values for {} points", packed.len(), n)));
            }
            packed
        }
    };
    let values = grid.normalise(raw);
    let param = Param::from_codes(st.discipline, prod.category, prod.number, prod.surface_type);
    Ok(Some(Field {
        param,
        discipline: st.discipline,
        category: prod.category,
        number: prod.number,
        surface_type: prod.surface_type,
        surface_value: prod.surface_value,
        centre: st.centre,
        ref_time: st.ref_time,
        valid_time: prod.valid_time,
        member: prod.member,
        grid: grid.canonical(),
        values,
        local: st.local.clone(),
    }))
}

fn grid_latlon(s: &[u8]) -> Result<Grid, GribError> {
    if s.len() < 72 {
        return Err(GribError::Format("grid section too short".into()));
    }
    let ni = u32be(&s[30..]) as usize;
    let nj = u32be(&s[34..]) as usize;
    let basic = u32be(&s[38..]);
    let subdiv = u32be(&s[42..]);
    let unit = if basic == 0 || basic == u32::MAX || subdiv == 0 || subdiv == u32::MAX {
        1e-6
    } else {
        basic as f64 / subdiv as f64
    };
    let la1 = sm32(&s[46..]) as f64 * unit;
    let lo1 = sm32(&s[50..]) as f64 * unit;
    let la2 = sm32(&s[55..]) as f64 * unit;
    let lo2 = sm32(&s[59..]) as f64 * unit;
    let di = u32be(&s[63..]) as f64 * unit;
    let dj = u32be(&s[67..]) as f64 * unit;
    let scan = s[71];
    Ok(Grid::from_grib(ni, nj, la1, lo1, la2, lo2, di, dj, scan))
}

fn product(s: &[u8], ref_time: i64) -> Option<Product> {
    let tmpl = u16be(&s[7..]);
    if !matches!(tmpl, 0 | 1 | 8 | 11) || s.len() < 34 {
        return None;
    }
    let category = s[9];
    let number = s[10];
    let unit = s[17];
    let fct = sm32(&s[18..]) as i64;
    let secs = match unit {
        0 => 60,
        1 => 3600,
        2 => 86400,
        10 => 3 * 3600,
        11 => 6 * 3600,
        12 => 12 * 3600,
        13 => 1,
        _ => 3600,
    };
    let surface_type = s[22];
    let scale = s[23] as i8;
    let sval = u32be(&s[24..]);
    let surface_value = if sval == u32::MAX { 0.0 } else { sval as f64 / 10f64.powi(scale as i32) };
    let mut valid_time = ref_time + fct * secs;
    let mut member = None;
    match tmpl {
        1 if s.len() >= 37 => member = Some(s[35] as u16),
        8 if s.len() >= 41 => valid_time = unix(u16be(&s[34..]) as i32, s[36], s[37], s[38], s[39], s[40]),
        11 if s.len() >= 44 => {
            member = Some(s[35] as u16);
            valid_time = unix(u16be(&s[37..]) as i32, s[39], s[40], s[41], s[42], s[43]);
        }
        _ => {}
    }
    Some(Product { category, number, surface_type, surface_value, valid_time, member })
}

struct Scale {
    r: f64,
    e: f64,
    d: f64,
    bits: usize,
}

fn scale(s5: &[u8]) -> Scale {
    let r = f32::from_be_bytes([s5[11], s5[12], s5[13], s5[14]]) as f64;
    let e = sm16(&s5[15..]) as i32;
    let d = sm16(&s5[17..]) as i32;
    Scale { r, e: 2f64.powi(e), d: 10f64.powi(-d), bits: s5[19] as usize }
}

fn unpack_simple(s5: &[u8], data: &[u8], n: usize) -> Result<Vec<f32>, GribError> {
    let sc = scale(s5);
    if sc.bits == 0 {
        return Ok(vec![(sc.r * sc.d) as f32; n]);
    }
    let mut br = Bits::new(data);
    (0..n)
        .map(|_| br.take(sc.bits).map(|x| ((sc.r + x as f64 * sc.e) * sc.d) as f32))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| GribError::Format("simple packing: data too short".into()))
}

fn unpack_complex(s5: &[u8], data: &[u8], n: usize, spatial: bool) -> Result<Vec<f32>, GribError> {
    let err = |m: &str| GribError::Format(format!("complex packing: {m}"));
    if s5.len() < 47 + spatial as usize * 2 {
        return Err(err("section 5 too short"));
    }
    let sc = scale(s5);
    let miss_mgmt = s5[22];
    let ng = u32be(&s5[31..]) as usize;
    let ref_width = s5[35] as u64;
    let width_bits = s5[36] as usize;
    let ref_len = u32be(&s5[37..]) as u64;
    let len_inc = s5[41] as u64;
    let last_len = u32be(&s5[42..]) as u64;
    let len_bits = s5[46] as usize;
    let (order, extra) = if spatial { (s5[47] as usize, s5[48] as usize) } else { (0, 0) };
    if miss_mgmt != 0 {
        return Err(GribError::Unsupported("complex packing with missing-value management".into()));
    }

    let mut br = Bits::new(data);
    let mut first = [0i64; 2];
    let mut minsd = 0i64;
    if spatial {
        for v in first.iter_mut().take(order) {
            *v = br.signed_octets(extra).ok_or_else(|| err("descriptors"))?;
        }
        minsd = br.signed_octets(extra).ok_or_else(|| err("descriptors"))?;
    }
    let refs: Vec<u64> = (0..ng).map(|_| br.take(sc.bits)).collect::<Option<_>>().ok_or_else(|| err("group refs"))?;
    br.align();
    let widths: Vec<usize> = (0..ng)
        .map(|_| br.take(width_bits).map(|w| (w + ref_width) as usize))
        .collect::<Option<_>>()
        .ok_or_else(|| err("group widths"))?;
    br.align();
    let mut lens: Vec<u64> = (0..ng)
        .map(|_| br.take(len_bits).map(|l| ref_len + l * len_inc))
        .collect::<Option<_>>()
        .ok_or_else(|| err("group lengths"))?;
    br.align();
    if let Some(l) = lens.last_mut() {
        *l = last_len;
    }
    let mut x: Vec<i64> = Vec::with_capacity(n);
    for g in 0..ng {
        for _ in 0..lens[g] {
            let v = if widths[g] == 0 { 0 } else { br.take(widths[g]).ok_or_else(|| err("values"))? };
            x.push((refs[g] + v) as i64);
        }
    }
    if x.len() != n {
        return Err(err(&format!("{} values, expected {n}", x.len())));
    }
    if spatial && order > 0 {
        for v in x.iter_mut().skip(order) {
            *v += minsd;
        }
        if order == 1 {
            x[0] = first[0];
            for i in 1..n {
                x[i] += x[i - 1];
            }
        } else if order == 2 && n > 1 {
            x[0] = first[0];
            x[1] = first[1];
            for i in 2..n {
                x[i] += 2 * x[i - 1] - x[i - 2];
            }
        }
    }
    Ok(x.into_iter().map(|v| ((sc.r + v as f64 * sc.e) * sc.d) as f32).collect())
}

struct Bits<'a> {
    b: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, bit: 0 }
    }
    fn take(&mut self, n: usize) -> Option<u64> {
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.b.get(self.bit / 8)?;
            v = (v << 1) | ((byte >> (7 - self.bit % 8)) & 1) as u64;
            self.bit += 1;
        }
        Some(v)
    }
    fn align(&mut self) {
        self.bit = self.bit.div_ceil(8) * 8;
    }
    /// Sign-magnitude integer stored in `n` whole octets.
    fn signed_octets(&mut self, n: usize) -> Option<i64> {
        self.align();
        let v = self.take(n * 8)?;
        let sign = 1u64 << (n * 8 - 1);
        Some(if v & sign != 0 { -((v & !sign) as i64) } else { v as i64 })
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}
fn u16be(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn u24(b: &[u8]) -> u32 {
    u32::from_be_bytes([0, b[0], b[1], b[2]])
}
fn u32be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn u64be(b: &[u8]) -> u64 {
    u64::from_be_bytes(b[..8].try_into().unwrap())
}
/// Sign-magnitude 16-bit (GRIB's signed integers).
fn sm16(b: &[u8]) -> i16 {
    let v = u16be(b);
    if v & 0x8000 != 0 { -((v & 0x7fff) as i16) } else { v as i16 }
}
fn sm32(b: &[u8]) -> i32 {
    let v = u32be(b);
    if v & 0x8000_0000 != 0 { -((v & 0x7fff_ffff) as i32) } else { v as i32 }
}

/// Seconds since 1970-01-01 UTC.
pub fn unix(y: i32, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> i64 {
    // Howard Hinnant's days_from_civil
    let (y, m) = if mo <= 2 { (y - 1, mo as i32 + 9) } else { (y, mo as i32 - 3) };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146097 + doe as i64 - 719468;
    days * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn epoch() {
        assert_eq!(unix(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(unix(2026, 10, 11, 12, 0, 0), 1_791_720_000);
    }
    #[test]
    fn sign_magnitude() {
        assert_eq!(sm16(&[0x80, 0x02]), -2);
        assert_eq!(sm16(&[0x00, 0x02]), 2);
        assert_eq!(sm32(&[0x82, 0x3B, 0x2C, 0x10]), -37_432_336);
    }
}

/// Header-only scan: every field's metadata without reading the data sections.
/// Reads the file in place, skipping section 7, so large files are quick.
pub fn scan_file(path: &std::path::Path) -> Result<Vec<FieldMeta>, GribError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::io::BufReader::with_capacity(1 << 16, std::fs::File::open(path)?);
    let size = f.get_ref().metadata()?.len();
    let mut out = Vec::new();
    let mut local: Option<serde_json::Value> = None;
    let mut pos: u64 = 0;
    let mut head = [0u8; 16];
    while pos + 16 <= size {
        f.seek(SeekFrom::Start(pos))?;
        f.read_exact(&mut head)?;
        if &head[0..4] != b"GRIB" {
            // resync: look for the next "GRIB" within the next 1 MB
            let mut buf = vec![0u8; (1 << 20).min((size - pos) as usize)];
            f.seek(SeekFrom::Start(pos))?;
            f.read_exact(&mut buf)?;
            match find(&buf[1..], b"GRIB") {
                Some(o) => {
                    pos += 1 + o as u64;
                    continue;
                }
                None => break,
            }
        }
        if head[7] == 1 {
            pos += (u24(&head[4..]) as u64).max(8);
            continue;
        }
        let len = u64be(&head[8..]);
        if head[7] != 2 || len < 20 || pos + len > size {
            break;
        }
        let mut st = State { discipline: head[6], local: local.clone(), ..Default::default() };
        let mut p = pos + 16;
        let end = pos + len - 4;
        while p + 5 <= end {
            let mut sh = [0u8; 5];
            f.seek(SeekFrom::Start(p))?;
            f.read_exact(&mut sh)?;
            let slen = u32be(&sh) as u64;
            let snum = sh[4];
            if slen < 5 || p + slen > pos + len {
                break;
            }
            if snum == 7 || snum == 6 || snum == 5 {
                if snum == 7 {
                    if let (Some(g), Some(pr)) = (&st.grid, &st.product) {
                        out.push(FieldMeta {
                            param: crate::Param::from_codes(st.discipline, pr.category, pr.number, pr.surface_type),
                            centre: st.centre,
                            ref_time: st.ref_time,
                            valid_time: pr.valid_time,
                            member: pr.member,
                            grid: g.canonical(),
                            local: st.local.clone(),
                        });
                    }
                }
                p += slen;
                continue;
            }
            let mut s = vec![0u8; slen as usize];
            s[..5].copy_from_slice(&sh);
            f.read_exact(&mut s[5..])?;
            match snum {
                1 if s.len() >= 19 => {
                    st.centre = u16be(&s[5..]);
                    st.ref_time = unix(u16be(&s[12..]) as i32, s[14], s[15], s[16], s[17], s[18]);
                }
                2 => {
                    st.local = std::str::from_utf8(&s[5..]).ok().and_then(|t| serde_json::from_str(t.trim_end_matches('\0')).ok());
                    local.clone_from(&st.local);
                }
                3 => st.grid = if u16be(&s[12..]) == 0 { grid_latlon(&s).ok() } else { None },
                4 => st.product = product(&s, st.ref_time),
                _ => {}
            }
            p += slen;
        }
        pos += len;
    }
    Ok(out)
}

/// Field metadata from [`scan_file`].
#[derive(Debug, Clone)]
pub struct FieldMeta {
    pub param: crate::Param,
    pub centre: u16,
    pub ref_time: i64,
    pub valid_time: i64,
    pub member: Option<u16>,
    pub grid: Grid,
    pub local: Option<serde_json::Value>,
}
