//! A decoder for Blosc (version 1) compressed buffers, as used for the chunks of
//! Zarr stores such as Copernicus Marine's ARCO data (Blosc with LZ4 and byte
//! shuffle). Pure Rust: LZ4 by `lz4_flex`, zlib by `flate2`.
//!
//! The format: a 16-byte header (version, version of the codec, flags,
//! type size, uncompressed size, block size, compressed size), then either the
//! data as it is (flag 0x02, "memcpyed") or a table of where each block starts
//! and the blocks. A block is split into one stream per byte of the type
//! (unless flag 0x10, or for the last, short block), each stream with its
//! compressed size before it (a stream as long as its uncompressed size is
//! stored as it is). After decompression, a byte-shuffled block (flag 0x01) is
//! put back in order.

use crate::{Error, Result};

fn bad(why: &str) -> Error {
    Error::Bad(format!("Blosc: {why}"))
}

fn u32_at(b: &[u8], at: usize) -> Result<usize> {
    let s = b.get(at..at + 4).ok_or_else(|| bad("the data is cut short"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Decompress one Blosc buffer.
pub fn decompress(src: &[u8]) -> Result<Vec<u8>> {
    if src.len() < 16 {
        return Err(bad("the data is cut short"));
    }
    let flags = src[2];
    let typesize = (src[3] as usize).max(1);
    let nbytes = u32_at(src, 4)?;
    let blocksize = u32_at(src, 8)?;
    let cbytes = u32_at(src, 12)?;
    if cbytes > src.len() {
        return Err(bad("the data is cut short"));
    }
    if nbytes == 0 {
        return Ok(Vec::new());
    }
    if flags & 0x02 != 0 {
        // stored as it is
        return Ok(src.get(16..16 + nbytes).ok_or_else(|| bad("the data is cut short"))?.to_vec());
    }
    if flags & 0x04 != 0 {
        return Err(bad("bit shuffle is not supported"));
    }
    if blocksize == 0 {
        return Err(bad("block size 0"));
    }
    let codec = (flags >> 5) & 0x07;
    let shuffled = flags & 0x01 != 0;
    let dont_split = flags & 0x10 != 0;
    let nblocks = nbytes.div_ceil(blocksize);
    let mut out = vec![0u8; nbytes];
    let mut tmp = vec![0u8; blocksize];
    for b in 0..nblocks {
        let start = u32_at(src, 16 + 4 * b)?;
        let bsize = (nbytes - b * blocksize).min(blocksize);
        let leftover = bsize < blocksize;
        let nstreams = if !dont_split && !leftover && bsize % typesize == 0 { typesize } else { 1 };
        let neblock = bsize / nstreams;
        let mut at = start;
        let dest = &mut tmp[..bsize];
        for s in 0..nstreams {
            let cb = u32_at(src, at)?;
            at += 4;
            let input = src.get(at..at + cb).ok_or_else(|| bad("a stream runs past the end"))?;
            let piece = &mut dest[s * neblock..(s + 1) * neblock];
            if cb == neblock {
                piece.copy_from_slice(input);
            } else {
                let n = match codec {
                    1 => lz4_flex::block::decompress_into(input, piece).map_err(|e| bad(&format!("LZ4: {e}")))?,
                    3 => {
                        use std::io::Read;
                        let mut d = flate2::read::ZlibDecoder::new(input);
                        let mut n = 0;
                        while n < piece.len() {
                            let k = d.read(&mut piece[n..]).map_err(|e| bad(&format!("zlib: {e}")))?;
                            if k == 0 {
                                break;
                            }
                            n += k;
                        }
                        n
                    }
                    0 => return Err(bad("the BloscLZ codec is not supported")),
                    2 => return Err(bad("the Snappy codec is not supported")),
                    4 => return Err(bad("the Zstd codec is not supported")),
                    _ => return Err(bad("unknown codec")),
                };
                if n != neblock {
                    return Err(bad("a stream decompressed to the wrong size"));
                }
            }
            at += cb;
        }
        let out_block = &mut out[b * blocksize..b * blocksize + bsize];
        if shuffled && typesize > 1 && bsize >= typesize {
            unshuffle(dest, out_block, typesize);
        } else {
            out_block.copy_from_slice(dest);
        }
    }
    Ok(out)
}

/// Undo Blosc's byte shuffle: the block holds all the first bytes of the
/// elements, then all the second bytes, …; bytes after the last whole element are as they were.
fn unshuffle(src: &[u8], dst: &mut [u8], typesize: usize) {
    let n = src.len() / typesize;
    for i in 0..n {
        for j in 0..typesize {
            dst[i * typesize + j] = src[j * n + i];
        }
    }
    let whole = n * typesize;
    dst[whole..].copy_from_slice(&src[whole..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32s(b: &[u8]) -> Vec<f32> {
        b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// Copernicus Marine's depth levels (stored as they are): 50 levels, the deepest first.
    #[test]
    fn stored_as_is() {
        let v = f32s(&decompress(include_bytes!("../testdata/cmems_elevation.blosc")).unwrap());
        assert_eq!(v.len(), 50);
        assert!((v[0] + 5727.917).abs() < 0.01);
        assert!((v[49] + 0.494025).abs() < 1e-5);
    }

    /// LZ4, shuffled, split into four streams: a time axis 6-hourly since 2022-06-01.
    #[test]
    fn lz4_shuffled_split() {
        let v = f32s(&decompress(include_bytes!("../testdata/cmems_time.blosc")).unwrap());
        assert_eq!(v.len(), 2633);
        // the third chunk starts at index 5266: 2022-06-01 is 634 776 h after 1950-01-01
        assert_eq!(v[0], 634_776.0 + 5266.0 * 6.0);
        // the array ends at index 6580 (1315 values in this chunk), 6-hourly throughout
        for k in 1..1315 {
            assert_eq!(v[k] - v[k - 1], 6.0, "at {k}");
        }
    }

    /// A latitude axis, -80 to 90 by 1/12°.
    #[test]
    fn latitudes() {
        let v = f32s(&decompress(include_bytes!("../testdata/cmems_lat.blosc")).unwrap());
        assert_eq!(v.len(), 2041);
        assert_eq!(v[0], -80.0);
        assert!((v[2040] - 90.0).abs() < 1e-3);
        assert!((v[1000] - (-80.0 + 1000.0 / 12.0)).abs() < 1e-3);
    }
}
