// Library drawer: the library in use (its name and the Library menu: new, open, merge,
// import, export, backups), and its marks, routes and routings, each in the same list
// (liblist.js: search, filters, sort, selection and actions). Tracks and barriers are in
// tracks.js and barriers.js, in the same list. The data is in the library's database,
// kept by the Rust side.
import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
import { fmt } from "./weather.js";
import { distNm } from "./instruments.js";
import { POI_KINDS, poiImage } from "./poi.js";
import { iconSelect, imageUrl } from "./iconselect.js";
import { featureImage } from "./chartfeatures.js";
import { libList, monthTest, dateTest } from "./liblist.js";
import { tripFilter, markInTrip, onTrips } from "./tripstore.js";
import { addKindOptions, symImage, SYMBOL_KINDS } from "./marksymbols.js";

const KEY = "racenav.library";

/** Library mark kinds drawn as IALA A buoys: kind -> chart symbol id (see chartfeatures.js). */
export const BUOY_ICONS = {
  port: "b|2|3||", starboard: "b|1|4||",
  north: "b|4|2,6|1|n", east: "b|4|2,6,2|1|e", south: "b|4|6,2|1|s", west: "b|4|6,2,6|1|w",
  isolated: "b|4|2,3,2|1|isd", safewater: "b|3|3,1|2|saw", special: "b|4|6||spp",
};

export function setupLibrary(map, { ui, toast = () => {} }) {
  const $ = (id) => document.getElementById(id);
  let marks = [];
  let routes = [];
  let routings = [];
  let route = null; // the course editor (set by main once both exist)
  let routing = null;
  let editing = null; // mark being edited
  let prefs = { show: true, chartFiltered: false };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  delete prefs.filter; // the filters belong to the list now
  const savePrefs = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  const listeners = [];
  const status = (s) => { $("lib-status").textContent = s; };

  // ---------- chart layers ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  map.addSource("lib-marks", { type: "geojson", data: empty });
  // marks given a buoy kind are drawn with the chart symbol (chartfeatures.js); the circle stays as an
  // invisible click target so selecting and routing to them works the same
  const isBuoy = ["has", ["get", "kind"], ["literal", BUOY_ICONS]];
  const isPoi = ["has", ["get", "kind"], ["literal", Object.fromEntries(Object.keys(POI_KINDS).map((k) => [k, 1]))]];
  // shapes, seabed and fish symbols ("sym-…"), drawn in the mark's own colour
  const isSym = ["==", ["slice", ["get", "kind"], 0, 4], "sym-"];
  const isIcon = ["any", isBuoy, isPoi, isSym];
  map.addLayer({ id: "lib-marks", type: "circle", source: "lib-marks",
    paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], 5, ["case", isIcon, 11, 2.5], 10, ["case", isIcon, 11, 5]],
             // a mark's own colour (from TimeZero and other plotters), else by its type
             "circle-color": ["coalesce", ["get", "colour"], ["match", ["get", "kind"], "danger", "#c0392b", "waypoint", "#5b6b75", "#e67e22"]],
             "circle-opacity": ["case", isIcon, 0, 1], "circle-stroke-opacity": ["case", isIcon, 0, 1],
             "circle-stroke-color": "#fff", "circle-stroke-width": ["interpolate", ["linear"], ["zoom"], 5, 0.5, 10, 1.5] } }, "track");
  map.addLayer({ id: "lib-buoys", type: "symbol", source: "lib-marks", filter: isBuoy,
    layout: { "icon-image": ["concat", "s57:", ["get", ["get", "kind"], ["literal", BUOY_ICONS]]], "icon-anchor": "bottom", "icon-offset": [0, 3],
              "icon-allow-overlap": true, "icon-ignore-placement": true } }, "lib-marks");
  map.addLayer({ id: "lib-poi", type: "symbol", source: "lib-marks", filter: isPoi,
    layout: { "icon-image": ["concat", "poi:", ["get", "kind"]], "icon-allow-overlap": true, "icon-ignore-placement": true } }, "lib-marks");
  map.addLayer({ id: "lib-sym", type: "symbol", source: "lib-marks", filter: isSym,
    layout: { "icon-image": ["concat", "sym:", ["get", "kind"], ":", ["coalesce", ["get", "colour"], ""]], "icon-size": 0.8, "icon-allow-overlap": true, "icon-ignore-placement": true } }, "lib-marks");
  map.addLayer({ id: "lib-labels", type: "symbol", source: "lib-marks", minzoom: 8, filter: ["!=", ["get", "name"], ""],
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 6], "icon-allow-overlap": false } }, "track");
  let hidden = new Set();
  let markList = null; // made below (draw() can run before)
  function draw() {
    const vis = prefs.show ? "visible" : "none";
    map.setLayoutProperty("lib-marks", "visibility", vis);
    map.setLayoutProperty("lib-labels", "visibility", vis);
    map.setLayoutProperty("lib-buoys", "visibility", vis);
    map.setLayoutProperty("lib-poi", "visibility", vis);
    map.setLayoutProperty("lib-sym", "visibility", vis);
    // every mark, or only those the list shows ("Chart shows only the marks listed")
    const shown = prefs.chartFiltered && markList ? markList.filtered() : marks;
    map.getSource("lib-marks").setData({ type: "FeatureCollection", features: shown.filter((m) => !hidden.has(m.id)).map((m) => ({
      type: "Feature", properties: m.colour ? { id: m.id, name: m.name, kind: m.kind, colour: m.colour } : { id: m.id, name: m.name, kind: m.kind }, geometry: { type: "Point", coordinates: [m.lon, m.lat] } })) });
  }
  map.on("mouseenter", "lib-marks", () => { if (!dragging) map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "lib-marks", () => { if (!dragging) map.getCanvas().style.cursor = ""; });

  // ---------- selecting and moving marks on the chart ----------------------------------
  // As course marks and barrier points: click a mark (amber ring), wait for the ring to
  // turn green, then drag it. The new position is saved to the library (and so to every
  // route using the mark). Clicks while placing course marks or drawing go to those.
  const ARM_MS = 1200;
  let sel = null, movable = false, armTimer = null, dragging = false, dragPos = null;
  map.addSource("lib-sel", { type: "geojson", data: empty });
  map.addLayer({ id: "lib-sel", type: "circle", source: "lib-sel",
    paint: { "circle-radius": 15, "circle-color": "rgba(0,0,0,0)", "circle-stroke-width": 4, "circle-stroke-color": "#f0a500" } }, "lib-marks");
  function drawSel() {
    const m = sel && (dragPos ? { lon: dragPos[0], lat: dragPos[1] } : byId(sel));
    map.getSource("lib-sel").setData(m ? { type: "Feature", properties: {}, geometry: { type: "Point", coordinates: [m.lon, m.lat] } } : empty);
    map.setPaintProperty("lib-sel", "circle-stroke-color", movable ? "#2e8b57" : "#f0a500");
    map.setPaintProperty("lib-sel", "circle-color", dragPos ? "rgba(46,139,87,0.45)" : "rgba(0,0,0,0)");
  }
  function selectMark(id) {
    clearTimeout(armTimer);
    sel = id; movable = false; drawSel();
    const m = byId(id);
    toast(`${m?.name || "Mark"} selected: wait for the ring to turn green, then drag it. Esc to let go.`);
    armTimer = setTimeout(() => { movable = true; drawSel(); toast(`Drag ${m?.name || "the mark"} to move it.`); }, ARM_MS);
  }
  function deselectMark() { clearTimeout(armTimer); if (sel) toast(""); sel = null; movable = false; drawSel(); }
  const busy = () => document.querySelector(".armed");
  map.on("click", (e) => {
    if (busy() || !prefs.show) return;
    const f = map.queryRenderedFeatures([[e.point.x - 6, e.point.y - 6], [e.point.x + 6, e.point.y + 6]], { layers: ["lib-marks"] })[0];
    if (f) { if (f.properties.id !== sel) selectMark(f.properties.id); }
    else if (sel) deselectMark();
  });
  map.on("mousedown", "lib-marks", (e) => {
    const id = e.features[0].properties.id;
    if (busy() || e.originalEvent.button !== 0 || !movable || id !== sel) return;
    e.preventDefault();
    map.dragPan.disable();
    dragging = true;
    map.getCanvas().style.cursor = "grabbing";
  });
  map.on("mousemove", (e) => {
    if (!dragging) return;
    dragPos = [e.lngLat.lng, e.lngLat.lat];
    const m = byId(sel);
    if (m) { m.lon = dragPos[0]; m.lat = dragPos[1]; }
    drawSel(); // only the ring follows the pointer: redrawing tens of thousands of marks each move is too slow
  });
  map.on("mouseup", async () => {
    if (!dragging) return;
    dragging = false;
    map.dragPan.enable();
    map.getCanvas().style.cursor = "";
    const m = byId(sel);
    dragPos = null;
    if (!m) return;
    try {
      const saved = await invoke("mark_save", { mark: m });
      route?.libraryMarkChanged(saved);
      toast(`Moved ${saved.name || "the mark"} to ${ddm(saved.lat, "N", "S")} ${ddm(saved.lon, "E", "W")}.`, 4000);
    } catch (err) { toast(`Could not save the mark: ${err}`, 6000); }
    await refresh();
    drawSel();
  });
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && sel && document.getElementById("ctx")?.hidden !== false) { deselectMark(); }
  });

  // ---------- data ----------------------------------------------------------
  async function refresh() {
    try {
      [marks, routes, routings] = await Promise.all([invoke("marks_list"), invoke("routes_list"), invoke("routings_list")]);
      marks ||= []; routes ||= []; routings ||= [];
      index();
    } catch (e) { status(`Library: ${e}`); }
    if (sel && !byId(sel)) deselectMark(); // the selected mark was deleted
    if (editing?.id && !byId(editing.id)) closeForm();
    routeList.setItems(routes);
    routingList.setItems(routings);
    markList.setItems(marks); // draws the chart too (changed)
    renderDupes();
    loadSymbols();
    showName();
    for (const f of listeners) f();
  }

  // ---------- routes tab ----------------------------------------------------------
  function routeLength(r) {
    const sm = r.start_mark && byId(r.start_mark);
    const pts = [sm ? [sm.lon, sm.lat] : r.start, ...r.marks.map((x) => byId(x.mark)).filter(Boolean).map((m) => [m.lon, m.lat])].filter(Boolean);
    let d = 0;
    for (let i = 1; i < pts.length; i++) d += distNm(pts[i - 1], pts[i]);
    return d;
  }
  // routes brought in more than once (same name, same marks in the same order): keep the oldest copy
  function duplicateRoutes() {
    const seen = new Map(), extra = [];
    for (const r of [...routes].sort((a, b) => (a.modified || 0) - (b.modified || 0))) {
      const k = `${r.name.toLowerCase()}|${r.start_mark || ""}|${r.marks.map((x) => x.mark).join(",")}`;
      if (seen.has(k)) extra.push(r); else seen.set(k, r);
    }
    return extra;
  }
  $("lib-dupes-go").onclick = async () => {
    const extra = duplicateRoutes();
    $("lib-dupes-go").disabled = true;
    let n = 0;
    const job = activity.start("Removing duplicate routes");
    const fails = [];
    for (const r of extra) {
      try { await invoke("route_delete", { id: r.id }); n++; } catch (e) { fails.push(`${r.name}: ${e}`); }
      job.progress("Routes", n + fails.length, extra.length);
    }
    $("lib-dupes-go").disabled = false;
    if (fails.length) job.fail(`Removed ${n} of ${extra.length} duplicate routes`, { details: fails });
    else job.done(`Removed ${n} duplicate route${n === 1 ? "" : "s"}.`);
    refresh();
  };
  $("lib-dupes-go").onclick = async () => {
    const extra = duplicateRoutes();
    $("lib-dupes-go").disabled = true;
    let n = 0;
    const job = activity.start("Removing duplicate routes");
    const fails = [];
    for (const r of extra) {
      try { await invoke("route_delete", { id: r.id }); n++; } catch (e) { fails.push(`${r.name}: ${e}`); }
      job.progress("Routes", n + fails.length, extra.length);
    }
    $("lib-dupes-go").disabled = false;
    if (fails.length) job.fail(`Removed ${n} of ${extra.length} duplicate routes`, { details: fails });
    else job.done(`Removed ${n} duplicate route${n === 1 ? "" : "s"}.`);
    refresh();
  };
  function renderDupes() {
    const extra = duplicateRoutes();
    $("lib-dupes").hidden = !extra.length;
    $("lib-dupes").querySelector("span").textContent = `${extra.length} route${extra.length === 1 ? " is a copy" : "s are copies"} of another (same name and marks), from importing a file more than once.`;
  }
  const routeSummary = (r) => `${r.start ? "start + " : ""}${r.marks.length} marks · ${routeLength(r).toFixed(1)} NM · ${[r.start_mark ? byId(r.start_mark)?.name : null, ...r.marks.map((x) => byId(x.mark)?.name || "?")].filter(Boolean).join(" → ")}`;
  const exportGpx = async (name, markIds, routeIds, what) => {
    try {
      const p = await invoke("gpx_export", { name, markIds, routeIds });
      activity.note(`${what} written as GPX`, { details: [`Written to ${p}`] });
    } catch (e) { activity.start(`Writing ${what} as GPX`).fail(String(e)); }
  };
  const routeList = libList($("lib-routes-list"), {
    key: "routes",
    noun: ["route", "routes"],
    search: (r, q) => r.name.toLowerCase().includes(q) || (r.notes || "").toLowerCase().includes(q),
    sorts: [
      { key: "new", label: "Newest first", cmp: (a, b) => (b.modified || 0) - (a.modified || 0) },
      { key: "name", label: "By name", cmp: (a, b) => collate(a.name, b.name) },
      { key: "len", label: "Longest first", cmp: (a, b) => routeLength(b) - routeLength(a) },
    ],
    filters: [
      { key: "imported", label: "From a file", type: "toggle", title: "Only routes brought in from GPX or TimeZero", test: (r) => !!r.import_id || /^Rte |^Imported route/.test(r.name) },
    ],
    row: {
      render(r, el) {
        el.innerHTML = `<div class="main" title="Open this route in the Route drawer"><b></b><small></small></div><button data-act="open">Open</button>`;
        el.querySelector("b").textContent = r.name;
        el.querySelector("small").textContent = routeSummary(r);
        el.querySelector('[data-act="open"]').onclick = () => openRoute(r);
      },
      update(r, row) { row.classList.toggle("active", route?.current().routeId === r.id); },
      click: (r) => openRoute(r),
    },
    actions: [
      { label: "GPX", title: "Write the selected routes (and their marks) as one GPX file", run: (rs) => exportGpx(rs.length === 1 ? rs[0].name : "routes", [], rs.map((r) => r.id), rs.length === 1 ? rs[0].name : `${rs.length} routes`) },
      { label: "Library file…", title: "Write the selected routes, with their marks, as a library file of their own", run: (rs) => exportLibraryFile({ routes: rs.map((r) => r.id) }, rs.length === 1 ? `“${rs[0].name}”` : `${rs.length} routes`, rs.length === 1 ? rs[0].name : "routes") },
      { label: "Delete", danger: true, title: "Delete the selected routes (their marks stay)", confirm: (rs) => `Delete ${rs.length === 1 ? `“${rs[0].name}”` : `${rs.length} routes`}? Their marks stay in the library.`,
        run: async (rs) => {
          let n = 0;
          for (const r of rs) { try { await invoke("route_delete", { id: r.id }); n++; if (route?.current().routeId === r.id) route.detach(); } catch { /* reported below */ } }
          activity.note(`Deleted ${n} route${n === 1 ? "" : "s"}`);
          refresh();
        } },
    ],
    empty: () => "No saved routes. Make one in the Route drawer and press Save, or import a GPX or TimeZero file (Library menu).",
  });
  function openRoute(r) { route?.loadRoute(r, marks); ui.open("course"); }

  // ---------- marks tab ----------------------------------------------------------
  const COURSE = new Set(["mark", "waypoint", "start", "finish"]);
  // the mark editor's Type list shows each type's symbol as the chart draws it
  let kindPicker = null;
  const DOTS = { danger: "#c0392b", waypoint: "#5b6b75" };
  function kindIcon(k, colour = "") {
    if (SYMBOL_KINDS[k]) return imageUrl(symImage(`sym:${k}:${colour || ""}`));
    if (BUOY_ICONS[k]) return imageUrl(featureImage(`s57:${BUOY_ICONS[k]}`));
    if (POI_KINDS[k]) return imageUrl(poiImage(`poi:${k}`));
    const c = document.createElement("canvas"); c.width = c.height = 16;
    const g = c.getContext("2d");
    g.beginPath(); g.arc(8, 8, 5.5, 0, 7); g.fillStyle = DOTS[k] || "#e67e22"; g.fill();
    g.lineWidth = 1.5; g.strokeStyle = "#fff"; g.stroke(); g.lineWidth = 0.8; g.strokeStyle = "rgba(0,0,0,0.35)"; g.beginPath(); g.arc(8, 8, 6.6, 0, 7); g.stroke();
    return c.toDataURL();
  }
  const markType = (m) => (COURSE.has(m.kind) || !m.kind ? "course" : m.kind === "danger" ? "danger" : BUOY_ICONS[m.kind] ? "buoy" : POI_KINDS[m.kind] ? "poi" : "course");
  const inRoute = (m) => usedBy.has(m.id);
  const collate = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" }).compare;
  const inView = (m) => {
    const b = map.getBounds();
    const lon = b.getEast() - b.getWest() >= 360 ? null : ((m.lon - b.getWest()) % 360 + 360) % 360;
    return !(m.lat < b.getSouth() || m.lat > b.getNorth() || (lon != null && lon > b.getEast() - b.getWest()));
  };
  const KIND_LABEL = () => Object.fromEntries([...form.kind.querySelectorAll("option")].map((o) => [o.value, o.textContent]));
  const countBy = (list, key, label = (k) => k) => {
    const c = new Map();
    for (const x of list) for (const k of [].concat(key(x))) if (k != null) c.set(k, (c.get(k) || 0) + 1);
    return [...c].sort((a, b) => b[1] - a[1]).map(([k, n]) => [k, label(k), n]);
  };
  const tagsOf = (m) => (m.tags ? m.tags.split(",").filter(Boolean) : []);
  const TAG_LABEL = { "at-boat": "made at the boat", depth: "depth in its name", event: "event marks", unsure: "unsure (?)", caution: "caution (!)" };
  const markSub = (m) => {
    const used = usedBy.get(m.id) || 0;
    const t = tagsOf(m).filter((x) => x !== "at-boat" && x !== "depth");
    return [m.group, `${ddm(m.lat, "N", "S")} ${ddm(m.lon, "E", "W")}`,
      m.created ? (m.date_unknown ? "date unknown" : new Date(m.created * 1000).toLocaleDateString()) : "",
      m.depth != null ? `${Math.round(m.depth)} m` : "", m.temp != null ? `${m.temp.toFixed(1)} °C` : "",
      t.join(", "), used ? `in ${used} route${used > 1 ? "s" : ""}` : ""].filter(Boolean).join(" · ");
  };
  // row icons, made once per type and colour (thousands of rows share a few)
  const dotIcons = new Map();
  const dotIcon = (k, c) => { const key = `${k}|${SYMBOL_KINDS[k] ? c || "" : ""}`; if (!dotIcons.has(key)) dotIcons.set(key, kindIcon(k, SYMBOL_KINDS[k] ? c : "")); return dotIcons.get(key); };
  const dot = (m) => {
    const i = document.createElement("span");
    i.className = "ll-dot";
    if (m.colour && !BUOY_ICONS[m.kind] && !POI_KINDS[m.kind] && !SYMBOL_KINDS[m.kind]) i.style.background = m.colour;
    else { i.style.backgroundImage = `url(${dotIcon(m.kind, m.colour)})`; i.classList.add("icon"); }
    return i;
  };
  markList = libList($("lib-marks-list"), {
    key: "marks",
    noun: ["mark", "marks"],
    search: (m, q) => m.name.toLowerCase().includes(q) || (m.group || "").toLowerCase().includes(q) || (m.notes || "").toLowerCase().includes(q) || (m.tags || "").includes(q),
    sorts: [
      { key: "name", label: "By name", cmp: (a, b) => collate(a.group, b.group) || (!a.name - !b.name) || collate(a.name, b.name) || (b.created || 0) - (a.created || 0) },
      { key: "new", label: "Newest first", cmp: (a, b) => (b.date_unknown ? 0 : b.created || 0) - (a.date_unknown ? 0 : a.created || 0) },
      { key: "old", label: "Oldest first", cmp: (a, b) => (a.date_unknown || !a.created ? Infinity : a.created) - (b.date_unknown || !b.created ? Infinity : b.created) },
      { key: "deep", label: "Deepest first", cmp: (a, b) => (b.depth ?? -1) - (a.depth ?? -1) },
      { key: "near", label: "Nearest the chart centre", cmp: (a, b) => { const c = map.getCenter(); return distNm([c.lng, c.lat], [a.lon, a.lat]) - distNm([c.lng, c.lat], [b.lon, b.lat]); } },
    ],
    filters: [
      { key: "group", label: "Group", type: "choice", options: (ms) => countBy(ms, (m) => m.group || "", (g) => g || "(no group)"), test: (m, v) => (m.group || "") === v },
      { key: "type", label: "Type", type: "choice", options: (ms) => { const L = KIND_LABEL(); return countBy(ms, (m) => m.kind || "mark", (k) => L[k] || k); }, test: (m, v) => (m.kind || "mark") === v },
      { key: "months", label: "Months", type: "multi", months: true, title: "Only marks made in these months, any year (e.g. Nov, Dec, Jan for the tuna season)",
        note: "Any year. Marks with an unknown date are left out.", test: (m, v) => !m.date_unknown && monthTest(m.created, v) },
      { key: "dates", label: "Dates", type: "dates", title: "Only marks made between two dates", note: "Marks with an unknown date are left out.", test: (m, v) => !m.date_unknown && dateTest(m.created, v) },
      tripFilter(markInTrip),
      { key: "tag", label: "Tag", type: "choice", options: (ms) => countBy(ms, tagsOf, (t) => TAG_LABEL[t] || t), test: (m, v) => tagsOf(m).includes(v) },
      { key: "symbol", label: "TimeZero symbol", type: "choice", options: (ms) => countBy(ms.filter((m) => m.symbol?.startsWith("tz:")), (m) => m.symbol, (s) => `Symbol ${s.slice(3)}`), test: (m, v) => m.symbol === v },
      { key: "route", label: "Routes", type: "choice", options: () => [["used", "In a saved route"], ["unused", "In no route"]], test: (m, v) => inRoute(m) === (v === "used") },
      { key: "unknown", label: "Date unknown", type: "toggle", title: "Only marks whose date is only the day they were moved in from another plotter", test: (m) => !!m.date_unknown },
      { key: "view", label: "In chart view", type: "toggle", title: "Only the marks in the area shown on the chart", test: (m) => inView(m) },
    ],
    row: {
      render(m, el) {
        el.innerHTML = `<div class="main" title="Show it on the chart"><b></b><small></small></div><button data-act="add" title="Add to the end of the route being edited">+ Route</button><button data-act="edit">Edit</button>`;
        el.prepend(dot(m));
        el.querySelector("b").textContent = m.name || "(no name)";
        el.querySelector("b").classList.toggle("unnamed", !m.name);
        el.querySelector("small").textContent = markSub(m);
        el.querySelector('[data-act="add"]').onclick = () => { route?.addLibraryMark(m); status(`${m.name || "The mark"} added to the route.`); };
        el.querySelector('[data-act="edit"]').onclick = () => editMark(m);
      },
      update(m, row) { row.classList.toggle("editing", editing?.id === m.id); },
      click: (m) => map.flyTo({ center: [m.lon, m.lat], zoom: Math.max(map.getZoom(), 11) }),
    },
    actions: [
      { label: "GPX", title: "Write the selected marks as one GPX file", run: (ms) => exportGpx(ms.length === 1 ? (ms[0].name || "mark") : "marks", ms.map((m) => m.id), [], `${ms.length.toLocaleString()} mark${ms.length === 1 ? "" : "s"}`) },
      { label: "Library file…", title: "Write the selected marks as a library file of their own (to give another boat, or keep apart)", run: (ms) => exportLibraryFile({ marks: ms.map((m) => m.id) }, `${ms.length.toLocaleString()} mark${ms.length === 1 ? "" : "s"}`, ms.length === 1 ? ms[0].name || "mark" : "marks") },
      { label: "Group…", title: "Move the selected marks to a group", input: { type: "text", label: "Group", list: () => [...new Set(marks.map((m) => m.group).filter(Boolean))].sort(collate), value: (ms) => (new Set(ms.map((m) => m.group)).size === 1 ? ms[0].group : "") },
        run: (ms, v) => bulk(ms, { group: (v ?? "").trim() }, `moved to ${v?.trim() ? `“${v.trim()}”` : "no group"}`) },
      { label: "Type…", title: "Give the selected marks a type (a symbol)", input: { type: "select", label: "Type", select: () => kindSelect(), icons: (v) => kindIcon(v) },
        run: (ms, v) => bulk(ms, { kind: v }, `now ${(KIND_LABEL()[v] || v).toLowerCase()}`) },
      { label: "Colour…", title: "Give the selected marks a colour", input: { type: "colour", label: "Colour", value: (ms) => ms[0].colour || "#e67e22" },
        run: (ms, v) => bulk(ms, { colour: v }, `coloured ${v}`) },
      { label: "Route", title: "Make a route of the selected marks, in the order listed (opens in the Route drawer)", run: (ms) => {
        route?.loadRoute({ id: "", name: "", notes: "", start: null, start_mark: null, marks: ms.map((m) => ({ mark: m.id, rounding: "", radius_nm: 0 })) }, marks);
        ui.open("course");
      } },
      { label: "Delete", danger: true, title: "Delete the selected marks", confirm: (ms) => {
          const inR = ms.filter((m) => usedBy.has(m.id)).length;
          return `Delete ${ms.length === marks.length ? "all " : ""}${ms.length.toLocaleString()} mark${ms.length === 1 ? "" : "s"}?${inR ? ` ${inR.toLocaleString()} ${inR === 1 ? "is" : "are"} in saved routes and will be taken out of them.` : ""}`;
        },
        run: async (ms) => {
          const inR = ms.filter((m) => usedBy.has(m.id)).length;
          try {
            const n = await invoke("marks_delete", { ids: ms.map((m) => m.id), force: true });
            activity.note(`Deleted ${n.toLocaleString()} mark${n === 1 ? "" : "s"}`, { details: inR ? [`${inR.toLocaleString()} were taken out of saved routes`] : [] });
          } catch (e) { activity.start("Deleting marks").fail(`Could not delete: ${e}`); }
          refresh();
        } },
    ],
    empty: () => "No marks yet. Right-click the chart: New library mark here, or import a GPX or TimeZero file (Library menu).",
    changed: () => draw(),
  });
  async function bulk(ms, change, what) {
    try {
      const n = await invoke("marks_update", { ids: ms.map((m) => m.id), ...change });
      activity.note(`${n.toLocaleString()} mark${n === 1 ? "" : "s"} ${what}`);
    } catch (e) { activity.start("Changing marks").fail(String(e)); }
    refresh();
  }
  // the chart-view filter follows the chart
  let viewTimer = null;
  map.on("moveend", () => {
    if (!markList.filters.view && markList.sort !== "near") return;
    clearTimeout(viewTimer);
    viewTimer = setTimeout(() => markList.refresh(), 200);
  });
  $("lib-show").checked = prefs.show;
  $("lib-show").onchange = (e) => { prefs.show = e.target.checked; savePrefs(); draw(); };
  $("lib-f-chart").checked = !!prefs.chartFiltered;
  $("lib-f-chart").onchange = (e) => { prefs.chartFiltered = e.target.checked; savePrefs(); draw(); };

  // ---------- the mark editor (above the list) ----------
  const form = $("mark-edit");
  addKindOptions(form.kind, POI_KINDS); // every type: course marks, buoys, points of interest, shapes, seabed, fish
  /** A copy of the full type list, for the selection's Type… and the TimeZero symbols. */
  const kindSelect = () => { const s = form.kind.cloneNode(true); s.removeAttribute("name"); return s; };
  function closeForm() {
    form.hidden = true;
    editing = null;
    markList.redraw();
  }
  function editMark(m) {
    editing = { ...m };
    form.hidden = false;
    form.name.value = m.name;
    form.kind.value = m.kind || "mark";
    kindPicker ??= iconSelect(form.kind, kindIcon);
    kindPicker.refresh();
    form.group.value = m.group || "";
    form.lat.value = ddm(m.lat, "N", "S");
    form.lon.value = ddm(m.lon, "E", "W");
    form.notes.value = m.notes || "";
    form.barrier.value = m.barrier_nm > 0 ? m.barrier_nm : "";
    $("mark-made").textContent = [m.created ? `Made ${m.date_unknown ? "on an unknown date" : new Date(m.created * 1000).toLocaleString()}` : "",
      m.depth != null ? `depth ${m.depth} m` : "", m.temp != null ? `sea ${m.temp.toFixed(1)} °C` : "", m.sog != null ? `${m.sog.toFixed(1)} kn` : "",
      m.symbol ? (m.symbol.startsWith("tz:") ? `TimeZero symbol ${m.symbol.slice(3)}` : `symbol ${m.symbol}`) : "",
      tagsOf(m).length ? `tags: ${tagsOf(m).map((t) => TAG_LABEL[t] || t).join(", ")}` : ""].filter(Boolean).join(" · ");
    markList.redraw();
    form.scrollIntoView({ block: "nearest", behavior: "smooth" });
    form.name.focus();
  }
  form.onsubmit = async (e) => {
    e.preventDefault();
    const lat = parseCoord(form.lat.value, "NS"), lon = parseCoord(form.lon.value, "EW");
    if (lat === null || Math.abs(lat) > 90) { status("Latitude not understood: try 36 50.12 S or -36.8353"); return; }
    if (lon === null || Math.abs(lon) > 180) { status("Longitude not understood: try 174 49.8 E or 174.83"); return; }
    const bn = Number(form.barrier.value);
    const m = { ...editing, name: form.name.value.trim(), kind: form.kind.value, group: form.group.value.trim(), notes: form.notes.value.trim(), lat, lon,
      barrier_nm: Number.isFinite(bn) && bn > 0 ? bn : 0 };
    try {
      const saved = await invoke("mark_save", { mark: m });
      closeForm();
      status(`Saved ${saved.name}.`);
      route?.libraryMarkChanged(saved);
      await refresh();
    } catch (err) { status(String(err)); }
  };
  form.querySelector('[data-act="cancel"]').onclick = () => closeForm();
  form.querySelector('[data-act="start"]').onclick = () => { if (editing) { route?.setStartLibraryMark(editing); status(`The route now starts at ${editing.name || "the mark"}.`); } };
  form.querySelector('[data-act="delete"]').onclick = async (e) => {
    const b = e.currentTarget;
    if (!editing) return;
    if (!b.dataset.armed) { b.dataset.armed = "1"; b.textContent = usedBy.has(editing.id) ? "Delete (in routes)?" : "Delete it?"; setTimeout(() => { delete b.dataset.armed; b.textContent = "Delete"; }, 4000); return; }
    delete b.dataset.armed; b.textContent = "Delete";
    try { await invoke("mark_delete", { id: editing.id, force: true }); activity.note(`Deleted ${editing.name || "the mark"}`); } catch (err) { status(String(err)); }
    closeForm();
    refresh();
  };

  // ---------- TimeZero symbols: what each one means --------------------------------------
  let symbols = [];
  const KIND_CHOICES = () => [...form.kind.querySelectorAll("option")].map((o) => [o.value, o.textContent]);
  async function loadSymbols() {
    try { symbols = await invoke("symbols_list"); } catch { symbols = []; }
    $("lib-symbols").hidden = !symbols.length;
    renderSymbols();
  }
  function renderSymbols() {
    const ul = $("lib-sym-list");
    if (!$("lib-symbols").open) return;
    ul.innerHTML = "";
    for (const sy of symbols) {
      const li = document.createElement("li");
      const showing = markList.filters.symbol === sy.symbol;
      li.classList.toggle("active", showing);
      li.innerHTML = `<div class="main"><b></b><small></small></div><span class="chips"></span><select title="What this symbol means: every mark with it becomes this type"></select><button data-act="show"></button>`;
      li.querySelector("b").textContent = `Symbol ${sy.symbol.slice(3)}${sy.label ? ` · ${sy.label}` : ""}`;
      li.querySelector("small").textContent = `${sy.count.toLocaleString()} mark${sy.count === 1 ? "" : "s"}`;
      const chips = li.querySelector(".chips");
      for (const [c] of sy.colours.slice(0, 5)) {
        const i = document.createElement("i");
        i.style.background = c || "#e67e22";
        chips.append(i);
      }
      const old = li.querySelector("select");
      const sel = kindSelect();
      sel.title = old.title;
      sel.prepend(new Option("— not set —", ""));
      old.replaceWith(sel);
      sel.value = sy.kind;
      iconSelect(sel, (v) => (v ? kindIcon(v, sy.colours[0]?.[0]) : ""));
      sel.onchange = async () => {
        try {
          const n = await invoke("symbol_set", { symbol: sy.symbol, kind: sel.value, label: sy.label });
          activity.note(`TimeZero symbol ${sy.symbol.slice(3)}: ${n.toLocaleString()} mark${n === 1 ? "" : "s"} now ${sel.selectedOptions[0].textContent.toLowerCase()}`);
        } catch (e) { activity.start(`TimeZero symbol ${sy.symbol.slice(3)}`).fail(String(e)); }
        loadSymbols();
      };
      const show = li.querySelector('[data-act="show"]');
      show.textContent = showing ? "Show all" : "Show";
      show.title = showing ? "Show every mark again" : "Show only the marks with this symbol (list and chart)";
      show.onclick = () => {
        markList.setFilter("symbol", showing ? null : sy.symbol);
        if (!showing) { prefs.chartFiltered = true; $("lib-f-chart").checked = true; savePrefs(); draw(); }
        renderSymbols();
      };
      ul.append(li);
    }
  }
  $("lib-symbols").addEventListener("toggle", renderSymbols);


  // ---------- routings tab ----------------------------------------------------------
  const routingList = libList($("lib-routings-list"), {
    key: "routings",
    noun: ["routing", "routings"],
    search: (r, q) => r.name.toLowerCase().includes(q) || (r.route_name || "").toLowerCase().includes(q),
    sorts: [
      { key: "new", label: "Newest first", cmp: (a, b) => (b.created || 0) - (a.created || 0) },
      { key: "name", label: "By name", cmp: (a, b) => collate(a.name, b.name) },
    ],
    row: {
      render(r, el) {
        const result = r.status === "arrived" ? `ETA ${fmt(r.arrival, true)}` : r.status === "grib end" ? "to GRIB end" : r.status ?? "";
        el.innerHTML = `<div class="main" title="Show this routing"><b></b><small></small></div><button data-act="open">Show</button>`;
        el.querySelector("b").textContent = r.name;
        el.querySelector("small").textContent = [r.t0 ? `start ${fmt(r.t0, true)}` : "", result, r.distance_nm != null ? `${r.distance_nm.toFixed(1)} NM` : "", r.grib_name, r.polar].filter(Boolean).join(" · ");
        el.querySelector('[data-act="open"]').onclick = () => routing?.showSaved(r.id);
      },
      click: (r) => routing?.showSaved(r.id),
    },
    actions: [
      { label: "Delete", danger: true, confirm: (rs) => `Delete ${rs.length === 1 ? `“${rs[0].name}”` : `${rs.length} routings`}?`,
        run: async (rs) => {
          for (const r of rs) await invoke("routing_delete", { id: r.id }).catch(() => {});
          activity.note(`Deleted ${rs.length} routing${rs.length === 1 ? "" : "s"}`);
          refresh();
        } },
    ],
    empty: () => "No saved routings. Compute one in the Routing drawer and press Save.",
  });

  // ---------- the Trash and Undo an import: opened from the Library menu, in place of the lists ----------
  const pane = $("lib-trash-list").closest(".pane");
  const tabBar = pane.querySelector(".tabs");
  let lastTab = "marks";
  tabBar.addEventListener("click", () => { tabBar.style.display = ""; }); // any tab (or opening from the chart) leaves them
  function showSub(name) {
    lastTab = tabBar.querySelector("button.on")?.dataset.tab ?? lastTab;
    tabBar.style.display = "none";
    for (const t of pane.querySelectorAll(".tab")) t.hidden = t.dataset.tab !== name;
    ui.open("library");
  }
  for (const b of pane.querySelectorAll(".lib-back")) b.onclick = () => tabBar.querySelector(`button[data-tab="${lastTab}"]`)?.click();
  const when = (t) => new Date(t * 1000).toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
  const KINDS = { mark: ["mark", "marks"], route: ["route", "routes"], track: ["track", "tracks"], routing: ["routing", "routings"], barrier: ["barrier", "barriers"], trip: ["trip", "trips"] };
  const howMany = (n, k) => `${n.toLocaleString()} ${KINDS[k][n === 1 ? 0 : 1]}`;
  const cap = (t) => t.charAt(0).toUpperCase() + t.slice(1);
  const byKind = (xs) => { const g = {}; for (const x of xs) (g[x.kind] ??= []).push(x.rid); return g; };
  const trashList = libList($("lib-trash-list"), {
    key: "trash",
    noun: ["thing", "things"],
    saved: false,
    searchPlaceholder: "Find in the Trash",
    search: (x, q) => x.name.toLowerCase().includes(q) || x.detail.toLowerCase().includes(q),
    sorts: [
      { key: "new", label: "Deleted last first", cmp: (a, b) => b.deleted_at - a.deleted_at },
      { key: "name", label: "By name", cmp: (a, b) => collate(a.name, b.name) },
    ],
    filters: [
      { key: "kind", label: "Kind", type: "choice", options: (xs) => countBy(xs, (x) => x.kind, (k) => cap(KINDS[k][1])), test: (x, v) => x.kind === v },
      // one deletion (a minute) is one choice: "the 1,240 marks I deleted on Tuesday"
      { key: "when", label: "Deleted", type: "choice", title: "Only what was deleted at one time",
        options: (xs) => countBy(xs, (x) => Math.floor(x.deleted_at / 60) * 60, (t) => when(t)).sort((a, b) => b[0] - a[0]),
        test: (x, v) => Math.floor(x.deleted_at / 60) * 60 === Number(v) },
    ],
    row: {
      render(x, el) {
        el.innerHTML = `<div class="main"><b></b><small></small></div><button data-act="restore" title="Bring it back">Restore</button>`;
        el.querySelector("b").textContent = x.name || `(unnamed ${KINDS[x.kind][0]})`;
        const detail = x.kind === "track" && Number(x.detail) ? fmt(Number(x.detail), true) : x.detail;
        el.querySelector("small").textContent = [cap(KINDS[x.kind][0]), detail, `deleted ${when(x.deleted_at)}`].filter(Boolean).join(" · ");
        el.querySelector('[data-act="restore"]').onclick = () => restore([x]);
      },
    },
    actions: [
      { label: "Restore", title: "Bring the selected things back", run: (xs) => restore(xs) },
      { label: "Delete for good", danger: true, title: "Delete the selected things for good: they can't be brought back",
        confirm: (xs) => `Delete ${xs.length.toLocaleString()} thing${xs.length === 1 ? "" : "s"} for good? They can't be brought back.`,
        run: async (xs) => {
          let n = 0;
          for (const [kind, ids] of Object.entries(byKind(xs))) n += await invoke("library_purge", { kind, ids }).catch(() => 0);
          activity.note(`Deleted ${n.toLocaleString()} thing${n === 1 ? "" : "s"} for good`);
          loadTrash();
        } },
    ],
    empty: () => "The Trash is empty.",
  });
  async function loadTrash() {
    let v = [];
    try { v = await invoke("library_trash"); } catch (e) { activity.start("Opening the Trash").fail(String(e)); }
    trashList.clearSelection();
    trashList.setItems(v.map((x) => ({ ...x, rid: x.id, id: `${x.kind}:${x.id}` })));
    const btn = $("trash-empty");
    btn.disabled = !v.length;
    btn.textContent = "Empty the Trash";
    btn.dataset.armed = "";
  }
  async function restore(xs) {
    const parts = [];
    for (const [kind, ids] of Object.entries(byKind(xs))) {
      try { parts.push(howMany(await invoke("library_restore", { kind, ids }), kind)); } catch (e) { activity.start("Restoring").fail(String(e)); }
    }
    activity.note(`Restored ${parts.join(", ")}`, { details: xs.some((x) => x.kind === "mark") ? ["Restored marks are not put back into routes they were taken out of."] : [] });
    loadTrash();
    refresh();
  }
  // emptying the Trash asks by changing the button: press it again within a few seconds
  $("trash-empty").onclick = async () => {
    const btn = $("trash-empty");
    if (!btn.dataset.armed) {
      btn.dataset.armed = "1";
      btn.textContent = "Delete everything for good? Press again";
      setTimeout(() => { if (btn.dataset.armed) { btn.dataset.armed = ""; btn.textContent = "Empty the Trash"; } }, 4000);
      return;
    }
    let n = 0;
    for (const kind of Object.keys(KINDS)) n += await invoke("library_purge", { kind, ids: [] }).catch(() => 0);
    activity.note(`The Trash is empty: ${n.toLocaleString()} thing${n === 1 ? "" : "s"} deleted for good`);
    loadTrash();
  };
  function openTrash() { showSub("trash"); loadTrash(); }

  async function openImports() {
    showSub("imports");
    const ul = $("lib-imports");
    ul.innerHTML = "";
    let list = [];
    try { list = await invoke("library_imports"); } catch (e) { activity.start("Imports").fail(String(e)); }
    if (!list.length) { ul.innerHTML = `<li class="wx-note">Nothing imported is in the library now (imports made before the library moved to its database in October 2026 can't be undone this way).</li>`; return; }
    const KIND = { gpx: "GPX", timezero: "TimeZero", library: "Merged library" };
    for (const im of list) {
      const li = document.createElement("li");
      const what = [im.marks && howMany(im.marks, "mark"), im.routes && howMany(im.routes, "route"), im.tracks && howMany(im.tracks, "track")].filter(Boolean).join(", ");
      li.innerHTML = `<div class="main"><b></b><small></small></div><button class="warn">Undo</button>`;
      li.querySelector("b").textContent = im.file || "(no name)";
      li.querySelector("small").textContent = `${KIND[im.kind] ?? im.kind} · ${when(im.at)} · ${what}`;
      li.querySelector("button").onclick = () => {
        li.innerHTML = `<div class="main"><b></b><small></small></div><button class="on warn">Undo it</button><button>Keep</button>`;
        li.querySelector("b").textContent = `Take out ${what}?`;
        li.querySelector("small").textContent = `From ${im.file}. They go to the Trash for 30 days.`;
        const [ok, no] = li.querySelectorAll("button");
        no.onclick = openImports;
        ok.onclick = async () => {
          ok.disabled = true;
          try {
            const n = await invoke("library_undo_import", { id: im.id });
            const done = [n.marks && howMany(n.marks, "mark"), n.routes && howMany(n.routes, "route"), n.tracks && howMany(n.tracks, "track")].filter(Boolean).join(", ");
            activity.note(`Import of ${im.file} undone`, { details: [`${done || "Nothing"} moved to the Trash.`, "Library > Trash brings them back."] });
          } catch (e) { activity.start(`Undoing the import of ${im.file}`).fail(String(e)); }
          await refresh();
          openImports();
        };
      };
      ul.append(li);
    }
  }

  // ---------- the Library menu: which library, import, export, backups ----------
  const menu = $("lib-menu");
  let current = null;
  async function showName() {
    try { current = await invoke("library_current"); } catch { return; }
    $("lib-name").textContent = current.name === "library" ? "My library" : current.name;
    $("lib-menu-btn").title = `${current.path}\n(Library menu: new, open, merge, import, export, backups)`;
  }
  function closeMenu() { menu.hidden = true; $("lib-menu-btn").classList.remove("on"); }
  document.addEventListener("pointerdown", (e) => { if (!menu.hidden && !menu.contains(e.target) && !e.target.closest("#lib-menu-btn")) closeMenu(); });
  $("lib-menu-btn").onclick = () => {
    if (!menu.hidden) { closeMenu(); return; }
    const item = (label, title, run, disabled = false) => {
      const b = document.createElement("button");
      b.textContent = label;
      b.title = title;
      b.disabled = disabled;
      b.onclick = () => { closeMenu(); run(); };
      menu.append(b);
    };
    const line = () => menu.append(document.createElement("hr"));
    const head = (t) => { const h = document.createElement("div"); h.className = "lib-menu-head"; h.textContent = t; menu.append(h); };
    menu.innerHTML = "";
    head(current ? `${current.name === "library" ? "My library" : current.name} · ${current.path}` : "Library");
    item("New library…", "Start an empty library (another boat, another fishery): the one in use stays as it is", newLibrary);
    item("Open library…", "Use another TweedPlot library (.gpkg): one shared from another boat, or for another fishery", openLibrary);
    for (const r of current?.recent ?? []) item(`   ${r.name}`, r.path, () => useLibrary(r.path, false));
    item("Save a copy as…", "A copy of the whole library (to share, or keep): the one in use stays in use", saveCopy);
    item("Merge from another library…", "Bring another library's marks, routes, tracks, routings and barriers into this one (matched as imports are)", mergeLibrary);
    line();
    item("Import GPX or TimeZero…", "Marks, routes and tracks from a GPX file, or a TimeZero .tzd or .TzBackup", importFile);
    item("Export everything as GPX", "Every mark and route in one GPX file in the export folder", () => exportGpx("racenav library", [], [], "The library"));
    line();
    item("Backups…", "TweedPlot's own daily copies of this library: use one", showBackups);
    item("Trash…", "Deleted marks, routes, tracks, routings and barriers are kept 30 days: bring them back, or delete them for good", openTrash);
    item("Undo an import…", "Take out again everything a GPX or TimeZero file (or a merge) brought in", openImports);
    menu.hidden = false;
    $("lib-menu-btn").classList.add("on");
  };
  async function useLibrary(path, isNew) {
    const name = path.split(/[\\/]/).pop().replace(/\.gpkg$/i, "");
    try {
      await invoke("library_use", { path, new: isNew });
      closeForm();
      markList.clearSelection();
      activity.note(`${isNew ? "New library" : "Library"} ${name} in use`, { details: [path] });
      await refresh();
    } catch (e) { activity.start(`${isNew ? "Making" : "Opening"} library ${name}`).fail(String(e)); }
  }
  async function newLibrary() {
    const p = await invoke("library_pick_db", { save: true, title: "New library", suggest: "Tuna.gpkg" }).catch(() => null);
    if (p) useLibrary(p, true);
  }
  async function openLibrary() {
    const p = await invoke("library_pick_db", { save: false, title: "Open a library", suggest: null }).catch(() => null);
    if (p) useLibrary(p, false);
  }
  async function saveCopy() {
    const p = await invoke("library_pick_db", { save: true, title: "Save a copy of the library", suggest: `${current?.name ?? "library"} copy.gpkg` }).catch(() => null);
    if (!p) return;
    const job = activity.start("Saving a copy of the library", { params: p });
    try { await invoke("library_save_copy", { path: p }); job.done("Done.", { details: [`Written to ${p}`] }); } catch (e) { job.fail(String(e)); }
  }
  async function mergeLibrary() {
    const p = await invoke("library_pick_db", { save: false, title: "Merge from another library", suggest: null }).catch(() => null);
    if (!p) return;
    const name = p.split(/[\\/]/).pop();
    const job = activity.start(`Merging ${name} into this library`, { params: p, cancel: (id) => invoke("job_cancel", { job: id }) });
    try {
      const rep = await invoke("library_merge", { path: p, job: job.id });
      reportImport(job, rep);
      await refresh();
    } catch (e) { job.fail(`Merging failed: ${e}`); }
  }
  async function showBackups() {
    let list = [];
    try { list = await invoke("library_backups"); } catch { /* none */ }
    menu.innerHTML = "";
    const h = document.createElement("div");
    h.className = "lib-menu-head";
    h.textContent = list.length ? "Backups of this library (a copy is made each day TweedPlot starts). Using one copies it beside the library and opens the copy: the backup and the library stay as they are." : "No backups yet: one is made each day TweedPlot starts.";
    menu.append(h);
    for (const b of list) {
      const btn = document.createElement("button");
      btn.textContent = `${b.day}  (${(b.bytes / 1e6).toFixed(0)} MB)`;
      btn.title = `Use this backup: ${b.path}`;
      btn.onclick = async () => {
        closeMenu();
        try {
          const p = await invoke("library_use_backup", { path: b.path });
          activity.note(`Backup of ${b.day} in use`, { details: [`Copied to ${p}`, "The library before is unchanged: open it again from the Library menu."] });
          await refresh();
        } catch (e) { activity.start("Using a backup").fail(String(e)); }
      };
      menu.append(btn);
    }
    menu.hidden = false;
  }

  // ---------- importing GPX and TimeZero (picked and read by the app itself: a TimeZero file can be gigabytes)
  function reportImport(job, rep) {
    const n = (k, one, many = `${one}s`) => `${k.toLocaleString()} ${k === 1 ? one : many}`;
    const parts = [
      rep.marks_added && `${n(rep.marks_added, "new mark")}`,
      rep.marks_matched && `${n(rep.marks_matched, "mark")} already here`,
      rep.routes_added.length && n(rep.routes_added.length, "new route"),
      rep.tracks_added?.length && n(rep.tracks_added.length, "new track"),
    ].filter(Boolean);
    const details = [
      ...(rep.routes_matched ? [`${n(rep.routes_matched, "route")} already here`] : []),
      ...(rep.routes_added.length && rep.routes_added.length <= 12 ? [`Routes: ${rep.routes_added.join(", ")}`] : []),
      ...(rep.notes || []).map((t) => t.charAt(0).toUpperCase() + t.slice(1)),
      ...(rep.not_imported || []).map((t) => `Not imported: ${t}`),
    ];
    job.done(parts.length ? parts.join(", ") + "." : "Nothing new: everything in it is already in the library.", { details, keep: !!rep.not_imported?.length });
    if (rep.tracks_added?.length) window.dispatchEvent(new CustomEvent("racenav-tracks-imported", { detail: rep.tracks_added }));
  }
  async function importFile() {
    const path = await invoke("library_pick_file").catch(() => null);
    if (!path) return;
    const name = path.split(/[\\/]/).pop();
    const job = activity.start(`Importing ${name}`, { params: path, cancel: (id) => invoke("job_cancel", { job: id }) });
    try {
      const rep = await invoke("library_import_path", { path, job: job.id });
      reportImport(job, rep);
      await refresh();
    } catch (err) { job.fail(`Import failed: ${err}`); }
  }
  window.addEventListener("racenav-import", importFile);

  // looked up by id, and how many routes use each mark, worked out once per refresh
  // rather than for every mark on every redraw (a fisher's library runs to tens of thousands)
  let ids = new Map(), usedBy = new Map();
  function index() {
    ids = new Map(marks.map((m) => [m.id, m]));
    usedBy = new Map();
    for (const r of routes) {
      for (const id of new Set([r.start_mark, ...r.marks.map((x) => x.mark)].filter(Boolean))) usedBy.set(id, (usedBy.get(id) || 0) + 1);
    }
  }
  const byId = (id) => ids.get(id);

  onTrips(() => markList.refresh());
  refresh();
  return {
    refresh,
    /** Show a trip's marks: the Marks list filtered to it and the chart showing only those. Returns them. */
    showTrip(trip) {
      markList.setFilter("trip", trip.id);
      if (!prefs.chartFiltered) { prefs.chartFiltered = true; $("lib-f-chart").checked = true; savePrefs(); }
      draw();
      return marks.filter((m) => markInTrip(m, trip));
    },
    marks: () => marks,
    byId,
    attach(r, x) { route = r; routing = x; routeList.redraw(); },
    onChange: (f) => listeners.push(f),
    /** Library marks drawn under the route's own (hide the ones the route shows). */
    hide(list) {
      const next = new Set(list);
      if (next.size === hidden.size && [...next].every((id) => hidden.has(id))) return;
      hidden = next; draw();
    },
    /** Open a mark in the Library: its row (search and filters cleared if they hide it) and its editor. */
    openMark(id) {
      const m = byId(id);
      if (!m) return;
      ui.openTab("library", "marks");
      editMark(m);
      if (!markList.highlight(id)) { markList.clearFilters(); markList.highlight(id); }
    },
    /** A saved route's name. */
    routeName: (id) => routes.find((r) => r.id === id)?.name,
    /** Open a saved route's row in the Library. */
    openRoute(id) {
      ui.openTab("library", "routes");
      if (!routeList.highlight(id)) { routeList.clearFilters(); routeList.highlight(id); }
    },
    /** The library mark under a map point, if any. */
    markAt(point) {
      if (!prefs.show) return null;
      const f = map.queryRenderedFeatures([[point.x - 6, point.y - 6], [point.x + 6, point.y + 6]], { layers: ["lib-marks"] })[0];
      return f ? byId(f.properties.id) : null;
    },
    /** New library mark at a position (right-click menu). */
    async newMarkAt(p) {
      try {
        const m = await invoke("mark_save", { mark: { name: "", lat: p[1], lon: p[0], kind: "mark", group: "", notes: "" } });
        await refresh();
        ui.openTab("library", "marks");
        editMark(m);
      } catch (e) { status(String(e)); }
    },
  };
}

/** Write chosen marks, routes (with their marks) and tracks as a library file of their own
 *  (to give another boat, or keep a fishery apart). The library in use is unchanged. */
export async function exportLibraryFile({ marks = [], routes = [], tracks = [] }, what, suggest = "Part of the library") {
  const p = await invoke("library_pick_db", { save: true, title: `Write ${what} as a library file`, suggest: `${suggest.replace(/[\\/:*?"<>|]/g, " ").trim()}.gpkg` }).catch(() => null);
  if (!p) return;
  const job = activity.start(`Writing ${what} as a library file`, { params: p });
  try {
    const n = await invoke("library_export", { path: p, marks, routes, tracks });
    const N = (k, one) => k && `${k.toLocaleString()} ${one}${k === 1 ? "" : "s"}`;
    job.done(`${[N(n.marks, "mark"), N(n.routes, "route"), N(n.tracks, "track")].filter(Boolean).join(", ") || "Nothing"} written.`,
      { details: [`Written to ${p}`, "Open it with Library > Open library, or bring it into another library with Merge from another library."] });
  } catch (e) { job.fail(String(e)); }
}

/** Mark name labels drawn on demand: "lbl:<name>". */
export function labelImage(id) {
  if (!id.startsWith("lbl:")) return null;
  const text = id.slice(4);
  if (!text) return null;
  const c = document.createElement("canvas");
  const g = c.getContext("2d");
  const font = `600 22px "Segoe UI", system-ui, sans-serif`;
  g.font = font;
  const w = Math.ceil(g.measureText(text).width) + 12;
  c.width = w; c.height = 30;
  g.font = font;
  g.textBaseline = "middle";
  g.lineWidth = 5; g.strokeStyle = "rgba(255,255,255,0.9)"; g.lineJoin = "round";
  g.strokeText(text, 6, 15);
  g.fillStyle = "#1d2a33";
  g.fillText(text, 6, 15);
  return g.getImageData(0, 0, c.width, c.height);
}

export function ddm(deg, pos, neg) {
  if (pos === "E" && Number.isFinite(deg)) deg = ((((deg + 180) % 360) + 360) % 360) - 180; // map past ±180
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}

/**
 * Coordinates as people type them: "36 50.12 S", "36°50.12'S", "-36.8353",
 * "36 50 07 S" (d m s). `hemis` is "NS" or "EW".
 */
export function parseCoord(s, hemis) {
  let t = s.trim().toUpperCase();
  if (!t) return null;
  let sign = 1;
  const h = t.match(/[NSEW]/);
  if (h) {
    if (!hemis.includes(h[0])) return null;
    if (h[0] === "S" || h[0] === "W") sign = -1;
    t = t.replace(/[NSEW]/g, " ");
  }
  if (t.trim().startsWith("-")) { sign = -sign; t = t.replace("-", " "); }
  const nums = t.replace(/[°'"′″,]/g, " ").trim().split(/\s+/).filter(Boolean).map(Number);
  if (!nums.length || nums.some((n) => !Number.isFinite(n))) return null;
  const [d, m = 0, sec = 0] = nums;
  if (m < 0 || m >= 60 || sec < 0 || sec >= 60) return null;
  return sign * (Math.abs(d) + m / 60 + sec / 3600);
}
