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
