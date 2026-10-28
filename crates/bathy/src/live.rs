//! Live soundings: depths from the sounder as they come in, a fix a second at most, added a
//! batch at a time (about a minute) with the same checks as tracks. Each batch is checked
//! with the last few fixes of the one before it, so a spike or a jump at the join is seen.

use crate::check::{check_segment, Verdict};
use crate::{hours, Result, Writer, SRC_OWN, WHY_DEEP, WHY_RUN, WHY_SHALLOW, WHY_SPIKE};

/// A sounding: time (Unix s), position, depth below the surface (m, positive down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub t: f64,
    pub lon: f64,
    pub lat: f64,
    pub depth: f64,
}

/// Fixes kept from the end of one batch to check the next with.
const CONTEXT: usize = 10;
/// A gap (s) or a jump (m) between fixes that starts afresh: the checks don't look across it.
const GAP_S: f64 = 30.0;
const JUMP_M: f64 = 500.0;

/// Collects soundings and hands them over in batches.
#[derive(Debug)]
pub struct Logger {
    /// Least time between soundings kept (s).
    pub min_interval_s: f64,
    /// How long a batch is (s).
    pub batch_s: f64,
    buf: Vec<Fix>,
    context: Vec<Fix>,
    last_t: f64,
}

impl Default for Logger {
    fn default() -> Self {
        Logger { min_interval_s: 1.0, batch_s: 60.0, buf: Vec::new(), context: Vec::new(), last_t: f64::NEG_INFINITY }
    }
}

impl Logger {
    /// Offer a sounding; true if kept (not within `min_interval_s` of the last, depth real).
    pub fn offer(&mut self, f: Fix) -> bool {
        if !(f.depth.is_finite() && f.depth > 0.3 && f.depth < 11_000.0) || !f.lon.is_finite() || !f.lat.is_finite() {
            return false;
        }
        if f.t - self.last_t < self.min_interval_s && f.t >= self.last_t {
            return false;
        }
        self.last_t = f.t;
        self.buf.push(f);
        true
    }

    /// Soundings waiting.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// A batch to add, when the waiting soundings span `batch_s` (or any, with `force`):
    /// (context, new). The context is the end of the batch before, unless there's a gap.
    pub fn take(&mut self, force: bool) -> Option<(Vec<Fix>, Vec<Fix>)> {
        let span = match (self.buf.first(), self.buf.last()) {
            (Some(a), Some(b)) => b.t - a.t,
            _ => return None,
        };
        if !force && span < self.batch_s {
            return None;
        }
        let new = std::mem::take(&mut self.buf);
        let mut ctx = std::mem::take(&mut self.context);
        if let (Some(a), Some(b)) = (ctx.last(), new.first()) {
            if apart(a, b) {
                ctx.clear();
            }
        }
        self.context = new[new.len().saturating_sub(CONTEXT)..].to_vec();
        Some((ctx, new))
    }
}

fn metres(a: &Fix, b: &Fix) -> f64 {
    let k = ((a.lat + b.lat) / 2.0).to_radians().cos();
    let (dx, dy) = ((b.lon - a.lon) * k * 111_320.0, (b.lat - a.lat) * 111_320.0);
    (dx * dx + dy * dy).sqrt()
}

fn apart(a: &Fix, b: &Fix) -> bool {
    (b.t - a.t).abs() > GAP_S || metres(a, b) > JUMP_M
}

/// Check a batch and add it: what passes to the seabed, the rest to the left-out soundings
/// with why. `chart` as [`check_segment`]. Returns (added, left out).
pub fn add_batch(w: &mut Writer, context: &[Fix], new: &[Fix], chart: &dyn Fn(f64, f64) -> Option<(f32, f32)>) -> Result<(i64, i64)> {
    let all: Vec<Fix> = context.iter().chain(new).copied().collect();
    let (mut added, mut left) = (0, 0);
    // pieces between gaps, each checked on its own
    let mut start = 0;
    for i in 1..=all.len() {
        if i < all.len() && !apart(&all[i - 1], &all[i]) {
            continue;
        }
        let piece = &all[start..i];
        let fixes: Vec<(f64, f64, Option<f64>)> = piece.iter().map(|f| (f.lon, f.lat, Some(f.depth))).collect();
        let (v, _, _) = check_segment(&fixes, chart);
        for (k, (f, verdict)) in piece.iter().zip(v).enumerate() {
            if start + k < context.len() {
                continue; // added with the batch before
            }
            let h = hours(f.t as i64);
            let z = -(f.depth as f32);
            let why = match verdict {
                Verdict::Keep => {
                    w.add(f.lon, f.lat, z, h, SRC_OWN)?;
                    added += 1;
                    continue;
                }
                Verdict::None => continue,
                Verdict::Spike => WHY_SPIKE,
                Verdict::Echo => WHY_SPIKE | WHY_DEEP,
                Verdict::OffDeep => WHY_DEEP,
                Verdict::Run => WHY_RUN,
                Verdict::OffChart => WHY_SHALLOW,
            };
            w.reject(f.lon, f.lat, z, h, SRC_OWN | why);
            left += 1;
        }
        start = i;
    }
    Ok((added, left))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use std::sync::atomic::AtomicBool;

    fn fix(t: f64, d: f64) -> Fix {
        Fix { t, lon: 173.2 + t * 0.00005, lat: -41.2, depth: d }
    }

    #[test]
    fn batches_of_a_minute_with_context() {
        let mut l = Logger::default();
        // 5 Hz from the sounder: one a second kept
        for i in 0..400 {
            l.offer(fix(i as f64 * 0.2, 50.0));
        }
        assert_eq!(l.pending(), 80);
        let (ctx, new) = l.take(false).unwrap();
        assert!(ctx.is_empty() && new.len() == 80);
        assert!(l.take(false).is_none());
        for i in 80..200 {
            l.offer(fix(i as f64, 50.0));
        }
        let (ctx, new) = l.take(false).unwrap();
        assert_eq!((ctx.len(), new.len()), (10, 120));
        // after a gap the context is dropped
        l.offer(fix(1000.0, 50.0));
        let (ctx, _) = l.take(true).unwrap();
        assert!(ctx.is_empty());
        assert!(!l.offer(Fix { depth: f64::NAN, ..fix(2000.0, 0.0) }));
    }

    #[test]
    fn a_batch_is_checked_and_added() {
        let dir = std::env::temp_dir().join(format!("bathy-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("bathy.sqlite")).unwrap();
        let mut l = Logger::default();
        // 60 s at ~4 m/s, 50 m deep, one second lost-bottom blip at 3 m
        for i in 0..61 {
            l.offer(fix(i as f64, if i == 30 { 3.0 } else { 50.0 + (i % 2) as f64 * 0.2 }));
        }
        let (ctx, new) = l.take(false).unwrap();
        let mut w = s.writer("", "", 0).unwrap();
        let (a, r) = add_batch(&mut w, &ctx, &new, &|_, _| None).unwrap();
        assert_eq!((a, r), (60, 1));
        w.finish(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        let i = s.info().unwrap();
        assert_eq!(i.rejected, 1);
        assert!(i.cells > 50);
        // the next batch: its context isn't added twice
        for i in 61..130 {
            l.offer(fix(i as f64, 50.0));
        }
        let (ctx, new) = l.take(false).unwrap();
        let mut w = s.writer("", "", 0).unwrap();
        let (a, _) = add_batch(&mut w, &ctx, &new, &|_, _| None).unwrap();
        assert_eq!(a, new.len() as i64);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
