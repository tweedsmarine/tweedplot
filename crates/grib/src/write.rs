//! Writing GRIB2: cropping a field to an area and encoding it with simple
//! packing (template 5.0) on a regular lat/lon grid (3.0), product template
//! 4.0. Used to keep only the area wanted from global downloads (ECMWF open
//! data comes as global CCSDS-packed fields).
//!
//! No local-use section is written (other software reads section 2 by the
//! originating centre's own rules); instead the generating process identifier
//! of section 4 can carry `Process` codes this crate's reader turns into a
//! model name (ECMWF IFS / AIFS / WAM).

use crate::{Field, Grid, Param};

/// The part of `f` inside `area` = [west, south, east, north] (east may be
/// beyond 180 to cross the date line; a global field wraps). None if the area
/// misses the field.
pub fn crop(f: &Field, area: [f64; 4]) -> Option<Field> {
    let g = &f.grid;
    let [w, s, e, n] = area;
    let j0 = ((s - g.lat0) / g.dlat - 1e-6).ceil().max(0.0) as usize;
    let j1 = (((n - g.lat0) / g.dlat + 1e-6).floor() as isize).min(g.nj as isize - 1);
    if j1 < j0 as isize {
        return None;
    }
    let j1 = j1 as usize;
    let global = (g.ni as f64 * g.dlon - 360.0).abs() < g.dlon * 0.5;
    let width = {
        let mut x = e - w;
        if x < 0.0 {
            x += 360.0;
        }
        x
    };
    let start = ((w - g.lon0).rem_euclid(360.0) / g.dlon - 1e-6).ceil() as usize;
    let count = ((width / g.dlon) + 1e-6).floor() as usize + 1;
    let (i_start, ni) = if global {
        (start % g.ni, count.min(g.ni))
    } else {
        // non-global: clip to the grid
        let s0 = start.min(g.ni);
        let end = (s0 + count).min(g.ni);
        if end <= s0 {
            return None;
        }
        (s0, end - s0)
    };
    let nj = j1 - j0 + 1;
    let mut values = Vec::with_capacity(ni * nj);
    for j in j0..=j1 {
        for k in 0..ni {
            let i = (i_start + k) % g.ni;
            values.push(f.values[j * g.ni + i]);
        }
    }
    let lon0 = nav_core::wrap_lon(g.lon0 + i_start as f64 * g.dlon);
    let grid = Grid::regular(ni, nj, g.lat0 + j0 as f64 * g.dlat, lon0, g.dlat, g.dlon);
    Some(Field { grid, values, ..f.clone() })
}

/// Decimal scale used when packing: enough for each quantity, no more.
fn decimals(f: &Field) -> i32 {
    match f.param {
        Param::WindU | Param::WindV | Param::WindSpeed | Param::Gust | Param::CurrentU | Param::CurrentV | Param::SubCurrentU | Param::SubCurrentV => 2,
        Param::Mslp => 0,
        Param::WaveHeight | Param::SwellHeight | Param::WindWaveHeight | Param::WavePeriod => 2,
        Param::WaveDir | Param::WindDir => 1,
        Param::SeaTemp | Param::SubTemp => 2,
        Param::Salinity | Param::SubSalinity => 3,
        Param::SkinTemp => 2,
        Param::Chlorophyll | Param::SeaLevel => 3,
        Param::SeaIce => 2,
        Param::SeaTempError => 2,
        Param::MixedLayer => 1,
        Param::Phytoplankton => 3,
        Param::Oxygen => 1,
        Param::PrimaryProduction => 2,
        Param::Ph => 3,
        _ => 3,
    }
}

fn sm16(v: i32) -> [u8; 2] {
    let m = v.unsigned_abs().min(0x7fff) as u16;
    (if v < 0 { m | 0x8000 } else { m }).to_be_bytes()
}
fn sm32(v: i64) -> [u8; 4] {
    let m = v.unsigned_abs().min(0x7fff_ffff) as u32;
    (if v < 0 { m | 0x8000_0000 } else { m }).to_be_bytes()
}
fn section(num: u8, body: &[u8]) -> Vec<u8> {
    let mut s = ((body.len() + 5) as u32).to_be_bytes().to_vec();
    s.push(num);
    s.extend_from_slice(body);
    s
}
fn civil(t: i64) -> (u16, u8, u8, u8, u8, u8) {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil_from_days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    (y as u16, m as u8, d as u8, (secs / 3600) as u8, (secs / 60 % 60) as u8, (secs % 60) as u8)
}

/// Generating process identifiers racenav writes for models a centre runs
/// side by side (read back as the model name).
pub const PROCESS_ECMWF_IFS: u8 = 201;
pub const PROCESS_ECMWF_AIFS: u8 = 202;
pub const PROCESS_ECMWF_WAM: u8 = 203;
/// Ocean data racenav converts to GRIB2 from NetCDF (see gribdl::ocean).
pub const PROCESS_MUR_SST: u8 = 204;
pub const PROCESS_HYCOM_ESPC: u8 = 205;
pub const PROCESS_HIMAWARI_SST: u8 = 206;
pub const PROCESS_VIIRS_CHL: u8 = 207;
pub const PROCESS_ASI_ICE: u8 = 208;
pub const PROCESS_ACSPO_SST: u8 = 209;
pub const PROCESS_BLENDED_SST: u8 = 210;
/// Copernicus Marine: global physics (Mercator 1/12°), biogeochemistry (1/4°), ocean colour (4 km).
pub const PROCESS_CMEMS_PHY: u8 = 211;
pub const PROCESS_CMEMS_BGC: u8 = 212;
pub const PROCESS_CMEMS_OC: u8 = 213;
/// NOAA AIGFS, cut from NOMADS' global files (see gribdl::aigfs).
pub const PROCESS_NOAA_AIGFS: u8 = 214;
/// GRIB2 originating centres used with them: NASA, US Navy FNMOC, NOAA NESDIS.
pub const CENTRE_NASA: u16 = 173;
pub const CENTRE_FNMOC: u16 = 58;
pub const CENTRE_NESDIS: u16 = 160;
/// "Missing" in the WMO list: for sources with no centre code of their own (University of Bremen).
pub const CENTRE_LOCAL: u16 = 255;

/// The model name for a centre and racenav process code, if it is one of ours.
pub fn process_name(centre: u16, process: u8) -> Option<&'static str> {
    match (centre, process) {
        (98, PROCESS_ECMWF_IFS) => Some("ECMWF IFS"),
        (98, PROCESS_ECMWF_AIFS) => Some("ECMWF AIFS"),
        (98, PROCESS_ECMWF_WAM) => Some("ECMWF WAM"),
        (CENTRE_NASA, PROCESS_MUR_SST) => Some("MUR SST 1 km"),
        (CENTRE_FNMOC, PROCESS_HYCOM_ESPC) => Some("HYCOM ESPC ocean"),
        (CENTRE_NESDIS, PROCESS_HIMAWARI_SST) => Some("Himawari-9 SST hourly"),
        (CENTRE_NESDIS, PROCESS_VIIRS_CHL) => Some("VIIRS chlorophyll"),
        (CENTRE_LOCAL, PROCESS_ASI_ICE) => Some("AMSR2 sea ice (Bremen)"),
        (CENTRE_NESDIS, PROCESS_ACSPO_SST) => Some("ACSPO SST 2 km"),
        (CENTRE_NESDIS, PROCESS_BLENDED_SST) => Some("Geo-Polar SST 5 km"),
        (CENTRE_LOCAL, PROCESS_CMEMS_PHY) => Some("Copernicus ocean 1/12°"),
        (CENTRE_LOCAL, PROCESS_CMEMS_BGC) => Some("Copernicus biology 1/4°"),
        (CENTRE_LOCAL, PROCESS_CMEMS_OC) => Some("Copernicus chlorophyll 4 km"),
        (7, PROCESS_NOAA_AIGFS) => Some("NOAA AIGFS"),
        _ => None,
    }
}

/// One GRIB2 message for `f` (simple packing; NaN = missing, with a bitmap);
/// `process` goes in section 4's generating process identifier (0 = none).
pub fn encode(f: &Field, process: u8) -> Vec<u8> {
    let g = &f.grid;
    let n = g.ni * g.nj;
    // section 1: identification
    let (y, mo, d, h, mi, s) = civil(f.ref_time);
    let mut s1 = Vec::new();
    s1.extend_from_slice(&f.centre.to_be_bytes());
    s1.extend_from_slice(&0u16.to_be_bytes()); // sub-centre
    s1.push(28); // master tables version
    s1.push(0); // local tables
    s1.push(1); // reference time = start of forecast
    s1.extend_from_slice(&y.to_be_bytes());
    s1.extend_from_slice(&[mo, d, h, mi, s]);
    s1.push(0); // operational products
    s1.push(1); // forecast
                // section 3: grid, template 3.0
    let mut s3 = vec![0u8]; // source of grid definition
    s3.extend_from_slice(&(n as u32).to_be_bytes());
    s3.extend_from_slice(&[0, 0]);
    s3.extend_from_slice(&0u16.to_be_bytes());
    s3.push(6); // earth: sphere r = 6 371 229 m
    s3.extend_from_slice(&[0; 15]);
    s3.extend_from_slice(&(g.ni as u32).to_be_bytes());
    s3.extend_from_slice(&(g.nj as u32).to_be_bytes());
    s3.extend_from_slice(&0u32.to_be_bytes()); // basic angle
    s3.extend_from_slice(&u32::MAX.to_be_bytes()); // subdivisions: missing → micro-degrees
    let micro = |x: f64| (x * 1e6).round() as i64;
    let lon1 = g.lon0.rem_euclid(360.0);
    let lon2 = (g.lon0 + (g.ni as f64 - 1.0) * g.dlon).rem_euclid(360.0);
    s3.extend_from_slice(&sm32(micro(g.lat0)));
    s3.extend_from_slice(&sm32(micro(lon1)));
    s3.push(0x30); // increments given
    s3.extend_from_slice(&sm32(micro(g.lat0 + (g.nj as f64 - 1.0) * g.dlat)));
    s3.extend_from_slice(&sm32(micro(lon2)));
    s3.extend_from_slice(&(micro(g.dlon) as u32).to_be_bytes());
    s3.extend_from_slice(&(micro(g.dlat) as u32).to_be_bytes());
    s3.push(0x40); // west to east, south to north, rows
                   // section 4: product, template 4.0
    let lead = f.valid_time - f.ref_time;
    let (unit, ft) = if lead % 3600 == 0 { (1u8, lead / 3600) } else { (0u8, lead / 60) };
    let mut s4 = Vec::new();
    s4.extend_from_slice(&0u16.to_be_bytes()); // no coordinate values
    s4.extend_from_slice(&0u16.to_be_bytes()); // template 4.0
    s4.extend_from_slice(&[f.category, f.number, 2, 0, process]);
    s4.extend_from_slice(&0u16.to_be_bytes());
    s4.push(0);
    s4.push(unit);
    s4.extend_from_slice(&sm32(ft));
    s4.push(f.surface_type);
    let (sc, sv) = if f.surface_value.fract().abs() < 1e-9 { (0u8, f.surface_value as u32) } else { (2u8, (f.surface_value * 100.0).round() as u32) };
    let missing_level = matches!(f.surface_type, 1 | 101);
    s4.push(if missing_level { 255 } else { sc });
    s4.extend_from_slice(&(if missing_level { u32::MAX } else { sv }).to_be_bytes());
    s4.push(255);
    s4.push(255);
    s4.extend_from_slice(&u32::MAX.to_be_bytes());
    // sections 5–7: simple packing
    let dsc = decimals(f);
    let k = 10f64.powi(dsc);
    let valid: Vec<f64> = f.values.iter().filter(|v| v.is_finite()).map(|&v| v as f64 * k).collect();
    let (lo, hi) = valid.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    let r = if valid.is_empty() { 0.0f32 } else { lo as f32 };
    let r = if (r as f64) > lo { f32::from_bits(r.to_bits().wrapping_add(if r > 0.0 { u32::MAX } else { 1 })) } else { r }; // never above the minimum
    let span = if valid.is_empty() { 0.0 } else { (hi - r as f64).round() };
    let bits = if span <= 0.0 { 0 } else { (span + 1.0).log2().ceil() as usize };
    let mut s5 = Vec::new();
    s5.extend_from_slice(&(valid.len() as u32).to_be_bytes());
    s5.extend_from_slice(&0u16.to_be_bytes());
    s5.extend_from_slice(&r.to_be_bytes());
    s5.extend_from_slice(&sm16(0));
    s5.extend_from_slice(&sm16(dsc));
    s5.push(bits as u8);
    s5.push(0);
    let has_missing = valid.len() < n;
    let s6 = if has_missing {
        let mut b = vec![0u8];
        let mut bm = vec![0u8; n.div_ceil(8)];
        for (i, v) in f.values.iter().enumerate() {
            if v.is_finite() {
                bm[i / 8] |= 0x80 >> (i % 8);
            }
        }
        b.extend(bm);
        b
    } else {
        vec![255u8]
    };
    let mut packed = Vec::with_capacity((valid.len() * bits).div_ceil(8));
    let (mut acc, mut nacc) = (0u64, 0usize);
    if bits > 0 {
        for v in &valid {
            let x = ((v - r as f64).round().max(0.0) as u64).min((1u64 << bits) - 1);
            acc = (acc << bits) | x;
            nacc += bits;
            while nacc >= 8 {
                nacc -= 8;
                packed.push((acc >> nacc) as u8);
            }
            acc &= (1u64 << nacc) - 1;
        }
        if nacc > 0 {
            packed.push((acc << (8 - nacc)) as u8);
        }
    }
    let mut body = Vec::new();
    body.extend(section(1, &s1));
    body.extend(section(3, &s3));
    body.extend(section(4, &s4));
    body.extend(section(5, &s5));
    body.extend(section(6, &s6));
    body.extend(section(7, &packed));
    body.extend_from_slice(b"7777");
    let total = 16 + body.len() as u64;
    let mut m = b"GRIB".to_vec();
    m.extend_from_slice(&[0, 0, f.discipline, 2]);
    m.extend_from_slice(&total.to_be_bytes());
    m.extend(body);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(param: Param, number: u8, f: impl Fn(f64, f64) -> f32) -> Field {
        // global 1° grid, -180..179
        let g = Grid::regular(360, 181, -90.0, -180.0, 1.0, 1.0);
        let mut values = Vec::new();
        for j in 0..181 {
            for i in 0..360 {
                values.push(f(-90.0 + j as f64, -180.0 + i as f64));
            }
        }
        Field {
            param,
            discipline: 0,
            category: 2,
            number,
            surface_type: 103,
            surface_value: 10.0,
            centre: 98,
            ref_time: 1_792_108_800,
            valid_time: 1_792_108_800 + 9 * 3600,
            member: None,
            grid: g,
            values,
            local: None,
        }
    }

    #[test]
    fn crop_and_round_trip_across_the_date_line() {
        let u = field(Param::WindU, 2, |la, lo| (la / 10.0 + lo / 100.0) as f32);
        // 170E to 170W, 50S to 30S
        let c = crop(&u, [170.0, -50.0, -170.0, -30.0]).unwrap();
        assert_eq!((c.grid.ni, c.grid.nj), (21, 21));
        assert_eq!(c.grid.lon0, 170.0);
        let bytes = encode(&c, PROCESS_ECMWF_IFS);
        let g = crate::GribFile::from_bytes(&bytes);
        assert!(g.skipped.is_empty(), "{:?}", g.skipped);
        assert_eq!(g.model(), ("ECMWF IFS".to_string(), 1_792_108_800));
        let f = &g.fields[0];
        assert_eq!(f.param, Param::WindU);
        assert_eq!(f.valid_time, 1_792_108_800 + 9 * 3600);
        assert_eq!((f.grid.ni, f.grid.nj, f.grid.lat0, f.grid.lon0), (21, 21, -50.0, 170.0));
        // 175W, 40S: lo = -175
        let v = g.value(Param::WindU, -40.0, -175.0, f.valid_time).unwrap();
        assert!((v - (-4.0 - 1.75)).abs() < 0.006, "{v}");
        let v = g.value(Param::WindU, -40.0, 175.0, f.valid_time).unwrap();
        assert!((v - (-4.0 + 1.75)).abs() < 0.006, "{v}");
    }

    #[test]
    fn missing_values_and_pressure() {
        let mut p = field(Param::Mslp, 0, |la, _| 101_325.0 + la as f32 * 10.0);
        p.category = 3;
        p.surface_type = 101;
        p.values[5] = f32::NAN;
        let c = crop(&p, [-180.0, -90.0, 179.0, -88.0]).unwrap();
        let g = crate::GribFile::from_bytes(&encode(&c, 0));
        let f = &g.fields[0];
        assert_eq!(f.param, Param::Mslp);
        assert!(f.values[5].is_nan() && f.values[4].is_finite());
        assert!((f.values[400] as f64 - (101_325.0 - 890.0)).abs() <= 1.0, "{}", f.values[400]);
        assert_eq!(g.model().0, "ECMWF");
    }
}
