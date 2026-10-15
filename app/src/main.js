// racenav front end: offline chart, own boat, track and instrument panel.
import * as maplibregl from "maplibre-gl";
// Bundle MapLibre's web worker with the app so the map works offline.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
maplibregl.setWorkerUrl(workerUrl);
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

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
      { id: "land-world", type: "fill", source: "land-world", maxzoom: 6,
        paint: { "fill-color": css("--land"), "fill-outline-color": css("--coast") } },
      { id: "land-nz", type: "fill", source: "land-nz", minzoom: 4,
        paint: { "fill-color": css("--land") } },
      { id: "coast-nz", type: "line", source: "land-nz", minzoom: 4,
        paint: { "line-color": css("--coast"), "line-width": 0.8 } },
      // LINZ hydrographic chart data (not for navigation)
      { id: "depare", type: "fill", source: "chart", "source-layer": "depare",
        layout: { "fill-sort-key": ["get", "band"] },
        paint: { "fill-color": depareColour(), "fill-antialias": false } },
      { id: "depcnt", type: "line", source: "chart", "source-layer": "depcnt",
        paint: { "line-color": "#7d97a6", "line-width": 0.6 } },
      { id: "safety-contour", type: "line", source: "chart", "source-layer": "depcnt",
        filter: ["==", ["get", "valdco"], settings.safetyContour],
        paint: { "line-color": "#1d2a33", "line-width": 1.6 } },
      { id: "chart-land", type: "fill", source: "chart", "source-layer": "land",
        paint: { "fill-color": css("--land") } },
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
  const m = /^(sh|dp):(-?[0-9.]+)$/.exec(id);
  if (m && !map.hasImage(id)) map.addImage(id, soundingIcon(parseFloat(m[2]), m[1] === "sh"), { pixelRatio: 2 });
});

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
    if (Number.isFinite(v) && v >= 0) { settings[key] = v; saveSettings(); applyChartSettings(); }
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
listen("sk-status", ({ payload }) => {
  document.getElementById("sk-dot").classList.toggle("on", payload.connected);
  document.getElementById("sk-text").textContent = payload.connected ? "Signal K: connected" : "Signal K: not connected";
  document.getElementById("sk-text").title = payload.detail;
});

listen("vessel", ({ payload: v }) => {
  text("lat", v.lat != null ? ddm(v.lat, "N", "S") : "—");
  text("lon", v.lon != null ? ddm(v.lon, "E", "W") : "—");
  text("cog", v.cog != null ? `${Math.round(v.cog).toString().padStart(3, "0")}°T` : "—");
  text("sog", v.sog != null ? `${v.sog.toFixed(1)} kn` : "—");
  text("hdg", v.heading != null ? `${Math.round(v.heading).toString().padStart(3, "0")}°T` : "—");
  text("depth", v.depth != null ? `${v.depth.toFixed(1)} m` : "—");
  const t = v.datetime ?? v.timestamp;
  text("time", t ? t.replace("T", " ").slice(0, 19) : "—");
  if (v.lat == null || v.lon == null) return;

  const pos = [v.lon, v.lat];
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
});

// ---------- helpers ---------------------------------------------------------
function text(id, s) { document.getElementById(id).textContent = s; }
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
