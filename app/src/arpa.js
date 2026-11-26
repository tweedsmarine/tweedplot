// Radar (ARPA) targets on the chart, from NMEA TTM and TLL (decoded in the back end: radar.rs).
// Each target is placed from its TLL position when the radar sends one, else from own ship's
// position and the TTM's range and bearing (a relative bearing is turned true with the heading),
// with a 6-minute vector for its course and speed and its number, name and the radar's CPA and
// TCPA beside it. Lost targets go grey and are dropped after 30 s; one not reported for a minute
// is dropped too. The CPA and TCPA are the radar's own. A target inside the collision limits
// (Alarms › Targets: CPA under, within) is drawn red; with "Alarms for radar (ARPA) targets too" on,
// it also raises a collision alarm, escalating as AIS ones do (off by default: radars alarm themselves).
import { listen } from "@tauri-apps/api/event";

const EMPTY = { type: "FeatureCollection", features: [] };
const R_NM = 3440.065;
const VECTOR_MIN = 6;

/** The point `nm` along `brg` (degrees true) from [lon, lat] (great circle). */
export function destination([lon, lat], brg, nm) {
  const d = nm / R_NM, b = (brg * Math.PI) / 180, p1 = (lat * Math.PI) / 180, l1 = (lon * Math.PI) / 180;
  const p2 = Math.asin(Math.sin(p1) * Math.cos(d) + Math.cos(p1) * Math.sin(d) * Math.cos(b));
  const l2 = l1 + Math.atan2(Math.sin(b) * Math.sin(d) * Math.cos(p1), Math.cos(d) - Math.sin(p1) * Math.sin(p2));
  return [((((l2 * 180) / Math.PI) + 540) % 360) - 180, (p2 * 180) / Math.PI];
}

/** Where a target is and where its vector ends, or null when it can't be placed. Exported for tests. */
export function place(t, own) {
  let pos = Number.isFinite(t.lat) && Number.isFinite(t.lon) ? [t.lon, t.lat] : null;
  const hdg = own?.heading ?? own?.cog;
  if (!pos && own?.pos && Number.isFinite(t.rangeNm) && Number.isFinite(t.bearing)) {
    const brg = t.bearingTrue ? t.bearing : Number.isFinite(hdg) ? (t.bearing + hdg) % 360 : null;
    if (brg != null) pos = destination(own.pos, brg, t.rangeNm);
  }
  if (!pos) return null;
  let end = null;
  if (Number.isFinite(t.speedKn) && t.speedKn > 0.2 && Number.isFinite(t.course)) {
    const crs = t.courseTrue ? t.course : Number.isFinite(hdg) ? (t.course + hdg) % 360 : null;
    if (crs != null) end = destination(pos, crs, (t.speedKn * VECTOR_MIN) / 60);
  }
  return { pos, end };
}

/** Inside the collision limits ({ cpa NM, tcpa min }): closing, CPA and TCPA under them. Exported for tests. */
export function inside(t, lim) {
  return t.status !== "lost" && Number.isFinite(t.cpaNm) && Number.isFinite(t.tcpaMin) && t.tcpaMin >= 0 && t.cpaNm < lim.cpa && t.tcpaMin < lim.tcpa;
}

export function setupRadarTargets(map, { getOwn, getAlarms = () => null, limits = () => null }) {
  const targets = new Map(); // `${sourceId}:${number}` → { t, at, lostAt }
  function layers() {
    if (map.getSource("radar-targets")) return;
    map.addSource("radar-targets", { type: "geojson", data: EMPTY });
    map.addLayer({ id: "radar-vec", type: "line", source: "radar-targets", filter: ["==", ["geometry-type"], "LineString"],
      paint: { "line-color": ["case", ["get", "lost"], "#8a8f93", "#1b8a3a"], "line-width": 1.6 } });
    map.addLayer({ id: "radar-tgt", type: "circle", source: "radar-targets", filter: ["==", ["geometry-type"], "Point"],
      paint: { "circle-radius": 6, "circle-color": "rgba(0,0,0,0)", "circle-stroke-width": 2,
        "circle-stroke-color": ["case", ["get", "lost"], "#8a8f93", ["get", "close"], "#d0021b", "#1b8a3a"] } });
    map.addLayer({ id: "radar-lbl", type: "symbol", source: "radar-targets", filter: ["==", ["geometry-type"], "Point"],
      layout: { "icon-image": ["concat", "lbl:", ["get", "label"]], "icon-anchor": "left", "icon-offset": [10, 0], "icon-allow-overlap": false } });
  }
  const lim = () => { const l = limits(); return l && Number.isFinite(l.cpa) && Number.isFinite(l.tcpa) ? l : { cpa: 0.5, tcpa: 30 }; };
  function draw() {
    if (!map.getSource("radar-targets")) layers();
    const own = getOwn(), features = [];
    for (const { t } of targets.values()) {
      const p = place(t, own);
      if (!p) continue;
      const lost = t.status === "lost";
      const close = inside(t, lim());
      const cpa = Number.isFinite(t.cpaNm) && Number.isFinite(t.tcpaMin) && t.tcpaMin >= 0 ? ` · CPA ${t.cpaNm.toFixed(2)} NM in ${Math.round(t.tcpaMin)} min` : "";
      features.push({ type: "Feature", properties: { lost, close, label: `R${t.number}${t.name ? ` ${t.name}` : ""}${lost ? " lost" : cpa}` },
        geometry: { type: "Point", coordinates: p.pos } });
      if (p.end) features.push({ type: "Feature", properties: { lost }, geometry: { type: "LineString", coordinates: [p.pos, p.end] } });
    }
    map.getSource("radar-targets").setData({ type: "FeatureCollection", features });
  }

  // collision alarms (with the option on): raised while inside the limits, cleared when out of them or gone
  const raised = new Set();
  function checkAlarms() {
    const al = getAlarms(), l = limits();
    const want = new Set();
    if (al && l?.on && l.radar) {
      const own = getOwn();
      for (const [key, { t, source }] of targets) {
        if (!inside(t, l)) continue;
        const id = `radar:${key}`;
        want.add(id);
        const stage = Math.max(t.tcpaMin < l.tcpa / 4 ? 2 : t.tcpaMin < l.tcpa / 2 ? 1 : 0, t.cpaNm < l.cpa / 4 ? 2 : t.cpaNm < l.cpa / 2 ? 1 : 0);
        const p = place(t, own);
        const where = Number.isFinite(t.rangeNm) && Number.isFinite(t.bearing) ? `${t.rangeNm.toFixed(2)} NM at ${String(Math.round(t.bearing)).padStart(3, "0")}°${t.bearingTrue ? "T" : " relative"}` : "";
        al.raise(id, { level: "alarm", stage, holdable: true,
          text: `COLLISION RISK radar target R${t.number}${t.name ? ` ${t.name}` : ""}: CPA ${t.cpaNm.toFixed(2)} NM in ${Math.round(t.tcpaMin)} min`,
          detail: `${where}${where ? " · " : ""}CPA and TCPA from the radar${source ? ` on ${source}` : ""} · click to show`,
          onClick: p ? () => map.easeTo({ center: p.pos }) : null });
      }
    }
    for (const id of raised) if (!want.has(id)) al?.clear(id);
    raised.clear();
    for (const id of want) raised.add(id);
  }

  let pending = false;
  listen("radar-target", ({ payload }) => {
    const t = payload?.target;
    if (!t) return;
    const key = `${payload.sourceId}:${t.number}`;
    const old = targets.get(key);
    targets.set(key, { t, source: payload.source, at: Date.now(), lostAt: t.status === "lost" ? old?.lostAt ?? Date.now() : null });
    checkAlarms();
    // several targets a second: drawn once a frame
    if (!pending) { pending = true; requestAnimationFrame(() => { pending = false; try { draw(); } catch { /* map not ready */ } }); }
  }).catch(() => {});

  // lost for 30 s, or not reported for a minute: off the chart; and redrawn as own ship moves
  setInterval(() => {
    const now = Date.now();
    let changed = false;
    for (const [k, v] of targets) if ((v.lostAt && now - v.lostAt > 30000) || now - v.at > 60000) { targets.delete(k); changed = true; }
    if (changed || targets.size) { try { draw(); } catch { /* map not ready */ } }
    checkAlarms();
  }, 2000);

  return { count: () => targets.size };
}
