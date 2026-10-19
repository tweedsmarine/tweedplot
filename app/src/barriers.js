// Barriers the routing keeps off (Library › Barriers): lines it may not cross
// and zones it may not enter, kept in the library folder\barriers.json, plus
// the clearance rings of library marks given a barrier radius.
//
// Draw line / Draw zone → click points, double-click to finish, Esc to stop.
// Points move like course marks: click one (amber ring), wait for green, drag.
// Right-click a barrier: delete a point, delete it, or switch it off/on.
import { invoke } from "@tauri-apps/api/core";

const KEY = "racenav.barriers";
const RED = "#c62828";
const ARM_MS = 1200;
const empty = { type: "FeatureCollection", features: [] };

function ring(c, nm) {
  const out = [];
  const k = Math.cos((c[1] * Math.PI) / 180);
  for (let i = 0; i <= 48; i++) {
    const a = (i / 48) * 2 * Math.PI;
    out.push([c[0] + (Math.sin(a) * nm) / 60 / k, c[1] + (Math.cos(a) * nm) / 60]);
  }
  return out;
}

export function setupBarriers(map, { before, library, toast }) {
  const $ = (id) => document.getElementById(id);
  let list = [];
  let prefs = { show: true };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const savePrefs = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  let drawing = null; // { kind, pts }
  let cursor = null;

  // ---------- chart ----------
  for (const s of ["bar-shapes", "bar-pts", "bar-rubber", "bar-sel", "bar-discs"]) map.addSource(s, { type: "geojson", data: empty });
  map.addLayer({ id: "bar-fill", type: "fill", source: "bar-shapes", filter: ["==", ["geometry-type"], "Polygon"],
    paint: { "fill-color": RED, "fill-opacity": ["case", ["get", "active"], 0.12, 0.04] } }, before);
  map.addLayer({ id: "bar-line", type: "line", source: "bar-shapes",
    paint: { "line-color": RED, "line-width": ["case", ["get", "active"], 2.4, 1.2], "line-opacity": ["case", ["get", "active"], 0.9, 0.45],
             "line-dasharray": [3, 1.5] } }, before);
  map.addLayer({ id: "bar-hit", type: "line", source: "bar-shapes", paint: { "line-color": "#000", "line-opacity": 0.01, "line-width": 14 } }, before);
  map.addLayer({ id: "bar-discs", type: "line", source: "bar-discs", paint: { "line-color": RED, "line-width": 1.6, "line-dasharray": [2, 2] } }, before);
  map.addLayer({ id: "bar-rubber", type: "line", source: "bar-rubber", paint: { "line-color": RED, "line-width": 1.6, "line-dasharray": [2, 2] } }, before);
  map.addLayer({ id: "bar-sel", type: "circle", source: "bar-sel",
    paint: { "circle-radius": 12, "circle-color": "rgba(0,0,0,0)", "circle-stroke-width": 3.5, "circle-stroke-color": "#f0a500" } }, before);
  map.addLayer({ id: "bar-pt", type: "circle", source: "bar-pts", minzoom: 7,
    paint: { "circle-radius": 3.5, "circle-color": "#fff", "circle-stroke-color": RED, "circle-stroke-width": 1.8 } }, before);

  function draw() {
    const vis = prefs.show ? "visible" : "none";
    for (const id of ["bar-fill", "bar-line", "bar-hit", "bar-discs", "bar-pt", "bar-sel"]) map.setLayoutProperty(id, "visibility", vis);
    const shapes = [], pts = [];
    for (const b of list) {
      if (b.points.length < 2) continue;
      const geom = b.kind === "zone" && b.points.length >= 3
        ? { type: "Polygon", coordinates: [[...b.points, b.points[0]]] }
        : { type: "LineString", coordinates: b.points };
      shapes.push({ type: "Feature", properties: { id: b.id, active: b.active }, geometry: geom });
      b.points.forEach((p, i) => pts.push({ type: "Feature", properties: { id: b.id, i }, geometry: { type: "Point", coordinates: p } }));
    }
    if (drawing?.pts.length) {
      const coords = cursor ? [...drawing.pts, cursor] : drawing.pts;
      if (drawing.kind === "zone" && coords.length >= 3) coords.push(coords[0]);
      map.getSource("bar-rubber").setData({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: coords } });
      drawing.pts.forEach((p, i) => pts.push({ type: "Feature", properties: { id: "", i }, geometry: { type: "Point", coordinates: p } }));
    } else map.getSource("bar-rubber").setData(empty);
    map.getSource("bar-shapes").setData({ type: "FeatureCollection", features: shapes });
    map.getSource("bar-pts").setData({ type: "FeatureCollection", features: pts });
    const discs = library.marks().filter((m) => m.barrier_nm > 0)
      .map((m) => ({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: ring([m.lon, m.lat], m.barrier_nm) } }));
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

  function renderList() {
    const ul = $("lib-barriers");
    ul.innerHTML = "";
    if (!list.length) ul.innerHTML = `<li class="empty">No barriers. Draw a line or a zone.</li>`;
    for (const b of list) {
      const li = document.createElement("li");
      li.innerHTML = `<input type="checkbox" title="Used by routings"><div class="main"><input class="nm" type="text" spellcheck="false"><small></small></div>` +
        `<button data-act="zoom" title="Zoom to it">⌖</button><button data-act="x" title="Delete">✕</button>`;
      const [cb] = li.children;
      cb.checked = b.active;
      cb.onchange = () => save({ ...b, active: cb.checked });
      const nm = li.querySelector(".nm");
      nm.value = b.name;
      nm.onchange = () => save({ ...b, name: nm.value.trim() });
      li.querySelector("small").textContent = `${b.kind === "zone" ? "Zone" : "Line"} · ${b.points.length} points${b.active ? "" : " · off"}`;
      li.querySelector('[data-act="zoom"]').onclick = () => fit(b.points);
      li.querySelector('[data-act="x"]').onclick = () => remove(b.id);
      ul.append(li);
    }
    $("bar-show").checked = prefs.show;
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

  // ---------- drawing ----------
  function arm(kind) {
    drawing = kind ? { kind, pts: [] } : null;
    cursor = null;
    $("bar-line").classList.toggle("armed", kind === "line");
    $("bar-zone").classList.toggle("armed", kind === "zone");
    map.getCanvas().style.cursor = kind ? "crosshair" : "";
    if (kind) {
      map.doubleClickZoom.disable();
      deselect();
      prefs.show = true; savePrefs();
      toast(`${kind === "zone" ? "Zone" : "Line"}: click to set points, double-click to finish, Esc to stop.`);
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
    if (pts.length < (d.kind === "zone" ? 3 : 2)) { toast(d.kind === "zone" ? "A zone needs at least three corners." : "A line needs at least two points.", 4000); return; }
    const s = await save({ id: "", name: "", kind: d.kind, points: pts, active: true, notes: "" });
    if (s) toast(`Saved “${s.name}”. Routings keep off it while it is ticked in Library › Barriers.`, 5000);
  }
  $("bar-line").onclick = () => (drawing ? finish() : arm("line"));
  $("bar-zone").onclick = () => (drawing ? finish() : arm("zone"));
  map.on("click", (e) => { if (drawing) { drawing.pts.push([e.lngLat.lng, e.lngLat.lat]); draw(); } });
  map.on("dblclick", (e) => { if (drawing) { e.preventDefault(); finish(); } });
  map.on("mousemove", (e) => { if (drawing?.pts.length) { cursor = [e.lngLat.lng, e.lngLat.lat]; draw(); } });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && drawing) { e.preventDefault(); finish(); } });

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
    if (drawing || document.querySelector(".armed")) return;
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
  function hitAt(point) {
    if (!prefs.show) return null;
    const box = [[point.x - 6, point.y - 6], [point.x + 6, point.y + 6]];
    const p = map.queryRenderedFeatures(box, { layers: ["bar-pt"] }).find((f) => f.properties.id);
    const l = p ?? map.queryRenderedFeatures(box, { layers: ["bar-hit", "bar-fill"] })[0];
    const b = l && list.find((x) => x.id === l.properties.id);
    return b ? { id: b.id, i: p ? p.properties.i : null, active: b.active, name: b.name } : null;
  }
  function deletePoint(id, i) {
    const b = list.find((x) => x.id === id);
    if (!b) return;
    if (b.points.length <= (b.kind === "zone" ? 3 : 2)) { remove(id); return; }
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
  return { hitAt, deletePoint, remove, toggle, refresh, isDrawing: () => !!drawing };
}
