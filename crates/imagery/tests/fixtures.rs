//! The reader and renderer against small images made with GDAL (tests/data/make_fixtures.py;
//! the values GDAL reads back are in expected.json).

use std::path::PathBuf;
use std::sync::Arc;

use imagery::kap::Kap;
use imagery::render::{render_rgba, tile_range, Raster};
use imagery::source::FileSource;
use imagery::tiff::Tiff;
use serde_json::Value;

/// A test file: GeoTIFFs are kept as base64 text (`name.tif.b64`) and decoded to a temporary
/// folder on first use.
fn data(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let b64 = dir.join(format!("{name}.b64"));
    if !b64.exists() {
        return dir.join(name);
    }
    use base64::Engine;
    let out = std::env::temp_dir().join(format!("imagery-fixtures-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let path = out.join(name);
    if !path.exists() {
        let text: String = std::fs::read_to_string(&b64).unwrap().chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD.decode(text).unwrap();
        // written under another name, then renamed: tests run in parallel
        let tmp = out.join(format!("{name}.{:?}.part", std::thread::current().id()));
        std::fs::write(&tmp, bytes).unwrap();
        let _ = std::fs::rename(&tmp, &path);
    }
    path
}

fn expected() -> Value {
    serde_json::from_slice(&std::fs::read(data("expected.json")).unwrap()).unwrap()
}

fn open(name: &str) -> Tiff {
    Tiff::open(Arc::new(FileSource::open(&data(name)).unwrap())).unwrap()
}

fn pixel(r: &dyn Raster, level: usize, c: usize, row: usize) -> [u8; 4] {
    let (bw, bh) = r.block_size(level);
    let b = r.block(level, c / bw, row / bh).expect("block");
    let (x, y) = (c % bw, row % bh);
    let i = y * b.w + x;
    match &b.palette {
        Some(p) => p[b.data[i] as usize],
        None => [b.data[i * 4], b.data[i * 4 + 1], b.data[i * 4 + 2], b.data[i * 4 + 3]],
    }
}

fn check_pixels(name: &str, tol: i32) {
    let ex = expected();
    let t = open(name);
    let px = ex[name]["pixels"].as_object().unwrap();
    for (k, v) in px {
        let (c, r) = k.split_once(',').unwrap();
        let (c, r) = (c.parse::<usize>().unwrap(), r.parse::<usize>().unwrap());
        let got = pixel(&t, 0, c, r);
        let want: Vec<i32> = v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect();
        if want == [0, 0, 0] && t.nodata == Some(0) {
            assert_eq!(got[3], 0, "{name} {k}: no data should be transparent");
            continue;
        }
        for i in 0..3 {
            assert!((got[i] as i32 - want[i]).abs() <= tol, "{name} {k}: {got:?} vs {want:?}");
        }
        assert_eq!(got[3], 255, "{name} {k}");
    }
}

#[test]
fn deflate_cog_pixels_and_overviews() {
    check_pixels("utm_deflate.tif", 0);
    let t = open("utm_deflate.tif");
    assert_eq!(t.levels(), 3);
    assert_eq!(t.level_size(1), (300, 200));
    assert_eq!(t.level_size(2), (150, 100));
    assert_eq!(t.epsg, Some(32759));
    assert_eq!(t.nodata, Some(0));
    assert!((t.resolution_m() - 10.0).abs() < 0.1, "{}", t.resolution_m());
}

#[test]
fn jpeg_cogs() {
    check_pixels("utm_jpeg_ycbcr.tif", 6);
    check_pixels("utm_jpeg_rgb.tif", 6);
}

#[test]
fn palette_lzw_strips_in_lon_lat() {
    check_pixels("ll_palette_lzw.tif", 0);
    let t = open("ll_palette_lzw.tif");
    let b = t.bounds();
    assert!((b[0] - 173.0).abs() < 1e-9 && (b[3] + 41.0).abs() < 1e-9 && (b[2] - 173.6).abs() < 1e-9 && (b[1] + 41.4).abs() < 1e-9, "{b:?}");
}

#[test]
fn positions_match_gdal() {
    let ex = expected();
    let t = open("utm_deflate.tif");
    for p in ex["utm_points"].as_array().unwrap() {
        let (c, r) = t.to_pixel(p["lon"].as_f64().unwrap(), p["lat"].as_f64().unwrap()).unwrap();
        assert!((c - p["col"].as_f64().unwrap()).abs() < 0.01, "{c} vs {}", p["col"]);
        assert!((r - p["row"].as_f64().unwrap()).abs() < 0.01, "{r} vs {}", p["row"]);
    }
}

#[test]
fn renders_tiles() {
    let t = open("utm_deflate.tif");
    let b = t.bounds();
    for z in [10u8, 13, 16] {
        let (x0, y0, x1, y1) = tile_range(b, z);
        let mut drawn = 0;
        for x in x0..=x1 {
            for y in y0..=y1 {
                if render_rgba(&t, z, x, y).is_some() {
                    drawn += 1;
                }
            }
        }
        assert!(drawn > 0, "z{z}");
    }
    // far away: nothing
    assert!(render_rgba(&t, 10, 0, 0).is_none());
    // the tile under a known point shows that point's colour
    let (lon, lat): (f64, f64) = (173.18, -41.19);
    let z = 17u8;
    let n = (1u32 << z) as f64;
    let fx = (lon + 180.0) / 360.0 * n;
    let p = lat.to_radians();
    let fy = (1.0 - (p.tan() + 1.0 / p.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    let tile = render_rgba(&t, z, fx as u32, fy as u32).unwrap();
    let (ox, oy) = (((fx.fract()) * 256.0) as usize, ((fy.fract()) * 256.0) as usize);
    let got = &tile[(oy * 256 + ox) * 4..(oy * 256 + ox) * 4 + 4];
    let (c, r) = t.to_pixel(lon, lat).unwrap();
    let want = pixel(&t, 0, c as usize, r as usize);
    for i in 0..3 {
        assert!((got[i] as i32 - want[i] as i32).abs() <= 6, "{got:?} vs {want:?}");
    }
}

#[test]
fn kap_reads_like_gdal() {
    let ex = expected();
    assert_eq!(ex["test.kap"]["indices_match"], true);
    let k = Kap::open(&data("test.kap")).unwrap();
    assert_eq!((k.width, k.height), (300, 200));
    assert_eq!(k.name, "Test Harbour");
    assert_eq!(k.number, "TEST1");
    assert_eq!(k.scale, Some(25000));
    assert_eq!(k.edition, "05/11/2026");
    assert!(k.has_border());
    // same pixels as the fixture's own indices
    let idx = k.indices();
    for (r, c) in [(0usize, 0usize), (30, 25), (199, 299), (160, 260), (77, 143)] {
        let want = if r >= 150 && c >= 250 { 6 } else { 1 + ((c / 20 + r / 25) % 5) as u8 };
        assert_eq!(idx[r * 300 + c], want, "{c},{r}");
    }
    let gdal = ex["test.kap"]["gdal_pixels"].as_array().unwrap();
    for ((c, r), want) in [(0usize, 0usize), (25, 30), (299, 199), (260, 160)].iter().zip(gdal) {
        let got = pixel(&k, 0, *c, *r);
        let want: Vec<u8> = want.as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
        assert_eq!(&got[..3], &want[..], "{c},{r}");
    }
    // positions: the mercator fit is exact
    assert!(k.fit_rms_px < 0.01, "{}", k.fit_rms_px);
    for p in ex["test.kap"]["points"].as_array().unwrap() {
        let (c, r) = k.to_pixel(p["lon"].as_f64().unwrap(), p["lat"].as_f64().unwrap()).unwrap();
        assert!((c - p["col"].as_f64().unwrap()).abs() < 0.01 && (r - p["row"].as_f64().unwrap()).abs() < 0.01, "{c},{r} vs {p}");
    }
    // the border cuts off the east side (east of 173.25)
    let (c_in, r_in) = k.to_pixel(173.2, -41.2).unwrap();
    let (c_out, r_out) = k.to_pixel(173.28, -41.2).unwrap();
    assert!(k.inside(c_in, r_in));
    assert!(!k.inside(c_out, r_out));
    let b = k.bounds();
    assert!((b[0] - 173.0).abs() < 1e-6 && (b[2] - 173.25).abs() < 1e-6, "{b:?}");
    assert!(k.resolution_m() > 50.0 && k.resolution_m() < 120.0, "{}", k.resolution_m());
}

#[test]
fn kap_without_row_index_and_with_datum_shift() {
    let mut bytes = std::fs::read(data("test.kap")).unwrap();
    // spoil the index pointer: rows are found by walking them
    let n = bytes.len();
    bytes[n - 4..].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    let k = Kap::from_bytes(bytes.clone()).unwrap();
    assert_eq!(k.indices()[199 * 300 + 299], 6);
    // a DTM shift of +36" latitude moves the chart 0.01° north (the header grows, so the
    // spoilt index makes the rows be walked again)
    let hdr = bytes.iter().position(|&b| b == 0x1a).unwrap();
    let mut nb = String::from_utf8_lossy(&bytes[..hdr]).replace("DTM/0.0,0.0", "DTM/36.0,0.0").into_bytes();
    nb.extend_from_slice(&bytes[hdr..]);
    let k2 = Kap::from_bytes(nb).unwrap();
    let a = k.to_pixel(173.1, -41.15).unwrap();
    let b = k2.to_pixel(173.1, -41.14).unwrap();
    assert!((a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6, "{a:?} {b:?}");
}

#[test]
fn kap_renders() {
    let k = Kap::open(&data("test.kap")).unwrap();
    let (x0, y0, x1, y1) = tile_range(k.bounds(), 13);
    let mut drawn = 0;
    for x in x0..=x1 {
        for y in y0..=y1 {
            if render_rgba(&k, 13, x, y).is_some() {
                drawn += 1;
            }
        }
    }
    assert!(drawn > 0);
}

#[test]
fn geotiff_writer_round_trip() {
    let t = open("utm_deflate.tif");
    let (w, h) = t.size();
    let mut rgba = vec![0u8; w * h * 4];
    for r in 0..h {
        for c in 0..w {
            rgba[(r * w + c) * 4..(r * w + c) * 4 + 4].copy_from_slice(&pixel(&t, 0, c, r));
        }
    }
    let (_, aff) = t.geo.unwrap();
    let dir = std::env::temp_dir().join(format!("imagery-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("round.tif");
    imagery::tiffw::write_geotiff(&out, w, h, &rgba, &imagery::tiffw::GeoRef { epsg: 32759, aff }, "test").unwrap();
    let back = Tiff::open(Arc::new(FileSource::open(&out).unwrap())).unwrap();
    assert_eq!(back.size(), (w, h));
    assert_eq!(back.levels(), 3); // 600 → 300 → 150
    assert_eq!(back.epsg, Some(32759));
    assert_eq!(back.geo.unwrap().1, aff);
    for (c, r) in [(0, 0), (35, 25), (599, 399), (300, 200), (257, 3)] {
        assert_eq!(pixel(&back, 0, c, r), pixel(&t, 0, c, r), "{c},{r}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cut_a_window() {
    let t = open("utm_deflate.tif");
    let p = imagery::s2::Progress::default();
    // an area inside the image: 173.16..173.20 E, 41.20..41.18 S
    let area = [173.16, -41.20, 173.20, -41.18];
    let cut = imagery::s2::cut(&t, area, 4096, 3, &p).unwrap();
    assert!(cut.w > 300 && cut.h > 200, "{}x{}", cut.w, cut.h);
    assert_eq!(cut.geo.epsg, 32759);
    // the same place in the cut and the source
    let (lon, lat) = (173.18, -41.19);
    let (c, r) = t.to_pixel(lon, lat).unwrap();
    let (x, y) = cut
        .geo
        .aff
        .invert(imagery::geo::Crs::from_epsg(32759).unwrap().forward(lon, lat).0, imagery::geo::Crs::from_epsg(32759).unwrap().forward(lon, lat).1)
        .unwrap();
    let a = pixel(&t, 0, c as usize, r as usize);
    let i = (y as usize * cut.w + x as usize) * 4;
    assert_eq!(&cut.rgba[i..i + 4], &a);
    // a size limit picks an overview
    let small = imagery::s2::cut(&t, area, 100, 2, &p).unwrap();
    assert!(small.w <= 100 && small.h <= 100, "{}x{}", small.w, small.h);
    assert!(small.res_m > 35.0, "{}", small.res_m);
}

#[test]
fn store_imports_and_draws() {
    let dir = std::env::temp_dir().join(format!("imagery-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = imagery::store::Store::open(&dir);
    let k = s.import(&data("test.kap"), imagery::store::How::Copy).unwrap();
    assert_eq!(k.kind, "kap");
    assert_eq!(k.name, "TEST1 Test Harbour");
    let g = s.import(&data("utm_deflate.tif"), imagery::store::How::Copy).unwrap();
    assert_eq!(g.kind, "geotiff");
    assert_eq!(s.list().len(), 2);
    let again = imagery::store::Store::open(&dir);
    let ids = |l: Vec<imagery::store::Entry>| l.into_iter().map(|e| (e.id, e.name, e.file, e.bounds)).collect::<Vec<_>>();
    assert_eq!(ids(again.list()), ids(s.list()));
    let (x0, y0, _, _) = tile_range(k.bounds, 12);
    let mut any = false;
    for dx in 0..3 {
        for dy in 0..3 {
            any |= again.tile_png(&k.id, 12, x0 + dx, y0 + dy).is_some();
        }
    }
    assert!(any);
    again.update(&g.id, imagery::store::Patch { opacity: Some(0.5), placement: Some("over".into()), ..Default::default() }).unwrap();
    assert_eq!(again.list()[1].placement, "over");
    again.remove(&k.id).unwrap();
    assert_eq!(again.list().len(), 1);
    assert!(!dir.join(&k.file).exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// Images catalogued where they are: read from their own folder, found again by the full
/// path, not added twice, reported missing while the file is away, and never deleted.
#[test]
fn store_catalogues_in_place() {
    use imagery::store::{find_images, How, Store};
    let base = std::env::temp_dir().join(format!("imagery-linked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("tweedplot-images");
    // another program's chart folder, with a subfolder, a hidden folder and something else
    let charts = base.join("OpenCPN charts");
    std::fs::create_dir_all(charts.join("NZ").join("Nelson")).unwrap();
    std::fs::create_dir_all(charts.join(".cache")).unwrap();
    std::fs::copy(data("test.kap"), charts.join("NZ").join("Nelson").join("NZ61.KAP")).unwrap();
    std::fs::copy(data("utm_deflate.tif"), charts.join("NZ").join("sat.tif")).unwrap();
    std::fs::copy(data("test.kap"), charts.join(".cache").join("hidden.kap")).unwrap();
    std::fs::write(charts.join("readme.txt"), "not an image").unwrap();

    let (found, more) = find_images(&charts, Some(&dir), 100).unwrap();
    assert!(!more);
    let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["NZ61.KAP", "sat.tif"]);
    let (one, more) = find_images(&charts, None, 1).unwrap();
    assert!(more && one.len() == 1);

    let s = Store::open(&dir);
    let r = s.import_many(&found, How::InPlace, true);
    assert_eq!(r.added.len(), 2, "{:?}", r.errors);
    let k = r.added.iter().find(|e| e.kind == "kap").unwrap().clone();
    assert!(k.linked && std::path::Path::new(&k.file).is_absolute());
    assert_eq!(k.name, "TEST1 Test Harbour");
    // nothing copied in
    assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| e.file_name() == "images.json"));
    // again: nothing new
    let r = s.import_many(&found, How::InPlace, true);
    assert!(r.added.is_empty() && r.already.len() == 2);
    // reopened from images.json, and drawn from where it is
    let again = Store::open(&dir);
    assert_eq!(again.list().len(), 2);
    let (x0, y0, _, _) = tile_range(k.bounds, 12);
    assert!((0..3).any(|dx| (0..3).any(|dy| again.tile_png(&k.id, 12, x0 + dx, y0 + dy).is_some())));
    // the drive unplugged: missing, nothing drawn, still listed
    let away = charts.with_file_name("away");
    std::fs::rename(&charts, &away).unwrap();
    let fresh = Store::open(&dir);
    assert!(fresh.list().iter().all(|e| e.missing));
    assert!(fresh.tile_png(&k.id, 12, x0, y0).is_none());
    std::fs::rename(&away, &charts).unwrap();
    assert!(fresh.list().iter().all(|e| !e.missing));
    // removing it leaves the other program's file alone
    fresh.remove(&k.id).unwrap();
    assert_eq!(fresh.list().len(), 1);
    assert!(charts.join("NZ").join("Nelson").join("NZ61.KAP").is_file());
    // copying in still works next to linked images
    let c = fresh.import(&data("test.kap"), How::Copy).unwrap();
    assert!(!c.linked && dir.join(&c.file).is_file());
    std::fs::remove_dir_all(&base).ok();
}

/// The download path end to end over HTTP range requests, from a tiny server on this
/// computer (Earth Search's images are the same kind of file on S3).
#[test]
fn cut_over_http_range_requests() {
    use std::io::{BufRead, BufReader, Write};
    let bytes = std::fs::read(data("utm_jpeg_rgb.tif")).unwrap(); // larger than the 64 kB header read
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let s2 = served.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let bytes = bytes.clone();
            let s2 = s2.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut range = None;
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (a, b) = v.trim().split_once('-').unwrap();
                            range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
                        }
                    }
                    let (a, b) = range.unwrap();
                    let b = b.min(bytes.len() - 1);
                    let body = &bytes[a..=b];
                    s2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let head =
                        format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\n\r\n", body.len(), bytes.len());
                    if stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body)).is_err() {
                        return;
                    }
                }
            });
        }
    });
    let url = format!("http://127.0.0.1:{port}/TCI.tif");
    let src = Arc::new(imagery::source::HttpSource::new(&url).unwrap());
    let remote = Tiff::open(src.clone()).unwrap();
    let local = open("utm_jpeg_rgb.tif");
    let p = imagery::s2::Progress::default();
    // the whole image: its blocks run past the first 64 kB
    let area = local.bounds();
    let a = imagery::s2::cut(&remote, area, 4096, 6, &p).unwrap();
    let b = imagery::s2::cut(&local, area, 4096, 1, &p).unwrap();
    assert_eq!((a.w, a.h), (b.w, b.h));
    assert!(a.rgba == b.rgba);
    // the overviews (written after the full image, past the first 64 kB)
    let a = imagery::s2::cut(&remote, area, 200, 6, &p).unwrap();
    let b = imagery::s2::cut(&local, area, 200, 1, &p).unwrap();
    assert_eq!((a.w, a.h), (b.w, b.h));
    assert!(a.w <= 200 && a.rgba == b.rgba);
    assert_eq!(p.done.load(std::sync::atomic::Ordering::Relaxed), p.total.load(std::sync::atomic::Ordering::Relaxed));
    // the header once, then only the blocks needed
    let n = served.load(std::sync::atomic::Ordering::Relaxed);
    assert!(n >= 2 && n <= 1 + p.total.load(std::sync::atomic::Ordering::Relaxed), "{n} requests");
}
