// Library panel: the mark library, saved routes and saved routings, GPX in and out.
// The data lives in JSON files written by the Rust side (the library folder).
import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
import { fmt } from "./weather.js";
import { distNm } from "./instruments.js";
import { POI_KINDS, poiImage } from "./poi.js";
import { iconSelect, imageUrl } from "./iconselect.js";
import { featureImage } from "./chartfeatures.js";

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
  let prefs = { show: true, filter: { group: "*", type: "*", route: "*", view: false, chart: false } };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  prefs.filter = { group: "*", type: "*", route: "*", view: false, chart: false, ...(prefs.filter || {}) };
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
  const isIcon = ["any", isBuoy, isPoi];
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
  map.addLayer({ id: "lib-labels", type: "symbol", source: "lib-marks", minzoom: 8, filter: ["!=", ["get", "name"], ""],
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 6], "icon-allow-overlap": false } }, "track");
  let hidden = new Set();
  function draw() {
    const vis = prefs.show ? "visible" : "none";
    map.setLayoutProperty("lib-marks", "visibility", vis);
    map.setLayoutProperty("lib-labels", "visibility", vis);
    map.setLayoutProperty("lib-buoys", "visibility", vis);
    map.setLayoutProperty("lib-poi", "visibility", vis);
    const shown = prefs.filter.chart ? marks.filter((m) => passes(m, false)) : marks;
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
    draw();
    if (sel && !byId(sel)) deselectMark(); // the selected mark was deleted
    if (editing?.id && !byId(editing.id)) closeForm();
    renderGroups();
    renderRoutes(); renderMarks(); renderRoutings();
    loadSymbols();
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
  function renderRoutes() {
    const extra = duplicateRoutes();
    $("lib-dupes").hidden = !extra.length;
    $("lib-dupes").querySelector("span").textContent = `${extra.length} route${extra.length === 1 ? " is a copy" : "s are copies"} of another (same name and marks), from importing a file more than once.`;
    const ul = $("lib-routes");
    ul.innerHTML = "";
    if (!routes.length) ul.innerHTML = `<li class="empty">No saved routes. Make one in the Route drawer and press Save, or import a GPX file.</li>`;
    for (const r of routes) {
      const li = document.createElement("li");
      li.classList.toggle("active", route?.current().routeId === r.id);
      li.innerHTML = `<div class="main" title="Open this route"><b></b><small></small></div><button data-act="open">Open</button><button data-act="gpx" title="Write this route as GPX">GPX</button><button data-act="del" title="Delete this route (its marks stay in the library)">✕</button>`;
      li.querySelector("b").textContent = r.name;
      li.querySelector("small").textContent = `${r.start ? "start + " : ""}${r.marks.length} marks · ${routeLength(r).toFixed(1)} NM · ${[r.start_mark ? byId(r.start_mark)?.name : null, ...r.marks.map((x) => byId(x.mark)?.name ?? "?")].filter(Boolean).join(" → ")}`;
      const open = () => { route?.loadRoute(r, marks); ui.open("course"); };
      li.querySelector(".main").onclick = open;
      li.querySelector('[data-act="open"]').onclick = open;
      li.querySelector('[data-act="gpx"]').onclick = async () => {
        try { status(`Written to ${await invoke("gpx_export", { name: r.name, markIds: [], routeIds: [r.id] })}`); } catch (e) { status(String(e)); }
      };
      li.querySelector('[data-act="del"]').onclick = () => confirmThen(li, `Delete route “${r.name}”?`, async () => {
        await invoke("route_delete", { id: r.id }).catch((e) => status(String(e)));
        if (route?.current().routeId === r.id) route.detach();
        refresh();
      });
      ul.append(li);
    }
  }

  // ---------- marks tab ----------------------------------------------------------
  // ---------- marks filters ----------------------------------------------------------
  const COURSE = new Set(["mark", "waypoint", "start", "finish"]);
  // the mark editor's Type list shows each type's symbol as the chart draws it
  let kindPicker = null;
  const DOTS = { danger: "#c0392b", waypoint: "#5b6b75" };
  function kindIcon(k) {
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
  /** Whether a mark passes the filters (`view`: the chart-view filter too). */
  function passes(m, view) {
    const f = prefs.filter;
    if (f.group !== "*" && (m.group || "") !== f.group) return false;
    if (f.type !== "*" && markType(m) !== f.type) return false;
    if (f.route !== "*" && inRoute(m) !== (f.route === "used")) return false;
    if (f.symbol && m.symbol !== f.symbol) return false;
    if (view && f.view) {
      const b = map.getBounds();
      const lon = b.getEast() - b.getWest() >= 360 ? null : ((m.lon - b.getWest()) % 360 + 360) % 360;
      if (m.lat < b.getSouth() || m.lat > b.getNorth() || (lon != null && lon > b.getEast() - b.getWest())) return false;
    }
    return true;
  }
  function renderGroups() {
    const sel = $("lib-f-group");
    const groups = [...new Set(marks.map((m) => m.group || ""))].sort((a, b) => a.localeCompare(b));
    if (prefs.filter.group !== "*" && !groups.includes(prefs.filter.group)) prefs.filter.group = "*";
    sel.innerHTML = `<option value="*">All groups</option>`;
    for (const g of groups) {
      const o = document.createElement("option");
      o.value = g;
      o.textContent = g || "(no group)";
      sel.append(o);
    }
    sel.value = prefs.filter.group;
  }
  function syncFilterControls() {
    const f = prefs.filter;
    $("lib-f-type").value = f.type; $("lib-f-route").value = f.route;
    $("lib-f-view").checked = f.view; $("lib-f-chart").checked = f.chart;
    const on = f.group !== "*" || f.type !== "*" || f.route !== "*" || f.view || !!f.symbol;
    $("lib-f-clear").disabled = !on;
    for (const id of ["lib-f-group", "lib-f-type", "lib-f-route"]) $(id).classList.toggle("filtering", $(id).value !== "*");
  }
  const filterChanged = () => { pageEnd = PAGE; savePrefs(); syncFilterControls(); renderMarks(); draw(); };
  $("lib-f-group").onchange = (e) => { prefs.filter.group = e.target.value; filterChanged(); };
  $("lib-f-type").onchange = (e) => { prefs.filter.type = e.target.value; filterChanged(); };
  $("lib-f-route").onchange = (e) => { prefs.filter.route = e.target.value; filterChanged(); };
  $("lib-f-view").onchange = (e) => { prefs.filter.view = e.target.checked; filterChanged(); };
  $("lib-f-chart").onchange = (e) => { prefs.filter.chart = e.target.checked; filterChanged(); };
  $("lib-f-clear").onclick = () => { Object.assign(prefs.filter, { group: "*", type: "*", route: "*", view: false, symbol: "" }); renderGroups(); filterChanged(); renderSymbols(); };
  // the chart-view filter follows the chart while the Marks tab is open
  let viewTimer = null;
  map.on("moveend", () => {
    if (!prefs.filter.view || $("lib-marks").closest(".tab")?.hidden || $("lib-marks").closest(".pane")?.hidden) return;
    clearTimeout(viewTimer);
    viewTimer = setTimeout(renderMarks, 150);
  });

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
    // the type list needs the point-of-interest options the mark editor adds
    if (!form.kind.querySelector("optgroup.poi")) {
      const og = document.createElement("optgroup");
      og.className = "poi";
      og.label = "Point of interest";
      for (const [k, [label]] of Object.entries(POI_KINDS)) og.append(new Option(label, k));
      form.kind.append(og);
    }
    for (const sy of symbols) {
      const li = document.createElement("li");
      const showing = prefs.filter.symbol === sy.symbol;
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
      const sel = li.querySelector("select");
      sel.append(new Option("— not set —", ""));
      for (const [v, t] of KIND_CHOICES()) sel.append(new Option(t, v));
      sel.value = sy.kind;
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
        prefs.filter.symbol = showing ? "" : sy.symbol;
        if (!showing) prefs.filter.chart = true;
        filterChanged();
        renderSymbols();
      };
      ul.append(li);
    }
  }
  $("lib-symbols").addEventListener("toggle", renderSymbols);

  // delete every mark the list shows (after a check, with the numbers)
  let shownMarks = [];
  $("lib-f-delete").onclick = () => {
    const ids = shownMarks.map((m) => m.id);
    if (!ids.length) return;
    const inRoutes = shownMarks.filter((m) => usedBy.has(m.id)).length;
    const all = ids.length === marks.length;
    $("lib-bulk").querySelector("span").textContent = `Delete ${all ? "all " : ""}${ids.length} mark${ids.length === 1 ? "" : "s"}${all ? " in the library" : " shown"}?` +
      `${inRoutes ? ` ${inRoutes} ${inRoutes === 1 ? "is" : "are"} in saved routes and will be taken out of them.` : ""} This can't be undone.`;
    $("lib-bulk").hidden = false;
    $("lib-bulk-yes").onclick = async () => {
      $("lib-bulk-yes").disabled = true;
      try {
        const n = await invoke("marks_delete", { ids, force: true });
        activity.note(`Deleted ${n.toLocaleString()} mark${n === 1 ? "" : "s"}`, { details: inRoutes ? [`${inRoutes.toLocaleString()} were taken out of saved routes`] : [] });
      } catch (e) { activity.start("Deleting marks").fail(`Could not delete: ${e}`); }
      $("lib-bulk-yes").disabled = false;
      $("lib-bulk").hidden = true;
      await refresh();
    };
  };
  $("lib-bulk-no").onclick = () => { $("lib-bulk").hidden = true; };
  const PAGE = 200;
  let pageEnd = PAGE, lastQuery = "";
  const collate = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" }).compare;
  function renderMarks() {
    const q = $("lib-search").value.trim().toLowerCase();
    const ul = $("lib-marks");
    parkForm(); // the edit form lives in the list while open: keep it out of the wipe
    ul.innerHTML = "";
    if ($("lib-f-group").options.length <= 1 && marks.length) renderGroups();
    syncFilterControls();
    const list = marks.filter((m) => (!q || m.name.toLowerCase().includes(q) || m.group.toLowerCase().includes(q) || m.notes?.toLowerCase().includes(q)) && passes(m, true))
      .sort((a, b) => collate(a.group, b.group) || (!a.name - !b.name) || collate(a.name, b.name) || (b.created || 0) - (a.created || 0));
    shownMarks = list;
    $("lib-f-delete").disabled = !list.length;
    $("lib-bulk").hidden = true;
    $("lib-f-count").textContent = list.length === marks.length ? `${marks.length} mark${marks.length === 1 ? "" : "s"}` : `${list.length} of ${marks.length} marks`;
    if (!list.length) ul.innerHTML = `<li class="empty">${marks.length ? "No marks match the search and filters." : "No marks yet. Right-click the chart: New library mark here, or save a route."}</li>`;
    // a page at a time: thousands of rows would make the drawer slow to open and scroll
    if (q !== lastQuery) { pageEnd = PAGE; lastQuery = q; }
    if (editing?.id) { const at = list.findIndex((m) => m.id === editing.id); if (at >= pageEnd) pageEnd = at + 1; }
    for (const m of list.slice(0, pageEnd)) {
      const used = usedBy.get(m.id) || 0;
      const li = document.createElement("li");
      li.dataset.id = m.id;
      li.innerHTML = `<div class="main" title="Show on the chart"><b></b><small></small></div><button data-act="start" title="Make this the route's start">Start</button><button data-act="add" title="Add to the end of the route being edited">+ Route</button><button data-act="edit">Edit</button><button data-act="del">✕</button>`;
      li.querySelector("b").textContent = m.name || "(no name)";
      li.querySelector("b").classList.toggle("unnamed", !m.name);
      if (m.colour) li.querySelector(".main").style.borderLeft = `4px solid ${m.colour}`;
      li.querySelector("small").textContent = `${m.group ? m.group + " · " : ""}${ddm(m.lat, "N", "S")} ${ddm(m.lon, "E", "W")}${m.created ? ` · ${m.date_unknown ? "date unknown" : new Date(m.created * 1000).toLocaleDateString()}` : ""}${m.depth != null ? ` · ${Math.round(m.depth)} m` : ""}${m.temp != null ? ` · ${m.temp.toFixed(1)} °C` : ""}${(() => { const t = (m.tags || "").split(",").filter((x) => x && x !== "at-boat" && x !== "depth"); return t.length ? ` · ${t.join(", ")}` : ""; })()}${used ? ` · in ${used} route${used > 1 ? "s" : ""}` : ""}`;
      li.querySelector(".main").onclick = () => map.flyTo({ center: [m.lon, m.lat], zoom: Math.max(map.getZoom(), 11) });
      li.querySelector('[data-act="start"]').onclick = () => { route?.setStartLibraryMark(m); status(`The route now starts at ${m.name}.`); };
      li.querySelector('[data-act="add"]').onclick = () => { route?.addLibraryMark(m); status(`${m.name} added to the route.`); };
      li.querySelector('[data-act="edit"]').onclick = () => editMark(m);
      li.querySelector('[data-act="del"]').onclick = () => confirmThen(li, used ? `${m.name} is in ${used} route${used > 1 ? "s" : ""}: delete it and take it out of them?` : `Delete ${m.name}?`, async () => {
        await invoke("mark_delete", { id: m.id, force: true }).catch((e) => status(String(e)));
        refresh();
      });
      ul.append(li);
    }
    if (list.length > pageEnd) {
      const li = document.createElement("li");
      li.className = "more";
      li.innerHTML = `<button>Show ${Math.min(PAGE, list.length - pageEnd)} more</button><small></small>`;
      li.querySelector("small").textContent = ` ${list.length - pageEnd} not shown: search, filter by group or tick "In chart view" to narrow the list.`;
      li.querySelector("button").onclick = () => { pageEnd += PAGE; renderMarks(); };
      ul.append(li);
    }
    if (editing && !form.hidden) placeForm(editing.id);
  }
  $("lib-search").oninput = renderMarks;
  $("lib-show").checked = prefs.show;
  $("lib-show").onchange = (e) => { prefs.show = e.target.checked; savePrefs(); draw(); };

  const form = $("mark-edit");
  // the form's home when no mark is being edited (just after the list)
  const formHome = document.createComment("mark-edit");
  form.before(formHome);
  function parkForm() { if (form.parentElement?.id === "lib-marks-edit-row" || form.closest("#lib-marks")) formHome.after(form); }
  /** Open the form in the list, under the mark's row (or where it was, if that row is filtered out). */
  function placeForm(id) {
    const ul = $("lib-marks");
    const li = [...ul.children].find((x) => x.dataset.id === id);
    if (!li) { formHome.after(form); return; }
    const row = document.createElement("li");
    row.className = "edit-row";
    row.id = "lib-marks-edit-row";
    row.append(form);
    li.after(row);
    li.classList.add("editing");
  }
  function closeForm() {
    form.hidden = true;
    editing = null;
    const row = document.getElementById("lib-marks-edit-row");
    parkForm();
    row?.remove();
    $("lib-marks").querySelector("li.editing")?.classList.remove("editing");
  }
  function editMark(m) {
    if (!document.getElementById("lib-marks-edit-row") || editing?.id !== m.id) { closeForm(); }
    editing = { ...m };
    placeForm(m.id);
    form.hidden = false;
    form.scrollIntoView({ block: "nearest", behavior: "smooth" });
    form.name.value = m.name;
    if (!form.kind.querySelector("optgroup.poi")) {
      const og = document.createElement("optgroup");
      og.className = "poi";
      og.label = "Point of interest";
      for (const [k, [label]] of Object.entries(POI_KINDS)) og.append(new Option(label, k));
      form.kind.append(og);
    }
    form.kind.value = m.kind || "mark";
    kindPicker ??= iconSelect(form.kind, kindIcon);
    kindPicker.refresh();
    form.group.value = m.group || "";
    form.lat.value = ddm(m.lat, "N", "S");
    form.lon.value = ddm(m.lon, "E", "W");
    form.notes.value = m.notes || "";
    form.barrier.value = m.barrier_nm > 0 ? m.barrier_nm : "";
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

  // ---------- routings tab ----------------------------------------------------------
  function renderRoutings() {
    const ul = $("lib-routings");
    ul.innerHTML = "";
    if (!routings.length) ul.innerHTML = `<li class="empty">No saved routings. Compute one in the Routing panel and press Save.</li>`;
    for (const r of routings) {
      const li = document.createElement("li");
      const result = r.status === "arrived" ? `ETA ${fmt(r.arrival, true)}` : r.status === "grib end" ? "to GRIB end" : r.status ?? "";
      li.innerHTML = `<div class="main" title="Show this routing"><b></b><small></small></div><button data-act="open">Show</button><button data-act="del">✕</button>`;
      li.querySelector("b").textContent = r.name;
      li.querySelector("small").textContent = [r.t0 ? `start ${fmt(r.t0, true)}` : "", result, r.distance_nm != null ? `${r.distance_nm.toFixed(1)} NM` : "", r.grib_name, r.polar].filter(Boolean).join(" · ");
      const open = async () => {
        await routing?.showSaved(r.id);
      };
      li.querySelector(".main").onclick = open;
      li.querySelector('[data-act="open"]').onclick = open;
      li.querySelector('[data-act="del"]').onclick = () => confirmThen(li, `Delete routing “${r.name}”?`, async () => {
        await invoke("routing_delete", { id: r.id }).catch((e) => status(String(e)));
        refresh();
      });
      ul.append(li);
    }
  }

  // ---------- GPX ----------------------------------------------------------
  // GPX, or TimeZero .tzd / .TzBackup: picked and read by the app itself (a TimeZero file can be gigabytes)
  const many = (list, one) => (list.length > 6 ? `${list.length} ${one}s` : `${one}s: ${list.join(", ")}`);
  $("lib-import").onclick = async () => {
    $("lib-import").disabled = true;
    try {
      const path = await invoke("library_pick_file");
      if (!path) return;
      const name = path.split(/[\\/]/).pop();
      const job = activity.start(`Importing ${name}`, { params: path, cancel: (id) => invoke("job_cancel", { job: id }) });
      try {
        const rep = await invoke("library_import_path", { path, job: job.id });
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
        job.done(parts.length ? parts.join(", ") + "." : "Nothing new: everything in the file is already in the library.", { details, keep: !!rep.not_imported?.length });
        status("");
        await refresh();
        if (rep.tracks_added?.length) window.dispatchEvent(new CustomEvent("racenav-tracks-imported", { detail: rep.tracks_added }));
      } catch (err) { job.fail(`Import failed: ${err}`); }
    } catch (err) { status(`Import failed: ${err}`); }
    finally { $("lib-import").disabled = false; }
  };
  $("lib-export").onclick = async () => {
    try {
      const p = await invoke("gpx_export", { name: "racenav library", markIds: [], routeIds: [] });
      activity.note("Library exported as GPX", { details: [`Written to ${p}`] });
    } catch (e) { activity.start("Exporting the library").fail(String(e)); }
  };

  /** Inline "are you sure" in place of a list row's buttons. */
  function confirmThen(li, question, action) {
    const keep = li.innerHTML;
    li.innerHTML = `<div class="main"><small></small></div><button data-act="yes" class="on">Delete</button><button data-act="no">Keep</button>`;
    li.querySelector("small").textContent = question;
    li.querySelector('[data-act="yes"]').onclick = action;
    li.querySelector('[data-act="no"]').onclick = () => { li.innerHTML = keep; refresh(); };
  }

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

  refresh();
  return {
    refresh,
    marks: () => marks,
    byId,
    attach(r, x) { route = r; routing = x; renderRoutes(); },
    onChange: (f) => listeners.push(f),
    /** Library marks drawn under the route's own (hide the ones the route shows). */
    hide(list) {
      const next = new Set(list);
      if (next.size === hidden.size && [...next].every((id) => hidden.has(id))) return;
      hidden = next; draw();
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
