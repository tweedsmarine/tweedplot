// Instrument history, the part without a screen: what is recorded, how it is kept small,
// and the pressure tendency. history.js draws it and keeps it over a restart.
//
// Each reading is sampled once a second while the live data is fresh and kept two ways:
//  - one-minute bins (mean, lowest, highest; for a direction the vector mean) for as many
//    hours as the user keeps (24 to begin with): 1,440 bins a reading a day, so a day of
//    eleven readings is about 16,000 small records — cheap to keep, store and draw;
//  - five-second points for the last hour, so a short window shows the detail.
// A gap in the data stays a gap.

/** What can be recorded: key → { label, unit, get(v), dir: "true" (0–360) | "rel" (±180), dp }. */
export const CHANNELS = {
  TWS: { label: "TWS", unit: "kn", dp: 1, get: (v) => v.tws, tint: "t", band: "Gusts" },
  TWD: { label: "TWD", unit: "°T", dp: 0, get: (v) => v.twd, dir: "true", tint: "t" },
  AWS: { label: "AWS", unit: "kn", dp: 1, get: (v) => v.aws, tint: "a", band: "Range" },
  AWA: { label: "AWA", unit: "°", dp: 0, get: (v) => v.awa, dir: "rel", tint: "a" },
  SOG: { label: "SOG", unit: "kn", dp: 1, get: (v) => v.sog },
  BSP: { label: "BSP", unit: "kn", dp: 1, get: (v) => v.stw },
  PRES: { label: "Pressure", unit: "hPa", dp: 1, get: (v) => v.pressure, tendency: true },
  AIRT: { label: "Air temp", unit: "°C", dp: 1, get: (v) => v.air_temp },
  HUM: { label: "Humidity", unit: "%", dp: 0, get: (v) => v.humidity },
  WTEMP: { label: "Water temp", unit: "°C", dp: 1, get: (v) => v.water_temp },
  DPT: { label: "Depth", unit: "m", dp: 1, get: (v) => v.depth ?? v.depth_surface },
  DRIFT: { label: "Drift", unit: "kn", dp: 1, get: (v) => v.drift },
  SET: { label: "Set", unit: "°T", dp: 0, get: (v) => v.set, dir: "true" },
};

export const MIN_MS = 60_000;
export const RAW_MS = 5_000;
export const RAW_KEEP_MS = 3600e3;
export const KEEP_HOURS = [6, 12, 24, 48, 72];

const rad = (d) => (d * Math.PI) / 180;
const wrap360 = (d) => ((d % 360) + 360) % 360;
const wrap180 = (d) => ((((d + 180) % 360) + 360) % 360) - 180;

/** A running mean/min/max (or vector mean for a direction) of the samples in one bin. */
function acc() { return { n: 0, s: 0, lo: Infinity, hi: -Infinity, sx: 0, sy: 0 }; }
function add(a, x, dir) {
  a.n++;
  if (dir) { a.sx += Math.sin(rad(x)); a.sy += Math.cos(rad(x)); } else { a.s += x; if (x < a.lo) a.lo = x; if (x > a.hi) a.hi = x; }
}
function mean(a, dir) {
  if (!a.n) return null;
  if (!dir) return a.s / a.n;
  const d = (Math.atan2(a.sx, a.sy) * 180) / Math.PI;
  return dir === "rel" ? wrap180(d) : wrap360(d);
}

export class Recorder {
  constructor({ keepHours = 24 } = {}) {
    this.keepHours = keepHours;
    this.bins = {}; // key → [{ t (ms, start of minute), m, lo, hi }]
    this.raw = {}; // key → [{ t (ms, start of 5 s), m }]
    this.cur = {}; // key → { min: {t, a}, raw: {t, a} }
    this.lastSample = 0;
    for (const k of Object.keys(CHANNELS)) { this.bins[k] = []; this.raw[k] = []; }
  }

  /** One sample of every reading the boat has (call at least once a second; extra calls are skipped). */
  sample(v, now = Date.now()) {
    if (now - this.lastSample < 1000) return false;
    this.lastSample = now;
    let closed = false;
    for (const [k, ch] of Object.entries(CHANNELS)) {
      let x = null;
      try { x = ch.get(v ?? {}); } catch { x = null; }
      const c = (this.cur[k] ??= {});
      const tm = Math.floor(now / MIN_MS) * MIN_MS, tr = Math.floor(now / RAW_MS) * RAW_MS;
      if (c.min && c.min.t !== tm) { closed = this.#close(k, "min") || closed; }
      if (c.raw && c.raw.t !== tr) this.#close(k, "raw");
      if (x == null || !Number.isFinite(x)) continue;
      c.min ??= { t: tm, a: acc() };
      c.raw ??= { t: tr, a: acc() };
      add(c.min.a, x, ch.dir);
      add(c.raw.a, x, ch.dir);
    }
    this.trim(now);
    return closed;
  }

  #close(k, which) {
    const c = this.cur[k], b = c[which];
    c[which] = null;
    const dir = CHANNELS[k].dir;
    const m = mean(b.a, dir);
    if (m == null) return false;
    if (which === "min") this.bins[k].push(dir ? { t: b.t, m } : { t: b.t, m, lo: b.a.lo, hi: b.a.hi });
    else this.raw[k].push({ t: b.t, m });
    return which === "min";
  }

  trim(now = Date.now()) {
    const cut = now - this.keepHours * 3600e3, rcut = now - RAW_KEEP_MS;
    for (const k of Object.keys(CHANNELS)) {
      const b = this.bins[k];
      let i = 0;
      while (i < b.length && b[i].t < cut) i++;
      if (i) b.splice(0, i);
      const r = this.raw[k];
      i = 0;
      while (i < r.length && r[i].t < rcut) i++;
      if (i) r.splice(0, i);
    }
  }

  /** Has this reading ever been recorded (or is it coming in now)? */
  has(k) { return this.bins[k]?.length > 0 || this.raw[k]?.length > 0 || !!this.cur[k]?.raw; }

  /** Points from `from` (ms) to now, oldest first: { t, m, lo?, hi? }, five-second points
   *  where there are any and the window is short, minute bins before that. */
  series(k, from, now = Date.now()) {
    const span = now - from;
    const out = [];
    const raw = span <= 2 * RAW_KEEP_MS ? this.raw[k] : [];
    const rawFrom = raw.length ? raw[0].t : Infinity;
    for (const b of this.bins[k]) if (b.t >= from - MIN_MS && b.t + MIN_MS <= rawFrom) out.push(b);
    for (const r of raw) if (r.t >= from - RAW_MS) out.push({ ...r, raw: true });
    // the bin being filled, so the line reaches now
    const c = this.cur[k];
    const live = raw.length ? c?.raw : c?.min;
    if (live?.a.n) {
      const m = mean(live.a, CHANNELS[k].dir);
      out.push(raw.length ? { t: live.t, m, raw: true } : CHANNELS[k].dir ? { t: live.t, m } : { t: live.t, m, lo: live.a.lo, hi: live.a.hi });
    }
    return out;
  }

  /** The latest value of a reading (the bin being filled, else the last kept). */
  latest(k) {
    const c = this.cur[k];
    if (c?.raw?.a.n) return mean(c.raw.a, CHANNELS[k].dir);
    return this.raw[k].at(-1)?.m ?? this.bins[k].at(-1)?.m ?? null;
  }

  /** Change in a reading over the last `hours` (from minute bins): null without data near both ends. */
  change(k, hours = 3, now = Date.now()) {
    const b = this.bins[k];
    if (b.length < 2) return null;
    const end = b.at(-1);
    if (now - end.t > 10 * MIN_MS) return null;
    const want = end.t - hours * 3600e3;
    let start = null;
    for (const x of b) { if (x.t >= want) { start = x; break; } }
    if (!start || start.t - want > 15 * MIN_MS || end.t - start.t < 0.75 * hours * 3600e3) return null;
    return ((end.m - start.m) * hours * 3600e3) / (end.t - start.t);
  }

  // ---------- kept small for storage ----------
  /** Minute bins as compact columns: { v, keep, t0, ch: { k: [[dt min, m×10, lo×10, hi×10], …] } }. */
  toJSON() {
    let t0 = Infinity;
    for (const k of Object.keys(CHANNELS)) if (this.bins[k].length) t0 = Math.min(t0, this.bins[k][0].t);
    const ch = {};
    if (t0 !== Infinity) {
      for (const k of Object.keys(CHANNELS)) {
        if (!this.bins[k].length) continue;
        ch[k] = this.bins[k].map((b) => (b.lo == null ? [(b.t - t0) / MIN_MS, Math.round(b.m * 10)]
          : [(b.t - t0) / MIN_MS, Math.round(b.m * 10), Math.round(b.lo * 10), Math.round(b.hi * 10)]));
      }
    }
    return { v: 1, keep: this.keepHours, t0: t0 === Infinity ? 0 : t0, ch };
  }

  /** Replace the minute bins with stored ones (what the main window saved). */
  load(o, now = Date.now()) {
    if (!o || o.v !== 1 || !o.ch) return false;
    for (const k of Object.keys(CHANNELS)) {
      const rows = o.ch[k];
      if (!Array.isArray(rows)) { this.bins[k] = []; continue; }
      const list = [];
      for (const r of rows) {
        if (!Array.isArray(r) || !Number.isFinite(r[0]) || !Number.isFinite(r[1])) continue;
        const t = o.t0 + r[0] * MIN_MS;
        if (t > now) continue;
        list.push(r.length >= 4 ? { t, m: r[1] / 10, lo: r[2] / 10, hi: r[3] / 10 } : { t, m: r[1] / 10 });
      }
      this.bins[k] = list;
    }
    this.trim(now);
    return true;
  }
}

/** Shipping-forecast words for a pressure change over three hours (hPa). */
export function tendencyWords(d3) {
  if (d3 == null) return "";
  const a = Math.abs(d3);
  if (a < 0.1) return "Steady";
  const way = d3 > 0 ? "Rising" : "Falling";
  if (a <= 1.5) return `${way} slowly`;
  if (a <= 3.5) return way;
  if (a <= 6) return `${way} quickly`;
  return `${way} very rapidly`;
}

/** Undo the 360° wrap of a direction series so a veer through north is a smooth line. */
export function unwrap(values) {
  const out = [];
  let prev = null;
  for (const v of values) {
    if (v == null) { out.push(null); continue; }
    if (prev == null) { out.push(v); prev = v; continue; }
    const x = prev + wrap180(v - prev);
    out.push(x);
    prev = x;
  }
  return out;
}

export { wrap360, wrap180 };
