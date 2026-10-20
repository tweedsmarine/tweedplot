// Ruler: measure bearing and distance on the chart, over one or more legs.
//
// Ruler button (left rail) → click to set a point, click again for the next …
// double-click to finish (the ruler stays on the chart); Esc stops. Each leg is
// labelled with its rhumb-line bearing (°T, and °M when Signal K gives the
// variation) and distance, with the total at the end. Points move like course
// marks: click one (amber ring), wait for the ring to turn green, then drag.
// Right-click a ruler: delete it, delete a point, or save it as a route.
import { invoke } from "@tauri-apps/api/core";
import { rhumb, fmtBrg, fmtNm } from "./navmath.js";

const KEY = "racenav.rulers";
const COLOUR = "#c2185b";
const ARM_MS = 1200;
const empty = { type: "FeatureCollection", features: [] };

export function setupRuler(map, { before, ui, library, route, toast, getVariation }) {
  const $ = (id) => document.getElementById(id);
  let rulers = []; // { id, pts: [[lon, lat], …] }
  try { rulers = JSON.parse(localStorage.getItem(KEY) || "[]").filter((r) => Array.isArray(r.pts) && r.pts.length > 1); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(rulers)); } catch { /* ignore */ } };
  let n = rulers.reduce((m, r) => Math.max(m, Number(String(r.id).slice(1)) || 0), 0);
  let armed = false;
  let active = null; // the ruler being drawn
  let cursor = null;

  // ---------- chart ----------------------------------------------------------
  for (const s of ["ruler-lines", "ruler-rubber", "ruler-pts", "ruler-labels", "ruler-sel"]) map.addSource(s, { type: "geojson", data: empty });
  map.addLayer({ id: "ruler-line", type: "line", source: "ruler-lines", layout: { "line-cap": "round" },
    paint: { "line-color": COLOUR, "line-width": 2.2 } }, before);
  // a wide invisible line makes the ruler easy to hit with a right-click
  map.addLayer({ id: "ruler-hit", type: "line", source: "ruler-lines", paint: { "line-color": "#000", "line-opacity": 0.01, "line-width": 14 } }, before);
  map.addLayer({ id: "ruler-rubber", type: "line", source: "ruler-rubber",
    paint: { "line-color": COLOUR, "line-width": 1.6, "line-dasharray": [3, 2] } }, before);
  map.addLayer({ id: "ruler-sel", type: "circle", source: "ruler-sel",
    paint: { "circle-radius": 13, "circle-color": "rgba(0,0,0,0)", "circle-stroke-width": 3.5, "circle-stroke-color": "#f0a500" } }, before);
  map.addLayer({ id: "ruler-pt", type: "circle", source: "ruler-pts",
    paint: { "circle-radius": 4.5, "circle-color": "#fff", "circle-stroke-color": COLOUR, "circle-stroke-width": 2 } }, before);
  map.addLayer({ id: "ruler-lbl", type: "symbol", source: "ruler-labels",
    layout: { "icon-image": ["concat", "txt:r:", ["get", "lbl"]], "icon-anchor": ["get", "anchor"], "icon-offset": ["get", "offset"],
              "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);

  const variation = () => { const v = getVariation?.(); return Number.isFinite(v) ? v : null; };
  function legText(a, b) {
    const { brg, nm } = rhumb(a, b);
    const v = variation();
    return `${fmtBrg(brg)}°T${v != null ? ` ${fmtBrg(brg - v)}°M` : ""}  ${fmtNm(nm)}`;
  }
  const total = (pts) => pts.slice(1).reduce((s, p, i) => s + rhumb(pts[i], p).nm, 0);

  function draw() {
    const lines = [], pts = [], labels = [];
    const all = active ? [...rulers, active] : rulers;
    for (const r of all) {
      r.pts.forEach((p, i) => pts.push({ type: "Feature", properties: { rid: r.id, i }, geometry: { type: "Point", coordinates: p } }));
      for (let i = 1; i < r.pts.length; i++) {
        const a = r.pts[i - 1], b = r.pts[i];
        lines.push({ type: "Feature", properties: { rid: r.id, i }, geometry: { type: "LineString", coordinates: [a, b] } });
        labels.push(label(midpoint(a, b), legText(a, b), a, b));
      }
      if (r.pts.length > 2 && r !== active) {
        labels.push({ type: "Feature", properties: { lbl: `Total ${fmtNm(total(r.pts))}`, anchor: "top-left", offset: [8, 8] }, geometry: { type: "Point", coordinates: r.pts.at(-1) } });
      }
    }
    // the leg being drawn, to the cursor
    let rubber = empty;
    if (active?.pts.length && cursor) {
      const a = active.pts.at(-1);
      rubber = { type: "FeatureCollection", features: [{ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: [a, cursor] } }] };
      const t = active.pts.length > 1 ? `  (total ${fmtNm(total([...active.pts, cursor]))})` : "";
      labels.push({ type: "Feature", properties: { lbl: `${legText(a, cursor)}${t}`, anchor: "bottom-left", offset: [10, -8] }, geometry: { type: "Point", coordinates: cursor } });
    }
    map.getSource("ruler-lines").setData({ type: "FeatureCollection", features: lines });
    map.getSource("ruler-pts").setData({ type: "FeatureCollection", features: pts });
    map.getSource("ruler-labels").setData({ type: "FeatureCollection", features: labels });
    map.getSource("ruler-rubber").setData(rubber);
    drawSel();
  }
  function midpoint(a, b) { return [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]; }
  /** A leg label beside the middle of the leg, on the side away from the line. */
  function label(at, text, a, b) {
    const pa = map.project(a), pb = map.project(b);
    const steep = Math.abs(pb.y - pa.y) > Math.abs(pb.x - pa.x);
    return { type: "Feature", properties: { lbl: text, anchor: steep ? "left" : "bottom", offset: steep ? [10, 0] : [0, -6] }, geometry: { type: "Point", coordinates: at } };
  }
  map.on("moveend", () => { if (rulers.length || active) draw(); }); // label sides depend on the view

  // ---------- drawing a ruler ----------------------------------------------------------
  const btn = $("ruler-btn");
  function arm(on) {
    armed = on;
    btn.classList.toggle("armed", on);
    map.getCanvas().style.cursor = on ? "crosshair" : "";
    map.getContainer().classList.toggle("drawing", on);
    if (on) {
      map.doubleClickZoom.disable();
      deselect();
      toast("Ruler: click to set points, double-click to finish, Esc to stop.");
    } else {
      // double-click zoom back on after this double-click has been handled
      setTimeout(() => map.doubleClickZoom.enable(), 0);
      toast("");
    }
  }
  function finish() {
    if (active) {
      // a double-click sets the last point twice: drop points on top of each other
      const pts = [];
      for (const p of active.pts) {
        const q = pts.at(-1);
        if (q) {
          const a = map.project(q), b = map.project(p);
          if (Math.hypot(a.x - b.x, a.y - b.y) < 4) continue;
        }
        pts.push(p);
      }
      active.pts = pts;
      if (pts.length > 1) { rulers.push(active); save(); }
      active = null;
    }
    cursor = null;
    arm(false);
    draw();
  }
  btn.onclick = () => (armed ? finish() : arm(true));
  map.on("click", (e) => {
    if (!armed) return;
    active ??= { id: `r${++n}`, pts: [] };
    active.pts.push([e.lngLat.lng, e.lngLat.lat]);
    draw();
  });
  map.on("dblclick", (e) => {
    if (!armed) return;
    e.preventDefault();
    finish();
  });
  map.on("mousemove", (e) => {
    if (!armed || !active) return;
    cursor = [e.lngLat.lng, e.lngLat.lat];
    draw();
  });
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && armed) { e.preventDefault(); finish(); }
  });

  // ---------- selecting and moving points ----------------------------------------------------------
  let sel = null; // { rid, i }
  let movable = false;
  let armTimer = null;
  let dragging = false;
  const find = (rid) => rulers.find((r) => r.id === rid);
  function drawSel() {
    const r = sel && find(sel.rid);
    const p = r?.pts[sel.i];
    map.getSource("ruler-sel").setData(p ? { type: "FeatureCollection", features: [{ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: p } }] } : empty);
    map.setPaintProperty("ruler-sel", "circle-stroke-color", movable ? "#2e8b57" : "#f0a500");
  }
  function select(rid, i) {
    clearTimeout(armTimer);
    sel = { rid, i };
    movable = false;
    drawSel();
    toast("Ruler point selected: wait for the ring to turn green, then drag it.");
    armTimer = setTimeout(() => { movable = true; drawSel(); toast("Drag the point to move it."); }, ARM_MS);
  }
  function deselect() {
    clearTimeout(armTimer);
    if (sel) toast("");
    sel = null;
    movable = false;
    drawSel();
  }
  map.on("click", "ruler-pt", (e) => {
    if (armed || document.querySelector(".armed")) return;
    const f = e.features[0].properties;
    if (sel && sel.rid === f.rid && sel.i === f.i) return;
    select(f.rid, f.i);
  });
  map.on("click", (e) => {
    if (armed || !sel) return;
    if (!map.queryRenderedFeatures(e.point, { layers: ["ruler-pt"] }).length) deselect();
  });
  map.on("mousedown", "ruler-pt", (e) => {
    const f = e.features[0].properties;
    if (armed || e.originalEvent.button !== 0 || !sel || !movable || f.rid !== sel.rid || f.i !== sel.i) return;
    e.preventDefault();
    map.dragPan.disable();
    dragging = true;
    map.getCanvas().style.cursor = "grabbing";
  });
  map.on("mousemove", (e) => {
    if (!dragging) return;
    const r = find(sel.rid);
    if (r) { r.pts[sel.i] = [e.lngLat.lng, e.lngLat.lat]; draw(); }
  });
  map.on("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    map.dragPan.enable();
    map.getCanvas().style.cursor = "";
    save();
    draw();
  });
  map.on("mouseenter", "ruler-pt", () => { if (!armed && !dragging) map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "ruler-pt", () => { if (!armed && !dragging) map.getCanvas().style.cursor = ""; });

  // ---------- right-click actions (offered by the chart's menu) ----------------------------------------------------------
  /** The ruler (and point) under a screen point, for the right-click menu: { rid, i?, points } or null. */
  function hitAt(point) {
    const box = [[point.x - 6, point.y - 6], [point.x + 6, point.y + 6]];
    const p = map.queryRenderedFeatures(box, { layers: ["ruler-pt"] })[0];
    const l = p ?? map.queryRenderedFeatures(box, { layers: ["ruler-hit"] })[0];
    if (!l) return null;
    const r = find(l.properties.rid);
    if (!r) return null;
    return { rid: r.id, i: p ? p.properties.i : null, points: r.pts.length };
  }
  function deleteRuler(rid) {
    rulers = rulers.filter((r) => r.id !== rid);
    if (sel?.rid === rid) deselect();
    save(); draw();
  }
  function deletePoint(rid, i) {
    const r = find(rid);
    if (!r) return;
    if (r.pts.length <= 2) { deleteRuler(rid); return; }
    r.pts.splice(i, 1);
    deselect();
    save(); draw();
  }
  function deleteAll() { rulers = []; deselect(); save(); draw(); }

  /** Save a ruler as a library route: first point the start, the rest new library marks; opens it in the Course panel. */
  async function saveAsRoute(rid) {
    const r = find(rid);
    if (!r) return;
    const routes = await invoke("routes_list").catch(() => []);
    let k = 1;
    while (routes.some((x) => x.name === `Ruler route ${k}`)) k++;
    const name = `Ruler route ${k}`;
    try {
      const ids = [];
      for (let i = 1; i < r.pts.length; i++) {
        const m = await invoke("mark_save", { mark: { name: `${name} – ${i}`, lat: r.pts[i][1], lon: r.pts[i][0], kind: "mark", group: name, notes: "" } });
        ids.push(m.id);
      }
      const saved = await invoke("route_save", { route: { id: "", name, notes: "", start: r.pts[0], start_mark: null, marks: ids.map((id) => ({ mark: id, rounding: "" })) } });
      await library.refresh();
      route.loadRoute(saved, library.marks());
      ui.open("course");
      toast(`Saved “${name}” in the Library and opened it in the Course panel.`, 6000);
    } catch (e) { toast(`Could not save the route: ${e}`, 6000); }
  }

  draw();
  return { hitAt, deleteRuler, deletePoint, deleteAll, saveAsRoute, count: () => rulers.length, isArmed: () => armed };
}
