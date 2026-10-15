// Route panel: the route being edited (start + marks, saved in the library),
// navigation to the next mark, routing (compute, show, save, GPX).
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { distNm } from "./instruments.js";

const BOAT_DEFAULTS = { polar: "", eff: 100, tack: 60, gybe: 60 };
const ROUTE_DEFAULTS = { step: 0, radius: 0.1, current: true, v: 2 };
const WORK_KEY = "racenav.route.work"; // the route being edited survives a restart

function load(key, defaults) {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem(key) || "{}") }; } catch { return { ...defaults }; }
}
function save(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch { /* ignore */ } }

export function setupRoute(map, { weather, library, getBoat, getSafetyDepth }) {
  const $ = (id) => document.getElementById(id);
  const boat = load("racenav.boat", BOAT_DEFAULTS);
  const opts = load("racenav.route", ROUTE_DEFAULTS);
  if (opts.v !== 2) { opts.step = 0; opts.v = 2; save("racenav.route", opts); } // time step now defaults to auto

  // The route being edited. Marks: { id (library id, or null until saved), name, lon, lat, rounding }.
  const work = load(WORK_KEY, { routeId: null, name: "", start: null, marks: [], dirty: false, nav: 0 });
  let start = work.start; // [lon, lat]
  let marks = work.marks;
  let routeId = work.routeId;
  let dirty = work.dirty;
  let navIndex = work.nav || 0;
  let mode = null; // "start" | "marks" | null
  let last = null; // last routing shown

  // ---------- layers ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  for (const id of ["rt-iso", "rt-line", "rt-pts", "rt-marks", "rt-legs"]) map.addSource(id, { type: "geojson", data: empty });
  map.addLayer({ id: "rt-legs", type: "line", source: "rt-legs",
    paint: { "line-color": "#c2185b", "line-width": ["case", ["get", "active"], 2.5, 1.2], "line-dasharray": [4, 3], "line-opacity": 0.8 } }, "track");
  map.addLayer({ id: "rt-iso", type: "line", source: "rt-iso",
    paint: { "line-color": "#4a5a64", "line-width": 1, "line-opacity": 0.7 } }, "track");
  map.addLayer({ id: "rt-line", type: "line", source: "rt-line",
    paint: { "line-color": "#c2185b", "line-width": 2.5 } }, "track");
  map.addLayer({ id: "rt-pts", type: "circle", source: "rt-pts",
    paint: { "circle-radius": 3, "circle-color": "#c2185b", "circle-stroke-color": "#fff", "circle-stroke-width": 1 } }, "track");
  map.addLayer({ id: "rt-marks", type: "symbol", source: "rt-marks",
    layout: { "icon-image": ["get", "icon"], "icon-allow-overlap": true, "icon-ignore-placement": true } });
  map.addLayer({ id: "rt-labels", type: "symbol", source: "rt-marks", filter: ["!=", ["get", "name"], ""],
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 12], "icon-allow-overlap": true } });

  function persist() {
    save(WORK_KEY, { routeId, name: $("rt-name").value, start, marks, dirty, nav: navIndex });
  }
  function changed() {
    dirty = true;
    persist();
    drawMarks();
    if (last) $("rt-status").textContent = "Route changed: compute the routing again.";
  }

  function drawMarks() {
    const f = [];
    if (start) f.push(pt(start, { icon: "mk:S", sel: -1, name: "" }));
    marks.forEach((m, i) => f.push(pt([m.lon, m.lat], { icon: `mk:${i + 1}`, sel: i, name: m.name || "" })));
    map.getSource("rt-marks").setData({ type: "FeatureCollection", features: f });
    // legs, the one being sailed drawn heavier
    const pts = [start, ...marks.map((m) => [m.lon, m.lat])].filter(Boolean);
    const legs = [];
    for (let i = 1; i < pts.length; i++) legs.push({ type: "Feature", properties: { active: i - (start ? 1 : 0) === navIndex }, geometry: { type: "LineString", coordinates: [pts[i - 1], pts[i]] } });
    map.getSource("rt-legs").setData({ type: "FeatureCollection", features: last ? [] : legs });
    library.hide(marks.map((m) => m.id).filter(Boolean));
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
      const li = row("S", "s", "Start", ddm(start[1], "N", "S") + " " + ddm(start[0], "E", "W"), `<button data-act="x" title="Remove the start">✕</button>`);
      li.querySelector(".main").onclick = () => map.flyTo({ center: start });
      li.querySelector('[data-act="x"]').onclick = () => { start = null; changed(); };
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
  const labelOf = (k) => (k === -1 ? "start" : marks[k]?.name || `mark ${k + 1}`);
  const status = (s) => { $("rt-status").textContent = s; };
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
    if (selected === -1) start = null; else marks.splice(selected, 1);
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

  // ---------- boat settings ----------------------------------------------------------
  async function loadPolars() {
    const list = (await invoke("polar_files").catch(() => [])) || [];
    const sel = $("boat-polar");
    sel.innerHTML = "";
    if (!list.length) sel.append(new Option("No polars found", ""));
    for (const p of list) sel.append(new Option(p.name, p.path));
    if (boat.polar && list.some((p) => p.path === boat.polar)) sel.value = boat.polar;
    boat.polar = sel.value;
    save("racenav.boat", boat);
  }
  $("boat-polar").onchange = (e) => { boat.polar = e.target.value; save("racenav.boat", boat); };
  for (const [id, key] of [["boat-eff", "eff"], ["boat-tack", "tack"], ["boat-gybe", "gybe"]]) {
    $(id).value = boat[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v)) { boat[key] = v; save("racenav.boat", boat); } };
  }
  for (const [id, key, min] of [["rt-step", "step", 0], ["rt-radius", "radius", 0.001]]) {
    $(id).value = opts[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v) && v >= min) { opts[key] = v; save("racenav.route", opts); } };
  }
  $("rt-current").checked = opts.current;
  $("rt-current").onchange = (e) => { opts.current = e.target.checked; save("racenav.route", opts); };

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
    start = b; changed();
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
    if (mode === "start") { start = p; arm(null); changed(); return; }
    const lm = library.markAt(e.point);
    marks.push(lm ? { id: lm.id, name: lm.name, lon: lm.lon, lat: lm.lat, rounding: "" } : { id: null, name: "", lon: p[0], lat: p[1], rounding: "" });
    changed();
  });

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
      const r = await invoke("route_save", { route: { id: routeId ?? "", name, notes: "", start, marks: marks.map((m) => ({ mark: m.id, rounding: m.rounding || "" })) } });
      routeId = r.id;
      dirty = false;
      persist();
      drawMarks();
      status(`Saved “${r.name}” with ${marks.length} marks.`);
      await library.refresh();
    } catch (e) { status(`Could not save: ${e}`); }
  };
  $("rt-new").onclick = () => {
    start = null; marks = []; routeId = null; dirty = false; navIndex = 0; $("rt-name").value = "";
    clearRouting(); arm(null); deselect(); persist(); drawMarks();
  };

  // ---------- navigation ----------------------------------------------------------
  $("nav-prev").onclick = () => { navIndex = Math.max(0, navIndex - 1); persist(); drawMarks(); };
  $("nav-next").onclick = () => { navIndex = Math.min(Math.max(0, marks.length - 1), navIndex + 1); persist(); drawMarks(); };

  // Start time follows the weather slider.
  weather.onTime((t) => { $("rt-when").textContent = t ? `Start: ${fmt(t)}` : "Start: load a GRIB first"; });

  // ---------- compute ----------------------------------------------------------
  $("rt-go").onclick = async () => {
    const w = weather.current();
    if (!w.path) { status("Load a GRIB in the Weather panel first."); return; }
    if (!boat.polar) { status("Choose a polar in the Boat panel."); return; }
    if (!start || !marks.length) { status("Set a start and at least one mark."); return; }
    arm(null);
    $("rt-go").disabled = true;
    status(`Routing with ${w.name}…`);
    const request = {
      start, marks: marks.map((m) => [m.lon, m.lat]), t0: w.t, grib_path: w.path, polar_path: boat.polar, safety_depth: getSafetyDepth(),
      params: { step_s: opts.step * 60, efficiency: boat.eff / 100, tack_s: boat.tack, gybe_s: boat.gybe,
                arrival_nm: opts.radius, use_current: opts.current, keep_isochrones_every: 1 },
    };
    try {
      const r = await invoke("route_compute", { req: request });
      last = { ...r, grib: w.name, t0: r.legs[0]?.start ?? w.t, request: { ...request, grib_name: w.name, mark_refs: marks.map((m) => ({ ...m })) }, saved: null };
      show(last);
    } catch (e) {
      status(`Routing failed: ${e}`);
    } finally {
      $("rt-go").disabled = false;
    }
  };

  function clearRouting() {
    last = null;
    for (const id of ["rt-iso", "rt-line", "rt-pts"]) map.getSource(id).setData(empty);
    document.querySelector("#rt-summary tbody").innerHTML = "";
    drawMarks();
  }
  $("rt-clear").onclick = () => { clearRouting(); status(""); };

  function show(r) {
    const line = r.points.map((p) => [p.lon, p.lat]);
    map.getSource("rt-line").setData({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: line } });
    map.getSource("rt-pts").setData({ type: "FeatureCollection", features: r.points.map((p, i) => pt([p.lon, p.lat], { i })) });
    // Isochrones: each is a set of lines along the outer edge of the front.
    const every = Math.max(1, Math.ceil((r.isochrones?.length ?? 0) / 40));
    const iso = (r.isochrones ?? []).filter((_, k) => k % every === 0)
      .map((lines) => ({ type: "Feature", properties: {}, geometry: { type: "MultiLineString", coordinates: lines } }));
    map.getSource("rt-iso").setData({ type: "FeatureCollection", features: iso });
    drawMarks();

    const ok = r.status === "arrived";
    const gribEnd = r.status === "grib end";
    const end = r.points.length ? r.points[r.points.length - 1] : null;
    const toGo = end && r.legs.length ? distNm([end.lon, end.lat], r.legs[r.legs.length - 1].to) : null;
    const secs = ((r.compute_ms ?? 0) / 1000).toFixed(1);
    const charts = r.obstacles?.charts ? ` · land and < ${r.request?.safety_depth ?? getSafetyDepth()} m avoided (${r.obstacles.cell_m} m cells)` : " · no chart: land not avoided";
    const savedAs = r.saved ? `Saved routing “${r.saved}”. ` : "";
    status(savedAs + (ok ? `Routed in ${secs} s${charts}`
      : gribEnd ? `Weather ran out before the mark: the routing goes as far as the GRIB allows (${secs} s). ${r.message}`
      : `${r.status}: ${r.message}`));
    const dur = (t) => `${Math.floor((t - r.t0) / 3600)} h ${String(Math.round(((t - r.t0) % 3600) / 60)).padStart(2, "0")} min`;
    const rows = [];
    rows.push(["Start", fmt(r.t0)]);
    if (ok) {
      rows.push(["ETA", fmt(r.arrival)], ["Time", dur(r.arrival)]);
    } else if (end && r.points.length > 1) {
      rows.push([gribEnd ? "GRIB ends" : "Routing ends", fmt(end.t)], ["Time sailed", dur(end.t)]);
      if (toGo !== null) rows.push(["Still to go", `${toGo.toFixed(1)} NM to leg ${r.legs.length} mark`]);
    }
    rows.push(["Distance", `${r.distance_nm.toFixed(1)} NM`]);
    const tEnd = ok ? r.arrival : end?.t;
    if (tEnd > r.t0) rows.push(["Average speed", `${(r.distance_nm / ((tEnd - r.t0) / 3600)).toFixed(1)} kn`]);
    rows.push(["Tacks / gybes", `${r.legs.reduce((a, l) => a + l.tacks, 0)} / ${r.legs.reduce((a, l) => a + l.gybes, 0)}`]);
    r.legs.forEach((l, i) => rows.push([`Leg ${i + 1}`, l.arrival ? `${l.distance_nm.toFixed(1)} NM, at ${fmt(l.arrival, true)}` : `${l.distance_nm.toFixed(1)} NM, not reached`]));
    const steps = [...new Set(r.legs.map((l) => l.step_s).filter(Boolean))].map(stepText);
    if (steps.length) rows.push(["Time step", (r.request?.params?.step_s ? "" : "auto, ") + steps.join(" / ")]);
    rows.push(["Weather", r.grib]);
    const body = document.querySelector("#rt-summary tbody");
    body.innerHTML = "";
    for (const [k, v] of rows) {
      const tr = document.createElement("tr");
      tr.innerHTML = "<td></td><td></td>";
      tr.children[0].textContent = k; tr.children[1].textContent = v;
      body.append(tr);
    }
  }
  function stepText(s) { return s >= 3600 ? `${+(s / 3600).toFixed(1)} h` : `${Math.round(s / 60)} min`; }

  // ---------- save / load routings ----------------------------------------------------------
  $("rt-keep").onclick = async () => {
    if (!last) { status("Compute a routing first."); return; }
    const routeName = $("rt-name").value.trim();
    const name = `${routeName || "Routing"} · ${fmt(last.t0, true)}`;
    const { request, saved, ...result } = last;
    try {
      const s = await invoke("routing_save", { routing: { name, route_id: routeId ?? "", route_name: routeName, request, result } });
      last.saved = s.name;
      show(last);
      await library.refresh();
    } catch (e) { status(`Could not save the routing: ${e}`); }
  };

  // ---------- hover a routing point: what the boat is doing there ----------------------------
  const popup = new maplibregl.Popup({ closeButton: false, closeOnClick: false, offset: 8 });
  map.on("mousemove", "rt-pts", (e) => {
    const i = e.features[0].properties.i;
    const p = last?.points[i];
    if (!p) return;
    const side = p.twa >= 0 ? "stbd" : "port";
    popup.setLngLat([p.lon, p.lat]).setHTML(
      `<div class="rt-popup"><b>${fmt(p.t)}</b><br>HDG ${pad3(p.heading)}° · TWA ${Math.abs(p.twa).toFixed(0)}° ${side}<br>` +
      `TWS ${p.tws.toFixed(1)} kn from ${pad3(p.twd)}°<br>BSP ${p.bsp.toFixed(1)} kn · SOG ${p.sog.toFixed(1)} kn</div>`).addTo(map);
    if (weather.setTime) weather.setTime(p.t);
  });
  map.on("mouseleave", "rt-pts", () => popup.remove());

  // ---------- GPX of the routing ----------------------------------------------------------
  $("rt-gpx").onclick = async () => {
    if (!last || !last.points.length) { status("Compute a routing first."); return; }
    const name = `${$("rt-name").value.trim() || "racenav"} ${new Date(last.t0 * 1000).toISOString().slice(0, 16).replace("T", " ")}Z`;
    const pts = last.points.map((p, i) =>
      `    <rtept lat="${p.lat.toFixed(6)}" lon="${p.lon.toFixed(6)}"><time>${new Date(p.t * 1000).toISOString()}</time><name>R${String(i).padStart(3, "0")}</name></rtept>`).join("\n");
    const gpx = `<?xml version="1.0" encoding="UTF-8"?>\n<gpx version="1.1" creator="racenav" xmlns="http://www.topografix.com/GPX/1/1">\n  <rte>\n    <name>${xml(name)}</name>\n${pts}\n  </rte>\n</gpx>\n`;
    try { status(`Written to ${await invoke("gpx_write", { name: name.replace(/[: ]/g, "-"), text: gpx })}`); }
    catch (e) { status(`Could not write the GPX: ${e}`); }
  };

  loadPolars();
  drawMarks();
  library.onChange(() => {
    // keep names of library marks in step with edits made in the Library panel
    for (const m of marks) { const x = m.id && library.byId(m.id); if (x) m.name = x.name; }
    persist();
    drawMarks();
  });

  return {
    setStart: (p) => { start = p; changed(); },
    addMark: (p) => { marks.push({ id: null, name: "", lon: p[0], lat: p[1], rounding: "" }); changed(); },
    /** The selected mark for the right-click menu: { label, movable } or null. */
    selection: () => (selected === null ? null : { label: labelOf(selected), movable }),
    moveSelected,
    deleteSelected,
    current: () => ({ routeId, dirty }),
    addLibraryMark(m) { marks.push({ id: m.id, name: m.name, lon: m.lon, lat: m.lat, rounding: "" }); changed(); },
    libraryMarkChanged(m) {
      let hit = false;
      for (const x of marks) if (x.id === m.id) { x.name = m.name; x.lon = m.lon; x.lat = m.lat; hit = true; }
      if (hit) { persist(); drawMarks(); }
    },
    /** The saved route was deleted: keep editing it as a new, unsaved one. */
    detach() { routeId = null; dirty = true; persist(); drawMarks(); },
    loadRoute(r, lib) {
      routeId = r.id;
      $("rt-name").value = r.name;
      start = r.start ?? start;
      marks = r.marks.map((x) => lib.find((m) => m.id === x.mark)).filter(Boolean)
        .map((m, i) => ({ id: m.id, name: m.name, lon: m.lon, lat: m.lat, rounding: r.marks[i]?.rounding || "" }));
      dirty = false;
      navIndex = 0;
      clearRouting();
      persist();
      drawMarks();
      fit([start, ...marks.map((m) => [m.lon, m.lat])].filter(Boolean));
      status(`Opened “${r.name}”.`);
    },
    loadRouting(x, lib) {
      const req = x.request || {};
      start = req.start ?? null;
      const refs = req.mark_refs ?? (req.marks ?? []).map((c) => ({ id: null, name: "", lon: c[0], lat: c[1] }));
      marks = refs.map((m) => {
        const now = m.id && lib.find((y) => y.id === m.id);
        return { id: now ? m.id : null, name: now ? now.name : m.name, lon: m.lon, lat: m.lat, rounding: m.rounding || "" };
      });
      routeId = x.route_id && lib.length ? x.route_id : null;
      $("rt-name").value = x.route_name || "";
      dirty = !routeId;
      navIndex = 0;
      last = { ...x.result, grib: req.grib_name ?? x.result.grib, t0: x.result.legs?.[0]?.start ?? req.t0, request: req, saved: x.name };
      persist();
      show(last);
      fit(last.points.map((p) => [p.lon, p.lat]).concat(marks.map((m) => [m.lon, m.lat])));
    },
    /** For the instruments: the leg being sailed and the routing's points. */
    nav() {
      const next = marks[navIndex];
      if (!next) return null;
      const from = navIndex > 0 ? [marks[navIndex - 1].lon, marks[navIndex - 1].lat] : start;
      return { next: { name: next.name || `Mark ${navIndex + 1}`, lon: next.lon, lat: next.lat }, from, index: navIndex, count: marks.length, points: last?.points ?? null };
    },
    /** Boat moved: step on to the next mark once inside the arrival circle. */
    boatAt(pos) {
      const next = marks[navIndex];
      if (!next || navIndex >= marks.length - 1) return;
      if (distNm(pos, [next.lon, next.lat]) <= Math.max(opts.radius, 0.05)) { navIndex += 1; persist(); drawMarks(); }
    },
  };

  function fit(pts) {
    if (!pts.length) return;
    const b = new maplibregl.LngLatBounds(pts[0], pts[0]);
    for (const p of pts) b.extend(p);
    map.fitBounds(b, { padding: { top: 60, bottom: 120, left: 420, right: 160 }, maxZoom: 13, duration: 600 });
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
function pad3(x) { return String(Math.round(x) % 360).padStart(3, "0"); }
function cap(s) { return s[0].toUpperCase() + s.slice(1); }
function xml(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]); }
function ddm(deg, pos, neg) {
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}
