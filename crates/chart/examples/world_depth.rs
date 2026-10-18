//! Build `world_depth.gpkg` (table `wdepth_ne`) from the Natural Earth 10 m
//! bathymetry GeoJSON files (public domain), for bundling with the app.
//!
//!     cargo run --release -p chart --example world_depth -- <dir with ne_10m_bathymetry_*.geojson> <out.gpkg> [tolerance°]
//!
//! Rings are simplified (Douglas–Peucker, default 0.004° ≈ 400 m): plenty for a
//! world overview that hydrographic charts or ETOPO areas replace closer in.

use chart::bathy::{polygon_rows, GpkgWriter};
use std::path::Path;

fn dp(pts: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (p, q) = (pts[a], pts[b]);
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let l2 = dx * dx + dy * dy;
        let mut best = (0.0, 0);
        for i in a + 1..b {
            let r = pts[i];
            let d = if l2 > 0.0 {
                ((r[0] - p[0]) * dy - (r[1] - p[1]) * dx).abs() / l2.sqrt()
            } else {
                ((r[0] - p[0]).powi(2) + (r[1] - p[1]).powi(2)).sqrt()
            };
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > tol {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Sutherland–Hodgman: a ring clipped to the box [w, s, e, n] (closed, or None if nothing is left).
fn clip(r: &[[f64; 2]], b: [f64; 4]) -> Option<Vec<[f64; 2]>> {
    let mut pts: Vec<[f64; 2]> = r.to_vec();
    if pts.first() == pts.last() {
        pts.pop();
    }
    for edge in 0..4 {
        let inside = |p: [f64; 2]| match edge {
            0 => p[0] >= b[0],
            1 => p[0] <= b[2],
            2 => p[1] >= b[1],
            _ => p[1] <= b[3],
        };
        let cross = |p: [f64; 2], q: [f64; 2]| {
            let t = match edge {
                0 => (b[0] - p[0]) / (q[0] - p[0]),
                1 => (b[2] - p[0]) / (q[0] - p[0]),
                2 => (b[1] - p[1]) / (q[1] - p[1]),
                _ => (b[3] - p[1]) / (q[1] - p[1]),
            };
            [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]
        };
        let mut out = Vec::with_capacity(pts.len());
        for i in 0..pts.len() {
            let (p, q) = (pts[(i + pts.len() - 1) % pts.len()], pts[i]);
            match (inside(p), inside(q)) {
                (true, true) => out.push(q),
                (true, false) => out.push(cross(p, q)),
                (false, true) => {
                    out.push(cross(p, q));
                    out.push(q);
                }
                _ => {}
            }
        }
        pts = out;
        if pts.len() < 3 {
            return None;
        }
    }
    pts.push(pts[0]);
    Some(pts)
}

/// Cut a polygon into the cells of a CELL° grid, so tiles only read small pieces.
const CELL: f64 = 5.0;
fn split(rings: Vec<Vec<[f64; 2]>>, cell: f64) -> Vec<Vec<Vec<[f64; 2]>>> {
    let ext = &rings[0];
    let (mut w, mut s, mut e, mut n) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in ext {
        w = w.min(p[0]);
        s = s.min(p[1]);
        e = e.max(p[0]);
        n = n.max(p[1]);
    }
    if e - w <= cell && n - s <= cell {
        return vec![rings];
    }
    let mut out = Vec::new();
    let mut x = (w / cell).floor() * cell;
    while x < e {
        let mut y = (s / cell).floor() * cell;
        while y < n {
            let b = [x, y, x + cell, y + cell];
            if let Some(ex) = clip(ext, b) {
                let mut piece = vec![ex];
                piece.extend(rings[1..].iter().filter_map(|h| clip(h, b)));
                out.push(piece);
            }
            y += cell;
        }
        x += cell;
    }
    out
}

fn ring(v: &serde_json::Value, tol: f64) -> Option<Vec<[f64; 2]>> {
    let pts: Vec<[f64; 2]> = v.as_array()?.iter().filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect();
    let r = dp(&pts, tol);
    (r.len() >= 4).then_some(r)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: world_depth <dir> <out.gpkg> [tolerance]");
        std::process::exit(2);
    }
    let dir = Path::new(&args[1]);
    let tol: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.004);
    let mut w = GpkgWriter::create(Path::new(&args[2])).unwrap();
    w.table("wdepth_ne", "POLYGON").unwrap();
    w.table("wdepth_nelo", "POLYGON").unwrap();
    w.meta("source", "Natural Earth 10 m bathymetry (public domain), naturalearthdata.com").unwrap();
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("ne_10m_bathymetry_"))).collect();
    files.sort();
    files.reverse(); // L_0 first … A_10000 last: shallow under deep
    let mut total = 0;
    for f in files {
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        let mut polys = Vec::new();
        let mut lo = Vec::new();
        for feat in j["features"].as_array().unwrap() {
            let depth = feat["properties"]["depth"].as_f64().unwrap_or(0.0);
            let g = &feat["geometry"];
            let parts: Vec<&serde_json::Value> = match g["type"].as_str() {
                Some("Polygon") => vec![&g["coordinates"]],
                Some("MultiPolygon") => g["coordinates"].as_array().unwrap().iter().collect(),
                _ => vec![],
            };
            for p in parts {
                let rings: Vec<Vec<[f64; 2]>> = p.as_array().unwrap().iter().filter_map(|r| ring(r, tol)).collect();
                // small scales (zoom 0–3): five depths, coarser, and no rings under ~0.5° across
                let lrings: Vec<Vec<[f64; 2]>> = p.as_array().unwrap().iter().filter_map(|r| ring(r, tol * 25.0)).filter(|r| {
                    let (mut a, mut b) = ([f64::MAX; 2], [f64::MIN; 2]);
                    for q in r { a = [a[0].min(q[0]), a[1].min(q[1])]; b = [b[0].max(q[0]), b[1].max(q[1])]; }
                    (b[0] - a[0]).max(b[1] - a[1]) > 0.5
                }).collect();
                if !lrings.is_empty() && [0.0, 200.0, 1000.0, 3000.0, 5000.0].contains(&depth) {
                    for piece in split(lrings, 30.0) {
                        lo.push((piece, depth));
                    }
                }
                if !rings.is_empty() {
                    for piece in split(rings, CELL) {
                        polys.push((piece, depth));
                    }
                }
            }
        }
        let n = w.insert("wdepth_ne", polygon_rows(polys)).unwrap();
        w.insert("wdepth_nelo", polygon_rows(lo)).unwrap();
        println!("{}: {n} polygons", f.file_name().unwrap().to_string_lossy());
        total += n;
    }
    let out = w.finish().unwrap();
    println!("{} polygons → {} ({} kB)", total, out.display(), std::fs::metadata(&out).unwrap().len() / 1024);
}
