// Live set and drift (H11): the current the boat is in, worked out from what it is doing over the
// ground (COG, SOG) against what it is doing through the water (heading, speed through the water).
// Ground vector − water vector = the current's vector. Averaged over the last minute, as one
// sample on its own is mostly the noise of the log, the compass and the GPS.
//
// Leeway is not allowed for, so under sail the "current" includes the boat's leeway. A paddle-wheel
// log that is not calibrated, or reads nothing under a knot, shows as a false current along the heading.

const rad = (d) => (d * Math.PI) / 180;
const num = (x) => typeof x === "number" && Number.isFinite(x);

/** A set-and-drift tracker: add(tMs, v) with v = { sog, cog, stw, heading } (kn, °T); value(tMs) → { set, drift, n } or null. */
export function setDriftTracker({ windowMs = 60_000, minSamples = 5, minSpanMs = 10_000 } = {}) {
  let s = [];
  return {
    add(t, v) {
      if (!v || !num(v.sog) || !num(v.cog) || !num(v.stw) || !num(v.heading)) return;
      if (s.length && t - s[s.length - 1].t < 900) return; // at most about one a second
      const gx = v.sog * Math.sin(rad(v.cog)), gy = v.sog * Math.cos(rad(v.cog));
      const wx = v.stw * Math.sin(rad(v.heading)), wy = v.stw * Math.cos(rad(v.heading));
      s.push({ t, x: gx - wx, y: gy - wy });
      while (s.length && s[0].t < t - windowMs) s.shift();
    },
    value(t) {
      const w = s.filter((q) => q.t >= t - windowMs && q.t <= t + 1000);
      if (w.length < minSamples || w[w.length - 1].t - w[0].t < minSpanMs) return null;
      let x = 0, y = 0;
      for (const q of w) { x += q.x; y += q.y; }
      x /= w.length; y /= w.length;
      const drift = Math.hypot(x, y);
      // the set is where the current flows to
      const set = ((Math.atan2(x, y) * 180) / Math.PI + 360) % 360;
      return { set, drift, n: w.length };
    },
    clear() { s = []; },
  };
}

/** The one tracker the readings, the history and the chart vector share. */
export const setDrift = setDriftTracker();
