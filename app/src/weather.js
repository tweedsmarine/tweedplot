// Weather panel: GRIB files, time slider, wind barbs and speed shading,
// and a table comparing every loaded run at the cursor.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Wind speed shading (knots).
// Palette sampled every 2 kn so light-air differences show too.
export const PALETTE = [
  [0, "#e8f4ff"], [2, "#c4e2ff"], [4, "#9fd0f7"], [6, "#79bdec"], [8, "#5ab9cf"], [10, "#5cc6a3"],
  [12, "#7fd27a"], [14, "#b3dc5c"], [16, "#e2e04f"], [18, "#f7cd45"], [20, "#f7aa3b"], [22, "#f38536"],
  [25, "#e85c3a"], [28, "#d6404f"], [32, "#b8357a"], [36, "#9334a3"], [40, "#6c2fae"], [50, "#45247f"],
];

// Other quantities: value stops and colours.
const WAVES = [[0, "#eef6fb"], [0.5, "#cfe6f5"], [1, "#a6d2ec"], [1.5, "#7fbde0"], [2, "#5aa8d3"], [3, "#5cc6a3"], [4, "#b3dc5c"],
  [5, "#f7cd45"], [6, "#f38536"], [8, "#d6404f"], [10, "#9334a3"], [14, "#45247f"]];
const PERIOD = [[0, "#eef6fb"], [4, "#cfe6f5"], [6, "#a6d2ec"], [8, "#7fbde0"], [10, "#5cc6a3"], [12, "#b3dc5c"], [14, "#f7cd45"],
  [16, "#f38536"], [18, "#d6404f"], [22, "#9334a3"]];
const RAIN = [[0.1, "#d7ecf7"], [0.5, "#a6d2ec"], [1, "#6fb3dc"], [2, "#3f8fcf"], [4, "#5cc6a3"], [8, "#f7cd45"], [16, "#f38536"],
  [32, "#d6404f"], [64, "#9334a3"]];
const CAPE = [[100, "#f4f0c0"], [250, "#f7e08a"], [500, "#f7cd45"], [1000, "#f7aa3b"], [1500, "#f38536"], [2000, "#e85c3a"],
  [3000, "#d6404f"], [4000, "#9334a3"]];
const PRESS = [[960, "#45247f"], [980, "#6c2fae"], [995, "#5aa8d3"], [1005, "#a6d2ec"], [1013, "#eef6fb"], [1020, "#f7e08a"],
  [1030, "#f7aa3b"], [1040, "#d6404f"]];

const TEMP = [[-10, "#45247f"], [-5, "#6c2fae"], [0, "#3f8fcf"], [5, "#79bdec"], [10, "#a6d2ec"], [15, "#b3dc5c"], [20, "#f7cd45"],
  [25, "#f38536"], [30, "#d6404f"], [35, "#9334a3"]];

// Agreement between GRIBs: green = the forecasts agree, red = they don't.
const AGREE_KN = [[0, "#1a9850"], [1, "#66bd63"], [2, "#a6d96a"], [3, "#d9ef8b"], [4, "#fee08b"], [6, "#fdae61"], [8, "#f46d43"],
  [10, "#d73027"], [15, "#a50026"]];
const AGREE_DEG = [[0, "#1a9850"], [5, "#66bd63"], [10, "#a6d96a"], [15, "#d9ef8b"], [20, "#fee08b"], [30, "#fdae61"], [45, "#f46d43"],
  [60, "#d73027"], [90, "#a50026"]];
const AGREE_HPA = [[0, "#1a9850"], [0.5, "#66bd63"], [1, "#a6d96a"], [1.5, "#d9ef8b"], [2, "#fee08b"], [3, "#fdae61"], [4, "#f46d43"],
  [6, "#d73027"], [8, "#a50026"]];
// newest run minus oldest: blue = lighter now, red = windier now
const TREND = [[-10, "#2166ac"], [-6, "#4393c3"], [-3, "#92c5de"], [-1, "#d1e5f0"], [1, "#f7f7f7"], [3, "#fddbc7"], [6, "#f4a582"],
  [10, "#d6604d"], [15, "#b2182b"]];

// Verification: a forecast's error against a later run's hour 0.
const VERR_SPD = [[-12, "#2166ac"], [-8, "#4393c3"], [-4, "#92c5de"], [-1.5, "#d1e5f0"], [1.5, "#f7f7f7"], [4, "#fddbc7"], [8, "#f4a582"],
  [12, "#d6604d"], [16, "#b2182b"]];
// signed direction error: blue = backed, red = veered (degrees, clockwise positive)
const VERR_DIR = [[-60, "#2166ac"], [-30, "#4393c3"], [-15, "#92c5de"], [-5, "#d1e5f0"], [5, "#f7f7f7"], [15, "#fddbc7"], [30, "#f4a582"],
  [60, "#d6604d"], [90, "#b2182b"]];

// Ocean: sea temperature and salinity (`auto`) are shown through a filter the
// skipper sets with three pointers under the colour bar: a low and a high limit
// (nothing shown outside them) and a target, which sits at the middle of the
// colours with its own bold contour. The colours follow the filter, never the zoom.
// These scales are only the starting point before a file is loaded.
const SST = [[8, "#30123b"], [10, "#4145ab"], [12, "#4675ed"], [14, "#39a2fc"], [16, "#1bcfd4"], [18, "#24eca6"], [20, "#61fc6c"],
  [22, "#a4fc3b"], [24, "#d1e834"], [26, "#f3c63a"], [28, "#fe9b2d"], [30, "#f36315"]];
const SSS = [[33.5, "#30123b"], [34, "#4145ab"], [34.3, "#39a2fc"], [34.6, "#1bcfd4"], [34.9, "#61fc6c"], [35.2, "#d1e834"], [35.5, "#fe9b2d"], [36, "#f36315"]];
// metres above the geoid: blue low, red high (eddies and ridges of the ocean surface)
const SSH = [[-1, "#2166ac"], [-0.6, "#4393c3"], [-0.3, "#92c5de"], [-0.1, "#d1e5f0"], [0.1, "#f7f7f7"], [0.3, "#fddbc7"], [0.6, "#f4a582"],
  [1, "#d6604d"], [1.4, "#b2182b"]];
// log10 of mg/m³: 0.03 … 10
const CHL = [[-1.5, "#30123b"], [-1.2, "#4145ab"], [-1, "#4675ed"], [-0.7, "#39a2fc"], [-0.5, "#1bcfd4"], [-0.3, "#24eca6"], [0, "#61fc6c"],
  [0.3, "#d1e834"], [0.5, "#fe9b2d"], [0.7, "#f36315"], [1, "#a51301"]];
/** A chlorophyll value for people: two figures. */
export const fmtChl = (v) => (v >= 10 ? v.toFixed(0) : v >= 1 ? v.toFixed(1) : v >= 0.1 ? v.toFixed(2) : v.toFixed(3));
const CHL_LEVELS = [0.02, 0.03, 0.05, 0.07, 0.1, 0.15, 0.2, 0.3, 0.5, 0.7, 1, 1.5, 2, 3, 5, 7, 10, 15, 20, 30, 50];
const CUR = [[0, "#f7fbff"], [0.25, "#c6dbef"], [0.5, "#9ecae1"], [0.75, "#6baed6"], [1, "#4292c6"], [1.5, "#2171b5"], [2, "#08519c"], [3, "#08306b"]];
// the colours an auto range is spread over ("turbo": dark blue, cyan, green, yellow, red)
const TURBO = ["#30123b", "#4145ab", "#4675ed", "#39a2fc", "#1bcfd4", "#24eca6", "#61fc6c", "#a4fc3b", "#d1e834", "#f3c63a", "#fe9b2d", "#f36315", "#d93806", "#a51301"];

/** A colour scale over lo..hi (the visible values), with stops at tidy values. */
export function autoScale(lo, hi) {
  if (!(hi > lo)) hi = lo + 0.5;
  const n = TURBO.length;
  const raw = (hi - lo) / (n - 1);
  const p = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= raw) ?? raw;
  const start = Math.floor(lo / step) * step;
  return TURBO.map((c, i) => [Math.round((start + i * step) * 1000) / 1000, c]);
}

/** The colours for a sea filter: lo..target over the cool half of TURBO, target..hi over the warm half. */
export function seaScale(f) {
  const half = TURBO.length / 2;
  const lo = f.lo, hi = f.hi > f.lo ? f.hi : f.lo + 1e-3;
  const out = [];
  // no target: the whole palette runs evenly from lo to hi
  if (!Number.isFinite(f.tg)) return TURBO.map((c, i) => [lo + ((hi - lo) * i) / (TURBO.length - 1), c]);
  const margin = (hi - lo) * 1e-6;
  const tg = Math.min(Math.max(f.tg, lo + margin), hi - margin);
  for (let i = 0; i < half; i++) out.push([lo + ((tg - lo) * i) / half, TURBO[i]]);
  for (let i = 0; i < half; i++) out.push([tg + ((hi - tg) * i) / (half - 1), TURBO[half + i]]);
  return out;
}

/** A tidy contour interval giving about `n` lines over lo..hi. */
function niceStep(lo, hi, n = 8) {
  const raw = Math.max((hi - lo) / n, 0.05);
  const p = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 5, 10].map((m) => m * p).find((s) => s >= raw);
}

/** What can be shaded: the GRIB parameters it needs, unit, colour scale, and values below `min` left clear.
 * `compare`: a display of how much the loaded GRIBs disagree (grib::compare), not of one GRIB. */
export const LAYERS = {
  wind: { label: "Wind speed", unit: "kn", params: ["WindU", "WindSpeed"], scale: PALETTE, knots: true },
  gust: { label: "Gusts", unit: "kn", params: ["Gust"], scale: PALETTE, knots: true },
  waves: { label: "Wave height (Hs)", unit: "m", params: ["WaveHeight"], scale: WAVES },
  swell: { label: "Swell height", unit: "m", params: ["SwellHeight"], scale: WAVES },
  period: { label: "Wave period", unit: "s", params: ["WavePeriod"], scale: PERIOD },
  rain: { label: "Rain", unit: "mm/h", params: ["Rain"], scale: RAIN, min: 0.1 },
  cape: { label: "CAPE (squalls)", unit: "J/kg", params: ["Cape"], scale: CAPE, min: 100 },
  mslp: { label: "Pressure", unit: "hPa", params: ["Mslp"], scale: PRESS },
  temp: { label: "Air temperature", unit: "°C", params: ["AirTemp"], scale: TEMP },
  // ocean layers come from whichever loaded file has them, under the shown GRIB's wind barbs
  sst: { label: "Sea temperature", unit: "°C", params: ["SeaTemp"], scale: SST, auto: true, contour: true, ocean: true },
  sss: { label: "Salinity", unit: "PSU", params: ["Salinity"], scale: SSS, auto: true, contour: true, ocean: true },
  subt: { label: "Sea temperature at depth", unit: "°C", params: ["SubTemp"], scale: SST, auto: true, contour: true, ocean: true },
  subs: { label: "Salinity at depth", unit: "PSU", params: ["SubSalinity"], scale: SSS, auto: true, contour: true, ocean: true },
  skin: { label: "Sea temperature, satellite hourly (Himawari)", unit: "°C", params: ["SkinTemp"], scale: SST, auto: true, contour: true, ocean: true },
  // chlorophyll is shaded, filtered and contoured on a log scale (the grid comes as log10 mg/m³)
  chl: { label: "Chlorophyll-a", unit: "mg/m³", params: ["Chlorophyll"], scale: CHL, auto: true, contour: true, ocean: true, log: true },
  current: { label: "Current", unit: "kn", params: ["CurrentU"], scale: CUR, ocean: true },
  ssh: { label: "Sea surface height", unit: "m", params: ["SeaLevel"], scale: SSH, ocean: true },
  agree_vec: { label: "Wind disagreement (vector)", unit: "kn", params: ["WindU", "WindSpeed"], scale: AGREE_KN, compare: "vector",
    help: "RMS distance of each GRIB's wind from their mean wind: speed and direction together." },
  agree_spd: { label: "Wind speed spread", unit: "kn", params: ["WindU", "WindSpeed"], scale: AGREE_KN, compare: "speed",
    help: "Highest minus lowest wind speed (P90 − P10 with seven or more GRIBs)." },
  agree_dir: { label: "Wind direction spread", unit: "°", params: ["WindU", "WindSpeed"], scale: AGREE_DEG, compare: "direction",
    help: "Spread of wind direction about the mean direction; blank where the mean wind is under 6 kn." },
  agree_mslp: { label: "Pressure spread", unit: "hPa", params: ["Mslp"], scale: AGREE_HPA, compare: "pressure",
    help: "Highest minus lowest MSL pressure (P90 − P10 with seven or more GRIBs)." },
  agree_trend: { label: "Run trend (newest − oldest)", unit: "kn", params: ["WindU", "WindSpeed"], scale: TREND, compare: "trend",
    help: "Wind speed of the newest run minus the oldest: red = the newer run is windier." },
  verr_vec: { label: "Forecast error (vector)", unit: "kn", params: ["WindU", "WindSpeed"], scale: AGREE_KN, verify: "vector",
    help: "Vector difference between the forecast and the later run's hour 0." },
  verr_spd: { label: "Forecast speed error", unit: "kn", params: ["WindU", "WindSpeed"], scale: VERR_SPD, verify: "speed",
    help: "Forecast speed minus the hour-0 speed: red = the forecast was too windy." },
  verr_dir: { label: "Forecast direction error", unit: "°", params: ["WindU", "WindSpeed"], scale: VERR_DIR, verify: "direction",
    help: "Forecast direction minus the hour-0 direction: red = forecast veered (clockwise), blue = backed. Blank under 6 kn." },
};

/** Layers the low / target / high filter works on: anything shaded from one GRIB (not the agreement or error displays). */
export const filterable = (l) => !!l && !l.compare && !l.verify;

function seaQuantum(lo, hi) {
  const r = Math.max(hi - lo, 0.01);
  return 10 ** Math.floor(Math.log10(r / 20));
}

/** The full grid range at this time, and the filter to use with it. */
export function filterRangeForGrid(grid, filter) {
  let [min, max] = grid.range ?? [];
  if (!Number.isFinite(min) || !Number.isFinite(max)) {
    const values = (grid.values ?? []).filter(Number.isFinite);
    if (!values.length) return null;
    min = values.reduce((a, v) => Math.min(a, v), Infinity);
    max = values.reduce((a, v) => Math.max(a, v), -Infinity);
  }
  const q = seaQuantum(min, max);
  const lo = +(Math.floor(min / q) * q).toFixed(6);
  let hi = +(Math.ceil(max / q) * q).toFixed(6);
  if (hi <= lo) hi = +(lo + q).toFixed(6);
  const keep = filter && (filter.set || Number.isFinite(filter.tg) || (filter.lo === lo && filter.hi === hi));
  return { min: lo, max: hi, filter: keep ? filter : { lo, hi, tg: null } };
}

/** Fit a filter track around the selected range, with room to widen it again. */
export function seaTrackRange(filter, full, previous = null, edge = null) {
  const span = Math.max(filter.hi - filter.lo, 1e-6);
  const quantum = 10 ** Math.floor(Math.log10(Math.max(span, 0.01) / 20));
  let min = Math.floor((filter.lo - span * 0.25) / quantum) * quantum;
  let max = Math.ceil((filter.hi + span * 0.25) / quantum) * quantum;
  if (previous && edge === "lo") min = Math.min(min, filter.lo - (previous.max - previous.min) * 0.6);
  if (previous && edge === "hi") max = Math.max(max, filter.hi + (previous.max - previous.min) * 0.6);
  return {
    min: +Math.max(Math.min(full.min, filter.lo), min).toFixed(6),
    max: +Math.min(Math.max(full.max, filter.hi), max).toFixed(6),
  };
}

export function dataTimeStatus(summary, params, time, allowPastAnalysis = false) {
  const times = (summary?.params ?? []).filter((p) => params.includes(p.param)).flatMap((p) => p.times ?? []).filter(Number.isFinite);
  if (!times.length || !Number.isFinite(time)) return { state: "missing" };
  const first = Math.min(...times), last = Math.max(...times);
  if (time >= first && time <= last) return { state: "valid" };
  const analysis = /VIIRS|MUR SST|Himawari/i.test(`${summary?.model ?? ""} ${summary?.name ?? ""}`);
  return time > last && analysis && allowPastAnalysis ? { state: "past", time: last } : { state: "missing" };
}

const SEA_POINT_PARAMS = {
  sst: "SeaTemp", skin: "SkinTemp", sss: "Salinity", subt: "SubTemp",
  subs: "SubSalinity", chl: "Chlorophyll", ssh: "SeaLevel", cur: "CurrentU",
};

export function applyPointTimePolicy(row, summary, time, allowPastAnalysis) {
  const result = { ...row, past: {} };
  for (const [key, param] of Object.entries(SEA_POINT_PARAMS)) {
    if (result[key] == null) continue;
    const status = dataTimeStatus(summary, [param], time, allowPastAnalysis);
    if (status.state === "missing") result[key] = null;
    else if (status.state === "past") result.past[key] = status.time;
  }
  return result;
}

/**
 * Sea temperature fronts: the cells of a temperature grid where it changes faster than
 * `min` °C per km (the larger of the two directions), each with its gradient `g`.
 */
export function fronts(gs, min = 0.02) {
  const { ni, nj, values: v } = gs;
  const out = [];
  const at = (i, j) => v[j * ni + i];
  for (let j = 1; j + 1 < nj; j++) {
    const lat = gs.lat0 + j * gs.dlat;
    const kmx = 111.32 * Math.cos((lat * Math.PI) / 180) * gs.dlon * 2, kmy = 111.32 * gs.dlat * 2;
    for (let i = 1; i + 1 < ni; i++) {
      const [a, b, c, d] = [at(i + 1, j), at(i - 1, j), at(i, j + 1), at(i, j - 1)];
      if (a == null || b == null || c == null || d == null) continue;
      const g = Math.hypot((a - b) / kmx, (c - d) / kmy);
      if (!(g >= min)) continue;
      const lon = gs.lon0 + i * gs.dlon, hx = gs.dlon / 2, hy = gs.dlat / 2;
      out.push({ type: "Feature", properties: { g },
        geometry: { type: "Polygon", coordinates: [[[lon - hx, lat - hy], [lon + hx, lat - hy], [lon + hx, lat + hy], [lon - hx, lat + hy], [lon - hx, lat - hy]]] } });
    }
  }
  return out;
}

// Display settings (per viewer)
const WX_DEFAULTS = { density: 5, size: 1.0, shading: "2", opacity: 0.45, land: "land", layer: "wind", arrows: true, waveArrows: true, currArrows: true, isobars: false,
  // isobar interval (hPa), and whether to widen it when zoomed out
  isoStep: 4, isoAuto: true,
  // contours over sea temperature and salinity (fronts)
  isotherms: true,
  // per sea layer: { lo, tg, hi } (see seaScale)
  seaFilter: {},
  // loaded GRIBs left out of the agreement displays
  cmpOff: [],
  // which loaded GRIB feeds each layer or overlay when the choice is not the shown one: { key: path }
  src: {},
  // highlight sea temperature fronts (where the temperature changes fastest)
  breaks: false,
  // all weather layers hidden from the chart (the toolbar button)
  hidden: false };
const DENSITY_POINTS = [80, 150, 300, 500, 900, 1400, 2200, 3500, 6000, 1e9];
const WX_VIEWS = "racenav.wx.views";
/** Weather is drawn in Planning and Environment unless hidden there; in Navigation only once it has been switched on there. */
export const weatherVisibleInView = (view, hidden, shownInNav = false) => (view === "nav" ? shownInNav === true : !hidden);
/** Whether a layer is coloured and limited by its filter: the ocean layers always, the weather layers once theirs has been set. */
export const filterInUse = (layer, filter) => !!filter && filterable(layer) && (!!layer.auto || !!filter.set);
/** Which ends of a filter hide values: only the ends that were set (an older filter without the marks: both). */
export const filterBounds = (filter) => ({ lo: filter.loSet ?? !!filter.set, hi: filter.hiSet ?? !!filter.set });
function loadWx() {
  try { return { ...WX_DEFAULTS, ...JSON.parse(localStorage.getItem("racenav.wx") || "{}") }; } catch { return { ...WX_DEFAULTS }; }
}
function loadWxViews() {
  try {
    const saved = JSON.parse(localStorage.getItem(WX_VIEWS) || "{}");
    if (!saved || typeof saved !== "object" || Array.isArray(saved)) throw new Error("Invalid weather view settings");
    return saved;
  } catch (error) {
    console.error("Could not read weather settings by view:", error);
    return {};
  }
}

/** Colour of a value on a scale (linear between stops); the wind palette by default. */
function paletteAt(kn, scale = PALETTE) {
  if (kn <= scale[0][0]) return scale[0][1];
  for (let i = 1; i < scale.length; i++) {
    const [k1, c1] = scale[i - 1], [k2, c2] = scale[i];
    if (kn <= k2) {
      const f = Math.max(0, (kn - k1) / (k2 - k1));
      const a = [1, 3, 5].map((o) => parseInt(c1.slice(o, o + 2), 16));
      const b = [1, 3, 5].map((o) => parseInt(c2.slice(o, o + 2), 16));
      return "#" + a.map((v, j) => Math.round(v + (b[j] - v) * f).toString(16).padStart(2, "0")).join("");
    }
  }
  return scale.at(-1)[1];
}

/** fill-color expression: smooth, or bands (of `mode` knots for wind; between the scale's stops otherwise). */
function shadingExpression(mode, layer = LAYERS.wind) {
  const scale = layer.scale;
  if (mode === "smooth") return ["interpolate", ["linear"], ["get", "v"], ...scale.flat()];
  if (layer.knots) {
    const step = Number(mode);
    const expr = ["step", ["get", "v"], paletteAt(step / 2)];
    for (let k = step; k <= 50; k += step) expr.push(k, paletteAt(k + step / 2));
    return expr;
  }
  const expr = ["step", ["get", "v"], scale[0][1]];
  for (let i = 1; i < scale.length; i++) expr.push(scale[i][0], paletteAt((scale[i][0] + (scale[i + 1]?.[0] ?? scale[i][0] * 1.2)) / 2, scale));
  return expr;
}

/** Matching chart colours and colour-bar stops for a filtered layer. */
export function filterColourStyle(layer, filter, mode) {
  const scale = seaScale(filter);
  if (mode === "smooth" || layer.auto) {
    return { expression: ["interpolate", ["linear"], ["get", "v"], ...scale.flat()], stops: scale };
  }
  const lo = scale[0][0], hi = scale.at(-1)[0];
  const boundaries = [];
  if (layer.knots) {
    const step = Number(mode);
    for (let k = Math.floor(lo / step) + 1; k * step < hi; k++) boundaries.push(k * step);
    if (Number.isFinite(filter.tg) && filter.tg > lo && filter.tg < hi) boundaries.push(filter.tg);
    boundaries.sort((a, b) => a - b);
  } else {
    boundaries.push(...scale.slice(1, -1).map(([v]) => v));
  }
  const edges = [lo, ...new Set(boundaries), hi];
  const colours = edges.slice(1).map((edge, i) => paletteAt((edges[i] + edge) / 2, scale));
  const expression = ["step", ["get", "v"], colours[0]];
  const stops = [[lo, colours[0]]];
  for (let i = 1; i < edges.length - 1; i++) {
    expression.push(edges[i], colours[i]);
    stops.push([edges[i], colours[i - 1]], [edges[i], colours[i]]);
  }
  stops.push([hi, colours.at(-1)]);
  return { expression, stops };
}

/** The outer time span of all parameters in a loaded GRIB, not a guarantee of every field. */
export function forecastSpan(summary) {
  const times = (summary?.params ?? []).flatMap((p) => p.times ?? []).filter(Number.isFinite);
  return times.length ? [Math.min(...times), Math.max(...times)] : null;
}

/** Preserve exact forecast times even when different files use different time grids. */
export function sliderStepForTime(time, start, step) {
  return (time - start) % step === 0 ? step : "any";
}

export function weatherSelectionSummary({ visible, sources, layerInfo, layerOptions, overlayOptions, view = "" }) {
  // in Navigation weather is off until switched on there, whatever the other views show
  if (!visible) return view === "nav" ? "Weather is off in Navigation" : "Chart weather hidden";
  if (!sources.length) return "No GRIB loaded";
  const available = layerOptions.some((o) => o.key === layerInfo.key);
  const shading = layerInfo.off ? "No shading" : `${layerInfo.layer.label}${available ? "" : " (no data)"}`;
  const active = overlayOptions.filter((o) => o.on).map((o) => o.label);
  return `Selected: ${[shading, ...active].join(", ")}`;
}

/**
 * `layers.landTop`: first land fill layer (shading goes under it for "land on top");
 * `layers.coastTop`: first coastline layer (shading goes under it for "coastline on top");
 * `layers.top`: barbs go under this one.
 */
export function setupWeather(map, layers) {
  const beforeTop = layers.top;
  const shadingBefore = () => {
    // the online base map's land stands in for the bundled land, so the shading goes under it
    if (map.getLayer("osm") && map.getLayoutProperty("osm", "visibility") === "visible") return "osm";
    return wx.land === "coast" ? layers.coastTop : layers.landTop;
  };
  const $ = (id) => document.getElementById(id);
  const loaded = new Map(); // path -> summary
  let display = null; // path shown on the chart
  let times = [];
  let timeStep = 3600;
  let t = 0;
  let playing = null;
  let cursor = null; // [lon, lat]
  let weatherView = ["nav", "plan", "env"].includes(document.documentElement.dataset.view) ? document.documentElement.dataset.view : "nav";
  // Environment: show the latest past analysis (satellite sea temperature and the like) when the chart time is
  // later than it. On to begin with in the Fishing use, where those layers are the point; the tick is remembered.
  const pastDefault = () => wx.pastAnalysis ?? document.documentElement.dataset.use === "fish";
  let allowPastAnalysis = false;
  // a GRIB call that failed while drawing: said in the panel, not swallowed (the chart would just be blank)
  let drawError = null;
  const failed = (e) => { drawError = String(e?.message ?? e); console.error("GRIB call failed:", e); return null; };
  function showDrawError() {
    const el = $("side-wx-error");
    if (!el) return;
    el.hidden = !drawError;
    el.textContent = drawError ? `Weather could not be drawn: ${drawError}` : "";
  }
  const savedViews = loadWxViews();
  const wx = { ...loadWx(), ...structuredClone(savedViews[weatherView] ?? {}) };
  const viewStates = new Map();
  const timeListeners = [];
  const loadedListeners = [];
  const changeListeners = [];
  const changed = () => { for (const fn of changeListeners) fn(); };
  const pointListeners = [];
  // a forecast chosen in the Verify dock: { t, path, name, lead } (enables the forecast error layers)
  let verifyTarget = null;

  map.addSource("wx-cells", { type: "geojson", data: fc([]) });
  map.addSource("wx-barbs", { type: "geojson", data: fc([]) });
  const moveShading = () => { map.moveLayer("wx-cells", shadingBefore()); if (map.getLayer("wx-backdrop")) map.moveLayer("wx-backdrop", "wx-cells"); };
  map.addLayer({
    id: "wx-cells", type: "fill", source: "wx-cells",
    paint: { "fill-color": shadingExpression(wx.shading === "off" ? "2" : wx.shading, LAYERS[wx.layer] ?? LAYERS.wind), "fill-opacity": wx.opacity, "fill-antialias": false },
    layout: { visibility: weatherVisibleInView(weatherView, wx.hidden, wx.navShow) && wx.shading !== "off" ? "visible" : "none" },
  }, shadingBefore());
  // Environment: a plain backdrop straight under the shading, so depth tints and bathymetry do not show
  // through it and shift its colours. Only while a layer is shaded there; the coast, marks and everything
  // drawn above the shading stay.
  map.addLayer({ id: "wx-backdrop", type: "background", paint: { "background-color": "#e3e9ed" }, layout: { visibility: "none" } }, "wx-cells");
  window.addEventListener("webmap-changed", () => moveShading());
  // isobars and wave-direction arrows
  map.addSource("wx-isobars", { type: "geojson", data: fc([]) });
  map.addSource("wx-wavedir", { type: "geojson", data: fc([]) });
  map.addLayer({ id: "wx-isobars", type: "line", source: "wx-isobars",
    paint: { "line-color": "#3d4f5c", "line-width": ["case", ["==", ["%", ["get", "hpa"], 8], 0], 1.6, 0.9], "line-opacity": 0.85 } }, beforeTop);
  map.addLayer({ id: "wx-isobar-labels", type: "symbol", source: "wx-isobars", filter: ["==", ["geometry-type"], "Point"],
    layout: { "icon-image": ["concat", "lbl:", ["to-string", ["get", "hpa"]]], "icon-allow-overlap": false, "icon-size": 0.85 } }, beforeTop);
  // isotherms (and salinity contours) over the ocean layers: fronts show as bunched lines
  map.addSource("wx-isotherms", { type: "geojson", data: fc([]) });
  map.addLayer({ id: "wx-isotherms", type: "line", source: "wx-isotherms",
    paint: { "line-color": "#1d2a33", "line-width": ["case", ["==", ["get", "target"], 1], 2.6, 0.8], "line-opacity": ["case", ["==", ["get", "target"], 1], 0.95, 0.65] } }, beforeTop);
  map.addLayer({ id: "wx-isotherm-labels", type: "symbol", source: "wx-isotherms", filter: ["==", ["geometry-type"], "Point"],
    layout: { "icon-image": ["concat", "lbl:", ["to-string", ["get", "hpa"]]], "icon-allow-overlap": false, "icon-size": 0.75 } }, beforeTop);
  if (!map.hasImage("wave-arrow")) map.addImage("wave-arrow", waveArrow(), { pixelRatio: 2 });
  map.addSource("wx-breaks", { type: "geojson", data: fc([]) });
  map.addLayer({ id: "wx-breaks", type: "fill", source: "wx-breaks",
    paint: { "fill-color": "#d81b60", "fill-antialias": false, "fill-opacity": ["interpolate", ["linear"], ["get", "g"], 0.02, 0.2, 0.12, 0.8] } }, beforeTop);
  map.addSource("wx-currdir", { type: "geojson", data: fc([]) });
  if (!map.hasImage("current-arrow")) map.addImage("current-arrow", waveArrow("#1e7bff"), { pixelRatio: 2 });
  map.addLayer({ id: "wx-currdir", type: "symbol", source: "wx-currdir",
    layout: { "icon-image": "current-arrow", "icon-rotate": ["get", "dir"], "icon-rotation-alignment": "map",
              "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-size": ["*", wx.size, ["get", "s"]] } }, beforeTop);
  map.addLayer({ id: "wx-wavedir", type: "symbol", source: "wx-wavedir",
    layout: { "icon-image": "wave-arrow", "icon-rotate": ["get", "dir"], "icon-rotation-alignment": "map",
              "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-size": wx.size } }, beforeTop);
  map.addLayer({
    id: "wx-barbs", type: "symbol", source: "wx-barbs",
    layout: {
      "icon-image": ["get", "icon"], "icon-rotate": ["get", "twd"], "icon-rotation-alignment": "map",
      "icon-anchor": "bottom", "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-size": wx.size,
    },
  }, beforeTop);

  // ---------- files ----------------------------------------------------------
  const fileSelect = $("wx-files"), fileButton = $("wx-load");
  let loadingPath = null;
  let unloadingPath = null;
  function fileMessage(text) {
    $("wx-where").textContent = text;
  }
  function syncFileSelection() {
    fileButton.disabled = !!loadingPath || !!unloadingPath || !fileSelect.value;
    for (const button of $("wx-loaded").querySelectorAll(".wx-unload")) button.disabled = !!loadingPath || !!unloadingPath;
  }
  fileSelect.addEventListener("change", syncFileSelection);
  fileButton.disabled = true;
  async function refreshFiles(report = false) {
    let files;
    try {
      files = await invoke("grib_files");
    } catch (e) {
      const message = `Could not list GRIB files: ${e}`;
      alertLine(message);
      fileMessage(message);
      return;
    }
    const selected = fileSelect.value;
    fileSelect.innerHTML = "";
    if (!files.length) fileSelect.append(new Option("No GRIB files found", ""));
    for (const f of files) {
      const mb = f.size ? `${(f.size / 1e6).toFixed(f.size < 1e7 ? 1 : 0)} MB` : "";
      const fetched = f.mtime ? `fetched ${fmtLocal(f.mtime)}` : "";
      const desc = f.info ? f.info.label : f.file;
      const opt = new Option(`${f.loaded ? "✓ " : ""}${desc} · ${fetched} · ${mb}`, f.path);
      opt.title = `${f.file}\n${f.dir}`;
      fileSelect.append(opt);
    }
    if (files.some((f) => f.path === selected)) fileSelect.value = selected;
    syncFileSelection();
    if (report && !loadingPath) fileMessage(files.length ? `Found ${files.length} GRIB ${files.length === 1 ? "file" : "files"}.` : "No GRIB files found in the GRIB folders.");
  }

  // Loading progress (the Rust side reports while it reads and decodes).
  const prog = $("wx-progress");
  function showProgress(pc, stage) {
    prog.hidden = false;
    prog.querySelector(".bar").style.width = `${pc}%`;
    prog.querySelector("span").textContent = `${stage === "reading" ? "Reading" : stage === "indexing" ? "Indexing" : "Decoding"} … ${pc} %`;
    fileMessage(`${prog.querySelector("span").textContent} · ${loadingPath}`);
  }
  listen("grib-progress", ({ payload }) => { if (payload.path === loadingPath) showProgress(payload.percent, payload.stage); }).catch(() => {});

  async function load(path) {
    if (!path || loadingPath || unloadingPath) {
      const message = loadingPath || unloadingPath ? "A GRIB operation is already in progress." : "Select a GRIB file to load.";
      alertLine(message);
      fileMessage(message);
      throw new Error(message);
    }
    loadingPath = path;
    syncFileSelection();
    $("wx-load").textContent = "…";
    showProgress(0, "reading");
    const t0 = performance.now();
    try {
      const s = await invoke("grib_load", { path });
      loaded.set(path, s);
      if (!display) display = path;
      // an ocean file: show it (sea temperature first) under whatever weather is shown
      const ps = paramsOf(s);
      const oceanKey = ["skin", "sst", "chl", "subt", "sss", "current"].find((k) => ps.has(LAYERS[k].params[0]));
      if (oceanKey && !LAYERS[wx.layer]?.ocean) {
        wx.layer = oceanKey;
        if (wx.shading === "off") wx.shading = "smooth";
        saveWx();
        applyLayer();
      }
      renderLoaded();
      setTimes();
      const message = `Loaded ${s.name} in ${((performance.now() - t0) / 1000).toFixed(1)} s`;
      alertLine(message);
      fileMessage(message);
      await refreshFiles();
    } catch (e) {
      const message = `Could not read GRIB: ${e}`;
      alertLine(message);
      fileMessage(message);
      throw e;
    } finally {
      loadingPath = null;
      prog.hidden = true;
      $("wx-load").textContent = "Load";
      syncFileSelection();
    }
  }

  function alertLine(msg) {
    $("wx-where").textContent = msg;
  }

  function renderLoaded() {
    const box = $("wx-loaded");
    box.innerHTML = "";
    if (loaded.size) {
      // what the two columns of buttons are for
      const head = document.createElement("div");
      head.className = "run run-head";
      head.innerHTML = `<span>Show</span><span class="grow"></span><span${loaded.size < 2 ? " hidden" : ""}>Compare</span><span class="pad"></span>`;
      box.append(head);
    }
    for (const [path, s] of loaded) {
      const row = document.createElement("div");
      row.className = "run";
      const wind = s.params.find((p) => p.param === "WindU") ?? s.params.find((p) => p.times?.length);
      const span = wind ? `${fmt(wind.times[0], true)} → ${fmt(wind.times.at(-1), true)}` : "no data";
      row.innerHTML = `<input type="radio" name="wx-show" title="Show this GRIB on the chart"><label></label>` +
        `<input type="checkbox" class="cmp" title="Include in the agreement displays (Show > Agreement of the loaded GRIBs)"><button class="wx-unload" title="Unload">×</button>`;
      const [radio, label, cmp, x] = row.children;
      radio.setAttribute("aria-label", `Show ${s.name}`);
      cmp.setAttribute("aria-label", `Include ${s.name} in agreement displays`);
      x.setAttribute("aria-label", `Unload ${s.name}`);
      cmp.hidden = loaded.size < 2;
      cmp.checked = !wx.cmpOff.includes(path);
      cmp.onchange = () => {
        wx.cmpOff = cmp.checked ? wx.cmpOff.filter((p) => p !== path) : [...wx.cmpOff, path];
        saveWx();
        renderLayers();
        draw();
        changed();
      };
      radio.checked = path === display;
      label.textContent = s.name;
      // the times it covers under the name: what tells two runs apart
      const covers = document.createElement("small");
      covers.textContent = span;
      label.append(covers);
      label.title = `${path}\n${span}`;
      radio.onchange = () => showGrib(path);
      label.onclick = () => showGrib(path);
      x.disabled = !!loadingPath || !!unloadingPath;
      x.onclick = () => unload(path);
      box.append(row);
    }
    // a wind GRIB and a wave GRIB of the same run: offer to put them in one file
    const has = (s, p) => s.params.some((x) => x.param === p);
    const winds = [...loaded].filter(([, s]) => has(s, "WindU") && !has(s, "WaveHeight"));
    const waves = [...loaded].filter(([, s]) => has(s, "WaveHeight") && !has(s, "WindU"));
    for (const [wp, ws] of winds) {
      const match = waves.find(([, vs]) => vs.ref_time === ws.ref_time);
      if (!match) continue;
      const [vp, vs] = match;
      const row = document.createElement("div");
      row.className = "wx-combine";
      row.innerHTML = `<span></span><button title="Write one file with the wind and the waves, load it and unload the two">Combine</button>`;
      row.firstChild.textContent = `${ws.name} and ${vs.name}: same run, wind and waves. Put them in one file?`;
      row.lastChild.onclick = () => combine(wp, vp);
      box.append(row);
    }
    syncMirrors();
    for (const fn of loadedListeners) fn();
  }

  async function unload(path) {
    if (!loaded.has(path) || loadingPath || unloadingPath) {
      const message = loadingPath || unloadingPath ? "A GRIB operation is already in progress." : "Select a loaded GRIB to unload.";
      alertLine(message);
      fileMessage(message);
      return;
    }
    unloadingPath = path;
    syncFileSelection();
    const name = loaded.get(path).name;
    try {
      await invoke("grib_unload", { path });
      loaded.delete(path);
      const wasShown = display === path;
      if (wasShown) display = loaded.keys().next().value ?? null;
      renderLoaded();
      setTimes();
      if (wasShown) changed();
      const message = `Unloaded ${name}. The file remains on disk.`;
      alertLine(message);
      fileMessage(message);
      await refreshFiles();
    } catch (e) {
      const message = `Could not unload GRIB: ${e}`;
      alertLine(message);
      fileMessage(message);
    } finally {
      unloadingPath = null;
      syncFileSelection();
    }
  }

  /** Combine a loaded wind GRIB and wave GRIB of the same run into one file, and load that instead. */
  async function combine(a, b) {
    try {
      alertLine("Combining wind and waves…");
      const path = await invoke("grib_combine", { a, b });
      await load(path);
      for (const p of [a, b]) { await invoke("grib_unload", { path: p }); loaded.delete(p); if (display === p) display = null; }
      renderLoaded();
      showGrib(path);
      await refreshFiles();
      alertLine(`Wind and waves are now in one file: ${path.split(/[\\/]/).pop()} (the two original files are kept).`);
    } catch (e) {
      alertLine(`Could not combine: ${e}`);
    }
  }

  /** Show this loaded GRIB on the chart (the time is kept when it is inside the new GRIB). */
  function showGrib(path) {
    if (!loaded.has(path)) return;
    display = path;
    for (const r of $("wx-loaded").querySelectorAll("input[type=radio]")) r.checked = false;
    const i = [...loaded.keys()].indexOf(path);
    const radio = $("wx-loaded").querySelectorAll("input[type=radio]")[i];
    if (radio) radio.checked = true;
    setTimes();
    syncMirrors();
    changed();
  }

  // ---------- time -------------------------------------------------------------
  const paramsOf = (s) => new Set(s.params.map((p) => (typeof p.param === "string" ? p.param : "")));
  /** Loaded GRIBs ticked for the agreement displays. */
  const comparePaths = () => [...loaded.keys()].filter((p) => !wx.cmpOff.includes(p));
  /** Layers the shown GRIB has data for; agreement layers when two or more ticked GRIBs have the data. */
  function available(path = display) {
    const s = path && loaded.get(path);
    if (!s) return [];
    const has = paramsOf(s);
    const cmp = comparePaths().map((p) => paramsOf(loaded.get(p)));
    return Object.keys(LAYERS).filter((k) => {
      const l = LAYERS[k];
      if (l.compare) return cmp.filter((h) => l.params.some((p) => h.has(p))).length >= 2;
      if (l.verify) return !!verifyTarget && loaded.has(verifyTarget.path);
      if (l.ocean) return !!oceanPath(l);
      return l.params.some((p) => has.has(p));
    });
  }
  const hasIn = (path, name) => !!loaded.get(path)?.params.some((p) => p.param === name);
  const hasParam = (name) => hasIn(display, name);
  /** The GRIB feeding an overlay or layer: the one chosen for it in the Weather panel if still loaded, else the shown one. */
  const OVERLAY_PARAMS = { arrows: ["WindU", "WindSpeed"], waveArrows: ["WaveDir"], isobars: ["Mslp"] };
  // an overlay is drawn from the file chosen for it, else the shown GRIB, else any loaded file that has its data
  function srcOf(name) {
    if (wx.src[name] && loaded.has(wx.src[name])) return wx.src[name];
    const want = OVERLAY_PARAMS[name];
    if (!want) return display;
    const has = (p) => want.some((n) => hasIn(p, n));
    return has(display) ? display : [...loaded.keys()].reverse().find(has) ?? display;
  }
  const keyOf = (l) => Object.keys(LAYERS).find((k) => LAYERS[k] === l);
  /** The file an ocean layer comes from: the one chosen, else the shown GRIB if it has it, else the newest loaded one that does. */
  function oceanPath(l) {
    const has = (p) => p && loaded.has(p) && paramsOf(loaded.get(p)).has(l.params[0]);
    if (has(wx.src[keyOf(l)])) return wx.src[keyOf(l)];
    if (has(display)) return display;
    return [...loaded.keys()].reverse().find(has) ?? null;
  }
  /** The file a shaded layer is read from. */
  const shadePath = (l) => (l.ocean ? oceanPath(l) ?? display : display);
  function renderLayers() {
    const av = available();
    if (av.length && !av.includes(wx.layer)) { wx.layer = av[0]; saveWx(); applyLayer(); }
    for (const sel of [...mirrors].filter((m) => m.isConnected).map((m) => m.querySelector(".layer"))) {
      if (!sel) continue;
      sel.innerHTML = "";
      const group = document.createElement("optgroup");
      group.label = "Agreement of the loaded GRIBs";
      const vgroup = document.createElement("optgroup");
      vgroup.label = verifyTarget ? `Verify ${verifyTarget.name} (+${Math.round(verifyTarget.lead)} h)` : "Verify: choose a forecast in the Verify dock";
      for (const [k, l] of Object.entries(LAYERS)) {
        const o = new Option(l.label, k);
        o.disabled = !av.includes(k);
        if (l.help) o.title = l.help;
        (l.compare ? group : l.verify ? vgroup : sel).append(o);
      }
      sel.append(group);
      sel.append(vgroup);
      sel.append(new Option("No shading", "none"));
      sel.value = wx.shading === "off" ? "none" : wx.layer;
    }
  }
  /** First and last time any parameter of a GRIB has data. */
  function spanOf(path) {
    return forecastSpan(loaded.get(path));
  }
  function syncPanel() {
    const span = display && spanOf(display);
    const coverage = $("side-wx-coverage");
    const outside = span && (t < span[0] || t > span[1]);
    coverage.textContent = span ? `${loaded.get(display)?.name ?? "Shown GRIB"}: ${fmt(span[0], true)} → ${fmt(span[1], true)}${outside ? " · Chart time outside this range" : ""}` : "No times for the shown GRIB.";
    coverage.classList.toggle("warn", !!outside);
    const slider = $("side-wx-time");
    slider.value = t;
    slider.disabled = !times.length;
    $("side-wx-when").textContent = times.length ? fmt(t) : "—";
    $("side-wx-source").textContent = display ? `Shown GRIB: ${loaded.get(display)?.name ?? display}` : "No GRIB loaded";
    document.getElementById("side")?.classList.toggle("no-grib", !loaded.size); // nothing to step through, time or compare
    $("side-wx-start").disabled = !span;
    $("side-wx-end").disabled = !span;
    $("side-wx-now").disabled = !times.length;
    for (const id of ["side-wx-prev", "side-wx-play", "side-wx-next"]) $(id).disabled = !times.length;
    const play = $("side-wx-play");
    const playLabel = playing ? "Pause chart time" : "Play chart time";
    play.title = playLabel;
    play.setAttribute("aria-label", playLabel);
    play.querySelector("path").setAttribute("d", playing ? "M8 5v14M16 5v14" : "M7 5 19 12 7 19z");
    syncFileSelection();
  }
  function setTimes() {
    // the slider covers every loaded GRIB; what each has at the slider time is shown as it is found there
    const all = new Set();
    let step = Infinity;
    for (const s of loaded.values()) {
      for (const p of s.params) {
        const ts = p.times ?? [];
        for (const v of ts) all.add(v);
        for (let i = 1; i < ts.length; i++) step = Math.min(step, ts[i] - ts[i - 1]);
      }
    }
    times = [...all].sort((a, b) => a - b);
    renderLayers();
    const slider = $("side-wx-time");
    if (!times.length) {
      slider.min = slider.max = 0;
      t = 0;
      if (playing) {
        clearInterval(playing);
        playing = null;
      }
      draw();
      return;
    }
    if (!Number.isFinite(step)) step = 3600;
    slider.min = times[0];
    slider.max = times.at(-1);
    timeStep = Math.max(900, Math.min(step, 3600));
    if (t < times[0] || t > times.at(-1)) {
      const now = Math.floor(Date.now() / 1000);
      t = now >= times[0] && now <= times.at(-1) ? now - (now % 3600) : times[0];
    }
    slider.value = t;
    draw();
  }

  function setT(v) {
    if (!times.length) return;
    $("side-wx-time-status").textContent = "";
    const s = $("side-wx-time");
    // whole seconds: with its step at "any" the slider gives fractions, which the GRIB commands (whole seconds) refuse
    t = Math.round(Math.min(Number(s.max), Math.max(Number(s.min), v)));
    s.value = t;
    draw();
  }
  const stepBy = (n) => {
    $("side-wx-time-status").textContent = "";
    const s = $("side-wx-time");
    t = Math.round(Math.min(Number(s.max), Math.max(Number(s.min), t + n * timeStep)));
    s.value = t;
    draw();
  };
  function togglePlay() {
    if (playing) { clearInterval(playing); playing = null; }
    else {
      playing = setInterval(() => {
        const s = $("side-wx-time");
        if (t >= Number(s.max)) { t = Number(s.min) - timeStep; }
        stepBy(1);
      }, 700);
    }
    syncMirrors();
  }
  $("side-wx-time").oninput = (e) => setT(Number(e.target.value));
  $("side-wx-prev").onclick = () => stepBy(-1);
  $("side-wx-next").onclick = () => stepBy(1);
  $("side-wx-play").onclick = togglePlay;
  $("side-wx-start").onclick = () => { const span = spanOf(display); if (span) setT(span[0]); };
  $("side-wx-end").onclick = () => { const span = spanOf(display); if (span) setT(span[1]); };
  $("side-wx-now").onclick = () => {
    const now = Math.floor(Date.now() / 1000);
    if (!times.length || now < times[0] || now > times.at(-1)) {
      $("side-wx-time-status").textContent = "Now is outside the loaded GRIBs' time range.";
      return;
    }
    setT(now);
  };

  // ---------- copies of the GRIB choice and time slider elsewhere (e.g. the compare dock) ----------
  // Each copy drives the same state as the data panel.
  const mirrors = new Set();
  function mountControls(el, { compact = false, bare = false } = {}) {
    el.classList.add("wx-mini");
    if (compact) el.dataset.compact = "1";
    el.innerHTML = (compact || bare ? "" : `<div class="chips"></div>`) +
      `<div class="wx-row time">` + (compact ? `<select class="gsel" title="The GRIB shown on the chart"></select>` : "") +
      `<button data-act="prev" title="Back one step">◀</button><button data-act="play" title="Play / pause">▶</button>` +
      `<button data-act="next" title="Forward one step">▶|</button><button data-act="now" title="Set the time to now">Now</button>` +
      (bare ? "" : `<select class="layer" title="What the shading shows"></select>`) +
      `<input type="range" class="grow"><span class="when">—</span></div>`;
    el.querySelector(".gsel")?.addEventListener("change", (e) => showGrib(e.target.value));
    const sl = el.querySelector("input[type=range]");
    sl.oninput = () => setT(Number(sl.value));
    el.querySelector('[data-act="prev"]').onclick = () => stepBy(-1);
    el.querySelector('[data-act="next"]').onclick = () => stepBy(1);
    el.querySelector('[data-act="play"]').onclick = togglePlay;
    el.querySelector('[data-act="now"]').onclick = () => $("side-wx-now").click();
    const lsel = el.querySelector(".layer");
    if (lsel) lsel.onchange = (e) => setLayer(e.target.value);
    mirrors.add(el);
    syncMirror(el, true);
    renderLayers();
  }
  function syncMirror(el, chips) {
    const gsel = el.querySelector(".gsel");
    if (chips && gsel) {
      gsel.innerHTML = "";
      if (!loaded.size) gsel.append(new Option("No GRIB loaded", ""));
      for (const [path, s] of loaded) gsel.append(new Option(s.name, path));
      gsel.value = display ?? "";
    } else if (chips) {
      const box = el.querySelector(".chips");
      if (!box) { /* bare: no chooser */ } else {
      box.innerHTML = "";
      if (!loaded.size) box.innerHTML = `<span class="wx-note">No GRIB loaded: load one in the Weather panel.</span>`;
      for (const [path, s] of loaded) {
        const lab = document.createElement("label");
        lab.className = `chip${path === display ? " on" : ""}`;
        lab.title = `Show ${s.name} on the chart`;
        lab.innerHTML = `<input type="checkbox"><span></span>`;
        lab.querySelector("input").checked = path === display;
        lab.querySelector("span").textContent = s.name;
        lab.querySelector("input").onchange = (e) => { if (e.target.checked) showGrib(path); else e.target.checked = true; };
        box.append(lab);
      }
      }
    }
    const main = $("side-wx-time"), sl = el.querySelector("input[type=range]");
    sl.min = main.min; sl.max = main.max; sl.step = main.step; sl.value = t;
    sl.disabled = !times.length;
    el.querySelector(".when").textContent = times.length ? (el.dataset.compact ? fmt(t, true) : fmt(t)) : "—";
    el.querySelector('[data-act="play"]').textContent = playing ? "❚❚" : "▶";
  }
  function syncMirrors(timeOnly) {
    syncPanel();
    for (const el of mirrors) {
      if (!el.isConnected) { mirrors.delete(el); continue; }
      syncMirror(el, !timeOnly);
    }
  }

  // ---------- drawing --------------------------------------------------------------
  let pending = false;
  let drawVersion = 0;
  async function draw() {
    drawVersion++;
    const slider = $("side-wx-time");
    slider.step = times.length ? sliderStepForTime(t, Number(slider.min), timeStep) : timeStep;
    slider.value = t;
    syncMirrors(true);
    for (const fn of timeListeners) fn(t);
    updatePoint();
    if (pending) return;
    pending = true;
    requestAnimationFrame(async () => {
      pending = false;
      const version = drawVersion;
      if (!display || !times.length || !weatherVisibleInView(weatherView, wx.hidden, wx.navShow)) {
        map.getSource("wx-cells").setData(fc([]));
        map.getSource("wx-barbs").setData(fc([]));
        map.getSource("wx-wavedir").setData(fc([]));
        map.getSource("wx-currdir").setData(fc([]));
        map.getSource("wx-breaks").setData(fc([]));
        map.getSource("wx-isobars").setData(fc([]));
        map.getSource("wx-isotherms").setData(fc([]));
        showCompareNote({}, null);
        return;
      }
      const b = map.getBounds();
      const bbox = [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()];
      const layer = LAYERS[wx.layer] ?? LAYERS.wind;
      const timeAllowed = (path, params) => weatherView !== "env" ||
        dataTimeStatus(loaded.get(path), params, t, allowPastAnalysis).state !== "missing";
      const arrowPts = Math.round(DENSITY_POINTS[wx.density - 1]);
      const shade = wx.shading !== "off";
      // Barbs and wave arrows at the chosen density; shading at full GRIB resolution (up to 20 000 cells).
      const curPath = wx.currArrows ? oceanPath(LAYERS.current) : null;
      const curPts = Math.max(40, arrowPts / 2);
      const pBarbs = srcOf("arrows"), pWaves = srcOf("waveArrows"), pIso = srcOf("isobars");
      const bsrc = loaded.has(wx.src.breaks) ? wx.src.breaks : null;
      const breakKey = wx.breaks ? ["sst", "skin"].find((k) => (bsrc ? hasIn(bsrc, LAYERS[k].params[0]) : oceanPath(LAYERS[k]))) : null;
      const breakPath = breakKey ? bsrc ?? oceanPath(LAYERS[breakKey]) : null;
      drawError = null;
      const [g, gs, gw, gp, [gcd, gcs], gb] = await Promise.all([
        wx.arrows && (hasIn(pBarbs, "WindU") || hasIn(pBarbs, "WindSpeed")) ? invoke("grib_wind_grid", { path: pBarbs, t, bbox, maxPoints: arrowPts }).catch(failed) : null,
        !shade ? null : layer.verify
          ? (verifyTarget ? invoke("grib_verify_grid", { paths: [], t: verifyTarget.t, at: t, forecast: verifyTarget.path, kind: layer.verify, bbox, maxPoints: 20000 })
            .catch((e) => ({ error: String(e) })) : { error: "Choose a forecast to verify in the Verify dock." })
          : layer.compare
          ? invoke("grib_compare", { paths: comparePaths(), metric: layer.compare, t, bbox, maxPoints: 20000 }).catch((e) => ({ error: String(e) }))
          : timeAllowed(shadePath(layer), layer.params)
          ? invoke("grib_grid", { path: shadePath(layer), layer: wx.layer, t, bbox, maxPoints: 20000 }).catch(failed) : null,
        wx.waveArrows && hasIn(pWaves, "WaveDir") ? invoke("grib_grid", { path: pWaves, layer: "wavedir", t, bbox, maxPoints: Math.max(40, arrowPts / 2) }).catch(failed) : null,
        wx.isobars && hasIn(pIso, "Mslp") ? invoke("grib_grid", { path: pIso, layer: "mslp", t, bbox, maxPoints: 20000 }).catch(failed) : null,
        curPath && timeAllowed(curPath, ["CurrentU"])
          ? Promise.all(["currentdir", "current"].map((layer) => invoke("grib_grid", { path: curPath, layer, t, bbox, maxPoints: curPts }).catch(failed)))
          : [null, null],
        breakPath && timeAllowed(breakPath, LAYERS[breakKey].params) ? invoke("grib_grid", { path: breakPath, layer: breakKey, t, bbox, maxPoints: 20000 }).catch(failed) : null,
      ]);
      if (version !== drawVersion) return;
      showDrawError();
      drawLegend(); // the key and the filter bar follow what was just drawn (a newly loaded GRIB too)
      const cells = [], barbs = [], waves = [], currents = [];
      if (g) {
        for (let j = 0; j < g.nj; j++) {
          for (let i = 0; i < g.ni; i++) {
            const k = j * g.ni + i;
            const tws = g.tws[k], twd = g.twd[k];
            if (tws == null) continue;
            const lon = g.lon0 + i * g.dlon, lat = g.lat0 + j * g.dlat;
            barbs.push({ type: "Feature", properties: { tws, twd, icon: barbId(tws, lat) },
              geometry: { type: "Point", coordinates: [lon, lat] } });
          }
        }
      }
      showCompareNote(layer, gs);
      // ocean layers: colours from the skipper's filter (not the view); contours within it
      let contours = [];
      if (filterable(layer) && gs && !gs.error) {
        updateSeaRange(wx.layer, shadePath(layer), gs);
        const f = wx.seaFilter[wx.layer];
        // a layer without the sea colours: only the target's own line, once the filter has been set
        if (!layer.auto && f?.set && f.tg != null) {
          contours.push(...isobars(gs, 0, [f.tg]).map((x) => ({ ...x, properties: { hpa: +f.tg.toFixed(2), target: 1 } })));
        }
      }
      if (layer.auto && gs && !gs.error) {
        const f = wx.seaFilter[wx.layer];
        if (f && layer.contour && wx.isotherms && layer.log) {
          // chlorophyll: lines at tidy values, labelled in mg/m³
          const lv = CHL_LEVELS.filter((x) => Math.log10(x) >= f.lo - 1e-9 && Math.log10(x) <= f.hi + 1e-9);
          const pick = lv.length > 9 ? lv.filter((_, i) => i % 2 === 0) : lv;
          for (const x of pick) contours.push(...isobars(gs, 0, [Math.log10(x)]).map((c) => ({ ...c, properties: { hpa: fmtChl(x) } })));
          if (f.tg != null) contours.push(...isobars(gs, 0, [f.tg]).map((c) => ({ ...c, properties: { hpa: fmtChl(10 ** f.tg), target: 1 } })));
        } else if (f && layer.contour && wx.isotherms) {
          const step = niceStep(f.lo, f.hi);
          const dp = step < 0.1 ? 2 : step < 1 ? 1 : 0;
          contours = isobars(gs, step)
            .map((x) => ({ ...x, properties: { hpa: +(Math.round(x.properties.hpa / step) * step).toFixed(dp) } }))
            .filter((x) => x.properties.hpa >= f.lo - 1e-9 && x.properties.hpa <= f.hi + 1e-9);
          // the target's own line
          if (f.tg != null) contours.push(...isobars(gs, 0, [f.tg]).map((x) => ({ ...x, properties: { hpa: +f.tg.toFixed(dp + 1), target: 1 } })));
        }
      }
      map.getSource("wx-isotherms").setData(fc(contours));
      if (gs && !gs.error) {
        const hx = gs.dlon / 2, hy = gs.dlat / 2, min = layer.min ?? -Infinity;
        for (let j = 0; j < gs.nj; j++) {
          for (let i = 0; i < gs.ni; i++) {
            const v = gs.values[j * gs.ni + i];
            if (v == null || v < min) continue;
            const lon = gs.lon0 + i * gs.dlon, lat = gs.lat0 + j * gs.dlat;
            cells.push({ type: "Feature", properties: { v },
              geometry: { type: "Polygon", coordinates: [[[lon - hx, lat - hy], [lon + hx, lat - hy], [lon + hx, lat + hy], [lon - hx, lat + hy], [lon - hx, lat - hy]]] } });
          }
        }
      }
      if (gw) {
        for (let j = 0; j < gw.nj; j++) {
          for (let i = 0; i < gw.ni; i++) {
            const d = gw.values[j * gw.ni + i];
            if (d == null) continue;
            // arrows point the way the waves travel (direction is where they come from)
            waves.push({ type: "Feature", properties: { dir: (d + 180) % 360 }, geometry: { type: "Point", coordinates: [gw.lon0 + i * gw.dlon, gw.lat0 + j * gw.dlat] } });
          }
        }
      }
      if (gcd && gcs) {
        for (let j = 0; j < gcd.nj; j++) {
          for (let i = 0; i < gcd.ni; i++) {
            const d = gcd.values[j * gcd.ni + i], kn = gcs.values[j * gcs.ni + i];
            if (d == null || kn == null || kn < 0.05) continue;
            // longer arrows for stronger current (0.5 at slack, 1.6 at 3 kn and over)
            currents.push({ type: "Feature", properties: { dir: d, s: 0.5 + Math.min(kn, 3) / 3 * 1.1 },
              geometry: { type: "Point", coordinates: [gcd.lon0 + i * gcd.dlon, gcd.lat0 + j * gcd.dlat] } });
          }
        }
      }
      map.getSource("wx-currdir").setData(fc(currents));
      map.getSource("wx-breaks").setData(fc(gb && !gb.error ? fronts(gb) : []));
      map.getSource("wx-cells").setData(fc(cells));
      map.getSource("wx-barbs").setData(fc(barbs));
      map.getSource("wx-wavedir").setData(fc(waves));
      map.getSource("wx-isobars").setData(fc(gp ? isobars(gp, isobarStep()) : []));
    });
  }
  map.on("moveend", draw);

  /** Under the legend: which GRIBs an agreement display compares (or why it can't). */
  function showCompareNote(layer, r) {
    const el = $("wx-cmp-note");
    if (!(layer.compare || layer.verify) || wx.shading === "off") { el.hidden = true; return; }
    el.hidden = false;
    if (!r || r.error) { el.textContent = r?.error ?? "Nothing to compare here."; return; }
    if (layer.verify) {
      const vals = r.values.filter((v) => v != null);
      const ref = r.used.slice(0, -1).map((u) => u.name).join(", ");
      const mean = vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null;
      const at = t === verifyTarget.t ? `against hour 0 of ${ref}, ${fmt(verifyTarget.t)}` : `against the later run (${ref}) at ${fmt(t)}; hour 0 is ${fmt(verifyTarget.t)}`;
      el.textContent = `${layer.help} ${verifyTarget.name} at +${Math.round(verifyTarget.lead)} h ${at}.` +
        (mean != null ? ` In view: mean ${mean.toFixed(1)} ${layer.unit}.` : "");
      return;
    }
    const vals = r.values.filter((v) => v != null);
    const mean = vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null;
    const runs = [...r.used].sort((a, b) => a.run - b.run);
    const names = runs.map((u) => `${u.name.replace(/ \d+ \w+ \d+Z$/, "")} ${fmtRun(u.run)}`);
    el.textContent = `${layer.help} Comparing ${r.used.length}: ${names.join(", ")}.` +
      (r.members ? ` Every one is an ensemble, so each of the ${r.layers} members counts as a layer.` : "") +
      (mean != null ? ` In view: mean ${mean.toFixed(1)} ${layer.unit}, max ${Math.max(...vals).toFixed(1)} ${layer.unit}.` : "") +
      (layer.compare === "trend" ? ` Newest ${names.at(-1)} − oldest ${names[0]}.` : "");
  }
  const fmtRun = (t) => {
    const d = new Date(t * 1000);
    return `${d.getUTCDate()} ${d.toLocaleString("en-NZ", { month: "short", timeZone: "UTC" })} ${String(d.getUTCHours()).padStart(2, "0")}Z`;
  };

  // ---------- comparison table --------------------------------------------------------
  const SEA_KEYS = ["sst", "skin", "sss", "subt", "subs", "cur", "depth", "chl", "ssh"];
  /** The sea values of one file at the cursor, as short labelled pieces. */
  function seaParts(r) {
    const out = [];
    if (r.sst != null) out.push(`Sea ${r.sst.toFixed(1)} °C`);
    if (r.skin != null) out.push(`Satellite ${r.skin.toFixed(1)} °C`);
    if (r.sss != null) out.push(`Salinity ${r.sss.toFixed(2)}`);
    if (r.subt != null) out.push(`${r.depth ?? "?"} m ${r.subt.toFixed(1)} °C`);
    if (r.subs != null) out.push(`${r.depth ?? "?"} m salinity ${r.subs.toFixed(2)}`);
    if (r.cur) out.push(`Current ${r.cur.kn.toFixed(1)} kn ${String(r.cur.set).padStart(3, "0")}°`);
    if (r.chl != null) out.push(`Chl-a ${fmtChl(r.chl)} mg/m³`);
    if (r.ssh != null) out.push(`Sea height ${r.ssh >= 0 ? "+" : ""}${r.ssh.toFixed(2)} m`);
    return out;
  }
  let pointTimer = null;
  function updatePoint() {
    if (pointTimer) return;
    pointTimer = setTimeout(async () => {
      pointTimer = null;
      const body = document.querySelector("#wx-point tbody");
      if (!cursor || !loaded.size || !times.length) {
        body.innerHTML = "";
        $("wx-sea-point").innerHTML = "";
        for (const fn of pointListeners) fn(null, cursor);
        return;
      }
      const at = cursor, tt = t, shownPath = display, requestedView = weatherView, requestedPast = allowPastAnalysis;
      const result = await invoke("grib_point", { lat: at[1], lon: at[0], t: tt }).catch(() => []) || [];
      if (at !== cursor || tt !== t || shownPath !== display || requestedView !== weatherView || requestedPast !== allowPastAnalysis) { updatePoint(); return; }
      const rows = result.filter((r) => loaded.has(r.path)).map((r) =>
        weatherView === "env" ? applyPointTimePolicy(r, loaded.get(r.path), tt, allowPastAnalysis) : r);
      let shown = (rows || []).find((r) => r.path === display) ?? null;
      // sea readings from whichever loaded file has them (MUR, HYCOM), under the shown GRIB's weather
      // the shown GRIB's values, with anything it lacks (sea data, or weather when an ocean file is shown) from the other loaded files
      // (a file only gives values for times it covers, so only what exists at the slider time is shown)
      if (shown) {
        const extra = {};
        const src = {}; // key → the name of the file a borrowed value came from
        const past = { ...shown.past };
        for (const k of ["tws", "twd", "gust", "mslp", "wave", "period", "wavedir", "temp", "rain", ...SEA_KEYS]) {
          if (shown[k] != null) continue;
          const from = (rows || []).find((r) => r[k] != null);
          if (from) {
            extra[k] = from[k]; src[k] = loaded.get(from.path)?.name ?? from.name ?? "another file";
            if (from.past?.[k] != null) past[k] = from.past[k];
          }
        }
        shown = { ...shown, ...extra, src, past };
      }
      for (const fn of pointListeners) fn(shown && { ...shown, t: tt, name: loaded.get(display)?.name ?? shown.name }, at);
      body.innerHTML = "";
      const seaBox = $("wx-sea-point");
      seaBox.innerHTML = "";
      for (const r of rows) {
        const weather = ["tws", "gust", "mslp", "wave", "temp", "rain"].some((k) => r[k] != null);
        const sea = seaParts(r);
        // weather in the table; the sea on its own lines under it (an ocean file has no weather)
        if (weather || !sea.length) {
          const tr = document.createElement("tr");
          const cells = [r.name, n(r.tws, 1), r.twd == null ? "—" : String(r.twd).padStart(3, "0") + "°", n(r.gust, 0), n(r.mslp, 0),
            r.wave == null ? "—" : `${n(r.wave, 1)}${r.period != null ? ` / ${r.period}s` : ""}`];
          for (const c of cells) { const td = document.createElement("td"); td.textContent = c; tr.append(td); }
          if (r.path === display) tr.style.fontWeight = "600";
          body.append(tr);
        }
        if (sea.length) {
          const d = document.createElement("div");
          d.className = `row${r.path === display ? " shown" : ""}`;
          const src = document.createElement("span");
          src.className = "src";
          const pastTimes = [...new Set(Object.values(r.past ?? {}))];
          src.textContent = `${r.name}${pastTimes.length ? ` · past analysis ${pastTimes.map((v) => fmt(v)).join(", ")}` : ""}`;
          const vals = document.createElement("span");
          vals.className = "vals";
          for (const s of sea) { const x = document.createElement("span"); x.textContent = s; vals.append(x); }
          d.append(src, vals);
          seaBox.append(d);
        }
      }
    }, 120);
  }
  map.on("mousemove", (e) => { cursor = [e.lngLat.lng, e.lngLat.lat]; updatePoint(); });
  map.on("click", (e) => { cursor = [e.lngLat.lng, e.lngLat.lat]; updatePoint(); }); // touch: a tap is the cursor

  // load() has already reported any error.
  $("wx-load").onclick = () => { load($("wx-files").value).catch(() => {}); };
  $("wx-refresh").onclick = () => refreshFiles(true);

  // ---------- display settings ----------------------------------------------------------
  const saveWx = () => {
    try {
      const saved = loadWxViews();
      saved[weatherView] = structuredClone(wx);
      localStorage.setItem(WX_VIEWS, JSON.stringify(saved));
      if (weatherView === "nav") localStorage.setItem("racenav.wx", JSON.stringify(wx));
    } catch (error) { console.error("Could not save weather settings by view:", error); }
  };
  $("env-use-past").onchange = (event) => {
    allowPastAnalysis = event.target.checked;
    wx.pastAnalysis = allowPastAnalysis;
    saveWx();
    draw();
    changed();
  };
  const dens = $("wx-density"), size = $("wx-size"), shade = $("wx-shading"), opac = $("wx-opacity");
  dens.value = wx.density; size.value = wx.size; shade.value = wx.shading; opac.value = wx.opacity;
  const showVals = () => {
    $("wx-density-v").textContent = wx.density === 10 ? "every point" : String(wx.density);
    $("wx-size-v").textContent = `${Math.round(wx.size * 100)} %`;
    $("wx-opacity-v").textContent = `${Math.round(wx.opacity * 100)} %`;
  };
  showVals();
  dens.oninput = () => { wx.density = Number(dens.value); showVals(); saveWx(); draw(); };
  size.oninput = () => { wx.size = Number(size.value); showVals(); saveWx(); map.setLayoutProperty("wx-barbs", "icon-size", wx.size); map.setLayoutProperty("wx-currdir", "icon-size", ["*", wx.size, ["get", "s"]]); };
  opac.oninput = () => { wx.opacity = Number(opac.value); showVals(); saveWx(); map.setPaintProperty("wx-cells", "fill-opacity", wx.opacity); };
  const landSel = $("wx-land");
  landSel.value = wx.land;
  landSel.onchange = () => { wx.land = landSel.value; saveWx(); moveShading(); };
  // ---------- sea filter: low, target, high ----------------------------------------------
  // The full grid range at the current chart time, independent of the map bounds.
  const seaRange = {};
  function updateSeaRange(key, path, grid) {
    const old = seaRange[key], previous = wx.seaFilter[key];
    const next = filterRangeForGrid(grid, previous);
    if (!next) return;
    const changedRange = !old || old.path !== path || old.min !== next.min || old.max !== next.max;
    if (!changedRange && previous === next.filter) return;
    seaRange[key] = { path, min: next.min, max: next.max, tracks: next.filter === previous ? old?.tracks : undefined };
    wx.seaFilter[key] = next.filter;
    // an automatic range is not saved: it is worked out again at every time step
    applyLayer();
  }
  const roundTo = (v, q) => Math.round(v / q) * q;
  function filterTrack() {
    const r = seaRange[wx.layer], f = wx.seaFilter[wx.layer];
    if (!r || !f) return null;
    r.tracks ??= {};
    let track = r.tracks[weatherView];
    if (!track || f.lo < track.min || f.hi > track.max) {
      track = f.set ? seaTrackRange(f, r) : { min: r.min, max: r.max };
      r.tracks[weatherView] = track;
    }
    return track;
  }
  function refitFilterTrack(edge = null) {
    const r = seaRange[wx.layer], f = wx.seaFilter[wx.layer], previous = filterTrack();
    if (r && f) r.tracks[weatherView] = seaTrackRange(f, r, previous, edge);
  }

  /** Filter widgets under the colour bar. */
  const filterBoxes = [];
  function drawSeaFilter(layer) {
    for (const { el: box, all } of filterBoxes) {
      if (!box.isConnected) continue;
      const f = wx.seaFilter[wx.layer], r = filterTrack();
      box.hidden = !((layer.auto || (all && filterable(layer))) && f && r && wx.shading !== "off");
      if (box.hidden) continue;
      const span = Math.max(r.max - r.min, 1e-6);
      const pc = (v) => `${Math.min(100, Math.max(0, ((v - r.min) / span) * 100))}%`;
      const grad = box.querySelector(".grad");
      grad.style.left = pc(f.lo);
      grad.style.width = `calc(${pc(f.hi)} - ${pc(f.lo)})`;
      const w = f.hi - f.lo;
      const stops = filterInUse(layer, f) ? filterColourStyle(layer, f, wx.shading).stops : fixedStops(layer, f.lo, f.hi);
      grad.style.background = `linear-gradient(to right, ${stops.map(([v, c]) => `${c} ${((v - f.lo) / w) * 100}%`).join(", ")})`;
      for (const k of ["lo", "hi"]) box.querySelector(`.h.${k}`).style.left = pc(f[k]);
      const th = box.querySelector(".h.tg");
      th.style.left = pc(f.tg ?? (f.lo + f.hi) / 2);
      th.classList.toggle("off", f.tg == null);
      const q = seaQuantum(r.min, r.max);
      const dp = Math.max(0, -Math.round(Math.log10(q)));
      const fmtv = (v) => (layer.log ? fmtChl(10 ** v) : `${v.toFixed(dp)}${layer.unit === "°C" ? "°" : ""}`);
      box.querySelector(".v.lo").textContent = fmtv(f.lo);
      box.querySelector(".v.tg").textContent = f.tg == null ? "no target" : `target ${fmtv(f.tg)}`;
      box.querySelector(".v.hi").textContent = fmtv(f.hi);
      box.querySelector(".v.min").textContent = fmtv(r.min);
      box.querySelector(".v.max").textContent = fmtv(r.max);
      // the ends of the track only when they say something the limits do not
      box.querySelector(".v.min").style.visibility = fmtv(r.min) === fmtv(f.lo) ? "hidden" : "";
      box.querySelector(".v.max").style.visibility = fmtv(r.max) === fmtv(f.hi) ? "hidden" : "";
      // a set limit that leaves nothing to show at this time: say so, the chart would just be blank
      const raw = seaRange[wx.layer], ends = filterBounds(f);
      const nothing = !!raw && filterInUse(layer, f) && ((ends.hi && f.hi < raw.min) || (ends.lo && f.lo > raw.max));
      let none = box.querySelector(".none");
      if (!none) { none = document.createElement("div"); none.className = "wx-note none"; box.append(none); }
      none.hidden = !nothing;
      if (nothing) none.textContent = `Nothing ${ends.lo && ends.hi ? `between ${fmtv(f.lo)} and ${fmtv(f.hi)}` : ends.lo ? `above ${fmtv(f.lo)}` : `below ${fmtv(f.hi)}`} at this time: the data runs from ${fmtv(raw.min)} to ${fmtv(raw.max)}. Press Fit to show it all.`;
      box.querySelector("[data-filter-clear]").disabled = f.tg == null;
      box.dataset.unit = layer.unit;
      box.title = `${layer.label} (${layer.unit}): drag the outer pointers to set the lowest and highest shown, the middle one to the target (bold contour); click a number to type it. Fit: back to the full range at chart time.`;
    }
  }
  /** Make a filter widget work: drag the pointers, click the numbers to type them, Fit. */
  function wireFilter(box) {
    const track = box.querySelector(".track");
    let drag = null;
    track.addEventListener("pointerdown", (e) => {
      const f = wx.seaFilter[wx.layer], r = filterTrack();
      if (!f || !r) return;
      const rect = track.getBoundingClientRect();
      const v = r.min + ((e.clientX - rect.left) / rect.width) * (r.max - r.min);
      // the pointer clicked, else the nearest one (the target wins a tie)
      // with no target set, only the target pointer itself can set one
      const k = e.target.classList.contains("h") ? [...e.target.classList].find((c) => ["lo", "tg", "hi"].includes(c))
        : (f.tg == null ? ["lo", "hi"] : ["tg", "lo", "hi"]).reduce((a, b) => (Math.abs(f[b] - v) < Math.abs(f[a] - v) ? b : a));
      drag = { k, rect };
      track.setPointerCapture(e.pointerId);
      move(e);
    });
    const set = (k, v, typed = false) => {
      const f = wx.seaFilter[wx.layer], trackRange = filterTrack(), full = seaRange[wx.layer];
      const q = seaQuantum(trackRange.min, trackRange.max);
      const r = typed ? full : trackRange;
      v = Math.min(r.max, Math.max(r.min, roundTo(v, q)));
      if (!f.set) { f.loSet = false; f.hiSet = false; }
      if (k === "lo") f.loSet = true; else if (k === "hi") f.hiSet = true;
      if (k === "lo") f.lo = Math.min(v, f.tg == null ? f.hi - q : f.tg - q);
      else if (k === "hi") f.hi = Math.max(v, f.tg == null ? f.lo + q : f.tg + q);
      else f.tg = Math.min(Math.max(v, f.lo + q), f.hi - q);
      f.lo = +f.lo.toFixed(6); f.hi = +f.hi.toFixed(6);
      if (f.tg != null) {
        f.tg = Math.min(Math.max(f.tg, f.lo), f.hi);
        f.tg = +f.tg.toFixed(6);
      }
      f.set = true;
      applyLayer();
    };
    const move = (e) => {
      if (!drag) return;
      const r = filterTrack();
      set(drag.k, r.min + ((e.clientX - drag.rect.left) / drag.rect.width) * (r.max - r.min));
    };
    track.addEventListener("pointermove", move);
    const finishDrag = () => {
      if (!drag) return;
      const f = wx.seaFilter[wx.layer], r = filterTrack();
      const edge = drag.k === "lo" && f.lo <= r.min ? "lo" : drag.k === "hi" && f.hi >= r.max ? "hi" : null;
      drag = null;
      refitFilterTrack(edge);
      saveWx(); applyLayer(); draw();
    };
    track.addEventListener("pointerup", finishDrag);
    track.addEventListener("pointercancel", finishDrag);
    // double-click the target pointer to clear the target (the track captures the pointer, so the event arrives at the track)
    track.addEventListener("dblclick", (e) => {
      const f = wx.seaFilter[wx.layer];
      if (!f || f.tg == null) return;
      const h = box.querySelector(".h.tg").getBoundingClientRect();
      if (Math.abs(e.clientX - (h.left + h.width / 2)) > Math.max(14, h.width)) return;
      f.tg = null;
      saveWx(); applyLayer(); draw();
    });
    // typing a value: click the number, Enter to set (a chlorophyll value is typed in mg/m³)
    for (const k of ["lo", "tg", "hi"]) {
      const span = box.querySelector(`.v.${k}`);
      span.style.cursor = "text";
      span.addEventListener("click", () => {
        const f = wx.seaFilter[wx.layer], layer = LAYERS[wx.layer];
        if (!f || !layer || span.querySelector("input")) return;
        const input = document.createElement("input");
        input.type = "number";
        input.step = "any";
        input.setAttribute("aria-label", `${{ lo: "Low", tg: "Target", hi: "High" }[k]} ${layer.label} (${layer.unit}). Press Enter to apply`);
        const cur = f[k] ?? (f.lo + f.hi) / 2;
        input.value = String(+(layer.log ? 10 ** cur : cur).toPrecision(4));
        input.style.width = "64px";
        span.textContent = "";
        span.append(input);
        input.focus();
        input.select();
        let finished = false;
        const done = (ok) => {
          if (finished) return;
          finished = true;
          const x = parseFloat(input.value);
          if (ok && Number.isFinite(x) && !(layer.log && x <= 0)) {
            set(k, layer.log ? Math.log10(x) : x, true);
            refitFilterTrack();
            saveWx(); applyLayer(); draw();
          } else drawSeaFilter(layer);
        };
        input.onkeydown = (e) => { if (e.key === "Enter") done(true); else if (e.key === "Escape") done(false); e.stopPropagation(); };
        input.onblur = () => done(true);
      });
      box.querySelector(`[data-filter-edit="${k}"]`).onclick = () => span.click();
    }
    box.querySelector("[data-filter-clear]").onclick = () => {
      const f = wx.seaFilter[wx.layer];
      if (!f || f.tg == null) return;
      f.tg = null;
      saveWx(); applyLayer(); draw();
    };
    box.querySelector(".fit").onclick = () => {
      const r = seaRange[wx.layer];
      if (!r) return;
      wx.seaFilter[wx.layer] = { lo: r.min, hi: r.max, tg: null };
      if (r.tracks) r.tracks[weatherView] = { min: r.min, max: r.max };
      saveWx(); applyLayer(); draw();
    };
  }
  /** A filter widget for the Weather panel, working for every shaded layer. */
  function mountFilter(el) {
    el.className = "seafilter";
    el.innerHTML = `
      <div class="track"><div class="grad"></div><span class="h lo"></span><span class="h tg"></span><span class="h hi"></span></div>
      <div class="vals"><span class="v min"></span><span class="v lo"></span><span class="v tg"></span><span class="v hi"></span><span class="v max"></span><button class="fit" title="Back to the full range at chart time">Fit</button></div>
      <div class="filter-actions">
      <button type="button" data-filter-edit="lo" title="Set low" aria-label="Set low"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12m-5-5 5 5 5-5M4 20h16"/></svg></button>
      <button type="button" data-filter-edit="hi" title="Set high" aria-label="Set high"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 21V9m-5 5 5-5 5 5M4 4h16"/></svg></button>
      <button type="button" data-filter-edit="tg" title="Set target" aria-label="Set target"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="7"/><path d="M12 2v5m0 10v5M2 12h5m10 0h5"/><circle cx="12" cy="12" r="2"/></svg></button>
      <button type="button" data-filter-clear title="Clear target" aria-label="Clear target"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="7"/><path d="M12 2v5m0 10v5M2 12h5m10 0h5M4 4l16 16"/></svg></button></div>`;
    wireFilter(el);
    filterBoxes.push({ el, all: true });
    drawLegend();
  }
  function applyLayer() {
    const visible = weatherVisibleInView(weatherView, wx.hidden, wx.navShow);
    for (const id of ["wx-barbs", "wx-wavedir", "wx-currdir", "wx-breaks", "wx-isobars", "wx-isobar-labels", "wx-isotherms", "wx-isotherm-labels"]) {
      map.setLayoutProperty(id, "visibility", visible ? "visible" : "none");
    }
    map.setLayoutProperty("wx-cells", "visibility", visible && wx.shading !== "off" ? "visible" : "none");
    map.setLayoutProperty("wx-backdrop", "visibility", visible && wx.shading !== "off" && weatherView === "env" && display ? "visible" : "none");
    const lay = LAYERS[wx.layer] ?? LAYERS.wind;
    const sf = filterInUse(lay, wx.seaFilter[wx.layer]) ? wx.seaFilter[wx.layer] : null;
    // only the ends that were set hide anything: beyond an end left alone the last colour carries on
    const limits = [];
    if (sf && filterBounds(sf).lo) limits.push([">=", ["get", "v"], sf.lo]);
    if (sf && filterBounds(sf).hi) limits.push(["<=", ["get", "v"], sf.hi]);
    map.setFilter("wx-cells", limits.length ? ["all", ...limits] : null);
    if (wx.shading !== "off") map.setPaintProperty("wx-cells", "fill-color", sf
      ? filterColourStyle(lay, sf, wx.shading).expression
      : shadingExpression(lay.auto ? "smooth" : wx.shading, lay));
    $("wx-shading-label").textContent = `${(LAYERS[wx.layer] ?? LAYERS.wind).label} shading`;
    drawLegend();
    changed();
  }
  shade.onchange = () => {
    if (shade.value === "off") wx.shading = "off";
    else wx.shading = shade.value;
    saveWx();
    applyLayer();
    renderLayers();
    draw();
  };
  function setLayer(v) {
    if (v === "none") { wx.shading = "off"; }
    else { wx.layer = v; if (wx.shading === "off") wx.shading = "smooth"; }
    shade.value = wx.shading;
    saveWx();
    applyLayer();
    setTimes();
  }
  /** The isobar interval for this zoom: the chosen one, ×2 below zoom 4 and ×4 below 2.5 when Auto is on. */
  function isobarStep() {
    const base = Number(wx.isoStep) || 4;
    if (!wx.isoAuto) return base;
    const z = map.getZoom();
    return z < 2.5 ? base * 4 : z < 4 ? base * 2 : base;
  }
  $("wx-isostep").value = String(wx.isoStep);
  $("wx-isostep").onchange = (e) => { wx.isoStep = Number(e.target.value); saveWx(); draw(); };
  for (const [id, key] of [["wx-isoauto", "isoAuto"]]) {
    $(id).checked = !!wx[key];
    $(id).onchange = (e) => { wx[key] = e.target.checked; saveWx(); draw(); changed(); };
  }
  // el -> { always }: a legend marked `always` is the key beside the chart time, shown whenever weather is drawn
  /** A layer's fixed scale between two values, as the chart draws it: in bands when the shading is banded. */
  function fixedStops(layer, lo, hi) {
    const step = Number(wx.shading);
    if (!layer.knots || !(step > 0)) {
      return [[lo, paletteAt(lo, layer.scale)], ...layer.scale.filter(([v]) => v > lo && v < hi), [hi, paletteAt(hi, layer.scale)]];
    }
    const out = [];
    for (let a = Math.floor(lo / step) * step; a < hi; a += step) {
      const c = paletteAt(a + step / 2);
      out.push([Math.max(lo, a), c], [Math.min(hi, a + step), c]);
    }
    return out;
  }
  /** Round numbers to label a scale from lo to hi: about `n` of them. */
  function ticks(lo, hi, n = 5) {
    const raw = (hi - lo) / n;
    if (!(raw > 0)) return [];
    const p = 10 ** Math.floor(Math.log10(raw)), m = raw / p;
    const step = (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * p;
    const out = [];
    for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9; v += step) out.push(+v.toFixed(6));
    return out;
  }
  const legends = new Map();
  /** Dark text on pale colours, white on dark ones (the key's numbers sit on the colours). */
  const inkFor = (c) => {
    const h = /^#([0-9a-f]{6})$/i.exec(c), q = /rgba?\((\d+)[ ,]+(\d+)[ ,]+(\d+)/.exec(c);
    const [r, g, b] = h ? [0, 2, 4].map((i) => parseInt(h[1].slice(i, i + 2), 16)) : q ? [q[1], q[2], q[3]].map(Number) : [255, 255, 255];
    return 0.299 * r + 0.587 * g + 0.114 * b < 120 ? "#fff" : "#1d2a33";
  };
  function swatches(el, layer) {
    if (layer.knots) {
      const step = wx.shading === "smooth" ? 5 : Math.max(Number(wx.shading), wx.shading === "1" ? 2 : Number(wx.shading));
      for (let k = 0; k <= 40; k += step) {
        const sw = document.createElement("span");
        sw.style.background = paletteAt(wx.shading === "smooth" ? k : k + Number(wx.shading) / 2);
        sw.style.color = inkFor(sw.style.background);
        sw.title = `${k} kn`;
        sw.textContent = k % 10 === 0 ? k : "";
        el.append(sw);
      }
    } else {
      for (const [v, c] of layer.scale) {
        const sw = document.createElement("span");
        sw.style.background = c;
        sw.style.color = inkFor(c);
        sw.title = `${layer.log ? fmtChl(10 ** v) : v} ${layer.unit}`;
        sw.textContent = layer.log ? fmtChl(10 ** v) : layer.auto ? (Math.abs(v) >= 10 && layer.unit === "°C" ? v.toFixed(v % 1 ? 1 : 0) : String(Number(Number(v).toFixed(Math.abs(v) >= 10 ? 1 : 2)))) : v;
        el.append(sw);
      }
    }
  }
  function drawLegend() {
    const layer = LAYERS[wx.layer] ?? LAYERS.wind;
    drawSeaFilter(layer);
    const boxShown = filterBoxes.some(({ el }) => el.isConnected && !el.hidden);
    const f = filterInUse(layer, wx.seaFilter[wx.layer]) ? wx.seaFilter[wx.layer] : null;
    for (const [el, { always }] of legends) {
      if (!el.isConnected) { if (!always) legends.delete(el); continue; }
      el.innerHTML = "";
      el.hidden = wx.shading === "off" || (always ? !display || !weatherVisibleInView(weatherView, wx.hidden, wx.navShow) : boxShown);
      if (el.hidden) continue;
      el.title = `${layer.label}, ${layer.unit}`;
      if (!always) { swatches(el, layer); continue; }
      const cap = document.createElement("div");
      cap.className = "wx-key-title";
      cap.textContent = `${layer.label} (${layer.unit})${f?.set ? " · filter set" : ""}`;
      const bar = document.createElement("div");
      bar.className = "wx-legend";
      el.append(cap, bar);
      if (!f) { swatches(bar, layer); continue; }
      // coloured through the filter: its colours from low to high, with the numbers at the ends
      const w = f.hi - f.lo || 1;
      const { stops } = filterColourStyle(layer, f, wx.shading);
      bar.classList.add("grad");
      bar.style.background = `linear-gradient(to right, ${stops.map(([v, c]) => `${c} ${((v - f.lo) / w) * 100}%`).join(", ")})`;
      const num = (v) => (layer.log ? fmtChl(10 ** v) : String(+v.toFixed(2)));
      // numbers under it: round values across the scale, and the two ends (an end left alone hides
      // nothing: beyond it the last colour carries on, shown as ≤ or ≥)
      const ends_ = filterBounds(f);
      const row = document.createElement("div");
      row.className = "wx-key-ticks";
      const put = (v, text, cls = "") => { const x = document.createElement("span"); x.textContent = text; x.className = cls; x.style.left = `${((v - f.lo) / w) * 100}%`; row.append(x); };
      put(f.lo, `${ends_.lo || layer.auto ? "" : "≤ "}${num(f.lo)}`, "lo");
      for (const v of layer.log ? [] : ticks(f.lo, f.hi)) if ((v - f.lo) / w > 0.14 && (f.hi - v) / w > 0.14) put(v, num(v));
      put(f.hi, `${ends_.hi || layer.auto ? "" : "≥ "}${num(f.hi)}`, "hi");
      el.append(row);
      if (f.tg != null) { const t = document.createElement("div"); t.className = "wx-key-ends"; t.textContent = `target ${num(f.tg)}`; el.append(t); }
    }
  }
  changeListeners.push(drawLegend);
  loadedListeners.push(drawLegend);
  applyLayer();
  allowPastAnalysis = pastDefault();
  $("env-use-past").checked = allowPastAnalysis;
  window.addEventListener("racenav-use", () => {
    if (wx.pastAnalysis != null || allowPastAnalysis === pastDefault()) return;
    allowPastAnalysis = pastDefault();
    $("env-use-past").checked = allowPastAnalysis;
    draw(); changed();
  });

  window.addEventListener("racenav-view", (event) => {
    const next = event.detail?.id;
    if (!["nav", "plan", "env"].includes(next) || next === weatherView) return;
    saveWx();
    // the chart time belongs to the window, not to the view
    viewStates.set(weatherView, { settings: structuredClone(wx), display });
    const previous = viewStates.get(next);
    const settings = previous?.settings ?? loadWxViews()[next] ?? wx;
    Object.assign(wx, structuredClone(settings));
    weatherView = next;
    allowPastAnalysis = pastDefault();
    $("env-use-past").checked = allowPastAnalysis;
    if (previous && loaded.has(previous.display)) display = previous.display;
    dens.value = wx.density; size.value = wx.size; shade.value = wx.shading; opac.value = wx.opacity;
    landSel.value = wx.land;
    $("wx-isostep").value = String(wx.isoStep);
    $("wx-isoauto").checked = !!wx.isoAuto;
    showVals();
    map.setLayoutProperty("wx-barbs", "icon-size", wx.size);
    map.setLayoutProperty("wx-currdir", "icon-size", ["*", wx.size, ["get", "s"]]);
    map.setPaintProperty("wx-cells", "fill-opacity", wx.opacity);
    moveShading();
    applyLayer();
    renderLoaded();
    setTimes();
    saveWx();
    changed();
  });

  refreshFiles();
  invoke("grib_loaded").then((list) => {
    for (const s of list || []) loaded.set(s.file, s);
    if (loaded.size) { display = loaded.keys().next().value; renderLoaded(); setTimes(); }
  }).catch(() => {});

  return {
    setCursor: (lonlat) => { cursor = lonlat; updatePoint(); },
    /** The GRIB shown on the chart and the slider time. */
    current: () => ({ path: display, t, name: display && loaded.get(display)?.name }),
    /** Every loaded GRIB: { path, name, start, end } (wind times). */
    all: () => [...loaded].map(([path, s]) => {
      const w = s.params.find((p) => p.param === "WindU") ?? s.params.find((p) => p.times?.length);
      return { path, name: s.name, start: w?.times[0], end: w?.times.at(-1) };
    }),
    /** Move the slider to a time (clamped to the loaded GRIBs' combined span). */
    setTime: (sec) => {
      const s = $("side-wx-time");
      if (!times.length) return;
      t = Math.round(Math.min(Number(s.max), Math.max(Number(s.min), sec)));
      s.value = t;
      draw();
    },
    onTime: (fn) => { timeListeners.push(fn); },
    /** The shown GRIB's values at the cursor (and slider time) each time they change: fn(row | null, [lon, lat]). */
    onPoint: (fn) => { pointListeners.push(fn); },
    /** Whether any loaded file has sea data (sea temperature, salinity, currents, chlorophyll). */
    hasSea: () => [...loaded.values()].some((s) => {
      const ps = paramsOf(s);
      return Object.values(LAYERS).some((l) => l.ocean && ps.has(l.params[0]));
    }),
    /** Called when a GRIB is loaded or unloaded. */
    onLoaded: (fn) => { loadedListeners.push(fn); },
    /** Unload a loaded GRIB (the file stays on disk). */
    unload: (path) => unload(path),
    /** Make a loaded GRIB the shown one. */
    show: (path) => { if (loaded.has(path)) { showGrib(path); changed(); } },
    /** Loaded GRIBs ticked for the agreement displays. */
    comparePaths,
    /** Every loaded GRIB, whether it is in the agreement displays and whether it has wind or pressure to compare: [{ path, name, on, can }]. */
    compareFiles: () => [...loaded].map(([path, s]) => ({ path, name: s.name, on: !wx.cmpOff.includes(path),
      can: ["WindU", "WindSpeed", "Mslp"].some((p) => paramsOf(s).has(p)) })),
    /** Put a loaded GRIB in the agreement displays, or leave it out. */
    setCompare(path, on) {
      wx.cmpOff = on ? wx.cmpOff.filter((p) => p !== path) : [...new Set([...wx.cmpOff, path])];
      saveWx(); renderLoaded(); renderLayers(); draw(); changed();
    },
    /** Shade a forecast's error at a verify time ({ t, path, name, lead, kind }), or null to stop. */
    setVerify(v) {
      verifyTarget = v;
      if (v) {
        const key = { vector: "verr_vec", speed: "verr_spd", direction: "verr_dir" }[v.kind ?? "vector"];
        if (!LAYERS[wx.layer]?.verify || v.kind) { wx.layer = key; if (wx.shading === "off") wx.shading = "smooth"; shade.value = wx.shading; saveWx(); applyLayer(); }
        renderLayers();
        const s = $("side-wx-time");
        if (times.length && v.t >= Number(s.min) && v.t <= Number(s.max)) { t = Math.round(v.t); s.value = t; }
        draw();
      } else {
        if (LAYERS[wx.layer]?.verify) { wx.layer = "wind"; saveWx(); applyLayer(); }
        setTimes();
      }
    },
    verifyTarget: () => verifyTarget,
    /** Put a GRIB chooser and time slider into `el`, working the same state as the Weather drawer. */
    mountControls,
    /** Show or hide every weather layer on the chart (shading, barbs, arrows, isobars, contours); the data stays loaded. */
    setVisible(on) { if (weatherView === "nav") wx.navShow = !!on; else wx.hidden = !on; saveWx(); applyLayer(); draw(); changed(); },
    visible: () => weatherVisibleInView(weatherView, wx.hidden, wx.navShow),
    mountFilter,
    /** Names of every parameter in the loaded GRIBs. */
    loadedParams: () => new Set([...loaded.values()].flatMap((s) => [...paramsOf(s)])),
    /** A colour-bar legend for the shown layer, kept current. */
    mountLegend(el, { always = false } = {}) { legends.set(el, { always }); drawLegend(); },
    onChange: (fn) => { changeListeners.push(fn); },
    /** The shown layer's definition, key and the slider's range. */
    layerInfo: () => {
      const layer = LAYERS[wx.layer] ?? LAYERS.wind;
      const path = shadePath(layer);
      const [srcMin, srcMax] = spanOf(path) ?? [undefined, undefined];
      const timeStatus = weatherView === "env" && !layer.compare && !layer.verify
        ? dataTimeStatus(loaded.get(path), layer.params, t, allowPastAnalysis) : null;
      return { key: wx.layer, layer, off: wx.shading === "off", min: times[0], max: times.at(-1), srcMin, srcMax, t, filter: wx.seaFilter[wx.layer] ?? null, path, timeStatus };
    },
    /** Loaded GRIBs: { path, name, model, ref }. */
    /** The first and last forecast time of a loaded GRIB (the shown one if none is given): [t0, t1] or null. */
    span: (path = display) => (path && loaded.has(path) ? spanOf(path) : null),
    sources: () => [...loaded].map(([path, s]) => ({ path, name: s.name, model: s.model, ref: s.ref_time, shown: path === display })),
    /** Shadable layers with the files that have them: [{ key, label, unit, paths, path }]. */
    layerOptions() {
      return Object.entries(LAYERS).filter(([key, l]) => !(l.compare || l.verify) || available().includes(key)).map(([key, l]) => {
        if (l.compare || l.verify) return { key, label: l.label, unit: l.unit, paths: [], path: null, help: l.help, compare: !!l.compare, verify: !!l.verify };
        const paths = [...loaded].filter(([, s]) => l.params.some((p) => paramsOf(s).has(p))).map(([p]) => p);
        // the shown GRIB if it has the layer, else the newest loaded one that does
        const path = l.ocean ? oceanPath(l) : paths.includes(display) ? display : paths.at(-1);
        return { key, label: l.label, unit: l.unit, paths, path };
      }).filter((o) => o.paths.length || LAYERS[o.key].compare || LAYERS[o.key].verify);
    },
    /** Overlays with the files that can draw them: [{ key, label, on, paths, path }]. */
    overlayOptions() {
      const defs = [
        ["arrows", "Wind barbs", ["WindU", "WindSpeed"], "arrows"],
        ["waveArrows", "Wave direction", ["WaveDir"], "waveArrows"],
        ["currArrows", "Current direction", ["CurrentU"], "current"],
        ["isobars", "Isobars", ["Mslp"], "isobars"],
        ["breaks", "Temperature breaks", ["SeaTemp", "SkinTemp"], "breaks"],
        ["isotherms", "Contours and target line", null, null],
      ];
      return defs.map(([key, label, params, sk]) => {
        const paths = params ? [...loaded].filter(([, s]) => params.some((p) => paramsOf(s).has(p))).map(([p]) => p) : [...loaded.keys()];
        const path = key === "currArrows" ? oceanPath(LAYERS.current) :
          key === "breaks" ? (loaded.has(wx.src.breaks) ? wx.src.breaks : oceanPath(LAYERS.sst) ?? oceanPath(LAYERS.skin)) :
          sk ? srcOf(sk) : null;
        const timeStatus = weatherView === "env" && params && path
          ? dataTimeStatus(loaded.get(path), params, t, allowPastAnalysis) : null;
        return { key, label, on: !!wx[key], paths: params ? paths : [], path, timeStatus, hasData: params ? paths.length > 0 : !!LAYERS[wx.layer]?.contour };
      }).filter((o) => o.hasData);
    },
    /** Shade `key` ("none" for no shading) from `path`. */
    pickLayer(key, path) {
      if (key !== "none" && path) {
        if (LAYERS[key]?.ocean) wx.src[key] = path; else showGrib(path);
      }
      setLayer(key);
      draw();
    },
    /** Switch an overlay on or off, and optionally the file it is drawn from. */
    setOverlay(key, on, path) {
      if (on != null) wx[key] = on;
      const sk = { arrows: "arrows", waveArrows: "waveArrows", currArrows: "current", isobars: "isobars", breaks: "breaks" }[key];
      if (path && sk) wx.src[sk] = path;
      saveWx(); draw(); changed();
    },
    /** What is shown, as a preset: { layer, src, overlays, filter }. */
    config: () => ({
      layer: wx.shading === "off" ? "none" : wx.layer,
      src: { ...wx.src },
      overlays: Object.fromEntries(["arrows", "waveArrows", "currArrows", "isobars", "isotherms", "breaks"].map((k) => [k, !!wx[k]])),
      filter: wx.seaFilter[wx.layer]?.set || LAYERS[wx.layer]?.auto ? { ...wx.seaFilter[wx.layer] } : null,
    }),
    applyConfig(c) {
      Object.assign(wx.src, c.src ?? {});
      for (const [k, v] of Object.entries(c.overlays ?? {})) {
        wx[k] = v;
      }
      if (c.layer && (c.layer === "none" || available().includes(c.layer))) setLayer(c.layer);
      if (c.filter && wx.layer === c.layer) { wx.seaFilter[c.layer] = { ...c.filter }; applyLayer(); }
      saveWx(); draw(); changed();
    },
    /** Resets the shown layer's filter to the file's whole range. */
    resetFilter() { delete wx.seaFilter[wx.layer]; saveWx(); applyLayer(); draw(); },
    showGrib,
    /** Load a GRIB file by path (e.g. one just downloaded), then show it. */
    async load(path) { await load(path); showGrib(path); },
    refreshFiles,
  };
}

/** Barb images are drawn on demand: id "barb:<knots rounded to 5>:<n|s>". */
export function barbImage(id) {
  return id.startsWith("barb:") ? barbIcon(id) : null;
}

function barbId(tws, lat) {
  const k = Math.round(tws / 5) * 5;
  return `barb:${k}:${lat < 0 ? "s" : "n"}`;
}

/** Wind barb pointing up (wind from the north): staff from the station (bottom) to the tip.
 *  Feathers go on the clockwise side in the northern hemisphere, anticlockwise in the southern. */
function barbIcon(id) {
  const [, ks, hem] = id.split(":");
  let k = Number(ks);
  const W = 64, H = 80, cx = W / 2, top = 6, bottom = H - 2;
  const side = hem === "s" ? -1 : 1;
  const c = document.createElement("canvas");
  c.width = W; c.height = H;
  const g = c.getContext("2d");
  const total = k;
  const path = () => {
    let k = total;
    g.beginPath();
    if (k < 3) {
      g.arc(cx, bottom - 8, 6, 0, Math.PI * 2);
      return;
    }
    g.moveTo(cx, bottom);
    g.lineTo(cx, top);
    let y = top;
    const L = 22;
    while (k >= 50) { g.moveTo(cx, y); g.lineTo(cx + side * L, y + 5); g.lineTo(cx, y + 10); y += 13; k -= 50; }
    while (k >= 10) { g.moveTo(cx, y); g.lineTo(cx + side * L, y - 7); y += 8; k -= 10; }
    if (k >= 5) { if (y === top) y += 8; g.moveTo(cx, y); g.lineTo(cx + side * L / 2, y - 4); }
  };
  g.lineCap = "round"; g.lineJoin = "round";
  path(); g.strokeStyle = "rgba(255,255,255,0.9)"; g.lineWidth = 7; g.stroke();
  path(); g.strokeStyle = "#1d2a33"; g.lineWidth = 3; g.stroke();
  // fill pennants
  k = Number(ks);
  let y = top;
  g.fillStyle = "#1d2a33";
  while (k >= 50) { g.beginPath(); g.moveTo(cx, y); g.lineTo(cx + side * 22, y + 5); g.lineTo(cx, y + 10); g.fill(); y += 13; k -= 50; }
  return g.getImageData(0, 0, W, H);
}

/** "12 Oct 16:55" in local time. */
function fmtLocal(sec) {
  return new Date(sec * 1000).toLocaleString("en-NZ", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hour12: false });
}

function fc(features) { return { type: "FeatureCollection", features }; }
function n(v, d) { return v == null ? "—" : v.toFixed(d); }

/** Local time plus UTC, e.g. "Tue 13 Oct 09:00 (20:00Z)". */
export function fmt(sec, short = false) {
  const d = new Date(sec * 1000);
  const local = d.toLocaleString("en-NZ", { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hour12: false });
  if (short) return local;
  const z = d.toISOString().slice(11, 16);
  return `${local} (${z}Z)`;
}

/** Wave arrow: a slim arrow pointing up, drawn once. */
function waveArrow(colour = "#0b3d6b") {
  const w = 18, h = 34, c = document.createElement("canvas");
  c.width = w; c.height = h;
  const g = c.getContext("2d");
  g.strokeStyle = colour; g.fillStyle = colour; g.lineWidth = 2.5; g.lineCap = "round";
  g.beginPath(); g.moveTo(w / 2, h - 3); g.lineTo(w / 2, 10); g.stroke();
  g.beginPath(); g.moveTo(w / 2, 2); g.lineTo(w / 2 + 6, 13); g.lineTo(w / 2 - 6, 13); g.closePath(); g.fill();
  return g.getImageData(0, 0, w, h);
}

/**
 * Isobars every `step` hPa from a pressure grid: marching squares, segments
 * joined into lines, with a label point on each longer line.
 */
export function isobars(gp, step, only = null) {
  const { ni, nj, values: v } = gp;
  const at = (i, j) => v[j * ni + i];
  const pos = (i, j) => [gp.lon0 + i * gp.dlon, gp.lat0 + j * gp.dlat];
  let lo = Infinity, hi = -Infinity;
  for (const x of v) if (x != null) { if (x < lo) lo = x; if (x > hi) hi = x; }
  const out = [];
  const levels = only ?? [];
  if (!only) for (let level = Math.ceil(lo / step) * step; level <= hi; level += step) levels.push(level);
  for (const level of levels) {
    const segs = [];
    for (let j = 0; j + 1 < nj; j++) {
      for (let i = 0; i + 1 < ni; i++) {
        const c = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
        if (c.some((x) => x == null)) continue;
        const p = [pos(i, j), pos(i + 1, j), pos(i + 1, j + 1), pos(i, j + 1)];
        const cross = [];
        for (let e = 0; e < 4; e++) {
          const a = c[e], b = c[(e + 1) % 4];
          if ((a < level) !== (b < level)) {
            const f = (level - a) / (b - a), pa = p[e], pb = p[(e + 1) % 4];
            cross.push([pa[0] + (pb[0] - pa[0]) * f, pa[1] + (pb[1] - pa[1]) * f]);
          }
        }
        if (cross.length === 2) segs.push(cross);
        else if (cross.length === 4) { segs.push([cross[0], cross[1]]); segs.push([cross[2], cross[3]]); }
      }
    }
    for (const line of joinSegments(segs)) {
      out.push({ type: "Feature", properties: { hpa: level }, geometry: { type: "LineString", coordinates: line } });
      if (line.length > 12) out.push({ type: "Feature", properties: { hpa: level }, geometry: { type: "Point", coordinates: line[Math.floor(line.length / 2)] } });
    }
  }
  return out;
}

function joinSegments(segs) {
  const key = (p) => `${p[0].toFixed(5)},${p[1].toFixed(5)}`;
  const ends = new Map();
  segs.forEach((s, i) => { for (const p of s) { const k = key(p); (ends.get(k) ?? ends.set(k, []).get(k)).push(i); } });
  const used = new Array(segs.length).fill(false);
  const lines = [];
  for (let i = 0; i < segs.length; i++) {
    if (used[i]) continue;
    used[i] = true;
    const line = [...segs[i]];
    for (const dir of [1, -1]) {
      for (;;) {
        const tip = dir > 0 ? line.at(-1) : line[0];
        const next = (ends.get(key(tip)) ?? []).find((k) => !used[k]);
        if (next === undefined) break;
        used[next] = true;
        const s = segs[next];
        const other = key(s[0]) === key(tip) ? s[1] : s[0];
        if (dir > 0) line.push(other); else line.unshift(other);
      }
    }
    lines.push(line);
  }
  return lines;
}
