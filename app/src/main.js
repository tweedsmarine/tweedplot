// racenav front end: offline chart, own boat, track and instrument panel.
import * as maplibregl from "maplibre-gl";
// Bundle MapLibre's web worker with the app so the map works offline.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
maplibregl.setWorkerUrl(workerUrl);
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { setupWeather, barbImage } from "./weather.js";
import { setupRoute, markImage } from "./route.js";
import { setupRouting } from "./routing.js";
import { setupBoat } from "./boat.js";
import { setupDock } from "./dock.js";
import { setupRail } from "./ui.js";
import { setupInstruments, waypointData, targetAt } from "./instruments.js";
import { setupLibrary, labelImage } from "./library.js";

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

// ---------- chart settings (kept per viewer in local storage) ----------------
const DEFAULTS = { shallow: 2, safetyDepth: 3, safetyContour: 5, deep: 10 };
let settings = { ...DEFAULTS };
try { Object.assign(settings, JSON.parse(localStorage.getItem("racenav.chart") || "{}")); } catch { /* storage unavailable */ }
function saveSettings() { try { localStorage.setItem("racenav.chart", JSON.stringify(settings)); } catch { /* ignore */ } }

// Chart tiles come from the Rust chart store through the app's own "chart" protocol.
const TILE_URL = window.RACENAV_TILE_URL
  ?? (navigator.userAgent.includes("Windows") ? "http://chart.localhost/{z}/{x}/{y}.pbf" : "chart://localhost/{z}/{x}/{y}.pbf");

// S-52 style depth shades (day palette).
const DEPTH = { drying: "#a9c8a0", vshallow: "#7fb8e8", mshallow: "#a6cdf0", mdeep: "#c9e1ee", deep: "#e9f3f6" };
function depareColour() {
  const d1 = ["coalesce", ["get", "drval1"], 0];
  return ["case",
    ["<", d1, 0], DEPTH.drying,
    ["<", d1, settings.shallow], DEPTH.vshallow,
    ["<", d1, settings.safetyContour], DEPTH.mshallow,
    ["<", d1, settings.deep], DEPTH.mdeep,
    DEPTH.deep];
}
function soundingImage() {
  return ["concat", ["case", ["<=", ["get", "depth"], settings.safetyDepth], "sh:", "dp:"], ["to-string", ["get", "depth"]]];
}

// ---------- chart ----------------------------------------------------------
const map = new maplibregl.Map({
  container: "map",
  center: [173.27, -41.26], // Nelson
  zoom: 8,
  attributionControl: false,
  style: {
    version: 8,
    sources: {
      "land-world": { type: "geojson", data: "data/land-world.geojson" },
      "land-nz": { type: "geojson", data: "data/land-nz.geojson" },
      graticule: { type: "geojson", data: graticule(1) },
      track: { type: "geojson", data: line([]) },
      boat: { type: "geojson", data: emptyFC() },
      cogvec: { type: "geojson", data: line([]) },
      chart: { type: "vector", tiles: [TILE_URL], minzoom: 0, maxzoom: 16 },
    },
    layers: [
      { id: "sea", type: "background", paint: { "background-color": css("--sea") } },
      { id: "grid", type: "line", source: "graticule", paint: { "line-color": "#9db8c8", "line-width": 0.5 } },
      // LINZ hydrographic chart data (not for navigation)
      { id: "depare", type: "fill", source: "chart", "source-layer": "depare",
        layout: { "fill-sort-key": ["get", "band"] },
        paint: { "fill-color": depareColour(), "fill-antialias": false } },
      { id: "depcnt", type: "line", source: "chart", "source-layer": "depcnt",
        paint: { "line-color": "#7d97a6", "line-width": 0.6 } },
      { id: "safety-contour", type: "line", source: "chart", "source-layer": "depcnt",
        filter: ["==", ["get", "valdco"], settings.safetyContour],
        paint: { "line-color": "#1d2a33", "line-width": 1.6 } },
      // Land fills, then coastlines. Wind shading goes under the land fills
      // ("land on top") or between the fills and the coastlines ("coastline on top").
      { id: "land-world", type: "fill", source: "land-world", maxzoom: 6,
        paint: { "fill-color": css("--land") } },
      { id: "land-nz", type: "fill", source: "land-nz", minzoom: 4,
        paint: { "fill-color": css("--land") } },
      { id: "chart-land", type: "fill", source: "chart", "source-layer": "land",
        paint: { "fill-color": css("--land") } },
      { id: "coast-world", type: "line", source: "land-world", maxzoom: 6,
        paint: { "line-color": css("--coast"), "line-width": 0.8 } },
      { id: "coast-nz", type: "line", source: "land-nz", minzoom: 4,
        paint: { "line-color": css("--coast"), "line-width": 0.8 } },
      { id: "chart-coast", type: "line", source: "chart", "source-layer": "land",
        paint: { "line-color": css("--coast"), "line-width": 0.9 } },
      { id: "soundg", type: "symbol", source: "chart", "source-layer": "soundg",
        layout: { "icon-image": soundingImage(), "icon-allow-overlap": false, "icon-padding": 1 } },
      { id: "track", type: "line", source: "track",
        paint: { "line-color": "#d35400", "line-width": 2 } },
      { id: "cogvec", type: "line", source: "cogvec",
        paint: { "line-color": "#1d2a33", "line-width": 1.5, "line-dasharray": [3, 2] } },
      { id: "boat", type: "symbol", source: "boat",
        layout: { "icon-image": "boat", "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map",
                  "icon-allow-overlap": true, "icon-ignore-placement": true } },
    ],
  },
});
window.__map = map; // handy for debugging in devtools
map.addControl(new maplibregl.NavigationControl({ visualizePitch: false }), "top-left");
map.addControl(new maplibregl.ScaleControl({ unit: "nautical" }), "bottom-right");
map.on("load", () => map.addImage("boat", boatIcon(), { pixelRatio: 2 }));

// Sounding figures are drawn on demand: no font files needed offline.
map.setMissingStyleImageResolver((id) => {
  if (map.hasImage(id)) return;
  const m = /^(sh|dp):(-?[0-9.]+)$/.exec(id);
  if (m) { map.addImage(id, soundingIcon(parseFloat(m[2]), m[1] === "sh"), { pixelRatio: 2 }); return; }
  const img = barbImage(id) || markImage(id) || labelImage(id);
  if (img) map.addImage(id, img, { pixelRatio: 2 });
});

// Weather: GRIB wind shading sits under the land, barbs under the boat and track.
let weather = null;
let lastBoat = null; // [lon, lat] from Signal K, or placed by hand when planning
let route = null; // the course being edited
let routing = null;
let library = null;
const ui = setupRail(map);
const inst = setupInstruments();
const boat = setupBoat();
const dock = setupDock(map);
map.on("load", () => {
  weather = setupWeather(map, { landTop: "land-world", coastTop: "coast-world", top: "track" });
  library = setupLibrary(map, { ui });
  route = setupRoute(map, { library, getBoat: () => lastBoat, getArrivalNm: () => Number(document.getElementById("rt-radius").value) || 0.1 });
  routing = setupRouting(map, { weather, course: route, library, dock, ui, getBoat: () => lastBoat,
    getSafetyDepth: () => settings.safetyDepth, getBoatSettings: boat.get });
  library.attach(route, routing);
  library.onChange(feed);
  feed();
});

// ---------- instruments ----------------------------------------------------------
let vessel = {}; // latest Signal K state
function boatTime() {
  // planning: the weather slider's time; live: the GPS time, else the clock
  if (!live && weather?.current().t) return weather.current().t;
  const t = Date.parse(vessel.datetime ?? "");
  return Number.isFinite(t) ? t / 1000 : Date.now() / 1000;
}
function feed() {
  const nav = route?.nav() ?? null;
  const pos = lastBoat;
  const wp = pos && nav ? waypointData(pos, vessel.sog, vessel.cog, nav.from, [nav.next.lon, nav.next.lat]) : null;
  const pts = routing?.activePoints();
  const target = pts ? targetAt(pts, boatTime()) : null;
  const v = live ? vessel : { ...vessel, lat: pos?.[1], lon: pos?.[0] };
  inst.update({ v, dataAt: live ? lastData : 0, nav, wp, target, safetyDepth: settings.safetyDepth });
}
setInterval(feed, 1000);

// When chart data is present, the Topo50 coastline replaces the coarse land from zoom 6.
function chartsLoaded(info) {
  if (!info || !(info.tables > 0)) return;
  map.setLayerZoomRange("land-nz", 4, 6);
  map.setLayerZoomRange("coast-nz", 4, 6);
  const src = map.getSource("chart");
  if (src) src.setTiles([TILE_URL]); // reload tiles requested before the charts were open
}
map.on("load", () => {
  invoke("chart_info").then(chartsLoaded).catch(() => {});
  listen("chart-ready", ({ payload }) => chartsLoaded(payload));
});

function applyChartSettings() {
  map.setPaintProperty("depare", "fill-color", depareColour());
  map.setFilter("safety-contour", ["==", ["get", "valdco"], settings.safetyContour]);
  map.setLayoutProperty("soundg", "icon-image", soundingImage());
}
for (const [id, key] of [["set-safety-depth", "safetyDepth"], ["set-safety-contour", "safetyContour"]]) {
  const el = document.getElementById(id);
  el.value = settings[key];
  el.onchange = () => {
    const v = parseFloat(el.value);
    if (Number.isFinite(v) && v >= 0) { settings[key] = v; saveSettings(); applyChartSettings(); feed(); }
  };
}

map.on("mousemove", (e) => {
  document.getElementById("cursor").textContent = `${ddm(e.lngLat.lat, "N", "S")}  ${ddm(e.lngLat.lng, "E", "W")}`;
});

// ---------- follow / track -------------------------------------------------
let follow = true;
const followBtn = document.getElementById("follow");
followBtn.onclick = () => { follow = !follow; followBtn.classList.toggle("on", follow); };
map.on("dragstart", () => { follow = false; followBtn.classList.remove("on"); });

const TRACK_MAX = 20000;
let track = [];
document.getElementById("clear-track").onclick = () => { track = []; setData("track", line(track)); };

// ---------- Signal K events -------------------------------------------------
// ---------- Signal K status, live / planning ----------------------------------
let skConnected = false;
let lastData = 0; // ms timestamp of the last vessel update
let live = true;
function showStatus() {
  const dot = document.getElementById("sk-dot");
  dot.classList.toggle("on", live && skConnected);
  dot.classList.toggle("paused", !live);
  document.getElementById("sk-text").textContent = !live
    ? "Signal K: paused (planning)"
    : skConnected ? "Signal K: connected" : "Signal K: not connected";
}
function setStatus(payload) {
  skConnected = payload.connected;
  document.getElementById("sk-text").title = payload.detail;
  document.getElementById("sk-detail").textContent = `${payload.connected ? "Connected to" : "Not connected:"} ${payload.detail}`;
  showStatus();
}
listen("sk-status", ({ payload }) => setStatus(payload));
// The page can load after the first status event was sent: ask for it.
invoke("sk_status").then(setStatus).catch(() => {});
const liveBtn = document.getElementById("sk-live");
liveBtn.onclick = () => {
  live = !live;
  liveBtn.classList.toggle("on", live);
  liveBtn.textContent = live ? "Live" : "Paused";
  showStatus();
  feed();
};
const signalKActive = () => live && skConnected && Date.now() - lastData < 15000;

/** Put the boat at a position by hand (planning, or when there is no Signal K). */
function placeBoat(pos) {
  lastBoat = pos;
  setData("boat", { type: "FeatureCollection", features: [{ type: "Feature", properties: { rot: 0 }, geometry: { type: "Point", coordinates: pos } }] });
  setData("cogvec", line([]));
  feed();
}

// ---------- right-click menu ----------------------------------------------------
const ctx = document.getElementById("ctx");
let ctxAt = null;
let ctxMark = null; // library mark under the right-click, if any
map.on("contextmenu", (e) => {
  e.originalEvent.preventDefault();
  ctxAt = [e.lngLat.lng, e.lngLat.lat];
  // right-click on a library mark: start / add use that mark
  ctxMark = library?.markAt(e.point) ?? null;
  ctx.querySelector('[data-act="start"]').textContent = ctxMark ? `Start route at ${ctxMark.name}` : "Start route here";
  ctx.querySelector('[data-act="mark"]').textContent = ctxMark ? `Add ${ctxMark.name} to route` : "Add mark to route here";
  ctx.querySelector('[data-act="libmark"]').hidden = !!ctxMark;
  // selected mark: offer Move (only once its ring is green) and Delete
  const sel = route?.selection();
  for (const el of ctx.querySelectorAll(".sel-item")) el.hidden = !sel;
  if (sel) {
    const mv = ctx.querySelector('[data-act="move-sel"]');
    mv.textContent = `Move ${sel.label} here`;
    mv.disabled = !sel.movable;
    mv.title = sel.movable ? "" : "Wait for the ring round the mark to turn green";
    ctx.querySelector('[data-act="del-sel"]').textContent = `Delete ${sel.label}`;
  }
  const boatBtn = ctx.querySelector('[data-act="boat"]');
  boatBtn.disabled = signalKActive();
  boatBtn.title = boatBtn.disabled ? "Signal K is plotting the boat: press Live to pause it first" : "";
  ctx.style.left = `${Math.min(e.point.x, window.innerWidth - 200)}px`;
  ctx.style.top = `${Math.min(e.point.y, window.innerHeight - 180)}px`;
  ctx.hidden = false;
});
document.addEventListener("contextmenu", (e) => e.preventDefault());
ctx.addEventListener("click", (e) => {
  const act = e.target.dataset.act;
  if (!act || e.target.disabled || !ctxAt) return;
  if (act === "move-sel") route?.moveSelected(ctxAt);
  if (act === "del-sel") route?.deleteSelected();
  if (act === "boat") placeBoat(ctxAt);
  if (act === "start") route?.setStart(ctxAt, ctxMark);
  if (act === "mark") route?.addMark(ctxAt, ctxMark);
  if (act === "libmark") library?.newMarkAt(ctxAt);
  if (act === "centre") map.easeTo({ center: ctxAt });
  ctx.hidden = true;
});
map.on("click", () => { ctx.hidden = true; });
map.on("movestart", () => { ctx.hidden = true; });
window.addEventListener("keydown", (e) => { if (e.key === "Escape") ctx.hidden = true; });

listen("vessel", ({ payload: v }) => {
  if (!skConnected) { skConnected = true; showStatus(); } // data proves the link is up
  lastData = Date.now();
  if (!live) return; // planning: leave the boat where it is
  vessel = v;
  if (v.lat == null || v.lon == null) { feed(); return; }

  const pos = [v.lon, v.lat];
  lastBoat = pos;
  const dir = v.heading ?? v.cog ?? 0;
  setData("boat", { type: "FeatureCollection", features: [{ type: "Feature", properties: { rot: dir }, geometry: { type: "Point", coordinates: pos } }] });

  // COG/SOG vector: where the boat will be in 6 minutes.
  if (v.cog != null && v.sog != null && v.sog > 0.2) {
    setData("cogvec", line([pos, destination(v.lat, v.lon, v.cog, v.sog * 0.1)]));
  } else {
    setData("cogvec", line([]));
  }

  const last = track.at(-1);
  if (!last || Math.abs(last[0] - pos[0]) > 1e-5 || Math.abs(last[1] - pos[1]) > 1e-5) {
    track.push(pos);
    if (track.length > TRACK_MAX) track.shift();
    setData("track", line(track));
  }
  if (follow) map.easeTo({ center: pos, duration: 200 });
  route?.boatAt(pos);
  feed();
});

// ---------- helpers ---------------------------------------------------------
function setData(id, data) { const s = map.getSource(id); if (s) s.setData(data); }
function emptyFC() { return { type: "FeatureCollection", features: [] }; }
function line(coords) { return { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: coords } }; }

/** Degrees and decimal minutes, e.g. 41°15.420'S. */
function ddm(deg, pos, neg) {
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}

/** Great-circle destination (same formula as nav-core). */
function destination(lat, lon, brg, nm) {
  const R = 3440.065, d = nm / R, t = (brg * Math.PI) / 180;
  const p1 = (lat * Math.PI) / 180, l1 = (lon * Math.PI) / 180;
  const p2 = Math.asin(Math.sin(p1) * Math.cos(d) + Math.cos(p1) * Math.sin(d) * Math.cos(t));
  const l2 = l1 + Math.atan2(Math.sin(t) * Math.sin(d) * Math.cos(p1), Math.cos(d) - Math.sin(p1) * Math.sin(p2));
  return [(l2 * 180) / Math.PI, (p2 * 180) / Math.PI];
}

/** Lat/lon grid every `step` degrees. */
function graticule(step) {
  const f = [];
  for (let lon = -180; lon <= 180; lon += step) f.push(line(Array.from({ length: 171 }, (_, i) => [lon, -85 + i])).geometry);
  for (let lat = -80; lat <= 80; lat += step) f.push(line(Array.from({ length: 361 }, (_, i) => [-180 + i, lat])).geometry);
  return { type: "FeatureCollection", features: f.map((g) => ({ type: "Feature", properties: {}, geometry: g })) };
}

/** Own-boat icon: a pointed hull, drawn once on a canvas. */
function boatIcon() {
  const w = 40, h = 64, c = document.createElement("canvas");
  c.width = w; c.height = h;
  const g = c.getContext("2d");
  g.beginPath();
  g.moveTo(w / 2, 2);
  g.bezierCurveTo(w - 4, 22, w - 6, 46, w - 10, h - 4);
  g.lineTo(10, h - 4);
  g.bezierCurveTo(6, 46, 4, 22, w / 2, 2);
  g.closePath();
  g.fillStyle = "#c0392b";
  g.strokeStyle = "#1d2a33";
  g.lineWidth = 3;
  g.fill();
  g.stroke();
  return g.getImageData(0, 0, w, h);
}

/** Sounding figure in S-52 style: metres, tenths as a smaller subscript below 31 m, drying heights underlined. */
function soundingIcon(depth, shallow) {
  const dry = depth < 0;
  const a = Math.abs(depth);
  const whole = a < 31 ? Math.floor(a) : Math.round(a);
  const tenth = a < 31 ? Math.round((a - whole) * 10) % 10 : 0;
  const big = 22, small = 16, pad = 2;
  const c = document.createElement("canvas");
  const g = c.getContext("2d");
  const fBig = `${big}px "Segoe UI", system-ui, sans-serif`, fSmall = `${small}px "Segoe UI", system-ui, sans-serif`;
  g.font = fBig;
  const w1 = g.measureText(String(whole)).width;
  g.font = fSmall;
  const w2 = tenth ? g.measureText(String(tenth)).width : 0;
  c.width = Math.ceil(w1 + w2 + pad * 2);
  c.height = big + 8;
  g.fillStyle = shallow ? "#1d2a33" : "#7d8f99";
  g.textBaseline = "alphabetic";
  g.font = fBig;
  g.fillText(String(whole), pad, big - 2);
  if (tenth) { g.font = fSmall; g.fillText(String(tenth), pad + w1, big + 3); }
  if (dry) { g.fillRect(pad, big + 4, w1 + w2, 2); }
  return g.getImageData(0, 0, c.width, c.height);
}
