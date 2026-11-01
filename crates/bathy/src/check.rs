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
    /// After [`RefGrid::widen`]: the shallowest `lo` and deepest `hi` within that many cells.
    wide: Option<(Vec<f32>, Vec<f32>)>,
}

/// How far the charted range is widened (cells of [`REF_STEP`]: about ±900 m north–south):
/// on a slope or a canyon wall the chart's depth areas are generalised polygons, and a
/// sounding near a band's edge may be in the next band.
pub const WIDEN: usize = 4;
/// How far the deepest charted depth is looked for when judging a sounding too deep (cells:
/// about ±5 km): a canyon's head is under shallow bands, and the deep water it leads to may
/// be kilometres away.
pub const WIDEN_DEEP: usize = 23;

impl RefGrid {
    /// An empty grid over [w, s, e, n] with cells of `step` degrees.
    pub fn new(b: [f64; 4], step: f64) -> RefGrid {
        let nx = (((b[2] - b[0]) / step).ceil() as usize).max(1);
        let ny = (((b[3] - b[1]) / step).ceil() as usize).max(1);
        RefGrid { west: b[0], south: b[1], step, nx, ny, lo: vec![f32::NAN; nx * ny], hi: vec![f32::INFINITY; nx * ny], band: vec![0; nx * ny], wide: None }
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

    /// Widen the charted range (after painting): each cell takes the shallowest `lo` within
    /// `k_lo` cells each way and the deepest `hi` within `k_hi` (min and max filters, row then
    /// column).
    pub fn widen(&mut self, k_lo: usize, k_hi: usize) {
        let (nx, ny) = (self.nx, self.ny);
        let lo0: Vec<f32> = self.lo.iter().map(|&v| if v.is_nan() { f32::INFINITY } else { v }).collect();
        let hi0: Vec<f32> = (0..nx * ny).map(|i| if self.lo[i].is_nan() { f32::NEG_INFINITY } else { self.hi[i] }).collect();
        let pass = |src: &[f32], min: bool, along_x: bool, k: usize| -> Vec<f32> {
            let mut out = vec![0f32; nx * ny];
            for a in 0..(if along_x { ny } else { nx }) {
                let len = if along_x { nx } else { ny };
                let at = |b: usize| if along_x { a * nx + b } else { b * nx + a };
                for b in 0..len {
                    let (b0, b1) = (b.saturating_sub(k), (b + k + 1).min(len));
                    let mut v = if min { f32::INFINITY } else { f32::NEG_INFINITY };
                    for c in b0..b1 {
                        let x = src[at(c)];
                        v = if min { v.min(x) } else { v.max(x) };
                    }
                    out[at(b)] = v;
                }
            }
            out
        };
        let lo = pass(&pass(&lo0, true, true, k_lo), true, false, k_lo);
        let hi = pass(&pass(&hi0, false, true, k_hi), false, false, k_hi);
        self.wide = Some((lo, hi));
    }

    /// The charted range at a place, widened to the cells around it (over [`WIDEN`] cells
    /// after [`RefGrid::widen`], else ±1 cell): the shallowest `lo` and deepest `hi` near it.
    /// None where there's no chart.
    pub fn at(&self, lon: f64, lat: f64) -> Option<(f32, f32)> {
        let i = ((lon - self.west) / self.step).floor() as i64;
        let j = ((lat - self.south) / self.step).floor() as i64;
        if i < 0 || j < 0 || i >= self.nx as i64 || j >= self.ny as i64 {
            return None;
        }
        if self.lo[j as usize * self.nx + i as usize].is_nan() {
            return None;
        }
        if let Some((lo, hi)) = &self.wide {
            let k = j as usize * self.nx + i as usize;
            return Some((lo[k], hi[k]));
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
        // a margin so the widening reaches across the square's edge
        let m = (WIDEN.max(WIDEN_DEEP) + 1) as f64 * REF_STEP;
        let mut g = (self.build)([k.0 as f64 - m, k.1 as f64 - m, k.0 as f64 + 1.0 + m, k.1 as f64 + 1.0 + m]);
        g.widen(WIDEN, WIDEN_DEEP);
        let t = Arc::new(g);
        let r = t.at(lon, lat);
        self.tiles.write().unwrap_or_else(|e| e.into_inner()).insert(k, t);
        r
    }
}

/// Whether a depth (m, positive down) is well outside a charted range: more than 10 m and
/// 20 % shallower than its shallowest, or deeper than its deepest.
///
/// Too deep is judged far more loosely than too shallow: a sounder's faults read shallow
/// (fish, a scattering layer, a held depth), while a channel, a drowned river valley or a
/// canyon narrower than the chart's depth areas really is deeper than charted. Only deeper
/// than 1.8 × the band's deepest + 20 m counts (a second echo reads about twice the depth;
/// runs of those are found along the track, see [`check_segment`]).
pub fn off_chart(d: f64, range: (f32, f32)) -> bool {
    let (lo, hi) = (range.0 as f64, range.1 as f64);
    let too_shallow = lo > 0.0 && d < lo - (10.0 + 0.2 * lo);
    let too_deep = hi.is_finite() && d > 1.8 * hi + 20.0;
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
    /// Well shallower than the charted depth here.
    OffChart,
    /// Well deeper than the charted depth here.
    OffDeep,
    /// In a run of the track that is mostly off the chart (the sounder locked onto something
    /// else for a while): left out as a whole.
    Run,
    /// In a run reading about twice the depth either side of it: a second echo.
    Echo,
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
    /// In runs reading about twice the depth either side (second echoes).
    pub echoes: usize,
}

impl CheckCounts {
    pub fn add(&mut self, v: &[Verdict], runs: usize) {
        for x in v {
            match x {
                Verdict::Keep => self.kept += 1,
                Verdict::Spike => self.spikes += 1,
                Verdict::OffChart | Verdict::OffDeep => self.off_chart += 1,
                Verdict::Run => self.in_runs += 1,
                Verdict::Echo => self.echoes += 1,
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
    let d: Vec<Option<f64>> = fixes.iter().map(|f| f.2.filter(|v| v.is_finite() && *v > 0.3 && *v < 11_000.0)).collect();
    let spike = spikes(fixes, &d);
    let mut v: Vec<Verdict> = (0..fixes.len())
        .map(|i| match (fixes[i].2, d[i], spike[i]) {
            (None, _, _) => Verdict::None,
            (Some(_), None, _) | (_, _, true) => Verdict::Spike,
            _ => Verdict::Keep,
        })
        .collect();
    // -1 well shallower than the chart, 1 well deeper, 0 within it, None no chart
    let mut off = vec![None::<i8>; fixes.len()];
    let mut unchecked = 0;
    for i in 0..fixes.len() {
        if v[i] == Verdict::Keep {
            let dd = d[i].unwrap_or(0.0);
            off[i] = chart(fixes[i].0, fixes[i].1).map(|r| {
                if !off_chart(dd, r) {
                    0
                } else if dd < r.0 as f64 {
                    -1
                } else {
                    1
                }
            });
            if off[i].is_none() {
                unchecked += 1;
            }
        }
    }
    let mark = |o: Option<i8>| if o == Some(1) { Verdict::OffDeep } else { Verdict::OffChart };
    // runs between impossible jumps
    let kept: Vec<usize> = (0..fixes.len()).filter(|&i| v[i] == Verdict::Keep).collect();
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for (k, &i) in kept.iter().enumerate() {
        let new = k == 0 || {
            let p = kept[k - 1];
            jump(fixes, &d, p, i)
        };
        if new {
            runs.push(Vec::new());
        }
        runs.last_mut().unwrap().push(i);
    }
    // second echoes: a run between jumps reading about twice the depths either side of it
    let med = |ix: &[usize]| -> f64 {
        let mut v: Vec<f64> = ix.iter().map(|&i| d[i].unwrap_or(0.0)).collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    for k in 1..runs.len().saturating_sub(1) {
        let (before, after) = (&runs[k - 1], &runs[k + 1]);
        let b = med(&before[before.len().saturating_sub(5)..]);
        let a = med(&after[..after.len().min(5)]);
        let m = med(&runs[k]);
        let twice = |x: f64| x > 0.0 && (m / x - 2.0).abs() < 0.15;
        if twice(a) && twice(b) {
            for &i in &runs[k] {
                v[i] = Verdict::Echo;
            }
        }
    }
    let mut dropped = 0;
    for run in &runs {
        if run.iter().all(|&i| v[i] == Verdict::Echo) {
            continue;
        }
        let checked = run.iter().filter(|&&i| off[i].is_some()).count();
        // only runs too shallow go whole (a sounder locked onto something above the bottom)
        let bad = run.iter().filter(|&&i| off[i] == Some(-1)).count();
        if checked > 0 && bad * 2 >= checked {
            dropped += 1;
            for &i in run {
                v[i] = if off[i].is_some_and(|o| o != 0) { mark(off[i]) } else { Verdict::Run };
            }
        } else {
            for &i in run {
                if off[i].is_some_and(|o| o != 0) {
                    v[i] = mark(off[i]);
                }
            }
        }
    }
    (v, dropped, unchecked)
}

/// How much the depth (m) may change between fixes `a` and `b` on a real seabed: a 63° slope
/// over the distance run, plus 10 m and 10 % (the sounder's own scatter, heave).
fn allow(fixes: &[(f64, f64, Option<f64>)], d: &[Option<f64>], a: usize, b: usize) -> f64 {
    let dist = metres((fixes[a].0, fixes[a].1), (fixes[b].0, fixes[b].1));
    10.0 + 0.1 * d[a].unwrap_or(0.0).max(d[b].unwrap_or(0.0)) + 2.0 * dist
}

fn jump(fixes: &[(f64, f64, Option<f64>)], d: &[Option<f64>], a: usize, b: usize) -> bool {
    (d[a].unwrap_or(0.0) - d[b].unwrap_or(0.0)).abs() > allow(fixes, d, a, b)
}

/// Spikes: short excursions (up to [`SPIKE_FIXES`] fixes) that start and end with depth jumps
/// steeper than a seabed could be ([`allow`]), one each way, with the depths either side of
/// them agreeing. Crossing a narrow channel or a bank goes down and up at a believable slope
/// and stays; a fish school, the wake or a lost bottom jumps and comes back.
fn spikes(fixes: &[(f64, f64, Option<f64>)], d: &[Option<f64>]) -> Vec<bool> {
    let idx: Vec<usize> = (0..fixes.len()).filter(|&i| d[i].is_some()).collect();
    let mut out = vec![false; fixes.len()];
    let dep = |i: usize| d[i].unwrap_or(0.0);
    let mut k = 1;
    while k + 1 < idx.len() {
        let (p, i) = (idx[k - 1], idx[k]);
        if !jump(fixes, d, p, i) {
            k += 1;
            continue;
        }
        let up = dep(i) < dep(p);
        let mut found = None;
        for m in k..(k + SPIKE_FIXES).min(idx.len() - 1) {
            let (a, b) = (idx[m], idx[m + 1]);
            if jump(fixes, d, a, b) && (dep(b) < dep(a)) != up && !jump(fixes, d, p, b) {
                found = Some(m);
                break;
            }
        }
        if let Some(m) = found {
            for &j in &idx[k..=m] {
                out[j] = true;
            }
            k = m + 1;
        } else {
            k += 1;
        }
    }
    out
}

/// The longest spike (fixes).
pub const SPIKE_FIXES: usize = 5;

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
        assert!(!off_chart(1300.0, (500.0, 1000.0))); // a canyon deeper than charted
        assert!(off_chart(2000.0, (500.0, 1000.0))); // twice: a second echo
        assert!(!off_chart(3.0, (0.0, 5.0)));
    }

    #[test]
    fn deep_water_further_away_counts_for_too_deep_only() {
        let mut g = grid();
        g.widen(1, 10); // shallow ±0.01°, deep ±0.1°
                        // on the bank, 0.08° from its edge: shallowest still the bank's, deepest the plain's
        assert_eq!(g.at(170.42, -41.5), Some((200.0, 1000.0)));
        assert!(!off_chart(700.0, g.at(170.42, -41.5).unwrap())); // a canyon head
        let mut g = grid();
        g.widen(1, 1);
        assert_eq!(g.at(170.42, -41.5), Some((200.0, 500.0)));
    }

    #[test]
    fn widening_takes_in_the_next_band_near_its_edge() {
        let mut g = grid();
        g.widen(4, 4); // 0.04° here (cells of 0.01°)
        assert_eq!(g.at(170.53, -41.5), Some((200.0, 1000.0)));
        assert_eq!(g.at(170.6, -41.5), Some((500.0, 1000.0)));
        assert_eq!(g.at(170.2, -41.5), Some((200.0, 500.0)));
        assert_eq!(g.at(169.95, -41.5), None);
    }

    #[test]
    fn channels_stay_and_spikes_go() {
        let none = |_: f64, _: f64| None;
        // steaming east, a fix every ~17 m, 300 m deep; a channel 80 m deeper crossed at
        // ~20° walls; a one-fix lost bottom; and a 3-fix fish school at 120 m
        let mut f = Vec::new();
        for i in 0..80 {
            let lon = 170.0 + i as f64 * 0.0002;
            let d = match i {
                20..=31 => 300.0 + 80.0 * (1.0 - ((i as f64 - 25.5) / 6.0).abs()).max(0.0) * 1.0,
                45 => 2.0,
                60..=62 => 120.0,
                _ => 300.0 + (i % 3) as f64,
            };
            f.push((lon, -41.5, Some(d)));
        }
        let (v, _, _) = check_segment(&f, &none);
        assert!(v[20..32].iter().all(|x| *x == Verdict::Keep), "{:?}", &v[18..34]);
        assert_eq!(v[45], Verdict::Spike);
        assert!(v[60..63].iter().all(|x| *x == Verdict::Spike), "{:?}", &v[58..65]);
        assert_eq!(v.iter().filter(|x| **x == Verdict::Spike).count(), 4);
    }

    #[test]
    fn a_second_echo_run_goes() {
        let none = |_: f64, _: f64| None;
        // 120 m, then 20 fixes locked on the second echo at ~240 m, then 120 m again
        let f: Vec<_> = (0..60)
            .map(|i| (170.0 + i as f64 * 0.0002, -41.5, Some(if (20..40).contains(&i) { 241.0 + (i % 2) as f64 } else { 120.0 + (i % 3) as f64 })))
            .collect();
        let (v, _, _) = check_segment(&f, &none);
        assert!(v[20..40].iter().all(|x| *x == Verdict::Echo), "{:?}", &v[18..42]);
        assert!(v[..20].iter().chain(&v[40..]).all(|x| *x == Verdict::Keep));
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
