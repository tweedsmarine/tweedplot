//! GRIB edition 1: regular lat/lon grids (GDS type 0), simple grid-point
//! packing, optional bitmap. Saildocs and many older tools send GRIB1.
//!
//! Parameters are mapped onto their GRIB2 codes so the rest of the crate sees
//! them the same way: WMO/NCEP table 2 (33/34 wind, 2 MSL pressure, 180 gust,
//! 59 rain rate, 157 CAPE, 100–108 waves, 49/50 current) and ECMWF tables 128
//! (165/166 wind, 151 MSL, 49 gust) and 140 (229 Hs, 230 direction, 231 period).

use crate::{Field, GribError, Grid, Param};

fn u16be(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn u24(b: &[u8]) -> u32 {
    u32::from_be_bytes([0, b[0], b[1], b[2]])
}
/// Sign-magnitude 16 and 24 bit.
fn sm16(b: &[u8]) -> i32 {
    let v = u16be(b) as i32;
    if v & 0x8000 != 0 {
        -(v & 0x7fff)
    } else {
        v
    }
}
fn sm24(b: &[u8]) -> i32 {
    let v = u24(b) as i32;
    if v & 0x80_0000 != 0 {
        -(v & 0x7f_ffff)
    } else {
        v
    }
}
/// IBM System/360 single-precision float.
fn ibm(b: &[u8]) -> f64 {
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exp = (b[0] & 0x7f) as i32 - 64;
    let mant = u24(&b[1..]) as f64 / 16_777_216.0;
    sign * mant * 16f64.powi(exp)
}

/// GRIB2 (discipline, category, number, surface type) for a GRIB1 parameter.
fn map(centre: u8, table: u8, param: u8, level_type: u8) -> Option<(u8, u8, u8, u8)> {
    let height = |lt: u8| if lt == 105 { 103 } else { lt };
    if centre == 98 && table == 128 {
        return match param {
            165 => Some((0, 2, 2, 103)),
            166 => Some((0, 2, 3, 103)),
            151 => Some((0, 3, 0, 101)),
            167 => Some((0, 0, 0, 103)),
            49 => Some((0, 2, 22, 103)),
            _ => None,
        };
    }
    if centre == 98 && table == 140 {
        return match param {
            229 => Some((10, 0, 3, 1)),
            230 => Some((10, 0, 14, 1)),
            231 => Some((10, 0, 34, 1)),
            _ => None,
        };
    }
    match param {
        33 => Some((0, 2, 2, height(level_type))),
        34 => Some((0, 2, 3, height(level_type))),
        32 => Some((0, 2, 1, height(level_type))),
        31 => Some((0, 2, 0, height(level_type))),
        2 => Some((0, 3, 0, 101)),
        11 if level_type == 105 => Some((0, 0, 0, 103)),
        1 if level_type == 102 => Some((0, 3, 0, 101)),
        180 => Some((0, 2, 22, 1)),
        59 => Some((0, 1, 7, 1)),
        157 => Some((0, 7, 6, 1)),
        100 => Some((10, 0, 3, 1)),
        102 => Some((10, 0, 5, 1)),
        105 => Some((10, 0, 8, 1)),
        107 => Some((10, 0, 10, 1)),
        108 => Some((10, 0, 11, 1)),
        49 => Some((10, 1, 2, 1)),
        50 => Some((10, 1, 3, 1)),
        _ => None,
    }
}

struct Pds {
    centre: u8,
    table: u8,
    param: u8,
    level_type: u8,
    level: u16,
    ref_time: i64,
    valid_time: i64,
    gds: bool,
    bms: bool,
    d: i32,
}

fn pds(s: &[u8]) -> Result<Pds, GribError> {
    if s.len() < 28 {
        return Err(GribError::Format("GRIB1 PDS too short".into()));
    }
    let century = if s[24] == 0 { 21 } else { s[24] as i32 };
    let year = (century - 1) * 100 + s[12] as i32;
    let ref_time = crate::decode::unix(year, s[13], s[14], s[15], s[16], 0);
    let unit = match s[17] {
        0 => 60,
        1 => 3600,
        2 => 86400,
        10 => 3 * 3600,
        11 => 6 * 3600,
        12 => 12 * 3600,
        254 => 1,
        _ => 3600,
    };
    let (p1, p2) = (s[18] as i64, s[19] as i64);
    let lead = match s[20] {
        10 => (p1 << 8) | p2,
        2..=5 => p2,
        _ => p1,
    };
    Ok(Pds {
        centre: s[4],
        table: s[3],
        param: s[8],
        level_type: s[9],
        level: u16be(&s[10..]),
        ref_time,
        valid_time: ref_time + lead * unit,
        gds: s[7] & 0x80 != 0,
        bms: s[7] & 0x40 != 0,
        d: sm16(&s[26..]),
    })
}

fn grid(s: &[u8]) -> Result<Grid, GribError> {
    if s.len() < 28 || s[5] != 0 {
        return Err(GribError::Unsupported(format!("GRIB1 grid type {}", s.get(5).copied().unwrap_or(255))));
    }
    let ni = u16be(&s[6..]) as usize;
    let nj = u16be(&s[8..]) as usize;
    let la1 = sm24(&s[10..]) as f64 / 1000.0;
    let lo1 = sm24(&s[13..]) as f64 / 1000.0;
    let la2 = sm24(&s[17..]) as f64 / 1000.0;
    let lo2 = sm24(&s[20..]) as f64 / 1000.0;
    let scan = s[27];
    let given = s[16] & 0x80 != 0;
    let di = u16be(&s[23..]);
    let dj = u16be(&s[25..]);
    let span_lon = {
        let mut d = if scan & 0x80 != 0 { lo1 - lo2 } else { lo2 - lo1 };
        if d < 0.0 {
            d += 360.0;
        }
        d
    };
    let di = if given && di != 0xffff {
        di as f64 / 1000.0
    } else if ni > 1 {
        span_lon / (ni - 1) as f64
    } else {
        1.0
    };
    let dj = if given && dj != 0xffff {
        dj as f64 / 1000.0
    } else if nj > 1 {
        (la2 - la1).abs() / (nj - 1) as f64
    } else {
        1.0
    };
    Ok(Grid::from_grib(ni, nj, la1, lo1, la2, lo2, di, dj, scan))
}

/// The field of one GRIB1 message (None for parameters this app doesn't use).
pub fn message(m: &[u8], with_values: bool) -> Result<Option<Field>, GribError> {
    if m.len() < 8 + 28 + 4 {
        return Err(GribError::Format("GRIB1 message too short".into()));
    }
    let mut p = 8;
    let sl = u24(&m[p..]) as usize;
    let pd = pds(&m[p..(p + sl).min(m.len())])?;
    p += sl;
    let Some((discipline, category, number, surface)) = map(pd.centre, pd.table, pd.param, pd.level_type) else {
        return Ok(None);
    };
    if !pd.gds {
        return Err(GribError::Unsupported("GRIB1 without a grid description (predefined grids)".into()));
    }
    let gl = u24(m.get(p..p + 3).ok_or_else(|| GribError::Format("GRIB1 message truncated".into()))?) as usize;
    let g = grid(m.get(p..p + gl).ok_or_else(|| GribError::Format("GRIB1 GDS truncated".into()))?)?;
    p += gl;
    let npoints = g.ni * g.nj;
    let mut bitmap: Option<&[u8]> = None;
    let short = || GribError::Format("GRIB1 message truncated".into());
    if pd.bms && with_values {
        let bl = u24(m.get(p..p + 3).ok_or_else(short)?) as usize;
        if u16be(m.get(p + 4..p + 6).ok_or_else(short)?) != 0 {
            return Err(GribError::Unsupported("GRIB1 predefined bitmap".into()));
        }
        bitmap = Some(m.get(p + 6..p + bl).ok_or_else(short)?);
        p += bl;
    }
    let values = if with_values {
        let bl = u24(m.get(p..p + 3).ok_or_else(short)?) as usize;
        let s = m.get(p..(p + bl).min(m.len())).filter(|s| s.len() >= 11).ok_or_else(short)?;
        let flag = s[3];
        if flag & 0xC0 != 0 {
            return Err(GribError::Unsupported("GRIB1 complex or spherical-harmonic packing".into()));
        }
        let unused = (flag & 0x0f) as usize;
        let e = 2f64.powi(sm16(&s[4..]));
        let r = ibm(&s[6..]);
        let bits = s[10] as usize;
        let dsc = 10f64.powi(-pd.d);
        let data = &s[11..];
        let npacked = match bitmap {
            Some(bm) => (0..npoints).filter(|&i| bm.get(i / 8).is_some_and(|b| b & (0x80 >> (i % 8)) != 0)).count(),
            None => (data.len() * 8).saturating_sub(unused).checked_div(bits).unwrap_or(npoints).min(npoints),
        };
        let mut packed = Vec::with_capacity(npacked);
        if bits == 0 {
            packed.resize(npacked, (r * dsc) as f32);
        } else {
            let mut pos = 0usize;
            for _ in 0..npacked {
                let mut v = 0u64;
                for _ in 0..bits {
                    let b = data.get(pos >> 3).ok_or_else(|| GribError::Format("GRIB1 data too short".into()))?;
                    v = (v << 1) | ((b >> (7 - (pos & 7))) & 1) as u64;
                    pos += 1;
                }
                packed.push(((r + v as f64 * e) * dsc) as f32);
            }
        }
        let raw: Vec<f32> = match bitmap {
            Some(bm) => {
                let mut it = packed.into_iter();
                (0..npoints).map(|i| if bm.get(i / 8).is_some_and(|b| b & (0x80 >> (i % 8)) != 0) { it.next().unwrap_or(f32::NAN) } else { f32::NAN }).collect()
            }
            None => {
                if packed.len() != npoints {
                    return Err(GribError::Format(format!("GRIB1: {} values for {} points", packed.len(), npoints)));
                }
                packed
            }
        };
        g.normalise(raw)
    } else {
        Vec::new()
    };
    let surface_value = if pd.level_type == 105 { pd.level as f64 } else { 0.0 };
    Ok(Some(Field {
        param: Param::from_codes(discipline, category, number, surface),
        discipline,
        category,
        number,
        surface_type: surface,
        surface_value,
        centre: pd.centre as u16,
        ref_time: pd.ref_time,
        valid_time: pd.valid_time,
        member: None,
        grid: g.canonical(),
        values,
        local: None,
    }))
}
