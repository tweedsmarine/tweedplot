//! Simulated radar, to test the path a real radar would take: spokes made in Rust
//! at a real radar's rate, sent to the web view as raw bytes over a Tauri channel
//! (no JSON), drawn there by a WebGL layer. A real radar replaces only `spoke()`
//! with packets decoded from the scanner on the network.
//!
//! Packet: u32 first spoke index, u16 spoke count, u16 samples per spoke,
//! u16 spokes per turn, u16 spare, then count × samples bytes (0..255 echo strength).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};

static RUNNING: AtomicBool = AtomicBool::new(false);
static GEN: AtomicU64 = AtomicU64::new(0);
static SENT_BYTES: AtomicU64 = AtomicU64::new(0);
static SENT_SPOKES: AtomicU64 = AtomicU64::new(0);
static GEN_NS: AtomicU64 = AtomicU64::new(0);

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A moving ship: range (fraction of the radar range), bearing (rad), speed (range fraction/s), course (rad).
struct Ship {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

/// Echo strength of one sample at range fraction `r` (0..1) on the bearing whose sine and cosine are given (from north).
fn echo(r: f32, (sb, cb): (f32, f32), t: f32, ships: &[Ship], rng: &mut Rng) -> f32 {
    let (x, y) = (r * sb, r * cb); // east, north
    let mut v = rng.next() * 0.12; // receiver noise
    // sea clutter close in, patchy
    v += (0.55 * (-r * 9.0).exp()) * (0.6 + 0.4 * rng.next());
    // two islands and a stretch of coast (fixed on the chart)
    for &(cx, cy, rad) in &[(0.45f32, 0.35f32, 0.10f32), (-0.30, 0.55, 0.07)] {
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        if d < rad {
            v += 0.75 + 0.2 * rng.next();
        }
    }
    if y < -0.55 + 0.1 * (x * 6.0).sin() {
        v += 0.7 + 0.25 * rng.next();
    }
    // a rain cell drifting east
    let (rx, ry) = (-0.5 + 0.02 * t % 1.4, -0.1);
    let dr = ((x - rx).powi(2) + (y - ry).powi(2)).sqrt();
    if dr < 0.18 {
        v += (0.18 - dr) * 2.2 * rng.next();
    }
    // ships
    for s in ships {
        let d2 = (x - s.x).powi(2) + (y - s.y).powi(2);
        v += 0.9 * (-d2 / 0.00004).exp();
    }
    v.min(1.0)
}

#[tauri::command]
pub fn radar_sim_start(rpm: f64, spokes: u16, samples: u16, channel: Channel<InvokeResponseBody>) {
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    RUNNING.store(true, Ordering::SeqCst);
    SENT_BYTES.store(0, Ordering::Relaxed);
    SENT_SPOKES.store(0, Ordering::Relaxed);
    GEN_NS.store(0, Ordering::Relaxed);
    let (spokes, samples) = (spokes.clamp(256, 8192), samples.clamp(128, 2048));
    let rpm = rpm.clamp(6.0, 120.0);
    let channel = Arc::new(channel);
    std::thread::spawn(move || {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut ships: Vec<Ship> = (0..8)
            .map(|k| {
                let a = k as f32 * 0.8;
                Ship { x: 0.7 * a.cos() * (0.4 + 0.07 * k as f32), y: 0.7 * a.sin() * 0.6, vx: 0.004 * (a * 1.7).sin(), vy: 0.004 * (a * 1.3).cos() }
            })
            .collect();
        let per_sec = spokes as f64 * rpm / 60.0;
        let start = Instant::now();
        let mut next = 0u64; // spokes made so far
        let mut last = Instant::now();
        while RUNNING.load(Ordering::SeqCst) && GEN.load(Ordering::SeqCst) == gen {
            std::thread::sleep(Duration::from_millis(16));
            let el = start.elapsed().as_secs_f64();
            let due = (el * per_sec) as u64;
            if due <= next {
                continue;
            }
            let dt = last.elapsed().as_secs_f32();
            last = Instant::now();
            for s in ships.iter_mut() {
                s.x += s.vx * dt;
                s.y += s.vy * dt;
                if s.x.abs() > 0.95 || s.y.abs() > 0.95 {
                    s.vx = -s.vx;
                    s.vy = -s.vy;
                }
            }
            let count = (due - next).min(spokes as u64) as usize;
            let first = (next % spokes as u64) as u32;
            let t0 = Instant::now();
            let mut buf = Vec::with_capacity(12 + count * samples as usize);
            buf.extend_from_slice(&first.to_le_bytes());
            buf.extend_from_slice(&(count as u16).to_le_bytes());
            buf.extend_from_slice(&samples.to_le_bytes());
            buf.extend_from_slice(&spokes.to_le_bytes());
            buf.extend_from_slice(&0u16.to_le_bytes());
            for k in 0..count {
                let idx = (first as usize + k) % spokes as usize;
                let b = idx as f32 / spokes as f32 * std::f32::consts::TAU;
                let sc = (b.sin(), b.cos());
                for i in 0..samples {
                    let r = (i as f32 + 0.5) / samples as f32;
                    buf.push((echo(r, sc, el as f32, &ships, &mut rng) * 255.0) as u8);
                }
            }
            GEN_NS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
            next = due;
            SENT_BYTES.fetch_add(buf.len() as u64, Ordering::Relaxed);
            SENT_SPOKES.fetch_add(count as u64, Ordering::Relaxed);
            if channel.send(InvokeResponseBody::Raw(buf)).is_err() {
                break;
            }
        }
    });
}

#[tauri::command]
pub fn radar_sim_stop() {
    RUNNING.store(false, Ordering::SeqCst);
}

/// Totals since the start: bytes and spokes sent, and the time spent making them (ms).
#[tauri::command]
pub fn radar_sim_stats() -> serde_json::Value {
    serde_json::json!({
        "bytes": SENT_BYTES.load(Ordering::Relaxed),
        "spokes": SENT_SPOKES.load(Ordering::Relaxed),
        "gen_ms": GEN_NS.load(Ordering::Relaxed) as f64 / 1e6,
    })
}
