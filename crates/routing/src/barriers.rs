//! User barriers for the router, besides the chart raster:
//!
//! - **lines**: a fence the boat may not cross (e.g. to keep a course inside a
//!   harbour, or off a traffic separation scheme);
//! - **zones**: closed polygons the boat may not enter (exclusion zones,
//!   marine reserves, marine farms not in the charts);
//! - **discs**: a point with a radius (a library mark set as a barrier, or the
//!   clearance round a mark being rounded).
//!
//! Geometry is worked in a local flat frame (longitudes scaled by the cosine of
//! the latitude): plenty for route steps of a few miles. Longitudes are taken
//! as continuous (a barrier that crosses 180° should use longitudes beyond 180).

use nav_core::LatLon;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Barriers {
    /// Polylines, [lon, lat] points.
    pub lines: Vec<Vec<[f64; 2]>>,
    /// Polygons (closed implicitly), [lon, lat] points.
    pub zones: Vec<Vec<[f64; 2]>>,
    /// (centre [lon, lat], radius NM).
    pub discs: Vec<([f64; 2], f64)>,
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Do segments p1–p2 and q1–q2 cross (touching counts)?
fn segments_cross(p1: [f64; 2], p2: [f64; 2], q1: [f64; 2], q2: [f64; 2]) -> bool {
    let d1 = cross(q1, q2, p1);
    let d2 = cross(q1, q2, p2);
    let d3 = cross(p1, p2, q1);
    let d4 = cross(p1, p2, q2);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return true;
    }
    let on = |a: [f64; 2], b: [f64; 2], p: [f64; 2], d: f64| {
        d == 0.0 && p[0] >= a[0].min(b[0]) && p[0] <= a[0].max(b[0]) && p[1] >= a[1].min(b[1]) && p[1] <= a[1].max(b[1])
    };
    on(q1, q2, p1, d1) || on(q1, q2, p2, d2) || on(p1, p2, q1, d3) || on(p1, p2, q2, d4)
}

/// Even-odd point in polygon.
pub fn inside(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut c = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Shortest distance (NM) from `c` to the segment a–b.
fn seg_dist_nm(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let k = c[1].to_radians().cos();
    let xy = |p: [f64; 2]| ((p[0] - c[0]) * 60.0 * k, (p[1] - c[1]) * 60.0);
    let (ax, ay) = xy(a);
    let (bx, by) = xy(b);
    let (dx, dy) = (bx - ax, by - ay);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (-(ax * dx + ay * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (ax + t * dx).hypot(ay + t * dy)
}

impl Barriers {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.zones.is_empty() && self.discs.is_empty()
    }

    /// Is a position inside a zone or a disc?
    pub fn blocked(&self, p: LatLon) -> bool {
        let q = [p.lon, p.lat];
        self.zones.iter().any(|z| inside(z, q)) || self.discs.iter().any(|(c, r)| seg_dist_nm(q, q, *c) < *r)
    }

    /// May the boat sail straight from a to b ([lon, lat])?
    pub fn segment_clear(&self, a: [f64; 2], b: [f64; 2]) -> bool {
        for l in &self.lines {
            if l.windows(2).any(|w| segments_cross(a, b, w[0], w[1])) {
                return false;
            }
        }
        for z in &self.zones {
            if inside(z, b) {
                return false;
            }
            let n = z.len();
            if (0..n).any(|i| segments_cross(a, b, z[i], z[(i + 1) % n])) {
                return false;
            }
        }
        for (c, r) in &self.discs {
            // a step that starts inside may leave (e.g. a start right by a mark)
            if seg_dist_nm(a, b, *c) < *r && seg_dist_nm(a, a, *c) >= *r {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_zones_discs() {
        let b = Barriers {
            lines: vec![vec![[174.0, -36.0], [174.0, -35.0]]],
            zones: vec![vec![[175.0, -36.0], [175.2, -36.0], [175.2, -35.8], [175.0, -35.8]]],
            discs: vec![([176.0, -36.0], 0.5)],
        };
        assert!(!b.segment_clear([173.9, -35.5], [174.1, -35.5]));
        assert!(b.segment_clear([173.9, -35.5], [173.95, -35.4]));
        assert!(!b.segment_clear([174.9, -35.9], [175.1, -35.9])); // into the zone
        assert!(!b.segment_clear([174.9, -35.9], [175.3, -35.9])); // through it
        assert!(b.blocked(LatLon::new(-35.9, 175.1)));
        assert!(!b.segment_clear([175.9, -36.0], [176.1, -36.0])); // through the disc
        assert!(b.segment_clear([175.9, -35.9], [176.1, -35.9])); // 6 NM north of it
        assert!(b.blocked(LatLon::new(-36.0, 176.005)));
    }
}
