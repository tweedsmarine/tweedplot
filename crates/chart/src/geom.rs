//! Geometry types and the GeoPackage binary geometry format (GP header + WKB).

pub type Pt = [f64; 2];
pub type Ring = Vec<Pt>;

#[derive(Debug, Clone, PartialEq)]
pub enum Geom {
    Points(Vec<Pt>),
    Lines(Vec<Vec<Pt>>),
    /// Polygons, each a list of rings: exterior first, then holes.
    Polygons(Vec<Vec<Ring>>),
}

/// Parse a GeoPackage geometry blob. Z/M values are read and dropped.
pub fn parse_gpkg(blob: &[u8]) -> Option<Geom> {
    if blob.len() < 8 || &blob[0..2] != b"GP" {
        return None;
    }
    let flags = blob[3];
    if flags & 0x10 != 0 {
        return None; // empty geometry
    }
    let env = match (flags >> 1) & 0x07 {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        _ => return None,
    };
    let mut r = Wkb { b: &blob[8 + env..], p: 0, le: true };
    let mut out = Acc::default();
    r.geometry(&mut out)?;
    out.finish()
}

#[derive(Default)]
struct Acc {
    points: Vec<Pt>,
    lines: Vec<Vec<Pt>>,
    polys: Vec<Vec<Ring>>,
}

impl Acc {
    fn finish(self) -> Option<Geom> {
        if !self.polys.is_empty() {
            Some(Geom::Polygons(self.polys))
        } else if !self.lines.is_empty() {
            Some(Geom::Lines(self.lines))
        } else if !self.points.is_empty() {
            Some(Geom::Points(self.points))
        } else {
            None
        }
    }
}

struct Wkb<'a> {
    b: &'a [u8],
    p: usize,
    le: bool,
}

impl Wkb<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let s: [u8; 4] = self.b.get(self.p..self.p + 4)?.try_into().ok()?;
        self.p += 4;
        Some(if self.le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }
    fn f64(&mut self) -> Option<f64> {
        let s: [u8; 8] = self.b.get(self.p..self.p + 8)?.try_into().ok()?;
        self.p += 8;
        Some(if self.le { f64::from_le_bytes(s) } else { f64::from_be_bytes(s) })
    }
    fn pt(&mut self, dims: usize) -> Option<Pt> {
        let x = self.f64()?;
        let y = self.f64()?;
        for _ in 2..dims {
            self.f64()?;
        }
        Some([x, y])
    }
    fn seq(&mut self, dims: usize) -> Option<Vec<Pt>> {
        let n = self.u32()? as usize;
        (0..n).map(|_| self.pt(dims)).collect()
    }

    fn geometry(&mut self, out: &mut Acc) -> Option<()> {
        self.le = self.u8()? == 1;
        let raw = self.u32()?;
        // ISO WKB (1000s for Z/M) and EWKB (high flag bits) both handled.
        let ewkb_z = raw & 0x8000_0000 != 0;
        let ewkb_m = raw & 0x4000_0000 != 0;
        let code = raw & 0x0FFF_FFFF;
        let (base, iso_dims) = (code % 1000, code / 1000);
        let dims = match iso_dims {
            1 | 2 => 3,
            3 => 4,
            _ => 2 + ewkb_z as usize + ewkb_m as usize,
        };
        match base {
            1 => out.points.push(self.pt(dims)?),
            2 => out.lines.push(self.seq(dims)?),
            3 => {
                let n = self.u32()? as usize;
                let rings = (0..n).map(|_| self.seq(dims)).collect::<Option<Vec<_>>>()?;
                out.polys.push(rings);
            }
            4..=7 => {
                let n = self.u32()?;
                for _ in 0..n {
                    self.geometry(out)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gp_header() -> Vec<u8> {
        // "GP", version 0, flags: little endian, no envelope; srs 4326
        let mut v = vec![b'G', b'P', 0, 0x01];
        v.extend_from_slice(&4326i32.to_le_bytes());
        v
    }

    #[test]
    fn point() {
        let mut b = gp_header();
        b.push(1);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&173.25f64.to_le_bytes());
        b.extend_from_slice(&(-41.25f64).to_le_bytes());
        assert_eq!(parse_gpkg(&b), Some(Geom::Points(vec![[173.25, -41.25]])));
    }

    #[test]
    fn multipolygon_with_envelope() {
        let mut b = vec![b'G', b'P', 0, 0x03];
        b.extend_from_slice(&4326i32.to_le_bytes());
        for v in [0.0f64, 1.0, 0.0, 1.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(1);
        b.extend_from_slice(&6u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(1);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&4u32.to_le_bytes());
        for (x, y) in [(0.0f64, 0.0f64), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)] {
            b.extend_from_slice(&x.to_le_bytes());
            b.extend_from_slice(&y.to_le_bytes());
        }
        let g = parse_gpkg(&b).unwrap();
        assert_eq!(g, Geom::Polygons(vec![vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]]));
    }
}
