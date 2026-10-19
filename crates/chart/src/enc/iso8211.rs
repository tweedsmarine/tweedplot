//! ISO/IEC 8211 reader: the file format of S-57 ENC cells (`.000`) and their updates.
//!
//! A file is a data descriptive record (DDR), which describes each field (its subfield
//! labels and their formats), then data records (DR) holding the fields. Only what S-57
//! uses is supported: ASCII text, integers and reals (fixed width or ended by the unit
//! terminator), little-endian binary integers (`b1n` unsigned, `b2n` signed) and bit
//! strings (`B(n)`), with repeating subfield groups (`*` in the array descriptor).

use std::collections::HashMap;
use std::sync::Arc;

const UT: u8 = 0x1f; // unit terminator
const FT: u8 = 0x1e; // field terminator

#[derive(Debug, Clone, Copy, PartialEq)]
enum Fmt {
    /// text, fixed width or (None) ended by the unit terminator
    A(Option<usize>),
    I(Option<usize>),
    R(Option<usize>),
    /// little-endian binary integer: width in bytes, signed
    B(usize, bool),
    /// bit string of this many bits
    Bits(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Null,
}

impl Val {
    pub fn int(&self) -> Option<i64> {
        match self {
            Val::Int(v) => Some(*v),
            Val::Float(v) => Some(*v as i64),
            Val::Text(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Val::Text(s) => Some(s),
            _ => None,
        }
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Val::Bytes(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct FieldDef {
    labels: Arc<Vec<String>>,
    repeating: bool,
    formats: Vec<Fmt>,
}

/// One field of a data record: its subfield labels and rows of values (several rows
/// when the field repeats, such as coordinates or attribute pairs).
#[derive(Debug, Clone)]
pub struct Field {
    pub tag: String,
    pub labels: Arc<Vec<String>>,
    pub rows: Vec<Vec<Val>>,
}

impl Field {
    pub fn col(&self, label: &str) -> Option<usize> {
        self.labels.iter().position(|l| l == label)
    }
    /// The value of `label` in the first row.
    pub fn get(&self, label: &str) -> Option<&Val> {
        self.rows.first()?.get(self.col(label)?)
    }
    pub fn int(&self, label: &str) -> Option<i64> {
        self.get(label)?.int()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Record {
    pub fields: Vec<Field>,
}

impl Record {
    pub fn field(&self, tag: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.tag == tag)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not an ISO 8211 file: {0}")]
    Format(String),
}

fn num(b: &[u8]) -> Option<usize> {
    std::str::from_utf8(b).ok()?.trim().parse().ok()
}

struct Leader {
    len: usize,
    base: usize,
    control_len: usize,
    size_len: usize,
    size_pos: usize,
    size_tag: usize,
}

fn leader(b: &[u8]) -> Result<Leader, Error> {
    if b.len() < 24 {
        return Err(Error::Format("short leader".into()));
    }
    let bad = || Error::Format("bad leader".into());
    let d = |c: u8| (c as char).to_digit(10).map(|v| v as usize).ok_or_else(bad);
    Ok(Leader {
        len: num(&b[0..5]).ok_or_else(bad)?,
        control_len: num(&b[10..12]).unwrap_or(0),
        base: num(&b[12..17]).ok_or_else(bad)?,
        size_len: d(b[20])?,
        size_pos: d(b[21])?,
        size_tag: d(b[23])?,
    })
}

/// The directory of a record: (tag, field bytes without the field terminator).
fn fields<'a>(rec: &'a [u8], l: &Leader) -> Result<Vec<(String, &'a [u8])>, Error> {
    let mut out = Vec::new();
    let mut p = 24;
    let entry = l.size_tag + l.size_len + l.size_pos;
    while p < rec.len() && rec[p] != FT {
        let e = rec.get(p..p + entry).ok_or_else(|| Error::Format("directory".into()))?;
        let tag = String::from_utf8_lossy(&e[..l.size_tag]).to_string();
        let len = num(&e[l.size_tag..l.size_tag + l.size_len]).ok_or_else(|| Error::Format("directory".into()))?;
        let pos = num(&e[l.size_tag + l.size_len..]).ok_or_else(|| Error::Format("directory".into()))?;
        let start = l.base + pos;
        let mut data = rec.get(start..start + len).ok_or_else(|| Error::Format(format!("field {tag} past the record")))?;
        if data.last() == Some(&FT) {
            data = &data[..data.len() - 1];
        }
        out.push((tag, data));
        p += entry;
    }
    Ok(out)
}

/// "(b11,2b14,A(2),(b24,b24))" → the list of formats, repeats and groups expanded.
fn parse_formats(s: &str) -> Vec<Fmt> {
    let s = s.trim();
    let inner = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')).unwrap_or(s);
    let mut out = Vec::new();
    // split on top-level commas
    let mut depth = 0;
    let mut cur = String::new();
    let mut items = Vec::new();
    for c in inner.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c)
            }
            ')' => {
                depth -= 1;
                cur.push(c)
            }
            ',' if depth == 0 => items.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        items.push(cur);
    }
    for item in items {
        let item = item.trim();
        let digits: String = item.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = &item[digits.len()..];
        // "b11" etc: the digits belong to the format, not a repeat count
        let (n, rest) = if rest.is_empty() { (1, item) } else { (digits.parse().unwrap_or(1), rest) };
        let one: Vec<Fmt> = if rest.starts_with('(') {
            parse_formats(rest)
        } else {
            parse_one(rest).into_iter().collect()
        };
        for _ in 0..n {
            out.extend(one.iter().copied());
        }
    }
    out
}

fn parse_one(s: &str) -> Option<Fmt> {
    let width = || s.find('(').and_then(|i| s[i + 1..].trim_end_matches(')').parse().ok());
    let c = s.chars().next()?;
    Some(match c {
        'A' => Fmt::A(width()),
        'I' => Fmt::I(width()),
        'R' | 'S' => Fmt::R(width()),
        'B' => Fmt::Bits(width().unwrap_or(8)),
        'b' => {
            let d: Vec<u32> = s[1..].chars().filter_map(|c| c.to_digit(10)).collect();
            let (kind, w) = (d.first().copied().unwrap_or(1), d.get(1).copied().unwrap_or(1));
            Fmt::B(w as usize, kind == 2)
        }
        _ => return None,
    })
}

/// A whole ISO 8211 file: its data records.
pub struct Module {
    pub records: Vec<Record>,
}

impl Module {
    pub fn parse(b: &[u8]) -> Result<Module, Error> {
        let l = leader(b)?;
        if b.get(6) != Some(&b'L') {
            return Err(Error::Format("no DDR".into()));
        }
        let ddr = b.get(..l.len).ok_or_else(|| Error::Format("short DDR".into()))?;
        let mut defs: HashMap<String, FieldDef> = HashMap::new();
        for (tag, data) in fields(ddr, &l)? {
            if tag == "0000" {
                continue;
            }
            let body = data.get(l.control_len..).unwrap_or(&[]);
            let parts: Vec<&[u8]> = body.split(|c| *c == UT).collect();
            let desc = parts.get(1).map(|p| String::from_utf8_lossy(p).to_string()).unwrap_or_default();
            let fmts = parts.get(2).map(|p| String::from_utf8_lossy(p).to_string()).unwrap_or_default();
            let repeating = desc.starts_with('*');
            let mut labels: Vec<String> = desc.trim_start_matches('*').split('!').map(|s| s.to_string()).filter(|s| !s.is_empty()).collect();
            let formats = parse_formats(&fmts);
            if labels.is_empty() && !formats.is_empty() {
                labels.push(String::new());
            }
            defs.insert(tag, FieldDef { labels: Arc::new(labels), repeating, formats });
        }
        let mut records = Vec::new();
        let mut p = l.len;
        // national text in UCS-2 (S-57 lexical level 2), from DSSI NALL
        let mut wide = false;
        while p + 24 <= b.len() {
            let rl = leader(&b[p..])?;
            if rl.len == 0 {
                break;
            }
            let rec = b.get(p..p + rl.len).ok_or_else(|| Error::Format("short record".into()))?;
            let mut r = Record::default();
            for (tag, data) in fields(rec, &rl)? {
                let Some(def) = defs.get(&tag) else { continue };
                if tag == "0001" {
                    continue;
                }
                let rows = parse_field(def, data, wide && (tag == "NATF"));
                r.fields.push(Field { tag, labels: def.labels.clone(), rows });
            }
            if let Some(n) = r.field("DSSI").and_then(|f| f.int("NALL")) {
                wide = n == 2;
            }
            records.push(r);
            p += rl.len;
        }
        Ok(Module { records })
    }
}

fn parse_field(def: &FieldDef, data: &[u8], wide: bool) -> Vec<Vec<Val>> {
    let n = def.labels.len();
    if n == 0 || def.formats.is_empty() {
        return vec![];
    }
    let fmt = |i: usize| def.formats[i % def.formats.len()];
    let mut rows = Vec::new();
    let mut p = 0;
    loop {
        let mut row = Vec::with_capacity(n);
        for i in 0..n {
            let (v, used) = value(fmt(i), &data[p.min(data.len())..], wide);
            p += used;
            row.push(v);
        }
        rows.push(row);
        if !def.repeating || p >= data.len() {
            break;
        }
    }
    rows
}

/// One subfield value and the bytes it took (with its unit terminator).
fn value(f: Fmt, b: &[u8], wide: bool) -> (Val, usize) {
    let text = |b: &[u8]| -> (String, usize) {
        if wide {
            // UCS-2 little-endian, ended by 0x1f 0x00
            let mut s = Vec::new();
            let mut i = 0;
            while i + 1 < b.len() {
                let u = u16::from_le_bytes([b[i], b[i + 1]]);
                i += 2;
                if u == UT as u16 || u == FT as u16 {
                    break;
                }
                s.push(u);
            }
            (String::from_utf16_lossy(&s), i)
        } else {
            let end = b.iter().position(|c| *c == UT || *c == FT).unwrap_or(b.len());
            let used = if end < b.len() && b[end] == UT { end + 1 } else { end };
            (String::from_utf8_lossy(&b[..end]).into_owned(), used)
        }
    };
    let fixed = |w: usize| -> (String, usize) {
        let w = w.min(b.len());
        (String::from_utf8_lossy(&b[..w]).into_owned(), w)
    };
    match f {
        Fmt::A(w) => {
            let (s, u) = match w {
                Some(w) => fixed(w),
                None => text(b),
            };
            (Val::Text(s), u)
        }
        Fmt::I(w) | Fmt::R(w) => {
            let (s, u) = match w {
                Some(w) => fixed(w),
                None => text(b),
            };
            let t = s.trim();
            let v = if t.is_empty() {
                Val::Null
            } else if matches!(f, Fmt::I(_)) {
                t.parse().map(Val::Int).unwrap_or(Val::Text(s.clone()))
            } else {
                t.parse().map(Val::Float).unwrap_or(Val::Text(s.clone()))
            };
            (v, u)
        }
        Fmt::B(w, signed) => {
            if b.len() < w {
                return (Val::Null, b.len());
            }
            let mut v: u64 = 0;
            for (i, c) in b[..w].iter().enumerate() {
                v |= (*c as u64) << (8 * i);
            }
            let v = if signed && w < 8 && v & (1 << (8 * w - 1)) != 0 { v as i64 - (1i64 << (8 * w)) } else { v as i64 };
            (Val::Int(v), w)
        }
        Fmt::Bits(n) => {
            let w = (n / 8).min(b.len());
            (Val::Bytes(b[..w].to_vec()), w)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(parse_formats("(b11,b14,2b11,A(2),3A)"), vec![
            Fmt::B(1, false), Fmt::B(4, false), Fmt::B(1, false), Fmt::B(1, false), Fmt::A(Some(2)), Fmt::A(None), Fmt::A(None), Fmt::A(None)
        ]);
        assert_eq!(parse_formats("(2b24)"), vec![Fmt::B(4, true), Fmt::B(4, true)]);
        assert_eq!(parse_formats("(B(40),4b11)").len(), 5);
        assert_eq!(parse_formats("(A(1),I(10),(b24,b24))").len(), 4);
    }
}
