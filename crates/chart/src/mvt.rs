//! Mapbox Vector Tile (MVT 2.1) encoder: tile coordinates in, protobuf out.

pub const EXTENT: u32 = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    F64(f64),
    UInt(u64),
}

pub enum TileGeom {
    /// Points in tile pixels.
    Points(Vec<[i32; 2]>),
    Lines(Vec<Vec<[i32; 2]>>),
    /// Rings with MVT winding already applied (exterior positive area, holes negative).
    Rings(Vec<Vec<[i32; 2]>>),
}

pub struct Layer {
    pub name: String,
    keys: Vec<String>,
    values: Vec<Value>,
    features: Vec<Vec<u8>>,
}

impl Layer {
    pub fn new(name: &str) -> Self {
        Self { name: name.into(), keys: vec![], values: vec![], features: vec![] }
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    pub fn len(&self) -> usize {
        self.features.len()
    }

    pub fn add(&mut self, geom: &TileGeom, props: &[(&str, Value)]) {
        let (gtype, cmds) = match geom {
            TileGeom::Points(p) => (1, encode_points(p)),
            TileGeom::Lines(l) => (2, encode_paths(l, false)),
            TileGeom::Rings(r) => (3, encode_paths(r, true)),
        };
        if cmds.is_empty() {
            return;
        }
        let mut tags = Vec::with_capacity(props.len() * 2);
        for (k, v) in props {
            tags.push(self.key(k));
            tags.push(self.value(v));
        }
        let mut f = Vec::new();
        packed(&mut f, 2, &tags);
        field_varint(&mut f, 3, gtype);
        packed(&mut f, 4, &cmds);
        self.features.push(f);
    }

    fn key(&mut self, k: &str) -> u32 {
        match self.keys.iter().position(|x| x == k) {
            Some(i) => i as u32,
            None => {
                self.keys.push(k.into());
                (self.keys.len() - 1) as u32
            }
        }
    }

    fn value(&mut self, v: &Value) -> u32 {
        match self.values.iter().position(|x| x == v) {
            Some(i) => i as u32,
            None => {
                self.values.push(v.clone());
                (self.values.len() - 1) as u32
            }
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let mut l = Vec::new();
        field_varint(&mut l, 15, 2);
        field_bytes(&mut l, 1, self.name.as_bytes());
        for f in &self.features {
            field_bytes(&mut l, 2, f);
        }
        for k in &self.keys {
            field_bytes(&mut l, 3, k.as_bytes());
        }
        for v in &self.values {
            let mut vb = Vec::new();
            match v {
                Value::Str(s) => field_bytes(&mut vb, 1, s.as_bytes()),
                Value::F64(d) => {
                    key(&mut vb, 3, 1);
                    vb.extend_from_slice(&d.to_le_bytes());
                }
                Value::UInt(u) => field_varint(&mut vb, 5, *u),
            }
            field_bytes(&mut l, 4, &vb);
        }
        field_varint(&mut l, 5, EXTENT as u64);
        field_bytes(out, 3, &l);
    }
}

/// Encode layers into one tile.
pub fn encode_tile(layers: &[Layer]) -> Vec<u8> {
    let mut out = Vec::new();
    for l in layers.iter().filter(|l| !l.is_empty()) {
        l.encode(&mut out);
    }
    out
}

fn cmd(id: u32, count: u32) -> u32 {
    (id & 7) | (count << 3)
}

fn zz(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

fn encode_points(pts: &[[i32; 2]]) -> Vec<u32> {
    if pts.is_empty() {
        return vec![];
    }
    let mut c = vec![cmd(1, pts.len() as u32)];
    let (mut cx, mut cy) = (0, 0);
    for p in pts {
        c.push(zz(p[0] - cx));
        c.push(zz(p[1] - cy));
        cx = p[0];
        cy = p[1];
    }
    c
}

fn encode_paths(paths: &[Vec<[i32; 2]>], closed: bool) -> Vec<u32> {
    let mut c = Vec::new();
    let (mut cx, mut cy) = (0, 0);
    for path in paths {
        // A closed ring repeats its first point; MVT closes with ClosePath instead.
        let pts: &[[i32; 2]] = if closed && path.len() > 1 && path.first() == path.last() {
            &path[..path.len() - 1]
        } else {
            path
        };
        let min = if closed { 3 } else { 2 };
        if pts.len() < min {
            continue;
        }
        c.push(cmd(1, 1));
        c.push(zz(pts[0][0] - cx));
        c.push(zz(pts[0][1] - cy));
        cx = pts[0][0];
        cy = pts[0][1];
        c.push(cmd(2, (pts.len() - 1) as u32));
        for p in &pts[1..] {
            c.push(zz(p[0] - cx));
            c.push(zz(p[1] - cy));
            cx = p[0];
            cy = p[1];
        }
        if closed {
            c.push(cmd(7, 1));
        }
    }
    c
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn key(out: &mut Vec<u8>, field: u32, wire: u32) {
    varint(out, ((field << 3) | wire) as u64);
}

fn field_varint(out: &mut Vec<u8>, field: u32, v: u64) {
    key(out, field, 0);
    varint(out, v);
}

fn field_bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    key(out, field, 2);
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn packed(out: &mut Vec<u8>, field: u32, vals: &[u32]) {
    let mut b = Vec::new();
    for v in vals {
        varint(&mut b, *v as u64);
    }
    field_bytes(out, field, &b);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zigzag() {
        assert_eq!(zz(0), 0);
        assert_eq!(zz(-1), 1);
        assert_eq!(zz(1), 2);
        assert_eq!(zz(-2), 3);
    }

    #[test]
    fn spec_example_point() {
        // MVT spec example: point (25, 17) -> [9, 50, 34]
        assert_eq!(encode_points(&[[25, 17]]), vec![9, 50, 34]);
    }

    #[test]
    fn spec_example_polygon() {
        // MVT spec example polygon (3,6) (8,12) (20,34)
        let r = vec![vec![[3, 6], [8, 12], [20, 34], [3, 6]]];
        assert_eq!(encode_paths(&r, true), vec![9, 6, 12, 18, 10, 12, 24, 44, 15]);
    }
}

/// For diagnostics: every feature of `layer` in an encoded tile as
/// (cls, band, minz, maxz, x, y of its first point). A small protobuf reader, not a general decoder.
pub fn debug_points(tile: &[u8], layer: &str) -> Vec<(String, u64, f64, f64, i64, i64)> {
    fn varint(b: &[u8], i: &mut usize) -> u64 {
        let (mut v, mut s) = (0u64, 0);
        while *i < b.len() {
            let x = b[*i];
            *i += 1;
            v |= ((x & 0x7f) as u64) << s;
            if x & 0x80 == 0 {
                break;
            }
            s += 7;
        }
        v
    }
    /// (field, wire type, bytes for length-delimited / value for varint and fixed)
    fn fields(b: &[u8]) -> Vec<(u64, u64, &[u8], u64)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            let k = varint(b, &mut i);
            let (f, w) = (k >> 3, k & 7);
            match w {
                0 => out.push((f, w, &b[0..0], varint(b, &mut i))),
                1 => { out.push((f, w, &b[i..i + 8], 0)); i += 8; }
                2 => { let n = varint(b, &mut i) as usize; out.push((f, w, &b[i..i + n], 0)); i += n; }
                5 => { out.push((f, w, &b[i..i + 4], 0)); i += 4; }
                _ => break,
            }
        }
        out
    }
    let packed = |b: &[u8]| { let mut v = Vec::new(); let mut i = 0; while i < b.len() { v.push(varint(b, &mut i)); } v };
    let mut out = Vec::new();
    for (f, _, lb, _) in fields(tile) {
        if f != 3 { continue; }
        let lf = fields(lb);
        let name = lf.iter().find(|x| x.0 == 1).map(|x| String::from_utf8_lossy(x.2).to_string()).unwrap_or_default();
        if name != layer { continue; }
        let keys: Vec<String> = lf.iter().filter(|x| x.0 == 3).map(|x| String::from_utf8_lossy(x.2).to_string()).collect();
        let vals: Vec<Value> = lf.iter().filter(|x| x.0 == 4).map(|x| {
            let vf = fields(x.2);
            match vf.first() {
                Some((1, _, s, _)) => Value::Str(String::from_utf8_lossy(s).to_string()),
                Some((3, _, d, _)) => Value::F64(f64::from_le_bytes(d[..8].try_into().unwrap())),
                Some((5, _, _, u)) => Value::UInt(*u),
                _ => Value::UInt(0),
            }
        }).collect();
        for (ff, _, fb, _) in lf.iter() {
            if *ff != 2 { continue; }
            let feat = fields(fb);
            let tags = feat.iter().find(|x| x.0 == 2).map(|x| packed(x.2)).unwrap_or_default();
            let geom = feat.iter().find(|x| x.0 == 4).map(|x| packed(x.2)).unwrap_or_default();
            let (mut cls, mut band, mut minz, mut maxz) = (String::new(), 0, 0.0, 99.0);
            for t in tags.chunks(2) {
                let (Some(k), Some(v)) = (keys.get(t[0] as usize), vals.get(t[1] as usize)) else { continue };
                match (k.as_str(), v) {
                    ("cls", Value::Str(s)) => cls = s.clone(),
                    ("band", Value::UInt(u)) => band = *u,
                    ("minz", Value::F64(m)) => minz = *m,
                    ("maxz", Value::F64(m)) => maxz = *m,
                    _ => {}
                }
            }
            if geom.len() >= 3 && geom[0] & 7 == 1 {
                let z = |v: u64| ((v >> 1) as i64) ^ -((v & 1) as i64);
                out.push((cls, band, minz, maxz, z(geom[1]), z(geom[2])));
            }
        }
    }
    out
}
