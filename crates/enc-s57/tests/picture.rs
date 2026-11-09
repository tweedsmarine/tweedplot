//! TIFF pictures (PICREP): each test file decoded and compared with a PNG of the same
//! picture. The files were written by libtiff (through Pillow) and ImageMagick in every
//! compression and colour type the decoder reads.

use enc_s57::picture::{decode_tiff, picture_for_display, to_png, Picture};
use std::path::PathBuf;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiff").join(name)
}

/// A PNG as 8-bit samples (grey, RGB or RGBA).
fn png(name: &str) -> Picture {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(data(name)).unwrap()));
    let mut dec = dec;
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let channels = info.color_type.samples() as u8;
    Picture { width: info.width, height: info.height, channels, data: buf }
}

fn check(tif: &str, reference: &str, tolerance: u8) {
    let got = decode_tiff(&std::fs::read(data(tif)).unwrap()).unwrap_or_else(|e| panic!("{tif}: {e}"));
    let want = png(reference);
    assert_eq!((got.width, got.height, got.channels), (want.width, want.height, want.channels), "{tif}");
    let bad = got.data.iter().zip(&want.data).position(|(a, b)| a.abs_diff(*b) > tolerance);
    if let Some(i) = bad {
        let px = i / got.channels as usize;
        panic!("{tif}: pixel ({}, {}) is {} not {}", px % got.width as usize, px / got.width as usize, got.data[i], want.data[i]);
    }
}

#[test]
fn bilevel_every_compression() {
    for f in [
        "bw-raw.tif",
        "bw-packbits.tif",
        "bw-tiff_lzw.tif",
        "bw-tiff_adobe_deflate.tif",
        "bw-tiff_ccitt.tif",
        "bw-group3.tif",
        "bw-g3-2d.tif",
        "bw-g3-2d-fill.tif",
        "bw-g3-lsb.tif",
        "bw-fax-im.tif",
        "bw-group4.tif",
        "bw-g4-strips.tif",
        "bw-g4-miniswhite.tif",
    ] {
        check(f, "bw.png", 0);
    }
}

#[test]
fn grey() {
    for f in ["grey-raw.tif", "grey-lzw.tif", "grey-packbits.tif", "grey-mm-pred.tif"] {
        check(f, "grey.png", 0);
    }
    check("grey16-mm.tif", "grey.png", 1);
    check("grey4.tif", "grey.png", 17);
}

#[test]
fn colour() {
    for f in ["rgb-lzw.tif", "rgb-deflate.tif", "rgb-tiles.tif"] {
        check(f, "rgb.png", 0);
    }
    check("pal-lzw.tif", "pal.png", 0);
    check("rgba-lzw.tif", "rgba.png", 0);
}

#[test]
fn png_out() {
    // a bilevel picture comes out as a 1-bit PNG that reads back the same
    let p = decode_tiff(&std::fs::read(data("bw-group4.tif")).unwrap()).unwrap();
    let out = to_png(&p).unwrap();
    assert!(out.len() < 3000, "{} bytes", out.len());
    let dir = std::env::temp_dir().join(format!("enc-s57-picture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.png"), &out).unwrap();
    let back = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(dir.join("x.png")).unwrap()));
    let mut back = back;
    back.set_transformations(png::Transformations::EXPAND);
    let mut r = back.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    assert_eq!(buf, png("bw.png").data);
    let _ = std::fs::remove_dir_all(dir);

    // PNG passes through
    let b = std::fs::read(data("bw.png")).unwrap();
    assert_eq!(picture_for_display(&b).unwrap(), (b, "image/png"));
}

/// Damaged files give an error, never a panic.
#[test]
fn damaged() {
    for f in ["bw-group4.tif", "bw-g3-2d.tif", "rgb-lzw.tif", "bw-tiff_ccitt.tif", "pal-lzw.tif"] {
        let b = std::fs::read(data(f)).unwrap();
        for cut in [8, 20, b.len() / 3, b.len() / 2, b.len() - 10] {
            let _ = decode_tiff(&b[..cut]);
        }
        let mut seed = 12345u32;
        for _ in 0..200 {
            let mut c = b.clone();
            for _ in 0..4 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let i = (seed >> 8) as usize % c.len();
                c[i] ^= (seed >> 3) as u8 | 1;
            }
            let _ = decode_tiff(&c);
        }
    }
}
