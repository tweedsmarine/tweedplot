// Course panel: the route being edited (start + marks, saved in the library)
// and navigation to the next mark. Routing lives in routing.js.
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { distNm } from "./instruments.js";

const WORK_KEY = "racenav.route.work"; // the route being edited survives a restart

function load(key, defaults) {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem(key) || "{}") }; } catch { return { ...defaults }; }
}
function save(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch { /* ignore */ } }

export function setupRoute(map, { library, getBoat, getArrivalNm }) {
  const $ = (id) => document.getElementById(id);
  const listeners = [];
  let legsHidden = false;

  // The route being edited. Marks: { id (library id, or null until saved), name, lon, lat, rounding }.
  const work = load(WORK_KEY, { routeId: null, name: "", start: null, startRef: null, marks: [], dirty: false, nav: 0 });
  let start = work.start; // [lon, lat]
  let startRef = work.startRef; // { id, name } when the start is a library mark
  let marks = work.marks;
  let routeId = work.routeId;
  let dirty = work.dirty;
  let navIndex = work.nav || 0;
  let mode = null; // "start" | "marks" | null

  // ---------- layers ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  for (const id of ["rt-marks", "rt-legs"]) map.addSource(id, { type: "geojson", data: empty });
  map.addLayer({ id: "rt-legs", type: "line", source: "rt-legs",
    paint: { "line-color": "#c2185b", "line-width": ["case", ["get", "active"], 2.5, 1.2], "line-dasharray": [4, 3], "line-opacity": 0.8 } }, "track");
  map.addLayer({ id: "rt-marks", type: "symbol", source: "rt-marks",
    layout: { "icon-image": ["get", "icon"], "icon-allow-overlap": true, "icon-ignore-placement": true } });
  map.addLayer({ id: "rt-labels", type: "symbol", source: "rt-marks", filter: ["!=", ["get", "name"], ""],
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 12], "icon-allow-overlap": true } });

  function persist() {
    save(WORK_KEY, { routeId, name: $("rt-name").value, start, startRef, marks, dirty, nav: navIndex });
  }
  function changed() {
    dirty = true;
    persist();
    drawMarks();
    for (const f of listeners) f();
  }

  function drawMarks() {
    const f = [];
    if (start) f.push(pt(start, { icon: "mk:S", sel: -1, name: startRef?.name || "" }));
    marks.forEach((m, i) => f.push(pt([m.lon, m.lat], { icon: `mk:${i + 1}`, sel: i, name: m.name || "" })));
    map.getSource("rt-marks").setData({ type: "FeatureCollection", features: f });
    // legs, the one being sailed drawn heavier
    const pts = [start, ...marks.map((m) => [m.lon, m.lat])].filter(Boolean);
    const legs = [];
    for (let i = 1; i < pts.length; i++) legs.push({ type: "Feature", properties: { active: i - (start ? 1 : 0) === navIndex }, geometry: { type: "LineString", coordinates: [pts[i - 1], pts[i]] } });
    map.getSource("rt-legs").setData({ type: "FeatureCollection", features: legsHidden ? [] : legs });
    library.hide([startRef?.id, ...marks.map((m) => m.id)].filter(Boolean));
    if (selected !== null && !posOf(selected)) deselect();
    drawSelection();
    renderList();
  }

  function renderList() {
    $("rt-save").textContent = dirty ? "Save*" : "Save";
    $("rt-save").title = dirty ? "Unsaved changes: save the route and its marks in the library" : "Saved in the library";
    const ul = $("rt-marklist");
    ul.innerHTML = "";
    const row = (num, cls, name, sub, buttons) => {
      const li = document.createElement("li");
      li.innerHTML = `<span class="num ${cls}">${num}</span><div class="main"><b></b><small></small></div>${buttons}`;
      li.querySelector("b").textContent = name;
      li.querySelector("small").textContent = sub;
      ul.append(li);
      return li;
    };
    if (start) {
      const li = row("S", "s", startRef ? `Start: ${startRef.name}` : "Start", ddm(start[1], "N", "S") + " " + ddm(start[0], "E", "W"), `<button data-act="x" title="Remove the start">✕</button>`);
      li.querySelector(".main").onclick = () => map.flyTo({ center: start });
      li.querySelector('[data-act="x"]').onclick = () => { start = null; startRef = null; changed(); };
    }
    let prev = start;
    marks.forEach((m, i) => {
      const leg = prev ? ` · ${distNm(prev, [m.lon, m.lat]).toFixed(1)} NM` : "";
      prev = [m.lon, m.lat];
      const li = row(i + 1, "", m.name || `New mark ${i + 1}`, `${m.id ? "" : "not saved · "}${ddm(m.lat, "N", "S")} ${ddm(m.lon, "E", "W")}${leg}`,
        `<button data-act="up" title="Earlier">↑</button><button data-act="down" title="Later">↓</button><button data-act="x" title="Remove from the route (stays in the library)">✕</button>`);
      li.classList.toggle("active", i === navIndex);
      li.querySelector(".main").onclick = () => map.flyTo({ center: [m.lon, m.lat] });
      li.querySelector('[data-act="up"]').disabled = i === 0;
      li.querySelector('[data-act="down"]').disabled = i === marks.length - 1;
      li.querySelector('[data-act="up"]').onclick = () => { [marks[i - 1], marks[i]] = [marks[i], marks[i - 1]]; changed(); };
      li.querySelector('[data-act="down"]').onclick = () => { [marks[i + 1], marks[i]] = [marks[i], marks[i + 1]]; changed(); };
      li.querySelector('[data-act="x"]').onclick = () => { marks.splice(i, 1); if (navIndex >= marks.length) navIndex = Math.max(0, marks.length - 1); changed(); };
    });
    if (!start && !marks.length) ul.innerHTML = `<li class="empty">Set a start, then add marks: click the chart, or click a library mark to reuse it.</li>`;
    $("nav-to").textContent = marks.length ? `${navIndex + 1}. ${marks[navIndex]?.name || `Mark ${navIndex + 1}`}` : "—";
  }

  // ---------- selecting, moving and deleting marks on the chart ---------------------------------
  // Left-click a mark to select it (amber ring). After a short pause the ring turns
  // green and the mark can be moved: drag it, or right-click "Move … here".
  const ARM_MS = 1200;
  let selected = null; // -1 = start, 0.. = mark index
  let movable = false;
  let armTimer = null;
  map.addSource("rt-sel", { type: "geojson", data: empty });
  map.addLayer({ id: "rt-sel", type: "circle", source: "rt-sel",
    paint: { "circle-radius": 17, "circle-color": "rgba(0,0,0,0)", "circle-stroke-width": 4, "circle-stroke-color": "#f0a500" } }, "rt-marks");
  const posOf = (k) => (k === -1 ? start : marks[k] && [marks[k].lon, marks[k].lat]);
  const labelOf = (k) => (k === -1 ? (startRef ? `start (${startRef.name})` : "start") : marks[k]?.name || `mark ${k + 1}`);
  const status = (s) => { $("crs-status").textContent = s; };
  function drawSelection() {
    const p = selected !== null && posOf(selected);
    map.getSource("rt-sel").setData(p ? { type: "FeatureCollection", features: [pt(p, {})] } : empty);
    map.setPaintProperty("rt-sel", "circle-stroke-color", movable ? "#2e8b57" : "#f0a500");
  }
  function select(k) {
    clearTimeout(armTimer);
    selected = k;
    movable = false;
    drawSelection();
    status(`Selected ${labelOf(k)}. Wait for the ring to turn green to move it, or right-click to remove it.`);
    armTimer = setTimeout(() => {
      movable = true;
      drawSelection();
      status(`${cap(labelOf(k))} can be moved: drag it, or right-click and choose Move.`);
    }, ARM_MS);
  }
  function deselect() {
    clearTimeout(armTimer);
    selected = null;
    movable = false;
    map.getSource("rt-sel").setData(empty);
  }
  function setPos(k, p) {
    if (k === -1) start = p; else { marks[k].lon = p[0]; marks[k].lat = p[1]; }
  }
  function moveSelected(p) {
    if (selected === null || !movable) return;
    setPos(selected, p);
    changed();
  }
  function deleteSelected() {
    if (selected === null) return;
    if (selected === -1) { start = null; startRef = null; } else marks.splice(selected, 1);
    deselect();
    changed();
  }

  // Drag a movable mark.
  let dragging = false;
  map.on("mousedown", "rt-marks", (e) => {
    const k = e.features[0].properties.sel;
    if (e.originalEvent.button !== 0 || k !== selected || !movable) return;
    e.preventDefault(); // stop the chart panning
    map.dragPan.disable();
    dragging = true;
    map.getCanvas().style.cursor = "grabbing";
  });
  map.on("mousemove", (e) => {
    if (!dragging) return;
    setPos(selected, [e.lngLat.lng, e.lngLat.lat]);
    drawMarks();
  });
  map.on("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    map.dragPan.enable();
    map.getCanvas().style.cursor = mode ? "crosshair" : "";
    changed();
  });
  map.on("mouseenter", "rt-marks", () => { if (!mode && !dragging) map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "rt-marks", () => { if (!mode && !dragging) map.getCanvas().style.cursor = ""; });
  // Esc deselects, unless it is closing a menu.
  window.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    if (mode) { arm(null); e.preventDefault(); return; }
    if (selected !== null && document.getElementById("ctx").hidden) { deselect(); e.preventDefault(); }
  }, true);

  // ---------- editing the route ----------------------------------------------------------
  function arm(m) {
    mode = mode === m ? null : m;
    $("rt-start").classList.toggle("armed", mode === "start");
    $("rt-add").classList.toggle("armed", mode === "marks");
    map.getCanvas().style.cursor = mode ? "crosshair" : "";
  }
  $("rt-start").onclick = () => arm("start");
  $("rt-add").onclick = () => arm("marks");
  $("rt-boat").onclick = () => {
    const b = getBoat();
    if (!b) { status("No boat position yet (Signal K, or right-click: Move boat here)."); return; }
    start = b; startRef = null; changed();
  };
  $("rt-undo").onclick = () => { marks.pop(); changed(); };
  $("rt-name").value = work.name;
  $("rt-name").oninput = () => { dirty = true; persist(); renderList(); };
  map.on("click", (e) => {
    if (!mode) {
      const hit = map.queryRenderedFeatures(e.point, { layers: ["rt-marks"] })[0];
      if (hit) { if (hit.properties.sel !== selected) select(hit.properties.sel); }
      else if (selected !== null) deselect();
      return;
    }
    const p = [e.lngLat.lng, e.lngLat.lat];
    const lm = library.markAt(e.point);
    if (mode === "start") { setStartAt(p, lm); arm(null); return; }
    marks.push(lm ? { id: lm.id, name: lm.name, lon: lm.lon, lat: lm.lat, rounding: "" } : { id: null, name: "", lon: p[0], lat: p[1], rounding: "" });
    changed();
  });

  /** Start at a chart position, or at a library mark (snapped to it, and kept linked). */
  function setStartAt(p, lm) {
    if (lm) { start = [lm.lon, lm.lat]; startRef = { id: lm.id, name: lm.name }; }
    else { start = p; startRef = null; }
    changed();
  }

  // Save: new marks go into the library first, moved/renamed library marks are updated.
  $("rt-save").onclick = async () => {
    if (!marks.length) { status("Add marks before saving the route."); return; }
    const name = $("rt-name").value.trim() || `Route ${new Date().toISOString().slice(0, 10)}`;
    $("rt-name").value = name;
    try {
      const lib = library.marks();
      for (const m of marks) {
        const known = m.id && lib.find((x) => x.id === m.id);
        if (!known) {
          const saved = await invoke("mark_save", { mark: { name: m.name, lat: m.lat, lon: m.lon, kind: "mark", group: name, notes: "" } });
          for (const x of marks) if (x === m || (m.id && x.id === m.id)) { x.id = saved.id; x.name = saved.name; }
        } else if (known.lat !== m.lat || known.lon !== m.lon) {
          await invoke("mark_save", { mark: { ...known, lat: m.lat, lon: m.lon } });
        }
      }
      const sk = startRef && lib.find((x) => x.id === startRef.id);
      if (sk && (sk.lat !== start[1] || sk.lon !== start[0])) await invoke("mark_save", { mark: { ...sk, lon: start[0], lat: start[1] } });
      if (startRef && !sk) startRef = null; // deleted from the library meanwhile
      const r = await invoke("route_save", { route: { id: routeId ?? "", name, notes: "", start, start_mark: startRef?.id ?? null, marks: marks.map((m) => ({ mark: m.id, rounding: m.rounding || "" })) } });
      routeId = r.id;
      dirty = false;
      persist();
      drawMarks();
      status(`Saved “${r.name}” with ${marks.length} marks.`);
      await library.refresh();
    } catch (e) { status(`Could not save: ${e}`); }
  };
  $("rt-new").onclick = () => {
    start = null; startRef = null; marks = []; routeId = null; dirty = false; navIndex = 0; $("rt-name").value = "";
    arm(null); deselect(); persist(); drawMarks();
    for (const f of listeners) f();
  };

  // ---------- navigation ----------------------------------------------------------
  $("nav-prev").onclick = () => { navIndex = Math.max(0, navIndex - 1); persist(); drawMarks(); };
  $("nav-next").onclick = () => { navIndex = Math.min(Math.max(0, marks.length - 1), navIndex + 1); persist(); drawMarks(); };

  drawMarks();
  library.onChange(() => {
    // keep names of library marks in step with edits made in the Library panel
    for (const m of marks) { const x = m.id && library.byId(m.id); if (x) m.name = x.name; }
    if (startRef) { const x = library.byId(startRef.id); if (x) startRef.name = x.name; else startRef = null; }
    persist();
    drawMarks();
  });

  return {
    /** Start here; `lm` = a library mark to start at instead. */
    setStart: (p, lm) => setStartAt(p, lm),
    setStartLibraryMark: (m) => setStartAt(null, m),
    addMark: (p, lm) => {
      marks.push(lm ? { id: lm.id, name: lm.name, lon: lm.lon, lat: lm.lat, rounding: "" } : { id: null, name: "", lon: p[0], lat: p[1], rounding: "" });
      changed();
    },
    /** The selected mark for the right-click menu: { label, movable } or null. */
    selection: () => (selected === null ? null : { label: labelOf(selected), movable }),
    moveSelected,
    deleteSelected,
    current: () => ({ routeId, dirty }),
    addLibraryMark(m) { marks.push({ id: m.id, name: m.name, lon: m.lon, lat: m.lat, rounding: "" }); changed(); },
    libraryMarkChanged(m) {
      let hit = false;
      for (const x of marks) if (x.id === m.id) { x.name = m.name; x.lon = m.lon; x.lat = m.lat; hit = true; }
      if (startRef?.id === m.id) { startRef.name = m.name; start = [m.lon, m.lat]; hit = true; }
      if (hit) { persist(); drawMarks(); }
    },
    /** The course for routing: start, marks and their library links. */
    get: () => ({ start, startRef, marks: marks.map((m) => ({ ...m })), routeId, name: $("rt-name").value.trim() }),
    onChange: (f) => listeners.push(f),
    /** Routings on the chart: hide the dashed course legs. */
    setLegsHidden(v) { if (v !== legsHidden) { legsHidden = v; drawMarks(); } },
    stopEditing: () => { if (mode) arm(null); },
    /** Put back the course a routing was computed for. */
    /** A course from a list of named points (e.g. a YB race course): first = start. */
    fromPoints(points, name) {
      if (!points.length) return;
      start = [points[0].lon, points[0].lat];
      startRef = null;
      marks = points.slice(1).map((p) => ({ id: null, name: p.name || "", lon: p.lon, lat: p.lat, rounding: "" }));
      routeId = null;
      $("rt-name").value = name || "";
      dirty = true;
      navIndex = 0;
      persist();
      drawMarks();
      for (const f of listeners) f();
      fit([start, ...marks.map((m) => [m.lon, m.lat])]);
    },
    /** Start from a position, keeping the marks (e.g. route a competitor from its fix). */
    startAt(p) { start = p; startRef = null; changed(); },
    fromRouting(req) {
      const lib = library.marks();
      start = req.start ?? null;
      const sr = req.start_ref && lib.find((y) => y.id === req.start_ref.id);
      startRef = sr ? { id: sr.id, name: sr.name } : null;
      const refs = req.mark_refs ?? (req.marks ?? []).map((c) => ({ id: null, name: "", lon: c[0], lat: c[1] }));
      marks = refs.map((m) => {
        const now = m.id && lib.find((y) => y.id === m.id);
        return { id: now ? m.id : null, name: now ? now.name : m.name, lon: m.lon, lat: m.lat, rounding: m.rounding || "" };
      });
      routeId = req.route_id && lib.length ? req.route_id : null;
      $("rt-name").value = req.course_name || "";
      dirty = !routeId;
      navIndex = 0;
      persist();
      drawMarks();
    },
    /** The saved route was deleted: keep editing it as a new, unsaved one. */
    detach() { routeId = null; dirty = true; persist(); drawMarks(); },
    loadRoute(r, lib) {
      routeId = r.id;
      $("rt-name").value = r.name;
      const sm = r.start_mark && lib.find((m) => m.id === r.start_mark);
      start = sm ? [sm.lon, sm.lat] : r.start ?? null;
      startRef = sm ? { id: sm.id, name: sm.name } : null;
      marks = r.marks.map((x) => lib.find((m) => m.id === x.mark)).filter(Boolean)
        .map((m, i) => ({ id: m.id, name: m.name, lon: m.lon, lat: m.lat, rounding: r.marks[i]?.rounding || "" }));
      dirty = false;
      navIndex = 0;
      persist();
      drawMarks();
      for (const f of listeners) f();
      fit([start, ...marks.map((m) => [m.lon, m.lat])].filter(Boolean));
      status(`Opened “${r.name}”.`);
    },
    /** For the instruments: the leg being sailed and the routing's points. */
    nav() {
      const next = marks[navIndex];
      if (!next) return null;
      const from = navIndex > 0 ? [marks[navIndex - 1].lon, marks[navIndex - 1].lat] : start;
      return { next: { name: next.name || `Mark ${navIndex + 1}`, lon: next.lon, lat: next.lat }, from, index: navIndex, count: marks.length };
    },
    /** Boat moved: step on to the next mark once inside the arrival circle. */
    boatAt(pos) {
      const next = marks[navIndex];
      if (!next || navIndex >= marks.length - 1) return;
      if (distNm(pos, [next.lon, next.lat]) <= Math.max(getArrivalNm(), 0.05)) { navIndex += 1; persist(); drawMarks(); }
    },
  };

  function fit(pts) {
    if (!pts.length) return;
    const b = new maplibregl.LngLatBounds(pts[0], pts[0]);
    for (const p of pts) b.extend(p);
    const dock = document.body.classList.contains("dock-open") ? parseInt(getComputedStyle(document.documentElement).getPropertyValue("--dock-h")) || 0 : 0;
    map.fitBounds(b, { padding: { top: 60, bottom: 120 + dock, left: 420, right: 160 }, maxZoom: 13, duration: 600 });
  }
}

/** Mark icons drawn on demand: "mk:S" (start) or "mk:<n>". */
export function markImage(id) {
  if (!id.startsWith("mk:")) return null;
  const label = id.slice(3);
  const s = 44, c = document.createElement("canvas");
  c.width = s; c.height = s;
  const g = c.getContext("2d");
  g.beginPath(); g.arc(s / 2, s / 2, s / 2 - 3, 0, Math.PI * 2);
  g.fillStyle = label === "S" ? "#2e8b57" : "#c2185b"; g.fill();
  g.lineWidth = 3; g.strokeStyle = "#fff"; g.stroke();
  g.fillStyle = "#fff"; g.font = `bold ${label.length > 1 ? 18 : 22}px "Segoe UI", system-ui, sans-serif`;
  g.textAlign = "center"; g.textBaseline = "middle"; g.fillText(label, s / 2, s / 2 + 1);
  return g.getImageData(0, 0, s, s);
}

function pt(c, props) { return { type: "Feature", properties: props, geometry: { type: "Point", coordinates: c } }; }
function cap(s) { return s[0].toUpperCase() + s.slice(1); }
function ddm(deg, pos, neg) {
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}
