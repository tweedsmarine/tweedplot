//! CCSDS 121.0-B adaptive entropy coding (AEC, "szip"), decoding only, as
//! used by GRIB2 data representation template 5.42 (ECMWF open data since
//! 2023). Follows the stream layout libaec writes: blocks of `block_size`
//! samples, a reference sample at the start of each reference sample interval
//! (RSI) when preprocessing is on, options zero-block, second extension,
//! k-split and uncompressed, and the unit-delay predictor with the standard
//! mapping of differences.
//!
//! Only unsigned samples (all GRIB uses) are supported.

pub const DATA_SIGNED: u8 = 1;
pub const DATA_PREPROCESS: u8 = 8;
pub const RESTRICTED: u8 = 16;
pub const PAD_RSI: u8 = 32;

/// Run-of-segment code for zero blocks.
const ROS: u64 = 5;

struct Bits<'a> {
    d: &'a [u8],
    pos: usize, // bit position
}

impl<'a> Bits<'a> {
    #[inline]
    fn bit(&mut self) -> Option<u32> {
        let b = *self.d.get(self.pos >> 3)?;
        let v = (b >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        Some(v as u32)
    }
    #[inline]
    fn take(&mut self, n: usize) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if self.pos + n > self.d.len() * 8 {
            return None;
        }
        let mut v: u64 = 0;
        let mut need = n;
        while need > 0 {
            let byte = self.d[self.pos >> 3] as u64;
            let off = self.pos & 7;
            let avail = 8 - off;
            let t = avail.min(need);
            let bits = (byte >> (avail - t)) & ((1u64 << t) - 1);
            v = (v << t) | bits;
            self.pos += t;
            need -= t;
        }
        Some(v as u32)
    }
    /// Fundamental sequence: the number of 0 bits before the next 1.
    #[inline]
    fn fs(&mut self) -> Option<u32> {
        let mut n = 0u32;
        loop {
            let byte = *self.d.get(self.pos >> 3)?;
            let off = self.pos & 7;
            let rest = (byte << off) as u32 & 0xff;
            if rest == 0 {
                n += (8 - off) as u32;
                self.pos += 8 - off;
                continue;
            }
            let z = rest.leading_zeros() - 24; // zeros before the 1 within the remaining bits
            n += z;
            self.pos += z as usize + 1;
            return Some(n);
        }
    }
    fn align(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AecError {
    Unsupported(&'static str),
    Truncated,
}

/// Decode `n` samples of `bps` bits each.
pub fn decode(data: &[u8], n: usize, bps: usize, block_size: usize, rsi: usize, flags: u8) -> Result<Vec<u32>, AecError> {
    if flags & DATA_SIGNED != 0 {
        return Err(AecError::Unsupported("signed samples"));
    }
    if bps == 0 || bps > 32 || block_size == 0 || rsi == 0 {
        return Err(AecError::Unsupported("parameters"));
    }
    let id_len = if flags & RESTRICTED != 0 && bps <= 4 {
        if bps <= 2 {
            1
        } else {
            2
        }
    } else if bps > 16 {
        5
    } else if bps > 8 {
        4
    } else {
        3
    };
    let id_uncomp = (1u32 << id_len) - 1;
    let pp = flags & DATA_PREPROCESS != 0;
    let xmax: u64 = if bps == 32 { u32::MAX as u64 } else { (1u64 << bps) - 1 };
    let mut br = Bits { d: data, pos: 0 };
    let mut out: Vec<u32> = Vec::with_capacity(n);
    let tr = |_: ()| AecError::Truncated;

    while out.len() < n {
        let rsi_start = out.len();
        // mapped (preprocessed) samples of this RSI, or raw samples without preprocessing
        let mut block = 0usize;
        while block < rsi {
            if out.len() >= n {
                break;
            }
            let has_ref = pp && block == 0;
            let id = br.take(id_len).ok_or(()).map_err(tr)?;
            if id == 0 {
                // low entropy: zero block(s) or second extension
                let sel = br.bit().ok_or(()).map_err(tr)?;
                let reference = if has_ref { Some(br.take(bps).ok_or(()).map_err(tr)?) } else { None };
                if sel == 0 {
                    let mut zb = br.fs().ok_or(()).map_err(tr)? as u64 + 1;
                    if zb == ROS {
                        let b = block as u64;
                        zb = (rsi as u64 - b).min(64 - (b % 64));
                    } else if zb > ROS {
                        zb -= 1;
                    }
                    let mut cnt = zb as usize * block_size;
                    if let Some(r) = reference {
                        out.push(r);
                        cnt -= 1;
                    }
                    out.resize(out.len() + cnt, 0);
                    block += zb as usize;
                    continue;
                } else {
                    let mut i = 0usize;
                    if let Some(r) = reference {
                        out.push(r);
                        i = 1;
                    }
                    while i < block_size {
                        let m = br.fs().ok_or(()).map_err(tr)? as u64;
                        // m = s(s+1)/2 + y with s = x + y
                        let s = (((8 * m + 1) as f64).sqrt() as u64 - 1) / 2;
                        let mut s = s;
                        while (s + 1) * (s + 2) / 2 <= m {
                            s += 1;
                        }
                        while s * (s + 1) / 2 > m {
                            s -= 1;
                        }
                        let y = m - s * (s + 1) / 2;
                        let x = s - y;
                        if i & 1 == 0 {
                            out.push(x as u32);
                            i += 1;
                        }
                        out.push(y as u32);
                        i += 1;
                    }
                }
            } else if id == id_uncomp {
                for _ in 0..block_size {
                    out.push(br.take(bps).ok_or(()).map_err(tr)?);
                }
            } else {
                let k = (id - 1) as usize;
                let start = out.len();
                if has_ref {
                    out.push(br.take(bps).ok_or(()).map_err(tr)?);
                }
                let m = block_size - has_ref as usize;
                for _ in 0..m {
                    let hi = br.fs().ok_or(()).map_err(tr)?;
                    out.push(hi << k);
                }
                if k > 0 {
                    for v in out[start + has_ref as usize..].iter_mut() {
                        *v |= br.take(k).ok_or(()).map_err(tr)?;
                    }
                }
            }
            block += 1;
        }
        if flags & PAD_RSI != 0 {
            br.align();
        }
        // undo the preprocessing of this RSI: unit-delay prediction and mapping
        if pp {
            let end = out.len().min(n);
            if end > rsi_start {
                let mut x = out[rsi_start] as u64;
                let med = xmax / 2 + 1;
                for v in out[rsi_start + 1..end].iter_mut() {
                    let d = *v as u64;
                    let half = (d >> 1) + (d & 1);
                    let mask = if x & med != 0 { xmax } else { 0 };
                    if half <= (mask ^ x) {
                        x = if d & 1 == 1 { x.wrapping_sub((d + 1) >> 1) } else { x + (d >> 1) };
                    } else {
                        x = mask ^ d;
                    }
                    *v = x as u32;
                }
            }
        }
    }
    out.truncate(n);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny bit writer to build streams by hand.
    struct W {
        v: Vec<u8>,
        n: usize,
    }
    impl W {
        fn new() -> Self {
            W { v: Vec::new(), n: 0 }
        }
        fn put(&mut self, x: u64, bits: usize) {
            for i in (0..bits).rev() {
                if self.n.is_multiple_of(8) {
                    self.v.push(0);
                }
                if (x >> i) & 1 == 1 {
                    *self.v.last_mut().unwrap() |= 1 << (7 - self.n % 8);
                }
                self.n += 1;
            }
        }
        fn fs(&mut self, m: u64) {
            self.put(0, m as usize);
            self.put(1, 1);
        }
    }

    #[test]
    fn uncompressed_and_split_without_preprocessing() {
        // bps 8 → id 3 bits; block 8; one uncompressed block then a k=2 split block
        let mut w = W::new();
        let a: Vec<u64> = vec![1, 2, 3, 250, 0, 7, 8, 9];
        w.put(7, 3);
        for &x in &a {
            w.put(x, 8);
        }
        let b: Vec<u64> = vec![5, 6, 7, 4, 3, 2, 1, 0];
        w.put(3, 3); // k = 2
        for &x in &b {
            w.fs(x >> 2);
        }
        for &x in &b {
            w.put(x & 3, 2);
        }
        let out = decode(&w.v, 16, 8, 8, 2, 0).unwrap();
        let want: Vec<u32> = a.iter().chain(&b).map(|&x| x as u32).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn preprocessing_zero_block_and_second_extension() {
        // constant 100 for one block (reference + zero deltas), then deltas via SE
        let mut w = W::new();
        w.put(0, 3); // low entropy
        w.put(0, 1); // zero block
        w.put(100, 8); // reference
        w.fs(0); // one zero block
                 // second block: SE, mapped deltas 2,1 → +1, -1 ...
        let deltas = [2u64, 1, 2, 1, 0, 0, 4, 3];
        w.put(0, 3);
        w.put(1, 1);
        for p in deltas.chunks(2) {
            let (x, y) = (p[0], p[1]);
            let s = x + y;
            w.fs(s * (s + 1) / 2 + y);
        }
        let out = decode(&w.v, 16, 8, 8, 4, DATA_PREPROCESS).unwrap();
        assert_eq!(&out[..8], &[100; 8]);
        assert_eq!(&out[8..], &[101, 100, 101, 100, 100, 100, 102, 100]);
    }
}
