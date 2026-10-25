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
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Read a shapefile's shapes, with the attributes from its `.dbf` (if given) and the
/// projection from its `.prj` text (if given).
pub fn read(shp: &[u8], dbf: Option<&[u8]>, prj: Option<&str>) -> Vec<Shape> {
    let attrs = dbf.map(read_dbf).unwrap_or_default();
    let nztm = prj.is_some_and(|p| {
        let p = p.to_ascii_lowercase();
        p.contains("transverse_mercator")
            && (p.contains("new_zealand") || p.contains("nztm") || p.contains("173"))
    });
    let mut out = Vec::new();
    if shp.len() < 100 {
        return out;
    }
    let be_i32 = |o: usize| {
        shp.get(o..o + 4)
            .map(|x| i32::from_be_bytes([x[0], x[1], x[2], x[3]]))
    };
    let le_i32 = |o: usize| {
        shp.get(o..o + 4)
            .map(|x| i32::from_le_bytes([x[0], x[1], x[2], x[3]]))
    };
    let le_f64 = |o: usize| {
        shp.get(o..o + 8)
            .map(|x| f64::from_le_bytes(x.try_into().unwrap()))
    };
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
            let starts: Vec<usize> = (0..np)
                .filter_map(|i| le_i32(content + 44 + i * 4))
                .map(|v| v.max(0) as usize)
                .collect();
            let pts0 = content + 44 + np * 4;
            let mut parts = Vec::new();
            for (k, &s) in starts.iter().enumerate() {
                let e = starts.get(k + 1).copied().unwrap_or(npt).min(npt);
                let part: Vec<Pt> = (s..e)
                    .filter_map(|i| Some([le_f64(pts0 + i * 16)?, le_f64(pts0 + i * 16 + 8)?]))
                    .map(|p| {
                        if nztm || p[0].abs() > 1000.0 {
                            nztm_to_lonlat(p)
                        } else {
                            p
                        }
                    })
                    .collect();
                if part.len() >= 2 {
                    parts.push(part);
                }
            }
            if !parts.is_empty() {
                out.push(Shape {
                    polygon: matches!(ty, 5 | 15 | 25),
                    parts,
                    attrs: attrs.get(rec).cloned().unwrap_or_default(),
                });
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
        let name: String = b[o..o + 11]
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as char)
            .collect();
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
            let v: String = b
                .get(p..p + len)
                .unwrap_or(&[])
                .iter()
                .map(|&c| c as char)
                .collect();
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
    let (a, f) = (6_378_137.0_f64, 1.0_f64 / 298.257_222_101);
    let (k0, lon0, fe, fn_) = (0.9996, 173.0_f64.to_radians(), 1_600_000.0, 10_000_000.0);
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
            * (d * d / 2.0
                - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d.powi(4) / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1 - 252.0 * ep2 - 3.0 * c1 * c1)
                    * d.powi(6)
                    / 720.0);
    let lon = lon0
        + (d - (1.0 + 2.0 * t1 + c1) * d.powi(3) / 6.0
            + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1)
                * d.powi(5)
                / 120.0)
            / c;
    [lon.to_degrees(), lat.to_degrees()]
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
        let shp = polyline_shp(&[
            vec![[170.0, -40.0], [171.0, -41.0]],
            vec![[175.0, -35.0], [176.0, -36.0], [177.0, -36.5]],
        ]);
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
        assert!(
            (p[0] - 172.739_194).abs() < 1e-5 && (p[1] + 34.444_066).abs() < 1e-5,
            "{p:?}"
        );
        let p = nztm_to_lonlat([1_200_000.0, 4_800_000.0]); // checked with PROJ
        assert!(
            (p[0] - 167.754_448).abs() < 1e-5 && (p[1] + 46.833_185).abs() < 1e-5,
            "{p:?}"
        );
        // a point read as NZTM because its numbers are far too big for degrees
        let shp = polyline_shp(&[vec![
            [1_576_041.15, 6_188_574.24],
            [1_600_000.0, 6_000_000.0],
        ]]);
        let s = read(&shp, None, None);
        assert!((s[0].parts[0][0][0] - 172.739_194).abs() < 1e-5);
    }
}
