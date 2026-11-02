//! Frames: a JSON header and a byte body, each with its length in front.

use std::io::{Read, Write};

/// Largest header accepted (16 MiB) and body (1 GiB): anything bigger is a broken stream.
const MAX_HEADER: u32 = 16 << 20;
const MAX_BODY: u32 = 1 << 30;

/// Write one frame and flush.
pub fn write_frame(w: &mut impl Write, header: &serde_json::Value, body: &[u8]) -> std::io::Result<()> {
    let h = serde_json::to_vec(header).map_err(std::io::Error::other)?;
    let mut buf = Vec::with_capacity(8 + h.len() + body.len());
    buf.extend_from_slice(&(h.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&h);
    buf.extend_from_slice(body);
    w.write_all(&buf)?;
    w.flush()
}

/// Read one frame. `Ok(None)` at a clean end of the stream.
pub fn read_frame(r: &mut impl Read) -> std::io::Result<Option<(serde_json::Value, Vec<u8>)>> {
    let mut len = [0u8; 8];
    match r.read_exact(&mut len[..1]) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    r.read_exact(&mut len[1..])?;
    let hl = u32::from_le_bytes(len[..4].try_into().unwrap());
    let bl = u32::from_le_bytes(len[4..].try_into().unwrap());
    if hl > MAX_HEADER || bl > MAX_BODY {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("frame too big ({hl} + {bl} bytes)")));
    }
    let mut h = vec![0u8; hl as usize];
    r.read_exact(&mut h)?;
    let mut body = vec![0u8; bl as usize];
    r.read_exact(&mut body)?;
    let header = serde_json::from_slice(&h).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some((header, body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &serde_json::json!({"id": 1, "method": "hello"}), b"").unwrap();
        write_frame(&mut buf, &serde_json::json!({"id": 2}), &[1, 2, 3]).unwrap();
        let mut r = std::io::Cursor::new(buf);
        let (h, b) = read_frame(&mut r).unwrap().unwrap();
        assert_eq!((h["method"].as_str(), b.len()), (Some("hello"), 0));
        let (h, b) = read_frame(&mut r).unwrap().unwrap();
        assert_eq!((h["id"].as_u64(), b), (Some(2), vec![1, 2, 3]));
        assert!(read_frame(&mut r).unwrap().is_none());
    }
}
