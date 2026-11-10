// AIS targets on the chart: from Signal K (a receiver on board) and online from
// aisstream.io shore stations, merged in Rust by MMSI. Symbols by kind and
// ship type, a 6-minute course vector, names, an info box, a target list with CPA
// and TCPA, and collision / distress alarms.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ddm } from "./navmath.js";
import { floatResize } from "./floatresize.js";
import { distNm, bearing } from "./instruments.js";
import { hullRing, bigEnough } from "./hull.js";
import { PRIMARY } from "./role.js";
import { createInfoBox } from "./infobox.js";
import { hullSymbolFeatures, targetSymbolFeatures } from "./ais-symbols.js";
import { s52PxPerMm } from "./chartfeatures.js";

const KEY = "racenav.ais";
const SRC_NAME = { signalk: "Signal K", nmea: "NMEA 0183", aisstream: "aisstream.io", openwaters: "Open Waters" };
const SRC_SHORT = { signalk: "SK", nmea: "NMEA", aisstream: "aisstream", openwaters: "OpenW" };
const DEFAULTS = { show: true, names: true, key: "", owKey: "", owSeed: "", owOn: false, online: false, cpaOn: true, cpa: 0.5, tcpa: 12, ignoreMoored: true, sort: "range", selTrack: false, trkCol: "#c98a00",
  // course vector length (min); CPA graphics: "none" | "danger" (dangerous and selected) | "all" (every target closing within the TCPA limit)
  vecMin: 6, curve: true, dayShapes: true, cpaDraw: "danger",
  // sleeping targets (vessels slower than sleepKn, or further than sleepNm if above 0): "off" | "small" (small symbol, no vector) | "hide"
  sleep: "off", sleepKn: 0.5, sleepNm: 0,
  // internet AIS area: "view" (the chart view) | "centre" (range round the chart centre) | "boat" (range round own boat); range NM
  netMode: "boat", netRange: 30, netAlarms: false,
  // floating target list
  win: { open: false, sort: "cpa", dir: 1, filter: "all", x: null, y: null } };
const FLEET_KEY = "racenav.ownfleet";
export const GROUP_COLOURS = { Friends: "#2e86de", Company: "#e67e22", "Race fleet": "#d63031", Other: "#8e44ad" };
/** One colour for every vessel (the night palettes invert the page, so it follows them); red is only for danger. */
const TARGET_COL = "#1d2a33", TARGET_HALO = "#ffffff", DANGER_COL = "#d00000";
/** A turn faster than this (degrees per minute) is marked on the heading line. */
const ROT_MIN = 3; // °/min: a short barb from here, a long one from ROT_FAST
const ROT_FAST = 10;
const fleetColour = (e) => e.colour || GROUP_COLOURS[e.group] || GROUP_COLOURS.Other;
const isFishing = (t) => t.ship_type === 30 || t.status === 7;
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const STATUS = ["Under way using engine", "At anchor", "Not under command", "Restricted manoeuvrability", "Constrained by draught", "Moored", "Aground",
  "Engaged in fishing", "Under way sailing", "", "", "Towing astern", "Pushing ahead or towing alongside", "", "AIS-SART active", ""];
/** Ship type → [category, colour]. */
function category(t) {
  if (t == null) return ["Unknown", "#56636e"];
  if (t === 30) return ["Fishing", "#e8792e"];
  if (t === 36 || t === 37) return [t === 36 ? "Sailing" : "Pleasure craft", "#9b59b6"];
  if (t === 31 || t === 32 || t === 52) return ["Tug / towing", "#1abc9c"];
  if (t >= 50 && t <= 59) return [{ 50: "Pilot", 51: "Search and rescue", 53: "Port tender", 54: "Anti-pollution", 55: "Law enforcement", 58: "Medical" }[t] ?? "Special craft", "#1abc9c"];
  if (t >= 40 && t <= 49) return ["High-speed craft", "#f1c40f"];
  if (t >= 60 && t <= 69) return ["Passenger", "#2e6fd8"];
  if (t >= 70 && t <= 79) return ["Cargo", "#27ae60"];
  if (t >= 80 && t <= 89) return ["Tanker", "#c0392b"];
  if (t === 33) return ["Dredging", "#1abc9c"];
  if (t === 35) return ["Military", "#5d6d7e"];
  return ["Other", "#56636e"];
}
const ATON = { 1: "Reference point", 2: "RACON", 3: "Fixed structure", 5: "Light", 9: "Beacon, cardinal N", 13: "Beacon, port", 14: "Beacon, starboard",
  20: "Cardinal buoy N", 21: "Cardinal buoy E", 22: "Cardinal buoy S", 23: "Cardinal buoy W", 24: "Port buoy", 25: "Starboard buoy", 28: "Isolated danger buoy", 29: "Safe water buoy", 30: "Special buoy", 31: "Light vessel / LANBY" };

/** Closest point of approach of target `t` to own boat `o` ({pos, cog, sog}): {cpa NM, tcpa min,
 *  range NM, own: [lon, lat] and tgt: [lon, lat] where each will be at the CPA} or null. */
export function cpaTcpa(o, t) {
  if (!o?.pos || t.lat == null) return null;
  const lat0 = (o.pos[1] * Math.PI) / 180;
  const p = [(t.lon - o.pos[0]) * 60 * Math.cos(lat0), (t.lat - o.pos[1]) * 60]; // NM east, north
  const vel = (sog, cog) => (sog && cog != null ? [sog * Math.sin((cog * Math.PI) / 180), sog * Math.cos((cog * Math.PI) / 180)] : [0, 0]);
  const vo = vel(o.sog, o.cog), vt = vel(t.sog, t.cog);
  const v = [vt[0] - vo[0], vt[1] - vo[1]];
  const v2 = v[0] * v[0] + v[1] * v[1];
  const range = Math.hypot(p[0], p[1]);
  if (v2 < 1e-6) return { cpa: range, tcpa: null, range };
  const th = -(p[0] * v[0] + p[1] * v[1]) / v2; // hours
  const c = [p[0] + v[0] * th, p[1] + v[1] * th];
  const toLL = (n) => [o.pos[0] + n[0] / (60 * Math.cos(lat0)), o.pos[1] + n[1] / 60];
  return { cpa: Math.hypot(c[0], c[1]), tcpa: th * 60, range, own: toLL([vo[0] * th, vo[1] * th]), tgt: toLL([p[0] + vt[0] * th, p[1] + vt[1] * th]) };
}

/** A white-on-transparent bitmap → a signed distance field, which is what MapLibre needs to colour
 *  an icon and give it a halo. A plain bitmap marked as SDF has no distances, so its halo comes out
 *  blocky: this gives the halo a smooth edge at any size. */
function toSdf(img) {
  const { width: w, height: h, data } = img;
  const R = 8, CUT = 0.25;
  const a = new Float32Array(w * h);
  for (let i = 0; i < a.length; i++) a[i] = data[i * 4 + 3] / 255;
  const out = new ImageData(w, h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const v = a[y * w + x], inside = v > 0.5;
      let d;
      if (v > 0 && v < 1) d = 0.5 - v; // edge pixel: the coverage says how far the edge is
      else {
        let best = R * R;
        for (let dy = -R; dy <= R; dy++) {
          const yy = y + dy;
          if (yy < 0 || yy >= h) continue;
          for (let dx = -R; dx <= R; dx++) {
            const xx = x + dx;
            if (xx < 0 || xx >= w) continue;
            const o = a[yy * w + xx];
            if ((o > 0.5) === inside && !(o > 0 && o < 1)) continue;
            const dd = dx * dx + dy * dy;
            if (dd < best) best = dd;
          }
        }
        d = Math.sqrt(best) - 0.5;
        if (inside) d = -d;
      }
      const p = Math.max(0, Math.min(255, Math.round(255 - 255 * (d / R + CUT))));
      const i = (y * w + x) * 4;
      out.data[i] = out.data[i + 1] = out.data[i + 2] = 255;
      out.data[i + 3] = p;
    }
  }
  return out;
}

export function addIcons(realMap) {
  // every icon drawn white and coloured by its layer goes through toSdf
  const map = { addImage: (name, img, opts) => realMap.addImage(name, opts?.sdf ? toSdf(img) : img, opts) };
  const make = (draw, size = 32) => {
    const c = document.createElement("canvas");
    c.width = c.height = size;
    const g = c.getContext("2d");
    draw(g, size);
    return g.getImageData(0, 0, size, size);
  };
  const makeWH = (w, h, draw) => {
    const c = document.createElement("canvas");
    c.width = w; c.height = h;
    const g = c.getContext("2d");
    draw(g);
    return g.getImageData(0, 0, w, h);
  };
  // heading line: a fixed length from the target's position (the image is twice the length, so it turns about its centre),
  // with a barb at the end towards the side it is turning to
  const HL = 44;
  // a short barb for a moderate turn, a long one for a fast turn
  for (const [k, side, len] of [["", 0, 0], ["-s", 1, 7], ["-p", -1, 7], ["-s2", 1, 14], ["-p2", -1, 14]]) {
    map.addImage(`ais-hdg${k}`, makeWH(32, HL * 2, (g) => {
      g.strokeStyle = "#fff"; g.lineWidth = 2;
      g.beginPath(); g.moveTo(16, HL); g.lineTo(16, 3);
      if (side) { g.moveTo(16, 3); g.lineTo(16 + side * len, 3); }
      g.stroke();
    }), { sdf: true });
  }
  // selected target: four corner brackets, drawn in the symbol colour with the same halo
  map.addImage("ais-sel", make((g, s) => {
    g.strokeStyle = "#fff"; g.lineWidth = 2; g.lineCap = "butt"; g.lineJoin = "miter";
    const a = 8, b = s - 8, l = 8;
    g.beginPath();
    for (const [x, y, dx, dy] of [[a, a, 1, 1], [b, a, -1, 1], [a, b, 1, -1], [b, b, -1, -1]]) {
      g.moveTo(x + dx * l, y); g.lineTo(x, y); g.lineTo(x, y + dy * l);
    }
    g.stroke();
  }, 48), { sdf: true });
}

/** Navigational status → the day shape a vessel shows for it (COLREGs), or a sign where
 *  there is none (moored: a bollard). Drawn beside the target. */
const STATUS_SHORT = { 1: "anchored", 2: "NUC", 3: "RAM", 4: "CBD", 5: "moored", 6: "aground", 7: "fishing", 11: "towing", 12: "towing" };
const STATUS_SHAPE = { 1: "ball", 2: "nuc", 3: "ram", 4: "cbd", 5: "moored", 6: "aground", 7: "fishing", 11: "tow", 12: "tow" };
export function addStatusIcons(map) {
  const R = 2; // pixel ratio: drawn at twice the size for a sharp edge
  const shapes = {
    ball: ["ball"], nuc: ["ball", "ball"], ram: ["ball", "diamond", "ball"], cbd: ["cyl"], aground: ["ball", "ball", "ball"],
    fishing: ["cones"], tow: ["diamond"], moored: ["bollard"],
  };
  const H = { ball: 9, diamond: 11, cyl: 13, cones: 15, bollard: 12 };
  for (const [k, parts] of Object.entries(shapes)) {
    const w = 14, h = parts.reduce((a, p) => a + H[p], 0) + (parts.length - 1) * 2 + 6;
    const c = document.createElement("canvas");
    c.width = w * R; c.height = h * R;
    const g = c.getContext("2d");
    g.scale(R, R);
    // white backing with a dark edge, so it shows on any chart colour
    g.fillStyle = "#fff"; g.strokeStyle = "#1d2a33"; g.lineWidth = 1;
    g.beginPath(); g.roundRect(0.5, 0.5, w - 1, h - 1, 3); g.fill(); g.stroke();
    g.fillStyle = g.strokeStyle = "#000";
    let y = 3;
    const cx = w / 2;
    for (const p of parts) {
      const ph = H[p];
      g.beginPath();
      if (p === "ball") g.arc(cx, y + ph / 2, ph / 2 - 0.5, 0, 2 * Math.PI);
      else if (p === "diamond") { g.moveTo(cx, y); g.lineTo(cx + 4.5, y + ph / 2); g.lineTo(cx, y + ph); g.lineTo(cx - 4.5, y + ph / 2); g.closePath(); }
      else if (p === "cyl") g.rect(cx - 3.5, y, 7, ph);
      else if (p === "cones") { g.moveTo(cx - 4.5, y); g.lineTo(cx + 4.5, y); g.lineTo(cx, y + ph / 2); g.closePath(); g.moveTo(cx - 4.5, y + ph); g.lineTo(cx + 4.5, y + ph); g.lineTo(cx, y + ph / 2); g.closePath(); }
      else if (p === "bollard") { g.rect(cx - 2.5, y + 3, 5, ph - 3); g.rect(cx - 4.5, y + ph - 2, 9, 2); g.ellipse(cx, y + 3, 4, 2.5, 0, 0, 2 * Math.PI); }
      g.fill();
      y += ph + 2;
    }
    map.addImage(`ais-st-${k}`, g.getImageData(0, 0, c.width, c.height), { pixelRatio: R });
  }
  // lost target (IEC 62288): the symbol crossed through, at the last position
  const s = 32, c = document.createElement("canvas");
  c.width = c.height = s * R;
  const g = c.getContext("2d");
  g.scale(R, R);
  g.lineCap = "round";
  for (const [col, wd] of [["#fff", 4], ["#000", 1.8]]) {
    g.strokeStyle = col; g.lineWidth = wd;
    g.beginPath(); g.moveTo(4, 4); g.lineTo(s - 4, s - 4); g.moveTo(s - 4, 4); g.lineTo(4, s - 4); g.stroke();
  }
  map.addImage("ais-lostx", g.getImageData(0, 0, c.width, c.height), { pixelRatio: R });
}

export function setupAis(map, { getOwn, alarms, ui, getOwnMmsi }) {
  let s = { ...DEFAULTS };
  try { Object.assign(s, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  s.win = { ...DEFAULTS.win, ...(s.win || {}) };
  // an earlier version had one source chosen from a menu: that one's switch moves to its own
  if (s.provider === "openwaters") { s.owOn = !!s.online; s.online = false; }
  delete s.provider;
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ } };
  const targets = new Map(); // mmsi → target
  const online = { aisstream: { connected: false, detail: "off" }, openwaters: { connected: false, detail: "off" } };
  let selected = null;
  let fleet = { entries: [], trackAll: false, only: false, watch: [] };
  try { Object.assign(fleet, JSON.parse(localStorage.getItem(FLEET_KEY) || "{}")); } catch { /* storage unavailable */ }
  // tracking is separate from the fleet: older saves kept a track flag on each fleet entry
  for (const e of fleet.entries) if (e.track) { fleet.watch.push(Number(e.mmsi)); delete e.track; }
  const isWatched = (mmsi) => fleet.watch.includes(Number(mmsi));
  const setWatched = (mmsi, on) => {
    fleet.watch = fleet.watch.filter((m) => m !== Number(mmsi));
    if (on) fleet.watch.push(Number(mmsi));
  };
  const saveFleet = () => { try { localStorage.setItem(FLEET_KEY, JSON.stringify(fleet)); } catch { /* ignore */ } };
  const inFleet = (mmsi) => fleet.entries.find((e) => Number(e.mmsi) === Number(mmsi));
  let ownMmsi = Number(getOwnMmsi?.() || 0) || null;
  const tracks = new Map(); // mmsi → [[t, lon, lat]]

  addIcons(map);
  addStatusIcons(map);
  map.addSource("ais-tracks", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "ais-track", type: "line", source: "ais-tracks", paint: { "line-color": ["get", "col"], "line-width": 1.8, "line-opacity": 0.85 } });
  // CPA graphics: where own boat and the target will be at the closest point of approach
  map.addSource("ais-cpa", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "ais-cpa-path", type: "line", source: "ais-cpa", filter: ["==", ["get", "k"], "path"],
    paint: { "line-color": ["get", "col"], "line-width": 1.4, "line-dasharray": [3, 2], "line-opacity": 0.9 } });
  map.addLayer({ id: "ais-cpa-line", type: "line", source: "ais-cpa", filter: ["==", ["get", "k"], "cpa"],
    paint: { "line-color": ["get", "col"], "line-width": 2.5 } });
  map.addLayer({ id: "ais-cpa-pt", type: "circle", source: "ais-cpa", filter: ["==", ["get", "k"], "pt"],
    paint: { "circle-radius": 4.5, "circle-color": "#fff", "circle-stroke-color": ["get", "col"], "circle-stroke-width": 2 } });
  map.addLayer({ id: "ais-cpa-lbl", type: "symbol", source: "ais-cpa", filter: ["==", ["get", "k"], "lbl"],
    layout: { "icon-image": ["concat", "lbl:", ["get", "text"]], "icon-anchor": "left", "icon-offset": [8, 0], "icon-allow-overlap": true } });
  map.addSource("ais", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "ais-shapes", type: "fill", source: "ais", filter: ["==", ["get", "k"], "shape"],
    layout: { "fill-sort-key": ["get", "z"] },
    paint: { "fill-color": ["get", "col"], "fill-opacity": ["get", "opacity"] } });
  // heading line (with a barb when turning), then the course vector
  map.addLayer({ id: "ais-hdg", type: "symbol", source: "ais", filter: ["==", ["get", "k"], "hdg"],
    layout: { "icon-image": ["get", "icon"], "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map", "icon-allow-overlap": true, "icon-ignore-placement": true },
    paint: { "icon-color": ["get", "col"], "icon-halo-color": ["get", "halo"], "icon-halo-width": 0.8 } });
  map.addLayer({ id: "ais-vec", type: "line", source: "ais", filter: ["==", ["get", "k"], "vec"],
    paint: { "line-color": ["get", "col"], "line-width": 1.5 } });
  map.addLayer({ id: "ais-sel", type: "symbol", source: "ais", filter: ["all", ["==", ["get", "k"], "t"], ["==", ["get", "sel"], true]],
    layout: { "icon-image": "ais-sel", "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-rotation-alignment": "viewport",
      "icon-size": ["interpolate", ["linear"], ["zoom"], 6, 0.7, 12, 1.1] },
    paint: { "icon-color": TARGET_COL, "icon-halo-color": TARGET_HALO, "icon-halo-width": 1.2 } });
  // not heard for a while: crossed through (IEC 62288 lost target), no course vector
  map.addLayer({ id: "ais-lost", type: "symbol", source: "ais", filter: ["all", ["==", ["get", "k"], "t"], ["get", "lost"]],
    layout: { "icon-image": "ais-lostx", "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-rotation-alignment": "viewport",
      "icon-size": ["interpolate", ["linear"], ["zoom"], 6, 0.6, 12, 0.96] } });
  // navigational status: the day shape (anchored, not under command, restricted, fishing...) beside it
  map.addLayer({ id: "ais-status", type: "symbol", source: "ais", filter: ["all", ["==", ["get", "k"], "t"], ["has", "st"]], minzoom: 8,
    layout: { "icon-image": ["concat", "ais-st-", ["get", "st"]], "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-rotation-alignment": "viewport",
      "icon-anchor": "bottom-left", "icon-offset": [9, -6] } });
  map.addLayer({ id: "ais-fleet-names", type: "symbol", source: "ais", filter: ["all", ["==", ["get", "k"], "t"], ["has", "flag"], ["has", "name"]],
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 17], "icon-allow-overlap": true } });
  map.addLayer({ id: "ais-names", type: "symbol", source: "ais", filter: ["all", ["==", ["get", "k"], "t"], ["has", "name"], ["!", ["has", "flag"]]], minzoom: 10,
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 14], "icon-allow-overlap": false } });

  const isLost = (t) => {
    if (t.last_pos == null) return true;
    const age = Date.now() / 1000 - t.last_pos;
    const moving = (t.sog ?? 0) > 2;
    // a distress transmitter sends every minute: lost after six minutes (as its alarm, below)
    if (["sart", "mob", "epirb"].includes(t.kind)) return age > 6 * 60;
    return t.kind === "aton" || t.kind === "base" ? age > 20 * 60 : age > (moving ? 3 : 10) * 60;
  };
  /** Heard only over the internet (no receiver on board heard it). */
  const netOnly = (t) => !/signalk|nmea/.test(String(t.source)); // heard on board: Signal K or NMEA 0183
  /** Centre of the internet AIS area when it is a range (chart centre or own boat), else null. */
  function netCentre() {
    if (s.netMode === "boat") return getOwn()?.pos ?? null;
    if (s.netMode === "centre") { const c = map.getCenter(); return [c.lng, c.lat]; }
    return null;
  }
  /** Whether a target is drawn and listed: not own boat, own fleet only if asked, internet ones inside the range. */
  function shown(t) {
    if (t.lat == null || t.mmsi === ownMmsi) return false;
    if (fleet.only && !inFleet(t.mmsi)) return false;
    if (netOnly(t)) {
      const c = netCentre();
      if (c && distNm(c, [t.lon, t.lat]) > s.netRange) return false;
    }
    return true;
  }
  /** Closing inside the alarm limits. */
  const isDanger = (c) => !!(c && c.tcpa != null && c.tcpa > 0 && c.tcpa < s.tcpa && c.cpa < s.cpa);
  /** A vessel too slow (or, if set, too far) to matter is "sleeping": shown small and without vectors, or hidden.
   *  Dangerous, selected and own-fleet targets are never asleep. */
  function isSleeping(t, c, fe) {
    if (s.sleep === "off" || t.kind !== "vessel" || fe || t.mmsi === selected || isDanger(c)) return false;
    if ((t.sog ?? 0) < s.sleepKn) return true;
    return s.sleepNm > 0 && c != null && c.range > s.sleepNm;
  }
  /** [icon, colour (null: the standard one), rotation]. */
  function style(t, sleeping) {
    if (t.kind === "sart" || t.kind === "mob" || t.kind === "epirb") return ["ais-sart", "#d00000", 0];
    if (t.kind === "aton") return ["ais-aton", t.virtual_aton ? "#c2185b" : "#6a1b9a", 0];
    if (t.kind === "aircraft") return ["ais-air", "#e67e22", t.cog ?? 0];
    if (t.kind === "base") return ["ais-base", "#5d6d7e", 0];
    const fish = isFishing(t);
    if (sleeping) return [fish ? "ais-sleep-fish" : "ais-sleep", null, 0];
    const dir = t.heading ?? ((t.sog ?? 0) > 0.5 ? t.cog : null);
    if (dir == null) return [fish ? "ais-still-fish" : "ais-still", null, 0];
    const cls = t.class === "B" ? "b" : t.class === "B+" ? "bp" : "a";
    return [`ais-${fish ? "f" : "s"}-${cls}`, null, dir];
  }
  // Night (IHO) draws the chart unfiltered, so there the symbol is a dim grey with a black outline instead
  // of dark with a white one (the other night colours turn the page round, symbols included)
  let mapCol = TARGET_COL, mapHalo = TARGET_HALO;
  const rootEl = document.documentElement;
  function mapTheme() {
    const raw = rootEl.dataset.palette === "iho-night" && rootEl.hasAttribute("data-iho");
    const col = raw ? "#575757" : TARGET_COL, halo = raw ? "#000000" : TARGET_HALO;
    if (col === mapCol && halo === mapHalo) return;
    mapCol = col; mapHalo = halo;
    map.setPaintProperty("ais-sel", "icon-color", mapCol);
    map.setPaintProperty("ais-sel", "icon-halo-color", mapHalo);
    draw();
  }
  new MutationObserver(mapTheme).observe(rootEl, { attributes: true, attributeFilter: ["data-palette", "data-iho"] });
  mapTheme();
  function draw() {
    const vis = s.show ? "visible" : "none";
    for (const id of ["ais-shapes", "ais-hdg", "ais-vec", "ais-sel", "ais-lost", "ais-status", "ais-fleet-names", "ais-track", "ais-cpa-path", "ais-cpa-line", "ais-cpa-pt", "ais-cpa-lbl"]) map.setLayoutProperty(id, "visibility", vis);
    map.setLayoutProperty("ais-names", "visibility", s.show && s.names ? "visible" : "none");
    const f = [];
    const cpaF = [];
    const own = getOwn();
    let shapeSortKey = 0;
    for (const t of targets.values()) {
      if (!shown(t)) continue;
      const fe = inFleet(t.mmsi);
      const lost = isLost(t);
      const c = t.kind === "vessel" && own?.pos ? cpaTcpa(own, t) : null;
      const danger = !lost && isDanger(c);
      const sleeping = isSleeping(t, c, fe);
      if (sleeping && s.sleep === "hide") continue;
      const [icon, ownCol, rot] = style(t, sleeping);
      const col = danger ? DANGER_COL : ownCol ?? mapCol;
      const props = { k: "t", mmsi: t.mmsi, icon, col, halo: mapHalo, hw: danger ? 3.2 : 2.2, rot, lost, sel: t.mmsi === selected };
      if (s.dayShapes && t.kind === "vessel" && STATUS_SHAPE[t.status]) props.st = STATUS_SHAPE[t.status];
      // to scale when zoomed in far enough, if it has sent its size and we know which way it points
      const dir = t.heading ?? ((t.sog ?? 0) > 1 ? t.cog : null);
      if (t.kind === "vessel" && t.length > 0 && dir != null && !sleeping && bigEnough(t.length, t.lat, map.getZoom())) {
        props.big = true;
        const ring = hullRing([t.lon, t.lat], dir, { length: t.length, beam: t.beam, fromBow: t.from_bow, fromCentre: t.from_centre });
        f.push(...hullSymbolFeatures({ ring, color: col, outline: mapHalo, zoom: map.getZoom(), length: t.length, mmsi: t.mmsi, sortKey: shapeSortKey }));
        shapeSortKey += 5;
      }
      const name = fe?.name || t.name;
      if (name && (fe || !sleeping)) props.name = name;
      if (fe) {
        props.flag = fleetColour(fe);
      }
      if (!props.big) {
        f.push(...targetSymbolFeatures({ position: [t.lon, t.lat], heading: rot, icon, color: col, outline: mapHalo, zoom: map.getZoom(), pxPerMm: s52PxPerMm(),
          mmsi: t.mmsi, sortKey: shapeSortKey, fleetColor: fe && /^ais-[sf]-/.test(icon) ? fleetColour(fe) : null }));
        shapeSortKey += 5;
      }
      f.push({ type: "Feature", properties: props, geometry: { type: "Point", coordinates: [t.lon, t.lat] } });
      // The course vector takes over from the heading line while it is drawn (the symbol itself points along the heading, and a
      // curved vector shows the turn). The line is kept for a ship with no vector, and for a turning ship if the vector is straight.
      const hasVec = !lost && !sleeping && (t.sog ?? 0) > 0.5 && t.cog != null && t.kind !== "aton";
      const turning = Math.abs(t.rot ?? 0) >= ROT_MIN;
      const astern = hasVec && t.heading != null && Math.abs(((t.cog - t.heading + 540) % 360) - 180) > 90;
      if (t.kind === "vessel" && !lost && !sleeping && t.heading != null && !astern && (!hasVec || (turning && !s.curve))) {
        const turn = t.rot ?? 0;
        const a = Math.abs(turn);
        const hdg = a < ROT_MIN ? "ais-hdg" : `ais-hdg-${turn > 0 ? "s" : "p"}${a >= ROT_FAST ? "2" : ""}`;
        f.push({ type: "Feature", properties: { k: "hdg", icon: hdg, rot: t.heading, col, halo: mapHalo }, geometry: { type: "Point", coordinates: [t.lon, t.lat] } });
      }
      // course vector (6 minutes unless set otherwise)
      if (hasVec) {
        const mins = s.vecMin || 6;
        const coslat = Math.cos((t.lat * Math.PI) / 180);
        // a steady turn at the reported rate of turn bends the vector into an arc
        const curved = s.curve && Math.abs(t.rot ?? 0) >= ROT_MIN;
        const n = curved ? 12 : 1;
        const stepNm = (t.sog * mins / 60) / n;
        const pts = [[t.lon, t.lat]];
        let [x, y] = pts[0];
        for (let i = 0; i < n; i++) {
          const b = ((t.cog + (curved ? t.rot * mins * (i + 0.5) / n : 0)) * Math.PI) / 180;
          x += (stepNm / 60) * Math.sin(b) / coslat;
          y += (stepNm / 60) * Math.cos(b);
          pts.push([x, y]);
        }
        f.push({ type: "Feature", properties: { k: "vec", col }, geometry: { type: "LineString", coordinates: pts } });
      }
      // the CPA: dashed tracks of both to where they will be, joined by the CPA line
      if (s.cpaDraw !== "none" && own?.pos && !lost && t.kind === "vessel") {
        const want = t.mmsi === selected || danger || (s.cpaDraw === "all" && c?.tcpa > 0 && c.tcpa < s.tcpa);
        if (want && c?.tcpa > 0 && c.tcpa <= 120) {
          const ccol = danger ? "#d00000" : "#e08a00";
          const line = (k, a, b) => cpaF.push({ type: "Feature", properties: { k, col: ccol }, geometry: { type: "LineString", coordinates: [a, b] } });
          line("path", [t.lon, t.lat], c.tgt);
          line("path", own.pos, c.own);
          line("cpa", c.own, c.tgt);
          for (const pt of [c.own, c.tgt]) cpaF.push({ type: "Feature", properties: { k: "pt", col: ccol }, geometry: { type: "Point", coordinates: pt } });
          const mid = [(c.own[0] + c.tgt[0]) / 2, (c.own[1] + c.tgt[1]) / 2];
          cpaF.push({ type: "Feature", properties: { k: "lbl", text: `CPA ${c.cpa.toFixed(2)} NM · ${Math.round(c.tcpa)} min` }, geometry: { type: "Point", coordinates: mid } });
        }
      }
    }
    map.getSource("ais").setData({ type: "FeatureCollection", features: f });
    map.getSource("ais-cpa").setData({ type: "FeatureCollection", features: cpaF });
    drawTracks();
  }

  // ---------- tracks ----------------------------------------------------------------
  /** MMSIs whose track is drawn: own-fleet boats ticked (or all), and the selected target. */
  function tracked() {
    const set = new Set(fleet.watch);
    if (fleet.trackAll) for (const e of fleet.entries) set.add(Number(e.mmsi));
    if (s.selTrack && selected != null) set.add(selected);
    return [...set];
  }
  function drawTracks() {
    const f = [];
    for (const m of tracked()) {
      const tr = tracks.get(m);
      const t = targets.get(m);
      if (!tr || tr.length < 1) continue;
      const pts = tr.map((p) => [p[1], p[2]]);
      if (t?.lat != null) pts.push([t.lon, t.lat]);
      if (pts.length < 2) continue;
      const fe = inFleet(m);
      f.push({ type: "Feature", properties: { col: fe ? fleetColour(fe) : s.trkCol }, geometry: { type: "LineString", coordinates: pts } });
    }
    map.getSource("ais-tracks").setData({ type: "FeatureCollection", features: f });
  }
  async function loadTracks() {
    const ms = tracked();
    if (!ms.length) { tracks.clear(); draw(); return; }
    try {
      const r = await invoke("ais_tracks", { mmsis: ms });
      tracks.clear();
      for (const [m, tr] of Object.entries(r || {})) tracks.set(Number(m), tr);
    } catch { /* not in Tauri */ }
    draw();
  }
  setInterval(loadTracks, 10000);

  // ---------- info box ----------------------------------------------------------------
  function describe(t) {
    const own = getOwn();
    const c = cpaTcpa(own, t);
    const rows = [];
    const add = (k, v) => { if (v != null && v !== "") rows.push(`<tr><th>${esc(k)}</th><td>${esc(v)}</td></tr>`); };
    const kindName = { vessel: t.ship_type != null ? category(t.ship_type)[0] : "Vessel", aton: `Aid to navigation${t.virtual_aton ? " (virtual: nothing physical there)" : ""}`, aircraft: "SAR aircraft",
      base: "AIS base station", sart: "AIS-SART (survival craft transmitter)", mob: "MOB device (person in the water)", epirb: "EPIRB" }[t.kind];
    add("Type", kindName + (t.class ? ` · class ${t.class}` : ""));
    add("MMSI", String(t.mmsi).padStart(9, "0"));
    add("Call sign", t.callsign);
    if (t.kind === "aton") add("Aid", ATON[t.aton_type] ?? (t.aton_type != null ? `type ${t.aton_type}` : null));
    if (t.off_position) add("Warning", "OFF POSITION");
    if (["sart", "mob", "epirb"].includes(t.kind)) {
      // active or test: what an operator must know first about a distress transmitter
      add("Mode", distressState(t));
      add("Navigational status", t.status != null ? `${t.status}${t.status === 14 ? " (active)" : t.status === 15 ? " (test, or not defined)" : ""}` : "not received");
      add("Safety message", t.safety_text);
    } else add("Status", t.status != null ? (STATUS[t.status] || `Not defined (${t.status})`) : null);
    if (t.sog != null || t.cog != null) add("SOG / COG", `${t.sog != null ? t.sog.toFixed(1) + " kn" : "—"} / ${t.cog != null ? Math.round(t.cog) + "°T" : "—"}`);
    if (t.heading != null) add("Heading", `${Math.round(t.heading)}°T`);
    if (t.rot != null && Math.abs(t.rot) >= 0.1) add("Rate of turn", `${Math.abs(t.rot).toFixed(1)}°/min to ${t.rot > 0 ? "starboard" : "port"}`);
    if (t.length) add("Size", `${t.length} × ${t.beam ?? "?"} m${t.draught ? `, draught ${t.draught} m` : ""}`);
    add("Destination", t.destination);
    if (t.altitude != null) add("Altitude", `${t.altitude} m`);
    add("Position", `${ddm(t.lat, "N", "S")} ${ddm(t.lon, "E", "W")}`);
    if (c) {
      add("Range / bearing", `${c.range.toFixed(2)} NM / ${String(Math.round(bearing(own.pos, [t.lon, t.lat]))).padStart(3, "0")}°T`);
      if (c.tcpa != null) add("CPA / TCPA", c.tcpa > 0 ? `${c.cpa.toFixed(2)} NM in ${Math.round(c.tcpa)} min` : "Moving apart");
    }
    if (t.last_pos) add("Position age", `${Math.round(Date.now() / 1000 - t.last_pos)} s`);
    add("Heard by", t.source.split("+").map((x) => SRC_NAME[x] || x).join(" and ") + (netOnly(t) ? " (internet: may be late or missing, not for navigation)" : ""));
    const fe = inFleet(t.mmsi);
    const trk = isWatched(t.mmsi) || (fe && fleet.trackAll);
    const btns = `<div class="ais-pop-btns">${fe ? `<span class="wx-note">Own fleet: ${esc(fe.group)}</span>` : ""}<button data-act="track">${trk ? "Stop tracking" : "Track"}</button>${fe ? `<button data-act="unfleet">Remove from own fleet</button>` : `<button data-act="fleet">Add to own fleet</button>`}</div>`;
    return `<div class="ais-pop">${fe?.name && t.name && fe.name !== t.name ? `<small>AIS name: ${esc(t.name)}</small>` : ""}<table>${rows.join("")}</table>${btns}</div>`;
  }
  const infoWin = createInfoBox(map, "ais", { width: 320, height: 380, showRing: false });
  const boxTitle = (t) => inFleet(t.mmsi)?.name || t.name || `MMSI ${t.mmsi}`;
  function openPopup(mmsi) {
    const t = targets.get(mmsi);
    if (!t || t.lat == null) return;
    selected = mmsi;
    infoWin.open(describe(t), { title: boxTitle(t), at: [t.lon, t.lat], onClose: () => { if (selected === mmsi) { selected = null; draw(); loadTracks(); } } });
    wirePopup(mmsi);
    draw();
    loadTracks();
  }
  function wirePopup(mmsi) {
    const el = infoWin.isOpen ? infoWin.body : null;
    if (!el) return;
    el.querySelector('[data-act="fleet"]')?.addEventListener("click", () => {
      const t = targets.get(mmsi);
      fleet.entries.push({ mmsi: String(mmsi).padStart(9, "0"), name: t?.name ?? "", group: "Friends" });
      saveFleet(); renderFleet(); renderList(); refreshPopup(); draw();
    });
    el.querySelector('[data-act="unfleet"]')?.addEventListener("click", () => {
      fleet.entries = fleet.entries.filter((e) => Number(e.mmsi) !== mmsi);
      saveFleet(); renderFleet(); renderList(); refreshPopup(); draw();
    });
    el.querySelector('[data-act="track"]')?.addEventListener("click", () => {
      const on = !(isWatched(mmsi) || (inFleet(mmsi) && fleet.trackAll));
      if (!on && fleet.trackAll) { fleet.trackAll = false; $("of-track-all").checked = false; }
      setWatched(mmsi, on);
      saveFleet(); renderFleet(); renderList(); refreshPopup(); loadTracks();
    });
  }
  function refreshPopup() {
    const t = selected != null ? targets.get(selected) : null;
    if (infoWin.isOpen && t) { infoWin.update(describe(t), { title: boxTitle(t), at: t.lat != null ? [t.lon, t.lat] : null }); wirePopup(selected); }
  }
  for (const l of ["ais-shapes"]) {
    map.on("click", l, (e) => { const f = e.features?.[0]; if (f) openPopup(f.properties.mmsi); });
    map.on("mouseenter", l, () => { map.getCanvas().style.cursor = "pointer"; });
    map.on("mouseleave", l, () => { map.getCanvas().style.cursor = ""; });
  }
  // symbols or outlines to scale, by zoom
  map.on("zoomend", () => draw());
  window.addEventListener("racenav-screen-calibration", () => draw());

  // ---------- list ----------------------------------------------------------------
  /** A drop-down or text/colour field in `el` is in use: rebuilding the rows now (every AIS update)
   *  would close it under the user's finger. `again` renders once it is left or changed. */
  function inUse(el, again) {
    const a = document.activeElement;
    if (!el || !a || !el.contains(a)) return false;
    if (!(a.tagName === "SELECT" || (a.tagName === "INPUT" && /^(color|text|number|search)$/.test(a.type)))) return false;
    if (!a.dataset.waiting) {
      a.dataset.waiting = "1";
      const done = () => { a.removeEventListener("blur", done); a.removeEventListener("change", leave); delete a.dataset.waiting; setTimeout(again, 0); };
      const leave = () => a.blur(); // a choice made: let go of it so the list updates again
      a.addEventListener("blur", done);
      a.addEventListener("change", leave);
    }
    return true;
  }
  function renderList() {
    const ul = $("ais-list");
    if ($("ais-list").closest(".pane")?.hidden || inUse(ul, renderList)) return;
    const own = getOwn();
    const rows = [...targets.values()].filter(shown).map((t) => ({ t, c: cpaTcpa(own, t) })).filter(({ t, c }) => !(s.sleep === "hide" && isSleeping(t, c, inFleet(t.mmsi))));
    const key = { range: (r) => r.c?.range ?? 1e9, cpa: (r) => (r.c?.tcpa > 0 ? r.c.cpa : 1e9), name: (r) => (r.t.name ?? "~").toLowerCase() }[s.sort];
    rows.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0));
    $("ais-count").textContent = rows.length ? `(${rows.length})` : "";
    ul.innerHTML = rows.length ? "" : `<li class="empty">No targets. Signal K passes on what a receiver on board hears; online AIS needs a key.</li>`;
    for (const { t, c } of rows.slice(0, 150)) {
      const li = document.createElement("li");
      const danger = c && c.tcpa > 0 && c.cpa < s.cpa && c.tcpa < s.tcpa;
      li.className = danger ? "danger" : "";
      li.innerHTML = `<div class="main"><b></b><small></small></div>
        <label class="of-trk" title="Draw this target's track and mark it with a pin"><input type="checkbox" data-act="trk" /> Track</label>
        <select data-act="grp" title="Own fleet group (none: not in your fleet)"><option value="">No group</option>${Object.keys(GROUP_COLOURS).map((g) => `<option>${g}</option>`).join("")}</select>`;
      const trkCb = li.querySelector('[data-act="trk"]'), grp = li.querySelector('[data-act="grp"]');
      trkCb.checked = isWatched(t.mmsi) || (fleet.trackAll && !!inFleet(t.mmsi));
      trkCb.disabled = fleet.trackAll && !!inFleet(t.mmsi);
      grp.value = inFleet(t.mmsi)?.group ?? "";
      trkCb.onchange = () => { setWatched(t.mmsi, trkCb.checked); saveFleet(); renderFleet(); refreshPopup(); loadTracks(); };
      grp.onchange = () => {
        const e = inFleet(t.mmsi);
        if (!grp.value) fleet.entries = fleet.entries.filter((x) => Number(x.mmsi) !== Number(t.mmsi));
        else if (e) e.group = grp.value;
        else fleet.entries.push({ mmsi: String(t.mmsi).padStart(9, "0"), name: t.name ?? "", group: grp.value });
        saveFleet(); renderFleet(); renderList(); refreshPopup(); draw();
      };
      li.querySelector("b").textContent = inFleet(t.mmsi)?.name || t.name || `MMSI ${t.mmsi}`;
      const [cat] = t.kind === "vessel" ? category(t.ship_type) : [{ aton: "AtoN", aircraft: "SAR aircraft", base: "Base station", sart: "AIS-SART", mob: "MOB", epirb: "EPIRB" }[t.kind]];
      li.querySelector("small").textContent = [cat, t.sog != null ? `${t.sog.toFixed(1)} kn` : null, c ? `${c.range.toFixed(1)} NM` : null,
        c && c.tcpa > 0 ? `CPA ${c.cpa.toFixed(2)} NM in ${Math.round(c.tcpa)} min` : null, isLost(t) ? "lost" : null, netOnly(t) ? "internet" : null].filter(Boolean).join(" · ");
      li.querySelector(".main").onclick = () => { map.easeTo({ center: [t.lon, t.lat], zoom: Math.max(map.getZoom(), 12) }); openPopup(t.mmsi); };
      ul.append(li);
    }
  }

  // ---------- alarms ----------------------------------------------------------------
  const raised = new Set();
  // A distress transmitter (AIS-SART, MOB device, EPIRB) sends a burst every minute while active.
  // Not heard for DISTRESS_LOST_MIN minutes it is lost: its alarm becomes a warning that can be
  // cleared. A test transmission (navigational status 15) is a caution only.
  const DISTRESS_LOST_MIN = 6;
  const distressLost = (t) => {
    const heard = t.last_pos ?? t.last_seen;
    return heard == null || Date.now() / 1000 - heard > DISTRESS_LOST_MIN * 60;
  };
  // Active or test (IEC 61097-14 and the like): an active transmitter reports navigational
  // status 14 and broadcasts "SART ACTIVE" (MOB, EPIRB likewise); in test, status 15 and "… TEST".
  const isTest = (t) => /\bTEST\b/i.test(String(t.safety_text ?? "")) || (t.status === 15 && !/ACTIVE/i.test(String(t.safety_text ?? ""))) || /\bTEST\b/i.test(String(t.name ?? ""));
  const clearedDistress = new Map(); // alarm id → time last heard when the user cleared it
  /** What a distress transmitter says about itself: active, test, or not known. */
  function distressState(t) {
    const txt = t.safety_text ? ` ("${t.safety_text}")` : "";
    if (isTest(t)) return `TEST${txt}`;
    if (t.status === 14 || /ACTIVE/i.test(String(t.safety_text ?? ""))) return `ACTIVE${txt}`;
    return "active or test not known (no status or safety message received)";
  }
  function checkAlarms() {
    const own = getOwn();
    const now = new Set();
    for (const t of targets.values()) {
      if (t.lat == null || t.mmsi === ownMmsi) continue;
      const id = `ais-${t.mmsi}`;
      if (["sart", "mob", "epirb"].includes(t.kind)) {
        const what = { sart: "AIS-SART", mob: "MOB DEVICE", epirb: "AIS EPIRB" }[t.kind];
        const heard = t.last_pos ?? t.last_seen ?? null;
        // cleared by the user while lost or in test: stays cleared until it is heard again
        if (clearedDistress.has(id) && (heard == null || heard <= clearedDistress.get(id))) continue;
        clearedDistress.delete(id);
        const c = own?.pos ? cpaTcpa(own, t) : null;
        const where = c ? ` · ${String(Math.round(bearing(own.pos, [t.lon, t.lat]))).padStart(3, "0")}°T ${c.range.toFixed(1)} NM` : "";
        const name = `${what} ${t.name ?? t.mmsi}`;
        const show = () => { map.easeTo({ center: [t.lon, t.lat], zoom: Math.max(map.getZoom(), 12) }); openPopup(t.mmsi); };
        const pos = `${ddm(t.lat, "N", "S")} ${ddm(t.lon, "E", "W")}`;
        const forget = () => clearedDistress.set(id, heard ?? Date.now() / 1000);
        const ago = heard != null ? Math.round((Date.now() / 1000 - heard) / 60) : null;
        const net = netOnly(t);
        if (isTest(t)) {
          // a test transmission (navigational status 15, "SART TEST"): a caution, not a distress alarm
          alarms.raise(id, { level: "warn", stage: 0, watchdog: false, clearable: true, onClear: forget, text: `${name} TEST transmission${where}`,
            detail: `${pos} · a transmitter being tested, not a distress · click to show`, onClick: show });
        } else if (net) {
          // heard only over the internet (late, patchy, maybe far away): a warning, not a distress
          // alarm for your own receiver to act on; it clears like a lost one
          alarms.raise(id, { level: "warn", stage: 0, watchdog: false, clearable: true, onClear: forget,
            text: `${name} (internet)${where}${ago != null && ago >= 1 ? ` · ${ago} min old` : ""}`,
            detail: `${pos} · ${distressState(t)} · heard over the internet only, not by your receiver · click to show`, onClick: show });
        } else if (distressLost(t)) {
          // not heard for a while: a warning that can be cleared (it comes back if heard again)
          alarms.raise(id, { level: "warn", stage: 0, watchdog: false, clearable: true, onClear: forget,
            text: `${name} not heard for ${ago ?? "?"} min`, detail: `last at ${pos} · Clear removes the alarm; it sounds again if heard again · click to show`, onClick: show });
        } else {
          // heard: an alarm that can be silenced but not cleared while the transmitter is heard;
          // the watchdog (Alarms › Sound) sounds it again while it lasts
          // (an alarm that was a clearable warning loses its Clear button again)
          alarms.raise(id, { level: "alarm", stage: 1, watchdog: true, clearable: false, dismiss: null, dismissLabel: null, dismissTitle: null, onClear: null, text: `${name}${where}`,
            detail: `${pos} · ${distressState(t)} · stays while it is heard; once not heard for ${DISTRESS_LOST_MIN} min it can be cleared · click to show`, onClick: show });
        }
        now.add(id);
        continue;
      }
      if (!s.cpaOn || !own?.live || t.kind !== "vessel" || isLost(t) || !shown(t)) continue;
      // internet AIS is late and patchy: no collision alarms from it unless asked
      if (netOnly(t) && !s.netAlarms) continue;
      if (s.ignoreMoored && (t.status === 1 || t.status === 5)) continue;
      const c = cpaTcpa(own, t);
      if (isDanger(c)) {
        // escalates (and sounds again) at the user's limit, then half, then a quarter of it
        const stage = Math.max(c.tcpa < s.tcpa / 4 ? 2 : c.tcpa < s.tcpa / 2 ? 1 : 0, c.cpa < s.cpa / 4 ? 2 : c.cpa < s.cpa / 2 ? 1 : 0);
        alarms.raise(id, { level: "alarm", stage, holdable: true, text: `COLLISION RISK ${t.name ?? t.mmsi}: CPA ${c.cpa.toFixed(2)} NM in ${Math.round(c.tcpa)} min`,
          detail: `${c.range.toFixed(2)} NM at ${String(Math.round(bearing(own.pos, [t.lon, t.lat]))).padStart(3, "0")}°T · ${t.sog?.toFixed(1) ?? "?"} kn ${t.cog != null ? Math.round(t.cog) + "°T" : ""} · click to show`,
          onClick: () => { map.easeTo({ center: [t.lon, t.lat] }); openPopup(t.mmsi); } });
        now.add(id);
      }
    }
    for (const id of raised) if (!now.has(id)) alarms.clear(id);
    raised.clear();
    for (const id of now) raised.add(id);
  }

  // ---------- data ----------------------------------------------------------------
  listen("ais", ({ payload }) => {
    if (payload.reset) targets.clear();
    for (const t of payload.targets || []) targets.set(t.mmsi, t);
    for (const m of payload.gone || []) targets.delete(m);
    draw();
    renderList();
    renderWin();
    checkAlarms();
    statusUi();
  });
  listen("ais-status", ({ payload }) => { if (online[payload.provider]) online[payload.provider] = payload; statusUi(); });
  invoke("ais_targets").then((list) => { for (const t of list || []) targets.set(t.mmsi, t); draw(); }).catch(() => {});
  setInterval(() => { draw(); renderList(); renderWin(); renderFleet(); checkAlarms(); refreshPopup(); }, 5000);

  // the online feed follows the chart: the view (wider when zoomed out), at most every 5 s
  function boxes() {
    const centre = netCentre();
    if (s.netMode !== "view") {
      if (!centre) return [];
      const h = Math.min(6, (2 * s.netRange) / 60), w = Math.min(6, h / Math.max(0.1, Math.cos((centre[1] * Math.PI) / 180)));
      return box(centre[1], centre[0], w, h);
    }
    const b = map.getBounds();
    const c = map.getCenter();
    const w = Math.min(6, Math.max(0.5, (b.getEast() - b.getWest()) * 1.2)), h = Math.min(6, Math.max(0.5, (b.getNorth() - b.getSouth()) * 1.2));
    return box(c.lat, c.lng, w, h);
  }
  /** The subscription box(es) [[lat, lon], [lat, lon]] of w × h degrees round a centre, split at the date line. */
  function box(lat, lng, w, h) {
    const lat0 = Math.max(-89, lat - h / 2), lat1 = Math.min(89, lat + h / 2);
    let lon0 = lng - w / 2, lon1 = lng + w / 2;
    const wrap = (x) => ((x + 540) % 360) - 180;
    lon0 = wrap(lon0); lon1 = wrap(lon1);
    if (lon0 <= lon1) return [[[lat0, lon0], [lat1, lon1]]];
    return [[[lat0, lon0], [lat1, 180]], [[lat0, -180], [lat1, lon1]]]; // across the date line
  }
  // two online sources, each its own feed with its own key, switch and status (same protocol, different networks)
  const FEEDS = {
    aisstream: { name: "aisstream.io", on: "online", key: "key", dot: "ais-on-dot", state: "ais-on-state", input: "ais-key", btn: "ais-online", what: "API key" },
    openwaters: { name: "Open Waters", on: "owOn", key: "owKey", dot: "ow-on-dot", state: "ow-on-state", input: "ow-key", btn: "ow-online", what: "token" },
  };
  const lastSub = {};
  let subTimer = null;
  function subscribe(force) {
    if (!PRIMARY) return; // one online AIS subscription per source, owned by the main window
    for (const [id, f] of Object.entries(FEEDS)) {
      const key = s[f.on] ? String(s[f.key] || "").trim() : "";
      const bx = key ? boxes() : [];
      const sig = key + JSON.stringify(bx.map((b) => b.map((p) => p.map((x) => Math.round(x * 4) / 4))));
      if (!force && sig === lastSub[id]) continue;
      lastSub[id] = sig;
      invoke("ais_online", { provider: id, key, boxes: bx }).catch(() => {});
    }
  }
  map.on("moveend", () => { if (s.netMode === "boat") return; clearTimeout(subTimer); subTimer = setTimeout(() => subscribe(false), 5000); });
  // round own boat: asked again when the boat has moved a fifth of the range
  let subAt = null;
  setInterval(() => {
    if (!(s.online || s.owOn) || s.netMode !== "boat") return;
    const p = getOwn()?.pos;
    if (p && (!subAt || distNm(subAt, p) > s.netRange / 5)) { subAt = p; subscribe(false); }
  }, 5000);

  // ---------- settings ----------------------------------------------------------------
  function statusUi() {
    const sk = [...targets.values()].filter((t) => !netOnly(t)).length;
    $("ais-sk-dot").className = `dot ${sk ? "on" : "paused"}`;
    $("ais-sk-state").textContent = sk ? `On board (Signal K or NMEA): ${sk} target${sk === 1 ? "" : "s"}` : "On board (Signal K or NMEA): no AIS targets (no receiver, or nothing in range)";
    for (const [id, f] of Object.entries(FEEDS)) {
      const st = online[id];
      const n = [...targets.values()].filter((t) => String(t.source).split("+").includes(id)).length;
      $(f.dot).className = `dot ${!s[f.on] ? "paused" : st.connected ? "on" : ""}`;
      $(f.state).textContent = !s[f.on] ? `${f.name}: off` : st.connected ? `${f.name}: connected, ${n} target${n === 1 ? "" : "s"}` : `${f.name}: ${st.detail || "connecting…"}`;
      $(f.btn).textContent = s[f.on] ? "Go offline" : "Go online";
      $(f.btn).classList.toggle("on", !!s[f.on]);
    }
  }
  $("ais-show").checked = s.show;
  $("ais-names").checked = s.names;
  for (const [id, f] of Object.entries(FEEDS)) {
    $(f.input).value = s[f.key] || "";
    $(f.input).onchange = (e) => { s[f.key] = e.target.value.trim(); save(); if (s[f.on]) subscribe(true); };
    $(f.btn).onclick = () => {
      if (!s[f.on] && !String(s[f.key] || "").trim()) { $(f.state).textContent = `${f.name}: paste your ${f.what} first`; $(f.input).focus(); return; }
      s[f.on] = !s[f.on]; save(); lastSub[id] = null; subscribe(true); statusUi();
    };
  }
  $("ais-key-link").onclick = (e) => { e.preventDefault(); invoke("open_url", { url: "https://aisstream.io/authenticate" }).catch(() => {}); };
  $("ow-link").onclick = (e) => { e.preventDefault(); invoke("open_url", { url: "https://openwaters.io/ais/token/" }).catch(() => {}); };
  // a free personal token, made here: no account (the private key stays in this app's settings, so it stays the same station)
  $("ow-get").onclick = async () => {
    const msg = (t) => { $("ow-msg").textContent = t; };
    if ((s.owKey || "").trim()) { msg("There is a token already: clear the box to get a new one."); return; }
    msg("Getting a token…");
    try {
      const r = await invoke("ow_register", { seed: s.owSeed || null, name: null });
      s.owSeed = r.seed; s.owKey = r.token; save();
      $("ow-key").value = r.token;
      msg("Token saved. Press Go online.");
    } catch (e) { msg(String(e)); }
  };
  $("ais-cpa-on").checked = s.cpaOn;
  $("ais-cpa").value = s.cpa;
  $("ais-tcpa").value = s.tcpa;
  $("ais-ignore-moored").checked = s.ignoreMoored;
  $("ais-sort").value = s.sort;
  $("ais-show").onchange = (e) => { s.show = e.target.checked; save(); draw(); };
  $("ais-names").onchange = (e) => { s.names = e.target.checked; save(); draw(); };
  $("ais-cpa-on").onchange = (e) => { s.cpaOn = e.target.checked; save(); checkAlarms(); };
  $("ais-cpa").onchange = (e) => { const v = parseFloat(e.target.value); if (v > 0) s.cpa = v; save(); };
  $("ais-tcpa").onchange = (e) => { const v = parseFloat(e.target.value); if (v > 0) s.tcpa = v; save(); };
  $("ais-ignore-moored").onchange = (e) => { s.ignoreMoored = e.target.checked; save(); };
  $("ais-sort").onchange = (e) => { s.sort = e.target.value; save(); renderList(); };
  ui?.onChange?.(() => { renderList(); renderFleet(); });
  $("ais-net-mode").value = s.netMode;
  $("ais-net-range").value = s.netRange;
  $("ais-net-range-row").hidden = s.netMode === "view";
  $("ais-net-alarms").checked = s.netAlarms;
  $("ais-vec-min").value = s.vecMin;
  $("ais-curve").checked = s.curve;
  $("ais-daysh").checked = s.dayShapes;
  $("ais-daysh").onchange = (e) => { s.dayShapes = e.target.checked; save(); draw(); };
  $("ais-curve").onchange = (e) => { s.curve = e.target.checked; save(); draw(); };
  $("ais-cpa-draw").value = s.cpaDraw;
  $("ais-sleep").value = s.sleep;
  $("ais-sleep-kn").value = s.sleepKn;
  $("ais-sleep-nm").value = s.sleepNm;
  const sleepChanged = () => { save(); draw(); renderList(); renderWin(); };
  $("ais-sleep").onchange = (e) => { s.sleep = e.target.value; sleepChanged(); };
  $("ais-sleep-kn").onchange = (e) => { const v = parseFloat(e.target.value); if (v >= 0) s.sleepKn = Math.min(30, v); e.target.value = s.sleepKn; sleepChanged(); };
  $("ais-sleep-nm").onchange = (e) => { const v = parseFloat(e.target.value); if (v >= 0) s.sleepNm = Math.min(200, v); e.target.value = s.sleepNm; sleepChanged(); };
  $("ais-net-mode").onchange = (e) => { s.netMode = e.target.value; $("ais-net-range-row").hidden = s.netMode === "view"; save(); subAt = null; subscribe(true); draw(); renderList(); };
  $("ais-net-range").onchange = (e) => { const v = parseFloat(e.target.value); if (v >= 1) s.netRange = Math.min(180, v); e.target.value = s.netRange; save(); subscribe(true); draw(); renderList(); };
  $("ais-net-alarms").onchange = (e) => { s.netAlarms = e.target.checked; save(); checkAlarms(); };
  $("ais-vec-min").onchange = (e) => { const v = parseFloat(e.target.value); if (v >= 1) s.vecMin = Math.min(60, v); e.target.value = s.vecMin; save(); draw(); };
  $("ais-cpa-draw").onchange = (e) => { s.cpaDraw = e.target.value; save(); draw(); };

  // ---------- floating target list ----------------------------------------------------------------
  // One row per target from any source: AIS now, radar (MARPA) targets later.
  const win = $("tgt-win");
  floatResize(win);
  const typeName = (t) => (t.kind === "vessel" ? category(t.ship_type)[0] : { aton: "AtoN", aircraft: "SAR aircraft", base: "Base station", sart: "AIS-SART", mob: "MOB", epirb: "EPIRB" }[t.kind] ?? t.kind);
  function winRows() {
    const own = getOwn();
    const rows = [];
    for (const t of targets.values()) {
      if (!shown(t)) continue;
      const c = cpaTcpa(own, t);
      if (s.sleep === "hide" && isSleeping(t, c, inFleet(t.mmsi))) continue;
      const r = { id: t.mmsi, t, c, name: inFleet(t.mmsi)?.name || t.name || `MMSI ${t.mmsi}`, type: typeName(t),
        range: c?.range ?? null, brg: own?.pos ? bearing(own.pos, [t.lon, t.lat]) : null,
        cpa: c?.tcpa > 0 ? c.cpa : null, tcpa: c?.tcpa > 0 ? c.tcpa : null, sog: t.sog ?? null, cog: t.cog ?? null,
        age: t.last_pos ? Date.now() / 1000 - t.last_pos : null, src: t.source.split("+").map((x) => SRC_SHORT[x] || x).join("+"),
        danger: isDanger(c), lost: isLost(t), net: netOnly(t), col: style(t)[1],
        grp: inFleet(t.mmsi)?.group ?? null, trk: isWatched(t.mmsi) || (fleet.trackAll && inFleet(t.mmsi)) ? 1 : 0 };
      r.col = r.danger ? DANGER_COL : r.col ?? TARGET_COL;
      const f = s.win.filter;
      if (f === "vessels" && t.kind !== "vessel") continue;
      if (f === "moving" && !((t.sog ?? 0) > 0.5)) continue;
      if (f === "closing" && r.tcpa == null) continue;
      if (f === "danger" && !r.danger && !["sart", "mob", "epirb"].includes(t.kind)) continue;
      rows.push(r);
    }
    const k = s.win.sort, dir = s.win.dir;
    rows.sort((a, b) => {
      // distress first, then dangerous targets, then the column chosen (blanks last)
      const pa = ["sart", "mob", "epirb"].includes(a.t.kind) ? 0 : a.danger ? 1 : 2, pb = ["sart", "mob", "epirb"].includes(b.t.kind) ? 0 : b.danger ? 1 : 2;
      if (pa !== pb) return pa - pb;
      const va = a[k], vb = b[k];
      if (va == null && vb == null) return 0;
      if (va == null) return 1;
      if (vb == null) return -1;
      return (typeof va === "string" ? va.localeCompare(vb) : va - vb) * dir;
    });
    return rows;
  }
  const fmt = (v, d = 1) => (v == null ? "—" : v.toFixed(d));
  const deg = (v) => (v == null ? "—" : String(Math.round(v) % 360).padStart(3, "0"));
  const age = (v) => (v == null ? "—" : v < 90 ? `${Math.round(v)} s` : v < 5400 ? `${Math.round(v / 60)} min` : `${(v / 3600).toFixed(1)} h`);
  function renderWin() {
    if (win.hidden || inUse($("tgt-table"), renderWin)) return;
    const rows = winRows();
    $("tgt-count").textContent = `${rows.length}`;
    for (const th of win.querySelectorAll("th")) {
      th.classList.toggle("sorted", th.dataset.k === s.win.sort);
      th.classList.toggle("up", th.dataset.k === s.win.sort && s.win.dir < 0);
    }
    const tb = $("tgt-table").tBodies[0];
    tb.innerHTML = rows.length ? "" : `<tr><td class="empty" colspan="12">No targets${s.win.filter !== "all" ? " for this filter" : ""}. A receiver on board (through Signal K), or online AIS in Targets › AIS.</td></tr>`;
    for (const r of rows.slice(0, 300)) {
      const tr = document.createElement("tr");
      tr.className = [r.danger ? "danger" : "", r.net ? "net" : "", r.lost ? "lost" : "", r.id === selected ? "sel" : ""].join(" ");
      if (r.net) tr.title = "Heard over the internet only: may be minutes late or missing. Not for navigation";
      tr.innerHTML = `<td><span class="sw"></span></td><td></td><td class="n">${fmt(r.range, r.range != null && r.range < 10 ? 2 : 1)}</td><td class="n">${deg(r.brg)}</td>
        <td class="n">${fmt(r.cpa, 2)}</td><td class="n">${r.tcpa == null ? "—" : Math.round(r.tcpa)}</td><td class="n">${fmt(r.sog)}</td><td class="n">${deg(r.cog)}</td><td class="n age">${age(r.age)}</td><td>${r.src}</td><td class="ctl"></td><td class="ctl"></td>`;
      const locked = fleet.trackAll && !!inFleet(r.id);
      const cb = document.createElement("input");
      cb.type = "checkbox"; cb.checked = !!r.trk; cb.disabled = locked;
      cb.onchange = () => { setWatched(r.id, cb.checked); saveFleet(); renderFleet(); refreshPopup(); loadTracks(); renderWin(); };
      tr.cells[10].append(cb);
      const grp = document.createElement("select");
      grp.innerHTML = `<option value="">—</option>${Object.keys(GROUP_COLOURS).map((g) => `<option>${g}</option>`).join("")}`;
      grp.value = r.grp ?? "";
      grp.onchange = () => {
        const e = inFleet(r.id);
        if (!grp.value) fleet.entries = fleet.entries.filter((x) => Number(x.mmsi) !== Number(r.id));
        else if (e) e.group = grp.value;
        else fleet.entries.push({ mmsi: String(r.id).padStart(9, "0"), name: r.t.name ?? "", group: grp.value });
        saveFleet(); renderFleet(); renderList(); refreshPopup(); draw(); renderWin();
      };
      tr.cells[11].append(grp);
      for (const c of [cb, grp]) c.onclick = (ev) => ev.stopPropagation();
      tr.cells[0].append(document.createTextNode(r.name));
      tr.cells[0].querySelector(".sw").style.background = r.col;
      tr.cells[1].textContent = r.type + (r.t.kind === "vessel" && STATUS_SHORT[r.t.status] ? ` · ${STATUS_SHORT[r.t.status]}` : "");
      if (r.lost) tr.cells[8].title = "Not heard for a while: shown crossed through on the chart at its last position";
      tr.onclick = () => { map.easeTo({ center: [r.t.lon, r.t.lat] }); openPopup(r.id); renderWin(); };
      tb.append(tr);
    }
  }
  for (const th of win.querySelectorAll("th")) {
    th.onclick = () => {
      const k = th.dataset.k;
      if (s.win.sort === k) s.win.dir = -s.win.dir; else { s.win.sort = k; s.win.dir = 1; }
      save(); renderWin();
    };
  }
  $("tgt-filter").value = s.win.filter;
  $("tgt-filter").onchange = (e) => { s.win.filter = e.target.value; save(); renderWin(); };
  function place() {
    if (s.win.x == null) return;
    const r = map.getContainer().getBoundingClientRect();
    win.style.left = `${r.left + Math.max(0, Math.min(r.width - 120, s.win.x))}px`;
    win.style.top = `${r.top + Math.max(0, Math.min(r.height - 40, s.win.y))}px`;
    win.style.right = "auto";
  }
  function showWin(on) {
    s.win.open = on;
    win.hidden = !on;
    $("targets-btn").classList.toggle("on", on);
    save();
    if (on) { place(); renderWin(); }
  }
  $("targets-btn").onclick = () => showWin(win.hidden);
  $("ais-win-open").onclick = () => showWin(true);
  $("tgt-close").onclick = () => showWin(false);
  // T (keys.js) presses the Targets button
  // drag by the title bar
  win.querySelector("header").addEventListener("pointerdown", (e) => {
    if (e.target.closest("select, button")) return;
    const r = win.getBoundingClientRect(), m = map.getContainer().getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    const move = (ev) => { s.win.x = ev.clientX - m.left - dx; s.win.y = ev.clientY - m.top - dy; place(); };
    const up = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); save(); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });
  showWin(!!s.win.open);

  // ---------- own fleet tab ----------------------------------------------------------------
  function renderFleet() {
    const ul = $("of-list");
    if (!ul || ul.closest(".pane")?.hidden || inUse(ul, renderFleet)) return;
    const own = getOwn();
    ul.innerHTML = fleet.entries.length ? "" : `<li class="empty">No boats yet. Add an MMSI above, or click a boat on the chart › Add to own fleet.</li>`;
    for (const e of fleet.entries) {
      const m = Number(e.mmsi);
      const t = targets.get(m);
      const li = document.createElement("li");
      const col = fleetColour(e);
      li.innerHTML = `<input type="color" class="of-col" value="${esc(col)}" title="Stern colour on the chart" /><div class="main"><b></b><small></small></div>
        <label class="of-trk" title="Draw this boat's track"><input type="checkbox" /> Track</label>
        <select title="Group">${Object.keys(GROUP_COLOURS).map((g) => `<option${g === e.group ? " selected" : ""}>${g}</option>`).join("")}</select><button data-act="del" title="Remove">✕</button>`;
      li.querySelector("b").textContent = e.name || t?.name || `MMSI ${e.mmsi}`;
      const c = t?.lat != null ? cpaTcpa(own, t) : null;
      li.querySelector("small").textContent = [e.mmsi, t ? (t.sog != null ? `${t.sog.toFixed(1)} kn` : null) : "not heard yet",
        c ? `${c.range.toFixed(1)} NM` : null, t?.last_pos ? `${Math.round((Date.now() / 1000 - t.last_pos) / 60)} min ago` : null].filter(Boolean).join(" · ");
      const cb = li.querySelector(".of-trk input");
      cb.checked = fleet.trackAll || isWatched(e.mmsi);
      cb.disabled = fleet.trackAll;
      cb.onchange = () => { setWatched(e.mmsi, cb.checked); saveFleet(); renderList(); loadTracks(); };
      li.querySelector(".of-col").onchange = (ev) => { e.colour = ev.target.value; saveFleet(); draw(); };
      li.querySelector("select").onchange = (ev) => { e.group = ev.target.value; saveFleet(); renderFleet(); draw(); };
      li.querySelector('[data-act="del"]').onclick = () => { fleet.entries = fleet.entries.filter((x) => x !== e); saveFleet(); renderFleet(); draw(); loadTracks(); };
      li.querySelector(".main").onclick = () => { if (t?.lat != null) { map.easeTo({ center: [t.lon, t.lat], zoom: Math.max(map.getZoom(), 11) }); openPopup(m); } };
      ul.append(li);
    }
  }
  $("of-add").onclick = () => {
    const mmsi = $("of-mmsi").value.replace(/\D/g, "");
    if (mmsi.length !== 9) { $("of-msg").textContent = "An MMSI has 9 digits."; return; }
    if (inFleet(mmsi)) { $("of-msg").textContent = "That MMSI is already in the list."; return; }
    if (Number(mmsi) === ownMmsi) { $("of-msg").textContent = "That is your own MMSI (Settings)."; return; }
    const colour = $("of-colour").value;
    fleet.entries.push({ mmsi, name: $("of-name").value.trim(), group: $("of-group").value,
      ...(colour.toLowerCase() !== (GROUP_COLOURS[$("of-group").value] ?? GROUP_COLOURS.Other).toLowerCase() ? { colour } : {}) });
    $("of-mmsi").value = ""; $("of-name").value = ""; $("of-msg").textContent = "";
    saveFleet(); renderFleet(); draw();
  };
  $("of-colour").value = GROUP_COLOURS[$("of-group").value] ?? GROUP_COLOURS.Other;
  $("of-group").onchange = (e) => { $("of-colour").value = GROUP_COLOURS[e.target.value] ?? GROUP_COLOURS.Other; };
  $("of-track-all").checked = fleet.trackAll;
  $("of-only").checked = fleet.only;
  $("of-track-all").onchange = (e) => { fleet.trackAll = e.target.checked; saveFleet(); renderFleet(); loadTracks(); };
  $("of-only").onchange = (e) => { fleet.only = e.target.checked; saveFleet(); draw(); renderList(); };
  $("ais-sel-track").checked = s.selTrack;
  $("ais-sel-track").onchange = (e) => { s.selTrack = e.target.checked; save(); loadTracks(); };
  $("ais-trk-col").value = s.trkCol;
  $("ais-trk-col").oninput = (e) => { s.trkCol = e.target.value; save(); drawTracks(); };
  // our own MMSI (Settings) is never a target
  const syncOwn = () => {
    ownMmsi = Number(getOwnMmsi?.() || 0) || null;
    invoke("ais_set_own", { mmsi: ownMmsi }).catch(() => {});
    if (ownMmsi) { targets.delete(ownMmsi); draw(); renderList(); }
  };
  window.addEventListener("racenav-boat", syncOwn);
  syncOwn();
  renderFleet();
  loadTracks();
  statusUi();
  if (s.online || s.owOn) setTimeout(() => subscribe(true), 1500);
  /** The targets that matter most, for the data panel: distress first, then dangerous ones,
   *  then closing targets by CPA, then the rest by range. Each: { id, name, range, brg, cpa,
   *  tcpa, danger, lost, col, kind }. */
  function closest(n = 5) {
    const own = getOwn();
    const out = [];
    for (const t of targets.values()) {
      if (!shown(t) || t.lat == null) continue;
      const c = cpaTcpa(own, t);
      if (s.sleep === "hide" && isSleeping(t, c, inFleet(t.mmsi))) continue;
      out.push({ id: t.mmsi, name: inFleet(t.mmsi)?.name || t.name || `MMSI ${t.mmsi}`, kind: t.kind,
        range: c?.range ?? null, brg: own?.pos ? bearing(own.pos, [t.lon, t.lat]) : null,
        cpa: c?.tcpa > 0 ? c.cpa : null, tcpa: c?.tcpa > 0 ? c.tcpa : null,
        danger: isDanger(c), lost: isLost(t), col: isDanger(c) ? DANGER_COL : style(t)[1] ?? TARGET_COL, lon: t.lon, lat: t.lat });
    }
    const rank = (r) => (["sart", "mob", "epirb"].includes(r.kind) ? 0 : r.danger ? 1 : r.tcpa != null ? 2 : 3);
    out.sort((a, b) => rank(a) - rank(b) || (rank(a) === 2 ? a.cpa - b.cpa : (a.range ?? 1e9) - (b.range ?? 1e9)));
    return out.slice(0, n);
  }
  /** Show a target on the chart and open its card. */
  function select(id) {
    const t = targets.get(id) ?? targets.get(Number(id)) ?? targets.get(String(id));
    if (!t) return;
    map.easeTo({ center: [t.lon, t.lat] });
    openPopup(t.mmsi);
  }
  return { targets, openPopup, closest, select };
}
