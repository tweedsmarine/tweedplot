//! Table rows as bytes: what the plugin sends for a query.
//!
//! `u32 count`, then per row: `i64 fid`, `u16 values`, each value a tag byte (0 null,
//! 1 integer i64, 2 real f64, 3 text: u32 length + UTF-8), then `u32 geometry length` and
//! the GeoPackage geometry blob. All little-endian.

/// A column value.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Real(f) => Some(*f),
            Value::Text(t) => t.trim().parse().ok(),
            Value::Null => None,
        }
    }

    /// As SQLite would print it (whole reals without ".0").
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Null => None,
            Value::Int(i) => Some(i.to_string()),
            Value::Real(f) => Some(if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", *f as i64) } else { format!("{f}") }),
            Value::Text(t) => Some(t.clone()),
        }
    }
}

/// One table row: its key, the columns asked for, its geometry (GeoPackage blob).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub fid: i64,
    pub vals: Vec<Value>,
    pub geom: Vec<u8>,
}

pub fn encode(rows: &[Row]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for r in rows {
        b.extend_from_slice(&r.fid.to_le_bytes());
        b.extend_from_slice(&(r.vals.len() as u16).to_le_bytes());
        for v in &r.vals {
            match v {
                Value::Null => b.push(0),
                Value::Int(i) => {
                    b.push(1);
                    b.extend_from_slice(&i.to_le_bytes());
                }
                Value::Real(f) => {
                    b.push(2);
                    b.extend_from_slice(&f.to_le_bytes());
                }
                Value::Text(t) => {
                    b.push(3);
                    b.extend_from_slice(&(t.len() as u32).to_le_bytes());
                    b.extend_from_slice(t.as_bytes());
                }
            }
        }
        b.extend_from_slice(&(r.geom.len() as u32).to_le_bytes());
        b.extend_from_slice(&r.geom);
    }
    b
}

pub fn decode(b: &[u8]) -> Result<Vec<Row>, crate::Error> {
    let bad = || crate::Error::Protocol("rows cut short".into());
    let mut p = 0usize;
    let mut take = |n: usize| -> Result<&[u8], crate::Error> {
        let s = b.get(p..p + n).ok_or_else(bad)?;
        p += n;
        Ok(s)
    };
    let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        let fid = i64::from_le_bytes(take(8)?.try_into().unwrap());
        let nv = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        let mut vals = Vec::with_capacity(nv);
        for _ in 0..nv {
            vals.push(match take(1)?[0] {
                0 => Value::Null,
                1 => Value::Int(i64::from_le_bytes(take(8)?.try_into().unwrap())),
                2 => Value::Real(f64::from_le_bytes(take(8)?.try_into().unwrap())),
                3 => {
                    let l = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
                    Value::Text(String::from_utf8_lossy(take(l)?).into_owned())
                }
                t => return Err(crate::Error::Protocol(format!("unknown value tag {t}"))),
            });
        }
        let gl = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let geom = take(gl)?.to_vec();
        out.push(Row { fid, vals, geom });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip() {
        let rows = vec![
            Row { fid: 1, vals: vec![Value::Null, Value::Int(-3), Value::Real(2.5), Value::Text("Ngā".into())], geom: vec![b'G', b'P', 0, 1] },
            Row { fid: 99, vals: vec![], geom: vec![] },
        ];
        assert_eq!(decode(&encode(&rows)).unwrap(), rows);
        assert!(decode(&encode(&rows)[..10]).is_err());
        assert_eq!(Value::Real(3.0).as_text().as_deref(), Some("3"));
        assert_eq!(Value::Text(" 4.5".into()).as_f64(), Some(4.5));
    }
}
