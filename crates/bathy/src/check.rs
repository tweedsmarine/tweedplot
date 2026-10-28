//! Sanity checks on soundings: against the chart's depth areas (each a band, e.g. 500 to
//! 1000 m), and runs of a track that a sounder locked onto something other than the bottom
//! (a scattering layer, fish, a second echo) or held its last depth.
//!
//! The chart is the reference because it was surveyed independently. Its depth areas are
//! generalised, so a sounding is only "off the chart" when it is well outside the band of
//! the area it's in *and* of the areas within about 250 m (`RefGrid::at`).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Charted depth range (m, positive down) on a regular grid: `lo` NaN where no chart,
/// `hi` infinite where the band has no lower limit.
pub struct RefGrid {
    pub west: f64,
    pub south: f64,
    pub step: f64,
    pub nx: usize,
    pub ny: usize,
    lo: Vec<f32>,
    hi: Vec<f32>,
    band: Vec<u8>,
}

impl RefGrid {
    /// An empty grid over [w, s, e, n] with cells of `step` degrees.
    pub fn new(b: [f64; 4], step: f64) -> RefGrid {
        let nx = (((b[2] - b[0]) / step).ceil() as usize).max(1);
        let ny = (((b[3] - b[1]) / step).ceil() as usize).max(1);
        RefGrid { west: b[0], south: b[1], step, nx, ny, lo: vec![f32::NAN; nx * ny], hi: vec![f32::INFINITY; nx * ny], band: vec![0; nx * ny] }
    }

    /// Paint a depth area (rings in lon/lat, exterior and holes, even-odd) with its range;
    /// a finer band (higher number) replaces a coarser one.
    pub fn paint(&mut self, band: u8, lo: f64, hi: Option<f64>, rings: &[Vec<[f64; 2]>]) {
        let mut xs: Vec<f64> = Vec::new();
        let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
        for r in rings {
            for p in r {
                ymin = ymin.min(p[1]);
                ymax = ymax.max(p[1]);
            }
        }
        if ymin > ymax {
            return;
        }
        let j0 = (((ymin - self.south) / self.step).floor().max(0.0)) as usize;
        let j1 = ((((ymax - self.south) / self.step).ceil()) as usize).min(self.ny);
        for j in j0..j1 {
            let y = self.south + (j as f64 + 0.5) * self.step;
            xs.clear();
            for r in rings {
                let n = r.len();
                for k in 0..n {
                    let (a, b) = (r[k], r[(k + 1) % n]);
                    if (a[1] > y) != (b[1] > y) {
                        xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.chunks_exact(2) {
                let i0 = ((pair[0] - self.west) / self.step - 0.5).ceil().max(0.0) as usize;
                let i1 = (((pair[1] - self.west) / self.step - 0.5).floor() + 1.0).max(0.0) as usize;
                for i in i0..i1.min(self.nx) {
                    let k = j * self.nx + i;
                    if band >= self.band[k] {
                        self.band[k] = band;
                        self.lo[k] = lo as f32;
                        self.hi[k] = hi.map_or(f32::INFINITY, |h| h as f32);
                    }
                }
            }
        }
    }

    /// The charted range at a place, widened to the cells around it (about ±1 cell): the
    /// shallowest `lo` and deepest `hi` near it. None where there's no chart.
    pub fn at(&self, lon: f64, lat: f64) -> Option<(f32, f32)> {
        let i = ((lon - self.west) / self.step).floor() as i64;
        let j = ((lat - self.south) / self.step).floor() as i64;
        if i < 0 || j < 0 || i >= self.nx as i64 || j >= self.ny as i64 {
            return None;
        }
        if self.lo[j as usize * self.nx + i as usize].is_nan() {
            return None;
        }
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for dj in -1..=1 {
            for di in -1..=1 {
                let (ii, jj) = (i + di, j + dj);
                if ii < 0 || jj < 0 || ii >= self.nx as i64 || jj >= self.ny as i64 {
                    continue;
                }
                let k = jj as usize * self.nx + ii as usize;
                if !self.lo[k].is_nan() {
                    lo = lo.min(self.lo[k]);
                    hi = hi.max(self.hi[k]);
                }
            }
        }
        Some((lo, hi))
    }
}

/// The chart's depth ranges, built a 1° square at a time when first asked for and kept.
pub struct Reference {
    build: Box<dyn Fn([f64; 4]) -> RefGrid + Send + Sync>,
    tiles: RwLock<HashMap<(i32, i32), Arc<RefGrid>>>,
    /// One square built at a time (threads asking for the same one wait for it).
    building: std::sync::Mutex<()>,
}

/// Reference cells: 0.002° (about 220 m north–south).
pub const REF_STEP: f64 = 0.002;

impl Reference {
    /// `build(bbox)` paints the chart's depth areas over a grid made with [`RefGrid::new`].
    pub fn new(build: impl Fn([f64; 4]) -> RefGrid + Send + Sync + 'static) -> Reference {
        Reference { build: Box::new(build), tiles: RwLock::new(HashMap::new()), building: std::sync::Mutex::new(()) }
    }

    pub fn at(&self, lon: f64, lat: f64) -> Option<(f32, f32)> {
        let k = (lon.floor() as i32, lat.floor() as i32);
        if let Some(t) = self.tiles.read().unwrap_or_else(|e| e.into_inner()).get(&k) {
            return t.at(lon, lat);
        }
        let _one = self.building.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = self.tiles.read().unwrap_or_else(|e| e.into_inner()).get(&k) {
            return t.at(lon, lat);
        }
        // a margin of two cells so the widening reaches across the square's edge
        let m = 2.0 * REF_STEP;
        let t = Arc::new((self.build)([k.0 as f64 - m, k.1 as f64 - m, k.0 as f64 + 1.0 + m, k.1 as f64 + 1.0 + m]));
        let r = t.at(lon, lat);
        self.tiles.write().unwrap_or_else(|e| e.into_inner()).insert(k, t);
        r
    }
}

/// Whether a depth (m, positive down) is well outside a charted range: more than 10 m and
/// 20 % shallower than its shallowest, or deeper than its deepest.
pub fn off_chart(d: f64, range: (f32, f32)) -> bool {
    let (lo, hi) = (range.0 as f64, range.1 as f64);
    let too_shallow = lo > 0.0 && d < lo - (10.0 + 0.2 * lo);
    let too_deep = hi.is_finite() && d > hi + 10.0 + 0.2 * hi;
    too_shallow || too_deep
}

/// What became of a fix's depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// No depth logged.
    None,
    Keep,
    /// A spike: far off the fixes around it.
    Spike,
    /// Well outside the charted depth here.
    OffChart,
    /// In a run of the track that is mostly off the chart (the sounder locked onto something
    /// else for a while): left out as a whole.
    Run,
}

/// Counts of each [`Verdict`] (and the fixes with no chart to check against).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CheckCounts {
    pub kept: usize,
    pub spikes: usize,
    pub off_chart: usize,
    pub in_runs: usize,
    pub runs: usize,
    pub unchecked: usize,
}

impl CheckCounts {
    pub fn add(&mut self, v: &[Verdict], runs: usize) {
        for x in v {
            match x {
                Verdict::Keep => self.kept += 1,
                Verdict::Spike => self.spikes += 1,
                Verdict::OffChart => self.off_chart += 1,
                Verdict::Run => self.in_runs += 1,
                Verdict::None => {}
            }
        }
        self.runs += runs;
    }
}

fn metres(a: (f64, f64), b: (f64, f64)) -> f64 {
    let k = ((a.1 + b.1) / 2.0).to_radians().cos();
    let (dx, dy) = ((b.0 - a.0) * k * 111_320.0, (b.1 - a.1) * 111_320.0);
    (dx * dx + dy * dy).sqrt()
}

/// Check a track segment: fixes (lon, lat, depth m positive down). `chart(lon, lat)` gives
/// the charted range there, if any. Returns each fix's verdict, the number of runs left
/// out, and how many depths had no chart to check against.
///
/// Spikes go first ([`crate::clean_depths`]). The rest are split into runs wherever the
/// depth jumps more than the seabed could between the fixes (5 m and 20 %, plus the
/// distance run: a 45° slope). A run where at least half the depths checked are off the
/// chart is left out whole, the ones that happened to fit included; elsewhere only the
/// depths off the chart are.
pub fn check_segment(fixes: &[(f64, f64, Option<f64>)], chart: &dyn Fn(f64, f64) -> Option<(f32, f32)>) -> (Vec<Verdict>, usize, usize) {
    let d: Vec<Option<f64>> = fixes.iter().map(|f| f.2).collect();
    let keep = crate::clean_depths(&d);
    let mut v: Vec<Verdict> = (0..fixes.len())
        .map(|i| match (d[i], keep[i]) {
            (None, _) => Verdict::None,
            (Some(_), false) => Verdict::Spike,
            _ => Verdict::Keep,
        })
        .collect();
    let mut off = vec![None::<bool>; fixes.len()];
    let mut unchecked = 0;
    for i in 0..fixes.len() {
        if v[i] == Verdict::Keep {
            off[i] = chart(fixes[i].0, fixes[i].1).map(|r| off_chart(d[i].unwrap_or(0.0), r));
            if off[i].is_none() {
                unchecked += 1;
            }
        }
    }
    // runs between impossible jumps
    let kept: Vec<usize> = (0..fixes.len()).filter(|&i| v[i] == Verdict::Keep).collect();
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for (k, &i) in kept.iter().enumerate() {
        let new = k == 0 || {
            let p = kept[k - 1];
            let (a, b) = (d[p].unwrap_or(0.0), d[i].unwrap_or(0.0));
            let dist = metres((fixes[p].0, fixes[p].1), (fixes[i].0, fixes[i].1));
            (a - b).abs() > 5.0 + 0.2 * a.max(b) + dist
        };
        if new {
            runs.push(Vec::new());
        }
        runs.last_mut().unwrap().push(i);
    }
    let mut dropped = 0;
    for run in &runs {
        let checked = run.iter().filter(|&&i| off[i].is_some()).count();
        let bad = run.iter().filter(|&&i| off[i] == Some(true)).count();
        if checked > 0 && bad * 2 >= checked {
            dropped += 1;
            for &i in run {
                v[i] = if off[i] == Some(true) { Verdict::OffChart } else { Verdict::Run };
            }
        } else {
            for &i in run {
                if off[i] == Some(true) {
                    v[i] = Verdict::OffChart;
                }
            }
        }
    }
    (v, dropped, unchecked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> RefGrid {
        // a chart: 500–1000 m everywhere in 170–171° E, 42–41° S, with a 200–500 m bank in the west half
        let mut g = RefGrid::new([169.9, -42.1, 171.1, -40.9], 0.01);
        let sq = |w: f64, s: f64, e: f64, n: f64| vec![vec![[w, s], [e, s], [e, n], [w, n]]];
        g.paint(1, 500.0, Some(1000.0), &sq(170.0, -42.0, 171.0, -41.0));
        g.paint(2, 200.0, Some(500.0), &sq(170.0, -42.0, 170.5, -41.0));
        g
    }

    #[test]
    fn paints_and_widens() {
        let g = grid();
        assert_eq!(g.at(170.8, -41.5), Some((500.0, 1000.0)));
        assert_eq!(g.at(170.2, -41.5), Some((200.0, 500.0)));
        // on the bank's edge: both bands
        assert_eq!(g.at(170.505, -41.5), Some((200.0, 1000.0)));
        assert_eq!(g.at(169.95, -41.5), None);
        assert!(off_chart(70.0, (500.0, 1000.0)));
        assert!(!off_chart(420.0, (500.0, 1000.0))); // within 20 % + 10 m
        assert!(off_chart(1300.0, (500.0, 1000.0)));
        assert!(!off_chart(3.0, (0.0, 5.0)));
    }

    #[test]
    fn a_locked_run_goes_whole() {
        let g = grid();
        let chart = |lon: f64, lat: f64| g.at(lon, lat);
        // steaming east at 800 m, then 40 fixes locked at ~70 m, then 800 m again
        let mut f = Vec::new();
        for i in 0..120 {
            let lon = 170.6 + i as f64 * 0.001;
            let d = if (40..80).contains(&i) { 70.0 + (i % 3) as f64 } else { 800.0 + (i % 5) as f64 };
            f.push((lon, -41.5, Some(d)));
        }
        let (v, runs, unchecked) = check_segment(&f, &chart);
        assert_eq!(unchecked, 0);
        assert_eq!(runs, 1);
        assert!(v[..40].iter().all(|x| *x == Verdict::Keep));
        assert!(v[40..80].iter().all(|x| *x == Verdict::OffChart));
        assert!(v[80..].iter().all(|x| *x == Verdict::Keep));
        // off the chart: kept (nothing to check against)
        let f2: Vec<_> = (0..10).map(|i| (169.92 + i as f64 * 0.0001, -41.5, Some(70.0))).collect();
        let (v, runs, unchecked) = check_segment(&f2, &chart);
        assert_eq!((runs, unchecked), (0, 10));
        assert!(v.iter().all(|x| *x == Verdict::Keep));
    }
}
