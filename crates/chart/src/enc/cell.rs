//! An S-57 ENC cell: the base cell (`.000`) with its updates (`.001`, `.002`, ...)
//! applied, and the features' geometry built from the vector records (nodes, edges and
//! their topology), ready to be written out for the chart store.

use std::collections::{BTreeMap, HashMap};

use super::iso8211::{Field, Module, Record, Val};

/// A record name: (RCNM record type, RCID).
pub type Name = (u8, u32);

/// A pointer from a feature or edge to a vector record.
#[derive(Debug, Clone, PartialEq)]
pub struct Ptr {
    pub name: Name,
    /// 1 forward, 2 reverse, 255 none
    pub ornt: u8,
    /// 1 exterior, 2 interior, 3 exterior boundary truncated by the cell edge
    pub usag: u8,
    /// edges: 1 beginning node, 2 end node
    pub topi: u8,
    /// 1 masked (not drawn), 2 shown, 255 none
    pub mask: u8,
}

#[derive(Debug, Clone, Default)]
pub struct Vector {
    pub rver: u32,
    pub ptrs: Vec<Ptr>,
    /// raw coordinates: y, x, (depth)
    pub coords: Vec<[i64; 3]>,
    pub three_d: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Feature {
    pub rcid: u32,
    pub objl: u16,
    /// 1 point, 2 line, 3 area, 255 none
    pub prim: u8,
    pub grup: u8,
    pub rver: u32,
    /// feature object identifier: agency, number, subdivision
    pub foid: (u16, u32, u16),
    pub attf: Vec<(u16, String)>,
    pub natf: Vec<(u16, String)>,
    pub fspt: Vec<Ptr>,
    /// feature-to-feature pointers (LNAM, relationship)
    pub ffpt: Vec<(Vec<u8>, u8)>,
}

#[derive(Debug, Clone)]
pub enum Shape {
    /// lon, lat, depth (soundings)
    Points(Vec<[f64; 3]>),
    Lines(Vec<Vec<[f64; 2]>>),
    /// polygons: exterior ring then holes
    Polygons(Vec<Vec<Vec<[f64; 2]>>>),
}

#[derive(Debug, Default)]
pub struct Cell {
    /// DSID DSNM: the file name, such as "NZ40001.000"
    pub name: String,
    /// intended usage 1 overview .. 6 berthing
    pub intu: u8,
    pub edition: String,
    pub update: String,
    pub issue_date: String,
    /// coordinate and sounding multiplication factors
    pub comf: f64,
    pub somf: f64,
    /// compilation scale
    pub cscl: u32,
    pub vectors: HashMap<Name, Vector>,
    pub features: BTreeMap<u32, Feature>,
}

#[derive(Debug, thiserror::Error)]
pub enum CellError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Iso(#[from] super::iso8211::Error),
    #[error("{0}")]
    Other(String),
}

fn int(f: &Field, row: &[Val], label: &str) -> Option<i64> {
    row.get(f.col(label)?)?.int()
}

fn name_of(b: &[u8]) -> Option<Name> {
    if b.len() < 5 {
        return None;
    }
    Some((b[0], u32::from_le_bytes([b[1], b[2], b[3], b[4]])))
}

fn ptrs(f: &Field) -> Vec<Ptr> {
    f.rows
        .iter()
        .filter_map(|r| {
            let name = name_of(r.get(f.col("NAME")?)?.bytes()?)?;
            let g = |l: &str| int(f, r, l).map(|v| v as u8).unwrap_or(255);
            Some(Ptr { name, ornt: g("ORNT"), usag: g("USAG"), topi: g("TOPI"), mask: g("MASK") })
        })
        .collect()
}

fn attrs(f: &Field) -> Vec<(u16, String)> {
    f.rows
        .iter()
        .filter_map(|r| {
            let code = int(f, r, "ATTL")? as u16;
            let v = r.get(f.col("ATVL")?).and_then(|v| match v {
                Val::Text(s) => Some(s.clone()),
                Val::Int(i) => Some(i.to_string()),
                Val::Float(x) => Some(x.to_string()),
                _ => None,
            })?;
            Some((code, v))
        })
        .collect()
}

fn coords(f: &Field, three: bool) -> Vec<[i64; 3]> {
    f.rows.iter().filter_map(|r| Some([int(f, r, "YCOO")?, int(f, r, "XCOO")?, if three { int(f, r, "VE3D").unwrap_or(0) } else { 0 }])).collect()
}

/// An update's attribute values: "\x7f" (DEL) removes the attribute.
fn merge_attrs(into: &mut Vec<(u16, String)>, upd: Vec<(u16, String)>) {
    for (k, v) in upd {
        into.retain(|(a, _)| *a != k);
        if v != "\x7f" {
            into.push((k, v));
        }
    }
}

/// Apply an update instruction (1 insert, 2 delete, 3 modify) to a list at a 1-based index.
fn edit_list<T: Clone>(list: &mut Vec<T>, ui: i64, ix: i64, n: i64, new: &[T]) {
    let ix = (ix.max(1) - 1) as usize;
    let n = n.max(0) as usize;
    match ui {
        1 => {
            let at = ix.min(list.len());
            for (k, v) in new.iter().take(n.max(new.len())).enumerate() {
                list.insert(at + k, v.clone());
            }
        }
        2 => {
            let end = (ix + n).min(list.len());
            if ix < end {
                list.drain(ix..end);
            }
        }
        3 => {
            for (k, v) in new.iter().take(n).enumerate() {
                if let Some(slot) = list.get_mut(ix + k) {
                    *slot = v.clone();
                }
            }
        }
        _ => {}
    }
}

impl Cell {
    /// Read a base cell and apply its updates (in order).
    pub fn open(base: &std::path::Path, updates: &[std::path::PathBuf]) -> Result<Cell, CellError> {
        let mut c = Cell { comf: 1e7, somf: 10.0, ..Default::default() };
        c.apply(&Module::parse(&std::fs::read(base)?)?, false);
        for u in updates {
            c.apply(&Module::parse(&std::fs::read(u)?)?, true);
        }
        Ok(c)
    }

    /// Read from bytes (a decrypted cell, or tests): base then updates.
    pub fn from_bytes(base: &[u8], updates: &[&[u8]]) -> Result<Cell, CellError> {
        let mut c = Cell { comf: 1e7, somf: 10.0, ..Default::default() };
        c.apply(&Module::parse(base)?, false);
        for u in updates {
            c.apply(&Module::parse(u)?, true);
        }
        Ok(c)
    }

    fn apply(&mut self, m: &Module, update: bool) {
        for r in &m.records {
            if let Some(f) = r.field("DSID") {
                if !update {
                    self.name = f.get("DSNM").and_then(|v| v.text()).unwrap_or("").to_string();
                    self.intu = f.int("INTU").unwrap_or(0) as u8;
                    self.edition = f.get("EDTN").and_then(|v| v.text()).unwrap_or("").to_string();
                }
                self.update = f.get("UPDN").and_then(|v| v.text()).unwrap_or("").to_string();
                self.issue_date = f.get("ISDT").and_then(|v| v.text()).unwrap_or("").to_string();
            }
            if let Some(f) = r.field("DSPM") {
                if let Some(v) = f.int("COMF").filter(|v| *v > 0) {
                    self.comf = v as f64;
                }
                if let Some(v) = f.int("SOMF").filter(|v| *v > 0) {
                    self.somf = v as f64;
                }
                self.cscl = f.int("CSCL").unwrap_or(0) as u32;
            }
            if let Some(f) = r.field("VRID") {
                self.vector_record(f, r);
            } else if let Some(f) = r.field("FRID") {
                self.feature_record(f, r);
            }
        }
    }

    fn vector_record(&mut self, f: &Field, r: &Record) {
        let name = (f.int("RCNM").unwrap_or(0) as u8, f.int("RCID").unwrap_or(0) as u32);
        let rver = f.int("RVER").unwrap_or(1) as u32;
        let ruin = f.int("RUIN").unwrap_or(1);
        let three = r.field("SG3D").is_some();
        let new_coords = r.field("SG3D").or(r.field("SG2D")).map(|c| coords(c, three)).unwrap_or_default();
        let new_ptrs = r.field("VRPT").map(ptrs).unwrap_or_default();
        match ruin {
            2 => {
                self.vectors.remove(&name);
            }
            3 => {
                let Some(v) = self.vectors.get_mut(&name) else { return };
                v.rver = rver;
                if let Some(pc) = r.field("VRPC") {
                    edit_list(&mut v.ptrs, pc.int("VPUI").unwrap_or(0), pc.int("VPIX").unwrap_or(1), pc.int("NVPT").unwrap_or(0), &new_ptrs);
                }
                if let Some(cc) = r.field("SGCC") {
                    edit_list(&mut v.coords, cc.int("CCUI").unwrap_or(0), cc.int("CCIX").unwrap_or(1), cc.int("CCNC").unwrap_or(0), &new_coords);
                }
            }
            _ => {
                self.vectors.insert(name, Vector { rver, ptrs: new_ptrs, coords: new_coords, three_d: three });
            }
        }
    }

    fn feature_record(&mut self, f: &Field, r: &Record) {
        let rcid = f.int("RCID").unwrap_or(0) as u32;
        let ruin = f.int("RUIN").unwrap_or(1);
        let rver = f.int("RVER").unwrap_or(1) as u32;
        let attf = r.field("ATTF").map(attrs).unwrap_or_default();
        let natf = r.field("NATF").map(attrs).unwrap_or_default();
        let fspt = r.field("FSPT").map(ptrs).unwrap_or_default();
        let ffpt: Vec<(Vec<u8>, u8)> = r
            .field("FFPT")
            .map(|ff| ff.rows.iter().filter_map(|row| Some((row.get(ff.col("LNAM")?)?.bytes()?.to_vec(), int(ff, row, "RIND").unwrap_or(0) as u8))).collect())
            .unwrap_or_default();
        match ruin {
            2 => {
                self.features.remove(&rcid);
            }
            3 => {
                let Some(ft) = self.features.get_mut(&rcid) else { return };
                ft.rver = rver;
                merge_attrs(&mut ft.attf, attf);
                merge_attrs(&mut ft.natf, natf);
                if let Some(pc) = r.field("FSPC") {
                    edit_list(&mut ft.fspt, pc.int("FSUI").unwrap_or(0), pc.int("FSIX").unwrap_or(1), pc.int("NSPT").unwrap_or(0), &fspt);
                }
                if let Some(pc) = r.field("FFPC") {
                    edit_list(&mut ft.ffpt, pc.int("FFUI").unwrap_or(0), pc.int("FFIX").unwrap_or(1), pc.int("NFPT").unwrap_or(0), &ffpt);
                }
            }
            _ => {
                let foid = r
                    .field("FOID")
                    .map(|o| (o.int("AGEN").unwrap_or(0) as u16, o.int("FIDN").unwrap_or(0) as u32, o.int("FIDS").unwrap_or(0) as u16))
                    .unwrap_or_default();
                self.features.insert(
                    rcid,
                    Feature {
                        rcid,
                        objl: f.int("OBJL").unwrap_or(0) as u16,
                        prim: f.int("PRIM").unwrap_or(255) as u8,
                        grup: f.int("GRUP").unwrap_or(0) as u8,
                        rver,
                        foid,
                        attf,
                        natf,
                        fspt,
                        ffpt,
                    },
                );
            }
        }
    }

    fn xy(&self, c: &[i64; 3]) -> [f64; 2] {
        [c[1] as f64 / self.comf, c[0] as f64 / self.comf]
    }

    /// A node's position.
    fn node(&self, n: Name) -> Option<[f64; 2]> {
        self.vectors.get(&n)?.coords.first().map(|c| self.xy(c))
    }

    /// An edge's points from its beginning node to its end node.
    fn edge(&self, n: Name) -> Option<Vec<[f64; 2]>> {
        let e = self.vectors.get(&n)?;
        let b = e.ptrs.iter().find(|p| p.topi == 1).or(e.ptrs.first())?;
        let en = e.ptrs.iter().find(|p| p.topi == 2).or(e.ptrs.get(1))?;
        let mut pts = vec![self.node(b.name)?];
        pts.extend(e.coords.iter().map(|c| self.xy(c)));
        pts.push(self.node(en.name)?);
        Some(pts)
    }

    /// A feature's geometry (None for features without one, such as collections).
    pub fn shape(&self, f: &Feature) -> Option<Shape> {
        match f.prim {
            1 => {
                let mut pts = Vec::new();
                for p in &f.fspt {
                    let v = self.vectors.get(&p.name)?;
                    for c in &v.coords {
                        let [x, y] = self.xy(c);
                        pts.push([x, y, if v.three_d { c[2] as f64 / self.somf } else { f64::NAN }]);
                    }
                }
                (!pts.is_empty()).then_some(Shape::Points(pts))
            }
            2 => {
                let mut lines: Vec<Vec<[f64; 2]>> = Vec::new();
                for p in &f.fspt {
                    let Some(mut e) = self.edge(p.name) else { continue };
                    if p.ornt == 2 {
                        e.reverse();
                    }
                    match lines.last_mut() {
                        Some(l) if l.last() == e.first() => l.extend_from_slice(&e[1..]),
                        _ => lines.push(e),
                    }
                }
                (!lines.is_empty()).then_some(Shape::Lines(lines))
            }
            3 => {
                let rings = self.rings(f);
                let mut polys: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
                let mut holes = Vec::new();
                for (ring, usag) in rings {
                    if usag == 2 {
                        holes.push(ring);
                    } else {
                        polys.push(vec![ring]);
                    }
                }
                if polys.is_empty() {
                    return None;
                }
                for h in holes {
                    let k = if polys.len() == 1 { 0 } else { polys.iter().position(|p| contains(&p[0], h[0])).unwrap_or(0) };
                    polys[k].push(h);
                }
                // exterior counter-clockwise, holes clockwise (OGC)
                for p in polys.iter_mut() {
                    for (i, r) in p.iter_mut().enumerate() {
                        if (area(r) > 0.0) != (i == 0) {
                            r.reverse();
                        }
                    }
                }
                Some(Shape::Polygons(polys))
            }
            _ => None,
        }
    }

    /// An area's boundary edges joined into closed rings, each with its usage (exterior/interior).
    fn rings(&self, f: &Feature) -> Vec<(Vec<[f64; 2]>, u8)> {
        let mut parts: Vec<(Vec<[f64; 2]>, u8)> = Vec::new();
        for p in &f.fspt {
            let Some(mut e) = self.edge(p.name) else { continue };
            if p.ornt == 2 {
                e.reverse();
            }
            parts.push((e, if p.usag == 2 { 2 } else { 1 }));
        }
        let mut out = Vec::new();
        let mut used = vec![false; parts.len()];
        for i in 0..parts.len() {
            if used[i] {
                continue;
            }
            used[i] = true;
            let (mut ring, usag) = parts[i].clone();
            // edges come in order round each ring; if not, find the one that carries on
            let mut guard = 0;
            while ring.first() != ring.last() && guard < parts.len() {
                guard += 1;
                let end = *ring.last().unwrap();
                let next = (0..parts.len()).find(|&j| !used[j] && (parts[j].0.first() == Some(&end) || parts[j].0.last() == Some(&end)));
                let Some(j) = next else { break };
                used[j] = true;
                let mut e = parts[j].0.clone();
                if e.first() != Some(&end) {
                    e.reverse();
                }
                ring.extend_from_slice(&e[1..]);
            }
            if ring.first() != ring.last() {
                ring.push(ring[0]);
            }
            if ring.len() >= 4 {
                out.push((ring, usag));
            }
        }
        out
    }

    /// Text value of an attribute by its acronym.
    pub fn attr<'a>(&self, f: &'a Feature, acronym: &str) -> Option<&'a str> {
        f.attf.iter().chain(&f.natf).find(|(k, _)| super::catalogue::attribute(*k).is_some_and(|a| a.0 == acronym)).map(|(_, v)| v.as_str())
    }
}

fn area(r: &[[f64; 2]]) -> f64 {
    r.windows(2).map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1]).sum::<f64>() / 2.0
}

fn contains(r: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for w in r.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}
