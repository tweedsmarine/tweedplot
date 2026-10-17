// Routing: settings and start time, compute (one GRIB or every loaded GRIB),
// the set of routings on the chart (this session's and saved ones), the active
// routing's summary, and the compare table in the dock.
//
// A routing here is the router's result plus what made it:
//   { key, name, colour, visible, savedId, stats,
//     r: { status, message, points, legs, isochrones, distance_nm, arrival, …,
//          request: { start, marks, t0, grib_path, grib_name, polar_path, params, … },
//          grib, t0 } }
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { routingStats, STATS, hm } from "./stats.js";
import { setupGates } from "./gates.js";

const OPTS_KEY = "racenav.route"; // kept from earlier versions: step, radius, current
const OPTS_DEFAULTS = { step: 0, radius: 0.1, current: true, v: 2, tmode: "slider", tfixed: null, hoverTime: false };
const COLS_KEY = "racenav.compare.cols";
const COLOURS = ["#c2185b", "#1565c0", "#2e7d32", "#ef6c00", "#6a1b9a", "#00838f", "#5d4037", "#455a64", "#d81b60", "#7cb342"];

function load(key, d) { try { return { ...d, ...JSON.parse(localStorage.getItem(key) || "{}") }; } catch { return { ...d }; } }
function save(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch { /* ignore */ } }

export function setupRouting(map, { weather, course, library, dock, ui, getBoat, getSafetyDepth, getAvoid, getBoatSettings }) {
  const $ = (id) => document.getElementById(id);
  const opts = load(OPTS_KEY, OPTS_DEFAULTS);
  if (opts.v !== 2) { opts.step = 0; opts.v = 2; }
  const items = []; // routings in this session's list
  let active = null; // key of the routing shown in detail
  let saved = []; // summaries of saved routings (library)
  let seq = 0;

  // ---------- messages: newest on top, one line each ----------------------------------------
  function msg(text, kind = "") {
    const ul = $("rt-log");
    const li = document.createElement("li");
    li.className = kind;
    const time = new Date().toLocaleTimeString("en-NZ", { hour12: false });
    li.innerHTML = `<time></time><span></span>`;
    li.children[0].textContent = time;
    li.children[1].textContent = text;
    ul.prepend(li);
    while (ul.children.length > 8) ul.lastChild.remove();
    // a "working…" line can be replaced by its outcome
    return { set(t, k = "") { li.className = k; li.children[1].textContent = t; } };
  }
  course.onChange(() => {
    const a = activeItem();
    if (a && courseKey(course.get()) !== a.courseKey) $("rt-stale").hidden = false;
  });

  // ---------- map layers ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  for (const id of ["rt-multi", "rt-iso", "rt-line", "rt-pts"]) map.addSource(id, { type: "geojson", data: empty });
  map.addLayer({ id: "rt-multi", type: "line", source: "rt-multi",
    paint: { "line-color": ["get", "colour"], "line-width": 2.2, "line-opacity": 0.85 } }, "track");
  map.addLayer({ id: "rt-iso", type: "line", source: "rt-iso",
    paint: { "line-color": "#4a5a64", "line-width": 1, "line-opacity": 0.7 } }, "track");
  map.addLayer({ id: "rt-line", type: "line", source: "rt-line",
    paint: { "line-color": ["get", "colour"], "line-width": 3.5 } }, "track");
  map.addLayer({ id: "rt-pts", type: "circle", source: "rt-pts",
    paint: { "circle-radius": 3, "circle-color": ["get", "colour"], "circle-stroke-color": "#fff", "circle-stroke-width": 1 } }, "track");

  // Where each shown routing has the boat at the weather slider's time: a
  // triangle in the routing's colour pointing along its heading.
  map.addSource("rt-now", { type: "geojson", data: empty });
  map.addImage("rt-tri", triangle(), { sdf: true, pixelRatio: 2 });
  map.addLayer({ id: "rt-now", type: "symbol", source: "rt-now",
    layout: { "icon-image": "rt-tri", "icon-rotate": ["get", "hdg"], "icon-rotation-alignment": "map",
              "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-size": ["case", ["get", "active"], 1.5, 1.25] },
    paint: { "icon-color": ["get", "colour"], "icon-halo-color": "#1d2a33", "icon-halo-width": 1.2,
             "icon-opacity": ["case", ["get", "done"], 0.55, 1] } });
  function drawNow() {
    const t = weather.current().t;
    const feats = [];
    if (t) for (const it of items) {
      if (!it.visible) continue;
      const p = positionAt(it.r.points, t);
      if (p) feats.push({ type: "Feature", properties: { key: it.key, colour: it.colour, hdg: p.heading, active: it.key === active, done: p.done, t },
        geometry: { type: "Point", coordinates: [p.lon, p.lat] } });
    }
    map.getSource("rt-now").setData({ type: "FeatureCollection", features: feats });
  }
  weather.onTime(drawNow);

  const activeItem = () => items.find((x) => x.key === active) ?? null;
  const drawListeners = [];
  function draw() {
    for (const fn of drawListeners) fn(active);
    const vis = items.filter((x) => x.visible && x.key !== active);
    map.getSource("rt-multi").setData({ type: "FeatureCollection", features: vis.map((x) => ({
      type: "Feature", properties: { key: x.key, colour: x.colour }, geometry: { type: "LineString", coordinates: x.r.points.map((p) => [p.lon, p.lat]) } })) });
    const a = activeItem();
    if (a && a.visible) {
      map.getSource("rt-line").setData({ type: "Feature", properties: { colour: a.colour }, geometry: { type: "LineString", coordinates: a.r.points.map((p) => [p.lon, p.lat]) } });
      map.getSource("rt-pts").setData({ type: "FeatureCollection", features: a.r.points.map((p, i) => ({ type: "Feature", properties: { i, colour: a.colour }, geometry: { type: "Point", coordinates: [p.lon, p.lat] } })) });
      const isos = a.r.isochrones ?? [];
      const every = Math.max(1, Math.ceil(isos.length / 40));
      map.getSource("rt-iso").setData({ type: "FeatureCollection", features: $("rt-iso-show").checked
        ? isos.filter((_, k) => k % every === 0).map((lines) => ({ type: "Feature", properties: {}, geometry: { type: "MultiLineString", coordinates: lines } })) : [] });
    } else {
      for (const id of ["rt-line", "rt-pts", "rt-iso"]) map.getSource(id).setData(empty);
    }
    course.setLegsHidden(items.some((x) => x.visible));
    drawNow();
    renderList();
    renderSummary();
    dock.refresh("compare");
  }
  $("rt-iso-show").onchange = draw;

  // ---------- settings ----------------------------------------------------------
  for (const [id, key, min] of [["rt-step", "step", 0], ["rt-radius", "radius", 0.001]]) {
    $(id).value = opts[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v) && v >= min) { opts[key] = v; save(OPTS_KEY, opts); } };
  }
  $("rt-current").checked = opts.current;
  $("rt-current").onchange = (e) => { opts.current = e.target.checked; save(OPTS_KEY, opts); };
  $("rt-hovertime").checked = opts.hoverTime;
  $("rt-hovertime").onchange = (e) => { opts.hoverTime = e.target.checked; save(OPTS_KEY, opts); };

  // ---------- start time ----------------------------------------------------------
  // "slider": the weather slider's time; "now": the clock when Compute is pressed;
  // "fixed": a time typed in (local time of this PC).
  const tmode = $("rt-tmode"), tinput = $("rt-time");
  tmode.value = opts.tmode;
  if (opts.tfixed) tinput.value = toLocalInput(opts.tfixed);
  function startTime() {
    if (opts.tmode === "now") return Math.floor(Date.now() / 60000) * 60;
    if (opts.tmode === "fixed") return opts.tfixed;
    return weather.current().t || null;
  }
  function showStart() {
    tinput.hidden = $("rt-time-now").hidden = opts.tmode !== "fixed";
    const t = startTime();
    const w = weather.current();
    const g = weather.all().find((x) => x.path === w.path);
    let note = t ? `Start ${fmt(t)}` : opts.tmode === "fixed" ? "Type a start time" : "Load a GRIB first";
    if (t && g && (t < g.start || t > g.end)) note += ` · outside the GRIB shown (${fmt(g.start, true)} → ${fmt(g.end, true)})`;
    $("rt-when").textContent = note;
    $("rt-when").classList.toggle("warn", !!(t && g && (t < g.start || t > g.end)));
  }
  tmode.onchange = () => {
    opts.tmode = tmode.value;
    if (opts.tmode === "fixed" && !opts.tfixed) { opts.tfixed = weather.current().t || Math.floor(Date.now() / 60000) * 60; tinput.value = toLocalInput(opts.tfixed); }
    save(OPTS_KEY, opts); showStart();
  };
  tinput.onchange = () => {
    const t = Date.parse(tinput.value); // datetime-local is local time
    if (Number.isFinite(t)) { opts.tfixed = Math.floor(t / 1000); save(OPTS_KEY, opts); }
    showStart();
  };
  $("rt-time-now").onclick = () => { opts.tfixed = Math.floor(Date.now() / 60000) * 60; tinput.value = toLocalInput(opts.tfixed); save(OPTS_KEY, opts); showStart(); };
  weather.onTime(showStart);
  setInterval(() => { if (opts.tmode === "now") showStart(); }, 30000);

  // ---------- compute ----------------------------------------------------------
  /** The router's request for this course and GRIB; `marks` (course marks, e.g. with a gate added) replaces the course's. */
  function request(grib, marks) {
    const c = course.get();
    if (marks) c.marks = marks;
    const b = getBoatSettings();
    return {
      start: c.start, marks: c.marks.map((m) => [m.lon, m.lat]), t0: startTime(), grib_path: grib.path, polar_path: b.polar, safety_depth: getSafetyDepth(), avoid: getAvoid?.() ?? { dangers: true, farms: true },
      params: { step_s: opts.step * 60, efficiency: b.eff / 100, tack_s: b.tack, gybe_s: b.gybe,
                arrival_nm: opts.radius, use_current: opts.current, keep_isochrones_every: 1 },
      // for saving and reopening: names and library links of the course
      grib_name: grib.name, course_name: c.name, route_id: c.routeId,
      start_ref: c.startRef ? { ...c.startRef } : null, mark_refs: c.marks.map((m) => ({ ...m })),
    };
  }
  function ready() {
    const c = course.get();
    if (!getBoatSettings().polar) { msg("Choose a polar in the Boat panel.", "warn"); return false; }
    if (!c.start || !c.marks.length) { msg("Set a start and at least one mark (Course panel).", "warn"); return false; }
    if (!startTime()) { msg("No start time: load a GRIB or set a time.", "warn"); return false; }
    return true;
  }
  async function computeOne(grib, nameExtra) {
    const req = request(grib);
    const line = msg(`Routing with ${grib.name}…`);
    try {
      const r = await invoke("route_compute", { req });
      const it = add({ ...r, request: req, grib: grib.name, t0: r.legs?.[0]?.start ?? req.t0 }, nameExtra);
      report(it, line);
      return it;
    } catch (e) { line.set(`Routing with ${grib.name} failed: ${e}`, "bad"); throw e; }
  }
  async function busy(fn) {
    for (const b of ["rt-go", "rt-all"]) $(b).disabled = true;
    try { await fn(); } catch { /* reported in the log */ } finally { for (const b of ["rt-go", "rt-all"]) $(b).disabled = false; }
  }
  $("rt-go").onclick = () => {
    const w = weather.current();
    if (!w.path) { msg("Load a GRIB in the Weather panel first.", "warn"); return; }
    if (!ready()) return;
    course.stopEditing();
    busy(() => computeOne({ path: w.path, name: w.name }));
  };
  $("rt-all").onclick = () => {
    const all = weather.all();
    if (!all.length) { msg("Load one or more GRIBs in the Weather panel first.", "warn"); return; }
    if (!ready()) return;
    course.stopEditing();
    busy(async () => {
      for (const g of all) await computeOne(g, g.name);
      msg(`${all.length} routings done: compare them in the table below.`, "good");
      dock.show("compare");
    });
  };

  function report(it, line) {
    const msg = (t, k) => line.set(t, k);
    const r = it.r, secs = ((r.compute_ms ?? 0) / 1000).toFixed(1);
    if (r.status === "arrived") {
      const av = r.request.avoid ?? {};
      const extra = [av.dangers !== false && "dangers", av.farms !== false && "marine farms"].filter(Boolean).join(", ");
      const charts = r.obstacles?.charts ? `, land, < ${r.request.safety_depth} m${extra ? `, ${extra}` : ""} avoided` : ", no chart: land not avoided";
      msg(`${it.name}: arrives ${fmt(r.arrival, true)} (${hm(r.arrival - r.t0)}), ${secs} s${charts}.`, "good");
    } else if (r.status === "grib end") {
      msg(`${it.name}: the weather runs out first. ${r.message}`, "warn");
    } else {
      msg(`${it.name}: ${r.status}. ${r.message}`, "bad");
    }
  }

  // A label for routings started from somewhere particular (a competitor's fix):
  // used while the course still starts there and the start time is unchanged.
  let runLabel = null; // { text, colour, start: [lon, lat], t }
  function labelFor(c, t0) {
    if (!runLabel || !c.start) return null;
    const same = Math.abs(c.start[0] - runLabel.start[0]) < 1e-6 && Math.abs(c.start[1] - runLabel.start[1]) < 1e-6;
    return same && t0 === runLabel.t ? runLabel : null;
  }
  function add(r, nameExtra) {
    seq += 1;
    const c = course.get();
    const used = new Set(items.map((x) => x.colour));
    const lab = labelFor(c, r.request?.t0);
    let name = lab ? `${lab.text}${nameExtra ? ` · ${nameExtra}` : ""}` : `${c.name || "Routing"} · ${nameExtra ?? fmt(r.t0, true)}`;
    if (items.some((x) => x.name === name)) { let n = 2; while (items.some((x) => x.name === `${name} (${n})`)) n++; name = `${name} (${n})`; }
    const it = {
      key: `k${seq}`,
      name,
      colour: (lab?.colour && !used.has(lab.colour) ? lab.colour : null) ?? COLOURS.find((x) => !used.has(x)) ?? COLOURS[seq % COLOURS.length],
      visible: true, savedId: null, r, stats: routingStats(r), courseKey: courseKey(c),
    };
    items.push(it);
    active = it.key;
    $("rt-stale").hidden = true;
    draw();
    return it;
  }

  // ---------- the active routing: summary ----------------------------------------------------------
  function renderSummary() {
    const a = activeItem();
    const body = document.querySelector("#rt-summary tbody");
    body.innerHTML = "";
    $("rt-active").textContent = a ? a.name : "No routing yet";
    $("rt-active-sw").style.background = a ? a.colour : "transparent";
    if (!a) return;
    const r = a.r, s = a.stats;
    const rows = [["Start", fmt(s.t0)]];
    if (r.status === "arrived") rows.push(["ETA", fmt(r.arrival)], ["Elapsed", hm(s.elapsed)]);
    else if (s.end) {
      rows.push([r.status === "grib end" ? "GRIB ends" : "Routing ends", fmt(s.end)], ["Time sailed", hm(s.elapsed)]);
      if (s.toGo != null) rows.push(["Still to go", `${s.toGo.toFixed(1)} NM`]);
    }
    rows.push(["Distance", `${s.distance.toFixed(1)} NM`]);
    if (s.avgSog) rows.push(["Average SOG", `${s.avgSog.toFixed(1)} kn`]);
    if (s.upwind != null) rows.push(["Up / reach / down", `${Math.round(s.upwind)} / ${Math.round(s.reach)} / ${Math.round(s.downwind)} %`]);
    if (s.maxTws != null) rows.push(["TWS range", `${s.minTws.toFixed(0)}–${s.maxTws.toFixed(0)} kn`]);
    rows.push(["Tacks / gybes", `${s.tacks} / ${s.gybes}`]);
    r.legs.forEach((l, i) => rows.push([`Leg ${i + 1}`, l.arrival ? `${l.distance_nm.toFixed(1)} NM, at ${fmt(l.arrival, true)}` : `${l.distance_nm.toFixed(1)} NM, not reached`]));
    const steps = [...new Set(r.legs.map((l) => l.step_s).filter(Boolean))].map((x) => (x >= 3600 ? `${+(x / 3600).toFixed(1)} h` : `${Math.round(x / 60)} min`));
    if (steps.length) rows.push(["Time step", (r.request?.params?.step_s ? "" : "auto, ") + steps.join(" / ")]);
    rows.push(["Weather", r.request?.grib_name ?? r.grib]);
    for (const [k, v] of rows) {
      const tr = document.createElement("tr");
      tr.innerHTML = "<td></td><td></td>";
      tr.children[0].textContent = k; tr.children[1].textContent = v;
      body.append(tr);
    }
  }

  // ---------- the list of routings ----------------------------------------------------------
  async function refreshSaved() {
    saved = (await invoke("routings_list").catch(() => [])) || [];
    renderList();
  }
  function renderList() {
    const ul = $("rt-list");
    ul.innerHTML = "";
    if (!items.length) ul.innerHTML = `<li class="empty">No routings yet. Compute one in Run, or show a saved one below.</li>`;
    for (const it of items) {
      const s = it.stats;
      const li = document.createElement("li");
      li.classList.toggle("active", it.key === active);
      li.innerHTML = `<input type="checkbox" title="Show on the chart"><span class="sw"></span><div class="main" title="Make this the routing shown in detail"><b></b><small></small></div>` +
        (it.savedId ? `<span class="tag" title="Saved in the library">saved</span>` : `<button data-act="save" title="Save in the library">Save</button>`) +
        `<button data-act="x" title="${it.savedId ? "Take off the list (stays saved)" : "Discard"}">✕</button>`;
      li.querySelector("input").checked = it.visible;
      li.querySelector("input").onchange = (e) => { it.visible = e.target.checked; draw(); };
      li.querySelector(".sw").style.background = it.colour;
      li.querySelector("b").textContent = it.name;
      li.querySelector("small").textContent = [it.r.status === "arrived" ? `ETA ${fmt(it.r.arrival, true)} · ${hm(s.elapsed)}` : it.r.status === "grib end" ? `GRIB ends · ${s.toGo?.toFixed(0)} NM to go` : it.r.status, `${s.distance.toFixed(1)} NM`].join(" · ");
      li.querySelector(".main").onclick = () => { active = it.key; it.visible = true; draw(); };
      li.querySelector('[data-act="save"]')?.addEventListener("click", () => saveItem(it));
      li.querySelector('[data-act="x"]').onclick = () => {
        items.splice(items.indexOf(it), 1);
        if (active === it.key) active = items.at(-1)?.key ?? null;
        draw();
      };
      ul.append(li);
    }
    // saved routings not on the list
    const sl = $("rt-saved");
    sl.innerHTML = "";
    const off = saved.filter((x) => !items.some((it) => it.savedId === x.id));
    if (!off.length) sl.innerHTML = `<li class="empty">${saved.length ? "All saved routings are on the list." : "No saved routings."}</li>`;
    for (const x of off) {
      const li = document.createElement("li");
      li.innerHTML = `<div class="main"><b></b><small></small></div><button data-act="show">Show</button>`;
      li.querySelector("b").textContent = x.name;
      li.querySelector("small").textContent = [x.t0 ? `start ${fmt(x.t0, true)}` : "", x.status === "arrived" ? `ETA ${fmt(x.arrival, true)}` : x.status, x.grib_name].filter(Boolean).join(" · ");
      li.querySelector('[data-act="show"]').onclick = () => showSaved(x.id);
      sl.append(li);
    }
  }
  async function showSaved(id) {
    try {
      const x = await invoke("routing_get", { id });
      const req = x.request || {};
      const r = { ...x.result, request: req, grib: req.grib_name ?? x.result.grib, t0: x.result.legs?.[0]?.start ?? req.t0 };
      const it = add(r);
      it.name = x.name;
      it.savedId = x.id;
      draw();
      msg(`Showing saved routing “${x.name}”.`);
      return it;
    } catch (e) { msg(String(e), "bad"); }
  }
  async function saveItem(it) {
    const { request, grib, t0, ...result } = it.r;
    try {
      const s = await invoke("routing_save", { routing: { name: it.name, route_id: request.route_id ?? "", route_name: request.course_name ?? "", request, result } });
      it.savedId = s.id;
      msg(`Saved “${it.name}”.`, "good");
      await refreshSaved();
      await library.refresh();
      draw();
    } catch (e) { msg(`Could not save the routing: ${e}`, "bad"); }
  }
  $("rt-keep").onclick = () => { const a = activeItem(); if (!a) msg("Compute a routing first.", "warn"); else if (a.savedId) msg(`“${a.name}” is already saved.`); else saveItem(a); };
  $("rt-showall").onclick = () => { for (const it of items) it.visible = true; draw(); };
  $("rt-hideall").onclick = () => { for (const it of items) it.visible = false; draw(); };
  $("rt-compare").onclick = () => dock.toggle("compare");
  $("rt-course").onclick = () => {
    const a = activeItem();
    if (!a) { msg("No routing shown.", "warn"); return; }
    course.fromRouting(a.r.request);
    $("rt-stale").hidden = true;
    msg(`Course set from “${a.name}”.`);
  };

  // GPX of the active routing, with times
  $("rt-gpx").onclick = async () => {
    const a = activeItem();
    if (!a || !a.r.points.length) { msg("Compute a routing first.", "warn"); return; }
    const name = `${a.name}`.replace(/[^\w .-]+/g, "-");
    const pts = a.r.points.map((p, i) =>
      `    <rtept lat="${p.lat.toFixed(6)}" lon="${p.lon.toFixed(6)}"><time>${new Date(p.t * 1000).toISOString()}</time><name>R${String(i).padStart(3, "0")}</name></rtept>`).join("\n");
    const gpx = `<?xml version="1.0" encoding="UTF-8"?>\n<gpx version="1.1" creator="racenav" xmlns="http://www.topografix.com/GPX/1/1">\n  <rte>\n    <name>${xml(a.name)}</name>\n${pts}\n  </rte>\n</gpx>\n`;
    try { msg(`Written to ${await invoke("gpx_write", { name, text: gpx })}`, "good"); } catch (e) { msg(`Could not write the GPX: ${e}`, "bad"); }
  };

  // ---------- hover and click on the chart ----------------------------------------------------------
  const popup = new maplibregl.Popup({ closeButton: false, closeOnClick: false, offset: 8 });
  map.on("mousemove", "rt-pts", (e) => {
    const a = activeItem();
    const p = a?.r.points[e.features[0].properties.i];
    if (!p) return;
    const side = p.twa >= 0 ? "stbd" : "port";
    popup.setLngLat([p.lon, p.lat]).setHTML(
      `<div class="rt-popup"><b>${fmt(p.t)}</b><br>HDG ${pad3(p.heading)}° · TWA ${Math.abs(p.twa).toFixed(0)}° ${side}<br>` +
      `TWS ${p.tws.toFixed(1)} kn from ${pad3(p.twd)}°<br>BSP ${p.bsp.toFixed(1)} kn · SOG ${p.sog.toFixed(1)} kn</div>`).addTo(map);
    if (opts.hoverTime && weather.setTime) weather.setTime(p.t);
  });
  map.on("mouseleave", "rt-pts", () => popup.remove());
  map.on("mousemove", "rt-now", (e) => {
    const f = e.features[0].properties;
    const it = items.find((x) => x.key === f.key);
    const p = it && positionAt(it.r.points, f.t);
    if (!p) return;
    const side = p.twa >= 0 ? "stbd" : "port";
    popup.setLngLat([p.lon, p.lat]).setHTML(`<div class="rt-popup"><b></b><br>${fmt(f.t)}${p.done ? " · finished" : ""}<br>` +
      `HDG ${pad3(p.heading)}° · TWA ${Math.abs(p.twa).toFixed(0)}° ${side}<br>TWS ${p.tws.toFixed(1)} kn · SOG ${p.sog.toFixed(1)} kn</div>`).addTo(map);
    popup.getElement().querySelector("b").textContent = it.name;
  });
  map.on("mouseleave", "rt-now", () => popup.remove());
  map.on("click", "rt-now", (e) => { const k = e.features[0].properties.key; if (items.some((x) => x.key === k)) { active = k; draw(); } });
  map.on("mouseenter", "rt-multi", (e) => {
    map.getCanvas().style.cursor = "pointer";
    const it = items.find((x) => x.key === e.features[0].properties.key);
    if (it) popup.setLngLat(e.lngLat).setHTML(`<div class="rt-popup"><b></b>Click to show in detail</div>`).addTo(map).getElement().querySelector("b").textContent = it.name;
  });
  map.on("mouseleave", "rt-multi", () => { map.getCanvas().style.cursor = ""; popup.remove(); });
  map.on("click", "rt-multi", (e) => {
    const k = e.features[0].properties.key;
    if (items.some((x) => x.key === k)) { active = k; draw(); }
  });

  // ---------- compare (dock view) ----------------------------------------------------------
  let cols = null;
  try { cols = JSON.parse(localStorage.getItem(COLS_KEY) || "null"); } catch { /* ignore */ }
  if (!Array.isArray(cols)) cols = STATS.filter((c) => c.def).map((c) => c.key);
  let sortKey = null, sortDir = 1;
  dock.register("compare", {
    title: "Compare routings",
    render(body, tools) {
      const shown = items.filter((x) => x.visible);
      const cog = document.createElement("button");
      cog.className = "cog";
      cog.title = "Choose the columns";
      cog.textContent = "⚙";
      cog.onclick = (e) => { e.stopPropagation(); colsMenu(cog); };
      tools.append(cog);
      // the GRIB shown on the chart and its time, so the Weather drawer can stay closed
      const wxBar = document.createElement("div");
      body.append(wxBar);
      weather.mountControls(wxBar);
      if (!shown.length) { body.insertAdjacentHTML("beforeend", `<p class="wx-note">Tick routings in Routing › Routings to compare them here.</p>`); return; }
      const defs = cols.map((k) => STATS.find((c) => c.key === k)).filter(Boolean);
      const val = (it, c) => { try { return c.get(it.stats, it.r); } catch { return null; } };
      let rows = [...shown];
      if (sortKey) {
        const c = STATS.find((x) => x.key === sortKey);
        rows.sort((a, b) => cmp(val(a, c), val(b, c)) * sortDir);
      }
      // best per column: among arrived routings when any arrived (times only compare like with like)
      const pool = shown.some((x) => x.r.status === "arrived") ? shown.filter((x) => x.r.status === "arrived") : shown;
      const best = {};
      for (const c of defs) {
        if (!c.best || pool.length < 2) continue;
        const vs = pool.map((x) => val(x, c)).filter((v) => typeof v === "number");
        if (vs.length) best[c.key] = c.best === "min" ? Math.min(...vs) : Math.max(...vs);
      }
      const t = document.createElement("table");
      t.className = "cmp";
      t.innerHTML = `<thead><tr><th>Routing</th>${defs.map((c) => `<th data-k="${c.key}" class="${c.key === sortKey ? (sortDir > 0 ? "asc" : "desc") : ""}">${esc(c.label)}</th>`).join("")}</tr></thead><tbody></tbody>`;
      for (const it of rows) {
        const tr = document.createElement("tr");
        tr.classList.toggle("active", it.key === active);
        tr.innerHTML = `<td class="nm"><span class="sw"></span><span></span></td>` + defs.map((c) => {
          const v = val(it, c);
          const isBest = best[c.key] !== undefined && v === best[c.key] && pool.includes(it);
          return `<td class="${isBest ? "best" : ""}">${v === null || v === undefined || v === "" ? "—" : esc(c.fmt(v))}</td>`;
        }).join("");
        tr.querySelector(".sw").style.background = it.colour;
        tr.querySelector(".nm span:last-child").textContent = it.name;
        tr.onclick = () => { active = it.key; draw(); };
        t.tBodies[0].append(tr);
      }
      t.tHead.onclick = (e) => {
        const k = e.target.closest("th")?.dataset.k;
        if (!k) return;
        if (sortKey === k) sortDir = -sortDir; else { sortKey = k; sortDir = 1; }
        dock.refresh("compare");
      };
      body.append(t);
    },
  });
  function colsMenu(anchor) {
    const m = $("cmp-menu");
    const groups = {};
    for (const c of STATS) (groups[c.group] ??= []).push(c);
    m.innerHTML = Object.entries(groups).map(([g, cs]) => `<div class="grp">${esc(g)}</div>` +
      cs.map((c) => `<label><input type="checkbox" data-k="${c.key}" ${cols.includes(c.key) ? "checked" : ""}> ${esc(c.label)}</label>`).join("")).join("") +
      `<hr><button data-act="def">Default columns</button>`;
    m.hidden = false;
    const r = anchor.getBoundingClientRect();
    m.style.left = `${Math.max(4, r.right - 260)}px`;
    m.style.top = `${Math.max(4, r.top - Math.min(m.scrollHeight, window.innerHeight * 0.8) - 6)}px`;
    m.onchange = (e) => {
      const k = e.target.dataset.k;
      if (!k) return;
      cols = e.target.checked ? STATS.map((c) => c.key).filter((x) => x === k || cols.includes(x)) : cols.filter((x) => x !== k);
      save(COLS_KEY, cols);
      dock.refresh("compare");
    };
    m.querySelector('[data-act="def"]').onclick = () => { cols = STATS.filter((c) => c.def).map((c) => c.key); save(COLS_KEY, cols); m.hidden = true; dock.refresh("compare"); };
  }
  document.addEventListener("pointerdown", (e) => { const m = $("cmp-menu"); if (!m.hidden && !m.contains(e.target) && !e.target.closest(".cog")) m.hidden = true; });

  refreshSaved();
  library.onChange(refreshSaved);
  showStart();
  draw();

  // gate sweep: routings through each of a row of gates (gates.js)
  setupGates(map, { weather, course, dock, ui, msg, ready, startTime,
    async compute(grib, marks, nameExtra) {
      const req = request(grib, marks);
      const r = await invoke("route_compute", { req });
      const it = add({ ...r, request: req, grib: grib.name, t0: r.legs?.[0]?.start ?? req.t0 }, nameExtra);
      report(it, msg(`${it.name}…`));
      return it;
    },
    item: (key) => items.find((x) => x.key === key) ?? null,
    activate(key) { const it = items.find((x) => x.key === key); if (it) { active = key; it.visible = true; draw(); } },
    setVisible(keys, only) { for (const it of items) { if (keys.includes(it.key)) it.visible = true; else if (only) it.visible = false; } draw(); },
    remove(keys) {
      for (const k of keys) { const i = items.findIndex((x) => x.key === k); if (i >= 0) items.splice(i, 1); }
      if (!items.some((x) => x.key === active)) active = items.at(-1)?.key ?? null;
      draw();
    },
    onDraw: (fn) => drawListeners.push(fn),
  });

  return {
    msg,
    /** Points of the active routing, for the instruments' targets. */
    activePoints: () => { const a = activeItem(); return a && a.visible ? a.r.points : null; },
    /** Show a saved routing (Library panel). */
    async showSaved(id) { const it = await showSaved(id); if (it) ui.openTab("routing", "list"); },
    /** Name (and colour) routings started from this position and time, e.g. "Guillou · fix 15 Oct 20:00Z". */
    setRunLabel(text, start, t, colour) { runLabel = { text, start, t: Math.round(t), colour }; },
    /** Use a set start time (e.g. a competitor's last fix). */
    setStartTime(t) {
      opts.tmode = "fixed"; opts.tfixed = Math.round(t);
      tmode.value = "fixed"; tinput.value = toLocalInput(opts.tfixed);
      save(OPTS_KEY, opts); showStart();
    },
  };
}

/**
 * The boat on a routing at time t: position along the leg being sailed, with
 * that leg's heading, TWA, TWS and SOG. After the finish it sits on the finish
 * (done = true); before the start or past a routing that stopped early, none.
 */
export function positionAt(points, t) {
  if (!points?.length || t < points[0].t) return null;
  const last = points.at(-1);
  if (t >= last.t) {
    if (t > last.t && points.length > 1) {
      const prev = points.at(-2);
      return { lon: last.lon, lat: last.lat, heading: prev.heading, twa: prev.twa, tws: prev.tws, sog: 0, done: true };
    }
    return { ...last, done: false };
  }
  let lo = 0, hi = points.length - 1;
  while (hi - lo > 1) { const m = (lo + hi) >> 1; if (points[m].t <= t) lo = m; else hi = m; }
  const a = points[lo], b = points[hi];
  const f = (t - a.t) / (b.t - a.t || 1);
  return { lon: a.lon + (b.lon - a.lon) * f, lat: a.lat + (b.lat - a.lat) * f, heading: a.heading, twa: a.twa, tws: a.tws, sog: a.sog, done: false };
}

/** Isosceles triangle pointing up (north before rotation), as an SDF mask. */
function triangle() {
  const w = 40, h = 48, c = document.createElement("canvas");
  c.width = w; c.height = h;
  const g = c.getContext("2d");
  g.beginPath(); g.moveTo(w / 2, 3); g.lineTo(w - 4, h - 4); g.lineTo(w / 2, h - 11); g.lineTo(4, h - 4); g.closePath();
  g.fillStyle = "#000"; g.fill();
  return g.getImageData(0, 0, w, h);
}

function courseKey(c) { return JSON.stringify([c.start, c.marks.map((m) => [m.lon, m.lat])]); }
function cmp(a, b) {
  if (a === b) return 0;
  if (a === null || a === undefined) return 1;
  if (b === null || b === undefined) return -1;
  return typeof a === "number" && typeof b === "number" ? a - b : String(a).localeCompare(String(b));
}
function toLocalInput(sec) {
  const d = new Date(sec * 1000);
  const z = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())}T${z(d.getHours())}:${z(d.getMinutes())}`;
}
function pad3(x) { return String(Math.round(x) % 360).padStart(3, "0"); }
function xml(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]); }
function esc(s) { return xml(s); }
