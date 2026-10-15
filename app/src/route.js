// Route panel: start, marks, boat settings, compute, draw, GPX.
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";

const BOAT_DEFAULTS = { polar: "", eff: 100, tack: 60, gybe: 60 };
const ROUTE_DEFAULTS = { step: 5, radius: 0.1, current: true };

function load(key, defaults) {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem(key) || "{}") }; } catch { return { ...defaults }; }
}
function save(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch { /* ignore */ } }

export function setupRoute(map, { weather, getBoat, getSafetyDepth }) {
  const $ = (id) => document.getElementById(id);
  const boat = load("racenav.boat", BOAT_DEFAULTS);
  const opts = load("racenav.route", ROUTE_DEFAULTS);
  let start = null; // [lon, lat]
  let marks = []; // [[lon, lat]]
  let mode = null; // "start" | "marks" | null
  let last = null; // last route result

  // ---------- layers ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  for (const id of ["rt-iso", "rt-line", "rt-pts", "rt-marks"]) map.addSource(id, { type: "geojson", data: empty });
  map.addLayer({ id: "rt-iso", type: "circle", source: "rt-iso",
    paint: { "circle-radius": 1, "circle-color": "#5b6b75", "circle-opacity": 0.35 } }, "track");
  map.addLayer({ id: "rt-line", type: "line", source: "rt-line",
    paint: { "line-color": "#c2185b", "line-width": 2.5 } }, "track");
  map.addLayer({ id: "rt-pts", type: "circle", source: "rt-pts",
    paint: { "circle-radius": 3, "circle-color": "#c2185b", "circle-stroke-color": "#fff", "circle-stroke-width": 1 } }, "track");
  map.addLayer({ id: "rt-marks", type: "symbol", source: "rt-marks",
    layout: { "icon-image": ["get", "icon"], "icon-allow-overlap": true, "icon-ignore-placement": true } });

  function drawMarks() {
    const f = [];
    if (start) f.push(pt(start, { icon: "mk:S" }));
    marks.forEach((m, i) => f.push(pt(m, { icon: `mk:${i + 1}` })));
    map.getSource("rt-marks").setData({ type: "FeatureCollection", features: f });
    $("rt-status").textContent = !start ? "Set a start, then add marks." : marks.length ? "" : "Add marks by clicking the chart.";
  }

  // ---------- boat settings ----------------------------------------------------------
  async function loadPolars() {
    const list = await invoke("polar_files").catch(() => []);
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
  for (const [id, key] of [["rt-step", "step"], ["rt-radius", "radius"]]) {
    $(id).value = opts[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v) && v > 0) { opts[key] = v; save("racenav.route", opts); } };
  }
  $("rt-current").checked = opts.current;
  $("rt-current").onchange = (e) => { opts.current = e.target.checked; save("racenav.route", opts); };

  // ---------- start and marks ----------------------------------------------------------
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
    if (!b) { $("rt-status").textContent = "No boat position yet (Signal K)."; return; }
    start = b; drawMarks();
  };
  $("rt-undo").onclick = () => { marks.pop(); drawMarks(); };
  $("rt-clear").onclick = () => {
    start = null; marks = []; last = null; arm(null);
    for (const id of ["rt-iso", "rt-line", "rt-pts"]) map.getSource(id).setData(empty);
    document.querySelector("#rt-summary tbody").innerHTML = "";
    drawMarks();
  };
  map.on("click", (e) => {
    if (!mode) return;
    const p = [e.lngLat.lng, e.lngLat.lat];
    if (mode === "start") { start = p; arm(null); } else { marks.push(p); }
    drawMarks();
  });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && mode) arm(null); });

  // Start time follows the weather slider.
  weather.onTime((t) => { $("rt-when").textContent = t ? `Start: ${fmt(t)}` : "Start: load a GRIB first"; });

  // ---------- compute ----------------------------------------------------------
  $("rt-go").onclick = async () => {
    const w = weather.current();
    if (!w.path) { $("rt-status").textContent = "Load a GRIB in the Weather section first."; return; }
    if (!boat.polar) { $("rt-status").textContent = "Choose a polar in the Boat section."; return; }
    if (!start || !marks.length) { $("rt-status").textContent = "Set a start and at least one mark."; return; }
    arm(null);
    $("rt-go").disabled = true;
    $("rt-status").textContent = `Routing with ${w.name}…`;
    try {
      const r = await invoke("route_compute", { req: {
        start, marks, t0: w.t, grib_path: w.path, polar_path: boat.polar, safety_depth: getSafetyDepth(),
        params: { step_s: opts.step * 60, efficiency: boat.eff / 100, tack_s: boat.tack, gybe_s: boat.gybe,
                  arrival_nm: opts.radius, use_current: opts.current, keep_isochrones_every: 1 },
      } });
      last = { ...r, grib: w.name, t0: r.legs[0]?.start ?? w.t };
      show(last);
    } catch (e) {
      $("rt-status").textContent = `Routing failed: ${e}`;
    } finally {
      $("rt-go").disabled = false;
    }
  };

  function show(r) {
    const line = r.points.map((p) => [p.lon, p.lat]);
    map.getSource("rt-line").setData({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: line } });
    map.getSource("rt-pts").setData({ type: "FeatureCollection", features: r.points.map((p, i) => pt([p.lon, p.lat], { i })) });
    const iso = [];
    const every = Math.max(1, Math.ceil(r.isochrones.length / 25));
    r.isochrones.forEach((ring, k) => { if (k % every === 0) for (const c of ring) iso.push(pt(c, {})); });
    map.getSource("rt-iso").setData({ type: "FeatureCollection", features: iso });

    const ok = r.status === "arrived";
    $("rt-status").textContent = ok ? `Routed in ${(r.compute_ms / 1000).toFixed(1)} s` +
      (r.obstacles?.charts ? ` · land and < ${getSafetyDepth()} m avoided (${r.obstacles.cell_m} m cells)` : " · no chart: land not avoided")
      : `${r.status}: ${r.message}`;
    const rows = [];
    if (ok) {
      const dur = r.arrival - r.t0;
      rows.push(["ETA", fmt(r.arrival)], ["Time", `${Math.floor(dur / 3600)} h ${String(Math.round((dur % 3600) / 60)).padStart(2, "0")} min`]);
    }
    rows.push(["Distance", `${r.distance_nm.toFixed(1)} NM`]);
    if (ok) rows.push(["Average speed", `${(r.distance_nm / ((r.arrival - r.t0) / 3600)).toFixed(1)} kn`]);
    rows.push(["Tacks / gybes", `${r.legs.reduce((a, l) => a + l.tacks, 0)} / ${r.legs.reduce((a, l) => a + l.gybes, 0)}`]);
    r.legs.forEach((l, i) => rows.push([`Leg ${i + 1}`, l.arrival ? `${l.distance_nm.toFixed(1)} NM, at ${fmt(l.arrival, true)}` : "not reached"]));
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

  // Hover a route point: what the boat is doing there.
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

  // ---------- GPX ----------------------------------------------------------
  $("rt-gpx").onclick = () => {
    if (!last || !last.points.length) { $("rt-status").textContent = "Compute a route first."; return; }
    const name = `racenav ${new Date(last.t0 * 1000).toISOString().slice(0, 16).replace("T", " ")}Z`;
    const pts = last.points.map((p, i) =>
      `    <rtept lat="${p.lat.toFixed(6)}" lon="${p.lon.toFixed(6)}"><time>${new Date(p.t * 1000).toISOString()}</time><name>R${String(i).padStart(3, "0")}</name></rtept>`).join("\n");
    const gpx = `<?xml version="1.0" encoding="UTF-8"?>\n<gpx version="1.1" creator="racenav" xmlns="http://www.topografix.com/GPX/1/1">\n  <rte>\n    <name>${name}</name>\n${pts}\n  </rte>\n</gpx>\n`;
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([gpx], { type: "application/gpx+xml" }));
    a.download = `${name.replace(/[: ]/g, "-")}.gpx`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
  };

  loadPolars();
  drawMarks();
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
