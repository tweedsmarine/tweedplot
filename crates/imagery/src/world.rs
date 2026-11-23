//! Plain pictures (PNG, JPEG, and TIFFs without georeferencing) and world files.
//!
//! A world file (.jgw, .pgw, .tfw, .wld…) beside a picture places it: six numbers, the pixel
//! size and rotation and the centre of the top-left pixel. It doesn't say in which coordinate
//! system: a .prj or .aux.xml beside it may, else the user is asked.

use std::path::{Path, PathBuf};

use crate::geo::{Affine, Crs};
use crate::{err, Error, Result};

/// The largest picture taken in (pixels): a 12,000 × 12,000 scan.
pub const MAX_PIXELS: usize = 144_000_000;

/// The world file beside a picture, if there is one.
pub fn find_world_file(image: &Path) -> Option<PathBuf> {
    let ext = image.extension()?.to_string_lossy().to_ascii_lowercase();
    let short = match ext.as_str() {
        "jpg" | "jpeg" => vec!["jgw", "jpgw", "jpegw"],
        "png" => vec!["pgw", "pngw"],
        "tif" | "tiff" => vec!["tfw", "tifw", "tiffw"],
        "gif" => vec!["gfw", "gifw"],
        _ => vec![],
    };
    for e in short.iter().copied().chain(["wld"]) {
        for cand in [e.to_string(), e.to_ascii_uppercase()] {
            let p = image.with_extension(&cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// A world file's six numbers as an affine from pixel corners (column, row from the top-left
/// corner of the first pixel) to map coordinates.
pub fn read_world_file(path: &Path) -> Result<Affine> {
    let s = std::fs::read_to_string(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let v: Vec<f64> = s.split_whitespace().filter_map(|t| t.replace(',', ".").parse::<f64>().ok()).collect();
    if v.len() < 6 || v.iter().take(6).any(|x| !x.is_finite()) {
        return err(format!("{}: a world file has six numbers, one a line", path.display()));
    }
    // x = C + A·col + B·row, y = F + D·col + E·row, for the centre of pixel (col, row)
    let (a, d, b, e, c, f) = (v[0], v[1], v[2], v[3], v[4], v[5]);
    if (a * e - b * d).abs() < 1e-30 {
        return err(format!("{}: the pixel size is zero", path.display()));
    }
    Ok(Affine { a: c - 0.5 * a - 0.5 * b, b: a, c: b, d: f - 0.5 * d - 0.5 * e, e: d, f: e })
}

/// The EPSG code a .prj or .aux.xml beside the picture gives, if any.
pub fn find_crs(image: &Path) -> Option<u32> {
    let mut cands = vec![image.with_extension("prj"), image.with_extension("PRJ")];
    let name = image.file_name()?.to_string_lossy().into_owned();
    cands.push(image.with_file_name(format!("{name}.aux.xml")));
    for p in cands {
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Some(code) = epsg_from_wkt(&s) {
                return Some(code);
            }
        }
    }
    None
}

/// An EPSG code from a WKT text (WKT1's AUTHORITY or WKT2's ID for the whole system, else
/// the common names).
pub fn epsg_from_wkt(wkt: &str) -> Option<u32> {
    // the outermost AUTHORITY is the last in WKT1; WKT2's ID is the last too
    let mut last = None;
    for key in ["AUTHORITY[\"EPSG\",\"", "ID[\"EPSG\","] {
        let mut from = 0;
        while let Some(i) = wkt[from..].find(key) {
            let s = &wkt[from + i + key.len()..];
            let digits: String = s.chars().skip_while(|c| *c == '"').take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                last = Some(n);
            }
            from += i + key.len();
        }
    }
    if let Some(n) = last.filter(|n| Crs::from_epsg(*n).is_some()) {
        return Some(n);
    }
    let w = wkt.to_ascii_lowercase().replace([' ', '-'], "_");
    if w.contains("new_zealand_transverse_mercator") || w.contains("nztm") {
        return Some(2193);
    }
    if w.contains("pseudo_mercator") || w.contains("web_mercator") || w.contains("popular_visualisation") {
        return Some(3857);
    }
    if let Some(i) = w.find("utm_zone_") {
        let rest = &w[i + 9..];
        let zone: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let hemi = rest.chars().nth(zone.len());
        if let Ok(z) = zone.parse::<u32>() {
            if (1..=60).contains(&z) {
                let south = hemi == Some('s') || w.contains("south");
                return Some(if south { 32700 + z } else { 32600 + z });
            }
        }
    }
    if !w.contains("projcs") && !w.contains("projcrs") && (w.contains("wgs_1984") || w.contains("wgs84") || w.contains("wgs 84")) {
        return Some(4326);
    }
    None
}

/// A guess at the coordinate system from the numbers alone: degrees if they fit, else none.
pub fn looks_geographic(aff: &Affine, w: usize, h: usize) -> bool {
    let corners = [aff.apply(0.0, 0.0), aff.apply(w as f64, h as f64)];
    corners.iter().all(|(x, y)| x.abs() <= 360.0 && y.abs() <= 90.0) && aff.b.abs() < 1.0
}

/// Decode a PNG or JPEG into RGBA.
pub fn decode_picture(path: &Path) -> Result<(usize, usize, Vec<u8>)> {
    let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let mut dec = png::Decoder::new(std::io::Cursor::new(&bytes));
        dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
        let mut r = dec.read_info().map_err(|e| Error(format!("PNG: {e}")))?;
        let (w, h) = (r.info().width as usize, r.info().height as usize);
        if w * h > MAX_PIXELS {
            return err(too_big(w, h));
        }
        let mut buf = vec![0u8; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).map_err(|e| Error(format!("PNG: {e}")))?;
        let data = &buf[..info.buffer_size()];
        let rgba = match info.color_type {
            png::ColorType::Rgba => data.to_vec(),
            png::ColorType::GrayscaleAlpha => data.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Rgb => data.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return err("PNG: palette not expanded"),
        };
        return Ok((w, h, rgba));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;
        let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA).set_max_width(1 << 16).set_max_height(1 << 16);
        let mut d = zune_jpeg::JpegDecoder::new_with_options(bytes.as_slice(), opts);
        d.decode_headers().map_err(|e| Error(format!("JPEG: {e:?}")))?;
        let (w, h) = d.dimensions().ok_or_else(|| Error("JPEG: no size".into()))?;
        if w * h > MAX_PIXELS {
            return err(too_big(w, h));
        }
        let px = d.decode().map_err(|e| Error(format!("JPEG: {e:?}")))?;
        if px.len() != w * h * 4 {
            return err("JPEG: unexpected pixel layout");
        }
        return Ok((w, h, px));
    }
    err("not a PNG or JPEG picture")
}

fn too_big(w: usize, h: usize) -> String {
    format!("the picture is {w} × {h} pixels: over the {} million TweedPlot takes in. Cut it into parts, or save it as a tiled TIFF", MAX_PIXELS / 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_file_corner_and_pixel() {
        let d = std::env::temp_dir().join(format!("tp-wld-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let img = d.join("a.png");
        std::fs::write(&img, b"x").unwrap();
        std::fs::write(d.join("a.pgw"), "2.0\n0.0\n0.0\n-2.0\n1600001.0\n5430001.0\n").unwrap();
        let wf = find_world_file(&img).unwrap();
        let a = read_world_file(&wf).unwrap();
        // the top-left corner is half a pixel up and left of the first pixel's centre
        assert_eq!(a.apply(0.0, 0.0), (1_600_000.0, 5_430_002.0));
        assert_eq!(a.apply(1.0, 1.0), (1_600_002.0, 5_430_000.0));
        std::fs::write(
            d.join("a.prj"),
            r#"PROJCS["NZGD2000 / New Zealand Transverse Mercator 2000",GEOGCS["NZGD2000",AUTHORITY["EPSG","4167"]],AUTHORITY["EPSG","2193"]]"#,
        )
        .unwrap();
        assert_eq!(find_crs(&img), Some(2193));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn crs_names() {
        assert_eq!(epsg_from_wkt(r#"PROJCS["WGS_1984_UTM_Zone_59S",GEOGCS["GCS_WGS_1984"]]"#), Some(32759));
        assert_eq!(epsg_from_wkt(r#"PROJCS["WGS_1984_Web_Mercator_Auxiliary_Sphere"]"#), Some(3857));
        assert_eq!(epsg_from_wkt(r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984"]]"#), Some(4326));
        assert_eq!(epsg_from_wkt(r#"PROJCRS["x",ID["EPSG",2193]]"#), Some(2193));
        assert_eq!(epsg_from_wkt("LOCAL_CS[\"x\"]"), None);
    }

    #[test]
    fn decodes_png() {
        let d = std::env::temp_dir().join(format!("tp-png-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("b.png");
        std::fs::write(&p, crate::render::encode_png(&[255, 0, 0, 255, 0, 255, 0, 128], 2, 1)).unwrap();
        let (w, h, px) = decode_picture(&p).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(px, vec![255, 0, 0, 255, 0, 255, 0, 128]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
