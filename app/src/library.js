// Library panel: the mark library, saved routes and saved routings, GPX in and out.
// The data lives in JSON files written by the Rust side (racenav-data\library).
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { distNm } from "./instruments.js";
import { POI_KINDS } from "./poi.js";

const KEY = "racenav.library";

/** Library mark kinds drawn as IALA A buoys: kind -> chart symbol id (see chartfeatures.js). */
export const BUOY_ICONS = {
  port: "b|2|3||", starboard: "b|1|4||",
  north: "b|4|2,6|1|n", east: "b|4|2,6,2|1|e", south: "b|4|6,2|1|s", west: "b|4|6,2,6|1|w",
  isolated: "b|4|2,3,2|1|isd", safewater: "b|3|3,1|2|saw", special: "b|4|6||spp",
};

export function setupLibrary(map, { ui }) {
  const $ = (id) => document.getElementById(id);
  let marks = [];
  let routes = [];
  let routings = [];
  let route = null; // the course editor (set by main once both exist)
  let routing = null;
  let editing = null; // mark being edited
  let prefs = { show: true };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
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
    paint: { "circle-radius": ["case", isIcon, 11, 5], "circle-color": ["match", ["get", "kind"], "danger", "#c0392b", "waypoint", "#5b6b75", "#e67e22"],
             "circle-opacity": ["case", isIcon, 0, 1], "circle-stroke-opacity": ["case", isIcon, 0, 1],
             "circle-stroke-color": "#fff", "circle-stroke-width": 1.5 } }, "track");
  map.addLayer({ id: "lib-buoys", type: "symbol", source: "lib-marks", filter: isBuoy,
    layout: { "icon-image": ["concat", "s57:", ["get", ["get", "kind"], ["literal", BUOY_ICONS]]], "icon-anchor": "bottom", "icon-offset": [0, 3],
              "icon-allow-overlap": true, "icon-ignore-placement": true } }, "lib-marks");
  map.addLayer({ id: "lib-poi", type: "symbol", source: "lib-marks", filter: isPoi,
    layout: { "icon-image": ["concat", "poi:", ["get", "kind"]], "icon-allow-overlap": true, "icon-ignore-placement": true } }, "lib-marks");
  map.addLayer({ id: "lib-labels", type: "symbol", source: "lib-marks", minzoom: 8,
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 6], "icon-allow-overlap": false } }, "track");
  let hidden = [];
  function draw() {
    const vis = prefs.show ? "visible" : "none";
    map.setLayoutProperty("lib-marks", "visibility", vis);
    map.setLayoutProperty("lib-labels", "visibility", vis);
    map.setLayoutProperty("lib-buoys", "visibility", vis);
    map.setLayoutProperty("lib-poi", "visibility", vis);
    map.getSource("lib-marks").setData({ type: "FeatureCollection", features: marks.filter((m) => !hidden.includes(m.id)).map((m) => ({
      type: "Feature", properties: { id: m.id, name: m.name, kind: m.kind }, geometry: { type: "Point", coordinates: [m.lon, m.lat] } })) });
  }
  map.on("mouseenter", "lib-marks", () => { map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "lib-marks", () => { map.getCanvas().style.cursor = ""; });

  // ---------- data ----------------------------------------------------------
  async function refresh() {
    try {
      [marks, routes, routings] = await Promise.all([invoke("marks_list"), invoke("routes_list"), invoke("routings_list")]);
      marks ||= []; routes ||= []; routings ||= [];
    } catch (e) { status(`Library: ${e}`); }
    draw();
    renderRoutes(); renderMarks(); renderRoutings();
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
  function renderRoutes() {
    const ul = $("lib-routes");
    ul.innerHTML = "";
    if (!routes.length) ul.innerHTML = `<li class="empty">No saved routes. Make one in the Course panel and press Save, or import a GPX file.</li>`;
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
  function renderMarks() {
    const q = $("lib-search").value.trim().toLowerCase();
    const ul = $("lib-marks");
    ul.innerHTML = "";
    const list = marks.filter((m) => !q || m.name.toLowerCase().includes(q) || m.group.toLowerCase().includes(q))
      .sort((a, b) => a.group.localeCompare(b.group) || a.name.localeCompare(b.name, undefined, { numeric: true }));
    if (!list.length) ul.innerHTML = `<li class="empty">${marks.length ? "No marks match." : "No marks yet. Right-click the chart: New library mark here, or save a route."}</li>`;
    for (const m of list) {
      const used = routes.filter((r) => r.start_mark === m.id || r.marks.some((x) => x.mark === m.id)).length;
      const li = document.createElement("li");
      li.innerHTML = `<div class="main" title="Show on the chart"><b></b><small></small></div><button data-act="start" title="Make this the route's start">Start</button><button data-act="add" title="Add to the end of the route being edited">+ Route</button><button data-act="edit">Edit</button><button data-act="del">✕</button>`;
      li.querySelector("b").textContent = m.name;
      li.querySelector("small").textContent = `${m.group ? m.group + " · " : ""}${ddm(m.lat, "N", "S")} ${ddm(m.lon, "E", "W")}${used ? ` · in ${used} route${used > 1 ? "s" : ""}` : ""}`;
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
  }
  $("lib-search").oninput = renderMarks;
  $("lib-show").checked = prefs.show;
  $("lib-show").onchange = (e) => { prefs.show = e.target.checked; try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } draw(); };

  const form = $("mark-edit");
  function editMark(m) {
    editing = { ...m };
    form.hidden = false;
    form.name.value = m.name;
    if (!form.kind.querySelector("optgroup.poi")) {
      const og = document.createElement("optgroup");
      og.className = "poi";
      og.label = "Point of interest";
      for (const [k, [label]] of Object.entries(POI_KINDS)) og.append(new Option(label, k));
      form.kind.append(og);
    }
    form.kind.value = m.kind || "mark";
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
      form.hidden = true;
      status(`Saved ${saved.name}.`);
      route?.libraryMarkChanged(saved);
      await refresh();
    } catch (err) { status(String(err)); }
  };
  form.querySelector('[data-act="cancel"]').onclick = () => { form.hidden = true; };

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
  $("lib-import").onclick = () => $("lib-file").click();
  $("lib-file").onchange = async (e) => {
    const file = e.target.files[0];
    e.target.value = "";
    if (!file) return;
    try {
      const rep = await invoke("gpx_import", { text: await file.text(), group: file.name.replace(/\.gpx$/i, "") });
      status(`${file.name}: ${rep.marks_added} new marks, ${rep.marks_matched} matched existing marks` +
        `${rep.routes_added.length ? `, routes: ${rep.routes_added.join(", ")}` : ""}${rep.tracks_added?.length ? `, tracks: ${rep.tracks_added.join(", ")}` : ""}.`);
      await refresh();
      if (rep.tracks_added?.length) window.dispatchEvent(new CustomEvent("racenav-tracks-imported", { detail: rep.tracks_added }));
    } catch (err) { status(`Import failed: ${err}`); }
  };
  $("lib-export").onclick = async () => {
    try { status(`Written to ${await invoke("gpx_export", { name: "racenav library", markIds: [], routeIds: [] })}`); } catch (e) { status(String(e)); }
  };

  /** Inline "are you sure" in place of a list row's buttons. */
  function confirmThen(li, question, action) {
    const keep = li.innerHTML;
    li.innerHTML = `<div class="main"><small></small></div><button data-act="yes" class="on">Delete</button><button data-act="no">Keep</button>`;
    li.querySelector("small").textContent = question;
    li.querySelector('[data-act="yes"]').onclick = action;
    li.querySelector('[data-act="no"]').onclick = () => { li.innerHTML = keep; refresh(); };
  }

  const byId = (id) => marks.find((m) => m.id === id);

  refresh();
  return {
    refresh,
    marks: () => marks,
    byId,
    attach(r, x) { route = r; routing = x; renderRoutes(); },
    onChange: (f) => listeners.push(f),
    /** Library marks drawn under the route's own (hide the ones the route shows). */
    hide(ids) { hidden = ids; draw(); },
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
