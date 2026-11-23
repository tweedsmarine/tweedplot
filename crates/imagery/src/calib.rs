//! Calibration (georeferencing by control points) of the user's images: a screenshot, a
//! scanned or sketch chart, a satellite capture, or a KAP or GeoTIFF that is offset.
//!
//! Each control point pairs a pixel of the image with its true position (clicked on the
//! chart, snapped to a mark or own-track point, or typed). A transform is fitted from the
//! image's own coordinates to web mercator:
//! - a plain picture's own coordinates are its pixels (column, and row upwards);
//! - a georeferenced image's (KAP, GeoTIFF) are where its georeferencing puts each pixel, so
//!   a calibration only corrects it, and one point is enough to shift it.
//!
//! Models by the number of points used: 1 shift (georeferenced images only), 2 shift, scale
//! and rotation, 3 or more affine, 6 or more optionally a second-order polynomial (skewed
//! scans and oblique captures). A nudge (move and turn the whole image, in ground metres and
//! degrees) is applied after the fit, so a picture can be lined up by eye with the own track.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::geo::{distance_m, web_mercator, web_mercator_inv};
use crate::render::{Block, Raster};
use crate::{err, Result};

/// A control point: a pixel of the image (full resolution, column and row from the top-left
/// corner) and its WGS84 position. Either end may be missing while it is being made.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    #[serde(default)]
    pub px: Option<[f64; 2]>,
    /// [longitude, latitude]
    #[serde(default)]
    pub ll: Option<[f64; 2]>,
    /// Left out of the fit (an outlier) but kept in the list.
    #[serde(default)]
    pub off: bool,
    /// Where the position came from ("chart", "mark: Bean Rock", "typed"…), for the list.
    #[serde(default)]
    pub from: String,
}

/// Moving and turning the whole image after the fit.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Nudge {
    /// Ground metres east and north.
    #[serde(default)]
    pub east_m: f64,
    #[serde(default)]
    pub north_m: f64,
    /// Degrees clockwise about the image's centre.
    #[serde(default)]
    pub turn_deg: f64,
}

impl Nudge {
    fn is_zero(&self) -> bool {
        self.east_m == 0.0 && self.north_m == 0.0 && self.turn_deg == 0.0
    }
}

/// A calibration as saved with the image.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Calib {
    #[serde(default)]
    pub points: Vec<Point>,
    /// "auto", "shift", "similarity", "affine" or "poly2".
    #[serde(default = "auto")]
    pub model: String,
    #[serde(default)]
    pub nudge: Nudge,
}

fn auto() -> String {
    "auto".into()
}

/// A polynomial (order 1 to 3) between two plane coordinate systems, on normalised
/// coordinates for numerical stability.
#[derive(Clone, Debug)]
pub struct Poly {
    cin: (f64, f64),
    kin: f64,
    cout: (f64, f64),
    kout: f64,
    order: u8,
    cx: Vec<f64>,
    cy: Vec<f64>,
}

fn terms(u: f64, v: f64, order: u8) -> Vec<f64> {
    let mut t = vec![1.0, u, v];
    if order >= 2 {
        t.extend([u * u, u * v, v * v]);
    }
    if order >= 3 {
        t.extend([u * u * u, u * u * v, u * v * v, v * v * v]);
    }
    t
}

impl Poly {
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (u, v) = ((x - self.cin.0) / self.kin, (y - self.cin.1) / self.kin);
        let t = terms(u, v, self.order);
        let p: f64 = t.iter().zip(&self.cx).map(|(a, b)| a * b).sum();
        let q: f64 = t.iter().zip(&self.cy).map(|(a, b)| a * b).sum();
        (p * self.kout + self.cout.0, q * self.kout + self.cout.1)
    }

    /// The exact inverse of a first-order polynomial.
    fn invert1(&self) -> Option<Poly> {
        if self.order != 1 {
            return None;
        }
        // out' = c + P in'  →  in' = P⁻¹ (out' − c)
        let (c0, p00, p01) = (self.cx[0], self.cx[1], self.cx[2]);
        let (c1, p10, p11) = (self.cy[0], self.cy[1], self.cy[2]);
        let det = p00 * p11 - p01 * p10;
        if det.abs() < 1e-15 {
            return None;
        }
        let (i00, i01, i10, i11) = (p11 / det, -p01 / det, -p10 / det, p00 / det);
        Some(Poly {
            cin: self.cout,
            kin: self.kout,
            cout: self.cin,
            kout: self.kin,
            order: 1,
            cx: vec![-(i00 * c0 + i01 * c1), i00, i01],
            cy: vec![-(i10 * c0 + i11 * c1), i10, i11],
        })
    }
}

fn centre_scale(pts: &[(f64, f64)]) -> ((f64, f64), f64) {
    let n = pts.len().max(1) as f64;
    let c = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
    let k = (pts.iter().map(|p| (p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sum::<f64>() / n).sqrt();
    (c, if k > 1e-9 { k } else { 1.0 })
}

/// Fit `model` taking `from` to `to`. Shift keeps the scale (both sides normalised alike).
fn fit_poly(from: &[(f64, f64)], to: &[(f64, f64)], model: &str) -> Option<Poly> {
    let (cin, kin) = centre_scale(from);
    let (cout, kout) = if model == "shift" { (cin, kin) } else { centre_scale(to) };
    let nf: Vec<(f64, f64)> = from.iter().map(|p| ((p.0 - cin.0) / kin, (p.1 - cin.1) / kin)).collect();
    let nt: Vec<(f64, f64)> = to.iter().map(|p| ((p.0 - cout.0) / kout, (p.1 - cout.1) / kout)).collect();
    let n = nf.len() as f64;
    let (cx, cy, order) = match model {
        "shift" => {
            let dx = nf.iter().zip(&nt).map(|(a, b)| b.0 - a.0).sum::<f64>() / n;
            let dy = nf.iter().zip(&nt).map(|(a, b)| b.1 - a.1).sum::<f64>() / n;
            (vec![dx, 1.0, 0.0], vec![dy, 0.0, 1.0], 1)
        }
        "similarity" => {
            // x' = a u − b v + tx, y' = b u + a v + ty
            let mut rows = Vec::new();
            let mut rhs = Vec::new();
            for (a, b) in nf.iter().zip(&nt) {
                rows.push(vec![a.0, -a.1, 1.0, 0.0]);
                rhs.push(b.0);
                rows.push(vec![a.1, a.0, 0.0, 1.0]);
                rhs.push(b.1);
            }
            let s = crate::geo::least_squares(&rows, &rhs)?;
            (vec![s[2], s[0], -s[1]], vec![s[3], s[1], s[0]], 1)
        }
        _ => {
            let order = match model {
                "poly2" => 2,
                "poly3" => 3,
                _ => 1,
            };
            let rows: Vec<Vec<f64>> = nf.iter().map(|p| terms(p.0, p.1, order)).collect();
            let fx = crate::geo::least_squares(&rows, &nt.iter().map(|p| p.0).collect::<Vec<_>>())?;
            let fy = crate::geo::least_squares(&rows, &nt.iter().map(|p| p.1).collect::<Vec<_>>())?;
            (fx, fy, order)
        }
    };
    Some(Poly { cin, kin, cout, kout, order, cx, cy })
}

/// A fitted calibration: image coordinates ↔ web mercator metres, with the nudge.
#[derive(Clone, Debug)]
pub struct Warp {
    fwd: Poly,
    inv: Poly,
    nudge: Nudge,
    /// Turning centre (web mercator) and the metres-per-ground-metre factor there.
    centre: (f64, f64),
    k: f64,
}

impl Warp {
    pub fn forward(&self, x: f64, y: f64) -> (f64, f64) {
        let (mx, my) = self.fwd.apply(x, y);
        if self.nudge.is_zero() {
            return (mx, my);
        }
        let t = -self.nudge.turn_deg.to_radians();
        let (dx, dy) = (mx - self.centre.0, my - self.centre.1);
        let (c, s) = (t.cos(), t.sin());
        (self.centre.0 + c * dx - s * dy + self.nudge.east_m * self.k, self.centre.1 + s * dx + c * dy + self.nudge.north_m * self.k)
    }

    pub fn inverse(&self, mx: f64, my: f64) -> (f64, f64) {
        let (mx, my) = if self.nudge.is_zero() {
            (mx, my)
        } else {
            let t = self.nudge.turn_deg.to_radians();
            let (dx, dy) = (mx - self.nudge.east_m * self.k - self.centre.0, my - self.nudge.north_m * self.k - self.centre.1);
            let (c, s) = (t.cos(), t.sin());
            (self.centre.0 + c * dx - s * dy, self.centre.1 + s * dx + c * dy)
        };
        self.inv.apply(mx, my)
    }
}

/// The result of fitting a calibration.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FitReport {
    /// The model used ("shift", "similarity", "affine", "poly2"), or "" when there was nothing to fit.
    pub model: String,
    /// Ground metres between each point's position and where the fit puts its pixel (None:
    /// the point isn't complete).
    pub residuals: Vec<Option<f64>>,
    /// Root mean square of the residuals of the points used.
    pub rms_m: Option<f64>,
    /// Why there is no fit yet, or a warning ("fewer points than this model needs").
    pub note: String,
    /// The image's [west, south, east, north] with this calibration.
    pub bounds: Option<[f64; 4]>,
    /// Ground metres per pixel with this calibration.
    pub res_m: Option<f64>,
}

/// An image's own coordinates for a pixel: (column, −row) for a plain picture, else web
/// mercator from its georeferencing.
fn own(base: &dyn Raster, pixel_space: bool, col: f64, row: f64) -> Option<(f64, f64)> {
    if pixel_space {
        Some((col, -row))
    } else {
        let (lon, lat) = base.to_lonlat(col, row)?;
        Some(web_mercator(lon, lat))
    }
}

/// Fit a calibration. Returns the warp (None while there is too little to fit) and the report.
pub fn fit(base: &dyn Raster, pixel_space: bool, c: &Calib) -> (Option<Warp>, FitReport) {
    let (w, h) = base.size();
    let mut report = FitReport { model: String::new(), residuals: vec![None; c.points.len()], rms_m: None, note: String::new(), bounds: None, res_m: None };
    let used: Vec<usize> = (0..c.points.len()).filter(|&i| !c.points[i].off && c.points[i].px.is_some() && c.points[i].ll.is_some()).collect();
    let mut from = Vec::new();
    let mut to = Vec::new();
    let mut idx = Vec::new();
    for &i in &used {
        let (px, ll) = (c.points[i].px.expect("checked"), c.points[i].ll.expect("checked"));
        if let Some(s) = own(base, pixel_space, px[0], px[1]) {
            from.push(s);
            to.push(web_mercator(ll[0], ll[1]));
            idx.push(i);
        }
    }
    let n = from.len();
    let model = match c.model.as_str() {
        "shift" | "similarity" | "affine" | "poly2" => c.model.clone(),
        _ => match n {
            0 => String::new(),
            1 => "shift".into(),
            2 => "similarity".into(),
            _ => "affine".into(),
        },
    };
    let need = match model.as_str() {
        "shift" => 1,
        "similarity" => 2,
        "affine" => 3,
        "poly2" => 6,
        _ => 1,
    };
    let fwd = if n == 0 {
        if pixel_space {
            report.note = "Add at least two points: click a place on the image, then the same place on the chart.".into();
            return (None, report);
        }
        // a georeferenced image with no points: as it is (with any nudge)
        Some(Poly { cin: (0.0, 0.0), kin: 1.0, cout: (0.0, 0.0), kout: 1.0, order: 1, cx: vec![0.0, 1.0, 0.0], cy: vec![0.0, 0.0, 1.0] })
    } else if pixel_space && n == 1 {
        report.note = "One point only places the image: add a second for its scale and direction.".into();
        return (None, report);
    } else if n < need {
        report.note = format!("{} needs at least {need} points; there are {n}.", model_name(&model));
        return (None, report);
    } else if pixel_space && model == "shift" {
        report.note = "A plain picture can't be shifted only: it has no scale yet. Use Automatic.".into();
        return (None, report);
    } else {
        fit_poly(&from, &to, &model)
    };
    let Some(fwd) = fwd else {
        report.note = "The points don't fix the image (are they all in a line?).".into();
        return (None, report);
    };
    let inv = match fwd.invert1() {
        Some(i) => i,
        None => {
            // a second-order fit: its inverse fitted on a grid over the image
            let mut gi = Vec::new();
            let mut go = Vec::new();
            for j in 0..=24 {
                for i in 0..=24 {
                    let (col, row) = (w as f64 * i as f64 / 24.0, h as f64 * j as f64 / 24.0);
                    if let Some(s) = own(base, pixel_space, col, row) {
                        gi.push(fwd.apply(s.0, s.1));
                        go.push(s);
                    }
                }
            }
            match fit_poly(&gi, &go, "poly3") {
                Some(p) => p,
                None => {
                    report.note = "The fit can't be reversed: check the points.".into();
                    return (None, report);
                }
            }
        }
    };
    let mid = own(base, pixel_space, w as f64 / 2.0, h as f64 / 2.0).map(|s| fwd.apply(s.0, s.1)).unwrap_or((0.0, 0.0));
    let lat = web_mercator_inv(mid.0, mid.1).1;
    let warp = Warp { fwd, inv, nudge: c.nudge, centre: mid, k: 1.0 / lat.to_radians().cos().max(0.01) };
    let mut sum = 0.0;
    for (i, p) in c.points.iter().enumerate() {
        let (Some(px), Some(ll)) = (p.px, p.ll) else { continue };
        let Some(s) = own(base, pixel_space, px[0], px[1]) else { continue };
        let (mx, my) = warp.forward(s.0, s.1);
        let d = distance_m(web_mercator_inv(mx, my), (ll[0], ll[1]));
        report.residuals[i] = Some(d);
        if idx.contains(&i) {
            sum += d * d;
        }
    }
    if n > 0 {
        report.rms_m = Some((sum / n as f64).sqrt());
    }
    report.model = if n == 0 { "nudge".into() } else { model };
    let cal = Calibrated::new(Arc::new(NoPixels(base.size())), pixel_space, warp.clone(), Some(base));
    report.bounds = Some(cal.bounds);
    report.res_m = Some(cal.res_m);
    (Some(warp), report)
}

fn model_name(m: &str) -> &'static str {
    match m {
        "shift" => "Shift",
        "similarity" => "Shift, scale and turn",
        "affine" => "Affine",
        "poly2" => "Second order",
        _ => "This model",
    }
}

/// A stand-in raster for measuring an image's extent without its pixels.
struct NoPixels((usize, usize));

impl Raster for NoPixels {
    fn size(&self) -> (usize, usize) {
        self.0
    }
    fn levels(&self) -> usize {
        1
    }
    fn level_size(&self, _: usize) -> (usize, usize) {
        self.0
    }
    fn block_size(&self, _: usize) -> (usize, usize) {
        self.0
    }
    fn block(&self, _: usize, _: usize, _: usize) -> Option<Arc<Block>> {
        None
    }
    fn to_pixel(&self, _: f64, _: f64) -> Option<(f64, f64)> {
        None
    }
    fn bounds(&self) -> [f64; 4] {
        [0.0; 4]
    }
    fn resolution_m(&self) -> f64 {
        1.0
    }
}

/// An image drawn through its calibration.
pub struct Calibrated {
    base: Arc<dyn Raster>,
    pixel_space: bool,
    warp: Warp,
    bounds: [f64; 4],
    res_m: f64,
}

impl Calibrated {
    /// `measure`: the raster to take the image's own coordinates from when `base` has no
    /// pixels of its own (fitting), else `base`.
    fn new(base: Arc<dyn Raster>, pixel_space: bool, warp: Warp, measure: Option<&dyn Raster>) -> Calibrated {
        let m: &dyn Raster = measure.unwrap_or(base.as_ref());
        let (w, h) = m.size();
        let to_ll = |col: f64, row: f64| {
            own(m, pixel_space, col, row).map(|s| {
                let (x, y) = warp.forward(s.0, s.1);
                web_mercator_inv(x, y)
            })
        };
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for i in 0..=24 {
            let t = i as f64 / 24.0;
            for (c, r) in [(t * w as f64, 0.0), (t * w as f64, h as f64), (0.0, t * h as f64), (w as f64, t * h as f64)] {
                if let Some((lon, lat)) = to_ll(c, r) {
                    b = [b[0].min(lon), b[1].min(lat), b[2].max(lon), b[3].max(lat)];
                }
            }
        }
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let res_m = match (to_ll(cx, cy), to_ll(cx + 1.0, cy), to_ll(cx, cy + 1.0)) {
            (Some(p), Some(a), Some(c)) => (distance_m(p, a) * distance_m(p, c)).sqrt(),
            _ => 1.0,
        };
        Calibrated { base, pixel_space, warp, bounds: b, res_m }
    }

    /// The base image drawn through a fitted calibration.
    pub fn wrap(base: Arc<dyn Raster>, pixel_space: bool, c: &Calib) -> Result<Calibrated> {
        let (warp, report) = fit(base.as_ref(), pixel_space, c);
        let Some(warp) = warp else { return err(report.note) };
        Ok(Calibrated::new(base, pixel_space, warp, None))
    }
}

impl Raster for Calibrated {
    fn size(&self) -> (usize, usize) {
        self.base.size()
    }
    fn levels(&self) -> usize {
        self.base.levels()
    }
    fn level_size(&self, level: usize) -> (usize, usize) {
        self.base.level_size(level)
    }
    fn block_size(&self, level: usize) -> (usize, usize) {
        self.base.block_size(level)
    }
    fn block(&self, level: usize, bx: usize, by: usize) -> Option<Arc<Block>> {
        self.base.block(level, bx, by)
    }
    fn to_pixel(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let (mx, my) = web_mercator(lon, lat);
        let (x, y) = self.warp.inverse(mx, my);
        if self.pixel_space {
            Some((x, -y))
        } else {
            let (lo, la) = web_mercator_inv(x, y);
            self.base.to_pixel(lo, la)
        }
    }
    fn to_lonlat(&self, col: f64, row: f64) -> Option<(f64, f64)> {
        let s = own(self.base.as_ref(), self.pixel_space, col, row)?;
        let (x, y) = self.warp.forward(s.0, s.1);
        Some(web_mercator_inv(x, y))
    }
    fn inside(&self, col: f64, row: f64) -> bool {
        self.base.inside(col, row)
    }
    fn bounds(&self) -> [f64; 4] {
        self.bounds
    }
    fn resolution_m(&self) -> f64 {
        self.res_m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain 1000 × 800 picture of nothing, for the geometry.
    struct Pic;
    impl Raster for Pic {
        fn size(&self) -> (usize, usize) {
            (1000, 800)
        }
        fn levels(&self) -> usize {
            1
        }
        fn level_size(&self, _: usize) -> (usize, usize) {
            (1000, 800)
        }
        fn block_size(&self, _: usize) -> (usize, usize) {
            (1000, 800)
        }
        fn block(&self, _: usize, _: usize, _: usize) -> Option<Arc<Block>> {
            None
        }
        fn to_pixel(&self, _: f64, _: f64) -> Option<(f64, f64)> {
            None
        }
        fn bounds(&self) -> [f64; 4] {
            [0.0; 4]
        }
        fn resolution_m(&self) -> f64 {
            1.0
        }
    }

    /// The truth: pixel → position, a rotated, scaled north-up-ish picture near Nelson.
    fn truth(col: f64, row: f64) -> [f64; 2] {
        let (x0, y0) = web_mercator(173.25, -41.25);
        let t: f64 = 0.2; // radians
        let k = 3.0; // mercator metres a pixel
        let (u, v) = (col * k, -row * k);
        let (x, y) = (x0 + t.cos() * u - t.sin() * v, y0 + t.sin() * u + t.cos() * v);
        let (lon, lat) = web_mercator_inv(x, y);
        [lon, lat]
    }

    fn pt(col: f64, row: f64) -> Point {
        Point { px: Some([col, row]), ll: Some(truth(col, row)), off: false, from: "test".into() }
    }

    #[test]
    fn two_points_place_a_picture_exactly() {
        let c = Calib { points: vec![pt(100.0, 100.0), pt(900.0, 700.0)], model: "auto".into(), nudge: Nudge::default() };
        let (w, r) = fit(&Pic, true, &c);
        assert_eq!(r.model, "similarity");
        assert!(r.rms_m.unwrap() < 0.01, "{r:?}");
        let cal = Calibrated::new(Arc::new(Pic), true, w.unwrap(), None);
        let p = truth(500.0, 300.0);
        let (col, row) = cal.to_pixel(p[0], p[1]).unwrap();
        assert!((col - 500.0).abs() < 1e-3 && (row - 300.0).abs() < 1e-3, "{col} {row}");
        let ll = cal.to_lonlat(500.0, 300.0).unwrap();
        assert!(distance_m(ll, (p[0], p[1])) < 0.01);
    }

    #[test]
    fn affine_and_outliers() {
        let mut pts = vec![pt(0.0, 0.0), pt(1000.0, 0.0), pt(0.0, 800.0), pt(1000.0, 800.0)];
        let mut bad = pt(500.0, 400.0);
        bad.ll = Some(truth(530.0, 400.0)); // 90 mercator m out, about 68 m on the ground at 41°S
        pts.push(bad);
        let mut c = Calib { points: pts, model: "auto".into(), nudge: Nudge::default() };
        let (_, r) = fit(&Pic, true, &c);
        assert_eq!(r.model, "affine");
        assert!(r.residuals[4].unwrap() > 50.0, "{r:?}");
        c.points[4].off = true;
        let (_, r) = fit(&Pic, true, &c);
        assert!(r.rms_m.unwrap() < 0.01, "{r:?}");
        // left out, but its residual is still shown
        assert!(r.residuals[4].unwrap() > 60.0);
    }

    #[test]
    fn second_order_round_trips() {
        let pts: Vec<Point> =
            [(0.0, 0.0), (1000.0, 0.0), (0.0, 800.0), (1000.0, 800.0), (500.0, 0.0), (0.0, 400.0), (500.0, 400.0)].iter().map(|&(c, r)| pt(c, r)).collect();
        let c = Calib { points: pts, model: "poly2".into(), nudge: Nudge::default() };
        let (w, r) = fit(&Pic, true, &c);
        assert_eq!(r.model, "poly2");
        assert!(r.rms_m.unwrap() < 0.01, "{r:?}");
        let cal = Calibrated::new(Arc::new(Pic), true, w.unwrap(), None);
        let p = truth(250.0, 600.0);
        let (col, row) = cal.to_pixel(p[0], p[1]).unwrap();
        assert!((col - 250.0).abs() < 0.05 && (row - 600.0).abs() < 0.05, "{col} {row}");
    }

    #[test]
    fn too_few_points_say_why() {
        let c = Calib { points: vec![pt(1.0, 1.0)], model: "auto".into(), nudge: Nudge::default() };
        let (w, r) = fit(&Pic, true, &c);
        assert!(w.is_none() && r.note.contains("second"), "{r:?}");
        let c = Calib { points: vec![pt(1.0, 1.0), pt(2.0, 9.0), pt(30.0, 4.0)], model: "poly2".into(), nudge: Nudge::default() };
        let (w, r) = fit(&Pic, true, &c);
        assert!(w.is_none() && r.note.contains("6"), "{r:?}");
    }

    #[test]
    fn nudge_moves_and_turns() {
        let c = Calib { points: vec![pt(100.0, 100.0), pt(900.0, 700.0)], model: "auto".into(), nudge: Nudge { east_m: 25.0, north_m: -10.0, turn_deg: 0.0 } };
        let (w, _) = fit(&Pic, true, &c);
        let cal = Calibrated::new(Arc::new(Pic), true, w.unwrap(), None);
        let p = truth(500.0, 400.0);
        let q = cal.to_lonlat(500.0, 400.0).unwrap();
        let d = distance_m((p[0], p[1]), q);
        assert!((d - (25.0f64.powi(2) + 10.0f64.powi(2)).sqrt()).abs() < 0.2, "{d}");
        assert!(q.0 > p[0] && q.1 < p[1]);
        // and back again
        let (col, row) = cal.to_pixel(q.0, q.1).unwrap();
        assert!((col - 500.0).abs() < 1e-3 && (row - 400.0).abs() < 1e-3);
        // a turn keeps the centre where it was
        let c = Calib { nudge: Nudge { east_m: 0.0, north_m: 0.0, turn_deg: 10.0 }, ..c };
        let (w, _) = fit(&Pic, true, &c);
        let cal = Calibrated::new(Arc::new(Pic), true, w.unwrap(), None);
        let m = truth(500.0, 400.0);
        assert!(distance_m(cal.to_lonlat(500.0, 400.0).unwrap(), (m[0], m[1])) < 0.01);
        assert!(
            distance_m(cal.to_lonlat(900.0, 400.0).unwrap(), {
                let t = truth(900.0, 400.0);
                (t[0], t[1])
            }) > 100.0
        );
    }
}
