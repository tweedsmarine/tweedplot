// Course panel: the route being edited (start + marks, saved in the library)
// and navigation to the next mark. Routing lives in routing.js.
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { distNm } from "./instruments.js";
import { greatCircleGeometry, greatCircle as gcInitial, rhumb } from "./navmath.js";

const ACTIVE_KEY = "racenav.route.active"; // the route being navigated: its own copy, so another can be planned meanwhile
const WORK_KEY = "racenav.route.work"; // the route being edited survives a restart

function load(key, defaults) {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem(key) || "{}") }; } catch { return { ...defaults }; }
}
function save(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch { /* ignore */ } }

/** A circle of `nm` round `c` ([lon, lat]) as a closed line. */
function ring(c, nm) {
  const out = [];
  const k = Math.cos((c[1] * Math.PI) / 180);
  for (let i = 0; i <= 48; i++) {
    const a = (i / 48) * 2 * Math.PI;
    out.push([c[0] + (Math.sin(a) * nm) / 60 / k, c[1] + (Math.cos(a) * nm) / 60]);
  }
  return out;
}

export function setupRoute(map, { library, getBoat, getArrivalNm }) {
  const roundingRadius = () => Math.max(Number(getArrivalNm?.()) || 0.1, 0.02);
  const $ = (id) => document.getElementById(id);
  const listeners = [];
  let legsHidden = false;

  // The route being edited. Marks: { id (library id, or null until saved), name, lon, lat, rounding }.
  const work = load(WORK_KEY, { routeId: null, name: "", start: null, startRef: null, marks: [], dirty: false, nav: 0, greatCircle: false, startRings: null });
  let start = work.start; // [lon, lat]
  let startRef = work.startRef; // { id, name } when the start is a library mark
  let marks = work.marks;
  let routeId = work.routeId;
  let dirty = work.dirty;
  let navIndex = work.nav || 0;
  // The route being navigated (route monitoring), as its own copy: { routeId, name, start, startRef, startRings,
  // marks, greatCircle, nav (index of the next mark), linked }. `linked`: it is the route in the editor, so edits
  // there change it; opening another route or New unlinks it, and it stays navigated while the other is planned.
  // Only this route feeds the instruments, the highway and the alarms in the Navigation view.
  let active = load(ACTIVE_KEY, null);
  if (!active?.marks?.length) active = work.navOn === true && (work.marks ?? []).length
    ? { routeId: work.routeId ?? null, name: work.name || "", start: work.start ?? null, startRef: work.startRef ?? null, startRings: work.startRings ?? null,
        marks: structuredClone(work.marks), greatCircle: Boolean(work.greatCircle), nav: work.nav || 0, linked: true }
    : null;
  const linked = () => !!active?.linked;
  const snapshot = () => ({ routeId, name: $("rt-name").value, start, startRef, startRings, marks: structuredClone(marks), greatCircle });
  const saveActive = () => { try { if (active) localStorage.setItem(ACTIVE_KEY, JSON.stringify(active)); else localStorage.removeItem(ACTIVE_KEY); } catch { /* ignore */ } };
  /** Another route is about to take the editor: the navigated one keeps going as it last was. */
  function unlink() { if (active?.linked) { active.linked = false; saveActive(); } }
  function setActiveIndex(i) {
    if (!active) return;
    active.nav = Math.min(Math.max(0, active.marks.length - 1), Math.max(0, i));
    if (active.linked) navIndex = active.nav;
    persist(); drawMarks();
    for (const f of listeners) f();
  }
  let greatCircle = Boolean(work.greatCircle);
  // range rings of a start that isn't a library mark (marks keep theirs: m.rings, or the library's)
  let startRings = work.startRings || null;
  // the IHO look (S-52 planned route), with the IHO symbols (mariner.js colours)
  let iho = null; // null, or { colour(token) }
  let mode = null; // "start" | "marks" | null

  // ---------- layers ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  for (const id of ["rt-marks", "rt-legs", "rt-round"]) map.addSource(id, { type: "geojson", data: empty });
  // rounding: the clearance ring, red = leave to port, green = to starboard, black = as close as possible;
  // a grey dashed ring for an arrival radius of its own
  map.addLayer({ id: "rt-round", type: "line", source: "rt-round",
    paint: { "line-color": ["match", ["get", "r"], "port", "#c62828", "stbd", "#2e7d32", "close", "#1d2a33", "#5b6b75"], "line-width": 2,
             "line-dasharray": ["match", ["get", "r"], "port", ["literal", [1, 0]], "stbd", ["literal", [1, 0]], ["literal", [2, 2]]] } }, "track");
  map.addLayer({ id: "rt-legs", type: "line", source: "rt-legs",
    paint: { "line-color": "#c2185b", "line-width": ["case", ["get", "active"], 2.5, 1.2], "line-dasharray": [4, 3], "line-opacity": 0.8 } }, "track");
  map.addLayer({ id: "rt-marks", type: "symbol", source: "rt-marks",
    layout: { "icon-image": ["get", "icon"], "icon-allow-overlap": true, "icon-ignore-placement": true } });
  map.addLayer({ id: "rt-labels", type: "symbol", source: "rt-marks", filter: ["!=", ["get", "name"], ""],
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 12], "icon-allow-overlap": true } });
  // with the IHO symbols: each leg's course and distance (S-52 planned route), beside its middle
  map.addSource("s52-rt-leglbl", { type: "geojson", data: empty });
  map.addLayer({ id: "s52-rt-leglbl", type: "symbol", source: "s52-rt-leglbl",
    layout: { "icon-image": ["get", "img"], "icon-anchor": ["get", "anc"], "icon-offset": ["get", "off"], "icon-allow-overlap": false, "icon-padding": 2 } });

  // The navigated route when it is not the one in the editor: drawn on its own, heavier, and the editor's
  // route turns orange (the alternate planned route of S-52) so the two cannot be confused.
  map.addSource("rt-act-legs", { type: "geojson", data: empty });
  map.addSource("rt-act-marks", { type: "geojson", data: empty });
  map.addLayer({ id: "rt-act-legs", type: "line", source: "rt-act-legs",
    paint: { "line-color": "#c2185b", "line-width": ["case", ["get", "active"], 3, 1.8], "line-dasharray": [4, 3], "line-opacity": 0.9 } }, "track");
  map.addLayer({ id: "rt-act-marks", type: "circle", source: "rt-act-marks",
    paint: { "circle-radius": ["case", ["get", "next"], 7, 5], "circle-color": ["case", ["get", "next"], "#c2185b", "#ffffff"], "circle-stroke-color": "#c2185b", "circle-stroke-width": 2 } });
  map.addLayer({ id: "rt-act-labels", type: "symbol", source: "rt-act-marks",
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 12], "icon-allow-overlap": true } });
  function drawActive() {
    const show = !!active && !active.linked;
    const pts = show ? [active.start, ...active.marks.map((m) => [m.lon, m.lat])].filter(Boolean) : [];
    const off = show && active.start ? 1 : 0;
    const legs = [];
    for (let i = 1; i < pts.length; i++) {
      legs.push({ type: "Feature", properties: { active: i - off === active.nav },
        geometry: active.greatCircle ? greatCircleGeometry(pts[i - 1], pts[i]) : { type: "LineString", coordinates: [pts[i - 1], pts[i]] } });
    }
    map.getSource("rt-act-legs").setData({ type: "FeatureCollection", features: legs });
    map.getSource("rt-act-marks").setData({ type: "FeatureCollection", features: show
      ? active.marks.map((m, i) => pt([m.lon, m.lat], { next: i === active.nav, name: `${i + 1}${m.name ? ` ${m.name}` : ""}` })) : [] });
    map.setPaintProperty("rt-legs", "line-color", show ? "#e07b00" : "#c2185b");
  }

  function persist() {
    save(WORK_KEY, { routeId, name: $("rt-name").value, start, startRef, marks, dirty, nav: navIndex, greatCircle, startRings });
    // the navigated route follows the editor while it is the same route
    if (active?.linked) {
      if (marks.length) { navIndex = Math.min(navIndex, marks.length - 1); Object.assign(active, snapshot(), { nav: navIndex }); }
      else active = null;
    }
    saveActive();
  }
  function changed() {
    dirty = true;
    persist();
    drawMarks();
    for (const f of listeners) f();
  }

  /** With the IHO symbols: each leg's course (°T) and distance beside its middle. */
  function drawLegLabels() {
    const ll = [];
    if (iho && !legsHidden) {
      const ps = [start, ...marks.map((m) => [m.lon, m.lat])].filter(Boolean);
      for (let i = 1; i < ps.length; i++) {
        const a = ps[i - 1], b = ps[i];
        // a great-circle leg: its initial course
        const r = greatCircle ? gcInitial(a, b) : rhumb(a, b);
        if (r.nm <= 0) continue;
        // beside the line: to the right of a steep leg, above a flat one
        const pa = map.project(a), pb = map.project(b);
        const steep = Math.abs(pb.y - pa.y) > Math.abs(pb.x - pa.x);
        const brg = String(Math.round(r.brg) % 360).padStart(3, "0");
        ll.push(pt(rhumbMid(a, b), { img: `s52m:t:PLRTE:${brg}° ${r.nm < 10 ? r.nm.toFixed(2) : r.nm.toFixed(1)} NM`, anc: steep ? "left" : "bottom", off: steep ? [8, 0] : [0, -5] }));
      }
    }
    map.getSource("s52-rt-leglbl").setData({ type: "FeatureCollection", features: ll });
  }
  map.on("moveend", () => { if (iho) drawLegLabels(); }); // the side of the leg depends on the view

  function drawMarks() {
    const f = [];
    // IHO: waypoint circles (the next one doubled), numbers and names in the route colour
    const ihoLabel = (n, name) => `s52m:t:PLRTE:${n}${name ? ` ${name}` : ""}`;
    if (start) f.push(pt(start, iho ? { icon: "s52m:wp:PLRTE", sel: -1, name: "", lbl: ihoLabel("S", startRef?.name) } : { icon: "mk:S", sel: -1, name: startRef?.name || "" }));
    marks.forEach((m, i) => f.push(pt([m.lon, m.lat], iho ? { icon: linked() && i === navIndex ? "s52m:wpn:PLRTE" : "s52m:wp:PLRTE", sel: i, name: "", lbl: ihoLabel(i + 1, m.name) }
      : { icon: `mk:${i + 1}`, sel: i, name: m.name || "" })));
    map.getSource("rt-marks").setData({ type: "FeatureCollection", features: f });
    drawLegLabels();
    // legs, the one being sailed drawn heavier
    const pts = [start, ...marks.map((m) => [m.lon, m.lat])].filter(Boolean);
    const legs = [];
    for (let i = 1; i < pts.length; i++) {
      const leg = [pts[i - 1], pts[i]];
      legs.push({ type: "Feature", properties: { active: linked() && i - (start ? 1 : 0) === navIndex },
        geometry: greatCircle ? greatCircleGeometry(...leg) : { type: "LineString", coordinates: leg } });
    }
    map.getSource("rt-legs").setData({ type: "FeatureCollection", features: legsHidden ? [] : legs });
    map.getSource("rt-round").setData({ type: "FeatureCollection", features: marks.filter((m) => m.rounding || m.radius_nm > 0).map((m) => {
      const r = m.rounding === "close" ? 0.02 : m.radius_nm > 0 ? m.radius_nm : roundingRadius();
      return { type: "Feature", properties: { r: m.rounding || "radius" }, geometry: { type: "LineString", coordinates: ring([m.lon, m.lat], r) } };
    }) });
    library.hide([startRef?.id, ...marks.map((m) => m.id)].filter(Boolean));
    if (selected !== null && !posOf(selected)) deselect();
    drawSelection();
    renderList();
    drawActive();
  }

  $("rt-great-circle").checked = greatCircle;
  $("rt-great-circle").onchange = (e) => {
    greatCircle = e.target.checked;
    persist();
    drawMarks();
  };

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
      li.classList.toggle("active", linked() && i === navIndex);
      // how the mark is reached: through its arrival circle, rounded to port/starboard, or as close as possible
      const rr = document.createElement("div");
      rr.className = "rnd";
      rr.innerHTML = `<select title="How the routing reaches this mark"><option value="">Pass through</option><option value="port">Leave to port</option>` +
        `<option value="stbd">Leave to starboard</option><option value="close">As close as possible</option></select>` +
        `<input type="number" min="0" max="20" step="0.05" title="Arrival radius, or clearance when rounding (NM; empty = the routing's arrival radius)" placeholder="radius">`;
      const [sel, rad] = rr.children;
      sel.value = m.rounding || "";
      rad.value = m.radius_nm > 0 ? m.radius_nm : "";
      rad.disabled = m.rounding === "close";
      sel.onchange = () => { m.rounding = sel.value; changed(); };
      rad.onchange = () => { const v = Number(rad.value); m.radius_nm = Number.isFinite(v) && v > 0 ? v : 0; changed(); };
      li.querySelector(".main").append(rr);
      rr.onclick = (e) => e.stopPropagation();
      li.querySelector(".main").onclick = () => map.flyTo({ center: [m.lon, m.lat] });
      li.querySelector('[data-act="up"]').disabled = i === 0;
      li.querySelector('[data-act="down"]').disabled = i === marks.length - 1;
      li.querySelector('[data-act="up"]').onclick = () => { [marks[i - 1], marks[i]] = [marks[i], marks[i - 1]]; changed(); };
      li.querySelector('[data-act="down"]').onclick = () => { [marks[i + 1], marks[i]] = [marks[i], marks[i + 1]]; changed(); };
      li.querySelector('[data-act="x"]').onclick = () => { marks.splice(i, 1); if (navIndex >= marks.length) navIndex = Math.max(0, marks.length - 1); changed(); };
    });
    if (!start && !marks.length) ul.innerHTML = `<li class="empty">Set a start, then add marks: click the chart, or click a library mark to reuse it.</li>`;
    const an = active ? active.marks[active.nav] : null;
    const apart = !!active && !active.linked; // navigating one route while another is in the editor
    $("nav-to").textContent = active ? `${apart ? `${active.name || "Unsaved route"}: ` : ""}${active.nav + 1}. ${an?.name || `Mark ${active.nav + 1}`}` : marks.length ? "Not navigating" : "—";
    const go = $("nav-go");
    go.textContent = !active ? "Start navigating" : apart ? `Stop navigating ${active.name || "the other route"}` : "Stop navigating";
    go.classList.toggle("on", !!active);
    go.disabled = !active && !marks.length;
    $("nav-prev").disabled = $("nav-next").disabled = !active;
    $("nav-apart").hidden = !apart;
    $("nav-swap").disabled = !marks.length;
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
    map.getContainer().classList.toggle("drawing", !!mode);
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
  // the route being edited in another window: follow it, so every window shows and navigates the same route
  window.addEventListener("storage", (e) => {
    if (e.key !== WORK_KEY || !e.newValue) return;
    let w;
    try { w = JSON.parse(e.newValue); } catch { return; }
    start = w.start ?? null; startRef = w.startRef ?? null; marks = w.marks ?? []; routeId = w.routeId ?? null;
    dirty = Boolean(w.dirty); navIndex = w.nav || 0; greatCircle = Boolean(w.greatCircle);
    if (document.activeElement !== $("rt-name")) $("rt-name").value = w.name || "";
    $("rt-great-circle").checked = greatCircle;
    deselect();
    drawMarks();
    for (const f of listeners) f();
  });
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
          // its range rings go with it into the library
          const data = m.rings ? { rings: m.rings } : {};
          const saved = await invoke("mark_save", { mark: { name: m.name, lat: m.lat, lon: m.lon, kind: "mark", group: name, notes: "", data } });
          for (const x of marks) if (x === m || (m.id && x.id === m.id)) { x.id = saved.id; x.name = saved.name; delete x.rings; }
        } else if (known.lat !== m.lat || known.lon !== m.lon) {
          await invoke("mark_save", { mark: { ...known, lat: m.lat, lon: m.lon } });
        }
      }
      const sk = startRef && lib.find((x) => x.id === startRef.id);
      if (sk && (sk.lat !== start[1] || sk.lon !== start[0])) await invoke("mark_save", { mark: { ...sk, lon: start[0], lat: start[1] } });
      if (startRef && !sk) startRef = null; // deleted from the library meanwhile
      const r = await invoke("route_save", { route: { id: routeId ?? "", name, notes: "", start, start_mark: startRef?.id ?? null, marks: marks.map((m) => ({ mark: m.id, rounding: m.rounding || "", radius_nm: m.radius_nm || 0 })) } });
      routeId = r.id;
      dirty = false;
      persist();
      drawMarks();
      status(`Saved “${r.name}” with ${marks.length} marks.`);
      await library.refresh();
    } catch (e) { status(`Could not save: ${e}`); }
  };
  $("rt-new").onclick = () => {
    start = null; startRef = null; startRings = null; marks = []; routeId = null; dirty = false; navIndex = 0; unlink(); $("rt-name").value = "";
    arm(null); deselect(); persist(); drawMarks();
    for (const f of listeners) f();
  };

  // ---------- navigation ----------------------------------------------------------
  $("nav-prev").onclick = () => setActiveIndex((active?.nav ?? 0) - 1);
  $("nav-next").onclick = () => setActiveIndex((active?.nav ?? 0) + 1);

  const begin = () => { active = { ...snapshot(), nav: navIndex, linked: true }; };
  $("nav-go").onclick = () => {
    if (!active && !marks.length) return;
    const was = active?.name;
    if (active) active = null; else begin();
    persist(); drawMarks();
    for (const f of listeners) f();
    status(active ? `Navigating ${active.name || "this route"}: to mark ${active.nav + 1}.` : `Navigation${was ? ` of ${was}` : ""} stopped.`);
  };
  // navigating one route with another in the editor: navigate the editor's instead, or bring the navigated one back to edit
  $("nav-swap").onclick = () => {
    if (!marks.length) return;
    begin(); persist(); drawMarks();
    for (const f of listeners) f();
    status(`Navigating ${active.name || "this route"}: to mark ${active.nav + 1}.`);
  };
  $("nav-edit").onclick = () => {
    if (!active) return;
    start = active.start ?? null; startRef = active.startRef ?? null; startRings = active.startRings ?? null;
    marks = structuredClone(active.marks); routeId = active.routeId ?? null; greatCircle = Boolean(active.greatCircle);
    $("rt-name").value = active.name || ""; $("rt-great-circle").checked = greatCircle;
    navIndex = active.nav; dirty = true; active.linked = true;
    deselect(); persist(); drawMarks();
    for (const f of listeners) f();
    status(`${active.name || "The navigated route"} is in the editor again.`);
  };
  // the navigated route changed in another window
  window.addEventListener("storage", (e) => {
    if (e.key !== ACTIVE_KEY) return;
    try { active = e.newValue ? JSON.parse(e.newValue) : null; } catch { return; }
    if (active?.linked) navIndex = active.nav;
    drawMarks();
    for (const f of listeners) f();
  });

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
    /** Whether a map point is on the course being edited (its legs or marks). */
    hitAt(point) {
      return map.queryRenderedFeatures([[point.x - 5, point.y - 5], [point.x + 5, point.y + 5]], { layers: ["rt-legs", "rt-marks"] }).length > 0;
    },
    current: () => ({ routeId, dirty }),
    /** The waypoint under a map point: -1 the start, 0.. a mark; null if none. */
    waypointAt(point) {
      const f = map.queryRenderedFeatures([[point.x - 8, point.y - 8], [point.x + 8, point.y + 8]], { layers: ["rt-marks"] })[0];
      return f ? Number(f.properties.sel) : null;
    },
    /** A waypoint: { id (library mark, if any), name, pos, rings }. */
    waypoint(k) {
      if (k === -1) return start ? { id: startRef?.id ?? null, name: startRef?.name || "start", pos: start, rings: startRings } : null;
      const m = marks[k];
      return m ? { id: m.id, name: m.name, pos: [m.lon, m.lat], rings: m.rings ?? null } : null;
    },
    /** Range rings of a waypoint that isn't a library mark ({ n, step } or null). */
    setRings(k, r) {
      if (k === -1) startRings = r; else if (marks[k]) { if (r) marks[k].rings = r; else delete marks[k].rings; }
      persist();
    },
    /** Waypoints with range rings of their own (not library marks): [{ id, pos, rings }]. */
    ringCentres() {
      const out = [];
      if (start && startRings && !startRef) out.push({ id: null, pos: start, rings: startRings });
      for (const m of marks) if (m.rings && !m.id) out.push({ id: null, pos: [m.lon, m.lat], rings: m.rings });
      return out;
    },
    /** The IHO look on or off (S-52 planned route colours, waypoint circles, leg courses). */
    setIho(on, colour) {
      iho = on ? { colour } : null;
      const c = on ? colour("PLRTE") : "#c2185b";
      map.setPaintProperty("rt-legs", "line-color", c);
      map.setPaintProperty("rt-legs", "line-opacity", on ? 1 : 0.8);
      map.setPaintProperty("rt-legs", "line-dasharray", on ? ["literal", [1, 0]] : ["literal", [4, 3]]);
      map.setPaintProperty("rt-legs", "line-width", ["case", ["get", "active"], on ? 3 : 2.5, on ? 1.6 : 1.2]);
      map.setLayoutProperty("rt-labels", "icon-image", on ? ["get", "lbl"] : ["concat", "lbl:", ["get", "name"]]);
      map.setLayoutProperty("rt-labels", "icon-anchor", on ? "left" : "top");
      map.setLayoutProperty("rt-labels", "icon-offset", on ? ["literal", [14, 0]] : ["literal", [0, 12]]);
      map.setFilter("rt-labels", on ? ["has", "lbl"] : ["!=", ["get", "name"], ""]);
      drawMarks();
    },
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
      startRings = null;
      marks = points.slice(1).map((p) => ({ id: null, name: p.name || "", lon: p.lon, lat: p.lat, rounding: "" }));
      routeId = null;
      $("rt-name").value = name || "";
      dirty = true;
      navIndex = 0; unlink();
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
        return { id: now ? m.id : null, name: now ? now.name : m.name, lon: m.lon, lat: m.lat, rounding: m.rounding || "", radius_nm: m.radius_nm || 0 };
      });
      routeId = req.route_id && lib.length ? req.route_id : null;
      $("rt-name").value = req.course_name || "";
      dirty = !routeId;
      navIndex = 0; unlink();
      persist();
      drawMarks();
    },
    /** The saved route was deleted: keep editing it as a new, unsaved one. */
    detach() { routeId = null; dirty = true; persist(); drawMarks(); },
    loadRoute(r, lib) {
      routeId = r.id;
      startRings = null;
      $("rt-name").value = r.name;
      const sm = r.start_mark && lib.find((m) => m.id === r.start_mark);
      start = sm ? [sm.lon, sm.lat] : r.start ?? null;
      startRef = sm ? { id: sm.id, name: sm.name } : null;
      marks = r.marks.map((x) => [x, lib.find((m) => m.id === x.mark)]).filter(([, m]) => m)
        .map(([x, m]) => ({ id: m.id, name: m.name, lon: m.lon, lat: m.lat, rounding: x.rounding || "", radius_nm: x.radius_nm || 0 }));
      dirty = false;
      navIndex = 0; unlink();
      persist();
      drawMarks();
      for (const f of listeners) f();
      status(`Opened “${r.name}”.`);
    },
    /** For the instruments: the leg being sailed and the routing's points. */
    /** `plan`: the route as planned, whether or not it is being navigated (the Planning view's readings). */
    nav(plan = false) {
      // what is navigated; in Planning with nothing navigated, the route in the editor
      const r = active ?? (plan ? { marks, start, nav: navIndex } : null);
      const next = r?.marks[r.nav];
      if (!next) return null;
      const from = r.nav > 0 ? [r.marks[r.nav - 1].lon, r.marks[r.nav - 1].lat] : r.start;
      // the marks after the next one, for the data panel's list of marks ahead
      const ahead = r.marks.slice(r.nav + 1).map((m, j) => ({ name: m.name || `Mark ${r.nav + j + 2}`, lon: m.lon, lat: m.lat, n: r.nav + j + 2 }));
      return { next: { name: next.name || `Mark ${r.nav + 1}`, lon: next.lon, lat: next.lat, n: r.nav + 1 }, from, index: r.nav, count: r.marks.length, ahead };
    },
    /** Whether the route is being navigated (Start navigating), not only planned. */
    navigating: () => !!active,
    /** The navigated route: { name, linked (it is the one in the editor) } or null. */
    navigated: () => (active ? { name: active.name, linked: !!active.linked } : null),
    /** Boat moved: step on to the next mark once inside the arrival circle. */
    boatAt(pos) {
      const next = active?.marks[active.nav];
      if (!next || active.nav >= active.marks.length - 1) return;
      if (distNm(pos, [next.lon, next.lat]) <= Math.max(getArrivalNm(), 0.05)) setActiveIndex(active.nav + 1);
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

/** The middle of a leg (near enough for a label). */
function rhumbMid(a, b) { return [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]; }
function pt(c, props) { return { type: "Feature", properties: props, geometry: { type: "Point", coordinates: c } }; }
function cap(s) { return s[0].toUpperCase() + s.slice(1); }
function ddm(deg, pos, neg) {
  if (pos === "E" && Number.isFinite(deg)) deg = ((((deg + 180) % 360) + 360) % 360) - 180; // map past ±180
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}
