// racenav front end: offline chart, own boat, track and instrument panel.
import * as maplibregl from "maplibre-gl";
// Bundle MapLibre's web worker with the app so the map works offline.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
maplibregl.setWorkerUrl(workerUrl);
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { setupWeather, barbImage } from "./weather.js";
import { setupChartFeatures, featureImage } from "./chartfeatures.js";
import { setupTides, tideImage } from "./tides.js";
import { setupTracks } from "./tracks.js";
import { setupRuler } from "./ruler.js";
import { setupGribTools } from "./gribtools.js";
import { setupWindArrows } from "./windarrows.js";
import { poiImage } from "./poi.js";
import { setupBarriers } from "./barriers.js";
import { setupRoute, markImage } from "./route.js";
import { setupRouting } from "./routing.js";
import { setupBoat } from "./boat.js";
import { setupDock } from "./dock.js";
import { createDamping } from "./damping.js";
import { setupFleet } from "./fleet.js";
import { setupGribDownload } from "./gribdl.js";
import { setupRail } from "./ui.js";
import { setupManual } from "./manual.js";
import { setupData } from "./data.js";
import { setupInstruments, waypointData, targetAt } from "./instruments.js";
import { setupLibrary, labelImage } from "./library.js";

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

// ---------- chart settings (kept per viewer in local storage) ----------------
const DEFAULTS = {
  shallow: 2, safetyDepth: 3, safetyContour: 5, deep: 10,
  // own-boat lines: shown, and length in minutes of travel or "inf" (to the edge of the chart)
  hdgLine: true, hdgLen: "inf", cogLine: true, cogLen: "10",
  // damping of Signal K readings, seconds (0 = none)
  dampMotion: 2, dampWind: 5, dampDepth: 0,
};
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
      hdgline: { type: "geojson", data: line([]) },
      cogline: { type: "geojson", data: line([]) },
      chart: { type: "vector", tiles: [TILE_URL], minzoom: 0, maxzoom: 16 },
    },
    layers: [
      { id: "sea", type: "background", paint: { "background-color": css("--sea") } },
      { id: "grid", type: "line", source: "graticule", paint: { "line-color": "#9db8c8", "line-width": 0.5 } },
      // World depths under the charts: Natural Earth (bundled) and downloaded ETOPO areas (not for navigation)
      { id: "wdepth", type: "fill", source: "chart", "source-layer": "wdepth",
        layout: { "fill-sort-key": ["+", ["*", ["get", "src"], 100000], ["get", "d"]] },
        paint: { "fill-color": ["step", ["get", "d"], "#a6cdf0", 10, "#b7d8f2", 20, "#c9e1ee", 50, "#dcecf3", 200, "#e4f0f5",
          1000, "#dbeaf2", 2000, "#d3e5ef", 3000, "#cbe0ec", 4000, "#c3dbe9", 5000, "#bbd6e6", 6000, "#b3d1e3"], "fill-antialias": false } },
      { id: "wdcnt", type: "line", source: "chart", "source-layer": "wdcnt", minzoom: 7,
        paint: { "line-color": "#8fa9b8", "line-width": 0.6, "line-opacity": 0.8 } },
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
      { id: "hdgline", type: "line", source: "hdgline",
        paint: { "line-color": "#1d2a33", "line-width": 1.6 } },
      { id: "cogline", type: "line", source: "cogline",
        paint: { "line-color": "#d35400", "line-width": 2, "line-dasharray": [3, 2] } },
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
  const img = barbImage(id) || markImage(id) || labelImage(id) || featureImage(id) || tideImage(id) || poiImage(id);
  if (img) map.addImage(id, img, { pixelRatio: 2 });
});

// Weather: GRIB wind shading sits under the land, barbs under the boat and track.
let weather = null;
let lastBoat = null; // [lon, lat] from Signal K, or placed by hand when planning
let route = null; // the course being edited
let routing = null;
let library = null;
const ui = setupRail(map);

// The user manual (bundled with the app): rail button, F1, ? in each drawer.
const manual = setupManual();
document.getElementById("manual-btn").onclick = () => manual.toggle();
manual.onToggle((on) => document.getElementById("manual-btn").classList.toggle("on", on));
document.getElementById("drawer-help").onclick = () => manual.help(ui.current());
const inst = setupInstruments();
const boat = setupBoat();
const dock = setupDock(map);
let features = null;
let ruler = null;
let gribTools = null;
let arrows = null;
let barriers = null;

/** A short message at the bottom of the chart ("" hides it); gone after `ms` if given. */
let toastTimer = null;
function toast(text, ms = 0) {
  const el = document.getElementById("toast");
  clearTimeout(toastTimer);
  el.textContent = text;
  el.hidden = !text;
  if (text && ms) toastTimer = setTimeout(() => { el.hidden = true; }, ms);
}
map.on("load", () => {
  features = setupChartFeatures(map, { before: "track", safetyContour: settings.safetyContour });
  weather = setupWeather(map, { landTop: "land-world", coastTop: "coast-world", top: "track" });
  window.__weather = weather; // debugging handle
  setupData(map, { toast });
  library = setupLibrary(map, { ui });
  route = setupRoute(map, { library, getBoat: () => lastBoat, getArrivalNm: () => Number(document.getElementById("rt-radius").value) || 0.1 });
  routing = setupRouting(map, { weather, course: route, library, dock, ui, getBoat: () => lastBoat,
    getSafetyDepth: () => settings.safetyDepth, getAvoid: () => features.avoid(), getBoatSettings: boat.get });
  library.attach(route, routing);
  setupFleet(map, { weather, course: route, routing, dock, ui });
  setupGribDownload(map, { weather, course: route });
  gribTools = setupGribTools(map, { weather, dock });
  document.getElementById("wx-verify").onclick = () => gribTools.openVerify();
  setupTides(map, { before: "track", getTime: boatTime, onTime: (fn) => weather.onTime(fn) });
  ruler = setupRuler(map, { before: "track", ui, library, route, toast, getVariation: () => vessel.variation });
  setupTracks(map, { before: "track", getTime: boatTime, onTime: (fn) => weather.onTime(fn),
    status: (s) => { document.getElementById("lib-status").textContent = s; } });
  barriers = setupBarriers(map, { before: "track", library, toast });
  arrows = setupWindArrows(map, { before: "boat" });
  const wa = document.getElementById("inst-arrows");
  wa.checked = arrows.enabled();
  wa.onchange = () => { arrows.setEnabled(wa.checked); if (live && lastBoat) arrows.live(lastBoat, vessel); };
  weather.onPoint((row, at) => {
    inst.update({ grib: row });
    if (!signalKActive()) planningArrows(row, at); // planning, or no live data: arrows on the cursor
  });
  library.onChange(feed);
  feed();
});

/** Planning: wind arrows at the cursor. True wind from the GRIB; apparent wind only
 * where a shown routing passes near the cursor (its heading and boat speed there):
 * without a routing there is no boat, so no apparent wind. */
let arrowSeq = 0;
async function planningArrows(row, at) {
  if (!arrows) return;
  if (!row || !at || row.tws == null || row.twd == null) { arrows.clear(); return; }
  const seq = ++arrowSeq;
  const tw = { twd: row.twd, tws: row.tws };
  let hdg = null, bsp = null;
  const pts = routing?.activePoints();
  if (pts?.length) {
    const c = map.project(at);
    let best = null, bd = 40 * 40;
    for (const p of pts) {
      const q = map.project([p.lon, p.lat]);
      const d = (q.x - c.x) ** 2 + (q.y - c.y) ** 2;
      if (d < bd) { bd = d; best = p; }
    }
    if (best) { hdg = best.heading; bsp = best.bsp; }
  }
  if (seq !== arrowSeq || signalKActive()) return;
  arrows.planning(at, tw, hdg, bsp);
}

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
  if (!info) return;
  window.dispatchEvent(new CustomEvent("racenav-charts", { detail: info }));
  if (!(info.tables > 0)) return;
  map.setLayerZoomRange("land-nz", 4, 6);
  map.setLayerZoomRange("coast-nz", 4, 6);
  if (info.world_land) {
    // GSHHG world land comes in the chart tiles at every zoom: the bundled coarse land is not needed
    for (const id of ["land-world", "coast-world", "land-nz", "coast-nz"]) map.setLayoutProperty(id, "visibility", "none");
  }
  const src = map.getSource("chart");
  // reload the tiles (asked for before the charts were open, or the chart data changed)
  if (src) src.setTiles([TILE_URL + (TILE_URL.includes("?") ? "&" : "?") + "v=" + Date.now()]);
}
map.on("load", () => {
  invoke("chart_info").then(chartsLoaded).catch(() => {});
  listen("chart-ready", ({ payload }) => chartsLoaded(payload));
});

function applyChartSettings() {
  map.setPaintProperty("depare", "fill-color", depareColour());
  map.setFilter("safety-contour", ["==", ["get", "valdco"], settings.safetyContour]);
  map.setLayoutProperty("soundg", "icon-image", soundingImage());
  features?.setSafetyContour(settings.safetyContour);
}
for (const [id, key] of [["set-safety-depth", "safetyDepth"], ["set-safety-contour", "safetyContour"]]) {
  const el = document.getElementById(id);
  el.value = settings[key];
  el.onchange = () => {
    const v = parseFloat(el.value);
    if (Number.isFinite(v) && v >= 0) { settings[key] = v; saveSettings(); applyChartSettings(); feed(); }
  };
}

// cursor position: the box at the bottom left and the Cursor instrument tiles
// at most ten updates a second: steadier to read, and less work while the mouse moves
let cursorNext = null, cursorTimer = null, cursorLast = 0;
function setCursor(c) {
  cursorNext = c;
  if (cursorTimer) return;
  const wait = Math.max(0, 100 - (Date.now() - cursorLast));
  cursorTimer = setTimeout(() => {
    cursorTimer = null;
    cursorLast = Date.now();
    inst.update({ cursor: cursorNext });
  }, wait);
}
map.on("mousemove", (e) => {
  document.getElementById("cursor").textContent = `${ddm(e.lngLat.lat, "N", "S")}  ${ddm(e.lngLat.lng, "E", "W")}`;
  setCursor([e.lngLat.lng, e.lngLat.lat]);
});
// touch screens have no hover: a tap sets the cursor
map.on("click", (e) => setCursor([e.lngLat.lng, e.lngLat.lat]));
map.getCanvas().addEventListener("mouseleave", () => { setCursor(null); if (!signalKActive()) arrows?.clear(); });

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
let skDetail = "connecting…";
let skSeen = false; // a status has arrived (else still connecting)
let lastData = 0; // ms timestamp of the last vessel update
let lastRaw = null; // the last vessel update, for what is arriving
let live = true;
const SK_STALE_MS = 10000;
/** The status button: colour = the link (green data arriving, amber connected
 * but quiet for 10 s, red no link), whether plotting is live or paused. */
function showStatus() {
  const el = document.getElementById("sk-state");
  const age = lastData ? Date.now() - lastData : Infinity;
  const state = !skConnected ? "bad" : age < SK_STALE_MS ? "good" : "stale";
  el.className = state + (skSeen ? "" : " connecting");
  const ago = (ms) => (ms < 90000 ? `${Math.max(1, Math.round(ms / 1000))} s` : ms < 5400000 ? `${Math.round(ms / 60000)} min` : `${(ms / 3600000).toFixed(1)} h`);
  const what = lastRaw ? [["lat", "position"], ["heading", "heading"], ["sog", "speed"], ["stw", "boat speed"], ["aws", "wind"], ["tws", "wind"], ["depth", "depth"], ["depth_keel", "depth"], ["depth_surface", "depth"]]
    .filter(([k]) => lastRaw[k] != null).map(([, w]) => w).filter((w, i, a) => a.indexOf(w) === i).join(", ") : "";
  const head = !skSeen ? "Signal K: connecting…"
    : state === "bad" ? "Signal K: not connected"
    : state === "good" ? "Signal K: receiving data" : "Signal K: connected, no data";
  el.title = [head, skSeen ? skDetail : "",
    lastData ? `Last update ${ago(age)} ago${what && state === "good" ? `: ${what}` : ""}` : skConnected ? "No data yet" : "",
    live ? "" : "Plotting paused (planning): the boat stays where you put it",
    "Click for the Signal K settings"].filter(Boolean).join("\n");
}
function setStatus(payload) {
  if (!payload) return;
  if (payload.connected || payload.detail !== "connecting") skSeen = true; // "connecting" = nothing heard yet
  skConnected = payload.connected;
  skDetail = payload.detail;
  document.getElementById("sk-detail").textContent = `${payload.connected ? "Connected to" : "Not connected:"} ${payload.detail}`;
  showStatus();
}
listen("sk-status", ({ payload }) => setStatus(payload));
// The page can load after the first status event was sent: ask for it.
invoke("sk_status").then(setStatus).catch(() => {});
setInterval(showStatus, 1000); // data going quiet turns it amber
document.getElementById("sk-state").onclick = () => {
  ui.open("chart");
  document.getElementById("sk-section")?.scrollIntoView({ block: "start", behavior: "smooth" });
};
const liveBtn = document.getElementById("sk-live");
liveBtn.onclick = () => {
  live = !live;
  liveBtn.classList.toggle("on", live);
  liveBtn.title = live ? "Live: the boat follows Signal K. Click to pause and plan (right-click the chart to move the boat)."
    : "Paused (planning): the boat stays where you put it. Click to go live.";
  arrows?.clear();
  if (live && lastBoat) arrows?.live(lastBoat, vessel);
  showStatus();
  feed();
};
const signalKActive = () => live && skConnected && Date.now() - lastData < 15000;

/** Put the boat at a position by hand (planning, or when there is no Signal K). */
function placeBoat(pos) {
  lastBoat = pos;
  setData("boat", { type: "FeatureCollection", features: [{ type: "Feature", properties: { rot: 0 }, geometry: { type: "Point", coordinates: pos } }] });
  drawLines(null);
  feed();
}

// ---------- right-click menu ----------------------------------------------------
const ctx = document.getElementById("ctx");
let ctxAt = null;
let ctxMark = null; // library mark under the right-click, if any
let ctxRuler = null; // ruler under the right-click: { rid, i, points }
let ctxBarrier = null; // barrier under the right-click: { id, i, active, name }
map.on("contextmenu", (e) => {
  e.originalEvent.preventDefault();
  ctxAt = [e.lngLat.lng, e.lngLat.lat];
  // right-click on a library mark: start / add use that mark
  ctxMark = library?.markAt(e.point) ?? null;
  ctx.querySelector('[data-act="start"]').textContent = ctxMark ? `Start route at ${ctxMark.name}` : "Start route here";
  ctx.querySelector('[data-act="mark"]').textContent = ctxMark ? `Add ${ctxMark.name} to route` : "Add mark to route here";
  ctx.querySelector('[data-act="libmark"]').hidden = !!ctxMark;
  // a ruler under the right-click: delete it, a point of it, or save it as a route
  ctxRuler = ruler?.isArmed() ? null : ruler?.hitAt(e.point) ?? null;
  for (const el of ctx.querySelectorAll(".ruler-item")) el.hidden = !ctxRuler;
  ctxBarrier = ctxRuler || barriers?.isDrawing() ? null : barriers?.hitAt(e.point) ?? null;
  for (const el of ctx.querySelectorAll(".bar-item")) el.hidden = !ctxBarrier;
  if (ctxBarrier) {
    ctx.querySelector('[data-act="bar-pt"]').hidden = ctxBarrier.i == null;
    ctx.querySelector('[data-act="bar-toggle"]').textContent = ctxBarrier.active ? `Switch “${ctxBarrier.name}” off` : `Switch “${ctxBarrier.name}” on`;
    ctx.querySelector('[data-act="bar-del"]').textContent = `Delete “${ctxBarrier.name}”`;
  }
  if (ctxRuler) {
    ctx.querySelector('[data-act="ruler-pt"]').hidden = ctxRuler.i == null;
    ctx.querySelector('[data-act="ruler-all"]').hidden = ruler.count() < 2;
  }
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
  ctx.style.top = `${Math.min(e.point.y, window.innerHeight - (ctxRuler || ctxBarrier ? 320 : 200))}px`;
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
  if (act === "meteogram") gribTools?.meteogramAt(ctxAt);
  if (act === "ruler-route" && ctxRuler) ruler.saveAsRoute(ctxRuler.rid);
  if (act === "ruler-pt" && ctxRuler) ruler.deletePoint(ctxRuler.rid, ctxRuler.i);
  if (act === "ruler-del" && ctxRuler) ruler.deleteRuler(ctxRuler.rid);
  if (act === "ruler-all") ruler.deleteAll();
  if (act === "bar-pt" && ctxBarrier) barriers.deletePoint(ctxBarrier.id, ctxBarrier.i);
  if (act === "bar-del" && ctxBarrier) barriers.remove(ctxBarrier.id);
  if (act === "bar-toggle" && ctxBarrier) barriers.toggle(ctxBarrier.id);
  ctx.hidden = true;
});
map.on("click", () => { ctx.hidden = true; });
map.on("movestart", () => { ctx.hidden = true; });
window.addEventListener("keydown", (e) => { if (e.key === "Escape") ctx.hidden = true; });

const damping = createDamping((g) => Number(settings[{ motion: "dampMotion", wind: "dampWind", depth: "dampDepth" }[g]]) || 0);
listen("vessel", ({ payload: raw }) => {
  if (!skConnected) skConnected = true; // data proves the link is up
  const fresh = !lastData || Date.now() - lastData >= SK_STALE_MS;
  lastData = Date.now();
  lastRaw = raw;
  if (fresh) showStatus();
  if (!live) return; // planning: leave the boat where it is
  const v = damping.push(raw);
  vessel = v;
  if (v.lat == null || v.lon == null) { feed(); return; }

  const pos = [v.lon, v.lat];
  lastBoat = pos;
  const dir = v.heading ?? v.cog ?? 0;
  setData("boat", { type: "FeatureCollection", features: [{ type: "Feature", properties: { rot: dir }, geometry: { type: "Point", coordinates: pos } }] });

  drawLines(v);

  const last = track.at(-1);
  if (!last || Math.abs(last[0] - pos[0]) > 1e-5 || Math.abs(last[1] - pos[1]) > 1e-5) {
    track.push(pos);
    if (track.length > TRACK_MAX) track.shift();
    setData("track", line(track));
  }
  if (follow) map.easeTo({ center: pos, duration: 200 });
  arrows?.live(pos, v);
  route?.boatAt(pos);
  feed();
});

// ---------- heading and COG lines ----------------------------------------------
// Each line runs from the boat along its bearing, either for a set time at the
// boat's speed (heading: speed through the water, else SOG; COG: SOG) or to the
// edge of the chart. Drawn as rhumb lines, so they are straight on the chart.
let lineBoat = null;
function lineLength(len, speed) {
  if (len === "inf") {
    const b = map.getBounds(), c = [lineBoat.lon, lineBoat.lat];
    const far = Math.max(...[[b.getWest(), b.getNorth()], [b.getEast(), b.getNorth()], [b.getWest(), b.getSouth()], [b.getEast(), b.getSouth()]].map((p) => gcNm(c, p)));
    return Math.min(far * 1.3 + 1, 12000);
  }
  return speed != null && speed > 0.2 ? speed * (Number(len) / 60) : 0;
}
function drawLines(v) {
  lineBoat = v && v.lat != null && v.lon != null ? v : null;
  const one = (show, brg, len, speed) => {
    if (!lineBoat || !show || brg == null) return line([]);
    const d = lineLength(len, speed);
    return d > 0 ? line([[lineBoat.lon, lineBoat.lat], rhumbDestination(lineBoat.lat, lineBoat.lon, brg, d)]) : line([]);
  };
  setData("hdgline", one(settings.hdgLine, v?.heading, settings.hdgLen, v?.stw ?? v?.sog));
  setData("cogline", one(settings.cogLine, v?.cog, settings.cogLen, v?.sog));
}
map.on("moveend", () => { if (lineBoat && (settings.hdgLen === "inf" || settings.cogLen === "inf")) drawLines(lineBoat); });

const LEN_OPTIONS = [["inf", "To the edge of the chart"], ["1", "1 min"], ["2", "2 min"], ["5", "5 min"], ["10", "10 min"], ["15", "15 min"],
  ["20", "20 min"], ["30", "30 min"], ["60", "1 h"], ["120", "2 h"], ["180", "3 h"], ["360", "6 h"], ["720", "12 h"], ["1440", "24 h"], ["2880", "48 h"]];
const DAMP_OPTIONS = [["0", "Off"], ["1", "1 s"], ["2", "2 s"], ["3", "3 s"], ["5", "5 s"], ["10", "10 s"], ["15", "15 s"], ["30", "30 s"], ["60", "60 s"]];
for (const [id, key, opts] of [["set-hdg-len", "hdgLen", LEN_OPTIONS], ["set-cog-len", "cogLen", LEN_OPTIONS],
  ["set-damp-motion", "dampMotion", DAMP_OPTIONS], ["set-damp-wind", "dampWind", DAMP_OPTIONS], ["set-damp-depth", "dampDepth", DAMP_OPTIONS]]) {
  const el = document.getElementById(id);
  for (const [v, label] of opts) el.append(new Option(label, v));
  el.value = String(settings[key]);
  el.onchange = () => { settings[key] = key.startsWith("damp") ? Number(el.value) : el.value; saveSettings(); drawLines(lineBoat); };
}
for (const [id, key] of [["set-hdg-line", "hdgLine"], ["set-cog-line", "cogLine"]]) {
  const el = document.getElementById(id);
  el.checked = !!settings[key];
  el.onchange = () => { settings[key] = el.checked; saveSettings(); drawLines(lineBoat); };
}

// Centre on the boat (rail button).
document.getElementById("centre-boat").onclick = () => {
  if (!lastBoat) return;
  map.easeTo({ center: lastBoat, duration: 400 });
};

// ---------- helpers ---------------------------------------------------------
function setData(id, data) { const s = map.getSource(id); if (s) s.setData(data); }
function emptyFC() { return { type: "FeatureCollection", features: [] }; }
function line(coords) { return { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: coords } }; }

/** Degrees and decimal minutes, e.g. 41°15.420'S. */
function ddm(deg, pos, neg) {
  if (pos === "E" && Number.isFinite(deg)) deg = ((((deg + 180) % 360) + 360) % 360) - 180; // map past ±180
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}

/** Rhumb-line destination: constant bearing, so a straight line on the chart. */
function rhumbDestination(lat, lon, brg, nm) {
  const R = 3440.065, d = nm / R, t = (brg * Math.PI) / 180;
  const p1 = (lat * Math.PI) / 180;
  let p2 = p1 + d * Math.cos(t);
  const lim = (85 * Math.PI) / 180;
  if (Math.abs(p2) > lim) { // stop at the edge of the Mercator chart
    const f = (Math.sign(p2) * lim - p1) / (p2 - p1 || 1);
    p2 = Math.sign(p2) * lim;
    return rhumbDestination(lat, lon, brg, nm * f);
  }
  const dpsi = Math.log(Math.tan(Math.PI / 4 + p2 / 2) / Math.tan(Math.PI / 4 + p1 / 2));
  const q = Math.abs(dpsi) > 1e-12 ? (p2 - p1) / dpsi : Math.cos(p1);
  const dl = (d * Math.sin(t)) / q;
  return [lon + (dl * 180) / Math.PI, (p2 * 180) / Math.PI];
}
function gcNm(a, b) {
  const r = Math.PI / 180, [l1, p1] = [a[0] * r, a[1] * r], [l2, p2] = [b[0] * r, b[1] * r];
  const h = Math.sin((p2 - p1) / 2) ** 2 + Math.cos(p1) * Math.cos(p2) * Math.sin((l2 - l1) / 2) ** 2;
  return 2 * Math.asin(Math.min(1, Math.sqrt(h))) * 3440.065;
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
