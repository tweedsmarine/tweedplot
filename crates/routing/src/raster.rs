//! Navigability raster: which small cells of sea a route may cross.
//!
//! Built from chart polygons for the area of one routing run. Depth areas are
//! painted coarse band first, so a finer (larger-scale) chart overrides a
//! coarser one where it exists. Cells shallower than the safety depth, and
//! land, are blocked; their outlines are blocked too, so a breakwater or
//! narrow spit thinner than a cell can't be slipped through.

use chart::geom::Ring;

pub const UNKNOWN: u8 = 0;
pub const FREE: u8 = 1;
pub const BLOCKED: u8 = 2;

/// Charted hazards to keep off besides land and shallow water.
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Avoid {
    /// Rocks, wrecks and obstructions that dry, are awash, or have less than the safety depth.
    pub dangers: bool,
    /// Marine farms.
    pub farms: bool,
}

impl Default for Avoid {
    fn default() -> Self {
        Avoid { dangers: true, farms: true }
    }
}

#[derive(Debug, Clone)]
pub struct Raster {
    pub w: usize,
    pub h: usize,
    /// South-west corner and cell size, degrees.
    pub lon0: f64,
    pub lat0: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub cells: Vec<u8>,
}

impl Raster {
    /// A raster over `bbox` [w, s, e, n] with cells about `cell_m` metres,
    /// coarsened if needed to stay under `max_cells`.
    pub fn new(bbox: [f64; 4], cell_m: f64, max_cells: usize) -> Self {
        let mid_lat = ((bbox[1] + bbox[3]) / 2.0).to_radians();
        let m_per_deg_lat = 111_320.0;
        let m_per_deg_lon = 111_320.0 * mid_lat.cos().max(0.05);
        let width_m = (bbox[2] - bbox[0]) * m_per_deg_lon;
        let height_m = (bbox[3] - bbox[1]) * m_per_deg_lat;
        let mut cell = cell_m.max(1.0);
        while (width_m / cell).ceil() * (height_m / cell).ceil() > max_cells as f64 {
            cell *= 1.25;
        }
        let w = ((width_m / cell).ceil() as usize).max(1);
        let h = ((height_m / cell).ceil() as usize).max(1);
        Raster {
            w,
            h,
            lon0: bbox[0],
            lat0: bbox[1],
            dlon: (bbox[2] - bbox[0]) / w as f64,
            dlat: (bbox[3] - bbox[1]) / h as f64,
            cells: vec![UNKNOWN; w * h],
        }
    }

    pub fn cell_size_m(&self) -> f64 {
        self.dlat * 111_320.0
    }

    fn xy(&self, p: &[f64; 2]) -> (f64, f64) {
        ((p[0] - self.lon0) / self.dlon, (p[1] - self.lat0) / self.dlat)
    }

    /// Cell value at a position; outside the raster counts as FREE (open sea, no chart).
    pub fn at(&self, lat: f64, lon: f64) -> u8 {
        let (x, y) = self.xy(&[lon, lat]);
        if x < 0.0 || y < 0.0 || x >= self.w as f64 || y >= self.h as f64 {
            return FREE;
        }
        self.cells[y as usize * self.w + x as usize]
    }

    /// True if any cell is blocked.
    pub fn any_blocked(&self) -> bool {
        self.cells.contains(&BLOCKED)
    }

    pub fn blocked(&self, lat: f64, lon: f64) -> bool {
        self.at(lat, lon) == BLOCKED
    }

    /// True if the straight segment a→b crosses no blocked cell.
    pub fn segment_clear(&self, a: [f64; 2], b: [f64; 2]) -> bool {
        let (x0, y0) = self.xy(&a);
        let (x1, y1) = self.xy(&b);
        let n = ((x1 - x0).abs().max((y1 - y0).abs()) * 2.0).ceil().max(1.0) as usize;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            if x < 0.0 || y < 0.0 || x >= self.w as f64 || y >= self.h as f64 {
                continue;
            }
            if self.cells[y as usize * self.w + x as usize] == BLOCKED {
                return false;
            }
        }
        true
    }

    /// Fill a polygon (exterior + holes, even-odd) with `value`, by cell centres.
    pub fn fill(&mut self, rings: &[Ring], value: u8) {
        let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
        for r in rings {
            for p in r {
                let y = (p[1] - self.lat0) / self.dlat;
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
        }
        if ymax < 0.0 || ymin >= self.h as f64 {
            return;
        }
        let j0 = (ymin - 0.5).ceil().max(0.0) as usize;
        let j1 = ((ymax - 0.5).floor() as isize).min(self.h as isize - 1);
        if j1 < j0 as isize {
            return;
        }
        let j1 = j1 as usize;
        let mut rows: Vec<Vec<f64>> = vec![Vec::new(); j1 - j0 + 1];
        for r in rings {
            for e in r.windows(2) {
                let (xa, ya) = self.xy(&e[0]);
                let (xb, yb) = self.xy(&e[1]);
                if ya == yb {
                    continue;
                }
                let (lo, hi) = if ya < yb { (ya, yb) } else { (yb, ya) };
                let ja = ((lo - 0.5).ceil().max(j0 as f64)) as usize;
                let jb_f = (hi - 0.5).ceil() - 1.0; // half-open [lo, hi)
                if jb_f < ja as f64 {
                    continue;
                }
                let jb = (jb_f as usize).min(j1);
                for j in ja..=jb {
                    let yc = j as f64 + 0.5;
                    let x = xa + (yc - ya) * (xb - xa) / (yb - ya);
                    rows[j - j0].push(x);
                }
            }
        }
        for (k, xs) in rows.iter_mut().enumerate() {
            xs.sort_by(|a, b| a.total_cmp(b));
            let row = (j0 + k) * self.w;
            for pair in xs.chunks_exact(2) {
                let i0 = (pair[0] - 0.5).ceil().max(0.0) as usize;
                let i1 = ((pair[1] - 0.5).floor() as isize).min(self.w as isize - 1);
                if i1 < i0 as isize {
                    continue;
                }
                for c in &mut self.cells[row + i0..=row + i1 as usize] {
                    *c = value;
                }
            }
        }
    }

    /// Mark every cell an outline passes through as blocked.
    pub fn block_outline(&mut self, rings: &[Ring]) {
        for r in rings {
            for e in r.windows(2) {
                let (x0, y0) = self.xy(&e[0]);
                let (x1, y1) = self.xy(&e[1]);
                let n = ((x1 - x0).abs().max((y1 - y0).abs()) * 2.0).ceil().max(1.0) as usize;
                for k in 0..=n {
                    let t = k as f64 / n as f64;
                    let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
                    if x >= 0.0 && y >= 0.0 && x < self.w as f64 && y < self.h as f64 {
                        self.cells[y as usize * self.w + x as usize] = BLOCKED;
                    }
                }
            }
        }
    }

    /// Block the cell holding a point (a rock, a wreck...).
    pub fn block_point(&mut self, p: &[f64; 2]) {
        let (x, y) = self.xy(p);
        if x >= 0.0 && y >= 0.0 && x < self.w as f64 && y < self.h as f64 {
            self.cells[y as usize * self.w + x as usize] = BLOCKED;
        }
    }

    /// Build from a LINZ chart store for `bbox`: depth areas (shallower than
    /// `safety_depth` blocked), then Topo50 land on top, then charted dangers and marine farms.
    pub fn from_chart(store: &chart::ChartStore, bbox: [f64; 4], safety_depth: f64, cell_m: f64, max_cells: usize) -> Self {
        Self::from_chart_with(store, bbox, safety_depth, cell_m, max_cells, Avoid::default())
    }

    pub fn from_chart_with(store: &chart::ChartStore, bbox: [f64; 4], safety_depth: f64, cell_m: f64, max_cells: usize, avoid: Avoid) -> Self {
        let mut r = Raster::new(bbox, cell_m, max_cells);
        let mut current_band = 0u8;
        let mut shallow: Vec<Vec<Ring>> = Vec::new();
        store.for_each_depth_area(bbox, |band, drval1, rings| {
            if band != current_band {
                for s in shallow.drain(..) {
                    r.block_outline(&s);
                }
                current_band = band;
            }
            let shallow_here = drval1.map_or(true, |d| d < safety_depth);
            r.fill(rings, if shallow_here { BLOCKED } else { FREE });
            if shallow_here {
                shallow.push(rings.to_vec());
            }
        });
        for s in shallow.drain(..) {
            r.block_outline(&s);
        }
        store.for_each_land(bbox, |rings| {
            r.fill(rings, BLOCKED);
            r.block_outline(rings);
        });
        let cell = r.cell_size_m();
        store.for_each_world_land(bbox, cell, |rings| {
            r.fill(rings, BLOCKED);
            r.block_outline(rings);
        });
        if avoid.dangers || avoid.farms {
            store.for_each_danger(bbox, safety_depth, avoid.dangers, avoid.farms, |g| match g {
                chart::geom::Geom::Points(pts) => pts.iter().for_each(|p| r.block_point(p)),
                chart::geom::Geom::Polygons(polys) => {
                    for p in polys {
                        r.fill(p, BLOCKED);
                        r.block_outline(p);
                    }
                }
                chart::geom::Geom::Lines(_) => {}
            });
        }
        r
    }

    /// Blocked cells as a GeoJSON MultiPolygon of row runs (for checking on the chart).
    pub fn blocked_geojson(&self) -> serde_json::Value {
        let mut polys = Vec::new();
        for j in 0..self.h {
            let row = &self.cells[j * self.w..(j + 1) * self.w];
            let mut i = 0;
            while i < self.w {
                if row[i] == BLOCKED {
                    let start = i;
                    while i < self.w && row[i] == BLOCKED {
                        i += 1;
                    }
                    let (w0, e0) = (self.lon0 + start as f64 * self.dlon, self.lon0 + i as f64 * self.dlon);
                    let (s0, n0) = (self.lat0 + j as f64 * self.dlat, self.lat0 + (j + 1) as f64 * self.dlat);
                    polys.push(serde_json::json!([[[w0, s0], [e0, s0], [e0, n0], [w0, n0], [w0, s0]]]));
                } else {
                    i += 1;
                }
            }
        }
        serde_json::json!({ "type": "Feature", "properties": {}, "geometry": { "type": "MultiPolygon", "coordinates": polys } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Ring {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]
    }

    #[test]
    fn fill_and_hole() {
        let mut r = Raster::new([0.0, 0.0, 0.01, 0.01], 100.0, 1_000_000);
        // big square with a hole in the middle
        r.fill(&[square(0.001, 0.001, 0.009, 0.009), square(0.004, 0.004, 0.006, 0.006)], BLOCKED);
        assert!(r.blocked(0.002, 0.002));
        assert!(!r.blocked(0.005, 0.005)); // in the hole
        assert!(!r.blocked(0.0095, 0.0095)); // outside
        assert!(!r.segment_clear([0.0, 0.005], [0.01, 0.005])); // crosses the block
        assert!(r.segment_clear([0.0, 0.0095], [0.01, 0.0095]));
    }

    #[test]
    fn thin_outline_is_blocked() {
        let mut r = Raster::new([0.0, 0.0, 0.01, 0.01], 100.0, 1_000_000);
        // a spit much thinner than a cell
        let spit = vec![vec![[0.002, 0.005], [0.008, 0.005], [0.008, 0.00501], [0.002, 0.00501], [0.002, 0.005]]];
        r.fill(&spit, BLOCKED);
        r.block_outline(&spit);
        assert!(!r.segment_clear([0.005, 0.0], [0.005, 0.01]));
    }
}
