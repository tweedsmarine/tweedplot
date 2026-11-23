// racenav front end: offline chart, own boat, track and instrument panel.
import { placing } from "./modes.js";
import "./clock24.js";
import * as maplibregl from "maplibre-gl";
import "@fontsource/atkinson-hyperlegible-next/400.css";
import "@fontsource/atkinson-hyperlegible-next/600.css";
import "@fontsource/atkinson-hyperlegible-next/700.css";
// Bundle MapLibre's web worker with the app so the map works offline.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
maplibregl.setWorkerUrl(workerUrl);
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PRIMARY } from "./role.js";
import * as activity from "./activity.js";
import { setupWeather, barbImage } from "./weather.js";
import { setupWeatherPanel } from "./wxpanel.js";
import { setupScreenshots } from "./screenshot.js";
import { setupBrand } from "./brand.js";
import { setupUpdates } from "./updates.js";
import { setupWebMap } from "./webmap.js";
import { setupImages } from "./images.js";
import { setupLayerVisibility } from "./layervis.js";
import { setupChartFeatures, featureImage, loadS52Palette, s52PxPerMm } from "./chartfeatures.js";
import { setupTides, tideImage } from "./tides.js";
import { tidalImage } from "./tidalstreams.js";
import { setupTracks } from "./tracks.js";
import { setupTrips } from "./trips.js";
import { setupOverlays } from "./overlays.js";
import { setupEvents } from "./events.js";
let tripsTimer = 0;
// the Library's one-line messages go on activity cards (seen with the drawer closed)
const libNote = (s) => import("./activity.js").then((a) => a.note(s));
import { setupRuler } from "./ruler.js";
import { rhumb, bowOf } from "./navmath.js";
import { magneticVariation } from "./wmm.js";
import { setupGribTools } from "./gribtools.js";
import { setupRouteWeather } from "./routewx.js";
import { setupRadar } from "./radar.js";
import { setupWindArrows } from "./windarrows.js";
import { poiImage } from "./poi.js";
import { symImage } from "./marksymbols.js";
import { setupBarriers } from "./barriers.js";
import { setupAnchor } from "./anchor.js";
import { setupAntennas } from "./antennas.js";
import { setupLayers } from "./layers.js";
import { setupLaylines } from "./laylines.js";
import { setupVarDiagram } from "./varrose.js";
import { setupRoute, markImage } from "./route.js";
import { setupMariner, marinerImage, tokenColour } from "./mariner.js";
import { setupRouting } from "./routing.js";
import { setupBoat } from "./boat.js";
import { setupDock } from "./dock.js";
import { setupDepthStack } from "./depthstack.js";
import { createDamping } from "./damping.js";
import { setupFleet } from "./fleet.js";
import { setupGribDownload } from "./gribdl.js";
import { setupRail } from "./ui.js";
import { setupViews } from "./views.js";
import { setupKeys } from "./keys.js";
import { setupUse } from "./use.js";
import { setupCards } from "./cards.js";
import { setupHints } from "./hints.js";
import { setupWelcome } from "./welcome.js";
import { setupDisclaimer } from "./disclaimer.js";
import { setupMonitors } from "./monitors.js";
import { setupWindowState } from "./windowstate.js";
import { setupUiPrefs } from "./uiprefs.js";
import { setupTooltips } from "./tooltips.js";
import { setupSelectPop } from "./selectpop.js";
import { setupManual } from "./manual.js";
import { setupData } from "./data.js";
import { setupPbathy, pbathyAt } from "./pbathy.js";
import { setupSurveyDepths, surveyDepthAt } from "./surveydepths.js";
import { setupS63 } from "./s63.js";
import { setupPublications } from "./publications.js";
import { setupReplay } from "./replay.js";
import { setupPalette } from "./palette.js";
import { setupMapPalette } from "./mappalette.js";
import { setupAlarms } from "./alarms.js";
import { createPositionWatch } from "./position-watch.js";
import { createDepthWatch, depthFresh } from "./depth-watch.js";
import { setupMarine } from "./marine.js";
import { setupAis } from "./ais.js";
import { hullRing, bigEnough } from "./hull.js";
import { setupStations, stationImage } from "./stations.js";
import { setupInstruments, waypointData, targetAt } from "./instruments.js";
import { setupLibrary, labelImage } from "./library.js";
import { setupSettings } from "./settings.js";
import { setupSidePanel } from "./sidepanel.js";
import { setupHistorySettings, restoreHistoryCards, recorder as histRecorder } from "./history.js";
import { setupNmeaMonitor, openMonitorWindow } from "./nmeamonitor.js";

/** A "1"/"0" flag from localStorage; false where storage is unavailable. */
const storedFlag = (key) => { try { return localStorage.getItem(key) === "1"; } catch { return false; } };
const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

// ---------- chart settings (kept per viewer in local storage) ----------------
const DEFAULTS = {
  shallow: 2, safetyDepth: 3, safetyContour: 5, deep: 10,
  // S-52 mariner option: the isolated danger symbol for dangers in water shallower than the
  // safety contour too (the IHO catalogue's default is on; off keeps the chart clear)
  shallowDangers: false,
  // S-52 mariner option: light sector limits drawn to the light's nominal range (else 25 mm)
  fullLightLines: false,
  // depth shades: four (TweedPlot's usual: shallow contour, safety contour, deep contour) or
  // two (S-52: shallower or deeper than the safety contour); the shallow water pattern
  fourShades: true, shallowPattern: false, contourLabels: false,
  // own-boat lines: shown, and length in minutes of travel or "inf" (to the edge of the chart)
  hdgLine: true, hdgLen: "inf", cogLine: true, cogLen: "10",
  // beam bearing line through own ship; range rings round own ship (count, spacing NM)
  beamLine: false, ownRings: false, ownRingN: 3, ownRingStep: 0.5,
  // damping of Signal K readings, seconds (0 = none)
  dampMotion: 2, dampWind: 5, dampDepth: 0,
};
let settings = { ...DEFAULTS };
try { Object.assign(settings, JSON.parse(localStorage.getItem("racenav.chart") || "{}")); } catch { /* storage unavailable */ }
function saveSettings() { try { localStorage.setItem("racenav.chart", JSON.stringify(settings)); } catch { /* ignore */ } }

// Chart tiles come from the Rust chart store through the app's own "chart" protocol.
// The tiles change when the chart data or the drawing code does, so they must never
// come from the WebView's HTTP cache: a new query each start (and no-store from Rust).
const TILE_URL = window.RACENAV_TILE_URL
  ?? `${navigator.userAgent.includes("Windows") ? "http://chart.localhost" : "chart://localhost"}/{z}/{x}/{y}.pbf?s=${Date.now().toString(36)}`;

// S-52 style depth shades (day palette).
const DEPTH_DAY = { drying: "#a9c8a0", vshallow: "#7fb8e8", mshallow: "#a6cdf0", mdeep: "#c9e1ee", deep: "#e9f3f6" };
/** The IHO palette's colours (token → colour) while the chart is drawn in it (IHO symbols
 *  on: day, dusk or night), else null: TweedPlot's own day colours. */
let ihoColours = null;
const iho = (token, fallback) => ihoColours?.[token] ?? fallback;
/** The chart's base colours: the IHO palette's (S-52 tokens), or TweedPlot's. */
function chartColours() {
  return {
    drying: iho("DEPIT", DEPTH_DAY.drying), vshallow: iho("DEPVS", DEPTH_DAY.vshallow), mshallow: iho("DEPMS", DEPTH_DAY.mshallow),
    mdeep: iho("DEPMD", DEPTH_DAY.mdeep), deep: iho("DEPDW", DEPTH_DAY.deep), nodata: iho("NODTA", "#93aebb"),
    land: iho("LANDA", css("--land")), coast: iho("CSTLN", css("--coast")), sea: iho("DEPDW", css("--sea")),
    contour: iho("DEPCN", "#7d97a6"), safety: iho("DEPSC", "#1d2a33"), grey: iho("CHGRD", "#5b6b75"),
    soundShallow: iho("SNDG2", "#1d2a33"), soundDeep: iho("SNDG1", "#7d8f99"),
  };
}
function depareColour() {
  const DEPTH = chartColours();
  const d1 = ["coalesce", ["get", "drval1"], 0];
  return ["case",
    // the chart's own land (under its depth areas: shows where Topo50 has water instead)
    ["has", "land"], DEPTH.land,
    // unsurveyed (S-52 NODTA, as the IHO colour profile has it; patterned in "unsare")
    ["has", "uns"], DEPTH.nodata,
    // (S-52 colours lakes and rivers as very shallow water, DEPVS; TweedPlot as deep)
    ["has", "lake"], ihoColours ? DEPTH.vshallow : DEPTH.deep,
    // inside a chart's coverage where it charts no depth (chart.rs: "gap"): shallow, to be
    // safe. (S-52 grey no-data read as land where port basins and channels are gaps in
    // LINZ's charts, so it waits for the S-52 engine's no-data pattern.)
    ["has", "gap"], DEPTH.vshallow,
    ["<", d1, 0], DEPTH.drying,
    // two shades (S-52 mariner option): shallower or deeper than the safety contour
    ...(settings.fourShades === false ? [["<", d1, settings.safetyContour], DEPTH.vshallow] : [
      ["<", d1, settings.shallow], DEPTH.vshallow,
      ["<", d1, settings.safetyContour], DEPTH.mshallow,
      ["<", d1, settings.deep], DEPTH.mdeep]),
    DEPTH.deep];
}
/** The S-52 shallow water pattern (DIAMOND1) over depth areas shallower than the safety
 *  contour, when chosen (for screens where the shades are hard to tell apart). Drawn as
 *  each depth area's own fill (an image of its colour, with the diamonds where shallow),
 *  so a finer chart's deep area covers a coarser chart's shallow one, as with the colours;
 *  null: plain colours. */
function depareFillPattern() {
  if (!settings.shallowPattern) return null;
  const shallow = ["all", ["!", ["has", "land"]], ["!", ["has", "uns"]], ["!", ["has", "lake"]],
    ["<", ["coalesce", ["get", "drval1"], 0], settings.safetyContour]];
  return ["concat", "dpat:", ["case", shallow, "1", "0"], ":", depareColour()];
}
/** The image for "dpat:<1 or 0>:<colour>": the colour, with the shallow water pattern on it. */
function depthPatternImage(id) {
  const [, diamonds, colour] = id.split(":");
  const pat = diamonds === "1" ? featureImage("s52f:DIAMOND1") : null;
  if (diamonds === "1" && !pat) return null; // the IHO symbols aren't loaded yet
  const w = pat?.width ?? 8, h = pat?.height ?? 8;
  const c = document.createElement("canvas");
  c.width = w; c.height = h;
  const g = c.getContext("2d");
  g.fillStyle = colour;
  g.fillRect(0, 0, w, h);
  if (pat) {
    const o = document.createElement("canvas");
    o.width = w; o.height = h;
    o.getContext("2d").putImageData(pat instanceof ImageData ? pat : new ImageData(new Uint8ClampedArray(pat.data), w, h), 0, 0);
    g.drawImage(o, 0, 0);
  }
  return g.getImageData(0, 0, w, h);
}
/** The safety contour (S-52 DEPCNT02): the contour between a depth area shallower than the
 *  chosen value and one at least as deep, so the next deeper contour the chart has when it
 *  has none at exactly that depth (`sh` from the chart store: the shallower area's DRVAL1). */
function safetyContourFilter() {
  const sc = settings.safetyContour;
  return ["all", ["<=", sc, ["get", "valdco"]],
    ["case", ["has", "sh"], ["<", ["get", "sh"], sc], ["==", ["get", "valdco"], sc]]];
}
function soundingImage() {
  return ["concat", ["case", ["<=", ["get", "depth"], settings.safetyDepth], "sh:", "dp:"], ["to-string", ["get", "depth"]]];
}

// ---------- chart ----------------------------------------------------------
const defaultCamera = { center: [173.27, -41.26], zoom: 8 };
let startupCamera = { ...defaultCamera };
const map = new maplibregl.Map({
  container: "map",
  ...defaultCamera,
  attributionControl: false,
  style: {
    version: 8,
    sources: {
      "land-world": { type: "geojson", data: "data/land-world.geojson" },
      "land-nz": { type: "geojson", data: "data/land-nz.geojson" },
      graticule: { type: "geojson", data: graticule(1) },
      track: { type: "geojson", data: line([]) },
      boat: { type: "geojson", data: emptyFC() },
      "boat-hull": { type: "geojson", data: emptyFC() },
      hdgline: { type: "geojson", data: line([]) },
      cogline: { type: "geojson", data: line([]) },
      chart: { type: "vector", tiles: [TILE_URL], minzoom: 0, maxzoom: 16 },
      // high-resolution depths (LINZ coastal elevation, 1 m), drawn in Rust as images
      hires: { type: "raster", tiles: [TILE_URL.replace("{z}/{x}/{y}.pbf", "hires/{z}/{x}/{y}.png")], tileSize: 256, minzoom: 10, maxzoom: 18 },
      depthpack: { type: "raster", tiles: [TILE_URL.replace("{z}/{x}/{y}.pbf", "depthpack/{z}/{x}/{y}.png")], tileSize: 256, minzoom: 8, maxzoom: 19 },
      // NIWA 2016 bathymetry grid (250 m), imported in Chart data and drawn in Rust as images
      nzbathy: { type: "raster", tiles: [TILE_URL.replace("{z}/{x}/{y}.pbf", "nzbathy/{z}/{x}/{y}.png")], tileSize: 256, minzoom: 3, maxzoom: 14 },
      // own soundings (from tracks, TimeZero PBG and XYZ imports), drawn in Rust as images
      pbathy: { type: "raster", tiles: [TILE_URL.replace("{z}/{x}/{y}.pbf", "pbathy/{z}/{x}/{y}.png")], tileSize: 256, minzoom: 3, maxzoom: 20 },
      // online maps: off unless turned on in Chart › Layers (webmap.js). Fetched by Rust, which sends nothing while off
      osm: { type: "raster", tiles: [TILE_URL.replace("{z}/{x}/{y}.pbf", "osm/{z}/{x}/{y}.png")], tileSize: 256, maxzoom: 19 },
      osea: { type: "raster", tiles: [TILE_URL.replace("{z}/{x}/{y}.pbf", "osea/{z}/{x}/{y}.png")], tileSize: 256, maxzoom: 18 },
    },
    layers: [
      { id: "sea", type: "background", paint: { "background-color": css("--sea") } },
      { id: "grid", type: "line", source: "graticule", minzoom: 5,
        paint: { "line-color": "#9db8c8", "line-width": 0.5 } },
      // Depth fills sit below weather shading; land fills are added above it below.
      { id: "wdepth", type: "fill", source: "chart", "source-layer": "wdepth",
        layout: { visibility: "none", "fill-sort-key": ["+", ["*", ["get", "src"], 100000], ["get", "d"]] },
        paint: { "fill-color": ["step", ["get", "d"], "#a6cdf0", 10, "#b7d8f2", 20, "#c9e1ee", 50, "#dcecf3", 200, "#e4f0f5",
          1000, "#dbeaf2", 2000, "#d3e5ef", 3000, "#cbe0ec", 4000, "#c3dbe9", 5000, "#bbd6e6", 6000, "#b3d1e3"], "fill-antialias": false } },
      { id: "wdcnt", type: "line", source: "chart", "source-layer": "wdcnt", minzoom: 7, layout: { visibility: "none" },
        paint: { "line-color": "#8fa9b8", "line-width": 0.6, "line-opacity": 0.8 } },
      // LINZ hydrographic chart data (not for navigation)
      { id: "depare", type: "fill", source: "chart", "source-layer": "depare",
        layout: { visibility: "none", "fill-sort-key": ["coalesce", ["get", "sk"], ["get", "band"]] },
        paint: { "fill-color": depareColour(), "fill-antialias": false } },
      // NIWA grid (250 m, not chart datum, not for navigation): the seabed's shape at ocean and
      // coastal scales. It fades out from z10 and is gone by z12, so at harbour scales it never
      // hides the chart's own depth shading; contours and the safety contour stay on top
      { id: "nzbathy", type: "raster", source: "nzbathy", minzoom: 3, maxzoom: 12, layout: { visibility: storedFlag("racenav.nzbathy.show") ? "visible" : "none" },
        paint: { "raster-opacity": ["interpolate", ["linear"], ["zoom"], 9.5, 0.9, 11.5, 0], "raster-resampling": "linear" } },
      // over the charted depth areas, under the chart's contours (the safety contour stays on top)
      { id: "hires", type: "raster", source: "hires", minzoom: 10, layout: { visibility: localStorage.getItem("racenav.hires.show") === "0" ? "none" : "visible" },
        paint: { "raster-opacity": 0.92, "raster-resampling": "linear" } },
      // LINZ's survey depths (multibeam and lidar, approximate LAT; surveydepths.js): over the
      // coastal elevation where both have data, under the chart's contours
      { id: "depthpack", type: "raster", source: "depthpack", minzoom: 8, layout: { visibility: localStorage.getItem("racenav.surveydepths.show") === "0" ? "none" : "visible" },
        paint: { "raster-opacity": 0.92, "raster-resampling": "linear" } },
      // over the charted depths too: it's what the skipper surveyed (not for navigation)
      { id: "pbathy", type: "raster", source: "pbathy", minzoom: 3, layout: { visibility: "none" }, // per view: pbathy.js
        paint: { "raster-opacity": 0.95, "raster-resampling": "nearest" } },
      // OpenStreetMap (optional): its sea is made transparent in Rust, so it draws only land over the depth colours
      { id: "osm", type: "raster", source: "osm", layout: { visibility: "none" } },
      { id: "depcnt", type: "line", source: "chart", "source-layer": "depcnt", layout: { visibility: "none" },
        paint: { "line-color": "#7d97a6", "line-width": 0.6 } },
      // unsurveyed areas: the S-52 no-data pattern (NODATA03) and a grey outline
      { id: "unsare", type: "fill", source: "chart", "source-layer": "unsare", layout: { visibility: "none" },
        paint: { "fill-pattern": "pat:nodata" } },
      { id: "unsare-line", type: "line", source: "chart", "source-layer": "unsare", layout: { visibility: "none" },
        paint: { "line-color": "#7a8a91", "line-width": 1.2 } },
      // dredged areas (channels, berths) outlined, as S-52 draws DRGARE: grey dashes
      { id: "depare-drg", type: "line", source: "chart", "source-layer": "depare", filter: ["has", "dredged"], layout: { visibility: "none" },
        paint: { "line-color": "#5b6b75", "line-width": 1.2, "line-dasharray": [4, 3] } },
      { id: "safety-contour", type: "line", source: "chart", "source-layer": "depcnt",
        filter: safetyContourFilter(), layout: { visibility: "none" },
        paint: { "line-color": "#1d2a33", "line-width": 1.6 } },
      // contour labels (S-52 mariner's option): each contour's depth along it, upright
      { id: "depcnt-lbl", type: "symbol", source: "chart", "source-layer": "depcnt", filter: contourLabelFilter(),
        layout: { visibility: "none", "symbol-placement": "line", "symbol-spacing": 220, "icon-rotation-alignment": "viewport",
                  "icon-image": ["concat", "cl:", ["to-string", ["get", "valdco"]]], "icon-allow-overlap": false, "icon-padding": 1 } },
      // Bundled land: shown until chart tiles arrive (setChartFallbackRange).
      { id: "land-world", type: "fill", source: "land-world", maxzoom: 6,
        paint: { "fill-color": css("--land") } },
      { id: "land-nz", type: "fill", source: "land-nz", minzoom: 4,
        paint: { "fill-color": css("--land") } },
      { id: "chart-world-land", type: "fill", source: "chart", "source-layer": "wland", layout: { visibility: "none" },
        paint: { "fill-color": css("--land"), "fill-antialias": false } },
      // Fallback coastlines stay underneath chart land to avoid duplicate outlines;
      // chart coastlines are drawn over the chart land below.
      { id: "coast-world", type: "line", source: "land-world", maxzoom: 6,
        paint: { "line-color": css("--coast"), "line-width": 0.8 } },
      { id: "coast-nz", type: "line", source: "land-nz", minzoom: 4,
        paint: { "line-color": css("--coast"), "line-width": 0.8 } },
      { id: "chart-land", type: "fill", source: "chart", "source-layer": "land", layout: { visibility: "none" },
        // no antialiasing: world land comes in pieces whose shared edges would show as faint lines
        paint: { "fill-color": css("--land"), "fill-antialias": false } },
      { id: "chart-coast", type: "line", source: "chart", "source-layer": "land", layout: { visibility: "none" }, filter: ["all", ["!", ["has", "w"]], ["!", ["has", "enc"]]],
        paint: { "line-color": css("--coast"), "line-width": 0.9 } },
      { id: "chart-wcoast", type: "line", source: "chart", "source-layer": "wcoast", layout: { visibility: "none" },
        paint: { "line-color": css("--coast"), "line-width": 0.9 } },
      { id: "soundg", type: "symbol", source: "chart", "source-layer": "soundg", layout: { visibility: "none", "icon-image": soundingImage(), "icon-allow-overlap": false, "icon-padding": 1 },
      },
      { id: "osea", type: "raster", source: "osea", layout: { visibility: "none" } },
      { id: "track", type: "line", source: "track",
        paint: { "line-color": "#d35400", "line-width": 2 } },
      { id: "hdgline", type: "line", source: "hdgline",
        paint: { "line-color": "#1d2a33", "line-width": 1.6 } },
      { id: "cogline", type: "line", source: "cogline",
        paint: { "line-color": "#d35400", "line-width": 2, "line-dasharray": [3, 2] } },
      // own boat to scale (Settings: length, breadth, GPS antenna) at harbour scales
      { id: "boat-hull", type: "fill", source: "boat-hull", filter: ["==", ["get", "k"], "hull"],
        paint: { "fill-color": "#d00000", "fill-opacity": 0.35 } },
      { id: "boat-hull-line", type: "line", source: "boat-hull", filter: ["==", ["get", "k"], "hull"],
        paint: { "line-color": "#8b0000", "line-width": 1.6 } },
      { id: "boat-gps", type: "circle", source: "boat-hull", filter: ["==", ["get", "k"], "gps"],
        paint: { "circle-radius": 3, "circle-color": "#fff", "circle-stroke-color": "#8b0000", "circle-stroke-width": 1.5 } },
      { id: "boat", type: "symbol", source: "boat", filter: ["!=", ["get", "big"], true],
        layout: { "icon-image": "boat", "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map",
                  "icon-allow-overlap": true, "icon-ignore-placement": true } },
    ],
  },
});
// IHO dusk and night: everything on the map that isn't chart is darkened to suit
const mapPalette = setupMapPalette(map);
/** Chart layers coloured from the IHO palette here (left alone by mapPalette). */
const IHO_CHART_LAYERS = new Set(["sea", "depare", "depcnt", "unsare", "unsare-line", "depare-drg", "safety-contour", "land-world", "land-nz",
  "chart-world-land", "coast-world", "coast-nz", "chart-land", "chart-coast", "chart-wcoast", "soundg",
  // own ship, past track and route: in the IHO mariner colours with the IHO symbols (mariner.js)
  "track", "hdgline", "boat-hull", "boat-hull-line", "boat-gps", "rt-legs"]);
/** The chart's base layers in the colours of the moment (chartColours). */
function applyChartColours() {
  const c = chartColours();
  const set = (id, prop, v) => { if (map.getLayer(id)) map.setPaintProperty(id, prop, v); };
  set("sea", "background-color", c.sea);
  for (const id of ["land-world", "land-nz", "chart-world-land", "chart-land"]) set(id, "fill-color", c.land);
  for (const id of ["coast-world", "coast-nz", "chart-coast", "chart-wcoast"]) set(id, "line-color", c.coast);
  set("depare", "fill-color", depareColour());
  set("depare", "fill-pattern", depareFillPattern());
  set("depcnt", "line-color", c.contour);
  set("safety-contour", "line-color", c.safety);
  set("unsare-line", "line-color", c.grey);
  set("depare-drg", "line-color", c.grey);
  set("unsare", "fill-pattern", ihoColours ? "s52f:NODATA03" : "pat:nodata");
  // soundings and contour labels are drawn in the palette's colours: redrawn in place
  for (const id of map.listImages()) {
    const m = /^(sh|dp):(-?[0-9.]+)$/.exec(id);
    if (m) map.updateImage(id, soundingIcon(parseFloat(m[2]), m[1] === "sh"));
    const l = /^cl:(-?[0-9.]+)$/.exec(id);
    if (l) { const im = contourLabelIcon(parseFloat(l[1])); const old = map.getImage(id); if (old && old.data.width === im.width && old.data.height === im.height) map.updateImage(id, im); else map.removeImage(id); }
  }
}
/** Day, dusk, night (IHO) with the IHO symbols: the IHO palette for the chart, the rest
 *  darkened to suit. Otherwise TweedPlot's colours (and the page's filter, style.css). */
let paletteMode = "day";
let paletteBusy = Promise.resolve();
function applyPalette() {
  paletteBusy = paletteBusy.then(async () => {
    const on = !!features?.usingS52();
    const palette = on ? ({ dusk: "Dusk", "iho-night": "Night" }[paletteMode] ?? "Day") : "Day";
    // everything loaded first (once per palette), then the whole switch in one go
    let bundle = null;
    try { if (on) bundle = await loadS52Palette(palette); } catch (e) { console.warn("IHO palette:", e); }
    const live = on && !!bundle;
    features?.usePalette(bundle);
    ihoColours = live ? bundle.colours : null;
    document.documentElement.toggleAttribute("data-iho", live);
    applyChartColours();
    mapPalette.apply(live ? palette : null, (id) => IHO_CHART_LAYERS.has(id) || id.startsWith("s52-"));
    // own ship, track, route, rings and EBLs in the palette's mariner colours (or TweedPlot's)
    route?.setIho(live, tokenColour);
    mariner?.restyle();
  });
  return paletteBusy;
}

let hasPositionFix = false;
let chartPositionChosen = false;
let initialPositionChosen = false;
for (const eventName of ["pointerdown", "wheel", "keydown"]) {
  map.getCanvasContainer().addEventListener(eventName, () => { chartPositionChosen = true; }, { passive: true });
}
function useReferencePosition([lon, lat]) {
  if (!hasPositionFix && !chartPositionChosen) {
    startupCamera = { center: [lon, lat], zoom: defaultCamera.zoom };
    map.setCenter([lon, lat]);
  }
}
function useInitialPosition([lon, lat]) {
  if (initialPositionChosen || chartPositionChosen) return;
  initialPositionChosen = true;
  startupCamera = { center: [lon, lat], zoom: 12 };
  map.jumpTo(startupCamera);
}
window.__map = map; // handy for debugging in devtools
// (no map scale at the bottom right: the S-52 scale bar or latitude scale at the left edge, mariner.js, is the scale)
setupUiPrefs();
setupTooltips();
setupSelectPop();
document.getElementById("zoom-in").onclick = () => map.zoomIn();
document.getElementById("zoom-out").onclick = () => map.zoomOut();
const northNeedle = document.getElementById("north-needle");
document.getElementById("north-btn").onclick = () => map.easeTo({ bearing: 0, pitch: 0 });
// CSS rotation about the needle's centre (style.css): the SVG transform attribute's own
// centre would be added to the CSS transform-origin, swinging it out of the button
map.on("rotate", () => { northNeedle.style.transform = `rotate(${-map.getBearing()}deg)`; });
map.on("load", () => map.addImage("boat", boatIcon(), { pixelRatio: 2 }));
map.on("error", (e) => {
  if (e.sourceId === "chart") retryFailedTiles();
  else if (e.error) console.warn(e.error);
});
map.on("sourcedata", (e) => {
  if (e.sourceId !== "chart" || e.tile?.state !== "loaded") return;
  chartTileLoaded = true;
  clearTimeout(chartTileRetry);
  chartTileRetry = null;
  clearTimeout(tileRetry);
  tileRetry = null;
  chartTileRetryAttempt = 0;
  setChartFallbackRange(false);
  document.getElementById("startup-splash").hidden = true;
  finishStartupWhenReady();
});

// Sounding figures are drawn on demand: no font files needed offline.
map.setMissingStyleImageResolver((id) => {
  if (map.hasImage(id)) return;
  if (id === "pat:nodata") { map.addImage(id, noDataPattern(), { pixelRatio: 2 }); return; }
  if (id.startsWith("dpat:")) { const im = depthPatternImage(id); if (im) map.addImage(id, im, { pixelRatio: 2 }); return; }
  const m = /^(sh|dp):(-?[0-9.]+)$/.exec(id);
  if (m) { map.addImage(id, soundingIcon(parseFloat(m[2]), m[1] === "sh"), { pixelRatio: 2 }); return; }
  const cl = /^cl:(-?[0-9.]+)$/.exec(id);
  if (cl) { map.addImage(id, contourLabelIcon(parseFloat(cl[1])), { pixelRatio: 2 }); return; }
  const img = barbImage(id) || markImage(id) || marinerImage(id) || labelImage(id) || featureImage(id) || tideImage(id) || poiImage(id) || symImage(id) || stationImage(id) || tidalImage(id);
  if (img) map.addImage(id, img, { pixelRatio: 2 });
});

// Weather: GRIB wind shading sits under the land, barbs under the boat and track.
let weather = null;
let lastBoat = null; // [lon, lat] from Signal K, or placed by hand when planning
let route = null; // the course being edited
let routing = null;
let aisApi = null; // AIS targets (ais.js), for the data panel's Targets section
let library = null;
const ui = setupRail(map);
const views = setupViews(map, { livePosition: () => live && positionWatch.age() != null && !positionWatch.lost(), onPlanningTimeout: (view) => toast(`${view === "env" ? "Environment" : "Planning"} inactive: returned to Navigation`, 8000) });
setupCards();
const use = setupUse();

// The user manual (bundled with the app): rail button, F1, ? in each drawer.
const manual = setupManual({
  // the symbols in the manual come from the chart's own drawing code
  image: (id) => {
    const im = barbImage(id) || markImage(id) || marinerImage(id) || featureImage(id) || tideImage(id) || poiImage(id) || symImage(id) || stationImage(id) || tidalImage(id);
    if (im) return { data: im, pixelRatio: 2 };
    const m = map.hasImage(id) ? map.getImage(id) : null;
    return m && !m.sdf ? { data: m.data, pixelRatio: m.pixelRatio } : null;
  },
});
document.getElementById("manual-btn").onclick = () => manual.toggle();
manual.onToggle((on) => document.getElementById("manual-btn").classList.toggle("on", on));
document.getElementById("drawer-help").onclick = () => manual.help(ui.current());
const inst = setupInstruments();
setupHints();
setupWelcome({ use, inst });
const sidePanel = setupSidePanel({ inst, views, use, map,
  getAis: () => aisApi,
  // show a place (a mark, a target, the MOB): Follow off so the chart stays there
  centreOn: (ll) => { setFollow(false); map.easeTo({ center: ll, duration: 400 }); },
  centreBoat: () => {
    if (!lastBoat) { toast("No boat position yet", 2500); return; }
    setFollow(true);
    map.easeTo({ center: lastBoat, offset: followOffset(), duration: 300 });
  } });
// the Route drawer opened (rail, Edit the route, the library) in Planning: its planning controls are in the panel
ui.onChange((name) => { if (name === "course") sidePanel.revealPlan(); });
window.__inst = inst; window.__side = sidePanel; window.__ais = () => aisApi; window.__rec = histRecorder; window.__map = map; // debugging handles
setupDisclaimer();
window.addEventListener("racenav-mob", (e) => { inst.update({ mob: e.detail }); });
const boat = setupBoat();
const antennas = setupAntennas(); // which receiver's antenna the position is from
setupSettings({ boat, ui, onReferencePosition: useReferencePosition });
function openConnectionCard(id) {
  ui.openTab("settings", "conn");
  const card = document.getElementById(id);
  if (card) {
    card.open = true;
    card.scrollIntoView({ block: "start", behavior: "smooth" });
  }
}
const openNmeaSettings = () => openConnectionCard("nmea-0183-section");
if (PRIMARY) {
  listen("nmea-open-settings", () => {
    const w = getCurrentWindow();
    w.unminimize().then(() => w.setFocus()).catch(() => {});
    openNmeaSettings();
  });
}
setupHistorySettings(document.getElementById("hist-keep"), document.getElementById("hist-info"), document.getElementById("hist-clear"));
restoreHistoryCards();
document.getElementById("nmea-popout").addEventListener("click", () => openMonitorWindow((message) => toast(message, 6000)));
setupNmeaMonitor(document.getElementById("nmea-monitor-inline"), { openSettings: openNmeaSettings });
setupNmeaMonitor(document.getElementById("nmea-monitor-window"), { standalone: true, openSettings: openNmeaSettings });
const dock = setupDock(map);
let features = null;
let ruler = null;
let mariner = null; // own ship, vectors, range rings, EBL/VRM, scale bar (mariner.js)
let gribTools = null;
let depthStack = null;
let arrows = null;
let barriers = null;
let anchor = null;
let pbathy = null;
let layersPanel = null;
let laylines = null;
let varRoses = null;
let ctxRose = false; // right-click was on the variation diagram

// "Go to here": the target for laylines (and later the autopilot); else the route's next mark
let gotoTarget = null;
try { gotoTarget = JSON.parse(localStorage.getItem("racenav.goto") || "null"); } catch { /* none */ }
function setGoto(g) {
  // the leg runs from where the boat was when the go to was set (cross-track error is off that line)
  if (g && !g.from && lastBoat) g = { ...g, from: [...lastBoat] };
  gotoTarget = g;
  try { if (g) localStorage.setItem("racenav.goto", JSON.stringify(g)); else localStorage.removeItem("racenav.goto"); } catch { /* storage unavailable */ }
  laylines?.refresh();
  feed();
}
/** What is being navigated to: a "Go to" while one is set (it takes over from the route, which
 *  keeps its place and comes back when the go to is cancelled), else the route's next mark. Same
 *  shape as route.nav(): the instruments, the data panel, the highway and the alarms all use it. */
function activeNav() {
  if (gotoTarget) {
    return { next: { name: gotoTarget.name || "Go to", lon: gotoTarget.lon, lat: gotoTarget.lat, n: "→" },
      from: gotoTarget.from ?? lastBoat ?? null, index: 0, count: 1, ahead: [], goto: true };
  }
  return route?.nav() ?? null;
}
/** For the readings, the highway and the laylines: in Planning and Environment the route as planned
 *  stands in when nothing is being navigated; in Navigation and Instruments only what is navigated shows. */
function shownNav() {
  const nav = activeNav();
  if (nav) return nav;
  return ["plan", "env"].includes(document.documentElement.dataset.view) ? route?.nav(true) ?? null : null;
}
function layTarget() {
  const nav = shownNav();
  return nav ? { ...nav.next, from: nav.from, goto: !!nav.goto } : null;
}
let layKey = "";
/** True wind at the target: the live instruments when the boat is live, else the GRIB at the slider time. */
async function layWind(target) {
  if (live && signalKActive() && Number.isFinite(vessel.twd)) return { twd: vessel.twd, tws: vessel.tws };
  const path = weather?.current().path;
  if (!path) return null;
  const rows = await invoke("grib_point", { lat: target.lat, lon: target.lon, t: boatTime() }).catch(() => []);
  const r = (rows || []).find((x) => x.path === path);
  return r && r.twd != null ? { twd: r.twd, tws: r.tws } : null;
}
let tracks = null;

/** A short message at the bottom of the chart ("" hides it); gone after `ms` if given. */
let toastTimer = null;
function toast(text, ms = 0, isStartup = false) {
  const el = document.getElementById("toast");
  clearTimeout(toastTimer);
  el.classList.toggle("startup-toast", isStartup);
  el.textContent = text;
  el.hidden = !text;
  if (text && ms) toastTimer = setTimeout(() => { el.hidden = true; }, ms);
}
const startup = {
  mapLoaded: false, appReady: false, chartsReady: false, chartInfo: null,
  readyAt: 0, finishTimer: null, finished: false, message: "Initialising - please wait", progress: "",
};
let chartStoreReady = false;
let chartTileLoaded = false;
let chartTileRetry = null;
let chartTileRetryAttempt = 0;
let tileRetry = null, lastRetry = 0;
const CHART_TILE_RETRY_DELAYS = [750, 1500, 3000, 6000, 10000];
function chartLoadingMessage() {
  if (!startup.progress) return "Loading charts";
  const detail = startup.progress.replace(/^preparing charts:\s*/i, "").replace(/^converting ENC cells:\s*/i, "converting ENC cells: ");
  return `Loading charts - ${detail}`;
}
function startupToast(text) {
  if (startup.finished || startup.message === text) return;
  startup.message = text;
  toast(text, 0, true);
}
function finishStartupWhenReady() {
  if (startup.finished || !startup.mapLoaded || !startup.appReady || !startup.chartsReady) return;
  const info = startup.chartInfo;
  if (info.tables > 0 && (!chartTileLoaded || !map.isSourceLoaded("chart"))) return;
  const remaining = 900 - (Date.now() - startup.readyAt);
  if (remaining > 0) {
    if (!startup.finishTimer) {
      startup.finishTimer = setTimeout(() => {
        startup.finishTimer = null;
        finishStartupWhenReady();
      }, remaining);
    }
    return;
  }
  startup.finished = true;
  if (info.error) toast(`Chart loading failed: ${info.error}`, 8000, true);
  else if (info.tables > 0) toast("System ready", 3000, true);
  else toast("System ready - no charts found", 5000, true);
}
function noteChartsReady(info) {
  if (!info) return;
  if (info.ready === true && !info.error && info.tables > 0) chartStoreReady = true;
  chartsLoaded(info);
  if (!startup.chartsReady && (info.ready === true || info.error)) {
    startup.chartsReady = true;
    startup.chartInfo = info;
    startup.readyAt = Date.now();
    startupToast(info.error ? "Chart loading failed" : info.tables === 0 ? "Charts ready - no charts found" : "Charts ready - loading map tiles");
    if (!info.error && info.tables > 0) {
      chartStoreReady = true;
      chartTileLoaded = false;
      chartTileRetryAttempt = 0;
      clearTimeout(tileRetry);
      tileRetry = null;
    }
  }
  finishStartupWhenReady();
}
map.on("load", () => {
  features = setupChartFeatures(map, { before: "track", safetyContour: settings.safetyContour, shallow: !!settings.shallowDangers,
    onSymbols: () => applyPalette(),
    // a new screen calibration: the images drawn to IHO sizes again
    onScale: () => {
      for (const id of map.listImages()) if (/^(cl:|dpat:)/.test(id)) map.removeImage(id);
      window.dispatchEvent(new Event("racenav-screen-calibration"));
    } });
  weather = setupWeather(map, { landTop: "land-world", coastTop: "coast-world", top: "track" });
  window.__weather = weather; // debugging handle
  setupData(map, { toast, reloadTiles: reloadChartTiles });
  setupSurveyDepths(map, { toast });
  pbathy = setupPbathy(map, { ui });
  setupS63(map, { toast });
  setupPublications({ toast });
  setupPalette((mode) => { paletteMode = mode; applyPalette(); });
  setupBrand();
  // never installs while a live position is arriving (installing closes the app)
  setupUpdates();
  setupWebMap(map);
  setupImages(map, { tileUrl: TILE_URL, toast });
  layerVis = setupLayerVisibility(map, { ready: () => chartMapLoaded && chartStoreReady });
  alarms = setupAlarms(map, {
    getBoat: () => ({ pos: lastBoat, v: vessel, live: live && signalKActive(), plottingLive: live, positionLost: positionWatch.lost(),
      depthFresh: depthFresh(vessel, (field) => depthWatch.age(field), true), depthGraceElapsed: depthWatch.graceElapsed() }),
    getBow: () => { const o = antennas.offsets(boat.get()); return bowOf(lastBoat, vessel, o.fromBow, o.fromCentre)?.pos ?? null; },
    getSafetyDepth: () => settings.safetyDepth,
    getSafetyContour: () => settings.safetyContour,
    getAirDraft: () => Number(boat.get().airDraft) || 0,
    getNav: () => {
      const nav = activeNav();
      if (!nav || !lastBoat) return null;
      return { nav, wp: waypointData(lastBoat, vessel.sog, vessel.cog, nav.from, [nav.next.lon, nav.next.lat]), arrivalNm: Number(document.getElementById("rt-radius").value) || 0.1 };
    },
    onMobMarkDeleted: () => library?.refresh(),
    toast,
    // no alarm sounds until the screen is ready (alarms raised meanwhile sound then); after two
    // minutes they sound regardless, so a start-up that never finishes can't keep an alarm quiet
    soundReady: () => startup.finished || performance.now() > 120000,
  });
  for (const alarm of pendingNmeaAlarms.splice(0)) alarms.raise(...alarm);
  setupMarine(map, {
    getBoat: () => ({ pos: lastBoat, cog: vessel?.cog, sog: vessel?.sog, live: live && liveDataConnected() }),
    getLength: () => Number(boat.get().length) || 0,
    alarms,
    ui,
  });
  setupStations(map, { weather });
  // marks, routes and routings changed in any window (the backend announces every save and delete)
  let libraryTimer = null, tracksTimer = null;
  // the one-off move of a library kept as JSON files into its database (first start after the change)
  let convertJob = null;
  listen("library-converting", (e) => {
    const { step, done, total } = e.payload;
    convertJob ??= activity.start("Moving your library into its new database");
    convertJob.progress(step === "tracks" ? "Tracks" : "Marks, routes and barriers", done, step === "tracks" ? total : 0);
  });
  listen("library-converted", (e) => {
    const job = convertJob ?? activity.start("Moving your library into its new database");
    convertJob = null;
    if (e.payload.ok) job.done("Done.", { details: [e.payload.text], keep: true });
    else job.fail(`It failed: ${e.payload.text}`);
  });
  listen("library-changed", () => {
    clearTimeout(libraryTimer);
    libraryTimer = setTimeout(() => library?.refresh(), 150);
  });
  setupMonitors(toast);
  activity.setupActivityLog();
  activity.setupSystemInfo().catch(() => {});
  setupWindowState();
  aisApi = setupAis(map, { alarms, ui, getOwnMmsi: () => boat.get().mmsi, getOwn: () => ({ pos: lastBoat, cog: live ? vessel?.cog : null, sog: live ? vessel?.sog : 0, live: live && signalKActive() }) });
  setupReplay({ onProgress: ({ at, start }) => { positionWatch.replayProgress(at, start); depthWatch.progress(at, start); }, onState: (on) => {
    replayOn = on;
    positionWatch.reset();
    depthWatch.reset();
    if (on && !live) liveBtn.click(); // a replay plays live
    if (!on) { lastFeed = 0; }
    showStatus();
  } });
  library = setupLibrary(map, { ui, toast, getBoat: () => lastBoat });
  route = setupRoute(map, { library, getBoat: () => lastBoat, getArrivalNm: () => Number(document.getElementById("rt-radius").value) || 0.1 });
  // navigation started or stopped, or the view changed: the readings, highway and laylines follow at once
  route.onChange(() => { feed(); laylines?.refresh(); });
  window.addEventListener("racenav-view", () => { feed(); laylines?.refresh(); });
  mariner = setupMariner(map, { settings, saveSettings, colours: () => ihoColours, usingS52: () => features?.usingS52(), library, route, toast, getVariation: variationAt });
  route.setIho(!!ihoColours, tokenColour);
  if (lastBoat) drawBoat();
  mariner.track(track, trackT);
  routing = setupRouting(map, { weather, course: route, library, dock, ui, getBoat: () => lastBoat,
    getSafetyDepth: () => settings.safetyDepth, getAvoid: () => features.avoid(), getBoatSettings: boat.get });
  library.attach(route, routing);
  setupFleet(map, { weather, course: route, routing, dock, ui });
  setupGribDownload(map, { weather, course: route });
  gribTools = setupGribTools(map, { weather, dock });
  setupWeatherPanel(map, weather, sidePanel, ui);
  setupScreenshots();
  depthStack = setupDepthStack(map, { weather, dock });
  // the navigated route's weather runs at the speed over ground, while live data is arriving
  const routeWx = setupRouteWeather(map, { weather, routing, course: route, dock,
    getSog: () => (live && positionWatch.age() != null && !positionWatch.lost() && Number.isFinite(vessel.sog) ? vessel.sog : null) });
  setupRadar(map, { getBoat: () => lastBoat });
  for (const id of ["rt-wx", "crs-wx"]) { const b = document.getElementById(id); if (b) b.onclick = () => routeWx.open(); }
  document.getElementById("wx-verify").onclick = () => gribTools.openVerify();
  // the tide gauges (rising / falling) and tidal streams follow the chart time in Planning and
  // Environment, as the weather does; in Navigation and Instruments they show now (live)
  setupTides(map, { before: "track", getTime: tideTime, onTime: (fn) => { weather.onTime(fn); window.addEventListener("racenav-view", fn); } });
  ruler = setupRuler(map, { before: "track", ui, library, route, toast, getVariation: variationAt });
  tracks = setupTracks(map, { before: "track", getTime: boatTime, onTime: (fn) => weather.onTime(fn),
    status: (s) => { if (s) libNote(s); } });
  listen("library-changed", () => { clearTimeout(tracksTimer); tracksTimer = setTimeout(() => { tracks.refresh(); barriers?.refresh(); }, 150); });
  barriers = setupBarriers(map, { before: "track", library, toast, getVariation: variationAt });
  anchor = setupAnchor(map, { library, alarms, before: "track", toast,
    getBoat: () => ({ pos: lastBoat, v: vessel }), getVessel: () => boat.get(), getAntenna: () => antennas.offsets(boat.get()) });
  layersPanel = setupLayers({ ui });
  setupAnchorButton(anchor);
  varRoses = setupVarDiagram(map, { getVariation: variationAt,
    getAnnualChange: (p) => magneticVariation(p[1], p[0])?.annualChange ?? null });
  laylines = setupLaylines(map, { before: "track", getVariation: variationAt, getTarget: layTarget, getBoat: () => lastBoat, getWind: layWind,
    // not for fishing: no laylines or cone (a Go to target still shows)
    getSettings: () => (use.use() === "fish" ? { ...boat.get(), layShow: false, layConeShow: false } : boat.get()) });
  window.addEventListener("racenav-boat", () => laylines.refresh());
  // Weather routing says which boat it routes: polar and efficiency (Settings › Vessel)
  const rtBoat = () => {
    const b = boat.get();
    const polar = String(b.polar || "").split(/[\\/]/).pop();
    const el = document.getElementById("rt-polar");
    if (el) el.textContent = polar ? `Boat: ${polar}, ${b.eff || 100} %, tack ${b.tack ?? 60} s, gybe ${b.gybe ?? 60} s` : "Boat: no polar chosen yet";
  };
  window.addEventListener("racenav-boat", rtBoat);
  rtBoat();
  document.getElementById("rt-polar-change").onclick = (e) => {
    e.preventDefault(); ui.openTab("settings", "vessel");
    setTimeout(() => { const c = [...document.querySelectorAll("details.dcard")].find((d) => d.dataset.title === "Performance (polar)"); if (c) { c.open = true; c.scrollIntoView({ block: "start" }); } }, 60);
  };
  window.addEventListener("racenav-use", () => laylines.refresh());
  weather.onTime(() => laylines.refresh());
  setInterval(() => laylines.refresh(), 20000);
  const trips = setupTrips(map, { ui, library, tracks });
  setupOverlays(map, { library });
  setupEvents(map, { ui, getBoat: () => ({ pos: live ? lastBoat : null, v: vessel }) });
  listen("library-changed", () => { clearTimeout(tripsTimer); tripsTimer = setTimeout(() => trips.refresh(), 200); });
  arrows = setupWindArrows(map, { before: "boat" });
  const wa = document.getElementById("inst-arrows");
  wa.checked = arrows.enabled();
  wa.onchange = () => { arrows.setEnabled(wa.checked); if (live && lastBoat) arrows.live(lastBoat, vessel); };
  weather.onLoaded(() => inst.update({ seaLoaded: weather.hasSea() }));
  weather.onPoint((row, at) => {
    inst.update({ grib: row });
    if (cursorArrows()) planningArrows(row, at); // planning (the view, paused, or no live data): arrows on the cursor
  });
  library.onChange(feed);
  feed();
  startup.appReady = true;
  startupToast(startup.chartsReady ? "Charts ready - loading map tiles" : chartLoadingMessage());
  finishStartupWhenReady();
});

/** Planning: wind arrows at the cursor. True wind from the GRIB at the slider time; near
 * a shown routing, the true wind, heading and boat speed of its nearest point (at that
 * point's time) and the apparent wind from them. Without a routing there is no boat,
 * so no apparent wind. */
let arrowSeq = 0;
// The wind arrows are at the cursor when planning: with no live data, and in the Planning view
// even with live data (the forecast under the cursor is what that view is for). Elsewhere, and
// while the cursor is off the forecast, they are on the boat.
let arrowsAtCursor = false;
const cursorArrows = () => !signalKActive() || document.documentElement.dataset.view === "plan";
function boatArrows() {
  if (!arrows) return;
  if (live && lastBoat && signalKActive()) arrows.live(lastBoat, vessel); else arrows.clear();
}
window.addEventListener("racenav-view", () => { arrowsAtCursor = false; boatArrows(); });
async function planningArrows(row, at) {
  if (!arrows) return;
  if (!row || !at || row.tws == null || row.twd == null) { arrowsAtCursor = false; boatArrows(); return; }
  const seq = ++arrowSeq;
  let tw = { twd: row.twd, tws: row.tws };
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
    // on a routing, show the wind the routing had there (its time, not the slider's),
    // so the arrows agree with the point's popup
    if (best) {
      hdg = best.heading; bsp = best.bsp;
      if (Number.isFinite(best.twd) && Number.isFinite(best.tws)) tw = { twd: best.twd, tws: best.tws };
    }
  }
  if (seq !== arrowSeq || !cursorArrows()) return;
  arrowsAtCursor = true;
  arrows.planning(at, tw, hdg, bsp);
}

// ---------- instruments ----------------------------------------------------------
let vessel = {}; // latest Signal K state
const positionWatch = createPositionWatch();
const depthWatch = createDepthWatch();
const damping = createDamping((g) => Number(settings[{ motion: "dampMotion", wind: "dampWind", depth: "dampDepth" }[g]]) || 0);
function tideTime() {
  const v = document.documentElement.dataset.view;
  if ((v === "plan" || v === "env") && weather?.current().t) return Math.floor(weather.current().t);
  return boatTime();
}
function boatTime() {
  // planning: the weather slider's time; live: the GPS time, else the clock
  if (!live && weather?.current().t) return weather.current().t;
  // whole seconds: the Rust commands that take a time want an integer
  const t = Date.parse(vessel.datetime ?? "");
  return Math.floor(Number.isFinite(t) ? t / 1000 : Date.now() / 1000);
}
// replayOn is declared further down; feed() can run before that line during start-up
function replaying() { try { return replayOn; } catch { return false; } }
function feed() {
  const nav = shownNav();
  const pos = lastBoat;
  const wp = pos && nav ? waypointData(pos, vessel.sog, vessel.cog, nav.from, [nav.next.lon, nav.next.lat]) : null;
  const pts = routing?.activePoints();
  const target = pts ? targetAt(pts, boatTime()) : null;
  const v = live ? vessel : { ...vessel, lat: pos?.[1], lon: pos?.[0] };
  const positionAge = positionWatch.age();
  inst.update({ v, dataAt: live ? lastFeed : 0, positionAt: live && positionAge != null ? Date.now() - positionAge : null,
    depthAge: (field) => live ? depthWatch.age(field) : null, nav, wp, target, safetyDepth: settings.safetyDepth, replay: replaying() });
  // the route's next mark changed (or the route's start moved): redraw the laylines
  const key = nav ? `${nav.next.lon},${nav.next.lat},${nav.from}` : "";
  if (key !== layKey) { layKey = key; laylines?.refresh(); }
}
setInterval(feed, 1000);

// The bundled land and coast show while the chart tiles load (and when there are no
// charts). Once tiles arrive they are hidden: the tiles have their own world land, with
// the lakes and charted water cut out, and the bundled land would cover those.
let chartMapLoaded = false, pendingChartInfo = null, layerVis = null;
const FALLBACK_LAND = ["land-world", "coast-world", "land-nz", "coast-nz"];
let chartWorldLand = false; // the chart store has GSHHG world land (chart_info)
function setChartFallbackRange(loading) {
  if (!FALLBACK_LAND.every((id) => map.getLayer(id))) return;
  const show = loading || !chartWorldLand;
  for (const id of FALLBACK_LAND) map.setLayoutProperty(id, "visibility", show ? "visible" : "none");
  map.setLayerZoomRange("land-world", 0, 22);
  map.setLayerZoomRange("coast-world", 0, 22);
}
function chartsLoaded(info) {
  if (!info) return;
  chartWorldLand = !!info.world_land;
  window.dispatchEvent(new CustomEvent("racenav-charts", { detail: info }));
  if (info.ready === false) return;
  if (!(info.tables > 0)) return;
  if (!chartMapLoaded) {
    pendingChartInfo = info;
    return;
  }
  if (!chartPositionChosen) map.jumpTo(startupCamera);
  map.setLayerZoomRange("land-nz", 4, 6);
  map.setLayerZoomRange("coast-nz", 4, 6);
  enableChartLayers();
  setChartFallbackRange(!chartTileLoaded);
  requestAnimationFrame(() => {
    map.resize();
    reloadChartTiles();
    if (chartStoreReady && !chartTileLoaded) queueChartTileRetry(CHART_TILE_RETRY_DELAYS[0]);
  });
}
function enableChartLayers() {
  if (!chartMapLoaded || !chartStoreReady) return;
  for (const layer of map.getStyle().layers) {
    // the boundary and fishing rule layers are switched by marine.js
    if (layer.source === "chart" && !layer.id.startsWith("mar-")) map.setLayoutProperty(layer.id, "visibility", "visible");
  }
  features?.setEnabled(true);
  layerVis?.apply();
}
// A failed chart tile isn't automatically requested again by the map. Retry at a
// bounded rate so transient failures and tiles drawn before chart startup recover.
function retryFailedTiles() {
  if (chartStoreReady && !chartTileLoaded) {
    queueChartTileRetry(Math.max(750, lastRetry + 3000 - Date.now()));
    return;
  }
  if (tileRetry) return;
  const wait = Math.max(3000, lastRetry + 15000 - Date.now());
  tileRetry = setTimeout(() => { tileRetry = null; lastRetry = Date.now(); reloadChartTiles(); }, wait);
}
function queueChartTileRetry(delay) {
  if (!chartStoreReady || chartTileLoaded || chartTileRetry) return;
  chartTileRetry = setTimeout(() => {
    chartTileRetry = null;
    if (!chartStoreReady || chartTileLoaded) return;
    if (chartTileRetryAttempt >= CHART_TILE_RETRY_DELAYS.length) {
      startupToast("Chart tiles still loading - retrying");
      queueChartTileRetry(CHART_TILE_RETRY_DELAYS.at(-1));
      return;
    }
    lastRetry = Date.now();
    chartTileRetryAttempt++;
    reloadChartTiles();
    queueChartTileRetry(CHART_TILE_RETRY_DELAYS[chartTileRetryAttempt] ?? CHART_TILE_RETRY_DELAYS.at(-1));
  }, delay);
}
// Chart tiles that failed (503 while the chart store was opening, 500 after a render
// failure) are dropped and asked for afresh. Not map.refreshTiles: MapLibre's reload puts a
// failed vector tile back into "loading" before asking again, and a tile already "loading"
// waits for a load that never comes, so it stayed blank for good.
function reloadChartTiles() {
  const tm = map.style?.tileManagers?.chart;
  if (tm?.clearTiles) {
    tm.clearTiles();
    if (map._update) map._update(true); // sources are updated (and the tiles asked for) on the next frame
    else map.triggerRepaint();
  } else if (map.getSource("chart")) map.refreshTiles("chart");
}
let encBusy = false;
listen("chart-ready", ({ payload }) => {
  if (encBusy) {
    encBusy = false;
    if (startup.finished) toast("Charts ready", 3000);
  }
  noteChartsReady(payload);
}).catch((e) => console.error("chart-ready listener:", e));
// converting ENC cells the first time: show how far it has got
listen("chart-status", ({ payload }) => {
  const d = payload?.detail || "";
  if (d.startsWith("converting ENC") || d.startsWith("preparing charts:")) {
    encBusy = true;
    if (!startup.finished) {
      startup.progress = d;
      if (startup.mapLoaded && !startup.chartsReady) startupToast(chartLoadingMessage());
    } else {
      toast(d.charAt(0).toUpperCase() + d.slice(1));
    }
  }
}).catch((e) => console.error("chart-status listener:", e));
map.on("load", () => {
  chartMapLoaded = true;
  startup.mapLoaded = true;
  setChartFallbackRange(!chartTileLoaded);
  startupToast(startup.chartsReady ? "Charts ready - loading map tiles" : chartLoadingMessage());
  if (pendingChartInfo) {
    const info = pendingChartInfo;
    pendingChartInfo = null;
    noteChartsReady(info);
  }
  invoke("chart_info").then(noteChartsReady).catch((e) => console.error("chart info:", e));
});
map.on("idle", finishStartupWhenReady);
map.on("moveend", () => {
  if (chartStoreReady && !chartTileLoaded) {
    clearTimeout(chartTileRetry);
    chartTileRetry = null;
    chartTileRetryAttempt = 0;
    reloadChartTiles();
    queueChartTileRetry(100);
  }
});

/** The IHO portrayal's context: the chart store portrays again when it changes. Sent once
 *  the setting has stopped changing (the arrows of a number box change it per click); the
 *  chart keeps its symbols until the new ones are ready. */
let portrayalTimer = 0;
function sendPortrayalContext() {
  clearTimeout(portrayalTimer);
  portrayalTimer = setTimeout(sendPortrayalContextNow, 700);
}
function sendPortrayalContextNow() {
  invoke("chart_s52_context", { id: "SafetyContour", value: String(settings.safetyContour) }).catch(() => {});
  invoke("chart_s52_context", { id: "SafetyDepth", value: String(settings.safetyDepth) }).catch(() => {});
  invoke("chart_s52_context", { id: "ShallowWaterDangers", value: String(!!settings.shallowDangers) }).catch(() => {});
  invoke("chart_s52_context", { id: "FullLightLines", value: String(!!settings.fullLightLines) }).catch(() => {});
}
sendPortrayalContextNow();
// the chart store has portrayed the features (or again, after a change): draw the tiles again
listen("chart-s52", () => reloadChartTiles()).catch(() => {});

function applyChartSettings() {
  sendPortrayalContext();
  map.setPaintProperty("depare", "fill-color", depareColour());
  map.setPaintProperty("depare", "fill-pattern", depareFillPattern());
  map.setFilter("safety-contour", safetyContourFilter());
  if (map.getLayer("depcnt-lbl")) map.setFilter("depcnt-lbl", contourLabelFilter());
  map.setLayoutProperty("soundg", "icon-image", soundingImage());
  features?.setSafetyContour(settings.safetyContour, !!settings.shallowDangers);
}
for (const [id, key] of [["set-safety-depth", "safetyDepth"], ["set-safety-contour", "safetyContour"]]) {
  const el = document.getElementById(id);
  el.value = settings[key];
  el.onchange = () => {
    const v = parseFloat(el.value);
    if (Number.isFinite(v) && v >= 0) { settings[key] = v; saveSettings(); applyChartSettings(); feed(); safeNote(); }
  };
}
{
  const el = document.getElementById("set-shallow-dangers");
  if (el) {
    el.checked = !!settings.shallowDangers;
    el.onchange = () => { settings.shallowDangers = el.checked; saveSettings(); applyChartSettings(); };
  }
  const sh = document.getElementById("set-shades");
  if (sh) {
    sh.value = settings.fourShades === false ? "2" : "4";
    sh.onchange = () => { settings.fourShades = sh.value !== "2"; saveSettings(); applyChartSettings(); };
  }
  const cl = document.getElementById("set-contour-labels");
  if (cl) {
    cl.checked = !!settings.contourLabels;
    cl.onchange = () => { settings.contourLabels = cl.checked; saveSettings(); applyChartSettings(); };
  }
  const sp = document.getElementById("set-shallow-pattern");
  if (sp) {
    sp.checked = !!settings.shallowPattern;
    sp.onchange = () => { settings.shallowPattern = sp.checked; saveSettings(); applyChartSettings(); };
  }
  // S-52 mariner option: sector limits drawn out to the light's nominal range
  const fl = document.getElementById("set-full-light-lines");
  if (fl) {
    fl.checked = !!settings.fullLightLines;
    fl.onchange = () => { settings.fullLightLines = fl.checked; saveSettings(); sendPortrayalContext(); };
  }
}
// Chart › Safety: the safety depth against the vessel's draft (Settings › Vessel)
function safeNote() {
  const el = document.getElementById("safe-note");
  if (!el) return;
  const d = Number(boat?.get?.().draft) || 0;
  el.textContent = d > 0
    ? `Draft ${d} m (Settings): safety depth ${settings.safetyDepth} m leaves ${(settings.safetyDepth - d).toFixed(1)} m under the keel.${settings.safetyDepth <= d ? " Raise it: it is no deeper than the draft." : ""} The safety contour is drawn bold; deeper dangers show as their depth only.`
    : "Set the draft in Settings › Vessel. Safety depth: the draft plus a margin for swell, squat and chart error. The safety contour is drawn bold.";
  el.classList.toggle("warn", d > 0 && settings.safetyDepth <= d);
}
window.addEventListener("racenav-boat", safeNote);
setTimeout(safeNote, 0);

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
/** Magnetic variation (east positive) at [lon, lat]: the instruments' own value near the boat,
 *  otherwise the World Magnetic Model. */
function variationAt(pos) {
  const live = vessel.variation;
  if (Number.isFinite(live) && lastBoat && rhumb(lastBoat, pos).nm < 100) return live;
  return magneticVariation(pos[1], pos[0])?.variation ?? null;
}
function cursorText(lngLat) {
  const where = `${ddm(lngLat.lat, "N", "S")}  ${ddm(lngLat.lng, "E", "W")}`;
  const leg = inst.legTo([lngLat.lng, lngLat.lat]);
  return leg ? `${where}  ·  ${leg}` : where;
}
let cursorOwn = "", cursorSurvey = "", cursorWhere = "";
const showCursor = () => { document.getElementById("cursor").textContent = [cursorWhere, cursorSurvey, cursorOwn].filter(Boolean).join("  ·  "); };
map.on("mousemove", (e) => {
  cursorWhere = cursorText(e.lngLat);
  showCursor();
  // with own soundings shown: the sounding under the cursor; with survey depths, theirs
  pbathyAt(e.lngLat.lng, e.lngLat.lat, (t, s) => { cursorOwn = t || ""; showCursor(); inst.update({ sounding: s }); });
  surveyDepthAt(map, e.lngLat.lng, e.lngLat.lat, (t) => { cursorSurvey = t; showCursor(); });
  setCursor([e.lngLat.lng, e.lngLat.lat]);
});
// a click sets the cursor too (it then stays put while the hover moves on)
map.on("click", (e) => setCursor([e.lngLat.lng, e.lngLat.lat]));
map.getCanvas().addEventListener("mouseleave", () => { setCursor(null); inst.update({ sounding: null }); arrowsAtCursor = false; boatArrows(); });

// ---------- follow / track -------------------------------------------------
let follow = true;
const followBtn = document.getElementById("follow");
function setFollow(on) {
  follow = on;
  followBtn.classList.toggle("on", on);
  followBtn.setAttribute("aria-pressed", String(on));
}
setFollow(true);
followBtn.onclick = () => setFollow(!follow);
map.on("dragstart", () => setFollow(false));

// The boat's place on the screen while following, as a fraction of the map's width and height from its
// centre. Drag the boat to set it (to see more chart ahead); double-click the boat to centre it again.
const FOLLOW_KEY = "tweedplot.followOffset";
let followFrac = (() => {
  try { const o = JSON.parse(localStorage.getItem(FOLLOW_KEY) || "null"); if (Array.isArray(o) && o.every(Number.isFinite)) return o; } catch { /* ignore */ }
  return [0, 0];
})();
const followOffset = () => {
  const c = map.getCanvas();
  return [followFrac[0] * c.clientWidth, followFrac[1] * c.clientHeight];
};
const saveFollowFrac = () => { try { localStorage.setItem(FOLLOW_KEY, JSON.stringify(followFrac)); } catch { /* ignore */ } };
let boatDrag = false;
const onBoat = (point) => {
  if (!lastBoat || !map.getLayer("boat")) return false;
  if (map.queryRenderedFeatures(point, { layers: ["boat", "boat-hull"].filter((l) => map.getLayer(l)) }).length > 0) return true;
  // the IHO own ship symbol (a double circle, mariner.js): within its 3 mm radius
  if (!features?.usingS52()) return false;
  const p = map.project(lastBoat);
  return Math.hypot(p.x - point.x, p.y - point.y) <= 3.2 * s52PxPerMm();
};
function startBoatDrag(e) {
  // the left button (or a touch) drags; a right-click on the boat opens the menu
  if (e.originalEvent?.button > 0) return;
  if (!follow || !onBoat(e.point)) return;
  e.preventDefault(); // not a chart pan
  map.stop();
  boatDrag = true;
  map.getCanvas().style.cursor = "grabbing";
}
function moveBoatDrag(e) {
  if (!boatDrag || !lastBoat) return;
  const c = map.getCanvas();
  const clamp = (v) => Math.min(0.45, Math.max(-0.45, v));
  followFrac = [clamp(e.point.x / c.clientWidth - 0.5), clamp(e.point.y / c.clientHeight - 0.5)];
  // easeTo, not jumpTo: jumpTo has no offset, so the boat went to the middle of the chart while it was held
  map.easeTo({ center: lastBoat, offset: followOffset(), duration: 0 });
}
function endBoatDrag() {
  if (!boatDrag) return;
  boatDrag = false;
  map.getCanvas().style.cursor = "";
  saveFollowFrac();
}
map.on("mousedown", startBoatDrag);
map.on("mousemove", moveBoatDrag);
map.on("mouseup", endBoatDrag);
window.addEventListener("blur", endBoatDrag);
map.on("dblclick", (e) => {
  if (!follow || !onBoat(e.point)) return;
  e.preventDefault(); // not a zoom
  followFrac = [0, 0];
  saveFollowFrac();
  if (lastBoat) map.easeTo({ center: lastBoat, offset: [0, 0], duration: 300 });
});

// ---------- keyboard shortcuts (keys.js holds the table) ------------------------------------
setupKeys(map, {
  ui, views,
  followOff: () => setFollow(false),
  centreBoat: () => {
    if (!lastBoat) { toast("No boat position yet", 2500); return; }
    setFollow(true);
    map.easeTo({ center: lastBoat, offset: followOffset(), duration: 300 });
  },
  weatherStep: (n) => document.getElementById(n < 0 ? "side-wx-prev" : "side-wx-next").click(),
  weatherHours: (h) => {
    const sl = document.getElementById("side-wx-time");
    if (!weather || Number(sl.max) <= Number(sl.min)) { toast("Load a GRIB first", 2500); return; }
    weather.setTime(Number(sl.value) + h * 3600);
  },
  findInLibrary: () => {
    ui.open("library");
    const box = [...document.querySelectorAll('.pane[data-pane="library"] .ll-search')].find((el) => el.offsetParent);
    box?.focus();
    box?.select();
  },
  silence: () => {
    const n = alarms?.ackAll() ?? 0;
    toast(n ? `${n === 1 ? "Alarm" : `${n} alarms`} silenced` : "No alarm is sounding", 2500);
  },
});

const TRACK_MAX = 20000;
let track = [];
let trackT = []; // the time of each track point (Unix seconds), for the time marks
document.getElementById("clear-track").onclick = () => { track = []; trackT = []; setData("track", line(track)); mariner?.track(track, trackT); };

// ---------- Signal K events -------------------------------------------------
// ---------- Signal K status, live / planning ----------------------------------
let skConnected = false;
let skDetail = "connecting…";
let skSeen = false; // a status has arrived (else still connecting)
let nmeaNavigationSourceId = null;
let nmeaNavigationSourceName = "";
let nmeaConnected = false;
let nmeaSeen = false;
let nmeaDetail = "";
let activeNmeaGroupSources = {};
let lastData = 0; // ms timestamp of the last update from the selected live-data source
let lastFeed = 0; // last vessel update, live or replayed (what the boat and instruments show)
let replayOn = false;
let alarms = null;
const pendingNmeaAlarms = [];
let lastRaw = null; // the last vessel update, for what is arriving
let live = true;
const SK_STALE_MS = 10000;
/** The status button: colour = the link (green data arriving, amber connected
 * but quiet for 10 s, red no link), whether plotting is live or paused. */
function showStatus() {
  const el = document.getElementById("sk-state");
  const usingNmea = !!nmeaNavigationSourceId;
  const connected = usingNmea ? nmeaConnected : skConnected;
  const seen = usingNmea ? nmeaSeen : skSeen;
  const detail = usingNmea ? nmeaDetail : skDetail;
  const sourceName = usingNmea ? `NMEA 0183 · ${nmeaNavigationSourceName}` : "Signal K";
  const age = lastData ? Date.now() - lastData : Infinity;
  const state = !connected ? "bad" : age < SK_STALE_MS ? "good" : "stale";
  el.className = state + (seen ? "" : " connecting");
  const ago = (ms) => (ms < 90000 ? `${Math.max(1, Math.round(ms / 1000))} s` : ms < 5400000 ? `${Math.round(ms / 60000)} min` : `${(ms / 3600000).toFixed(1)} h`);
  const what = lastRaw ? [["lat", "position"], ["heading", "heading"], ["sog", "speed"], ["stw", "boat speed"], ["aws", "wind"], ["tws", "wind"], ["depth", "depth"], ["depth_keel", "depth"], ["depth_surface", "depth"]]
    .filter(([k]) => lastRaw[k] != null).map(([, w]) => w).filter((w, i, a) => a.indexOf(w) === i).join(", ") : "";
  const head = !seen ? `${sourceName}: connecting…`
    : state === "bad" ? `${sourceName}: not connected`
    : state === "good" ? `${sourceName}: receiving data` : `${sourceName}: connected, no data`;
  el.title = [head, seen ? detail : "",
    lastData ? `Last update ${ago(age)} ago${what && state === "good" ? `: ${what}` : ""}` : connected ? "No data yet" : "",
    live ? "" : "Plotting paused (planning): the boat stays where you put it",
    usingNmea ? "Click for NMEA source settings" : "Click for the Signal K settings"].filter(Boolean).join("\n");
  el.setAttribute("aria-label", head);
  const liveControl = document.getElementById("sk-live");
  liveControl.setAttribute("aria-pressed", String(live));
  liveControl.setAttribute("aria-label", replayOn ? `Replay, plotting ${live ? "on" : "paused"}` : `Plotting ${live ? "live" : "paused"}`);
  liveControl.title = replayOn
    ? `Replay: plotting ${live ? "on. Click to pause" : "paused. Click to resume"}. Stop the replay to return to live data.`
    : live
      ? `Live: the boat follows ${usingNmea ? `NMEA 0183 · ${nmeaNavigationSourceName}` : "Signal K"}. Click to pause and plan (right-click the chart to move the boat).`
      : "Paused (planning): the boat stays where you put it. Click to go live.";
}
function setNmeaNavigationSource(payload) {
  const id = payload?.id ?? null;
  if (id !== nmeaNavigationSourceId) {
    positionWatch.reset();
    depthWatch.reset();
    nmeaConnected = false;
    nmeaSeen = false;
    nmeaDetail = "";
    lastData = 0;
    lastFeed = 0;
    lastRaw = null;
    vessel = {};
    damping.clear();
    if (live) {
      lastBoat = null;
      setData("boat", emptyFC());
      setData("boat-hull", emptyFC());
      mariner?.boat(null);
      drawLines(null);
      arrows?.clear();
    }
    feed();
  }
  nmeaNavigationSourceId = id;
  nmeaNavigationSourceName = payload?.name ?? "";
  showStatus();
}
listen("nmea-navigation-source", ({ payload }) => setNmeaNavigationSource(payload));
listen("nmea-mux-status", ({ payload }) => {
  if (!payload) return;
  const nextSources = payload.groupSources || {};
  if (JSON.stringify(nextSources) !== JSON.stringify(activeNmeaGroupSources)) damping.clear();
  activeNmeaGroupSources = nextSources;
  nmeaSeen = true;
  nmeaConnected = !!payload.connected;
  nmeaDetail = payload.detail || payload.state;
  if (Object.keys(payload.groupSources || {}).length) lastData = Date.now();
  showStatus();
});
listen("nmea-failover", ({ payload }) => {
  if (!payload) return;
  const id = `nmea-failover-${payload.group}`;
  const detail = {
    level: "alarm",
    clearable: true,
    text: `${payload.label || payload.group} source changed`,
    detail: `Using ${payload.to} instead of ${payload.from}. Check the preferred source and connection.`,
  };
  if (alarms) {
    alarms.clear(id);
    alarms.raise(id, detail);
  } else pendingNmeaAlarms.push([id, detail]);
});
invoke("nmea_sources_get").then((data) => {
  const enabled = data.sources?.some((source) => source.enabled);
  setNmeaNavigationSource(enabled ? { id: "nmea-multiplexer", name: "NMEA multiplexer" } : null);
}).catch((error) => console.error("Could not load selected NMEA source:", error));
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
  openConnectionCard(nmeaNavigationSourceId ? "nmea-0183-section" : "sk-section");
};
// the toolbar's ruler: one press to measure (the chart menu has "Start ruler here" too)
document.getElementById("ebl-btn").onclick = () => mariner?.startEbl();
document.getElementById("ruler-btn").onclick = () => { if (!ruler) return; if (ruler.isArmed()) ruler.finish(); else ruler.start(); };
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
const liveDataConnected = () => nmeaNavigationSourceId ? nmeaConnected || replayOn : skConnected || replayOn;
const signalKActive = () => live && liveDataConnected() && Date.now() - lastFeed < 15000;

/** The boat on the chart: its symbol, or at harbour scales its outline to scale from
 *  the Settings length and breadth, placed by the GPS antenna. */
let boatDir = 0;
function drawBoat() {
  if (!lastBoat) return;
  const b = boat.get();
  const big = b.length > 0 && bigEnough(b.length, lastBoat[1], map.getZoom());
  setData("boat", { type: "FeatureCollection", features: [{ type: "Feature", properties: { rot: boatDir, big }, geometry: { type: "Point", coordinates: lastBoat } }] });
  const f = [];
  if (big) {
    const ant = antennas.offsets(b);
    f.push({ type: "Feature", properties: { k: "hull" }, geometry: { type: "Polygon", coordinates: [hullRing(lastBoat, boatDir, { length: b.length, beam: b.beam, fromBow: ant.fromBow || null, fromCentre: ant.fromCentre })] } });
    f.push({ type: "Feature", properties: { k: "gps" }, geometry: { type: "Point", coordinates: lastBoat } });
  }
  setData("boat-hull", { type: "FeatureCollection", features: f });
  mariner?.boat(lastBoat, boatDir, live ? vessel : null, big);
}
map.on("zoomend", drawBoat);
window.addEventListener("racenav-boat", drawBoat);

/** Put the boat at a position by hand (planning, or when there is no Signal K). */
function placeBoat(pos) {
  lastBoat = pos;
  boatDir = 0;
  drawBoat();
  drawLines(null);
  feed();
}

// ---------- opening what's on the chart in the Library ------------------------------
// Double-click a library mark, barrier, track or the course on the chart: it opens in the
// Library drawer (its row, and its editor). The right-click menu has the same.
/** What library object is under a map point: { kind, id, name } (the first of mark, barrier, track, route). */
function libraryObjectAt(point, edge = false) {
  const m = library?.markAt(point);
  if (m) return { kind: "mark", id: m.id, name: m.name || "the mark" };
  const b = barriers?.isDrawing() ? null : barriers?.hitAt(point, edge);
  if (b) return { kind: "barrier", id: b.id, name: b.name };
  const t = tracks?.hitAt(point);
  if (t) return { kind: "track", id: t.id, name: t.name };
  if (route?.hitAt(point)) {
    const id = route.current().routeId;
    return { kind: "route", id, name: id ? (library?.routeName?.(id) ?? "the route") : "the route" };
  }
  return null;
}
function openInLibrary(o) {
  if (!o) return;
  if (o.kind === "mark") library?.openMark(o.id);
  else if (o.kind === "barrier") { ui.openTab("library", "barriers"); barriers?.reveal(o.id); }
  else if (o.kind === "track") { ui.openTab("library", "tracks"); tracks?.open(o.id); }
  else if (o.kind === "route") ui.open("course"); // the route on the chart is the one in the editor: Route › Plan (the panel in Planning)
}
map.on("dblclick", (e) => {
  // drawing a barrier or measuring uses double-click to finish
  if (placing() || barriers?.isDrawing() || ruler?.isArmed() || depthStack?.isDrawing()) return;
  const o = libraryObjectAt(e.point, true); // inside a big zone, a double-click still zooms
  if (!o) return;
  e.preventDefault(); // not a zoom
  openInLibrary(o);
});

// ---------- right-click menu ----------------------------------------------------
const ctx = document.getElementById("ctx");
let ctxAt = null;
let ctxPoint = null; // the same place, in map pixels
let ctxMark = null; // library mark under the right-click, if any
let ctxAnchor = null; // anchor mark under the right-click, if any
let ctxRuler = null; // ruler under the right-click: { rid, i, points }
let ctxBarrier = null; // barrier under the right-click: { id, i, active, name }
let ctxObject = null; // library object under the right-click (Edit in the library)
let ctxRings = null; // what range rings would go round (mariner.js ringTarget)
let ctxEbl = null; // EBL/VRM under the right-click (index)
let ctxReach = null; // route mark under the right-click: its index
let ctxRouting = null; // weather routing under the right-click: { key, name, saved }
map.on("contextmenu", (e) => {
  e.originalEvent.preventDefault();
  ctxAt = [e.lngLat.lng, e.lngLat.lat];
  ctxPoint = { x: e.point.x, y: e.point.y };
  // right-click on a library mark: start / add use that mark
  ctxMark = library?.markAt(e.point) ?? null;
  ctx.querySelector('[data-act="start"]').textContent = ctxMark ? `Start route at ${ctxMark.name}` : "Start route here";
  ctx.querySelector('[data-act="mark"]').textContent = ctxMark ? `Add ${ctxMark.name} to route` : "Add mark to route here";
  ctx.querySelector('[data-act="libmark"]').hidden = !!ctxMark;
  ctx.querySelector('[data-act="goto"]').textContent = ctxMark ? `Go to ${ctxMark.name}` : "Go to here";
  // the anchor: drop here, again at an anchor mark, or weigh the one watched
  ctxAnchor = anchor?.at(e.point) ?? (ctxMark?.kind === "anchor" ? ctxMark : null);
  const watching = !!anchor?.watching();
  ctx.querySelector('[data-act="rec-toggle"]').textContent = pbathy?.recording() ? "Stop recording soundings" : "Record soundings";
  ctx.querySelector('[data-act="rec-toggle"]').hidden = !pbathy?.recording() && use.use() !== "fish" && !("recAll" in document.documentElement.dataset);
  ctx.querySelector('[data-act="anc-drop"]').hidden = watching || !!ctxAnchor;
  ctx.querySelector('[data-act="anc-again"]').hidden = watching || !ctxAnchor;
  ctx.querySelector('[data-act="anc-weigh"]').hidden = !watching;
  ctx.querySelector('[data-act="goto-clear"]').hidden = !gotoTarget;
  ctxRose = !!varRoses?.hit();
  ctx.querySelector('[data-act="rose-add"]').hidden = ctxRose;
  ctx.querySelector('[data-act="rose-add"]').textContent = varRoses?.shown() ? "Move variation diagram here" : "Magnetic variation diagram here";
  ctx.querySelector('[data-act="rose-del"]').hidden = !ctxRose;
  const fishing = use.use() === "fish";
  ctx.querySelector('[data-act="lay-toggle"]').hidden = fishing;
  ctx.querySelector('[data-act="cone-toggle"]').hidden = fishing;
  ctx.querySelector('[data-act="lay-toggle"]').textContent = boat.get().layShow ? "Laylines off" : "Laylines on";
  ctx.querySelector('[data-act="cone-toggle"]').textContent = boat.get().layConeShow ? "Tacking cone off" : "Tacking cone on";
  // range rings: a library mark or route waypoint under the right-click, or own ship
  ctxRings = mariner?.ringTarget(e.point) ?? (onBoat(e.point) ? mariner?.ownRingTarget() : null) ?? null;
  const ringsBtn = ctx.querySelector('[data-act="rings"]');
  ringsBtn.hidden = !ctxRings;
  if (ctxRings) ringsBtn.textContent = `Range rings round ${ctxRings.name}…`;
  // EBL/VRM: a new one from here; one under the right-click: adjust or delete it
  ctxEbl = mariner?.eblAt(e.point) ?? null;
  ctx.querySelector('[data-act="ebl-here"]').hidden = ctxEbl != null;
  for (const el of ctx.querySelectorAll(".ebl-item")) el.hidden = ctxEbl == null;
  if (ctxEbl != null) {
    ctx.querySelector('[data-act="ebl-adj"]').textContent = `Adjust EBL/VRM ${ctxEbl + 1}`;
    ctx.querySelector('[data-act="ebl-del"]').textContent = `Delete EBL/VRM ${ctxEbl + 1}`;
  }
  // a ruler under the right-click: delete it, a point of it, or save it as a route
  ctxRuler = ruler?.isArmed() ? null : ruler?.hitAt(e.point) ?? null;
  for (const el of ctx.querySelectorAll(".ruler-item")) el.hidden = !ctxRuler;
  ctxBarrier = ctxRuler || barriers?.isDrawing() ? null : barriers?.hitAt(e.point) ?? null;
  ctxObject = ctxRuler ? null : libraryObjectAt(e.point);
  const openBtn = ctx.querySelector('[data-act="open-lib"]');
  openBtn.hidden = !ctxObject;
  if (ctxObject) openBtn.textContent = ctxObject.kind === "route" ? "Edit the route" : `Edit “${ctxObject.name}” in the library`;
  for (const el of ctx.querySelectorAll(".bar-item")) el.hidden = !ctxBarrier;
  if (ctxBarrier) {
    ctx.querySelector('[data-act="bar-pt"]').hidden = ctxBarrier.i == null;
    ctx.querySelector('[data-act="bar-toggle"]').textContent = ctxBarrier.active ? `Switch “${ctxBarrier.name}” off` : `Switch “${ctxBarrier.name}” on`;
    ctx.querySelector('[data-act="bar-del"]').textContent = `Delete “${ctxBarrier.name}”`;
  }
  if (ctxRuler) {
    ctx.querySelector('[data-act="ruler-pt"]').hidden = ctxRuler.i == null;
    ctx.querySelector('[data-act="ruler-all"]').hidden = ruler.count() < 2;
    ctx.querySelector('[data-act="ruler-great-circle"]').textContent =
      ctxRuler.greatCircle ? "Show ruler as rhumb line" : "Show ruler as great circle";
  }
  const rulerToggle = ctx.querySelector('[data-act="ruler-toggle"]');
  rulerToggle.hidden = !!ctxRuler;
  rulerToggle.textContent = ruler?.isArmed() ? "Finish ruler" : "Start ruler here";
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
  // a route mark: how the routing reaches it (not the start; not while it is navigated)
  const wp = route?.waypointAt(e.point);
  const reach = wp != null && wp >= 0 ? route.reachOf(wp) : null;
  ctxReach = reach ? wp : null;
  for (const el of ctx.querySelectorAll(".rnd-item")) el.hidden = !reach;
  if (reach) {
    ctx.querySelector(".ctx-head.rnd-item").textContent = `How to reach ${reach.name}`;
    for (const b of ctx.querySelectorAll('[data-act="reach"]')) {
      b.classList.toggle("on", b.dataset.v === reach.rounding);
      b.disabled = reach.locked;
      b.title = reach.locked ? "The route is being navigated: Stop navigating to change it" : "";
    }
  }
  // a weather routing: find it on the list, hide or discard it
  ctxRouting = routing?.hitAt(e.point) ?? null;
  for (const el of ctx.querySelectorAll(".wr-item")) el.hidden = !ctxRouting;
  if (ctxRouting) {
    ctx.querySelector(".ctx-head.wr-item").textContent = `Routing “${ctxRouting.name}”`;
    const del = ctx.querySelector('[data-act="wr-del"]');
    del.textContent = ctxRouting.saved ? "Take off the Routings list" : "Discard routing";
    del.title = ctxRouting.saved ? "It stays saved in the library" : "Delete it (it isn't saved)";
  }
  const boatBtn = ctx.querySelector('[data-act="boat"]');
  boatBtn.disabled = signalKActive();
  boatBtn.title = boatBtn.disabled ? "Signal K is plotting the boat: press Live to pause it first" : "";
  ctx.hidden = false;
  // the menu is on the page; e.point is within the chart, which no longer starts at the window's edge
  const cr = map.getContainer().getBoundingClientRect();
  ctx.style.left = `${Math.max(4, Math.min(cr.left + e.point.x, window.innerWidth - ctx.offsetWidth - 4))}px`;
  ctx.style.top = `${Math.max(4, Math.min(cr.top + e.point.y, window.innerHeight - ctx.offsetHeight - 4))}px`;
});
document.addEventListener("contextmenu", (e) => e.preventDefault());
ctx.addEventListener("click", (e) => {
  const act = e.target.dataset.act;
  if (!act || e.target.disabled || !ctxAt) return;
  if (act === "reach" && ctxReach != null) route?.setReach(ctxReach, e.target.dataset.v);
  if (act === "wr-show" && ctxRouting) routing?.reveal(ctxRouting.key);
  if (act === "wr-hide" && ctxRouting) routing?.hide(ctxRouting.key);
  if (act === "wr-del" && ctxRouting) routing?.discard(ctxRouting.key);
  if (act === "move-sel") route?.moveSelected(ctxAt);
  if (act === "del-sel") route?.deleteSelected();
  if (act === "boat") placeBoat(ctxAt);
  if (act === "start") route?.setStart(ctxAt, ctxMark);
  if (act === "mark") route?.addMark(ctxAt, ctxMark);
  if (act === "libmark") library?.newMarkAt(ctxAt);
  if (act === "rec-toggle") pbathy?.record(!pbathy.recording());
  if (act === "anc-drop") anchor?.drop(ctxAt);
  if (act === "anc-again" && ctxAnchor) anchor?.again(ctxAnchor);
  if (act === "anc-weigh") anchor?.weigh();
  if (act === "goto") setGoto({ name: ctxMark?.name ?? "Go to", lon: ctxMark?.lon ?? ctxAt[0], lat: ctxMark?.lat ?? ctxAt[1] });
  if (act === "goto-clear") setGoto(null);
  if (act === "rose-add") varRoses?.place(ctxPoint);
  if (act === "rose-del") varRoses?.remove();
  if (act === "lay-toggle") boat.toggle("layShow");
  if (act === "cone-toggle") boat.toggle("layConeShow");
  if (act === "open-lib") openInLibrary(ctxObject);
  if (act === "centre") map.easeTo({ center: ctxAt });
  if (act === "rings" && ctxRings) mariner?.openRings(ctxRings);
  if (act === "ebl-here") mariner?.startEbl(ctxAt);
  if (act === "ebl-adj" && ctxEbl != null) mariner?.adjustEbl(ctxEbl);
  if (act === "ebl-del" && ctxEbl != null) mariner?.deleteEbl(ctxEbl);
  if (act === "meteogram") gribTools?.meteogramAt(ctxAt);
  if (act === "ruler-toggle") {
    if (ruler.isArmed()) ruler.finish();
    else ruler.startAt(ctxAt);
  }
  if (act === "ruler-route" && ctxRuler) ruler.saveAsRoute(ctxRuler.rid);
  if (act === "ruler-great-circle" && ctxRuler) ruler.setGreatCircle(ctxRuler.rid, !ctxRuler.greatCircle);
  if (act === "ruler-pt" && ctxRuler) ruler.deletePoint(ctxRuler.rid, ctxRuler.i);
  if (act === "ruler-del" && ctxRuler) ruler.deleteRuler(ctxRuler.rid);
  if (act === "ruler-all") ruler.deleteAll();
  if (act === "bar-pt" && ctxBarrier) barriers.deletePoint(ctxBarrier.id, ctxBarrier.i);
  if (act === "bar-del" && ctxBarrier) barriers.remove(ctxBarrier.id);
  if (act === "bar-toggle" && ctxBarrier) barriers.toggle(ctxBarrier.id);
  ctx.hidden = true;
});
map.on("click", () => { ctx.hidden = true; });
map.on("movestart", (e) => { if (e.originalEvent) ctx.hidden = true; }); // not the boat-follow nudges
window.addEventListener("keydown", (e) => { if (e.key === "Escape") ctx.hidden = true; });

listen("vessel", ({ payload }) => {
  const raw = payload.v ?? payload;
  if (payload.replay) {
    if (!replayOn) return; // a late message after the replay stopped
  } else {
    if (replayOn) return;
    if (nmeaNavigationSourceId) {
      if (payload.source !== "nmea0183" || payload.sourceId !== "nmea-multiplexer") return;
      nmeaConnected = true;
      nmeaSeen = true;
    } else {
      if (payload.source === "nmea0183") return;
      if (!skConnected) skConnected = true; // data proves the link is up
    }
    const fresh = !lastData || Date.now() - lastData >= SK_STALE_MS;
    lastData = Date.now();
    lastRaw = raw;
    if (fresh) showStatus();
  }
  const validPosition = Number.isFinite(raw.lat) && raw.lat >= -90 && raw.lat <= 90
    && Number.isFinite(raw.lon) && raw.lon >= -180 && raw.lon <= 180;
  positionWatch.observe(raw, !!payload.replay);
  depthWatch.observe(raw, !!payload.replay);
  if (validPosition) hasPositionFix = true;
  if (validPosition && !payload.replay && payload.source === "nmea0183") {
    useInitialPosition([raw.lon, raw.lat]);
  }
  lastFeed = Date.now();
  if (!live) return; // planning: leave the boat where it is
  const v = damping.push(raw);
  vessel = v;
  if (v.lat == null || v.lon == null) {
    if (payload.source === "nmea0183" && lastBoat) {
      lastBoat = null;
      setData("boat", emptyFC());
      setData("boat-hull", emptyFC());
      mariner?.boat(null);
      drawLines(null);
      arrows?.clear();
    }
    feed();
    return;
  }

  const pos = [v.lon, v.lat];
  lastBoat = pos;
  boatDir = v.heading ?? v.cog ?? 0;
  drawBoat();

  drawLines(v);

  const last = track.at(-1);
  if (!last || Math.abs(last[0] - pos[0]) > 1e-5 || Math.abs(last[1] - pos[1]) > 1e-5) {
    track.push(pos);
    trackT.push(boatTime());
    if (track.length > TRACK_MAX) { track.shift(); trackT.shift(); }
    setData("track", line(track));
    mariner?.track(track, trackT);
  }
  if (follow && ctx.hidden && !boatDrag) map.easeTo({ center: pos, offset: followOffset(), duration: 200 }); // the menu stays where it was opened
  if (!(arrowsAtCursor && cursorArrows())) arrows?.live(pos, v);
  if (!gotoTarget) route?.boatAt(pos); // a go to holds the route where it was
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

/** The unsurveyed / no-data pattern (as S-52 NODATA03): thin grey diagonals on clear. */
function noDataPattern() {
  const n = 24, c = document.createElement("canvas");
  c.width = c.height = n;
  const g = c.getContext("2d");
  g.strokeStyle = "rgba(90, 105, 112, 0.55)";
  g.lineWidth = 1.5;
  g.beginPath();
  for (const o of [-n, 0, n]) { g.moveTo(o, n); g.lineTo(o + n, 0); }
  g.stroke();
  return g.getImageData(0, 0, n, n);
}

/** Sounding figure in S-52 style: metres, tenths as a smaller subscript below 31 m, drying heights underlined. */
/** A depth contour's label (S-52 SAFCON digits): its depth, small, in the contour colour. */
function contourLabelIcon(depth) {
  const t = String(Math.round(Math.abs(depth) * 10) / 10);
  const c = document.createElement("canvas");
  const g = c.getContext("2d");
  // the size of the IHO's text (10 pt, about 3.5 mm on a calibrated screen), at pixel ratio 2
  const px = Math.round(3.5 * s52PxPerMm() * 2);
  const f = `600 ${px}px "Segoe UI", system-ui, sans-serif`;
  g.font = f;
  c.width = Math.ceil(g.measureText(t).width) + 4;
  c.height = Math.ceil(px * 1.25);
  g.font = f;
  g.textBaseline = "middle";
  g.fillStyle = chartColours().safety;
  g.fillText(t, 2, c.height / 2);
  return g.getImageData(0, 0, c.width, c.height);
}
function contourLabelFilter() {
  return settings.contourLabels ? ["has", "valdco"] : ["has", "__never__"];
}
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
  const cc = chartColours();
  g.fillStyle = shallow ? cc.soundShallow : cc.soundDeep;
  g.textBaseline = "alphabetic";
  g.font = fBig;
  g.fillText(String(whole), pad, big - 2);
  if (tenth) { g.font = fSmall; g.fillText(String(tenth), pad + w1, big + 3); }
  if (dry) { g.fillRect(pad, big + 4, w1 + w2, 2); }
  return g.getImageData(0, 0, c.width, c.height);
}

/** The optional Anchor button (Settings › Display): drops anchor, or weighs it when anchored.
 * Weighing ends the anchor watch, so it takes a second press within 4 s (the button flashes). */
function setupAnchorButton(anchor) {
  const b = document.getElementById("anchor-btn");
  const KEY = "racenav.anchorBtn";
  const showBtn = (on) => { if (on) document.documentElement.dataset.anchorBtn = ""; else delete document.documentElement.dataset.anchorBtn; };
  try { showBtn(localStorage.getItem(KEY) === "1"); } catch { /* storage unavailable */ }
  const cb = document.getElementById("set-anchor-btn");
  if (cb) {
    cb.checked = "anchorBtn" in document.documentElement.dataset;
    cb.onchange = () => { showBtn(cb.checked); try { localStorage.setItem(KEY, cb.checked ? "1" : "0"); } catch { /* storage unavailable */ } };
  }
  let armed = null;
  const disarm = () => { clearTimeout(armed); armed = null; b.classList.remove("armed"); sync(); };
  const sync = () => {
    const on = anchor.watching();
    b.classList.toggle("on", on);
    const label = armed ? "Press again to weigh anchor" : on ? "Weigh anchor" : "Drop anchor";
    b.setAttribute("aria-label", label);
    b.title = armed ? "Press again to weigh anchor and stop the anchor watch (the anchor mark stays)"
      : on ? "Weigh anchor: stops the anchor watch; the anchor mark stays in the Library. Press twice"
        : "Drop anchor at the bow and watch it (then drag the anchor to where it lies)";
  };
  b.onclick = () => {
    if (!anchor.watching()) { disarm(); anchor.drop(); return; }
    if (!armed) { armed = setTimeout(disarm, 4000); b.classList.add("armed"); sync(); return; }
    disarm();
    window.dispatchEvent(new Event("racenav-anchor-weigh"));
  };
  window.addEventListener("racenav-anchor-watch", () => { if (!anchor.watching()) disarm(); else sync(); });
  sync();
}
