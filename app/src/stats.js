// Statistics of a routing, worked out from its points. Each point carries what
// the boat does FROM it until the next point (heading, TWA, TWS, BSP, SOG), so
// every segment is weighted by its duration.
//
// New statistics are added to STATS; the compare table and its settings pick
// them up by themselves.

import { fmt } from "./weather.js";

/** Points of sail by |TWA|, degrees. */
export const SAIL = { upwind: 70, downwind: 120 };

export function routingStats(r) {
  const pts = r.points ?? [];
  const t0 = r.legs?.[0]?.start ?? r.request?.t0 ?? pts[0]?.t;
  const end = r.status === "arrived" ? r.arrival : pts.at(-1)?.t;
  const s = {
    t0, end, status: r.status, elapsed: end != null && t0 != null ? end - t0 : null,
    distance: r.distance_nm, tacks: 0, gybes: 0,
    upwind: 0, reach: 0, downwind: 0, port: 0, stbd: 0,
    maxTws: null, minTws: null, avgTws: null, maxSog: null, avgBsp: null, maxBsp: null,
    toGo: null, steps: pts.length,
  };
  for (const l of r.legs ?? []) { s.tacks += l.tacks; s.gybes += l.gybes; }
  let total = 0, twsSum = 0, bspSum = 0;
  for (let i = 0; i + 1 < pts.length; i++) {
    const p = pts[i], dt = pts[i + 1].t - p.t;
    if (!(dt > 0)) continue;
    total += dt;
    const a = Math.abs(p.twa);
    if (a < SAIL.upwind) s.upwind += dt; else if (a > SAIL.downwind) s.downwind += dt; else s.reach += dt;
    if (p.twa < 0) s.port += dt; else s.stbd += dt;
    twsSum += p.tws * dt;
    bspSum += p.bsp * dt;
    s.maxTws = Math.max(s.maxTws ?? -Infinity, p.tws);
    s.minTws = Math.min(s.minTws ?? Infinity, p.tws);
    s.maxSog = Math.max(s.maxSog ?? -Infinity, p.sog);
    s.maxBsp = Math.max(s.maxBsp ?? -Infinity, p.bsp);
  }
  if (total > 0) {
    for (const k of ["upwind", "reach", "downwind", "port", "stbd"]) s[k] = (100 * s[k]) / total;
    s.avgTws = twsSum / total;
    s.avgBsp = bspSum / total;
  } else {
    for (const k of ["upwind", "reach", "downwind", "port", "stbd"]) s[k] = null;
  }
  s.avgSog = s.elapsed > 0 ? s.distance / (s.elapsed / 3600) : null;
  if (r.status !== "arrived" && pts.length && r.legs?.length) {
    const p = pts.at(-1), to = r.legs.at(-1).to;
    s.toGo = gcNm([p.lon, p.lat], to);
  }
  return s;
}

const f1 = (x) => x.toFixed(1);
const pc = (x) => `${Math.round(x)} %`;
const kn = (x) => `${x.toFixed(1)} kn`;
export function hm(sec) {
  const h = Math.floor(sec / 3600), m = Math.round((sec % 3600) / 60);
  return m === 60 ? `${h + 1} h 00` : `${h} h ${String(m).padStart(2, "0")}`;
}

/**
 * Columns for the compare table. get(stats, routing) → value or null;
 * best: "min" | "max" marks the best routing in the column; "progress": the most,
 * but only when no routing finished (then distance is progress; once they finish,
 * Elapsed and Finish show the winner and sailing further is no merit).
 */
export const STATS = [
  { key: "status", label: "Result", group: "Time", get: (s) => s.status, fmt: (x) => ({ arrived: "Arrived", "grib end": "GRIB ended" })[x] ?? x, def: true },
  { key: "t0", label: "Start", group: "Time", get: (s) => s.t0, fmt: (x) => fmt(x, true) },
  { key: "end", label: "Finish / end", group: "Time", get: (s) => s.end, fmt: (x) => fmt(x, true), best: "min", def: true },
  { key: "elapsed", label: "Elapsed", group: "Time", get: (s) => s.elapsed, fmt: hm, best: "min", def: true },
  { key: "toGo", label: "Still to go", group: "Time", get: (s) => s.toGo, fmt: (x) => `${f1(x)} NM`, best: "min" },
  { key: "distance", label: "Distance", group: "Distance and speed", get: (s) => s.distance, fmt: (x) => `${f1(x)} NM`, best: "progress", def: true },
  { key: "avgSog", label: "Avg SOG", group: "Distance and speed", get: (s) => s.avgSog, fmt: kn, best: "max", def: true },
  { key: "maxSog", label: "Max SOG", group: "Distance and speed", get: (s) => s.maxSog, fmt: kn },
  { key: "avgBsp", label: "Avg BSP", group: "Distance and speed", get: (s) => s.avgBsp, fmt: kn },
  { key: "maxBsp", label: "Max BSP", group: "Distance and speed", get: (s) => s.maxBsp, fmt: kn, def: true },
  { key: "avgTws", label: "Avg TWS", group: "Wind", get: (s) => s.avgTws, fmt: kn },
  { key: "maxTws", label: "Max TWS", group: "Wind", get: (s) => s.maxTws, fmt: kn, def: true },
  { key: "minTws", label: "Min TWS", group: "Wind", get: (s) => s.minTws, fmt: kn, def: true },
  { key: "upwind", label: `Upwind (<${SAIL.upwind}°)`, group: "Point of sail", get: (s) => s.upwind, fmt: pc, def: true },
  { key: "reach", label: "Reaching", group: "Point of sail", get: (s) => s.reach, fmt: pc, def: true },
  { key: "downwind", label: `Downwind (>${SAIL.downwind}°)`, group: "Point of sail", get: (s) => s.downwind, fmt: pc, def: true },
  { key: "port", label: "Port tack", group: "Point of sail", get: (s) => s.port, fmt: pc },
  { key: "stbd", label: "Stbd tack", group: "Point of sail", get: (s) => s.stbd, fmt: pc },
  { key: "tacks", label: "Tacks", group: "Manoeuvres", get: (s) => s.tacks, fmt: String, best: "min", def: true },
  { key: "gybes", label: "Gybes", group: "Manoeuvres", get: (s) => s.gybes, fmt: String, best: "min", def: true },
  { key: "grib", label: "GRIB", group: "Inputs", get: (s, r) => r.request?.grib_name ?? r.grib, fmt: String, def: true },
  { key: "polar", label: "Polar", group: "Inputs", get: (s, r) => (r.request?.polar_path ?? "").split(/[\\/]/).pop()?.replace(/\.[^.]+$/, ""), fmt: String },
  { key: "eff", label: "Efficiency", group: "Inputs", get: (s, r) => r.request?.params?.efficiency, fmt: (x) => `${Math.round(x * 100)} %` },
  { key: "step", label: "Time step", group: "Inputs", get: (s, r) => r.legs?.[0]?.step_s, fmt: (x) => (x >= 3600 ? `${+(x / 3600).toFixed(1)} h` : `${Math.round(x / 60)} min`) },
];

function gcNm(a, b) {
  const r = Math.PI / 180, [l1, p1] = [a[0] * r, a[1] * r], [l2, p2] = [b[0] * r, b[1] * r];
  const h = Math.sin((p2 - p1) / 2) ** 2 + Math.cos(p1) * Math.cos(p2) * Math.sin((l2 - l1) / 2) ** 2;
  return 2 * Math.asin(Math.min(1, Math.sqrt(h))) * 3440.065;
}
