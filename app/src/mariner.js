// Mariner's symbols (IHO S-52 / IEC 62288) drawn over the chart:
//  - own ship with the IHO symbols: the double circle (to scale at harbour scales, main.js),
//    the COG/SOG vector with one- and six-minute time marks and a double arrowhead, the
//    heading line, and a beam bearing line;
//  - the past track with time marks, labelled on the half hours;
//  - range rings round own ship and round any library mark or route waypoint;
//  - EBL/VRM: up to two electronic bearing lines with variable range markers, from own ship
//    or from a point on the chart;
//  - the S-52 scale bar (SCALEB10/11), the north arrow (NORTHAR1) and the overscale indication.
// With the paper-chart symbols, own ship, vector and track keep TweedPlot's own look; range
// rings, EBL/VRM and the overscale indication are there in both.
// Colours are the IHO palette's (NINFO, SHIPS, PSTRK, SCLBR...) when the IHO symbols are on.

import { invoke } from "@tauri-apps/api/core";
import { s52PxPerMm } from "./chartfeatures.js";
import { rhumb, rhumbDestination, fmtNm } from "./navmath.js";
import { infoBox } from "./infobox.js";
import { circle, destination, parseSpacing, fmtRange, fmtSpacing, displayScale, metresPerPixel, scaleBar, vectorMarks, trackMarks } from "./marinermath.js";

const empty = { type: "FeatureCollection", features: [] };
/** TweedPlot's colours for the tokens, used with the paper-chart symbols. */
const FALLBACK = { NINFO: "#d35400", SHIPS: "#1d2a33", PSTRK: "#d35400", PLRTE: "#c2185b", SCLBR: "#d35400", CHGRD: "#4c5b63",
  SNDG1: "#768c97", SNDG2: "#000000", UIBCK: "#ffffff", CHBLK: "#000000" };
let colourSource = () => null;
/** A colour token in the palette shown (IHO symbols), else TweedPlot's. */
export const tokenColour = (t) => colourSource()?.[t] ?? FALLBACK[t] ?? "#000000";
const pxmm = () => s52PxPerMm();
const RINGS_LAST = "racenav.rings.last";
const EBL_KEY = "racenav.ebl";

// ---------- images ---------------------------------------------------------------------
// "s52m:own" (own ship double circle), "s52m:wp:<token>" / "s52m:wpn:<token>" (waypoint, next
// waypoint), "s52m:ebo" (EBL/VRM origin), "s52m:t:<token>:<text>" (a label). Drawn at twice the
// size (pixelRatio 2) from the mm sizes and the screen calibration; "s52" at the front: redrawn
// when the IHO palette or the calibration changes (chartfeatures.js), and never darkened again.
function canvas(w, h) {
  const c = document.createElement("canvas");
  c.width = Math.max(1, Math.ceil(w)); c.height = Math.max(1, Math.ceil(h));
  return [c, c.getContext("2d")];
}
export function marinerImage(id) {
  if (!id.startsWith("s52m:")) return null;
  const k = 2 * pxmm(); // image px per mm
  const parts = id.slice(5).split(":");
  if (parts[0] === "own") {
    // OWNSHP: two circles, 6 and 3 mm across, 0.3 mm line
    const s = 6.8 * k, [c, g] = canvas(s, s);
    g.strokeStyle = tokenColour("SHIPS"); g.lineWidth = Math.max(2, 0.32 * k);
    for (const r of [3, 1.5]) { g.beginPath(); g.arc(s / 2, s / 2, r * k, 0, 2 * Math.PI); g.stroke(); }
    return g.getImageData(0, 0, c.width, c.height);
  }
  if (parts[0] === "wp" || parts[0] === "wpn") {
    // waypoint: a 4 mm circle; the next one (being steered for) another round it
    const next = parts[0] === "wpn", tok = parts[1] || "PLRTE";
    const s = (next ? 7.6 : 4.8) * k, [c, g] = canvas(s, s);
    g.strokeStyle = tokenColour(tok); g.lineWidth = Math.max(2, 0.5 * k);
    g.beginPath(); g.arc(s / 2, s / 2, 2 * k, 0, 2 * Math.PI); g.stroke();
    if (next) { g.beginPath(); g.arc(s / 2, s / 2, 3.4 * k, 0, 2 * Math.PI); g.stroke(); }
    return g.getImageData(0, 0, c.width, c.height);
  }
  if (parts[0] === "ebo") {
    // EBLVRM11: the origin of an offset EBL/VRM, a small circle with a cross (2 mm)
    const s = 2.6 * k, [c, g] = canvas(s, s);
    g.strokeStyle = tokenColour("NINFO"); g.lineWidth = Math.max(2, 0.32 * k);
    g.beginPath(); g.arc(s / 2, s / 2, 0.9 * k, 0, 2 * Math.PI); g.stroke();
    g.beginPath(); g.moveTo(s / 2, 0); g.lineTo(s / 2, s); g.moveTo(0, s / 2); g.lineTo(s, s / 2); g.stroke();
    return g.getImageData(0, 0, c.width, c.height);
  }
  if (parts[0] === "t") {
    const tok = parts[1], text = parts.slice(2).join(":");
    if (!text) return null;
    const font = `600 ${Math.round(2.8 * k)}px "Segoe UI", system-ui, sans-serif`;
    const [, g0] = canvas(1, 1);
    g0.font = font;
    const w = g0.measureText(text).width + 1.6 * k, h = 4.2 * k;
    const [c, g] = canvas(w, h);
    g.font = font; g.textBaseline = "middle";
    g.lineWidth = 0.9 * k; g.strokeStyle = tokenColour("UIBCK"); g.lineJoin = "round";
    g.strokeText(text, 0.8 * k, h / 2);
    g.fillStyle = tokenColour(tok); g.fillText(text, 0.8 * k, h / 2);
    return g.getImageData(0, 0, c.width, c.height);
  }
  return null;
}

// ---------- the module -----------------------------------------------------------------------
export function setupMariner(map, { settings, saveSettings, colours, usingS52, library, route, toast, getVariation }) {
  colourSource = colours;
  const $ = (id) => document.getElementById(id);
  const iho = () => !!usingS52();

  map.addSource("s52-mar", { type: "geojson", data: empty });
  map.addSource("s52-mar-pts", { type: "geojson", data: empty });
  const w = ["get", "px"];
  map.addLayer({ id: "s52-mar-line", type: "line", source: "s52-mar", filter: ["==", ["get", "d"], 0], layout: { "line-cap": "butt", "line-join": "round" },
    paint: { "line-color": ["get", "c"], "line-width": w } });
  // EBLs: ERBLNA01, 10 mm dashes with 2.2 mm gaps (dash lengths are in line widths)
  map.addLayer({ id: "s52-mar-dash", type: "line", source: "s52-mar", filter: ["==", ["get", "d"], 1],
    paint: { "line-color": ["get", "c"], "line-width": w, "line-dasharray": [10 / 0.64, 2.2 / 0.64] } });
  map.addLayer({ id: "s52-mar-dash2", type: "line", source: "s52-mar", filter: ["==", ["get", "d"], 2],
    paint: { "line-color": ["get", "c"], "line-width": w, "line-dasharray": [4, 3] } });
  // a wide invisible line: easy to hit with a right-click (EBLs, VRMs)
  map.addLayer({ id: "s52-mar-hit", type: "line", source: "s52-mar", filter: ["has", "ebl"],
    paint: { "line-color": "#000", "line-opacity": 0.01, "line-width": 14 } });
  map.addLayer({ id: "s52-mar-pt", type: "symbol", source: "s52-mar-pts", filter: ["!=", ["get", "lbl"], 1],
    layout: { "icon-image": ["get", "img"], "icon-anchor": ["coalesce", ["get", "anc"], "center"], "icon-offset": ["coalesce", ["get", "off"], ["literal", [0, 0]]],
              "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-rotation-alignment": "viewport" } });
  // labels that may give way to each other (track times, ring ranges)
  map.addLayer({ id: "s52-mar-lbl", type: "symbol", source: "s52-mar-pts", filter: ["==", ["get", "lbl"], 1],
    layout: { "icon-image": ["get", "img"], "icon-anchor": ["coalesce", ["get", "anc"], "center"], "icon-offset": ["coalesce", ["get", "off"], ["literal", [0, 0]]],
              "icon-allow-overlap": false, "icon-padding": 2 } });

  const groups = { own: [[], []], trk: [[], []], rings: [[], []], ebl: [[], []] };
  let queued = false;
  function flush() {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const ls = [], ps = [];
      for (const [l, p] of Object.values(groups)) { ls.push(...l); ps.push(...p); }
      map.getSource("s52-mar")?.setData({ type: "FeatureCollection", features: ls });
      map.getSource("s52-mar-pts")?.setData({ type: "FeatureCollection", features: ps });
    });
  }
  /** A line feature: colour token, width in mm, dash 0 solid / 1 EBL dash / 2 short dash. */
  const lineF = (coords, tok, mm, d = 0, props = {}) => ({ type: "Feature", properties: { c: tokenColour(tok), px: Math.max(1, mm * pxmm()), d, ...props }, geometry: { type: "LineString", coordinates: coords } });
  const ptF = (c, props) => ({ type: "Feature", properties: props, geometry: { type: "Point", coordinates: c } });
  const label = (tok, text) => `s52m:t:${tok}:${text}`;
  /** Enough points for a ring of nm to look round on the screen. */
  const stepsFor = (c, nm) => Math.round(Math.min(720, Math.max(72, (nm * 1852) / metresPerPixel(c[1], map.getZoom()) / 3)));
  /** How far (NM) from c to beyond the corners of the view: lines "to the edge". */
  function farNm(c) {
    const b = map.getBounds();
    const d = Math.max(...[[b.getWest(), b.getNorth()], [b.getEast(), b.getNorth()], [b.getWest(), b.getSouth()], [b.getEast(), b.getSouth()]].map((p) => rhumb(c, p).nm));
    return Math.min(d * 1.3 + 1, 12000);
  }

  // ---------- own ship ---------------------------------------------------------------------
  let boat = null; // { pos, dir, v, big }
  /** main.js layers kept in TweedPlot's look with the paper symbols, restyled with the IHO ones. */
  function styleMainLayers() {
    const on = iho();
    const vis = (id, show) => map.getLayer(id) && map.setLayoutProperty(id, "visibility", show ? "visible" : "none");
    const paint = (id, p, v) => map.getLayer(id) && map.setPaintProperty(id, p, v);
    vis("boat", !on);
    vis("cogline", !on);
    paint("boat-hull", "fill-opacity", on ? 0 : 0.35);
    paint("boat-hull-line", "line-color", on ? tokenColour("SHIPS") : "#8b0000");
    paint("boat-hull-line", "line-width", on ? Math.max(1, 0.32 * pxmm()) : 1.6);
    paint("boat-gps", "circle-stroke-color", on ? tokenColour("SHIPS") : "#8b0000");
    paint("boat-gps", "circle-color", on ? tokenColour("UIBCK") : "#fff");
    paint("hdgline", "line-color", on ? tokenColour("SHIPS") : "#1d2a33");
    paint("hdgline", "line-width", on ? Math.max(1, 0.32 * pxmm()) : 1.6);
    paint("track", "line-color", on ? tokenColour("PSTRK") : "#d35400");
    paint("track", "line-width", on ? Math.max(1.5, 0.5 * pxmm()) : 2);
  }

  function drawOwn() {
    const l = [], p = [];
    groups.own = [l, p];
    if (!boat) return flush();
    const { pos, v } = boat;
    const on = iho();
    if (on && !boat.big) p.push(ptF(pos, { img: "s52m:own" }));
    // the beam bearing line: through the ship at right angles to the heading, to the edge
    const hdg = v?.heading ?? v?.cog ?? null;
    if (settings.beamLine && hdg != null) {
      const far = farNm(pos);
      l.push(lineF([rhumbDestination(pos, hdg - 90, far), pos, rhumbDestination(pos, hdg + 90, far)], "SHIPS", 0.32, 2));
    }
    // the COG/SOG vector (ground stabilised: double arrowhead), time marks every minute
    if (on && settings.cogLine && v?.cog != null && v?.sog > 0.2) {
      const inf = settings.cogLen === "inf";
      const minutes = inf ? null : Number(settings.cogLen);
      const nm = inf ? farNm(pos) : (v.sog * minutes) / 60;
      const end = rhumbDestination(pos, v.cog, nm);
      l.push(lineF([pos, end], "SHIPS", 0.5));
      const P = map.project(pos), E = map.project(end);
      const len = Math.hypot(E.x - P.x, E.y - P.y);
      if (len > 2) {
        const u = { x: (E.x - P.x) / len, y: (E.y - P.y) / len }, n = { x: -u.y, y: u.x };
        const k = pxmm();
        const un = (x, y) => { const q = map.unproject([x, y]); return [q.lng, q.lat]; };
        const tick = (at, half) => l.push(lineF([un(at.x + n.x * half, at.y + n.y * half), un(at.x - n.x * half, at.y - n.y * half)], "SHIPS", 0.32));
        const marks = vectorMarks(inf ? Math.min(nm / v.sog * 60, 360) : minutes, v.sog);
        for (const m of marks) {
          const f = (m.nm / nm) * len;
          if (f > len - 0.5 * k) break;
          tick({ x: P.x + u.x * f, y: P.y + u.y * f }, (m.big ? 1.6 : 0.8) * k);
        }
        if (!inf) {
          // two arrowheads at the end of the vector, 2 mm long, 1.2 mm either side
          for (const back of [0, 1.4 * k]) {
            const tip = { x: E.x - u.x * back, y: E.y - u.y * back };
            const b = { x: tip.x - u.x * 2 * k, y: tip.y - u.y * 2 * k };
            l.push(lineF([un(b.x + n.x * 1.2 * k, b.y + n.y * 1.2 * k), un(tip.x, tip.y), un(b.x - n.x * 1.2 * k, b.y - n.y * 1.2 * k)], "SHIPS", 0.4));
          }
        }
      }
    }
    flush();
  }

  // ---------- past track ------------------------------------------------------------------
  let trk = { pts: [], times: [] };
  function drawTrack() {
    const l = [], p = [];
    groups.trk = [l, p];
    if (!iho() || trk.pts.length < 2) return flush();
    const k = pxmm();
    let lastPx = null;
    for (const m of trackMarks(trk.times, 10, 30)) {
      const a = trk.pts[m.i], b = trk.pts[Math.min(m.i + 1, trk.pts.length - 1)] ?? a, c = trk.pts[Math.max(m.i - 1, 0)];
      const A = map.project(a);
      // marks closer than 5 mm on the screen to the last are left out
      if (lastPx && Math.hypot(A.x - lastPx.x, A.y - lastPx.y) < 5 * k) continue;
      lastPx = A;
      const B = map.project(b === a ? a : b), C = map.project(c);
      let dx = B.x - C.x, dy = B.y - C.y;
      const d = Math.hypot(dx, dy) || 1; dx /= d; dy /= d;
      const half = 1.2 * k;
      const un = (x, y) => { const q = map.unproject([x, y]); return [q.lng, q.lat]; };
      l.push(lineF([un(A.x - dy * half, A.y + dx * half), un(A.x + dy * half, A.y - dx * half)], "PSTRK", 0.4));
      if (m.label) {
        const t = new Date(m.t * 1000);
        const hhmm = `${String(t.getHours()).padStart(2, "0")}${String(t.getMinutes()).padStart(2, "0")}`;
        p.push(ptF(a, { img: label("PSTRK", hhmm), lbl: 1, anc: dx > 0 ? "bottom-left" : "bottom-right", off: [0, -3] }));
      }
    }
    flush();
  }

  // ---------- range rings -----------------------------------------------------------------
  const lastRings = () => { try { return JSON.parse(localStorage.getItem(RINGS_LAST)) || { n: 3, step: 0.5 }; } catch { return { n: 3, step: 0.5 }; } };
  const keepLast = (r) => { try { localStorage.setItem(RINGS_LAST, JSON.stringify(r)); } catch { /* ignore */ } };
  const validRings = (r) => r && Number(r.n) >= 1 && Number(r.step) > 0;
  // the library marks with rings (a library can hold tens of thousands of marks: found once per change)
  let markRings = [];
  const findMarkRings = () => { markRings = (library?.marks?.() ?? []).filter((m) => validRings(m.data?.rings)); };
  findMarkRings();
  /** Every ring centre: own ship, library marks and route waypoints that have rings. */
  function ringCentres() {
    const out = [];
    if (settings.ownRings && boat) out.push({ pos: boat.pos, n: settings.ownRingN, step: settings.ownRingStep, own: true });
    const seen = new Set();
    for (const m of markRings) {
      seen.add(m.id);
      out.push({ pos: [m.lon, m.lat], n: m.data.rings.n, step: m.data.rings.step });
    }
    for (const c of route?.ringCentres?.() ?? []) if (validRings(c.rings) && !(c.id && seen.has(c.id))) out.push({ pos: c.pos, n: c.rings.n, step: c.rings.step });
    return out;
  }
  function drawRings() {
    const l = [], p = [];
    groups.rings = [l, p];
    for (const c of ringCentres()) {
      const n = Math.min(Math.max(1, Math.round(c.n)), 20);
      for (let i = 1; i <= n; i++) {
        const r = c.step * i;
        l.push(lineF(circle(c.pos, r, stepsFor(c.pos, r)), "NINFO", 0.32));
        // the range, where the ring crosses north of the centre
        const txt = i === n ? `${fmtRange(r)} NM` : fmtRange(r);
        p.push(ptF(destination(c.pos, 0, r), { img: label("NINFO", txt), lbl: 1, anc: "bottom", off: [0, -1] }));
      }
    }
    flush();
  }

  /** The rings card for a library mark, a route waypoint, or own ship. */
  const ringBox = infoBox(map, "rings", { width: 300, height: 150 });
  function openRings(target) {
    const cur = target.get();
    const r = validRings(cur) ? cur : lastRings();
    const el = document.createElement("div");
    el.className = "rings-card";
    el.innerHTML = `
      <div class="wx-row"><label for="rg-n">Rings</label> <select id="rg-n">${Array.from({ length: 10 }, (_, i) => `<option>${i + 1}</option>`).join("")}</select>
        <label for="rg-step" style="margin-left:10px">every</label> <input id="rg-step" list="rg-steps" style="width:5em" title="Spacing in nautical miles: 0.25, 1/4, 1 1/2, ½ …" /> NM
        <datalist id="rg-steps">${["0.1", "1/8", "0.2", "1/4", "0.3", "1/2", "3/4", "1", "1.5", "2", "3", "5", "10"].map((s) => `<option value="${s}">`).join("")}</datalist></div>
      <div class="wx-note rg-out"></div>
      <div class="wx-row" style="margin-top:8px"><button type="button" class="rg-del">${validRings(cur) ? "Remove rings" : "Cancel"}</button><span class="grow"></span><button type="button" class="rg-ok">${validRings(cur) ? "Done" : "Add rings"}</button></div>`;
    const nSel = el.querySelector("#rg-n"), stepIn = el.querySelector("#rg-step"), out = el.querySelector(".rg-out");
    nSel.value = String(Math.min(10, Math.max(1, Math.round(r.n))));
    stepIn.value = fmtSpacing(r.step);
    const read = () => { const step = parseSpacing(stepIn.value); return step ? { n: Number(nSel.value), step } : null; };
    const show = () => {
      const x = read();
      out.textContent = x ? `Out to ${fmtRange(x.n * x.step)} NM (${x.n} × ${fmtRange(x.step)} NM).` : "Spacing: a distance in NM, e.g. 0.25, 1/4 or 1 1/2.";
      out.classList.toggle("bad", !x);
      return x;
    };
    // live: each change shows on the chart at once (saved as it goes)
    const apply = async () => { const x = show(); if (x) { keepLast(x); await target.set(x); } };
    nSel.onchange = apply;
    stepIn.onchange = apply;
    stepIn.oninput = show;
    show();
    el.querySelector(".rg-ok").onclick = async () => { await apply(); ringBox.close(); };
    el.querySelector(".rg-del").onclick = async () => { if (validRings(target.get())) await target.set(null); ringBox.close(); };
    ringBox.open(el, { title: `Range rings: ${target.name}`, at: target.pos });
    if (!validRings(cur)) apply(); // a new set shows straight away
  }
  /** What the rings card works on, for something under a right-click: a library mark or a route waypoint. */
  function ringTarget(point) {
    const k = route?.waypointAt?.(point);
    if (k != null) {
      const wp = route.waypoint(k);
      if (wp?.id && library?.byId(wp.id)) return markTarget(library.byId(wp.id));
      if (wp) return { name: wp.name || (k === -1 ? "start" : `waypoint ${k + 1}`), pos: wp.pos, get: () => route.waypoint(k)?.rings, set: async (x) => { route.setRings(k, x); drawRings(); } };
    }
    const m = library?.markAt?.(point);
    return m ? markTarget(m) : null;
  }
  function markTarget(m) {
    return {
      name: m.name || "mark", pos: [m.lon, m.lat],
      get: () => library.byId(m.id)?.data?.rings,
      set: async (x) => {
        const cur = library.byId(m.id) || m;
        const data = { ...(cur.data || {}) };
        if (x) data.rings = { n: x.n, step: x.step }; else delete data.rings;
        try { await invoke("mark_save", { mark: { ...cur, data } }); library.refresh(); } catch (e) { toast?.(`Could not save the rings: ${e}`, 6000); }
      },
    };
  }

  // own ship rings: Layers › Own ship
  function setupOwnRingsUI() {
    const on = $("set-own-rings"), n = $("set-own-ring-n"), st = $("set-own-ring-step"), beam = $("set-beam-line");
    if (!on) return;
    for (let i = 1; i <= 10; i++) n.append(new Option(String(i), String(i)));
    on.checked = !!settings.ownRings;
    n.value = String(settings.ownRingN);
    st.value = fmtSpacing(settings.ownRingStep);
    on.onchange = () => { settings.ownRings = on.checked; saveSettings(); drawRings(); };
    n.onchange = () => { settings.ownRingN = Number(n.value); saveSettings(); drawRings(); };
    st.onchange = () => {
      const v = parseSpacing(st.value);
      st.classList.toggle("bad", !v);
      if (!v) return;
      settings.ownRingStep = v; saveSettings(); drawRings();
    };
    beam.checked = !!settings.beamLine;
    beam.onchange = () => { settings.beamLine = beam.checked; saveSettings(); drawOwn(); };
  }
  setupOwnRingsUI();

  // ---------- EBL / VRM ---------------------------------------------------------------------
  // Each: { origin: "boat" | [lon, lat], brg (°T), nm }. Up to two, as on a radar.
  let ebls = [];
  try { ebls = (JSON.parse(localStorage.getItem(EBL_KEY)) || []).filter((e) => e && Number.isFinite(e.brg) && e.nm > 0).slice(0, 2); } catch { /* ignore */ }
  const saveEbls = () => { try { localStorage.setItem(EBL_KEY, JSON.stringify(ebls)); } catch { /* ignore */ } };
  let armed = null; // { i, before } while the cursor sets one
  const originOf = (e) => (e.origin === "boat" ? boat?.pos ?? null : e.origin);
  const deg1 = (b) => ((((b % 360) + 360) % 360).toFixed(1)).padStart(5, "0");
  function eblText(e, i) {
    const o = originOf(e);
    const v = o ? getVariation?.(o) : null;
    const mag = Number.isFinite(v) ? ` ${deg1(e.brg - v)}°M` : "";
    return `EBL${i + 1} ${deg1(e.brg)}°T${mag}  VRM${i + 1} ${fmtNm(e.nm)}`;
  }
  function drawEbls() {
    const l = [], p = [];
    groups.ebl = [l, p];
    ebls.forEach((e, i) => {
      const o = originOf(e);
      if (!o) return;
      const far = Math.max(farNm(o), e.nm * 1.2);
      l.push(lineF([o, rhumbDestination(o, e.brg, far)], "NINFO", 0.64, 1, { ebl: i + 1 }));
      l.push(lineF(circle(o, e.nm, stepsFor(o, e.nm)), "NINFO", 0.64, 0, { ebl: i + 1 }));
      if (e.origin !== "boat") p.push(ptF(o, { img: "s52m:ebo" }));
      // the readout where the bearing line meets the range ring
      p.push(ptF(rhumbDestination(o, e.brg, e.nm), { img: label("NINFO", eblText(e, i)), anc: "bottom-left", off: [6, -4] }));
    });
    flush();
  }
  function arm(i, before) {
    armed = { i, before };
    document.body.classList.add("ebl-armed");
    map.getContainer().classList.add("drawing"); // clicks set the EBL: the info popups stand aside
    $("ebl-btn")?.classList.add("on");
    map.getCanvas().style.cursor = "crosshair";
    toast?.(`EBL/VRM ${i + 1}: move the cursor to set the bearing and range, click to fix it (Esc cancels).`, 6000);
  }
  function disarm(keep) {
    if (!armed) return;
    if (!keep) {
      if (armed.before) ebls[armed.i] = armed.before; else ebls.splice(armed.i, 1);
    }
    armed = null;
    document.body.classList.remove("ebl-armed");
    map.getContainer().classList.remove("drawing");
    $("ebl-btn")?.classList.remove("on");
    map.getCanvas().style.cursor = "";
    toast?.("");
    saveEbls();
    drawEbls();
  }
  /** A new EBL/VRM from own ship (or the chart centre with no boat), or from a point. */
  function startEbl(origin) {
    if (armed) disarm(false);
    if (ebls.length >= 2) ebls.shift(); // two at most: the oldest goes
    const o = origin ?? (boat ? "boat" : (() => { const c = map.getCenter(); return [c.lng, c.lat]; })());
    ebls.push({ origin: o, brg: 0, nm: 1 });
    arm(ebls.length - 1, null);
    drawEbls();
  }
  map.on("mousemove", (ev) => {
    if (!armed) return;
    const e = ebls[armed.i], o = e && originOf(e);
    if (!o) return;
    const r = rhumb(o, [ev.lngLat.lng, ev.lngLat.lat]);
    if (r.nm <= 0) return;
    e.brg = Math.round(r.brg * 10) / 10;
    e.nm = r.nm;
    drawEbls();
  });
  map.on("click", () => {
    if (!armed) return;
    // after the other click handlers: they see the drawing mode and stand aside
    setTimeout(() => disarm(true), 0);
  });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && armed) disarm(false); }, true);
  /** The EBL/VRM under a map point (index), or null. */
  function eblAt(point) {
    const f = map.queryRenderedFeatures([[point.x - 6, point.y - 6], [point.x + 6, point.y + 6]], { layers: ["s52-mar-hit"] })[0];
    return f ? f.properties.ebl - 1 : null;
  }

  // ---------- scale bar, north arrow, overscale --------------------------------------------
  const box = document.createElement("div");
  box.id = "mar-scale";
  // NORTHAR1's own drawing (IHO S-52 PresLib, 0.64 mm lines), turning about its pivot
  box.innerHTML = `<svg class="mar-north" viewBox="-2.25 -8.57 4.19 15.64" aria-label="North"><title>North (S-52 NORTHAR1)</title>
      <path d="M0,6.75 L0,-8.25" fill="none" stroke-width="0.64"/><path d="M-1.75,1.62 L-1.75,-1.69 L1.62,1.62 L1.62,-1.63" fill="none" stroke-width="0.64"/>
      <path d="M-1.93,-4.3 L1.62,-4.3 L-0.05,-8.22 Z" stroke="none"/></svg>
    <div class="mar-bar"><canvas></canvas><span></span></div>`;
  map.getContainer().append(box);
  const north = box.querySelector(".mar-north"), bar = box.querySelector(".mar-bar"), barCan = bar.querySelector("canvas"), barLbl = bar.querySelector("span");
  const ovr = document.createElement("div");
  ovr.id = "overscale-ind";
  ovr.className = "overscale-ind";
  ovr.hidden = true;
  ovr.title = "Overscale: the chart is shown at a larger scale than it was made for. Detail and positions are no better than at the chart's own scale";
  map.getContainer().append(ovr);
  // The scale bar and north arrow are part of the IMO display base: always shown, with either set
  // of chart symbols (the map's own scale at the bottom right is gone)
  function drawScale() {
    box.hidden = false;
    const k = pxmm();
    north.setAttribute("width", String(4.19 * k)); north.setAttribute("height", String(15.64 * k));
    north.style.stroke = north.style.fill = tokenColour("SCLBR");
    north.style.transformOrigin = `${(2.25 / 4.19) * 100}% ${(8.57 / 15.64) * 100}%`;
    north.style.transform = `rotate(${-map.getBearing()}deg)`;
    const c = map.getCenter();
    const H = map.getContainer().clientHeight;
    const maxPx = H * 0.45;
    // room for the ten-mile scale: from the bar's top (under the north arrow) to just above the
    // chart's bottom edge, leaving its label and the attribution clear
    const top = bar.offsetTop || 15.64 * k + 24;
    const room = Math.max(60, H - top - 44);
    const sb = scaleBar(c.lat, map.getZoom(), k, maxPx, room);
    const wPx = 1.2 * k, hPx = Math.max(8, sb.px);
    const dpr = window.devicePixelRatio || 1;
    barCan.width = Math.ceil(wPx * dpr); barCan.height = Math.ceil(hPx * dpr);
    barCan.style.width = `${wPx}px`; barCan.style.height = `${hPx}px`;
    const g = barCan.getContext("2d");
    const [a, b] = sb.symbol === "SCALEB10" ? ["SCLBR", "CHGRD"] : ["SNDG2", "SNDG1"];
    for (let i = 0; i < sb.segments; i++) {
      g.fillStyle = tokenColour(i % 2 ? b : a);
      g.fillRect(0, (i * barCan.height) / sb.segments, barCan.width, barCan.height / sb.segments + 0.5);
    }
    barLbl.textContent = `${fmtRange(sb.nm)} NM`;
    barLbl.style.color = tokenColour(a === "SCLBR" ? "SCLBR" : "SNDG2");
    bar.title = sb.symbol === "SCALEB10" ? `S-52 scale bar: ${fmtRange(sb.nm)} NM in tenths (display scale larger than 1:80,000)` : `S-52 latitude scale: ${fmtRange(sb.nm)} NM in tenths (display scale smaller than 1:80,000; longer than ten miles when zoomed out, so it stays at least 35 mm long)`;
  }
  let scaleSeq = 0;
  async function checkOverscale() {
    const c = map.getCenter();
    const seq = ++scaleSeq;
    let r = null;
    try { r = await invoke("chart_scale_at", { lon: c.lng, lat: c.lat }); } catch { r = null; }
    if (seq !== scaleSeq) return;
    const n = displayScale(c.lat, map.getZoom(), pxmm());
    const over = r?.scale ? r.scale / n : 0;
    // S-52: indicate overscale from 1.0; a little slack so it doesn't flicker at the edge
    ovr.hidden = !(over >= 1.05);
    if (!ovr.hidden) ovr.textContent = `Overscale ×${over < 10 ? over.toFixed(1) : Math.round(over)}`;
    ovr.title = `Overscale: shown at 1:${Math.round(n).toLocaleString()}, larger than the chart's scale (1:${r?.scale?.toLocaleString?.() ?? "?"}${r?.enc ? ", from its ENC usage band" : ", the largest of its LINZ scale band"}). Detail and positions are no better than at the chart's own scale.`;
  }

  // ---------- redraws -------------------------------------------------------------------------
  let moveQueued = false;
  map.on("move", () => {
    if (moveQueued) return;
    moveQueued = true;
    requestAnimationFrame(() => {
      moveQueued = false;
      drawScale();
      // screen-sized parts (vector marks and arrowheads, lines to the edge) follow the view
      if (boat && (iho() || settings.beamLine)) drawOwn();
      if (ebls.length) drawEbls();
    });
  });
  let overTimer = null;
  map.on("moveend", () => {
    drawTrack();
    drawRings(); // round at any zoom
    // the chart's scale at the centre: once the chart has stopped (following the boat moves it each fix)
    clearTimeout(overTimer);
    overTimer = setTimeout(checkOverscale, 400);
  });
  window.addEventListener("racenav-screen-calibration", () => all());
  library?.onChange?.(() => { findMarkRings(); drawRings(); });
  route?.onChange?.(() => drawRings());
  function all() { styleMainLayers(); drawOwn(); drawTrack(); drawRings(); drawEbls(); drawScale(); checkOverscale(); }
  all();

  return {
    /** Own ship moved (or placed): position, heading/course (°T), vessel data, drawn to scale. */
    boat(pos, dir, v, big) {
      boat = pos ? { pos, dir, v, big } : null;
      drawOwn();
      if (settings.ownRings) drawRings();
      if (ebls.some((e) => e.origin === "boat")) drawEbls();
    },
    /** The past track and its times (Unix seconds). */
    track(pts, times) { trk = { pts, times }; drawTrack(); },
    /** The symbols or palette changed. */
    restyle: () => all(),
    // range rings
    ringTarget,
    openRings,
    ownRingTarget: () => ({ name: "own ship", pos: boat?.pos ?? null,
      get: () => (settings.ownRings ? { n: settings.ownRingN, step: settings.ownRingStep } : null),
      set: async (x) => {
        settings.ownRings = !!x;
        if (x) { settings.ownRingN = x.n; settings.ownRingStep = x.step; }
        saveSettings(); drawRings();
        if ($("set-own-rings")) { $("set-own-rings").checked = !!x; if (x) { $("set-own-ring-n").value = String(x.n); $("set-own-ring-step").value = fmtSpacing(x.step); } }
      } }),
    // EBL / VRM
    startEbl,
    eblAt,
    adjustEbl(i) { if (ebls[i]) { if (armed) disarm(false); arm(i, { ...ebls[i] }); } },
    deleteEbl(i) { if (armed) disarm(false); ebls.splice(i, 1); saveEbls(); drawEbls(); },
    eblArmed: () => !!armed,
    eblCount: () => ebls.length,
  };
}
