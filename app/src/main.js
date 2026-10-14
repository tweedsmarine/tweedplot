// racenav front end: offline chart, own boat, track and instrument panel.
import * as maplibregl from "maplibre-gl";
// Bundle MapLibre's web worker with the app so the map works offline.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
maplibregl.setWorkerUrl(workerUrl);
import { listen } from "@tauri-apps/api/event";

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

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
map.addControl(new maplibregl.NavigationControl({ visualizePitch: false }), "top-left");
map.addControl(new maplibregl.ScaleControl({ unit: "nautical" }), "bottom-right");
map.on("load", () => map.addImage("boat", boatIcon(), { pixelRatio: 2 }));

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
