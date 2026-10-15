// Damping of Signal K readings: a moving average over the last N seconds.
// Angles are averaged as vectors (so 359° and 1° average to 0°, not 180°).
// Position is never damped. N = 0 passes readings straight through.

/** Which fields are damped together, and how. */
const GROUPS = {
  // course and speed
  motion: { linear: ["sog", "stw"], bearing: ["cog", "heading", "heading_mag"], signed: [] },
  // wind
  wind: { linear: ["aws", "tws"], bearing: ["twd"], signed: ["awa", "twa"] },
  // depth
  depth: { linear: ["depth", "depth_keel", "depth_surface"], bearing: [], signed: [] },
};

export function createDamping(getSeconds) {
  const buf = []; // { t (ms), v }
  return {
    /** Add a reading; returns the damped copy. */
    push(v, now = Date.now()) {
      buf.push({ t: now, v });
      const keep = Math.max(...Object.keys(GROUPS).map((g) => getSeconds(g))) * 1000;
      while (buf.length > 1 && now - buf[0].t > keep) buf.shift();
      const out = { ...v };
      for (const [g, f] of Object.entries(GROUPS)) {
        const s = getSeconds(g);
        if (!(s > 0)) continue;
        const win = buf.filter((x) => now - x.t <= s * 1000);
        for (const k of f.linear) out[k] = mean(win, k);
        for (const k of f.bearing) out[k] = angleMean(win, k, false);
        for (const k of f.signed) out[k] = angleMean(win, k, true);
      }
      return out;
    },
    clear() { buf.length = 0; },
  };
}

function mean(win, k) {
  let s = 0, n = 0;
  for (const { v } of win) if (Number.isFinite(v[k])) { s += v[k]; n++; }
  return n ? s / n : win.at(-1)?.v[k] ?? null;
}

function angleMean(win, k, signed) {
  let x = 0, y = 0, n = 0;
  for (const { v } of win) {
    if (!Number.isFinite(v[k])) continue;
    const r = (v[k] * Math.PI) / 180;
    x += Math.cos(r); y += Math.sin(r); n++;
  }
  if (!n) return win.at(-1)?.v[k] ?? null;
  let a = (Math.atan2(y, x) * 180) / Math.PI;
  if (!signed) a = (a + 360) % 360;
  return a;
}
