// Barriers the routing keeps off (Library › Barriers): lines it may not cross
// and zones it may not enter, kept in the library folder\barriers.json, plus
// the clearance rings of library marks given a barrier radius.
//
// Draw line / Draw zone → click points, double-click to finish, Esc to stop.
// Clearing bearing → click the mark (it snaps to library marks), click out along the line of the
// bearing, then
// click on the side to keep off: a triangle zone from the mark (the bearing line and the same
// distance at 90°). It is
// labelled in °T and °M. Points move like course marks: click one (amber ring), wait for green, drag.
// Right-click a barrier: delete a point, delete it, or switch it off/on.
import { placing } from "./modes.js";
import { invoke } from "@tauri-apps/api/core";
import { libList } from "./liblist.js";
import { fmtBrg, clearingZone, clearingInfo } from "./navmath.js";

const KEY = "racenav.barriers";
const RED = "#c62828";
const ARM_MS = 1200;
const SNAP_PX = 14;
const empty = { type: "FeatureCollection", features: [] };
const isArea = (b) => b.kind === "zone" || b.kind === "clearing";
const KIND_WORD = { line: "Line", zone: "Zone", clearing: "Clearing bearing" };

/**
 * The cross hatch that fills zones, so that zoomed in past a zone's edges it still shows that
 * the chart view is inside one: a square tile with both diagonals, which repeats without a seam.
 * `alpha` is fainter for barriers not in use. Returns ImageData at `ratio` pixels to the px.
 */
export function hatchTile(alpha, ratio = 2, size = 18, color = RED) {
  const s = size * ratio;
  const c = document.createElement("canvas");
  c.width = s; c.height = s;
  const g = c.getContext("2d");
  g.strokeStyle = color;
  g.globalAlpha = alpha;
  g.lineWidth = 1 * ratio;
  g.beginPath();
  // each diagonal, and its neighbours one tile away, so the corners join up
  for (const d of [-s, 0, s]) {
    g.moveTo(d, 0); g.lineTo(d + s, s);
    g.moveTo(d, s); g.lineTo(d + s, 0);
  }
  g.stroke();
  return g.getImageData(0, 0, s, s);
}

function ring(c, nm) {
  const out = [];
  const k = Math.cos((c[1] * Math.PI) / 180);
  for (let i = 0; i <= 48; i++) {
    const a = (i / 48) * 2 * Math.PI;
    out.push([c[0] + (Math.sin(a) * nm) / 60 / k, c[1] + (Math.cos(a) * nm) / 60]);
  }
  return out;
}

export function setupBarriers(map, { before, library, toast, getVariation }) {
  const $ = (id) => document.getElementById(id);
  let list = [];
  let prefs = { show: true, hatch: true };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const savePrefs = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  let drawing = null; // { kind, pts }
  let cursor = null;

  // ---------- chart ----------
  for (const s of ["bar-shapes", "bar-pts", "bar-rubber", "bar-sel", "bar-discs", "bar-clear", "bar-labels"]) map.addSource(s, { type: "geojson", data: empty });
  map.addLayer({ id: "bar-fill", type: "fill", source: "bar-shapes", filter: ["==", ["geometry-type"], "Polygon"],
    paint: { "fill-color": RED, "fill-opacity": ["case", ["get", "active"], 0.12, 0.04] } }, before);
  // the cross hatch over zones, clearing bearings and barrier marks' circles (Library › Barriers › Hatch zones)
  map.addImage("bar-hatch", hatchTile(0.55), { pixelRatio: 2 });
  map.addImage("bar-hatch-off", hatchTile(0.2), { pixelRatio: 2 });
  map.addLayer({ id: "bar-hatch", type: "fill", source: "bar-shapes", filter: ["==", ["geometry-type"], "Polygon"],
    paint: { "fill-pattern": ["case", ["get", "active"], "bar-hatch", "bar-hatch-off"] } }, before);
  map.addLayer({ id: "bar-disc-hatch", type: "fill", source: "bar-discs", paint: { "fill-pattern": "bar-hatch" } }, before);
  map.addLayer({ id: "bar-line", type: "line", source: "bar-shapes",
    paint: { "line-color": RED, "line-width": ["case", ["get", "active"], 2.4, 1.2], "line-opacity": ["case", ["get", "active"], 0.9, 0.45],
             "line-dasharray": [3, 1.5] } }, before);
  map.addLayer({ id: "bar-hit", type: "line", source: "bar-shapes", paint: { "line-color": "#000", "line-opacity": 0.01, "line-width": 14 } }, before);
  map.addLayer({ id: "bar-discs", type: "line", source: "bar-discs", paint: { "line-color": RED, "line-width": 1.6, "line-dasharray": [2, 2] } }, before);
  map.addLayer({ id: "bar-clear", type: "line", source: "bar-clear",
    paint: { "line-color": RED, "line-width": ["case", ["get", "active"], 3, 1.6], "line-opacity": ["case", ["get", "active"], 1, 0.5] } }, before);
  map.addLayer({ id: "bar-lbl", type: "symbol", source: "bar-labels",
    layout: { "icon-image": ["concat", "txt:r:", ["get", "lbl"]], "icon-anchor": "bottom-left", "icon-offset": [6, -4],
              "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);
  map.addLayer({ id: "bar-rubber", type: "line", source: "bar-rubber", paint: { "line-color": RED, "line-width": 1.6, "line-dasharray": [2, 2] } }, before);
  map.addLayer({ id: "bar-sel", type: "circle", source: "bar-sel",
    paint: { "circle-radius": 12, "circle-color": "rgba(0,0,0,0)", "circle-stroke-width": 3.5, "circle-stroke-color": "#f0a500" } }, before);
  map.addLayer({ id: "bar-pt", type: "circle", source: "bar-pts", minzoom: 7,
    paint: { "circle-radius": 3.5, "circle-color": "#fff", "circle-stroke-color": RED, "circle-stroke-width": 1.8 } }, before);

  /** "Clearing bearing 225°T 220°M": the bearing of the mark from the boat. */
  function clearingText(pts) {
    const { brg } = clearingInfo(pts);
    const v = getVariation?.(pts[0]);
    const mag = Number.isFinite(v) ? ` ${fmtBrg(brg - v)}°M` : "";
    return `Clearing bearing ${fmtBrg(brg)}°T${mag}`;
  }

  function draw() {
    const vis = prefs.show ? "visible" : "none";
    for (const id of ["bar-fill", "bar-line", "bar-hit", "bar-discs", "bar-pt", "bar-sel", "bar-clear", "bar-lbl"]) map.setLayoutProperty(id, "visibility", vis);
    for (const id of ["bar-hatch", "bar-disc-hatch"]) map.setLayoutProperty(id, "visibility", prefs.show && prefs.hatch ? "visible" : "none");
    const shapes = [], pts = [], clears = [], labels = [];
    for (const b of list) {
      if (b.points.length < 2) continue;
      const geom = isArea(b) && b.points.length >= 3
        ? { type: "Polygon", coordinates: [[...b.points, b.points[0]]] }
        : { type: "LineString", coordinates: b.points };
      shapes.push({ type: "Feature", properties: { id: b.id, active: b.active }, geometry: geom });
      b.points.forEach((p, i) => pts.push({ type: "Feature", properties: { id: b.id, i }, geometry: { type: "Point", coordinates: p } }));
      if (b.kind === "clearing" && b.points.length === 3) {
        clears.push({ type: "Feature", properties: { id: b.id, active: b.active }, geometry: { type: "LineString", coordinates: [b.points[0], b.points[1]] } });
        const mid = [(b.points[0][0] + b.points[1][0]) / 2, (b.points[0][1] + b.points[1][1]) / 2];
        labels.push({ type: "Feature", properties: { lbl: clearingText(b.points) }, geometry: { type: "Point", coordinates: mid } });
      }
    }
    if (drawing?.pts.length) {
      let coords = cursor ? [...drawing.pts, cursor] : drawing.pts;
      if (drawing.kind === "clearing" && drawing.pts.length === 2 && cursor) coords = clearingZone(drawing.pts[0], drawing.pts[1], cursor);
      if ((drawing.kind === "zone" || drawing.kind === "clearing") && coords.length >= 3) coords = [...coords, coords[0]];
      map.getSource("bar-rubber").setData({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: coords } });
      drawing.pts.forEach((p, i) => pts.push({ type: "Feature", properties: { id: "", i }, geometry: { type: "Point", coordinates: p } }));
    } else map.getSource("bar-rubber").setData(empty);
    map.getSource("bar-shapes").setData({ type: "FeatureCollection", features: shapes });
    map.getSource("bar-clear").setData({ type: "FeatureCollection", features: clears });
    map.getSource("bar-labels").setData({ type: "FeatureCollection", features: labels });
    map.getSource("bar-pts").setData({ type: "FeatureCollection", features: pts });
    const discs = library.marks().filter((m) => m.barrier_nm > 0)
      .map((m) => ({ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [ring([m.lon, m.lat], m.barrier_nm)] } }));
    map.getSource("bar-discs").setData({ type: "FeatureCollection", features: discs });
    drawSel();
  }

  // ---------- data ----------
  async function refresh() {
    list = (await invoke("barriers_list").catch(() => [])) || [];
    renderList();
    draw();
  }
  async function save(b) {
    try {
      const s = await invoke("barrier_save", { barrier: b });
      const i = list.findIndex((x) => x.id === s.id);
      if (i >= 0) list[i] = s; else list.push(s);
      renderList();
      draw();
      return s;
    } catch (e) { toast(`Could not save the barrier: ${e}`, 6000); return null; }
  }

  // the list: the same as every library tab (liblist.js); the tick at the left of a row
  // (the swatch) says whether routings use it
  const barrierList = libList($("lib-barriers-list"), {
    key: "barriers",
    noun: ["barrier", "barriers"],
    search: (b, q) => b.name.toLowerCase().includes(q) || (b.notes || "").toLowerCase().includes(q),
    sorts: [
      { key: "name", label: "By name", cmp: (a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }) },
      { key: "new", label: "Newest first", cmp: (a, b) => (b.modified || 0) - (a.modified || 0) },
    ],
    filters: [
      { key: "kind", label: "Kind", type: "choice", options: () => [["line", "Lines"], ["zone", "Zones"], ["clearing", "Clearing bearings"]], test: (b, v) => b.kind === v },
      { key: "off", label: "Not used", type: "toggle", title: "Only the barriers routings don't use", test: (b) => !b.active },
      { key: "tz", label: "Imported", type: "toggle", title: "Only the lines, areas and circles brought in from TimeZero or KML", test: (b) => b.id.startsWith("tz-") || b.id.startsWith("kml-") },
    ],
    row: {
      render(b, el) {
        el.innerHTML = `<input type="checkbox" class="ll-use" title="Used by routings"><div class="main" title="Zoom to it"><input class="nm" type="text" spellcheck="false"><small></small></div>`;
        const cb = el.querySelector(".ll-use");
        cb.checked = b.active;
        cb.onclick = (e) => e.stopPropagation();
        cb.onchange = () => save({ ...b, active: cb.checked });
        const nm = el.querySelector(".nm");
        nm.value = b.name;
        nm.onclick = (e) => e.stopPropagation();
        nm.onchange = () => save({ ...b, name: nm.value.trim() });
        el.querySelector("small").textContent = `${KIND_WORD[b.kind] ?? "Line"} · ${b.kind === "clearing" && b.points.length === 3 ? clearingText(b.points) : `${b.points.length} points`}${b.active ? "" : " · not used"}`;
      },
      click: (b) => fit(b.points),
    },
    actions: [
      { label: "Use", title: "Routings keep to the selected barriers", run: async (bs) => { for (const b of bs) await save({ ...b, active: true }); } },
      { label: "Don't use", title: "Keep the selected barriers but routings ignore them", run: async (bs) => { for (const b of bs) await save({ ...b, active: false }); } },
      { label: "Delete", danger: true, confirm: (bs) => `Delete ${bs.length === 1 ? `“${bs[0].name}”` : `${bs.length} barriers`}?`, run: async (bs) => { for (const b of bs) await remove(b.id); } },
    ],
    empty: () => "No barriers. Draw a line or a zone.",
  });
  function renderList() {
    barrierList.setItems(list);
    $("bar-show").checked = prefs.show;
    $("bar-hatch").checked = prefs.hatch;
  }
  function fit(pts) {
    if (!pts.length) return;
    const xs = pts.map((p) => p[0]), ys = pts.map((p) => p[1]);
    map.fitBounds([[Math.min(...xs), Math.min(...ys)], [Math.max(...xs), Math.max(...ys)]], { padding: 80, maxZoom: 13, duration: 500 });
  }
  async function remove(id) {
    try { await invoke("barrier_delete", { id }); } catch (e) { toast(`Could not delete: ${e}`, 6000); return; }
    list = list.filter((b) => b.id !== id);
    if (sel?.id === id) deselect();
    renderList();
    draw();
  }
  $("bar-show").onchange = (e) => { prefs.show = e.target.checked; savePrefs(); draw(); };
  $("bar-hatch").onchange = (e) => { prefs.hatch = e.target.checked; savePrefs(); draw(); };

  // ---------- drawing ----------
  function arm(kind) {
    drawing = kind ? { kind, pts: [] } : null;
    cursor = null;
    $("bar-line").classList.toggle("armed", kind === "line");
    $("bar-zone").classList.toggle("armed", kind === "zone");
    $("bar-clearing").classList.toggle("armed", kind === "clearing");
    map.getCanvas().style.cursor = kind ? "crosshair" : "";
    map.getContainer().classList.toggle("drawing", !!kind);
    if (kind) {
      map.doubleClickZoom.disable();
      deselect();
      prefs.show = true; savePrefs();
      toast(kind === "clearing" ? "Clearing bearing: click the mark, click out to sea along the bearing line, then click on the side to keep off. Esc to stop."
        : `${kind === "zone" ? "Zone" : "Line"}: click to set points, double-click to finish, Esc to stop.`);
    } else {
      setTimeout(() => map.doubleClickZoom.enable(), 0);
      toast("");
    }
    draw();
  }
  async function finish() {
    const d = drawing;
    arm(null);
    if (!d) return;
    // a double-click sets the last point twice: drop points on top of each other
    const pts = [];
    for (const p of d.pts) {
      const q = pts.at(-1);
      if (q) { const a = map.project(q), b = map.project(p); if (Math.hypot(a.x - b.x, a.y - b.y) < 4) continue; }
      pts.push(p);
    }
    if (d.kind === "clearing") {
      if (pts.length < 3) { toast("A clearing bearing needs the mark, a point along the bearing, and a point on the side to keep off.", 4000); return; }
      const zone = clearingZone(pts[0], pts[1], pts[2]);
      const s = await save({ id: "", name: `Clearing ${fmtBrg(clearingInfo(zone).brg)}°T`, kind: "clearing", points: zone, active: true, notes: "" });
      if (s) toast(`Saved “${s.name}”. Routings keep out of it while it is ticked in Library › Barriers.`, 5000);
      return;
    }
    if (pts.length < (d.kind === "zone" ? 3 : 2)) { toast(d.kind === "zone" ? "A zone needs at least three corners." : "A line needs at least two points.", 4000); return; }
    const s = await save({ id: "", name: "", kind: d.kind, points: pts, active: true, notes: "" });
    if (s) toast(`Saved “${s.name}”. Routings keep off it while it is ticked in Library › Barriers.`, 5000);
  }
  $("bar-line").onclick = () => (drawing ? finish() : arm("line"));
  $("bar-zone").onclick = () => (drawing ? finish() : arm("zone"));
  $("bar-clearing").onclick = () => (drawing ? finish() : arm("clearing"));
  map.on("click", (e) => {
    if (!drawing) return;
    let p = [e.lngLat.lng, e.lngLat.lat];
    if (drawing.kind === "clearing") {
      const last = drawing.pts.at(-1);
      if (last) { const a = map.project(last); if (Math.hypot(a.x - e.point.x, a.y - e.point.y) < 4) return; } // a double-click
      if (!drawing.pts.length) {
        // the bearing starts at a mark: snap to the nearest library mark
        let best = SNAP_PX;
        for (const m of library.marks()) {
          const q = map.project([m.lon, m.lat]);
          const d = Math.hypot(q.x - e.point.x, q.y - e.point.y);
          if (d < best) { best = d; p = [m.lon, m.lat]; }
        }
      }
      drawing.pts.push(p);
      if (drawing.pts.length === 3) finish(); else draw();
      return;
    }
    drawing.pts.push(p);
    draw();
  });
  map.on("dblclick", (e) => { if (drawing) { e.preventDefault(); if (drawing.kind !== "clearing") finish(); } });
  map.on("mousemove", (e) => { if (drawing?.pts.length) { cursor = [e.lngLat.lng, e.lngLat.lat]; draw(); } });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && drawing) { e.preventDefault(); if (drawing.kind === "clearing") arm(null); else finish(); } });

  // ---------- moving points ----------
  let sel = null, movable = false, armTimer = null, dragging = false;
  function drawSel() {
    const b = sel && list.find((x) => x.id === sel.id);
    const p = b?.points[sel.i];
    map.getSource("bar-sel").setData(p ? { type: "Feature", properties: {}, geometry: { type: "Point", coordinates: p } } : empty);
    map.setPaintProperty("bar-sel", "circle-stroke-color", movable ? "#2e8b57" : "#f0a500");
  }
  function select(id, i) {
    clearTimeout(armTimer);
    sel = { id, i }; movable = false; drawSel();
    toast("Barrier point selected: wait for the ring to turn green, then drag it.");
    armTimer = setTimeout(() => { movable = true; drawSel(); toast("Drag the point to move it."); }, ARM_MS);
  }
  function deselect() { clearTimeout(armTimer); if (sel) toast(""); sel = null; movable = false; drawSel(); }
  map.on("click", "bar-pt", (e) => {
    if (drawing || placing()) return;
    const f = e.features[0].properties;
    if (!f.id || (sel && sel.id === f.id && sel.i === f.i)) return;
    select(f.id, f.i);
  });
  map.on("click", (e) => {
    if (drawing || !sel) return;
    if (!map.queryRenderedFeatures(e.point, { layers: ["bar-pt"] }).length) deselect();
  });
  map.on("mousedown", "bar-pt", (e) => {
    const f = e.features[0].properties;
    if (drawing || e.originalEvent.button !== 0 || !sel || !movable || f.id !== sel.id || f.i !== sel.i) return;
    e.preventDefault();
    map.dragPan.disable();
    dragging = true;
  });
  map.on("mousemove", (e) => {
    if (!dragging) return;
    const b = list.find((x) => x.id === sel.id);
    if (b) { b.points[sel.i] = [e.lngLat.lng, e.lngLat.lat]; draw(); }
  });
  map.on("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    map.dragPan.enable();
    const b = list.find((x) => x.id === sel.id);
    if (b) save(b);
  });

  // ---------- right-click menu ----------
  /** The barrier at a map point: its point, its line or edge, or (unless `edge`) inside a zone. */
  function hitAt(point, edge = false) {
    if (!prefs.show) return null;
    const box = [[point.x - 6, point.y - 6], [point.x + 6, point.y + 6]];
    const p = map.queryRenderedFeatures(box, { layers: ["bar-pt"] }).find((f) => f.properties.id);
    const l = p ?? map.queryRenderedFeatures(box, { layers: edge ? ["bar-hit"] : ["bar-hit", "bar-fill"] })[0];
    const b = l && list.find((x) => x.id === l.properties.id);
    return b ? { id: b.id, i: p ? p.properties.i : null, active: b.active, name: b.name } : null;
  }
  function deletePoint(id, i) {
    const b = list.find((x) => x.id === id);
    if (!b) return;
    if (b.points.length <= (isArea(b) ? 3 : 2)) { remove(id); return; }
    b.points.splice(i, 1);
    deselect();
    save(b);
  }
  function toggle(id) {
    const b = list.find((x) => x.id === id);
    if (b) save({ ...b, active: !b.active });
  }

  library.onChange(draw);
  refresh();
  /** Show a barrier's row in the Library (the caller opens the Barriers tab). */
  function reveal(id) { if (!barrierList.highlight(id)) { barrierList.clearFilters(); barrierList.highlight(id); } }
  return { hitAt, deletePoint, remove, toggle, refresh, reveal, isDrawing: () => !!drawing };
}
