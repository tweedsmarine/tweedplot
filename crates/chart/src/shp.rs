//! Reading ESRI shapefiles (`.shp` geometry, `.dbf` attributes, `.prj` projection),
//! for data that is only published that way (LINZ maritime boundaries).
//!
//! Lines and polygons (shape types 3, 5 and their Z and M forms) are read; points are
//! skipped. Coordinates in NZTM2000 (EPSG:2193) are turned into longitude and
//! latitude; anything else is taken as longitude and latitude already.

use crate::geom::Pt;

/// One shape: its parts (polygon rings or line parts) and attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub polygon: bool,
    pub parts: Vec<Vec<Pt>>,
    pub attrs: Vec<(String, String)>,
}

impl Shape {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Read a shapefile's shapes, with the attributes from its `.dbf` (if given) and the
/// projection from its `.prj` text (if given).
pub fn read(shp: &[u8], dbf: Option<&[u8]>, prj: Option<&str>) -> Vec<Shape> {
    let attrs = dbf.map(read_dbf).unwrap_or_default();
    let nztm = prj.is_some_and(|p| {
        let p = p.to_ascii_lowercase();
        p.contains("transverse_mercator") && (p.contains("new_zealand") || p.contains("nztm") || p.contains("173"))
    });
    let mut out = Vec::new();
    if shp.len() < 100 {
        return out;
    }
    let be_i32 = |o: usize| shp.get(o..o + 4).map(|x| i32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    let le_i32 = |o: usize| shp.get(o..o + 4).map(|x| i32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    let le_f64 = |o: usize| shp.get(o..o + 8).map(|x| f64::from_le_bytes(x.try_into().unwrap()));
    let mut o = 100;
    let mut rec = 0usize;
    while o + 8 <= shp.len() {
        let Some(len_words) = be_i32(o + 4) else {
            break;
        };
        let content = o + 8;
        let end = content + len_words.max(0) as usize * 2;
        if end > shp.len() {
            break;
        }
        let ty = le_i32(content).unwrap_or(0);
        if matches!(ty, 3 | 13 | 23 | 5 | 15 | 25) {
            let np = le_i32(content + 36).unwrap_or(0).max(0) as usize;
            let npt = le_i32(content + 40).unwrap_or(0).max(0) as usize;
            let starts: Vec<usize> = (0..np).filter_map(|i| le_i32(content + 44 + i * 4)).map(|v| v.max(0) as usize).collect();
            let pts0 = content + 44 + np * 4;
            let mut parts = Vec::new();
            for (k, &s) in starts.iter().enumerate() {
                let e = starts.get(k + 1).copied().unwrap_or(npt).min(npt);
                let part: Vec<Pt> = (s..e)
                    .filter_map(|i| Some([le_f64(pts0 + i * 16)?, le_f64(pts0 + i * 16 + 8)?]))
                    .map(|p| if nztm || p[0].abs() > 1000.0 { nztm_to_lonlat(p) } else { p })
                    .collect();
                if part.len() >= 2 {
                    parts.push(part);
                }
            }
            if !parts.is_empty() {
                out.push(Shape { polygon: matches!(ty, 5 | 15 | 25), parts, attrs: attrs.get(rec).cloned().unwrap_or_default() });
            }
        }
        rec += 1;
        o = end;
    }
    out
}

/// A dBase III table: one list of (field, value) per record, values trimmed.
pub fn read_dbf(b: &[u8]) -> Vec<Vec<(String, String)>> {
    if b.len() < 32 {
        return Vec::new();
    }
    let n = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
    let header = u16::from_le_bytes([b[8], b[9]]) as usize;
    let reclen = u16::from_le_bytes([b[10], b[11]]) as usize;
    let mut fields = Vec::new();
    let mut o = 32;
    while o + 32 <= b.len() && b[o] != 0x0d && o + 32 <= header {
        let name: String = b[o..o + 11].iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
        fields.push((name, b[o + 16] as usize));
        o += 32;
    }
    let mut out = Vec::with_capacity(n);
    for r in 0..n {
        let s = header + r * reclen;
        if s + reclen > b.len() || reclen == 0 {
            break;
        }
        let mut p = s + 1; // deletion flag
        let mut rec = Vec::with_capacity(fields.len());
        for (name, len) in &fields {
            let v: String = b.get(p..p + len).unwrap_or(&[]).iter().map(|&c| c as char).collect();
            rec.push((name.clone(), v.trim().to_string()));
            p += len;
        }
        out.push(rec);
    }
    out
}

/// NZ Transverse Mercator 2000 (GRS80, central meridian 173°E, scale 0.9996, false
/// easting 1 600 000 m, false northing 10 000 000 m) to longitude and latitude.
pub fn nztm_to_lonlat(p: Pt) -> Pt {
    tm_to_lonlat(p, Tm::NZTM)
}

/// A transverse Mercator projection on GRS80/WGS84: central meridian (°), scale, false
/// easting and northing. NZTM2000 and the UTM zones LINZ's depth models come in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tm {
    pub lon0: f64,
    pub k0: f64,
    pub fe: f64,
    pub fn_: f64,
}

impl Tm {
    /// NZ Transverse Mercator 2000 (EPSG:2193).
    pub const NZTM: Tm = Tm { lon0: 173.0, k0: 0.9996, fe: 1_600_000.0, fn_: 10_000_000.0 };
    /// UTM zone `zone`, south (EPSG:327xx) or north (326xx).
    pub fn utm(zone: u8, south: bool) -> Tm {
        Tm { lon0: zone as f64 * 6.0 - 183.0, k0: 0.9996, fe: 500_000.0, fn_: if south { 10_000_000.0 } else { 0.0 } }
    }
    /// The projection of an EPSG code TweedPlot knows: 2193, 32601–32660, 32701–32760.
    pub fn from_epsg(code: u32) -> Option<Tm> {
        match code {
            2193 => Some(Tm::NZTM),
            32601..=32660 => Some(Tm::utm((code - 32600) as u8, false)),
            32701..=32760 => Some(Tm::utm((code - 32700) as u8, true)),
            _ => None,
        }
    }
}

/// Transverse Mercator (easting, northing) to longitude and latitude.
pub fn tm_to_lonlat(p: Pt, tm: Tm) -> Pt {
    let (a, f) = (6_378_137.0_f64, 1.0_f64 / 298.257_222_101);
    let (k0, lon0, fe, fn_) = (tm.k0, tm.lon0.to_radians(), tm.fe, tm.fn_);
    let e2 = f * (2.0 - f);
    let ep2 = e2 / (1.0 - e2);
    let x = p[0] - fe;
    let y = p[1] - fn_;
    let m = y / k0;
    let mu = m / (a * (1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2.powi(3) / 256.0));
    let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());
    let phi1 = mu
        + (3.0 * e1 / 2.0 - 27.0 * e1.powi(3) / 32.0) * (2.0 * mu).sin()
        + (21.0 * e1 * e1 / 16.0 - 55.0 * e1.powi(4) / 32.0) * (4.0 * mu).sin()
        + (151.0 * e1.powi(3) / 96.0) * (6.0 * mu).sin()
        + (1097.0 * e1.powi(4) / 512.0) * (8.0 * mu).sin();
    let (s, c, t) = (phi1.sin(), phi1.cos(), phi1.tan());
    let n1 = a / (1.0 - e2 * s * s).sqrt();
    let t1 = t * t;
    let c1 = ep2 * c * c;
    let r1 = a * (1.0 - e2) / (1.0 - e2 * s * s).powf(1.5);
    let d = x / (n1 * k0);
    let lat = phi1
        - (n1 * t / r1)
            * (d * d / 2.0 - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d.powi(4) / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1 - 252.0 * ep2 - 3.0 * c1 * c1) * d.powi(6) / 720.0);
    let lon = lon0
        + (d - (1.0 + 2.0 * t1 + c1) * d.powi(3) / 6.0 + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1) * d.powi(5) / 120.0) / c;
    [lon.to_degrees(), lat.to_degrees()]
}

/// Longitude and latitude to NZ Transverse Mercator 2000 (easting, northing).
pub fn lonlat_to_nztm(p: Pt) -> Pt {
    lonlat_to_tm(p, Tm::NZTM)
}

/// Longitude and latitude to transverse Mercator (easting, northing).
pub fn lonlat_to_tm(p: Pt, tm: Tm) -> Pt {
    let (a, f) = (6_378_137.0_f64, 1.0_f64 / 298.257_222_101);
    let (k0, lon0, fe, fn_) = (tm.k0, tm.lon0.to_radians(), tm.fe, tm.fn_);
    let e2 = f * (2.0 - f);
    let ep2 = e2 / (1.0 - e2);
    // the longitude on the same side of the date line as the central meridian
    let mut lon = p[0];
    while lon - tm.lon0 > 180.0 {
        lon -= 360.0;
    }
    while lon - tm.lon0 < -180.0 {
        lon += 360.0;
    }
    let (phi, lam) = (p[1].to_radians(), lon.to_radians());
    let (s, c) = (phi.sin(), phi.cos());
    let n = a / (1.0 - e2 * s * s).sqrt();
    let t = phi.tan().powi(2);
    let cc = ep2 * c * c;
    let aa = (lam - lon0) * c;
    let m = a
        * ((1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2.powi(3) / 256.0) * phi
            - (3.0 * e2 / 8.0 + 3.0 * e2 * e2 / 32.0 + 45.0 * e2.powi(3) / 1024.0) * (2.0 * phi).sin()
            + (15.0 * e2 * e2 / 256.0 + 45.0 * e2.powi(3) / 1024.0) * (4.0 * phi).sin()
            - (35.0 * e2.powi(3) / 3072.0) * (6.0 * phi).sin());
    let x = fe + k0 * n * (aa + (1.0 - t + cc) * aa.powi(3) / 6.0 + (5.0 - 18.0 * t + t * t + 72.0 * cc - 58.0 * ep2) * aa.powi(5) / 120.0);
    let y = fn_
        + k0 * (m + n
            * phi.tan()
            * (aa * aa / 2.0
                + (5.0 - t + 9.0 * cc + 4.0 * cc * cc) * aa.powi(4) / 24.0
                + (61.0 - 58.0 * t + t * t + 600.0 * cc - 330.0 * ep2) * aa.powi(6) / 720.0));
    [x, y]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A shapefile of polylines (type 3), one record per line.
    pub fn polyline_shp(lines: &[Vec<Pt>]) -> Vec<u8> {
        let mut body = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            let mut c = Vec::new();
            c.extend_from_slice(&3i32.to_le_bytes());
            let b = crate::gpkgw::bounds(l);
            for v in b {
                c.extend_from_slice(&v.to_le_bytes());
            }
            c.extend_from_slice(&1i32.to_le_bytes());
            c.extend_from_slice(&(l.len() as i32).to_le_bytes());
            c.extend_from_slice(&0i32.to_le_bytes());
            for p in l {
                c.extend_from_slice(&p[0].to_le_bytes());
                c.extend_from_slice(&p[1].to_le_bytes());
            }
            body.extend_from_slice(&(i as i32 + 1).to_be_bytes());
            body.extend_from_slice(&((c.len() / 2) as i32).to_be_bytes());
            body.extend_from_slice(&c);
        }
        let mut v = vec![0u8; 100];
        v[0..4].copy_from_slice(&9994i32.to_be_bytes());
        v[24..28].copy_from_slice(&(((100 + body.len()) / 2) as i32).to_be_bytes());
        v[28..32].copy_from_slice(&1000i32.to_le_bytes());
        v[32..36].copy_from_slice(&3i32.to_le_bytes());
        v.extend_from_slice(&body);
        v
    }

    /// A dBase table with one character field.
    pub fn dbf(field: &str, values: &[&str]) -> Vec<u8> {
        let len = 20usize;
        let header = 32 + 32 + 1;
        let mut v = vec![0u8; 32];
        v[0] = 3;
        v[4..8].copy_from_slice(&(values.len() as u32).to_le_bytes());
        v[8..10].copy_from_slice(&(header as u16).to_le_bytes());
        v[10..12].copy_from_slice(&((1 + len) as u16).to_le_bytes());
        let mut f = [0u8; 32];
        f[..field.len()].copy_from_slice(field.as_bytes());
        f[11] = b'C';
        f[16] = len as u8;
        v.extend_from_slice(&f);
        v.push(0x0d);
        for s in values {
            v.push(b' ');
            let mut cell = vec![b' '; len];
            cell[..s.len()].copy_from_slice(s.as_bytes());
            v.extend_from_slice(&cell);
        }
        v.push(0x1a);
        v
    }

    #[test]
    fn reads_lines_and_attributes() {
        let shp = polyline_shp(&[vec![[170.0, -40.0], [171.0, -41.0]], vec![[175.0, -35.0], [176.0, -36.0], [177.0, -36.5]]]);
        let dbf = dbf("NAME", &["Twelve mile", "EEZ"]);
        let s = read(&shp, Some(&dbf), None);
        assert_eq!(s.len(), 2);
        assert!(!s[0].polygon);
        assert_eq!(s[1].parts[0].len(), 3);
        assert_eq!(s[0].attr("name"), Some("Twelve mile"));
        assert_eq!(s[1].attr("NAME"), Some("EEZ"));
    }

    #[test]
    fn nztm_points() {
        // checked with PROJ (EPSG:2193 to 4326)
        let p = nztm_to_lonlat([1_576_041.150, 6_188_574.240]);
        assert!((p[0] - 172.739_194).abs() < 1e-5 && (p[1] + 34.444_066).abs() < 1e-5, "{p:?}");
        let p = nztm_to_lonlat([1_200_000.0, 4_800_000.0]); // checked with PROJ
        assert!((p[0] - 167.754_448).abs() < 1e-5 && (p[1] + 46.833_185).abs() < 1e-5, "{p:?}");
        // and back again (checked with PROJ too)
        let e = lonlat_to_nztm([172.739_193_967, -34.444_065_991]);
        assert!((e[0] - 1_576_041.15).abs() < 0.01 && (e[1] - 6_188_574.24).abs() < 0.01, "{e:?}");
        let e = lonlat_to_nztm([167.754_447_719, -46.833_184_968]);
        assert!((e[0] - 1_200_000.0).abs() < 0.01 && (e[1] - 4_800_000.0).abs() < 0.01, "{e:?}");
        // a point read as NZTM because its numbers are far too big for degrees
        let shp = polyline_shp(&[vec![[1_576_041.15, 6_188_574.24], [1_600_000.0, 6_000_000.0]]]);
        let s = read(&shp, None, None);
        assert!((s[0].parts[0][0][0] - 172.739_194).abs() < 1e-5);
    }
}
