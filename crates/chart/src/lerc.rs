//! LERC (Limited Error Raster Compression, Esri, Apache 2.0) decoding: the Lerc2 format,
//! versions 2 to 5, as GeoTIFF tiles use it (GDAL writes version 4). LINZ's coastal and
//! elevation models are float32 GeoTIFFs compressed this way.
//!
//! A blob is a header, a validity mask (run-length encoded bits), then the values: all
//! at once ("one sweep"), or in micro-blocks of 8×8 pixels, each raw, constant, or an
//! offset plus bit-stuffed integers scaled by twice the allowed error. Written from
//! Esri's format description and its JavaScript decoder (LercDecode.js); Huffman coding,
//! used only for 8-bit data, is not needed here and not supported.

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Truncated,
    Format(&'static str),
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Truncated => f.write_str("LERC: data ends early"),
            Error::Format(s) => write!(f, "LERC: {s}"),
            Error::Unsupported(s) => write!(f, "LERC: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// A decoded band (or bands, one after another: `depth` values per pixel are stored band
/// by band). Invalid pixels are `false` in `valid` and 0 in `data`.
#[derive(Debug, Clone)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub depth: usize,
    pub data: Vec<f32>,
    /// None when every pixel is valid.
    pub valid: Option<Vec<bool>>,
}

impl Raster {
    pub fn get(&self, x: usize, y: usize) -> Option<f32> {
        let i = y * self.width + x;
        if self.valid.as_ref().is_some_and(|v| !v[i]) {
            return None;
        }
        Some(self.data[i])
    }
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let s = self.b.get(self.p..self.p + n).ok_or(Error::Truncated)?;
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, Error> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// One value of LERC data type `dt` (0 i8, 1 u8, 2 i16, 3 u16, 4 i32, 5 u32, 6 f32, 7 f64).
    fn value(&mut self, dt: i32) -> Result<f64, Error> {
        let b = self.take(type_size(dt)?)?;
        Ok(match dt {
            0 => b[0] as i8 as f64,
            1 => b[0] as f64,
            2 => i16::from_le_bytes([b[0], b[1]]) as f64,
            3 => u16::from_le_bytes([b[0], b[1]]) as f64,
            4 => i32::from_le_bytes(b.try_into().unwrap()) as f64,
            5 => u32::from_le_bytes(b.try_into().unwrap()) as f64,
            6 => f32::from_le_bytes(b.try_into().unwrap()) as f64,
            _ => f64::from_le_bytes(b.try_into().unwrap()),
        })
    }
}

fn type_size(dt: i32) -> Result<usize, Error> {
    Ok(match dt {
        0 | 1 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        7 => 8,
        _ => return Err(Error::Format("unknown data type")),
    })
}

/// The type a block offset is stored in: smaller types where the value fits (`tc` from
/// bits 6 and 7 of the block header).
fn type_used(dt: i32, tc: i32) -> i32 {
    match dt {
        2 | 4 => dt - tc,
        3 | 5 => dt - 2 * tc,
        6 => match tc {
            0 => 6,
            1 => 2,
            _ => 1,
        },
        7 => {
            if tc == 0 {
                7
            } else {
                7 - 2 * tc + 1
            }
        }
        _ => dt,
    }
}

/// Decode one Lerc2 blob. Values come back as f32 whatever the stored type.
pub fn decode(blob: &[u8]) -> Result<Raster, Error> {
    let mut r = Rd { b: blob, p: 0 };
    if r.take(6)? != b"Lerc2 " {
        return Err(Error::Format("not a Lerc2 blob"));
    }
    let version = r.i32()?;
    if !(2..=5).contains(&version) {
        return Err(Error::Unsupported(format!("version {version}")));
    }
    if version >= 3 {
        let _checksum = r.u32()?;
    }
    let height = r.i32()?.max(0) as usize;
    let width = r.i32()?.max(0) as usize;
    let depth = if version >= 4 {
        r.i32()?.max(1) as usize
    } else {
        1
    };
    let num_valid = r.i32()?.max(0) as usize;
    let mb = r.i32()?.max(1) as usize;
    let _blob_size = r.i32()?;
    let dt = r.i32()?;
    let max_z_err = r.f64()?;
    let z_min = r.f64()?;
    let z_max = r.f64()?;
    type_size(dt)?;
    let n = width.checked_mul(height).ok_or(Error::Format("too big"))?;
    if n > 100_000_000 || depth > 64 {
        return Err(Error::Format("too big"));
    }

    // validity mask: run-length encoded bits (count > 0: that many literal bytes; < 0:
    // one byte repeated; -32768 ends it)
    let mask_bytes = r.i32()?.max(0) as usize;
    let mut valid: Option<Vec<bool>> = None;
    if num_valid == 0 {
        valid = Some(vec![false; n]);
        r.take(mask_bytes)?;
    } else if mask_bytes > 0 {
        let m = r.take(mask_bytes)?;
        let mut mr = Rd { b: m, p: 0 };
        let mut bits = vec![0u8; n.div_ceil(8)];
        let mut op = 0;
        loop {
            let cnt = mr.i16()?;
            if cnt == -32768 {
                break;
            }
            if cnt > 0 {
                for _ in 0..cnt {
                    let v = mr.u8()?;
                    *bits.get_mut(op).ok_or(Error::Format("mask overruns"))? = v;
                    op += 1;
                }
            } else {
                let v = mr.u8()?;
                for _ in 0..(-(cnt as i32)) {
                    *bits.get_mut(op).ok_or(Error::Format("mask overruns"))? = v;
                    op += 1;
                }
            }
        }
        valid = Some(
            (0..n)
                .map(|k| bits[k >> 3] & (0x80 >> (k & 7)) != 0)
                .collect(),
        );
    }

    let mut data = vec![0f32; n * depth];
    let mut out = Raster {
        width,
        height,
        depth,
        data: Vec::new(),
        valid,
    };
    if num_valid == 0 {
        out.data = data;
        return Ok(out);
    }
    let fill_const = |data: &mut [f32], valid: &Option<Vec<bool>>, d: usize, v: f64| {
        for k in 0..n {
            if valid.as_ref().is_none_or(|m| m[k]) {
                data[d * n + k] = v as f32;
            }
        }
    };
    if z_min == z_max {
        for d in 0..depth {
            fill_const(&mut data, &out.valid, d, z_min);
        }
        out.data = data;
        return Ok(out);
    }
    let mut z_maxes = vec![z_max; depth];
    if version >= 4 {
        let mins: Vec<f64> = (0..depth).map(|_| r.value(dt)).collect::<Result<_, _>>()?;
        let maxs: Vec<f64> = (0..depth).map(|_| r.value(dt)).collect::<Result<_, _>>()?;
        if mins.iter().zip(&maxs).all(|(a, b)| a == b) {
            for (d, v) in mins.iter().enumerate() {
                fill_const(&mut data, &out.valid, d, *v);
            }
            out.data = data;
            return Ok(out);
        }
        z_maxes = maxs;
    }

    let one_sweep = r.u8()?;
    if one_sweep != 0 {
        for k in 0..n {
            if out.valid.as_ref().is_none_or(|m| m[k]) {
                for d in 0..depth {
                    data[d * n + k] = r.value(dt)? as f32;
                }
            }
        }
        out.data = data;
        return Ok(out);
    }
    if version > 1 && dt <= 1 && (max_z_err - 0.5).abs() < 1e-5 {
        let huffman = r.u8()?;
        if huffman != 0 {
            return Err(Error::Unsupported("Huffman-coded 8-bit data".into()));
        }
    }

    // micro-blocks
    let check = if version >= 5 { 14 } else { 15 };
    let scale = 2.0 * max_z_err;
    let blocks_x = width.div_ceil(mb);
    let blocks_y = height.div_ceil(mb);
    let mut buf: Vec<u32> = Vec::with_capacity(mb * mb);
    for by in 0..blocks_y {
        let bh = if by + 1 == blocks_y {
            height - by * mb
        } else {
            mb
        };
        for bx in 0..blocks_x {
            let bw = if bx + 1 == blocks_x {
                width - bx * mb
            } else {
                mb
            };
            for d in 0..depth {
                let z_max = z_maxes[d];
                let head = r.u8()?;
                let diff = version >= 5 && head & 4 != 0;
                if diff {
                    return Err(Error::Unsupported("difference-coded bands".into()));
                }
                if ((head >> 2) as usize & check) != (((bx * mb) >> 3) & check) {
                    return Err(Error::Format("block integrity check failed"));
                }
                let bits67 = (head >> 6) as i32;
                let enc = head & 3;
                let pixels = (0..bh).flat_map(|row| {
                    (0..bw).map(move |col| (by * mb + row) * width + bx * mb + col)
                });
                match enc {
                    2 => {} // all zero
                    0 => {
                        for k in pixels {
                            if out.valid.as_ref().is_none_or(|m| m[k]) {
                                data[d * n + k] = r.value(dt)? as f32;
                            }
                        }
                    }
                    3 => {
                        let off = r.value(type_used(dt, bits67))?;
                        for k in pixels {
                            if out.valid.as_ref().is_none_or(|m| m[k]) {
                                data[d * n + k] = off as f32;
                            }
                        }
                    }
                    _ => {
                        let off = r.value(type_used(dt, bits67))?;
                        let count = unstuff(&mut r, version, &mut buf, off, scale, z_max)?;
                        let mut i = 0;
                        let mut vals: Vec<f64> = Vec::new();
                        if let Some(lut) = count {
                            vals = lut;
                        }
                        for k in pixels {
                            if out.valid.as_ref().is_none_or(|m| m[k]) {
                                let q = *buf
                                    .get(i)
                                    .ok_or(Error::Format("block has too few values"))?;
                                let v = if vals.is_empty() {
                                    let x = off + q as f64 * scale;
                                    if x > z_max {
                                        z_max
                                    } else {
                                        x
                                    }
                                } else {
                                    *vals
                                        .get(q as usize)
                                        .ok_or(Error::Format("bad table index"))?
                                };
                                data[d * n + k] = v as f32;
                                i += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    out.data = data;
    Ok(out)
}

/// Read a bit-stuffed array into `buf` (the integers). With a look-up table, returns the
/// table's values (offset first), which the integers index; else None.
fn unstuff(
    r: &mut Rd,
    version: i32,
    buf: &mut Vec<u32>,
    off: f64,
    scale: f64,
    z_max: f64,
) -> Result<Option<Vec<f64>>, Error> {
    let head = r.u8()?;
    let bits67 = head >> 6;
    let num = match bits67 {
        0 => r.u32()? as usize,
        1 => r.u16()? as usize,
        2 => r.u8()? as usize,
        _ => return Err(Error::Format("bad element count size")),
    };
    if num > 1 << 20 {
        return Err(Error::Format("too many values in a block"));
    }
    let lut = head & 32 != 0;
    let nbits = (head & 31) as u32;
    buf.clear();
    if !lut {
        if nbits == 0 {
            buf.resize(num, 0);
        } else {
            read_bits(r, version, buf, num, nbits)?;
        }
        return Ok(None);
    }
    let lut_len = r.u8()? as usize; // table values + 1
    if lut_len < 2 {
        return Err(Error::Format("bad look-up table"));
    }
    let mut table = Vec::new();
    read_bits(r, version, &mut table, lut_len - 1, nbits)?;
    let nmax = ((z_max - off) / scale).ceil();
    let mut vals = vec![off];
    vals.extend(table.iter().map(|&q| {
        if (q as f64) < nmax {
            off + q as f64 * scale
        } else {
            z_max
        }
    }));
    let mut idx_bits = 0;
    while ((lut_len - 1) >> idx_bits) != 0 {
        idx_bits += 1;
    }
    read_bits(r, version, buf, num, idx_bits)?;
    Ok(Some(vals))
}

/// `num` integers of `nbits` bits, packed in little-endian 32-bit words: from the low bits
/// up from version 3, from the high bits down before it.
fn read_bits(
    r: &mut Rd,
    version: i32,
    out: &mut Vec<u32>,
    num: usize,
    nbits: u32,
) -> Result<(), Error> {
    if nbits == 0 || nbits > 32 {
        out.resize(out.len() + num, 0);
        return if nbits == 0 {
            Ok(())
        } else {
            Err(Error::Format("bad bit count"))
        };
    }
    let nbytes = (num * nbits as usize).div_ceil(8);
    let src = r.take(nbytes)?;
    let words: Vec<u32> = src
        .chunks(4)
        .map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(w)
        })
        .collect();
    let mask: u64 = (1u64 << nbits) - 1;
    if version >= 3 {
        let mut bitpos = 0usize;
        for _ in 0..num {
            let w = bitpos / 32;
            let b = bitpos % 32;
            let lo = words[w] as u64 >> b;
            let v = if b + nbits as usize > 32 {
                lo | ((*words.get(w + 1).unwrap_or(&0) as u64) << (32 - b))
            } else {
                lo
            };
            out.push((v & mask) as u32);
            bitpos += nbits as usize;
        }
    } else {
        // the last word was written short: its bytes sit at the top
        let mut words = words;
        let tail = words.len() * 4 - nbytes;
        if let Some(l) = words.last_mut() {
            *l <<= 8 * tail as u32;
        }
        let mut bitpos = 0usize;
        for _ in 0..num {
            let w = bitpos / 32;
            let b = bitpos % 32;
            let hi = (words[w] as u64) << 32 | *words.get(w + 1).unwrap_or(&0) as u64;
            let v = hi >> (64 - b - nbits as usize);
            out.push((v & mask) as u32);
            bitpos += nbits as usize;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixtures made with Esri's LERC library (imagecodecs.lerc_encode), see testdata/README.
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata")
                .join(name),
        )
        .unwrap()
    }

    /// The test surface the fixtures were made from (64 × 48, with a hole and an edge of
    /// "no data" for the masked ones).
    fn surface(x: usize, y: usize) -> f32 {
        let (xf, yf) = (x as f32, y as f32);
        -12.0 + 0.3 * xf - 0.15 * yf + 2.0 * (xf * 0.21).sin() * (yf * 0.17).cos()
    }

    fn masked(x: usize, y: usize) -> bool {
        let (dx, dy) = (x as i32 - 40, y as i32 - 20);
        dx * dx + dy * dy < 36 || x < 3
    }

    fn check(name: &str, with_mask: bool, tol: f32) {
        let r = decode(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((r.width, r.height, r.depth), (64, 48, 1), "{name}");
        for y in 0..48 {
            for x in 0..64 {
                match r.get(x, y) {
                    None => assert!(with_mask && masked(x, y), "{name}: {x},{y} invalid"),
                    Some(v) => {
                        assert!(
                            !(with_mask && masked(x, y)),
                            "{name}: {x},{y} should be invalid"
                        );
                        assert!(
                            (v - surface(x, y)).abs() <= tol,
                            "{name}: {x},{y} {v} v {}",
                            surface(x, y)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn decodes_each_version() {
        for v in [2, 3, 4] {
            check(&format!("lerc_v{v}.bin"), false, 0.0101);
            check(&format!("lerc_v{v}_mask.bin"), true, 0.0101);
        }
        // lossless float (one sweep or raw blocks)
        check("lerc_v4_lossless.bin", false, 1e-4);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"Lerc2 ").is_err());
        assert!(decode(b"PK\x03\x04").is_err());
        let mut b = fixture("lerc_v4.bin");
        b.truncate(b.len() / 2);
        assert!(decode(&b).is_err());
    }
}
