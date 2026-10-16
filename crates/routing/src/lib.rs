//! Isochrone weather routing.
//!
//! The method is the modified isochrone method (Hagiwara, 1989), as used by
//! most routing software:
//!
//! 1. From every point on the current isochrone, sail for one time step on a
//!    fan of headings. Boat speed comes from the polar at the true wind angle
//!    and speed found in the GRIB at that place and time, times an efficiency
//!    factor; current, if wanted, is added as a vector.
//! 2. A change of tack or gybe costs time (the boat sails for less of the step).
//! 3. Any step that crosses land or water shallower than the safety depth is
//!    dropped (see [`raster::Raster`]).
//! 4. The new points are sorted into narrow sectors by bearing from the start,
//!    and only the point farthest from the start survives in each sector. That
//!    is the next isochrone.
//! 5. When a step reaches the mark's arrival circle, the arrival time is
//!    interpolated within the step and the route is traced back.
//!
//! Marks are routed leg by leg: each leg starts where and when the last one
//! arrived. Rounding sides are not handled yet (roadmap phase 3).
//!
//! Wind outside the GRIB's time span or area stops a branch; if every branch
//! stops, the result says so and gives the closest approach, rather than
//! inventing weather.

pub mod raster;

use nav_core::{norm180, norm360, LatLon};
use polar::Polar;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub use raster::{Avoid, Raster};

const KN_PER_MS: f64 = 3600.0 / 1852.0;

/// Where the weather comes from.
pub trait WindSource: Sync {
    /// True wind speed (kn) and direction it blows from (deg true).
    fn wind(&self, lat: f64, lon: f64, t: i64) -> Option<(f64, f64)>;
    /// Surface current as (east, north) in knots, if known.
    fn current(&self, _lat: f64, _lon: f64, _t: i64) -> Option<(f64, f64)> {
        None
    }
    /// Last time with wind data, if the source ends.
    fn end_time(&self) -> Option<i64> {
        None
    }
}

impl WindSource for grib::GribFile {
    fn wind(&self, lat: f64, lon: f64, t: i64) -> Option<(f64, f64)> {
        grib::GribFile::wind(self, lat, lon, t).map(|w| (w.tws, w.twd))
    }
    fn current(&self, lat: f64, lon: f64, t: i64) -> Option<(f64, f64)> {
        grib::GribFile::current(self, lat, lon, t).map(|(u, v)| (u * KN_PER_MS, v * KN_PER_MS))
    }
    fn end_time(&self) -> Option<i64> {
        self.times(grib::Param::WindU).last().copied()
    }
}

/// Steady wind everywhere, for tests and quick checks.
pub struct SteadyWind {
    pub tws: f64,
    pub twd: f64,
}

impl WindSource for SteadyWind {
    fn wind(&self, _: f64, _: f64, _: i64) -> Option<(f64, f64)> {
        Some((self.tws, self.twd))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// Time step, seconds (0 = automatic per leg, from its length and the boat's speed).
    pub step_s: f64,
    /// Heading fan resolution, degrees.
    pub heading_step_deg: f64,
    /// Width of the pruning sectors, degrees as seen from the start.
    pub sector_deg: f64,
    /// Polar multiplier (1.0 = 100 %).
    pub efficiency: f64,
    /// Time lost per tack and per gybe, seconds.
    pub tack_s: f64,
    pub gybe_s: f64,
    /// Arrival circle round each mark, NM.
    pub arrival_nm: f64,
    /// Add GRIB currents to the boat's velocity.
    pub use_current: bool,
    /// Give up after this many steps per leg.
    pub max_steps: usize,
    /// Keep every n-th isochrone for display (0 = none).
    pub keep_isochrones_every: usize,
    /// "sectors" (classic, open water), "grid" (works round land), or "auto"
    /// (grid when there are obstacles).
    pub pruning: String,
    /// Grid cell for "grid" pruning, NM (0 = automatic from boat speed and step).
    pub cell_nm: f64,
    /// Drop points whose start→point→mark distance exceeds this multiple of the direct distance.
    pub max_detour: f64,
    /// Isochrones returned for display (thinned evenly; 0 = all).
    pub max_isochrones: usize,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            step_s: 0.0,
            heading_step_deg: 4.0,
            sector_deg: 1.0,
            efficiency: 1.0,
            tack_s: 60.0,
            gybe_s: 60.0,
            arrival_nm: 0.1,
            use_current: true,
            max_steps: 2000,
            keep_isochrones_every: 1,
            pruning: "auto".into(),
            cell_nm: 0.0,
            max_detour: 1.6,
            max_isochrones: 60,
        }
    }
}

/// One point of the routed track.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RoutePoint {
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
    /// Heading sailed FROM this point (deg true).
    pub heading: f64,
    /// Signed TWA on that heading: positive = starboard tack (wind over the starboard side).
    pub twa: f64,
    pub tws: f64,
    pub twd: f64,
    /// Boat speed through the water and speed over ground, knots.
    pub bsp: f64,
    pub sog: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Leg {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub start: i64,
    pub arrival: Option<i64>,
    pub distance_nm: f64,
    pub tacks: usize,
    pub gybes: usize,
    /// Time step used for this leg, seconds.
    pub step_s: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteResult {
    /// "arrived", "grib end", "no wind data", "blocked", "too many steps", "start blocked", "mark blocked".
    /// Unless the start or a mark is blocked, `points` holds the route as far as it got:
    /// for "grib end" that is the point nearest the mark when the weather runs out.
    pub status: String,
    pub message: String,
    pub points: Vec<RoutePoint>,
    pub legs: Vec<Leg>,
    pub arrival: Option<i64>,
    pub distance_nm: f64,
    /// Kept isochrones for display. Each isochrone is a set of [lon, lat]
    /// polylines: the outer edge of the front, broken where land or a gap
    /// interrupts it.
    pub isochrones: Vec<Vec<Vec<[f64; 2]>>>,
    pub steps: usize,
    pub compute_ms: u128,
}

#[derive(Clone, Copy)]
struct Node {
    lat: f64,
    lon: f64,
    t: f64,
    parent: usize,
    heading: f64,
    twa: f64,
    tws: f64,
    twd: f64,
    bsp: f64,
    sog: f64,
}

/// Route from `start` at time `t0` through `marks` in order.
pub fn route(
    start: LatLon,
    t0: i64,
    marks: &[LatLon],
    polar: &Polar,
    wind: &dyn WindSource,
    obstacles: Option<&Raster>,
    p: &Params,
) -> RouteResult {
    let clock = std::time::Instant::now();
    // A raster with nothing blocked (open ocean, no chart there) only slows things down.
    let obstacles = obstacles.filter(|r| r.any_blocked());
    let mut res = RouteResult {
        status: "arrived".into(),
        message: String::new(),
        points: Vec::new(),
        legs: Vec::new(),
        arrival: None,
        distance_nm: 0.0,
        isochrones: Vec::new(),
        steps: 0,
        compute_ms: 0,
    };
    if let Some(r) = obstacles {
        if r.blocked(start.lat, start.lon) {
            res.status = "start blocked".into();
            res.message = "The start is on land or in water shallower than the safety depth. Move it into open water.".into();
            return res;
        }
    }
    if let Some(r) = obstacles {
        if let Some(i) = marks.iter().position(|m| r.blocked(m.lat, m.lon)) {
            res.status = "mark blocked".into();
            res.message = format!(
                "Mark {} is on land or in water shallower than the safety depth. Marks must be in navigable water: move it off the rock or into the channel.",
                i + 1
            );
            res.compute_ms = clock.elapsed().as_millis();
            return res;
        }
    }
    let mut from = start;
    let mut t = t0 as f64;
    for (i, mark) in marks.iter().enumerate() {
        let leg = route_leg(from, t, *mark, polar, wind, obstacles, p, &mut res);
        let n = res.legs.len();
        res.legs.push(leg.leg);
        // drop the duplicate join point between legs
        if n > 0 && !res.points.is_empty() {
            res.points.pop();
        }
        res.points.extend(leg.points);
        if leg.arrived.is_none() {
            res.status = leg.status;
            res.message = if marks.len() > 1 { format!("Leg {} of {}: {}", i + 1, marks.len(), leg.message) } else { leg.message };
            break;
        }
        let (lat, lon, ta) = leg.arrived.unwrap();
        from = LatLon::new(lat, lon);
        t = ta;
        res.arrival = Some(ta.round() as i64);
    }
    if res.status != "arrived" {
        res.arrival = None;
    }
    res.distance_nm = res.legs.iter().map(|l| l.distance_nm).sum();
    // Thin the isochrones for display: at most `max_isochrones` of them.
    if p.max_isochrones > 0 && res.isochrones.len() > p.max_isochrones {
        let k = res.isochrones.len().div_ceil(p.max_isochrones);
        res.isochrones = res.isochrones.iter().step_by(k).cloned().collect();
    }

    res.compute_ms = clock.elapsed().as_millis();
    res
}

struct LegOut {
    leg: Leg,
    points: Vec<RoutePoint>,
    arrived: Option<(f64, f64, f64)>,
    status: String,
    message: String,
}

#[allow(clippy::too_many_arguments)]
fn route_leg(
    start: LatLon,
    t0: f64,
    dest: LatLon,
    polar: &Polar,
    wind: &dyn WindSource,
    obstacles: Option<&Raster>,
    p: &Params,
    res: &mut RouteResult,
) -> LegOut {
    let vmax = polar.speeds.iter().flatten().fold(0.0f64, |a, &b| a.max(b)) * p.efficiency;
    let direct = start.distance_to(&dest);
    let step_s = if p.step_s > 0.0 { p.step_s } else { auto_step_s(direct, vmax, obstacles.is_some()) };
    let dt_h = step_s / 3600.0;
    let mut nodes: Vec<Node> = vec![Node { lat: start.lat, lon: start.lon, t: t0, parent: usize::MAX, heading: f64::NAN, twa: 0.0, tws: 0.0, twd: 0.0, bsp: 0.0, sog: 0.0 }];
    let mut front: Vec<usize> = vec![0];
    let n_sectors = (360.0 / p.sector_deg).ceil() as usize;
    // Pruning mode and grid size
    let grid_mode = match p.pruning.as_str() {
        "grid" => true,
        "sectors" => false,
        _ => obstacles.is_some(),
    };
    let cell_nm = if p.cell_nm > 0.0 { p.cell_nm } else { (vmax * dt_h * 0.5).max(0.02) };
    let cell_deg = cell_nm / 60.0;
    let kx = start.lat.to_radians().cos().max(0.05);
    let detour_limit = direct * p.max_detour + 2.0 * cell_nm + 1.0;
    let mut visited: std::collections::HashSet<(i32, i32), CellBuild> = std::collections::HashSet::default();
    visited.insert(cell_key(start.lat, start.lon, cell_deg, kx));
    // Step at which each grid cell was first reached: contoured for the isochrone display.
    let mut reached: std::collections::HashMap<(i32, i32), u32, CellBuild> = std::collections::HashMap::default();
    reached.insert(cell_key(start.lat, start.lon, cell_deg, kx), 0);
    let mut leg_steps: u32 = 0;
    let headings: Vec<f64> = {
        let n = (360.0 / p.heading_step_deg).round() as usize;
        (0..n).map(|k| k as f64 * 360.0 / n as f64).collect()
    };
    let mut no_wind = false;
    let mut best_approach = (start.distance_to(&dest), 0usize);
    let mut status = String::from("too many steps");
    let mut message = format!("No arrival within {} steps.", p.max_steps);

    for step in 0..p.max_steps {
        res.steps += 1;
        // Expand every point of the front in parallel.
        let cands: Vec<(Node, Option<f64>)> = front
            .par_iter()
            .flat_map_iter(|&ni| {
                let n = nodes[ni];
                let t = n.t;
                let w = wind.wind(n.lat, n.lon, t.round() as i64);
                let cur = if p.use_current { wind.current(n.lat, n.lon, t.round() as i64) } else { None };
                let brg_dest = LatLon::new(n.lat, n.lon).bearing_to(&dest);
                let here = LatLon::new(n.lat, n.lon);
                let mut out: Vec<(Node, Option<f64>)> = Vec::new();
                let Some((tws, twd)) = w else { return out.into_iter() };
                let mut hs: Vec<f64> = headings.clone();
                hs.push(brg_dest); // always try straight at the mark
                for h in hs {
                    let twa = norm180(twd - h); // + = wind from starboard
                    let bsp = polar.bsp(twa.abs(), tws) * p.efficiency;
                    if bsp < 0.05 {
                        continue;
                    }
                    let (mut ve, mut vn) = (bsp * h.to_radians().sin(), bsp * h.to_radians().cos());
                    if let Some((ce, cn)) = cur {
                        ve += ce;
                        vn += cn;
                    }
                    let sog = (ve * ve + vn * vn).sqrt();
                    let cog = norm360(ve.atan2(vn).to_degrees());
                    // manoeuvre penalty
                    let mut hours = dt_h;
                    if !n.heading.is_nan() && n.twa != 0.0 && twa.signum() != n.twa.signum() {
                        let pen = if twa.abs() < 90.0 && n.twa.abs() < 90.0 { p.tack_s } else { p.gybe_s };
                        hours = (dt_h - pen / 3600.0).max(0.0);
                    }
                    let dist = sog * hours;
                    let np = here.destination(cog, dist);
                    if grid_mode {
                        // cheap rejections before the obstacle check
                        let key = cell_key(np.lat, np.lon, cell_deg, kx);
                        if visited.contains(&key) && key != cell_key(n.lat, n.lon, cell_deg, kx) {
                            continue;
                        }
                        if start.distance_to(&np) + np.distance_to(&dest) > detour_limit {
                            continue;
                        }
                    }
                    if let Some(r) = obstacles {
                        if !r.segment_clear([n.lon, n.lat], [np.lon, np.lat]) {
                            continue;
                        }
                    }
                    // Arrival: does this step reach the arrival circle?
                    let arrive = arrival_fraction(here, np, dest, p.arrival_nm).map(|f| {
                        // time spent: penalty first, then sailing
                        let pen_h = dt_h - hours;
                        t + (pen_h + f * hours) * 3600.0
                    });
                    out.push((
                        Node { lat: np.lat, lon: np.lon, t: t + step_s, parent: ni, heading: h, twa, tws, twd, bsp, sog },
                        arrive,
                    ));
                }
                out.into_iter()
            })
            .collect();

        if cands.is_empty() {
            let any_wind = front.iter().any(|&ni| wind.wind(nodes[ni].lat, nodes[ni].lon, nodes[ni].t.round() as i64).is_some());
            if !any_wind {
                no_wind = true;
                let tf = nodes[front[0]].t;
                if let Some(end) = wind.end_time().filter(|&e| tf + step_s > e as f64) {
                    // Out of weather: finish at the point of the last isochrone nearest the mark.
                    let (dd, ni) = front
                        .iter()
                        .map(|&ni| (LatLon::new(nodes[ni].lat, nodes[ni].lon).distance_to(&dest), ni))
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .unwrap();
                    best_approach = (dd, ni);
                    status = "grib end".into();
                    message = format!("The GRIB ends at {}; the route stops there,", grib::fmt_run(end));
                } else {
                    status = "no wind data".into();
                    message = format!("The GRIB has no wind for the route at {} (outside its area).", grib::fmt_run(tf as i64));
                }
            } else {
                status = "blocked".into();
                message = "Every heading is blocked by land or shallow water.".into();
            }
            break;
        }

        // Earliest arrival this step finishes the leg.
        if let Some((node, ta)) = cands.iter().filter_map(|(n, a)| a.map(|ta| (n, ta))).min_by(|a, b| a.1.total_cmp(&b.1)) {
            let parent = node.parent;
            let arrive_pt = interpolate_arrival(&nodes[parent], node, dest, p.arrival_nm);
            let mut chain = trace(&nodes, parent);
            let mut last = *node;
            last.lat = arrive_pt.lat;
            last.lon = arrive_pt.lon;
            last.t = ta;
            chain.push(last);
            if p.keep_isochrones_every > 0 {
                if grid_mode {
                    res.isochrones.extend(contour_isochrones(&reached, cell_deg, kx, obstacles, leg_steps, step_s));
                } else {
                    res.isochrones.push(front_lines(&nodes, &front, start, obstacles));
                }
            }
            let mut out = finish(start, dest, t0, chain, Some((arrive_pt.lat, arrive_pt.lon, ta)), "arrived".into(), String::new());
            out.leg.step_s = step_s;
            return out;
        }

        // Prune.
        let survivors: Vec<Node> = if grid_mode {
            // Spatial grid: one point per cell per step, the one nearest the mark;
            // a cell reached in an earlier step was reached sooner, so later
            // arrivals there are dropped (unless the boat hasn't left its own cell).
            let mut best: std::collections::HashMap<(i32, i32), (f64, Node), CellBuild> = std::collections::HashMap::default();
            for (n, _) in cands {
                let key = cell_key(n.lat, n.lon, cell_deg, kx);
                let parent = nodes[n.parent];
                if visited.contains(&key) && key != cell_key(parent.lat, parent.lon, cell_deg, kx) {
                    continue;
                }
                let ll = LatLon::new(n.lat, n.lon);
                if start.distance_to(&ll) + ll.distance_to(&dest) > detour_limit {
                    continue;
                }
                let dd = ll.distance_to(&dest);
                if best.get(&key).is_none_or(|(bd, _)| dd < *bd) {
                    best.insert(key, (dd, n));
                }
            }
            for k in best.keys() {
                visited.insert(*k);
                reached.entry(*k).or_insert(step as u32 + 1);
            }
            best.into_values().map(|(_, n)| n).collect()
        } else {
            // Sectors from the start: keep the farthest point in each (Hagiwara).
            let mut best: Vec<Option<(f64, Node)>> = vec![None; n_sectors];
            for (n, _) in cands {
                let ll = LatLon::new(n.lat, n.lon);
                let d = start.distance_to(&ll);
                let s = ((start.bearing_to(&ll) / p.sector_deg) as usize).min(n_sectors - 1);
                if best[s].as_ref().is_none_or(|(bd, _)| d > *bd) {
                    best[s] = Some((d, n));
                }
            }
            best.into_iter().flatten().map(|(_, n)| n).collect()
        };
        front.clear();
        for n in survivors {
            let dd = LatLon::new(n.lat, n.lon).distance_to(&dest);
            nodes.push(n);
            if dd < best_approach.0 {
                best_approach = (dd, nodes.len() - 1);
            }
            front.push(nodes.len() - 1);
        }
        leg_steps = step as u32 + 1;
        if !grid_mode && p.keep_isochrones_every > 0 && step % p.keep_isochrones_every == 0 {
            res.isochrones.push(front_lines(&nodes, &front, start, obstacles));
        }
    }

    let _ = no_wind;
    if grid_mode && p.keep_isochrones_every > 0 {
        res.isochrones.extend(contour_isochrones(&reached, cell_deg, kx, obstacles, leg_steps, step_s));
    }
    let mut chain = trace(&nodes, best_approach.1);
    let grib_end = status == "grib end";
    if let (true, Some(end)) = (grib_end, wind.end_time()) {
        // The last step ran past the end of the weather: cut it back to the end time.
        let n = chain.len();
        if n >= 2 && chain[n - 1].t > end as f64 {
            let (a, b) = (chain[n - 2], chain[n - 1]);
            let f = ((end as f64 - a.t) / (b.t - a.t)).clamp(0.0, 1.0);
            let pa = LatLon::new(a.lat, a.lon);
            let pb = pa.destination(pa.bearing_to(&LatLon::new(b.lat, b.lon)), pa.distance_to(&LatLon::new(b.lat, b.lon)) * f);
            chain[n - 1].lat = pb.lat;
            chain[n - 1].lon = pb.lon;
            chain[n - 1].t = end as f64;
            best_approach.0 = pb.distance_to(&dest);
        }
    }
    let mut out = finish(start, dest, t0, chain, None, status, message);
    out.leg.step_s = step_s;
    if grib_end {
        out.message.push_str(&format!(" {:.1} NM from the mark.", best_approach.0));
    } else {
        out.message.push_str(&format!(" Closest approach {:.1} NM.", best_approach.0));
    }
    out
}

fn finish(start: LatLon, dest: LatLon, t0: f64, chain: Vec<Node>, arrived: Option<(f64, f64, f64)>, status: String, message: String) -> LegOut {
    let mut tacks = 0;
    let mut gybes = 0;
    let mut dist = 0.0;
    let mut points = Vec::with_capacity(chain.len());
    for (i, n) in chain.iter().enumerate() {
        // node i carries the heading sailed INTO it; report the heading sailed FROM each point
        let next = chain.get(i + 1);
        let (heading, twa, tws, twd, bsp, sog) = match next {
            Some(nx) => (nx.heading, nx.twa, nx.tws, nx.twd, nx.bsp, nx.sog),
            None => (n.heading, n.twa, n.tws, n.twd, n.bsp, n.sog),
        };
        if i > 0 {
            dist += LatLon::new(chain[i - 1].lat, chain[i - 1].lon).distance_to(&LatLon::new(n.lat, n.lon));
            if i > 1 && n.twa.signum() != chain[i - 1].twa.signum() && chain[i - 1].twa != 0.0 {
                if n.twa.abs() < 90.0 { tacks += 1 } else { gybes += 1 }
            }
        }
        points.push(RoutePoint { t: n.t.round() as i64, lat: n.lat, lon: n.lon, heading: norm360(heading), twa, tws, twd, bsp, sog });
    }
    LegOut {
        leg: Leg {
            from: [start.lon, start.lat],
            to: [dest.lon, dest.lat],
            start: t0.round() as i64,
            arrival: arrived.map(|a| a.2.round() as i64),
            distance_nm: dist,
            tacks,
            gybes,
            step_s: 0.0,
        },
        points,
        arrived,
        status,
        message,
    }
}

/// Automatic time step for a leg: about 80 steps at a typical speed, rounded
/// to a tidy value between 5 minutes and 3 hours (1 hour with land about).
fn auto_step_s(direct_nm: f64, vmax_kn: f64, obstacles: bool) -> f64 {
    let typical = (vmax_kn * 0.6).max(1.0);
    let minutes = direct_nm / typical * 60.0 / 80.0;
    let tidy = [5.0, 10.0, 15.0, 20.0, 30.0, 45.0, 60.0, 90.0, 120.0, 180.0];
    let cap = if obstacles { 60.0 } else { 180.0 };
    let m = tidy.iter().copied().filter(|&v| v <= cap).min_by(|a, b| (a - minutes).abs().total_cmp(&(b - minutes).abs())).unwrap();
    m * 60.0
}

fn trace(nodes: &[Node], mut i: usize) -> Vec<Node> {
    let mut v = Vec::new();
    while i != usize::MAX {
        v.push(nodes[i]);
        i = nodes[i].parent;
    }
    v.reverse();
    v
}

/// Outer edge of the front as polylines: the farthest point from the start in
/// each half-degree sector, joined in bearing order, broken where neighbours
/// are far apart or the join would cross an obstacle.
fn front_lines(nodes: &[Node], front: &[usize], start: LatLon, obstacles: Option<&Raster>) -> Vec<Vec<[f64; 2]>> {
    const SECTOR: f64 = 0.5;
    let n = (360.0 / SECTOR) as usize;
    let mut best: Vec<Option<(f64, [f64; 2])>> = vec![None; n];
    for &i in front {
        let ll = LatLon::new(nodes[i].lat, nodes[i].lon);
        let d = start.distance_to(&ll);
        let s = ((start.bearing_to(&ll) / SECTOR) as usize).min(n - 1);
        if best[s].is_none_or(|(bd, _)| d > bd) {
            best[s] = Some((d, [ll.lon, ll.lat]));
        }
    }
    let pts: Vec<(f64, [f64; 2])> = best.into_iter().flatten().collect();
    let mut lines: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    let mut prev: Option<(f64, [f64; 2])> = None;
    for p in pts.iter().copied() {
        if let Some((pd, pp)) = prev {
            let a = LatLon::new(pp[1], pp[0]);
            let b = LatLon::new(p.1[1], p.1[0]);
            let gap = a.distance_to(&b);
            // neighbouring sectors at radius r are about r * 1.5 sector apart; allow 4x that
            let allowed = (pd.max(p.0) * (4.0 * SECTOR).to_radians() * 1.5).max(0.3);
            let blocked = obstacles.is_some_and(|r| !r.segment_clear(pp, p.1));
            if gap > allowed || blocked {
                if cur.len() > 1 {
                    lines.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
        cur.push(p.1);
        prev = Some(p);
    }
    if cur.len() > 1 {
        lines.push(cur);
    }
    lines
}

/// Isochrones as contours of the time each grid cell was first reached.
///
/// Levels are at a round interval (10 min .. 24 h) giving at most about 40
/// lines. Squares touching land, shallows or unreached cells are skipped, so
/// lines stop at the coast instead of running along it. Lines are smoothed.
fn contour_isochrones(
    reached: &std::collections::HashMap<(i32, i32), u32, CellBuild>,
    cell_deg: f64,
    kx: f64,
    obstacles: Option<&Raster>,
    steps: u32,
    step_s: f64,
) -> Vec<Vec<Vec<[f64; 2]>>> {
    if reached.len() < 4 || steps == 0 {
        return Vec::new();
    }
    let (mut i0, mut i1, mut j0, mut j1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for &(i, j) in reached.keys() {
        i0 = i0.min(i);
        i1 = i1.max(i);
        j0 = j0.min(j);
        j1 = j1.max(j);
    }
    let (w, h) = ((i1 - i0 + 1) as usize, (j1 - j0 + 1) as usize);
    if w * h > 6_000_000 {
        return Vec::new();
    }
    let lon_of = |x: f64| (x + 0.5) * cell_deg / kx;
    let lat_of = |y: f64| (y + 0.5) * cell_deg;
    // -1 = unreached or blocked
    let mut v = vec![-1.0f32; w * h];
    for (&(i, j), &st) in reached {
        let (x, y) = ((i - i0) as usize, (j - j0) as usize);
        let blocked = obstacles.is_some_and(|r| r.blocked(lat_of(j as f64), lon_of(i as f64)));
        if !blocked {
            v[y * w + x] = st as f32;
        }
    }
    let total_s = steps as f64 * step_s;
    let nice = [600.0, 900.0, 1800.0, 3600.0, 7200.0, 10800.0, 21600.0, 43200.0, 86400.0];
    let interval = nice.iter().copied().find(|iv| total_s / iv <= 40.0).unwrap_or(86400.0);
    let every = ((interval / step_s).round() as u32).max(1);

    let mut out = Vec::new();
    let mut level = every;
    while level < steps {
        let lv = level as f32 + 0.5;
        let mut segs: Vec<([f64; 2], [f64; 2])> = Vec::new();
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let a = v[y * w + x];
                let b = v[y * w + x + 1];
                let c = v[(y + 1) * w + x + 1];
                let d = v[(y + 1) * w + x];
                if a < 0.0 || b < 0.0 || c < 0.0 || d < 0.0 {
                    continue;
                }
                let idx = (a <= lv) as u8 | ((b <= lv) as u8) << 1 | ((c <= lv) as u8) << 2 | ((d <= lv) as u8) << 3;
                if idx == 0 || idx == 15 {
                    continue;
                }
                let gx = (i0 + x as i32) as f64;
                let gy = (j0 + y as i32) as f64;
                let lerp = |va: f32, vb: f32| ((lv - va) / (vb - va)) as f64;
                // edge points: bottom (a-b), right (b-c), top (d-c), left (a-d)
                let e_b = || [lon_of(gx + lerp(a, b)), lat_of(gy)];
                let e_r = || [lon_of(gx + 1.0), lat_of(gy + lerp(b, c))];
                let e_t = || [lon_of(gx + lerp(d, c)), lat_of(gy + 1.0)];
                let e_l = || [lon_of(gx), lat_of(gy + lerp(a, d))];
                let centre_in = (a + b + c + d) / 4.0 <= lv;
                match idx {
                    1 | 14 => segs.push((e_l(), e_b())),
                    2 | 13 => segs.push((e_b(), e_r())),
                    3 | 12 => segs.push((e_l(), e_r())),
                    4 | 11 => segs.push((e_r(), e_t())),
                    6 | 9 => segs.push((e_b(), e_t())),
                    7 | 8 => segs.push((e_l(), e_t())),
                    5 => {
                        if centre_in {
                            segs.push((e_l(), e_t()));
                            segs.push((e_b(), e_r()));
                        } else {
                            segs.push((e_l(), e_b()));
                            segs.push((e_r(), e_t()));
                        }
                    }
                    10 => {
                        if centre_in {
                            segs.push((e_l(), e_b()));
                            segs.push((e_r(), e_t()));
                        } else {
                            segs.push((e_l(), e_t()));
                            segs.push((e_b(), e_r()));
                        }
                    }
                    _ => {}
                }
            }
        }
        let lines: Vec<Vec<[f64; 2]>> = join_segments(segs).into_iter().filter(|l| l.len() >= 3).map(|l| chaikin(&chaikin(&l))).collect();
        if !lines.is_empty() {
            out.push(lines);
        }
        level += every;
    }
    out
}

/// Join contour segments end to end into polylines.
fn join_segments(segs: Vec<([f64; 2], [f64; 2])>) -> Vec<Vec<[f64; 2]>> {
    let key = |p: &[f64; 2]| ((p[0] * 1e7).round() as i64, (p[1] * 1e7).round() as i64);
    let mut at: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (k, (a, b)) in segs.iter().enumerate() {
        at.entry(key(a)).or_default().push(k);
        at.entry(key(b)).or_default().push(k);
    }
    let mut used = vec![false; segs.len()];
    let mut lines = Vec::new();
    for s0 in 0..segs.len() {
        if used[s0] {
            continue;
        }
        used[s0] = true;
        let mut line = std::collections::VecDeque::from([segs[s0].0, segs[s0].1]);
        // grow forwards, then backwards
        for forward in [true, false] {
            loop {
                let end = if forward { *line.back().unwrap() } else { *line.front().unwrap() };
                let next = at.get(&key(&end)).and_then(|v| v.iter().copied().find(|&k| !used[k]));
                let Some(k) = next else { break };
                used[k] = true;
                let (a, b) = segs[k];
                let other = if key(&a) == key(&end) { b } else { a };
                if forward {
                    line.push_back(other);
                } else {
                    line.push_front(other);
                }
            }
        }
        lines.push(line.into_iter().collect());
    }
    lines
}

/// One pass of Chaikin corner cutting (endpoints kept).
fn chaikin(l: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if l.len() < 3 {
        return l.to_vec();
    }
    let mut out = Vec::with_capacity(l.len() * 2);
    out.push(l[0]);
    for w in l.windows(2) {
        let (a, b) = (w[0], w[1]);
        out.push([0.75 * a[0] + 0.25 * b[0], 0.75 * a[1] + 0.25 * b[1]]);
        out.push([0.25 * a[0] + 0.75 * b[0], 0.25 * a[1] + 0.75 * b[1]]);
    }
    out.push(*l.last().unwrap());
    out
}

/// Fast hasher for grid cell keys (the default SipHash is slow for this).
#[derive(Default, Clone, Copy)]
struct CellHasher(u64);
impl std::hash::Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ *b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_i32(&mut self, i: i32) {
        self.0 = (self.0.rotate_left(5) ^ i as u32 as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}
type CellBuild = std::hash::BuildHasherDefault<CellHasher>;

fn cell_key(lat: f64, lon: f64, cell_deg: f64, kx: f64) -> (i32, i32) {
    ((lon * kx / cell_deg).floor() as i32, (lat / cell_deg).floor() as i32)
}

/// Fraction (0..1) along a→b where the path first comes within `r_nm` of `c`,
/// using a local flat-earth frame (fine over one step).
fn arrival_fraction(a: LatLon, b: LatLon, c: LatLon, r_nm: f64) -> Option<f64> {
    let k = c.lat.to_radians().cos();
    let to_xy = |p: &LatLon| ((nav_core::norm180(p.lon - c.lon)) * 60.0 * k, (p.lat - c.lat) * 60.0);
    let (ax, ay) = to_xy(&a);
    let (bx, by) = to_xy(&b);
    let (dx, dy) = (bx - ax, by - ay);
    let aa = dx * dx + dy * dy;
    let bb = 2.0 * (ax * dx + ay * dy);
    let cc = ax * ax + ay * ay - r_nm * r_nm;
    if cc <= 0.0 {
        return Some(0.0);
    }
    if aa == 0.0 {
        return None;
    }
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 {
        return None;
    }
    let f = (-bb - disc.sqrt()) / (2.0 * aa);
    (0.0..=1.0).contains(&f).then_some(f)
}

fn interpolate_arrival(a: &Node, b: &Node, dest: LatLon, r_nm: f64) -> LatLon {
    let pa = LatLon::new(a.lat, a.lon);
    let pb = LatLon::new(b.lat, b.lon);
    let f = arrival_fraction(pa, pb, dest, r_nm).unwrap_or(1.0);
    LatLon::new(a.lat + (b.lat - a.lat) * f, a.lon + norm180(b.lon - a.lon) * f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple, smooth test polar: 6 kn reaching, slower upwind, nothing inside 40°.
    fn test_polar() -> Polar {
        let txt = "TWA\\TWS;6;10;14;20\n0;0;0;0;0\n35;0;0;0;0\n40;3.5;4.5;5.0;5.2\n45;4.0;5.0;5.5;5.7\n60;4.6;5.8;6.3;6.5\n90;5.0;6.4;7.0;7.4\n120;4.8;6.3;7.2;7.8\n150;4.0;5.6;6.6;7.5\n180;3.5;5.0;6.0;7.0\n";
        Polar::parse("test", txt).unwrap()
    }

    #[test]
    fn reach_goes_straight() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let dest = start.destination(90.0, 10.0); // 10 NM east, wind from north: beam reach
        let r = route(start, 0, &[dest], &pol, &SteadyWind { tws: 10.0, twd: 0.0 }, None, &Params::default());
        assert_eq!(r.status, "arrived", "{}", r.message);
        let hours = r.arrival.unwrap() as f64 / 3600.0;
        // 6.4 kn on the beam: about 1.56 h
        assert!((hours - 10.0 / 6.4).abs() < 0.05, "{hours}");
        assert_eq!(r.legs[0].tacks, 0);
    }

    #[test]
    fn dead_upwind_needs_tacks_and_vmg_time() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let dest = start.destination(0.0, 10.0); // 10 NM dead upwind
        let p = Params { tack_s: 30.0, ..Default::default() };
        let r = route(start, 0, &[dest], &pol, &SteadyWind { tws: 10.0, twd: 0.0 }, None, &p);
        assert_eq!(r.status, "arrived", "{}", r.message);
        let vmg = pol.best_vmg_up(10.0, 30.0).vmg;
        let hours = r.arrival.unwrap() as f64 / 3600.0;
        let ideal = 10.0 / vmg;
        assert!(hours >= ideal * 0.98 && hours < ideal * 1.10, "took {hours:.3} h, VMG ideal {ideal:.3} h");
        assert!(r.legs[0].tacks >= 1);
    }

    #[test]
    fn goes_round_an_island() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let dest = start.destination(90.0, 10.0);
        // island 2 NM wide square across the direct line, half way
        let mid = start.destination(90.0, 5.0);
        let d = 1.0 / 60.0; // 1 NM of latitude
        let k = mid.lat.to_radians().cos();
        let island = vec![vec![
            [mid.lon - d / k, mid.lat - d],
            [mid.lon + d / k, mid.lat - d],
            [mid.lon + d / k, mid.lat + d],
            [mid.lon - d / k, mid.lat + d],
            [mid.lon - d / k, mid.lat - d],
        ]];
        let mut ras = Raster::new([172.9, -41.2, 173.4, -40.8], 50.0, 4_000_000);
        ras.fill(&island, raster::BLOCKED);
        ras.block_outline(&island);
        let r = route(start, 0, &[dest], &pol, &SteadyWind { tws: 10.0, twd: 0.0 }, Some(&ras), &Params::default());
        assert_eq!(r.status, "arrived", "{}", r.message);
        for w in r.points.windows(2) {
            assert!(ras.segment_clear([w[0].lon, w[0].lat], [w[1].lon, w[1].lat]), "route crosses the island");
        }
        assert!(r.distance_nm > 10.05);
    }

    #[test]
    fn stops_when_the_grib_ends() {
        struct Short;
        impl WindSource for Short {
            fn wind(&self, _: f64, _: f64, t: i64) -> Option<(f64, f64)> {
                (t <= 3600).then_some((10.0, 0.0))
            }
            fn end_time(&self) -> Option<i64> {
                Some(3600)
            }
        }
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let dest = start.destination(90.0, 20.0);
        let p = Params { step_s: 600.0, ..Params::default() };
        let r = route(start, 0, &[dest], &pol, &Short, None, &p);
        assert_eq!(r.status, "grib end", "{}", r.message);
        assert!(r.arrival.is_none());
        // a partial route up to the end of the weather, heading for the mark
        let last = r.points.last().unwrap();
        assert_eq!(last.t, 3600);
        let left = LatLon::new(last.lat, last.lon).distance_to(&dest);
        assert!(left < 20.0 - 4.0, "{left}");
    }

    #[test]
    fn auto_step_scales_with_the_leg() {
        assert_eq!(auto_step_s(10.0, 8.0, true), 300.0);
        assert_eq!(auto_step_s(130.0, 12.0, true), 900.0);
        assert_eq!(auto_step_s(2000.0, 10.0, true), 3600.0);
        assert_eq!(auto_step_s(2000.0, 10.0, false), 10800.0);
    }

    #[test]
    fn two_legs() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let m1 = start.destination(90.0, 5.0);
        let m2 = m1.destination(180.0, 5.0);
        let r = route(start, 0, &[m1, m2], &pol, &SteadyWind { tws: 14.0, twd: 0.0 }, None, &Params::default());
        assert_eq!(r.status, "arrived", "{}", r.message);
        assert_eq!(r.legs.len(), 2);
        assert!(r.legs[1].start == r.legs[0].arrival.unwrap());
    }

    #[test]
    fn start_on_land_is_refused() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let mut ras = Raster::new([172.9, -41.1, 173.1, -40.9], 100.0, 1_000_000);
        ras.fill(&[vec![[172.95, -41.05], [173.05, -41.05], [173.05, -40.95], [172.95, -40.95], [172.95, -41.05]]], raster::BLOCKED);
        let r = route(start, 0, &[start.destination(90.0, 5.0)], &pol, &SteadyWind { tws: 10.0, twd: 0.0 }, Some(&ras), &Params::default());
        assert_eq!(r.status, "start blocked");
    }
}
