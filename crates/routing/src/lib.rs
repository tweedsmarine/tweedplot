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
//! arrived. A mark is reached by entering its arrival circle (its own radius,
//! or the routing's), "as close as possible" (a 0.01 NM circle), or rounded to
//! port or starboard. Rounding follows the string rule: the bearing from the
//! mark to the boat must turn, clockwise when the mark is left to starboard
//! (anticlockwise to port), from the bearing of the previous mark to the
//! bearing of the next one (to abeam for the last mark). So a mark inside the
//! turn is gone right round, one outside it just passed on the correct side.
//! The leg sails to a circle of open water round the mark with its usual
//! steps, then rounds it with short ones, keeping track of how far round each
//! branch has gone (the same water reached the wrong way round is a different
//! place to the router). The mark is a disc of its clearance the route may not
//! cut, on that leg and the next. If there is no way round on the side asked
//! (the gap to the shore too shallow, say), the routing stops and says so.
//!
//! Limits drop parts of the fan: headings outside ±`fan_deg` of the bearing to
//! the mark, true wind angles outside `min_twa`..`max_twa`; and points where
//! the wind is outside `min_tws`..`max_tws` or the waves above `max_hs` are
//! not sailed on from. User barriers (lines, zones, discs) are obstacles like
//! the chart (see [`barriers`]).
//!
//! Wind outside the GRIB's time span or area stops a branch; if every branch
//! stops, the result says so and gives the closest approach, rather than
//! inventing weather.

pub mod barriers;
pub mod raster;

use nav_core::{norm180, norm360, LatLon};
use polar::Polar;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub use barriers::Barriers;
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
    /// Significant wave height (m), if known.
    fn wave_height(&self, _lat: f64, _lon: f64, _t: i64) -> Option<f64> {
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
    fn wave_height(&self, lat: f64, lon: f64, t: i64) -> Option<f64> {
        self.value(grib::Param::WaveHeight, lat, lon, t)
    }
}

/// Wind (and current) from one source, waves from another (e.g. GFS + GFS-Wave).
pub struct WithWaves<'a> {
    pub wind: &'a dyn WindSource,
    pub waves: &'a dyn WindSource,
}

impl WindSource for WithWaves<'_> {
    fn wind(&self, lat: f64, lon: f64, t: i64) -> Option<(f64, f64)> {
        self.wind.wind(lat, lon, t)
    }
    fn current(&self, lat: f64, lon: f64, t: i64) -> Option<(f64, f64)> {
        self.wind.current(lat, lon, t)
    }
    fn end_time(&self) -> Option<i64> {
        self.wind.end_time()
    }
    fn wave_height(&self, lat: f64, lon: f64, t: i64) -> Option<f64> {
        self.waves.wave_height(lat, lon, t).or_else(|| self.wind.wave_height(lat, lon, t))
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
    /// Headings tried: within ± this of the bearing to the mark (180 = all round).
    pub fan_deg: f64,
    /// Don't sail on from places with less wind than this, or more (kn; 0 = no limit).
    pub min_tws: f64,
    pub max_tws: f64,
    /// Don't sail on from places with higher waves (Hs, m; 0 = no limit).
    pub max_hs: f64,
    /// True wind angles allowed (degrees off the wind, either side).
    pub min_twa: f64,
    pub max_twa: f64,
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
            fan_deg: 180.0,
            min_tws: 0.0,
            max_tws: 0.0,
            max_hs: 0.0,
            min_twa: 0.0,
            max_twa: 180.0,
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

/// How a mark is to be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Rounding {
    /// Enter the arrival circle.
    #[default]
    Through,
    /// Leave the mark to port (the boat passes with the mark on its left).
    Port,
    /// Leave the mark to starboard.
    Starboard,
    /// As close as possible: a 0.01 NM circle.
    Close,
}

impl Rounding {
    /// "port", "stbd"/"starboard", "close", anything else = through.
    pub fn parse(s: &str) -> Self {
        match s {
            "port" => Rounding::Port,
            "stbd" | "starboard" => Rounding::Starboard,
            "close" => Rounding::Close,
            _ => Rounding::Through,
        }
    }
}

/// A course mark with how it is to be reached.
#[derive(Debug, Clone, Copy)]
pub struct MarkSpec {
    pub pos: LatLon,
    pub rounding: Rounding,
    /// Arrival radius, or the clearance when rounding (NM); None = the routing's arrival radius.
    pub radius_nm: Option<f64>,
}

impl MarkSpec {
    pub fn at(pos: LatLon) -> Self {
        MarkSpec { pos, rounding: Rounding::Through, radius_nm: None }
    }
}

const ROUND_BLOCKED: &str = "There is no way round the mark on the side asked: land, water shallower than the safety depth, a barrier or the routing limits close it off. Check the rounding side, or lower the safety depth if that is safe.";

/// What ends a leg.
#[derive(Debug, Clone, Copy)]
enum Goal {
    /// Enter a circle.
    Circle { c: LatLon, r: f64 },
    /// Go round the mark `m`: the bearing from the mark to the boat must turn by
    /// `need` degrees, clockwise (`sense` +1, left to starboard) or anticlockwise
    /// (-1, to port), counted from `b0` (the bearing of the previous mark). The leg
    /// ends where that is done, at least `clear` NM off.
    /// `near`: the radius of water round the mark it is rounded within (NM).
    /// `exit`: with a next mark (its position, and a radius), the leg ends not on the
    /// line to the next mark, which may run up a blind inlet, but on leaving that
    /// circle heading away from the mark towards the next one, when the turn the
    /// boat has made, plus the bit still to come on the way there, is the rounding.
    Wrap { m: LatLon, sense: f64, b0: f64, need: f64, clear: f64, near: f64, exit: Option<(LatLon, f64)> },
}

impl Goal {
    /// How far round the mark (degrees, in the rounding sense) the step a→b goes.
    fn sweep(&self, a: LatLon, b: LatLon) -> f64 {
        match *self {
            Goal::Circle { .. } => 0.0,
            Goal::Wrap { m, sense, .. } => {
                if m.distance_to(&a) < 1e-9 || m.distance_to(&b) < 1e-9 {
                    return 0.0;
                }
                sense * norm180(m.bearing_to(&b) - m.bearing_to(&a))
            }
        }
    }

    /// The wrap at the start of a leg: where the start lies round the mark from `b0`.
    fn wrap0(&self, start: LatLon) -> f64 {
        match *self {
            Goal::Circle { .. } => 0.0,
            Goal::Wrap { m, sense, b0, .. } => {
                if m.distance_to(&start) < 1e-9 {
                    0.0
                } else {
                    sense * norm180(m.bearing_to(&start) - b0)
                }
            }
        }
    }

    /// With an exit: is the rounding done at `p`, the boat `wrap` round?
    fn exit_done(&self, p: LatLon, wrap: f64) -> bool {
        let Goal::Wrap { m, sense, need, exit: Some((n, r_exit)), .. } = *self else { return false };
        if m.distance_to(&p) < r_exit {
            return false;
        }
        let out = m.bearing_to(&p);
        // heading for the next mark takes the boat away from this one (not back past it)
        if norm180(p.bearing_to(&n) - out).abs() >= 90.0 {
            return false;
        }
        // and the turn round the mark it will have made there is the rounding asked
        (wrap + sense * norm180(m.bearing_to(&n) - out) - need).abs() < 90.0
    }

    /// Already round at the start of a leg?
    fn done_at_start(&self, p: LatLon) -> bool {
        match *self {
            Goal::Circle { .. } => false,
            Goal::Wrap { exit: Some(_), .. } => self.exit_done(p, self.wrap0(p)),
            Goal::Wrap { need, .. } => self.wrap0(p) >= need,
        }
    }

    /// Fraction along the step a→b (the boat at `wrap` round the mark at a) where the goal is reached, if it is.
    fn fraction(&self, a: LatLon, b: LatLon, wrap: f64) -> Option<f64> {
        match *self {
            Goal::Circle { c, r } => arrival_fraction(a, b, c, r),
            Goal::Wrap { m, exit: Some((_, r_exit)), .. } => {
                if !self.exit_done(b, wrap + self.sweep(a, b)) {
                    return None;
                }
                // where it leaves the circle, if in this step
                if m.distance_to(&a) < r_exit {
                    let k = m.lat.to_radians().cos();
                    let xy = |p: &LatLon| (norm180(p.lon - m.lon) * 60.0 * k, (p.lat - m.lat) * 60.0);
                    let (ax, ay) = xy(&a);
                    let (bx, by) = xy(&b);
                    let (dx, dy) = (bx - ax, by - ay);
                    let aa = dx * dx + dy * dy;
                    let bb = 2.0 * (ax * dx + ay * dy);
                    let cc = ax * ax + ay * ay - r_exit * r_exit;
                    let disc = bb * bb - 4.0 * aa * cc;
                    if aa > 0.0 && disc >= 0.0 {
                        return Some(((-bb + disc.sqrt()) / (2.0 * aa)).clamp(0.0, 1.0));
                    }
                }
                Some(1.0)
            }
            Goal::Wrap { m, sense, b0, need, clear, .. } => {
                if wrap >= need || wrap + self.sweep(a, b) < need {
                    return None;
                }
                // where the step crosses the line from the mark on the finishing bearing
                let dir = b0 + sense * need;
                let k = m.lat.to_radians().cos();
                let xy = |p: &LatLon| (norm180(p.lon - m.lon) * 60.0 * k, (p.lat - m.lat) * 60.0);
                let (ax, ay) = xy(&a);
                let (bx, by) = xy(&b);
                let (dx, dy) = (bx - ax, by - ay);
                let (ux, uy) = (dir.to_radians().sin(), dir.to_radians().cos());
                let den = dx * uy - dy * ux;
                if den.abs() < 1e-12 {
                    return None;
                }
                let t = (-(ax * uy - ay * ux) / den).clamp(0.0, 1.0);
                let s = (ax + t * dx) * ux + (ay + t * dy) * uy;
                (s >= clear * 0.999).then_some(t)
            }
        }
    }
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
    /// Degrees gone round the mark being rounded (0 for other legs).
    wrap: f64,
}

/// Route from `start` at time `t0` through `marks` in order (each entered within the arrival radius).
pub fn route(start: LatLon, t0: i64, marks: &[LatLon], polar: &Polar, wind: &dyn WindSource, obstacles: Option<&Raster>, p: &Params) -> RouteResult {
    let specs: Vec<MarkSpec> = marks.iter().map(|m| MarkSpec::at(*m)).collect();
    route_marks(start, t0, &specs, polar, wind, obstacles, &Barriers::default(), p)
}

/// Route from `start` at time `t0` through `marks` (with rounding and radius), avoiding `barriers` too.
#[allow(clippy::too_many_arguments)]
pub fn route_marks(
    start: LatLon,
    t0: i64,
    marks: &[MarkSpec],
    polar: &Polar,
    wind: &dyn WindSource,
    obstacles: Option<&Raster>,
    barriers: &Barriers,
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
    if barriers.blocked(start) {
        res.status = "start blocked".into();
        res.message = "The start is inside a barrier (zone or barrier mark). Move it, or switch that barrier off.".into();
        return res;
    }
    // a mark left to port or starboard may be the rock or island itself: only a
    // mark the boat must reach (pass through, as close as possible) needs open water
    let reached = |m: &MarkSpec| matches!(m.rounding, Rounding::Through | Rounding::Close);
    // a pass-through mark is reached anywhere inside its radius: it is blocked only
    // when the centre and the whole circle are (a finish given on the shore, say)
    let probe = |m: &MarkSpec| -> Vec<LatLon> {
        let r = if m.rounding == Rounding::Through { m.radius_nm.unwrap_or(p.arrival_nm) } else { 0.0 };
        let mut v = vec![m.pos];
        if r > 0.0 {
            for k in 0..24 {
                for f in [0.5, 0.95] {
                    v.push(m.pos.destination(k as f64 * 15.0, r * f));
                }
            }
        }
        v
    };
    if let Some(i) = marks.iter().position(|m| reached(m) && probe(m).iter().all(|q| barriers.blocked(*q))) {
        res.status = "mark blocked".into();
        res.message = format!("Mark {} is inside a barrier (zone or barrier mark). Move the mark, or switch that barrier off.", i + 1);
        return res;
    }
    if let Some(r) = obstacles {
        if let Some(i) = marks.iter().position(|m| reached(m) && probe(m).iter().all(|q| r.blocked(q.lat, q.lon))) {
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
    let disc = |m: &MarkSpec| -> Option<([f64; 2], f64)> {
        matches!(m.rounding, Rounding::Port | Rounding::Starboard).then(|| ([m.pos.lon, m.pos.lat], m.radius_nm.unwrap_or(p.arrival_nm).max(0.005)))
    };
    for (i, mark) in marks.iter().enumerate() {
        // the course point before this mark (not where the last leg ended): sets the side of a rounding
        let prev = if i == 0 { start } else { marks[i - 1].pos };
        let goal = match mark.rounding {
            Rounding::Through => Goal::Circle { c: mark.pos, r: mark.radius_nm.unwrap_or(p.arrival_nm) },
            Rounding::Close => Goal::Circle { c: mark.pos, r: 0.01 },
            Rounding::Port | Rounding::Starboard => {
                let sense = if mark.rounding == Rounding::Starboard { 1.0 } else { -1.0 };
                let b0 = if prev.distance_to(&mark.pos) > 1e-6 { mark.pos.bearing_to(&prev) } else { mark.pos.bearing_to(&from) };
                // round as far as the next mark's bearing (the string rule); the last
                // mark only to abeam. Straight back where it came from = all the way round.
                let need = match marks.get(i + 1).filter(|n| n.pos.distance_to(&mark.pos) > 1e-6) {
                    Some(n) => {
                        let d = norm360(sense * (mark.pos.bearing_to(&n.pos) - b0));
                        if d < 1.0 {
                            360.0
                        } else {
                            d
                        }
                    }
                    None => 90.0,
                };
                let clear = mark.radius_nm.unwrap_or(p.arrival_nm);
                // the water to round it in: a mile or so, more round a big island (the mark
                // its middle) until a good part of the circle is open water
                let open = |q: LatLon| !barriers.blocked(q) && obstacles.is_none_or(|r| !r.blocked(q.lat, q.lon));
                let mut near = (4.0 * clear).max(1.0);
                while near < 30.0 && (0..36).filter(|k| open(mark.pos.destination(*k as f64 * 10.0, near))).count() < 9 {
                    near *= 1.4;
                }
                // with a next mark, the rounding ends heading out towards it, a little way off
                let exit = marks
                    .get(i + 1)
                    .filter(|n| n.pos.distance_to(&mark.pos) > 1e-6)
                    .map(|n| (n.pos, near.min(0.5 * mark.pos.distance_to(&n.pos)).max(1.5 * clear)));
                Goal::Wrap { m: mark.pos, sense, b0, need, clear, near, exit }
            }
        };
        // this mark (if rounded) and the last one (if it was rounded) can't be cut through
        let mut leg_barriers = barriers.clone();
        leg_barriers.discs.extend(disc(mark));
        if i > 0 {
            leg_barriers.discs.extend(disc(&marks[i - 1]));
        }
        let leg = match goal {
            Goal::Wrap { need, near, .. } => {
                // Going round a mark is done close to it, with short steps; getting
                // there with the leg's usual ones. So: sail to a circle round the mark,
                // then round it from there.
                let r_near = near;
                let mut approach = if from.distance_to(&mark.pos) > r_near * 1.05 {
                    Some(route_leg(from, t, mark.pos, Goal::Circle { c: mark.pos, r: r_near }, polar, wind, obstacles, &leg_barriers, p, &mut res))
                } else {
                    None
                };
                let (f2, t2) = match approach.as_ref().map(|a| a.arrived) {
                    None => (from, t),
                    Some(Some((lat, lon, ta))) => (LatLon::new(lat, lon), ta),
                    Some(None) => (from, t), // didn't get there: reported below
                };
                let _ = need;
                if approach.as_ref().is_some_and(|a| a.arrived.is_none()) || goal.done_at_start(f2) {
                    approach.take().unwrap_or_else(|| route_leg(from, t, mark.pos, goal, polar, wind, obstacles, &leg_barriers, p, &mut res))
                } else {
                    let vmax = polar.speeds.iter().flatten().fold(0.0f64, |a, &b| a.max(b)) * p.efficiency;
                    let auto = (2.0 * std::f64::consts::PI * r_near / vmax.max(1.0) * 3600.0 / 60.0)
                        .clamp(60.0, 180.0)
                        // a tack or gybe must leave most of a step for sailing
                        .max(3.0 * p.tack_s.max(p.gybe_s))
                        .min(600.0);
                    let near_p = Params { step_s: if p.step_s > 0.0 { p.step_s.min(auto) } else { auto }, ..p.clone() };
                    // aim (for pruning) where the rounding ends: out from the mark towards the next
                    let aim = match goal {
                        Goal::Wrap { m, exit: Some((n, r_exit)), .. } => m.destination(m.bearing_to(&n), r_exit),
                        _ => mark.pos,
                    };
                    let round = route_leg(f2, t2, aim, goal, polar, wind, obstacles, &leg_barriers, &near_p, &mut res);
                    match approach {
                        None => round,
                        Some(mut a) => {
                            // one leg: the way in, then round the mark
                            if !round.points.is_empty() {
                                a.points.pop();
                            }
                            a.points.extend(round.points);
                            a.leg.to = round.leg.to;
                            a.leg.distance_nm += round.leg.distance_nm;
                            a.leg.tacks += round.leg.tacks;
                            a.leg.gybes += round.leg.gybes;
                            a.leg.arrival = round.leg.arrival;
                            a.arrived = round.arrived;
                            a.status = round.status;
                            a.message = round.message;
                            a
                        }
                    }
                }
            }
            Goal::Circle { .. } => route_leg(from, t, mark.pos, goal, polar, wind, obstacles, &leg_barriers, p, &mut res),
        };
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
    goal: Goal,
    polar: &Polar,
    wind: &dyn WindSource,
    obstacles: Option<&Raster>,
    barriers: &Barriers,
    p: &Params,
    res: &mut RouteResult,
) -> LegOut {
    let barriers = (!barriers.is_empty()).then_some(barriers);
    let limits = p.min_tws > 0.0 || p.max_tws > 0.0 || p.max_hs > 0.0 || p.min_twa > 0.0 || p.max_twa < 180.0 || p.fan_deg < 180.0;
    let vmax = polar.speeds.iter().flatten().fold(0.0f64, |a, &b| a.max(b)) * p.efficiency;
    let direct = start.distance_to(&dest);
    let step_s = if p.step_s > 0.0 { p.step_s } else { auto_step_s(direct, vmax, obstacles.is_some()) };
    let dt_h = step_s / 3600.0;
    let mut nodes: Vec<Node> = vec![Node {
        lat: start.lat,
        lon: start.lon,
        t: t0,
        parent: usize::MAX,
        heading: f64::NAN,
        twa: 0.0,
        tws: 0.0,
        twd: 0.0,
        bsp: 0.0,
        sog: 0.0,
        wrap: goal.wrap0(start),
    }];
    let mut front: Vec<usize> = vec![0];
    let n_sectors = (360.0 / p.sector_deg).ceil() as usize;
    // Pruning mode and grid size
    let grid_mode = match p.pruning.as_str() {
        "grid" => true,
        // going round a mark needs the grid (with the wrap in its key)
        "sectors" => matches!(goal, Goal::Wrap { .. }),
        _ => obstacles.is_some() || barriers.is_some() || matches!(goal, Goal::Wrap { .. }),
    };

    // Grid cells about half a step across at the speed the boat makes in the wind at
    // the start (not the polar's fastest, which may be a 40 kn surf): a step much
    // shorter than a cell stays in it, and the front can stick there
    let v_typ = wind
        .wind(start.lat, start.lon, t0.round() as i64)
        .map(|(tws, _)| polar.bsp(90.0, tws.max(4.0)) * p.efficiency)
        .filter(|v| *v > 0.5)
        .unwrap_or(vmax)
        .min(vmax);
    // (only going round a mark, where the steps are short and the water tight: elsewhere
    // the coarser cells keep the routing quick)
    let v_cell = if matches!(goal, Goal::Wrap { .. }) { v_typ } else { v_typ.max(polar.bsp(90.0, 8.0) * p.efficiency).min(vmax) };
    let cell_min = if matches!(goal, Goal::Wrap { .. }) { 0.04 } else { 0.02 };
    let cell_nm = if p.cell_nm > 0.0 { p.cell_nm } else { (v_cell * dt_h * 0.5).max(cell_min) };
    let cell_deg = cell_nm / 60.0;
    let kx = start.lat.to_radians().cos().max(0.05);
    // going round a mark may take the whole way round it, a fair way off
    let round_extra = match goal {
        Goal::Wrap { near, .. } => 2.0 * std::f64::consts::PI * near,
        Goal::Circle { .. } => 0.0,
    };
    let detour_limit = direct * p.max_detour + 2.0 * cell_nm + 1.0 + round_extra;
    // Cells are kept apart by how far round the mark the boat has gone: the same
    // water reached the wrong way round (or not yet round) is a different place.
    let wrapping = matches!(goal, Goal::Wrap { .. });
    let (wrap_need, wrap_r, wrap_far, wrap_m) = match goal {
        Goal::Wrap { m, need, clear, near, .. } => (need, clear.max(0.1), 3.0 * near, m),
        Goal::Circle { c, .. } => (0.0, 0.0, f64::MAX, c),
    };
    let sheet = |w: f64| ((w + 180.0) / 360.0).floor() as i32;
    let key3 = |lat: f64, lon: f64, w: f64| {
        let (i, j) = cell_key(lat, lon, cell_deg, kx);
        (i, j, sheet(w))
    };
    let mut visited: std::collections::HashSet<(i32, i32, i32), CellBuild> = std::collections::HashSet::default();
    visited.insert(key3(start.lat, start.lon, nodes[0].wrap));
    // Step at which each grid cell was first reached: contoured for the isochrone display.
    let mut reached: std::collections::HashMap<(i32, i32), u32, CellBuild> = std::collections::HashMap::default();
    reached.insert(cell_key(start.lat, start.lon, cell_deg, kx), 0);
    let mut leg_steps: u32 = 0;
    // time since the front last reached new water (grid mode): when it only marks
    // time where it is, there is no way on
    let mut stale_s = 0.0f64;
    // the furthest round the mark the front has been (degrees), and seconds since that grew
    let mut best_wrap = (f64::MIN, 0.0f64);
    let headings: Vec<f64> = {
        let n = (360.0 / p.heading_step_deg).round() as usize;
        (0..n).map(|k| k as f64 * 360.0 / n as f64).collect()
    };
    let mut no_wind = false;
    let mut best_approach = (start.distance_to(&dest), 0usize);
    let mut status = String::from("too many steps");
    let mut message = format!("No arrival within {} steps.", p.max_steps);

    for step in 0..p.max_steps {
        if front.is_empty() {
            status = "blocked".into();
            message = if matches!(goal, Goal::Wrap { .. }) {
                ROUND_BLOCKED.into()
            } else {
                "Every way on is blocked by land, shallow water, barriers or the routing limits.".into()
            };
            break;
        }
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
                // limits on where the boat may sail on from (not the start itself)
                if n.parent != usize::MAX {
                    if (p.max_tws > 0.0 && tws > p.max_tws) || (p.min_tws > 0.0 && tws < p.min_tws) {
                        return out.into_iter();
                    }
                    if p.max_hs > 0.0 && wind.wave_height(n.lat, n.lon, t.round() as i64).is_some_and(|h| h > p.max_hs) {
                        return out.into_iter();
                    }
                }
                let mut hs: Vec<f64> = headings.clone();
                if p.fan_deg < 180.0 {
                    hs.retain(|h| norm180(h - brg_dest).abs() <= p.fan_deg);
                }
                hs.push(brg_dest); // always try straight at the mark
                for h in hs {
                    let twa = norm180(twd - h); // + = wind from starboard
                    if twa.abs() < p.min_twa || twa.abs() > p.max_twa {
                        continue;
                    }
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
                    let wrap = n.wrap + goal.sweep(here, np);
                    if wrap < -120.0 || (wrapping && np.distance_to(&wrap_m) > wrap_far) {
                        continue; // well round the wrong way, or far off: never the fast way round
                    }
                    if grid_mode {
                        // cheap rejections before the obstacle check
                        let key = key3(np.lat, np.lon, wrap);
                        if visited.contains(&key) && key != key3(n.lat, n.lon, n.wrap) {
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
                    if let Some(b) = barriers {
                        if !b.segment_clear([n.lon, n.lat], [np.lon, np.lat]) {
                            continue;
                        }
                    }
                    // Arrival: does this step reach the goal (circle or rounding line)?
                    let arrive = goal.fraction(here, np, n.wrap).map(|f| {
                        // time spent: penalty first, then sailing
                        let pen_h = dt_h - hours;
                        t + (pen_h + f * hours) * 3600.0
                    });
                    out.push((Node { lat: np.lat, lon: np.lon, t: t + step_s, parent: ni, heading: h, twa, tws, twd, bsp, sog, wrap }, arrive));
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
                    let (dd, ni) =
                        front.iter().map(|&ni| (LatLon::new(nodes[ni].lat, nodes[ni].lon).distance_to(&dest), ni)).min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
                    best_approach = (dd, ni);
                    status = "grib end".into();
                    message = format!("The GRIB ends at {}; the route stops there,", grib::fmt_run(end));
                } else {
                    status = "no wind data".into();
                    message = format!("The GRIB has no wind for the route at {} (outside its area).", grib::fmt_run(tf as i64));
                }
            } else if matches!(goal, Goal::Wrap { .. }) {
                status = "blocked".into();
                message = ROUND_BLOCKED.into();
            } else if limits || barriers.is_some() {
                status = "blocked".into();
                message = "Every heading is blocked by land, shallow water, barriers or the routing limits (wind, waves, angles).".into();
            } else {
                status = "blocked".into();
                message = "Every heading is blocked by land or shallow water.".into();
            }
            break;
        }

        // Earliest arrival this step finishes the leg.
        if let Some((node, ta)) = cands.iter().filter_map(|(n, a)| a.map(|ta| (n, ta))).min_by(|a, b| a.1.total_cmp(&b.1)) {
            let parent = node.parent;
            let arrive_pt = interpolate_arrival(&nodes[parent], node, &goal);
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
            let mut best: std::collections::HashMap<(i32, i32, i32), (f64, Node), CellBuild> = std::collections::HashMap::default();
            for (n, _) in cands {
                let key = key3(n.lat, n.lon, n.wrap);
                let parent = nodes[n.parent];
                let stay = key == key3(parent.lat, parent.lon, parent.wrap);
                if visited.contains(&key) && !stay {
                    continue;
                }
                // Going round a mark in a working breeze: still in its cell and hardly moved
                // (a tack eating the step, head to wind), its parent has already done better.
                // Elsewhere, and in light air anywhere, it stays: a front in a calm waits for wind.
                if wrapping
                    && stay
                    && n.tws >= 4.0
                    && parent.parent != usize::MAX
                    && LatLon::new(parent.lat, parent.lon).distance_to(&LatLon::new(n.lat, n.lon)) < 0.1 * cell_nm
                {
                    continue;
                }
                let ll = LatLon::new(n.lat, n.lon);
                if start.distance_to(&ll) + ll.distance_to(&dest) > detour_limit {
                    continue;
                }
                // the point nearest the mark; going round one, the one with the least
                // way left: to the mark, then round it at its clearance
                let dd = ll.distance_to(&dest) + if wrapping { (wrap_need - n.wrap).max(0.0).to_radians() * wrap_r } else { 0.0 };
                if best.get(&key).is_none_or(|(bd, _)| dd < *bd) {
                    best.insert(key, (dd, n));
                }
            }
            let fresh = best.keys().filter(|k| !visited.contains(*k)).count();
            // a trickle of new cells along an edge doesn't count as getting anywhere
            stale_s = if fresh <= (visited.len() / 1000).max(2) { stale_s + step_s } else { 0.0 };
            for k in best.keys() {
                visited.insert(*k);
                reached.entry((k.0, k.1)).or_insert(step as u32 + 1);
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
        // going round a mark: how far round the front has got, and when that last grew
        if let Goal::Wrap { .. } = goal {
            let hi = front.iter().map(|&i| nodes[i].wrap).fold(f64::MIN, f64::max);
            if hi > best_wrap.0 + 1.0 {
                best_wrap = (hi, 0.0);
            } else {
                best_wrap.1 += step_s;
            }
        }
        // (only when going round a mark: elsewhere a front stuck in light air waits for wind)
        // the water near the mark is small: an hour with nowhere new reached means the
        // front has filled all it can
        if wrapping && (stale_s >= 3600.0 || best_wrap.1 >= 12.0 * 3600.0) {
            status = "blocked".into();
            message = ROUND_BLOCKED.into();
            break;
        }
        if !grid_mode && p.keep_isochrones_every > 0 && step % p.keep_isochrones_every == 0 {
            res.isochrones.push(front_lines(&nodes, &front, start, obstacles));
        }
    }

    let _ = no_wind;
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
                if n.twa.abs() < 90.0 {
                    tacks += 1
                } else {
                    gybes += 1
                }
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

fn interpolate_arrival(a: &Node, b: &Node, goal: &Goal) -> LatLon {
    let pa = LatLon::new(a.lat, a.lon);
    let pb = LatLon::new(b.lat, b.lon);
    let f = goal.fraction(pa, pb, a.wrap).unwrap_or(1.0);
    LatLon::new(a.lat + (b.lat - a.lat) * f, a.lon + norm180(b.lon - a.lon) * f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(pos: LatLon, rounding: Rounding) -> MarkSpec {
        MarkSpec { pos, rounding, radius_nm: Some(0.2) }
    }

    /// A mark on the way, then a finish to the west: rounding to port the route
    /// goes up the east side and over the top; to starboard it passes the west side.
    #[test]
    fn rounding_sides() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let m = start.destination(0.0, 6.0);
        let fin = m.destination(270.0, 4.0);
        let wind = SteadyWind { tws: 10.0, twd: 90.0 }; // easterly: reaching north, running west
        let p = Params { step_s: 300.0, ..Params::default() };
        let run = |r: Rounding| route_marks(start, 0, &[spec(m, r), MarkSpec::at(fin)], &pol, &wind, None, &Barriers::default(), &p);
        for (r, east) in [(Rounding::Port, true), (Rounding::Starboard, false)] {
            let res = run(r);
            assert_eq!(res.status, "arrived", "{:?}: {}", r, res.message);
            let leg1_end = res.legs[0].arrival.unwrap();
            let pt = res.points.iter().find(|q| q.t == leg1_end).copied().unwrap_or(*res.points.iter().min_by_key(|q| (q.t - leg1_end).abs()).unwrap());
            let dx = (pt.lon - m.lon) * 60.0 * m.lat.to_radians().cos();
            let dy = (pt.lat - m.lat) * 60.0;
            let _ = dy;
            if east {
                // to port the course turns round the mark: up the east side, over the top,
                // and leg 1 ends on the west side, on the line to the finish
                assert!(dx < -0.19, "{r:?}: leg 1 ends {dx:.2} NM east of the mark");
                assert!(res.points.iter().any(|q| (q.lon - m.lon) * 60.0 * m.lat.to_radians().cos() > 0.19 && (q.lat - m.lat).abs() * 60.0 < 0.5));
                assert!(res.points.iter().any(|q| (q.lat - m.lat) * 60.0 > 0.19 && (q.lon - m.lon).abs() * 60.0 < 0.5));
            } else {
                // to starboard the mark is outside the turn: passing it on the west side is enough
                assert!(dx < -0.19, "{r:?}: leg 1 ends {dx:.2} NM east of the mark");
            }
            // no point of the route within the clearance of the mark
            for q in &res.points {
                let d = LatLon::new(q.lat, q.lon).distance_to(&m);
                assert!(d > 0.15, "{r:?}: passes {d:.2} NM from the mark");
            }
        }
        // through: the route just touches the arrival circle
        let res = run(Rounding::Through);
        assert_eq!(res.status, "arrived");
    }

    /// Out to a mark and back: the route goes right round it (over the top), on
    /// the side asked, rather than touching the line abeam and turning back.
    #[test]
    fn rounds_and_returns() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let m = start.destination(0.0, 6.0);
        let fin = start.destination(0.0, 0.5);
        let wind = SteadyWind { tws: 10.0, twd: 90.0 }; // beam reach both ways
        let p = Params { step_s: 300.0, ..Params::default() };
        let k = m.lat.to_radians().cos();
        for (r, west_in) in [(Rounding::Starboard, true), (Rounding::Port, false)] {
            let res = route_marks(start, 0, &[spec(m, r), MarkSpec::at(fin)], &pol, &wind, None, &Barriers::default(), &p);
            assert_eq!(res.status, "arrived", "{r:?}: {}", res.message);
            assert_eq!(res.legs.len(), 2);
            let north = res.points.iter().map(|q| (q.lat - m.lat) * 60.0).fold(f64::MIN, f64::max);
            assert!(north > 0.15, "{r:?}: gets only {north:.2} NM north of the mark");
            // the route goes all the way round: the bearing from the mark turns a
            // full circle between start and finish (both south of it), clockwise to
            // starboard, anticlockwise to port
            let leg1: Vec<_> = res.points.iter().collect();
            let turn: f64 =
                leg1.windows(2).map(|w| norm180(m.bearing_to(&LatLon::new(w[1].lat, w[1].lon)) - m.bearing_to(&LatLon::new(w[0].lat, w[0].lon)))).sum();
            let _ = k;
            if west_in {
                assert!((turn - 360.0).abs() < 25.0, "{r:?}: turns {turn:.0}")
            } else {
                assert!((turn + 360.0).abs() < 25.0, "{r:?}: turns {turn:.0}")
            }
            for q in &res.points {
                assert!(LatLon::new(q.lat, q.lon).distance_to(&m) > 0.15, "{r:?}: cuts the mark");
            }
        }
    }

    /// A mark joined to the land on one side can't be rounded that way: the
    /// routing says so instead of touching it and turning back.
    #[test]
    fn rounding_closed_off_fails() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let m = start.destination(0.0, 6.0);
        let fin = start.destination(0.0, 0.5);
        let wind = SteadyWind { tws: 10.0, twd: 90.0 };
        let p = Params { step_s: 300.0, ..Params::default() };
        // a long spit of land running east from the mark
        let d = 0.05 / 60.0;
        let spit =
            Barriers { zones: vec![vec![[m.lon, m.lat - d], [m.lon + 3.0, m.lat - d], [m.lon + 3.0, m.lat + d], [m.lon, m.lat + d]]], ..Default::default() };
        let clock = std::time::Instant::now();
        let res = route_marks(start, 0, &[spec(m, Rounding::Starboard), MarkSpec::at(fin)], &pol, &wind, None, &spit, &p);
        assert_eq!(res.status, "blocked", "{}", res.message);
        assert!(res.message.contains("no way round"), "{}", res.message);
        assert!(clock.elapsed().as_secs() < 20);
    }

    /// A calm at the start, a breeze later: the front waits for the wind rather
    /// than giving up (the start in a harbour at dusk), also round a mark.
    #[test]
    fn waits_out_a_calm() {
        struct Calm;
        impl WindSource for Calm {
            fn wind(&self, _: f64, _: f64, t: i64) -> Option<(f64, f64)> {
                Some(if t < 3 * 3600 { (0.3, 90.0) } else { (10.0, 90.0) })
            }
        }
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let m = start.destination(0.0, 6.0);
        let fin = start.destination(0.0, 0.5);
        let p = Params { step_s: 300.0, ..Params::default() };
        // a channel: barriers either side of the first two miles
        let w = 0.15 / 60.0;
        let walls = Barriers {
            lines: vec![
                vec![[start.lon - w, start.lat - 0.01], [start.lon - w, start.lat + 0.03]],
                vec![[start.lon + w, start.lat - 0.01], [start.lon + w, start.lat + 0.03]],
            ],
            ..Default::default()
        };
        let res = route_marks(start, 0, &[spec(m, Rounding::Starboard), MarkSpec::at(fin)], &pol, &Calm, None, &walls, &p);
        assert_eq!(res.status, "arrived", "{}", res.message);
    }

    /// An island mark with a blind inlet on the side towards the next mark: the
    /// route goes round and on, not up the inlet to touch the line to the next mark.
    #[test]
    fn no_detour_up_a_blind_inlet() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let m = start.destination(0.0, 6.0);
        let fin = m.destination(260.0, 8.0);
        let wind = SteadyWind { tws: 10.0, twd: 0.0 };
        let p = Params { step_s: 300.0, ..Params::default() };
        let k = m.lat.to_radians().cos();
        let (dx, dy) = (0.15 / 60.0 / k, 0.15 / 60.0); // island half-size, degrees
        let (w, n) = (0.6 / 60.0 / k, 0.4 / 60.0); // inlet: 0.6 NM deep west of it, walls to 0.4 NM north
        let island = vec![[m.lon - dx, m.lat - dy], [m.lon + dx, m.lat - dy], [m.lon + dx, m.lat + dy], [m.lon - dx, m.lat + dy]];
        // north wall from the island west, then the west wall south: open to the south
        let walls = vec![
            vec![[m.lon - dx, m.lat + dy * 0.5], [m.lon - dx - w, m.lat + dy * 0.5]],
            vec![[m.lon - dx - w, m.lat + dy * 0.5], [m.lon - dx - w, m.lat - n]],
        ];
        let b = Barriers { zones: vec![island], lines: walls, ..Default::default() };
        let mut mk = spec(m, Rounding::Starboard);
        mk.radius_nm = Some(0.25);
        let res = route_marks(start, 0, &[mk, MarkSpec::at(fin)], &pol, &wind, None, &b, &p);
        assert_eq!(res.status, "arrived", "{}", res.message);
        let in_inlet = res.points.iter().filter(|q| q.lon < m.lon - dx && q.lon > m.lon - dx - w && q.lat < m.lat + dy * 0.5 && q.lat > m.lat - dy).count();
        assert_eq!(in_inlet, 0, "the route goes up the inlet");
    }

    /// A mark that is the island itself can be rounded, but not passed through.
    #[test]
    fn round_an_island_mark() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let m = start.destination(0.0, 6.0);
        let fin = m.destination(270.0, 4.0);
        let wind = SteadyWind { tws: 10.0, twd: 90.0 };
        let p = Params { step_s: 300.0, ..Params::default() };
        let d = 0.3 / 60.0; // a small island round the mark
        let island = Barriers {
            zones: vec![vec![[m.lon - d, m.lat - d], [m.lon + d, m.lat - d], [m.lon + d, m.lat + d], [m.lon - d, m.lat + d]]],
            ..Default::default()
        };
        let mut round = spec(m, Rounding::Port);
        round.radius_nm = Some(0.5);
        let r = route_marks(start, 0, &[round, MarkSpec::at(fin)], &pol, &wind, None, &island, &p);
        assert_eq!(r.status, "arrived", "{}", r.message);
        let r2 = route_marks(start, 0, &[MarkSpec::at(m), MarkSpec::at(fin)], &pol, &wind, None, &island, &p);
        assert_eq!(r2.status, "mark blocked");
        // passing through it with a radius that reaches open water is fine
        let mut wide = MarkSpec::at(m);
        wide.radius_nm = Some(0.6);
        let r3 = route_marks(start, 0, &[wide, MarkSpec::at(fin)], &pol, &wind, None, &island, &p);
        assert_eq!(r3.status, "arrived", "{}", r3.message);
    }

    /// A fence across the direct line makes the route go round its end.
    #[test]
    fn barrier_line_and_limits() {
        let pol = test_polar();
        let start = LatLon::new(-41.0, 173.0);
        let dest = start.destination(90.0, 10.0);
        let wind = SteadyWind { tws: 10.0, twd: 0.0 };
        let p = Params { step_s: 300.0, ..Params::default() };
        let free = route_marks(start, 0, &[MarkSpec::at(dest)], &pol, &wind, None, &Barriers::default(), &p);
        let mid = start.destination(90.0, 5.0);
        let fence = Barriers { lines: vec![vec![[mid.lon, mid.lat - 0.05], [mid.lon, mid.lat + 0.2]]], ..Default::default() };
        let r = route_marks(start, 0, &[MarkSpec::at(dest)], &pol, &wind, None, &fence, &p);
        assert_eq!(r.status, "arrived", "{}", r.message);
        assert!(r.distance_nm > free.distance_nm + 0.5, "{} vs {}", r.distance_nm, free.distance_nm);
        assert!(r.points.iter().any(|q| q.lat < mid.lat - 0.05), "goes round the south end");
        // a beam reach isn't allowed with max TWA 60: no way east
        let lim = Params { max_twa: 60.0, ..p.clone() };
        let r2 = route_marks(start, 0, &[MarkSpec::at(dest)], &pol, &wind, None, &Barriers::default(), &lim);
        assert!(r2.distance_nm > free.distance_nm + 1.0 || r2.status != "arrived", "{} {}", r2.status, r2.distance_nm);
        // too much wind everywhere: stuck after the first step
        let r3 = route_marks(start, 0, &[MarkSpec::at(dest)], &pol, &wind, None, &Barriers::default(), &Params { max_tws: 8.0, ..p.clone() });
        assert_eq!(r3.status, "blocked", "{}", r3.message);
    }

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
