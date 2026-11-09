//! Pictures that go with ENC cells (PICREP): TIFF files kept beside the cell in its
//! exchange set (S-57 3.1, Appendix B.1 5.4 says TIFF 6.0). Decoded here without outside
//! TIFF code and turned into PNG for display.
//!
//! The first image of the file is read. Supported: both byte orders; strips and tiles;
//! no compression, PackBits, LZW (with horizontal prediction), Deflate, CCITT modified
//! Huffman (compression 2), fax Group 3 (1-D and 2-D) and Group 4; bilevel, grey,
//! palette, RGB (with or without alpha) and CMYK; 1, 2, 4, 8 and 16 bits per sample.
//! Not supported (an error names it): JPEG, YCbCr, separate planes, floating point.

use std::collections::HashMap;
use std::io::Read;

/// A decoded picture: 8-bit samples, `channels` per pixel (1 grey, 3 RGB, 4 RGBA), rows top
/// to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub data: Vec<u8>,
}

/// Why a picture couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureError(pub String);

impl std::fmt::Display for PictureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PictureError {}

fn err<T>(m: impl Into<String>) -> Result<T, PictureError> {
    Err(PictureError(m.into()))
}

/// The largest picture read (pixels): far above any chart diagram.
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

// ---------------------------------------------------------------- the file structure

struct Reader<'a> {
    b: &'a [u8],
    be: bool,
}

impl Reader<'_> {
    fn u16(&self, at: usize) -> Result<u16, PictureError> {
        let Some(s) = self.b.get(at..at + 2) else { return err("the file is cut short") };
        Ok(if self.be { u16::from_be_bytes([s[0], s[1]]) } else { u16::from_le_bytes([s[0], s[1]]) })
    }
    fn u32(&self, at: usize) -> Result<u32, PictureError> {
        let Some(s) = self.b.get(at..at + 4) else { return err("the file is cut short") };
        let a = [s[0], s[1], s[2], s[3]];
        Ok(if self.be { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) })
    }
}

/// The tags of the first image: tag → its values (integers; rationals and others aren't
/// needed).
fn read_tags(r: &Reader) -> Result<HashMap<u16, Vec<u32>>, PictureError> {
    let ifd = r.u32(4)? as usize;
    let n = r.u16(ifd)? as usize;
    let mut tags = HashMap::new();
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        let tag = r.u16(e)?;
        let typ = r.u16(e + 2)?;
        let count = r.u32(e + 4)? as usize;
        let size = match typ {
            1 | 2 | 6 | 7 => 1, // BYTE, ASCII, SBYTE, UNDEFINED
            3 | 8 => 2,         // SHORT, SSHORT
            4 | 9 | 13 => 4,    // LONG, SLONG, IFD
            _ => continue,      // rationals, floats: not needed
        };
        let Some(total) = count.checked_mul(size) else { return err("a tag is too large") };
        if total > r.b.len() {
            return err("a tag is larger than the file");
        }
        let at = if total <= 4 { e + 8 } else { r.u32(e + 8)? as usize };
        let mut v = Vec::with_capacity(count);
        for k in 0..count {
            v.push(match size {
                1 => match r.b.get(at + k) {
                    Some(&x) => x as u32,
                    None => return err("the file is cut short"),
                },
                2 => r.u16(at + k * 2)? as u32,
                _ => r.u32(at + k * 4)?,
            });
        }
        tags.insert(tag, v);
    }
    Ok(tags)
}

/// Decode the first image of a TIFF file.
pub fn decode_tiff(b: &[u8]) -> Result<Picture, PictureError> {
    let be = match b.get(0..4) {
        Some([b'I', b'I', 42, 0]) => false,
        Some([b'M', b'M', 0, 42]) => true,
        Some([b'I', b'I', 43, 0]) | Some([b'M', b'M', 0, 43]) => return err("BigTIFF isn't supported"),
        _ => return err("not a TIFF file"),
    };
    let r = Reader { b, be };
    let tags = read_tags(&r)?;
    let one = |t: u16, d: u32| tags.get(&t).and_then(|v| v.first().copied()).unwrap_or(d);

    let width = one(256, 0);
    let height = one(257, 0);
    if width == 0 || height == 0 {
        return err("the picture has no size");
    }
    if width as u64 * height as u64 > MAX_PIXELS {
        return err(format!("the picture is too large ({width} × {height})"));
    }
    let spp = one(277, 1) as usize;
    let bps = one(258, 1) as usize;
    if let Some(v) = tags.get(&258) {
        if v.iter().any(|&x| x as usize != bps) {
            return err("samples of different sizes aren't supported");
        }
    }
    if !matches!(bps, 1 | 2 | 4 | 8 | 16) {
        return err(format!("{bps} bits per sample isn't supported"));
    }
    if !(1..=8).contains(&spp) {
        return err(format!("{spp} samples per pixel isn't supported"));
    }
    if one(284, 1) != 1 && spp > 1 {
        return err("separate colour planes aren't supported");
    }
    if one(339, 1) == 3 {
        return err("floating-point samples aren't supported");
    }
    let compression = one(259, 1);
    let photometric = one(262, if compression == 2 || compression == 3 || compression == 4 { 0 } else { 1 });
    let fill_lsb = one(266, 1) == 2;
    let predictor = one(317, 1);
    let t4 = one(292, 0);

    // strips or tiles: the chunks and their size
    let (cw, ch, offsets, counts) = if let (Some(o), Some(c)) = (tags.get(&324), tags.get(&325)) {
        (one(322, 0), one(323, 0), o, c)
    } else if let (Some(o), Some(c)) = (tags.get(&273), tags.get(&279)) {
        (width, one(278, height).min(height).max(1), o, c)
    } else if let Some(o) = tags.get(&273) {
        // no byte counts: only an uncompressed single strip can be read
        if o.len() != 1 || compression != 1 {
            return err("the strip sizes are missing");
        }
        let n = (width as usize * spp * bps).div_ceil(8) * height as usize;
        return finish(width, height, spp, bps, photometric, be, &tags, vec![(0, 0, width, height, b.get(o[0] as usize..o[0] as usize + n).map(|s| s.to_vec()).ok_or(PictureError("the file is cut short".into()))?)]);
    } else {
        return err("the picture data is missing");
    };
    if cw == 0 || ch == 0 {
        return err("the tile size is missing");
    }
    if cw as u64 * ch as u64 > MAX_PIXELS || cw > 65536 {
        return err("the tiles are too large");
    }
    let across = width.div_ceil(cw) as usize;
    let down = height.div_ceil(ch) as usize;
    if offsets.len() < across * down || counts.len() < across * down {
        return err("some of the picture data is missing");
    }

    let mut chunks = Vec::with_capacity(across * down);
    for i in 0..across * down {
        let (cx, cy) = ((i % across) as u32 * cw, (i / across) as u32 * ch);
        // strips: the last one holds only the rows left; tiles are always full size
        let rows = if tags.contains_key(&324) { ch } else { ch.min(height - cy) };
        let (o, n) = (offsets[i] as usize, counts[i] as usize);
        let Some(src) = b.get(o..o.saturating_add(n).min(b.len())) else { return err("the file is cut short") };
        let row_bytes = (cw as usize * spp * bps).div_ceil(8);
        let want = row_bytes * rows as usize;
        let mut data = match compression {
            1 => src.to_vec(),
            32773 => packbits(src, want),
            5 => lzw(src, want)?,
            8 | 32946 => {
                let mut out = Vec::with_capacity(want);
                flate2::read::ZlibDecoder::new(src).take(want as u64).read_to_end(&mut out).map_err(|e| PictureError(format!("deflate: {e}")))?;
                out
            }
            2..=4 => {
                if spp != 1 || bps != 1 {
                    return err("fax compression needs a bilevel picture");
                }
                let src: Vec<u8> = if fill_lsb { src.iter().map(|x| x.reverse_bits()).collect() } else { src.to_vec() };
                fax(&src, compression, t4, cw as usize, rows as usize)?
            }
            6 | 7 => return err("JPEG-compressed pictures aren't supported"),
            c => return err(format!("compression {c} isn't supported")),
        };
        data.resize(want, 0);
        if predictor == 2 {
            undo_predictor(&mut data, row_bytes, spp, bps, be);
        } else if predictor != 1 {
            return err(format!("predictor {predictor} isn't supported"));
        }
        chunks.push((cx, cy, cw, rows, data));
    }
    finish(width, height, spp, bps, photometric, be, &tags, chunks)
}

/// Horizontal differencing (predictor 2) undone, row by row.
fn undo_predictor(d: &mut [u8], row_bytes: usize, spp: usize, bps: usize, be: bool) {
    for row in d.chunks_mut(row_bytes) {
        match bps {
            8 => {
                for i in spp..row.len() {
                    row[i] = row[i].wrapping_add(row[i - spp]);
                }
            }
            16 => {
                let get = |r: &[u8], i: usize| if be { u16::from_be_bytes([r[2 * i], r[2 * i + 1]]) } else { u16::from_le_bytes([r[2 * i], r[2 * i + 1]]) };
                for i in spp..row.len() / 2 {
                    let v = get(row, i).wrapping_add(get(row, i - spp));
                    let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                    row[2 * i] = b[0];
                    row[2 * i + 1] = b[1];
                }
            }
            _ => {} // not used with fewer bits in practice
        }
    }
}

/// Put the decoded chunks together into 8-bit pixels.
#[allow(clippy::too_many_arguments)]
fn finish(width: u32, height: u32, spp: usize, bps: usize, photometric: u32, be: bool, tags: &HashMap<u16, Vec<u32>>, chunks: Vec<(u32, u32, u32, u32, Vec<u8>)>) -> Result<Picture, PictureError> {
    let extra = tags.get(&338).map(|v| v.len()).unwrap_or(0);
    let channels: u8 = match photometric {
        0 | 1 => {
            if spp >= 2 && extra >= 1 { 4 } else { 1 }
        }
        2 => {
            if spp < 3 {
                return err("an RGB picture needs three samples per pixel");
            }
            if spp >= 4 && extra >= 1 { 4 } else { 3 }
        }
        3 => {
            if spp != 1 || bps > 8 {
                return err("a palette picture needs one sample of up to 8 bits");
            }
            3
        }
        5 => {
            if spp < 4 {
                return err("a CMYK picture needs four samples per pixel");
            }
            3
        }
        6 => return err("YCbCr pictures aren't supported"),
        p => return err(format!("colour type {p} isn't supported")),
    };
    let map = if photometric == 3 {
        let Some(m) = tags.get(&320) else { return err("the palette is missing") };
        let n = 1usize << bps;
        if m.len() < 3 * n {
            return err("the palette is short");
        }
        Some((m.clone(), n))
    } else {
        None
    };
    let max = (1u32 << bps) - 1;
    let scale = |v: u32| -> u8 { if bps == 16 { (v >> 8) as u8 } else { (v * 255 / max) as u8 } };

    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * h * channels as usize];
    for (cx, cy, cw, rows, data) in chunks {
        let row_bytes = (cw as usize * spp * bps).div_ceil(8);
        // one sample of a row: bit-packed MSB first, 8-bit, or 16-bit in the file's byte order
        let sample = |row: &[u8], i: usize| -> u32 {
            match bps {
                8 => row[i] as u32,
                16 => {
                    let (a, b) = (row[2 * i], row[2 * i + 1]);
                    (if be { u16::from_be_bytes([a, b]) } else { u16::from_le_bytes([a, b]) }) as u32
                }
                _ => {
                    let bit = i * bps;
                    ((row[bit / 8] >> (8 - bps - bit % 8)) as u32) & max
                }
            }
        };
        for y in 0..rows as usize {
            let gy = cy as usize + y;
            if gy >= h {
                break;
            }
            let Some(row) = data.get(y * row_bytes..(y + 1) * row_bytes) else { break };
            for x in 0..cw as usize {
                let gx = cx as usize + x;
                if gx >= w {
                    break;
                }
                let s = |k: usize| -> u32 { sample(row, x * spp + k) };
                let o = (gy * w + gx) * channels as usize;
                let px = &mut out[o..o + channels as usize];
                match photometric {
                    0 | 1 => {
                        let v = scale(s(0));
                        let v = if photometric == 0 { 255 - v } else { v };
                        if channels == 4 {
                            // grey with alpha → RGBA
                            px.copy_from_slice(&[v, v, v, scale(s(1))]);
                        } else {
                            px[0] = v;
                        }
                    }
                    2 => {
                        for (k, v) in px.iter_mut().enumerate() {
                            *v = scale(s(k));
                        }
                    }
                    3 => {
                        let (m, n) = map.as_ref().unwrap();
                        let i = s(0) as usize;
                        px[0] = (m[i] >> 8) as u8;
                        px[1] = (m[n + i] >> 8) as u8;
                        px[2] = (m[2 * n + i] >> 8) as u8;
                    }
                    _ => {
                        // CMYK
                        let k = 255 - scale(s(3)) as u32;
                        for (c, v) in px.iter_mut().enumerate() {
                            *v = ((255 - scale(s(c)) as u32) * k / 255) as u8;
                        }
                    }
                }
            }
        }
    }
    Ok(Picture { width, height, channels, data: out })
}

// ---------------------------------------------------------------- compressions

/// PackBits (compression 32773).
fn packbits(src: &[u8], want: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(want);
    let mut i = 0;
    while i < src.len() && out.len() < want {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let end = (i + n as usize + 1).min(src.len());
            out.extend_from_slice(&src[i..end]);
            i = end;
        } else if n != -128 {
            if let Some(&v) = src.get(i) {
                out.extend(std::iter::repeat_n(v, (1 - n as isize) as usize));
            }
            i += 1;
        }
    }
    out
}

/// TIFF LZW (compression 5): 9- to 12-bit codes, most significant bit first, code width
/// growing one code early.
fn lzw(src: &[u8], want: usize) -> Result<Vec<u8>, PictureError> {
    if src.len() >= 2 && src[0] == 0 && src[1] & 1 == 1 {
        return err("old-style LZW isn't supported");
    }
    let mut out = Vec::with_capacity(want);
    // the table as (prefix code, last byte, first byte, length)
    let mut table: Vec<(u16, u8, u8, u16)> = (0..256).map(|i| (u16::MAX, i as u8, i as u8, 1)).collect();
    table.push((0, 0, 0, 0)); // 256 clear
    table.push((0, 0, 0, 0)); // 257 end
    let (mut acc, mut nbits, mut pos) = (0u32, 0u32, 0usize);
    let mut width = 9;
    let mut prev: Option<u16> = None;
    let mut buf = Vec::new();
    let emit = |table: &Vec<(u16, u8, u8, u16)>, mut c: u16, buf: &mut Vec<u8>| {
        buf.clear();
        loop {
            let (p, b, _, _) = table[c as usize];
            buf.push(b);
            if p == u16::MAX {
                break;
            }
            c = p;
        }
        buf.reverse();
    };
    loop {
        while nbits < width {
            let Some(&b) = src.get(pos) else { return Ok(out) };
            acc = (acc << 8) | b as u32;
            nbits += 8;
            pos += 1;
        }
        let code = ((acc >> (nbits - width)) & ((1 << width) - 1)) as u16;
        nbits -= width;
        match code {
            256 => {
                table.truncate(258);
                width = 9;
                prev = None;
                continue;
            }
            257 => return Ok(out),
            _ => {}
        }
        let next = table.len() as u16;
        match prev {
            None => {
                if code >= 256 {
                    return err("LZW data is damaged");
                }
                out.push(code as u8);
            }
            Some(p) => {
                let first = if code < next {
                    if code as usize >= table.len() || (256..258).contains(&code) {
                        return err("LZW data is damaged");
                    }
                    table[code as usize].2
                } else if code == next {
                    table[p as usize].2
                } else {
                    return err("LZW data is damaged");
                };
                if table.len() < 4096 {
                    let pf = table[p as usize].2;
                    let len = table[p as usize].3 + 1;
                    table.push((p, first, pf, len));
                }
                emit(&table, code, &mut buf);
                out.extend_from_slice(&buf);
            }
        }
        if out.len() >= want {
            return Ok(out);
        }
        prev = Some(code);
        if table.len() + 1 >= (1 << width) && width < 12 {
            width += 1;
        }
    }
}

// ---------------------------------------------------------------- fax (ITU-T T.4 / T.6)

/// Modified Huffman codes as (bits, run), "0"/"1" strings: white terminating and make-up,
/// black terminating and make-up, and the make-up codes both share.
const WHITE: &[(&str, u16)] = &[
    ("00110101", 0), ("000111", 1), ("0111", 2), ("1000", 3), ("1011", 4), ("1100", 5), ("1110", 6), ("1111", 7),
    ("10011", 8), ("10100", 9), ("00111", 10), ("01000", 11), ("001000", 12), ("000011", 13), ("110100", 14), ("110101", 15),
    ("101010", 16), ("101011", 17), ("0100111", 18), ("0001100", 19), ("0001000", 20), ("0010111", 21), ("0000011", 22), ("0000100", 23),
    ("0101000", 24), ("0101011", 25), ("0010011", 26), ("0100100", 27), ("0011000", 28), ("00000010", 29), ("00000011", 30), ("00011010", 31),
    ("00011011", 32), ("00010010", 33), ("00010011", 34), ("00010100", 35), ("00010101", 36), ("00010110", 37), ("00010111", 38), ("00101000", 39),
    ("00101001", 40), ("00101010", 41), ("00101011", 42), ("00101100", 43), ("00101101", 44), ("00000100", 45), ("00000101", 46), ("00001010", 47),
    ("00001011", 48), ("01010010", 49), ("01010011", 50), ("01010100", 51), ("01010101", 52), ("00100100", 53), ("00100101", 54), ("01011000", 55),
    ("01011001", 56), ("01011010", 57), ("01011011", 58), ("01001010", 59), ("01001011", 60), ("00110010", 61), ("00110011", 62), ("00110100", 63),
    ("11011", 64), ("10010", 128), ("010111", 192), ("0110111", 256), ("00110110", 320), ("00110111", 384), ("01100100", 448), ("01100101", 512),
    ("01101000", 576), ("01100111", 640), ("011001100", 704), ("011001101", 768), ("011010010", 832), ("011010011", 896), ("011010100", 960), ("011010101", 1024),
    ("011010110", 1088), ("011010111", 1152), ("011011000", 1216), ("011011001", 1280), ("011011010", 1344), ("011011011", 1408), ("010011000", 1472), ("010011001", 1536),
    ("010011010", 1600), ("011000", 1664), ("010011011", 1728),
];
const BLACK: &[(&str, u16)] = &[
    ("0000110111", 0), ("010", 1), ("11", 2), ("10", 3), ("011", 4), ("0011", 5), ("0010", 6), ("00011", 7),
    ("000101", 8), ("000100", 9), ("0000100", 10), ("0000101", 11), ("0000111", 12), ("00000100", 13), ("00000111", 14), ("000011000", 15),
    ("0000010111", 16), ("0000011000", 17), ("0000001000", 18), ("00001100111", 19), ("00001101000", 20), ("00001101100", 21), ("00000110111", 22), ("00000101000", 23),
    ("00000010111", 24), ("00000011000", 25), ("000011001010", 26), ("000011001011", 27), ("000011001100", 28), ("000011001101", 29), ("000001101000", 30), ("000001101001", 31),
    ("000001101010", 32), ("000001101011", 33), ("000011010010", 34), ("000011010011", 35), ("000011010100", 36), ("000011010101", 37), ("000011010110", 38), ("000011010111", 39),
    ("000001101100", 40), ("000001101101", 41), ("000011011010", 42), ("000011011011", 43), ("000001010100", 44), ("000001010101", 45), ("000001010110", 46), ("000001010111", 47),
    ("000001100100", 48), ("000001100101", 49), ("000001010010", 50), ("000001010011", 51), ("000000100100", 52), ("000000110111", 53), ("000000111000", 54), ("000000100111", 55),
    ("000000101000", 56), ("000001011000", 57), ("000001011001", 58), ("000000101011", 59), ("000000101100", 60), ("000001011010", 61), ("000001100110", 62), ("000001100111", 63),
    ("0000001111", 64), ("000011001000", 128), ("000011001001", 192), ("000001011011", 256), ("000000110011", 320), ("000000110100", 384), ("000000110101", 448), ("0000001101100", 512),
    ("0000001101101", 576), ("0000001001010", 640), ("0000001001011", 704), ("0000001001100", 768), ("0000001001101", 832), ("0000001110010", 896), ("0000001110011", 960), ("0000001110100", 1024),
    ("0000001110101", 1088), ("0000001110110", 1152), ("0000001110111", 1216), ("0000001010010", 1280), ("0000001010011", 1344), ("0000001010100", 1408), ("0000001010101", 1472), ("0000001011010", 1536),
    ("0000001011011", 1600), ("0000001100100", 1664), ("0000001100101", 1728),
];
const SHARED: &[(&str, u16)] = &[
    ("00000001000", 1792), ("00000001100", 1856), ("00000001101", 1920), ("000000010010", 1984), ("000000010011", 2048), ("000000010100", 2112), ("000000010101", 2176),
    ("000000010110", 2240), ("000000010111", 2304), ("000000011100", 2368), ("000000011101", 2432), ("000000011110", 2496), ("000000011111", 2560),
];

/// (length, code) → run, for each colour.
fn code_maps() -> &'static [HashMap<(u8, u16), u16>; 2] {
    static M: std::sync::OnceLock<[HashMap<(u8, u16), u16>; 2]> = std::sync::OnceLock::new();
    M.get_or_init(|| {
        let build = |t: &[(&str, u16)]| -> HashMap<(u8, u16), u16> {
            t.iter().chain(SHARED).map(|(s, r)| ((s.len() as u8, u16::from_str_radix(s, 2).unwrap()), *r)).collect()
        };
        [build(WHITE), build(BLACK)]
    })
}

struct Bits<'a> {
    b: &'a [u8],
    pos: usize, // in bits
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u8> {
        let byte = *self.b.get(self.pos / 8)?;
        let v = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(v)
    }
    fn peek(&self, n: usize) -> Option<u16> {
        let mut v = 0u16;
        for i in 0..n {
            let p = self.pos + i;
            let byte = *self.b.get(p / 8)?;
            v = (v << 1) | ((byte >> (7 - p % 8)) & 1) as u16;
        }
        Some(v)
    }
    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
    fn done(&self) -> bool {
        self.pos >= self.b.len() * 8
    }
    /// Skip fill bits and EOL codes (eleven or more 0s then a 1). True if one was found.
    fn skip_eols(&mut self) -> bool {
        let mut found = false;
        loop {
            let start = self.pos;
            let mut zeros = 0;
            while self.peek(1) == Some(0) {
                self.pos += 1;
                zeros += 1;
            }
            if zeros >= 11 && self.peek(1) == Some(1) {
                self.pos += 1;
                found = true;
            } else {
                self.pos = start;
                return found;
            }
        }
    }
    /// One run length of a colour: make-up codes then a terminating code.
    fn run(&mut self, black: bool) -> Result<usize, PictureError> {
        let map = &code_maps()[black as usize];
        let mut total = 0usize;
        loop {
            let (mut code, mut len) = (0u16, 0u8);
            let r = loop {
                let Some(b) = self.bit() else { return err("fax data ends inside a run") };
                code = (code << 1) | b as u16;
                len += 1;
                if let Some(&r) = map.get(&(len, code)) {
                    break r;
                }
                if len >= 13 {
                    return err("fax data is damaged (unknown run code)");
                }
            };
            total += r as usize;
            if r < 64 {
                return Ok(total);
            }
        }
    }
}

/// 2-D mode codes.
enum Mode {
    Pass,
    Horizontal,
    Vertical(i32),
}

fn mode(b: &mut Bits) -> Result<Mode, PictureError> {
    const CODES: &[(u8, u16, i32)] = &[(1, 0b1, 0), (3, 0b011, 1), (3, 0b010, -1), (3, 0b001, 99), (4, 0b0001, 100), (6, 0b000011, 2), (6, 0b000010, -2), (7, 0b0000011, 3), (7, 0b0000010, -3)];
    let (mut code, mut len) = (0u16, 0u8);
    loop {
        let Some(bit) = b.bit() else { return err("fax data ends inside a row") };
        code = (code << 1) | bit as u16;
        len += 1;
        if let Some(&(_, _, m)) = CODES.iter().find(|(l, c, _)| *l == len && *c == code) {
            return Ok(match m {
                99 => Mode::Horizontal,
                100 => Mode::Pass,
                d => Mode::Vertical(d),
            });
        }
        if len >= 7 {
            return err("fax data uses an extension mode, not supported");
        }
    }
}

/// Set bits a..b of a row to 1 (black).
fn fill(row: &mut [u8], a: usize, b: usize) {
    for x in a..b {
        row[x / 8] |= 0x80 >> (x % 8);
    }
}

/// One 1-D (modified Huffman) row: its changing elements.
fn row_1d(b: &mut Bits, row: &mut [u8], width: usize) -> Result<Vec<usize>, PictureError> {
    let (mut x, mut black, mut changes) = (0usize, false, Vec::new());
    while x < width {
        let r = b.run(black)?;
        let end = (x + r).min(width);
        if black {
            fill(row, x, end);
        }
        x += r;
        changes.push(x.min(width));
        black = !black;
    }
    Ok(changes)
}

/// One 2-D row against the reference row's changing elements.
fn row_2d(b: &mut Bits, row: &mut [u8], width: usize, refc: &[usize]) -> Result<Vec<usize>, PictureError> {
    let mut changes = Vec::new();
    let mut a0: isize = -1;
    let mut black = false;
    // b1: the first changing element on the reference row right of a0 and of the colour
    // opposite a0's (changes at even indices go white → black)
    let b1_of = |a0: isize, black: bool| -> (usize, usize) {
        let mut i = if black { 1 } else { 0 };
        while i < refc.len() && (refc[i] as isize) <= a0 {
            i += 2;
        }
        let b1 = refc.get(i).copied().unwrap_or(width);
        let b2 = refc.get(i + 1).copied().unwrap_or(width);
        (b1, b2)
    };
    while a0 < width as isize {
        let start = a0.max(0) as usize;
        match mode(b)? {
            Mode::Pass => {
                let (_, b2) = b1_of(a0, black);
                if black {
                    fill(row, start, b2.min(width));
                }
                a0 = b2 as isize;
            }
            Mode::Horizontal => {
                let r1 = b.run(black)?;
                let r2 = b.run(!black)?;
                let a1 = (start + r1).min(width);
                let a2 = (start + r1 + r2).min(width);
                if black {
                    fill(row, start, a1);
                } else {
                    fill(row, a1, a2);
                }
                changes.push(a1);
                changes.push(a2);
                a0 = a2 as isize;
            }
            Mode::Vertical(d) => {
                let (b1, _) = b1_of(a0, black);
                let a1 = b1 as isize + d as isize;
                if a1 < 0 || a1 > width as isize || a1 < a0 {
                    return err("fax data is damaged (vertical mode out of the row)");
                }
                let a1 = a1 as usize;
                if black {
                    fill(row, start, a1);
                }
                changes.push(a1);
                a0 = a1 as isize;
                black = !black;
            }
        }
    }
    Ok(changes)
}

/// Fax-compressed rows (compression 2, 3 or 4) → 1-bit rows, 1 = black (as fax codes it;
/// the picture's colour type says what that means).
fn fax(src: &[u8], compression: u32, t4: u32, width: usize, rows: usize) -> Result<Vec<u8>, PictureError> {
    let rb = width.div_ceil(8);
    let mut out = vec![0u8; rb * rows];
    let mut b = Bits { b: src, pos: 0 };
    let mut refc: Vec<usize> = Vec::new(); // an all-white row has no changes
    let two_d = compression == 4 || (compression == 3 && t4 & 1 == 1);
    for y in 0..rows {
        let row = &mut out[y * rb..(y + 1) * rb];
        let changes = match compression {
            2 => {
                // modified Huffman: every row starts on a byte
                if y > 0 {
                    b.align();
                }
                row_1d(&mut b, row, width)
            }
            3 => {
                // EOLs (with or without fill bits before them) start the rows
                b.skip_eols();
                if b.done() {
                    break;
                }
                if two_d {
                    match b.bit() {
                        Some(1) => row_1d(&mut b, row, width),
                        Some(_) => row_2d(&mut b, row, width, &refc),
                        None => break,
                    }
                } else {
                    row_1d(&mut b, row, width)
                }
            }
            _ => {
                if b.peek(24) == Some(0x001001) {
                    break; // end of facsimile block
                }
                row_2d(&mut b, row, width, &refc)
            }
        };
        match changes {
            Ok(c) => refc = c,
            // keep the rows read so far: a damaged end shouldn't lose the whole picture
            Err(_) if y > 0 && compression != 2 => break,
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- PNG

/// The picture as a PNG file. Black-and-white pictures are written with 1 bit per pixel.
pub fn to_png(p: &Picture) -> Result<Vec<u8>, PictureError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, p.width, p.height);
        let bilevel = p.channels == 1 && p.data.iter().all(|&v| v == 0 || v == 255);
        enc.set_color(match p.channels {
            1 => png::ColorType::Grayscale,
            3 => png::ColorType::Rgb,
            _ => png::ColorType::Rgba,
        });
        enc.set_depth(if bilevel { png::BitDepth::One } else { png::BitDepth::Eight });
        enc.set_compression(png::Compression::Best);
        let mut w = enc.write_header().map_err(|e| PictureError(format!("PNG: {e}")))?;
        let data = if bilevel {
            let rb = (p.width as usize).div_ceil(8);
            let mut d = vec![0u8; rb * p.height as usize];
            for y in 0..p.height as usize {
                for x in 0..p.width as usize {
                    if p.data[y * p.width as usize + x] == 255 {
                        d[y * rb + x / 8] |= 0x80 >> (x % 8);
                    }
                }
            }
            d
        } else {
            p.data.clone()
        };
        w.write_image_data(&data).map_err(|e| PictureError(format!("PNG: {e}")))?;
    }
    Ok(out)
}

/// A picture file as PNG: TIFF decoded, PNG passed through. JPEG and GIF are returned
/// as they are with their type, since a web view shows them. (bytes, MIME type)
pub fn picture_for_display(b: &[u8]) -> Result<(Vec<u8>, &'static str), PictureError> {
    if b.starts_with(b"\x89PNG") {
        return Ok((b.to_vec(), "image/png"));
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok((b.to_vec(), "image/jpeg"));
    }
    if b.starts_with(b"GIF8") {
        return Ok((b.to_vec(), "image/gif"));
    }
    Ok((to_png(&decode_tiff(b)?)?, "image/png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_prefix_free() {
        for t in [WHITE, BLACK] {
            let all: Vec<&str> = t.iter().chain(SHARED).map(|(s, _)| *s).collect();
            for (i, a) in all.iter().enumerate() {
                for (j, b) in all.iter().enumerate() {
                    assert!(i == j || !b.starts_with(a), "{a} is a prefix of {b}");
                }
            }
        }
    }

    #[test]
    fn packbits_example() {
        // the example in the TIFF 6.0 specification
        let src = [0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7, 0xAA];
        let want = [0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA];
        assert_eq!(packbits(&src, want.len()), want);
    }

    #[test]
    fn not_tiff() {
        assert!(decode_tiff(b"hello").is_err());
        assert!(decode_tiff(b"II*\0\x08\0\0\0").is_err());
    }
}
