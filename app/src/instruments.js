// The readings TweedPlot can show (ITEMS), drawn in the data panel beside the chart
// (sidepanel.js) and on the Instruments view's screen (insttiles.js). Each reading can show
// a state ("stale", "warn", "alarm") that styles it.

import { rhumb } from "./navmath.js";
import { ask } from "./ask.js";
import { own } from "./role.js";
import { setupTiles } from "./insttiles.js";
import { CHANNELS as HIST, sample as histSample, renderHistoryTile, openHistoryCard, recorder as histRec } from "./history.js";
import { depthFresh, depthHistoryV } from "./depth-watch.js";

const KEY = "racenav.instruments";
const STALE_S = 10;

// ---------- what can be shown ---------------------------------------------------------
const f1 = (x) => x.toFixed(1);
const deg3 = (x) => String(Math.round(((x % 360) + 360) % 360) % 360).padStart(3, "0");
const sideAngle = (x) => `${Math.round(Math.abs(x))}`;

/**
 * Each item: label, unit, group, get(ctx) → value or null, fmt(value) → text,
 * optional state(value, ctx) → "" | "warn" | "alarm", side(value) → "port" | "stbd",
 * wide (needs a wide tile), live (from Signal K: greys out when the data stops).
 */
export const ITEMS = {
  SOG: { label: "SOG", unit: "kn", group: "Navigation", live: true, get: (c) => c.v.sog, fmt: f1 },
  COG: { label: "COG", unit: "°T", group: "Navigation", live: true, get: (c) => c.v.cog, fmt: deg3 },
  HDG: { label: "HDG", unit: "°T", group: "Navigation", live: true, get: (c) => c.v.heading, fmt: deg3 },
  HDM: { label: "HDG", unit: "°M", group: "Navigation", live: true, get: (c) => c.v.heading_mag, fmt: deg3 },
  BSP: { label: "BSP", unit: "kn", group: "Navigation", live: true, get: (c) => c.v.stw, fmt: f1 },
  DPT: {
    label: "Depth", unit: "m", group: "Navigation", live: true, get: (c) => c.v.depth ?? c.v.depth_surface, fmt: f1,
    // default depth warning: below safety depth + 2 m warns, below the safety depth alarms
    state: (d, c) => (d < c.safetyDepth ? "alarm" : d < c.safetyDepth + 2 ? "warn" : ""),
  },
  POS: { label: "Position", unit: "", group: "Navigation", live: true, wide: true, get: (c) => (c.v.lat != null && c.v.lon != null ? [c.v.lat, c.v.lon] : null), fmt: (p) => `${ddm(p[0], "N", "S")}<br>${ddm(p[1], "E", "W")}` },
  LOG: { label: "Log", unit: "NM", group: "Navigation", live: true, get: (c) => c.v.log, fmt: (x) => x.toFixed(0) },
  TRIP: { label: "Trip", unit: "NM", group: "Navigation", live: true, get: (c) => c.v.trip, fmt: f1 },
  // local and UTC time together (the data panel's own-ship block)
  TIME: { label: "Time", unit: "", group: "Navigation", wide: true, get: (c) => c.now,
    fmt: (t) => { const d = new Date(t); return `${d.toLocaleDateString("en-NZ", { day: "numeric", month: "short" })} ${d.toLocaleTimeString("en-NZ", { hour12: false })}<span class="u">local</span> ${d.toISOString().slice(11, 16)}<span class="u">UTC</span>`; } },
  UTC: { label: "UTC", unit: "", group: "Navigation", get: (c) => c.now, fmt: (t) => new Date(t).toISOString().slice(11, 19) },
  LOCAL: { label: "Local", unit: "", group: "Navigation", get: (c) => c.now, fmt: (t) => new Date(t).toLocaleTimeString("en-NZ", { hour12: false }) },

  AWS: { label: "AWS", unit: "kn", group: "Wind", live: true, tint: "a", get: (c) => c.v.aws, fmt: f1 },
  AWA: { label: "AWA", unit: "°", group: "Wind", live: true, tint: "a", get: (c) => c.v.awa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  TWS: { label: "TWS", unit: "kn", group: "Wind", live: true, tint: "t", get: (c) => c.v.tws, fmt: f1 },
  TWA: { label: "TWA", unit: "°", group: "Wind", live: true, tint: "t", get: (c) => c.v.twa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  // TWD as sent (MWD, Signal K), else worked out from the true wind angle and the heading (MWV sends only the angle)
  TWD: { label: "TWD", unit: "°T", group: "Wind", live: true, tint: "t",
    get: (c) => c.v.twd ?? ((c.v.heading ?? c.v.cog) != null && c.v.twa != null ? ((c.v.heading ?? c.v.cog) + c.v.twa + 360) % 360 : null), fmt: deg3 },
  VMG: { label: "VMG", unit: "kn", group: "Wind", live: true, get: (c) => (c.v.twa != null && (c.v.stw ?? c.v.sog) != null ? (c.v.stw ?? c.v.sog) * Math.cos((c.v.twa * Math.PI) / 180) : null), fmt: (x) => f1(Math.abs(x)) },

  // the weather station (Signal K environment.outside, or NMEA MDA / XDR)
  PRES: { short: "Baro", label: "Pressure", unit: "hPa", group: "Weather", live: true, get: (c) => c.v.pressure, fmt: f1 },
  AIRT: { short: "Air", label: "Air temp", unit: "°C", group: "Weather", live: true, get: (c) => c.v.air_temp, fmt: f1 },
  HUM: { short: "RH", label: "Humidity", unit: "%", group: "Weather", live: true, get: (c) => c.v.humidity, fmt: (x) => x.toFixed(0) },
  WTEMP: { short: "Water", label: "Water temp", unit: "°C", group: "Weather", live: true, get: (c) => c.v.water_temp, fmt: f1 },

  NEXT: { label: "To mark", unit: "", group: "Route", wide: true, get: (c) => c.nav?.next?.name ?? null, fmt: (s) => esc(s) },
  BTW: { label: "BTW", unit: "°T", group: "Route", get: (c) => c.wp?.btw, fmt: deg3 },
  DTW: { label: "DTW", unit: "NM", group: "Route", get: (c) => c.wp?.dtw, fmt: (x) => (x < 10 ? x.toFixed(2) : x.toFixed(1)) },
  VMC: { label: "VMC", unit: "kn", group: "Route", live: true, get: (c) => c.wp?.vmc, fmt: f1 },
  TTG: { label: "TTG", unit: "", group: "Route", get: (c) => c.wp?.ttg, fmt: hm },
  ETA: { label: "ETA", unit: "", group: "Route", wide: true, get: (c) => (c.wp?.ttg != null ? c.now + c.wp.ttg * 1000 : null), fmt: (t) => new Date(t).toLocaleString("en-NZ", { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false }) },
  // + = boat to the right of the track (steer left to get back)
  XTE: { label: "XTE", unit: "", group: "Route", get: (c) => c.wp?.xte, fmt: (x) => `${Math.abs(x) < 10 ? Math.abs(x).toFixed(2) : Math.abs(x).toFixed(1)}<span class="u">NM ${x >= 0 ? "R" : "L"}</span>` },
  TTWA: { label: "Tgt TWA", unit: "°", group: "Route", get: (c) => c.target?.twa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  TBSP: { label: "Tgt BSP", unit: "kn", group: "Route", get: (c) => c.target?.bsp, fmt: f1 },
  THDG: { label: "Tgt HDG", unit: "°T", group: "Route", get: (c) => c.target?.heading, fmt: deg3 },
  PERF: { label: "BSP / tgt", unit: "%", group: "Route", live: true, get: (c) => (c.target?.bsp > 0.2 && c.v.stw != null ? (100 * c.v.stw) / c.target.bsp : null), fmt: (x) => x.toFixed(0) },

  // the mouse cursor on the chart: position, and bearing and range from the boat (rhumb line)
  CPOS: { label: "Cursor", unit: "", group: "Cursor", wide: true, get: (c) => c.cursor ?? null, fmt: (p) => `${ddm(p[1], "N", "S")}<br>${ddm(p[0], "E", "W")}` },
  CLAT: { label: "Cursor lat", unit: "", group: "Cursor", wide: true, get: (c) => c.cursor?.[1] ?? null, fmt: (x) => ddm(x, "N", "S") },
  CLON: { label: "Cursor lon", unit: "", group: "Cursor", wide: true, get: (c) => c.cursor?.[0] ?? null, fmt: (x) => ddm(x, "E", "W") },
  CBRG: { label: "Brg to cursor", unit: "°T", group: "Cursor", get: (c) => cursorLeg(c)?.brg ?? null, fmt: deg3 },
  // own soundings (Chart data) under the cursor, from zoom 9 in
  CSND: { label: "Own sounding", unit: "m", group: "Cursor", get: (c) => (c.sounding ? -c.sounding.z : null), fmt: f1 },
  CSNDS: { label: "Sounding spread", unit: "±m", group: "Cursor", get: (c) => (c.sounding && c.sounding.n > 1 ? c.sounding.spread : null), fmt: f1 },
  CSNDN: { label: "Soundings", unit: "", group: "Cursor", get: (c) => c.sounding?.n ?? null, fmt: (n) => String(n) },
  CRNG: { label: "Rng to cursor", unit: "NM", group: "Cursor", get: (c) => cursorLeg(c)?.nm ?? null, fmt: (x) => (x < 10 ? x.toFixed(2) : x.toFixed(1)) },

  // the GRIB shown on the chart, at the cursor and the slider time (Weather panel)
  GTWS: { label: "GRIB TWS", unit: "kn", group: "GRIB at cursor", get: (c) => c.grib?.tws, fmt: f1 },
  GTWD: { label: "GRIB TWD", unit: "°T", group: "GRIB at cursor", get: (c) => c.grib?.twd, fmt: deg3 },
  GGUST: { label: "GRIB gust", unit: "kn", group: "GRIB at cursor", get: (c) => c.grib?.gust, fmt: (x) => x.toFixed(0) },
  GMSLP: { label: "GRIB MSLP", unit: "hPa", group: "GRIB at cursor", get: (c) => c.grib?.mslp, fmt: (x) => x.toFixed(0) },
  GHS: { label: "GRIB Hs", unit: "m", group: "GRIB at cursor", get: (c) => c.grib?.wave, fmt: f1 },
  GTP: { label: "Wave period", unit: "s", group: "GRIB at cursor", wide: true, get: (c) => c.grib?.period, fmt: (x) => x.toFixed(0) },
  GWVD: { label: "GRIB wave dir", unit: "°T", group: "GRIB at cursor", get: (c) => c.grib?.wavedir, fmt: deg3 },
  GRAIN: { label: "GRIB rain", unit: "mm/h", group: "GRIB at cursor", get: (c) => c.grib?.rain, fmt: f1 },
  GTEMP: { label: "GRIB air temp", unit: "°C", group: "GRIB at cursor", get: (c) => c.grib?.temp, fmt: f1 },
  // the sea, from any loaded ocean file (MUR, HYCOM) at the cursor
  GSST: { label: "Sea temp", unit: "°C", group: "Sea at cursor", get: (c) => c.grib?.sst, fmt: (x) => x.toFixed(1) },
  GSKIN: { label: "Sea temp sat", unit: "°C", group: "Sea at cursor", get: (c) => c.grib?.skin, fmt: (x) => x.toFixed(1) },
  GCHL: { label: "Chlorophyll", unit: "mg/m³", group: "Sea at cursor", get: (c) => c.grib?.chl, fmt: (x) => (x >= 10 ? x.toFixed(0) : x >= 1 ? x.toFixed(1) : x.toFixed(2)) },
  GSSS: { label: "Salinity", unit: "PSU", group: "Sea at cursor", get: (c) => c.grib?.sss, fmt: (x) => x.toFixed(2) },
  GSUBT: { label: "Temp at depth", unit: "°C", group: "Sea at cursor", get: (c) => c.grib?.subt, fmt: (x) => x.toFixed(1) },
  GSUBS: { label: "Salinity at depth", unit: "PSU", group: "Sea at cursor", get: (c) => c.grib?.subs, fmt: (x) => x.toFixed(2) },
  GCUR: { label: "Current", unit: "kn", group: "Sea at cursor", get: (c) => c.grib?.cur?.kn, fmt: (x) => x.toFixed(1) },
  GSET: { label: "Current set", unit: "°T", group: "Sea at cursor", get: (c) => c.grib?.cur?.set, fmt: deg3 },
  GTIME: { label: "GRIB time", unit: "", group: "GRIB at cursor", wide: true, get: (c) => c.grib?.t, fmt: (t) => new Date(t * 1000).toLocaleString("en-NZ", { weekday: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false }) },
  GSRC: { label: "GRIB", unit: "", group: "GRIB at cursor", wide: true, get: (c) => c.grib?.name, fmt: (s) => esc(s) },

  // man overboard: from the boat to the MOB position (rhumb line), while a MOB is set
  MOBBRG: { label: "Brg to MOB", unit: "°T", group: "Man overboard", get: (c) => mobLeg(c)?.brg ?? null, fmt: deg3, state: () => "alarm" },
  MOBRNG: { label: "Rng to MOB", unit: "", group: "Man overboard", get: (c) => mobLeg(c)?.nm ?? null, state: () => "alarm",
    // metres close in, where it matters most
    fmt: (x) => (x < 0.2 ? `${Math.round(x * 1852)}<span class="u">m</span>` : `${x < 10 ? x.toFixed(2) : x.toFixed(1)}<span class="u">NM</span>`) },
  // where and when MOB was pressed: for the log and a distress call (time in local and UTC)
  MOBPOS: { label: "MOB at", unit: "", group: "Man overboard", wide: true, get: (c) => c.mob ?? null,
    fmt: (m) => `${ddm(m.lat, "N", "S")}<br>${ddm(m.lon, "E", "W")}<br><span class="u">${new Date(m.t).toLocaleTimeString("en-NZ", { hour12: false })} · ${new Date(m.t).toISOString().slice(11, 19)} UTC</span>` },
  MOBAGO: { label: "Since MOB", unit: "", group: "Man overboard", get: (c) => (c.mob ? (c.now - c.mob.t) / 1000 : null), state: () => "alarm",
    fmt: (s) => `${Math.floor(s / 60)}<span class="u">min</span>${String(Math.floor(s % 60)).padStart(2, "0")}<span class="u">s</span>` },
};

const histChange = (k) => histRec.change(k, 3);

// ---------- graphical instruments ----------------------------------------------------------
const polar = (r, deg) => [50 + r * Math.sin((deg * Math.PI) / 180), 50 - r * Math.cos((deg * Math.PI) / 180)];
const pt = (p) => `${p[0].toFixed(2)},${p[1].toFixed(2)}`;
function ticks(step, rIn, rOut, cls) {
  let d = "";
  for (let a = 0; a < 360; a += step) d += `M${pt(polar(rIn, a))}L${pt(polar(rOut, a))}`;
  return `<path d="${d}" class="${cls}"/>`;
}
/** Wind dial: boat up, true (blue) and apparent (orange) needles pointing to where each wind
 *  comes from. True wind speed and angle in the top corners, apparent in the bottom corners,
 *  each labelled in its needle's colour (the same colours as the wind arrows on the chart). */
function windDial(w) {
  const sector = (a0, a1, cls) => {
    const p0 = polar(44, a0), p1 = polar(44, a1);
    return `<path d="M${pt(polar(38, a0))}L${pt(p0)}A44,44 0 0 ${a1 > a0 ? 1 : 0} ${pt(p1)}L${pt(polar(38, a1))}A38,38 0 0 ${a1 > a0 ? 0 : 1} ${pt(polar(38, a0))}z" class="${cls}"/>`;
  };
  const ptr = (a, cls, len) => (a == null ? "" : `<path d="M${pt(polar(len, a))}L${pt(polar(10, a - 9))}L${pt(polar(10, a + 9))}z" class="${cls}"/>`);
  const spd = (x) => (x != null ? x.toFixed(1) : "—");
  const ang = (x) => (x == null ? "—" : `${Math.round(Math.abs(x))}°<tspan class="ps ${x < 0 ? "p" : "s"}">${x < 0 ? "P" : "S"}</tspan>`);
  return `<svg viewBox="-10 -10 120 120" class="dial wdial">
    ${sector(20, 60, "close-stbd")}${sector(-60, -20, "close-port")}
    <circle cx="50" cy="50" r="44" class="ring"/>${ticks(10, 41, 44, "tk")}${ticks(30, 37, 44, "tk2")}
    <path d="M50,32 L56,47 L56,62 L50,68 L44,62 L44,47z" class="hull"/>
    ${ptr(w.twa, "ptr-t", 40)}${ptr(w.awa, "ptr-a", 43)}
    <text x="-8" y="-2.5" class="wc t">TWS <tspan class="u">kn</tspan></text><text x="-8" y="10" class="wv t">${spd(w.tws)}</text>
    <text x="108" y="-2.5" class="wc t end">TWA</text><text x="108" y="10" class="wv t end">${ang(w.twa)}</text>
    <text x="-8" y="100" class="wv a">${spd(w.aws)}</text><text x="-8" y="108.5" class="wc a">AWS <tspan class="u">kn</tspan></text>
    <text x="108" y="100" class="wv a end">${ang(w.awa)}</text><text x="108" y="108.5" class="wc a end">AWA</text>
  </svg>`;
}
/** Compass: heading up; COG (orange tick) and bearing to the mark (green). */
function compassRose(c) {
  const h = c.hdg ?? c.cog;
  let card = ticks(10, 40, 44, "tk") + ticks(30, 36, 44, "tk2");
  for (const [a, l] of [[0, "N"], [90, "E"], [180, "S"], [270, "W"]]) {
    const p = polar(30, a);
    card += `<text x="${p[0].toFixed(1)}" y="${(p[1] + 3.5).toFixed(1)}" class="card${a === 0 ? " n" : ""}" transform="rotate(${a} ${p[0].toFixed(1)} ${p[1].toFixed(1)})">${l}</text>`;
  }
  const mark = (a, cls) => (a == null ? "" : `<path d="M${pt(polar(46, a))}L${pt(polar(38, a - 5))}L${pt(polar(38, a + 5))}z" class="${cls}"/>`);
  return `<svg viewBox="0 0 100 100" class="dial">
    <g transform="rotate(${(-h).toFixed(1)} 50 50)"><circle cx="50" cy="50" r="44" class="ring"/>${card}${mark(c.cog, "m-cog")}${mark(c.btw, "m-btw")}</g>
    <path d="M50,3 L54,10 L46,10z" class="lubber"/>
    <text x="50" y="56" class="num">${String(Math.round(((h % 360) + 360) % 360) % 360).padStart(3, "0")}°</text>
    <text x="50" y="68" class="cap mid">${c.hdg != null ? "HDG" : "COG"}${c.cog != null && c.hdg != null ? ` · COG ${String(Math.round(c.cog)).padStart(3, "0")}` : ""}</text>
  </svg>`;
}
ITEMS.WDIAL = { label: "Wind", unit: "", group: "Graphic", live: true, graphic: true,
  get: (c) => (c.v.awa == null && c.v.twa == null ? null : { awa: c.v.awa, twa: c.v.twa, aws: c.v.aws, tws: c.v.tws }), fmt: windDial };
ITEMS.COMPASS = { label: "Compass", unit: "", group: "Graphic", live: true, graphic: true,
  get: (c) => (c.v.heading == null && c.v.cog == null ? null : { hdg: c.v.heading, cog: c.v.cog, btw: c.wp?.btw }), fmt: compassRose };
ITEMS.HIGHWAY = {
  label: "Highway", unit: "", group: "Graphic", live: true, graphic: true, wide: true, highway: true,
  get: highwayData, fmt: highwayMarkup, render: renderHighway,
};

// ---------- depth history (a fish-finder style sounder) ------------------------------------
const SND_KEY = "racenav.sounder";
const SND_WINDOWS = [5, 15, 30, 60, 120]; // minutes across the screen
const SND_RANGES = [0, 10, 20, 50, 100, 200, 500]; // metres to the bottom of the screen; 0 = auto
const SND_KEEP_MS = 2 * 3600e3;
const sndHist = []; // { t (ms), d (m below the surface or transducer, as the Depth reading), w (water °C) }
const sndPrefs = { win: 15, range: 0 };
try { Object.assign(sndPrefs, JSON.parse(localStorage.getItem(SND_KEY) || "{}")); } catch { /* ignore */ }

// The last two hours are kept over a restart (each window its own copy), saved every 30 s and
// when the window closes, as [seconds after t0, depth ×10, water °C ×10] to keep it small.
const SND_HIST_KEY = own("racenav.sounder.hist");
(() => {
  try {
    const o = JSON.parse(localStorage.getItem(SND_HIST_KEY) || "null");
    if (!o || !Array.isArray(o.s)) return;
    const cut = Date.now() - SND_KEEP_MS;
    for (const [dt, d, w] of o.s) {
      const t = o.t0 + dt * 1000;
      if (t >= cut && t <= Date.now() && Number.isFinite(d)) sndHist.push({ t, d: d / 10, w: w == null ? null : w / 10 });
    }
  } catch { /* damaged or no storage: start empty */ }
})();
let sndSaved = sndHist.at(-1)?.t ?? 0;
function saveSounder() {
  const last = sndHist.at(-1);
  if (!last || last.t === sndSaved) return;
  sndSaved = last.t;
  const t0 = sndHist[0].t;
  const s = sndHist.map((x) => [Math.round((x.t - t0) / 1000), Math.round(x.d * 10), x.w == null ? null : Math.round(x.w * 10)]);
  try { localStorage.setItem(SND_HIST_KEY, JSON.stringify({ t0, s })); } catch { /* full or unavailable: kept in memory only */ }
}
setInterval(saveSounder, 30_000);
window.addEventListener("beforeunload", saveSounder);
window.addEventListener("pagehide", saveSounder);

/** One sample a second while the depth is live; a gap in the data leaves a gap in the trace. */
function sampleDepth(c) {
  const d = c.v?.depth ?? c.v?.depth_surface;
  const now = Date.now();
  if (!depthFresh(c.v, c.depthAge ?? (() => null))) return;
  const last = sndHist[sndHist.length - 1];
  if (last && now - last.t < 1000) return;
  sndHist.push({ t: now, d, w: c.v?.water_temp ?? null });
  while (sndHist.length && now - sndHist[0].t > SND_KEEP_MS) sndHist.shift();
}

const niceStep = (span) => {
  const raw = span / 5, p = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 5, 10].map((m) => m * p).find((s) => s >= raw) ?? raw;
};

function drawSounder(cv, c) {
  const dpr = window.devicePixelRatio || 1;
  const W = Math.max(1, Math.round(cv.clientWidth * dpr)), H = Math.max(1, Math.round(cv.clientHeight * dpr));
  if (cv.width !== W || cv.height !== H) { cv.width = W; cv.height = H; }
  const g = cv.getContext("2d");
  const font = (px, bold) => `${bold ? "bold " : ""}${Math.round(px * dpr)}px system-ui, sans-serif`;
  const now = Date.now(), winMs = sndPrefs.win * 60e3, t0 = now - winMs;
  let pts = sndHist.filter((s) => s.t >= t0 - 5000);
  // two hours is 7,200 samples: more than the screen has columns, so draw about one in every pixel and a half (the last one always)
  const stride = Math.floor(pts.length / Math.max(1, W / 1.5));
  if (stride > 1) pts = pts.filter((_, i) => i % stride === 0 || i === pts.length - 1);
  const gap = 6000 * Math.max(1, stride); // a gap in the data this long leaves a gap in the trace
  const maxD = pts.reduce((m, s) => Math.max(m, s.d), 0);
  const want = Math.max(5, maxD * 1.4), ast = niceStep(want);
  const range = sndPrefs.range || Math.ceil(want / ast) * ast;
  const X = (t) => ((t - t0) / winMs) * W, Y = (d) => (d / range) * H;

  const bg = g.createLinearGradient(0, 0, 0, H);
  bg.addColorStop(0, "#0b4f86"); bg.addColorStop(0.5, "#06305a"); bg.addColorStop(1, "#031a33");
  g.fillStyle = bg; g.fillRect(0, 0, W, H);

  // depth lines and time ticks
  const step = niceStep(range);
  g.lineWidth = Math.max(1, dpr * 0.6); g.font = font(10); g.textBaseline = "bottom";
  for (let d = step; d < range; d += step) {
    g.strokeStyle = "rgba(255,255,255,.18)"; g.beginPath(); g.moveTo(0, Y(d)); g.lineTo(W, Y(d)); g.stroke();
    g.fillStyle = "rgba(255,255,255,.75)"; g.textAlign = "right"; g.fillText(`${d} m`, W - 3 * dpr, Y(d) - 1 * dpr);
  }
  const tick = winMs / 60e3 <= 15 ? 60e3 : winMs / 60e3 <= 60 ? 5 * 60e3 : 15 * 60e3;
  g.textBaseline = "top";
  // the "DEPTH UNAVAILABLE" label sits in the top left corner, where the first time ticks are: leave them out
  const last = sndHist[sndHist.length - 1];
  const live = last && now - last.t < STALE_S * 1000 && depthFresh(c.v, c.depthAge ?? (() => null));
  let clearOf = 0;
  if (!live && last) { g.font = font(11, true); clearOf = 6 * dpr + g.measureText("DEPTH UNAVAILABLE").width + 6 * dpr; g.font = font(10); }
  for (let t = Math.ceil(t0 / tick) * tick; t < now; t += tick) {
    const x = X(t);
    g.strokeStyle = "rgba(255,255,255,.12)"; g.beginPath(); g.moveTo(x, 0); g.lineTo(x, H); g.stroke();
    if (x - 14 * dpr < clearOf || x > W - 90 * dpr) continue; // not under the label, nor under the window and range buttons
    g.fillStyle = "rgba(255,255,255,.6)"; g.textAlign = "center";
    g.fillText(`−${Math.round((now - t) / 60e3)}m`, Math.min(W - 14 * dpr, Math.max(14 * dpr, x)), 2 * dpr);
  }

  // the bottom: brown body under a hard orange and yellow return, like a sounder's bottom line
  const segs = [];
  for (const s of pts) {
    const cur = segs[segs.length - 1];
    if (cur && s.t - cur[cur.length - 1].t < gap) cur.push(s); else segs.push([s]);
  }
  const band = (off, colour) => {
    g.fillStyle = colour;
    for (const sg of segs) {
      if (sg.length < 2) continue;
      g.beginPath();
      sg.forEach((s, i) => (i ? g.lineTo(X(s.t), Y(s.d)) : g.moveTo(X(s.t), Y(s.d))));
      for (let i = sg.length - 1; i >= 0; i--) g.lineTo(X(sg[i].t), Math.min(H, Y(sg[i].d) + off));
      g.closePath(); g.fill();
    }
  };
  band(H, "#5a2d0c"); band(20 * dpr, "#a8481a"); band(12 * dpr, "#e0701c"); band(6 * dpr, "#f5b52e"); band(2.5 * dpr, "#fff2a0");

  // the safety depth
  const safe = c.safetyDepth;
  if (safe > 0 && safe < range) {
    g.setLineDash([6 * dpr, 4 * dpr]); g.strokeStyle = "rgba(255,90,90,.85)"; g.lineWidth = dpr;
    g.beginPath(); g.moveTo(0, Y(safe)); g.lineTo(W, Y(safe)); g.stroke(); g.setLineDash([]);
  }

  // the readout
  g.textAlign = "left"; g.textBaseline = "top";
  g.shadowColor = "rgba(0,0,0,.8)"; g.shadowBlur = 4 * dpr;
  g.fillStyle = "#fff"; g.font = font(Math.max(16, Math.min(40, cv.clientHeight / 4)), true);
  const big = live ? last.d.toFixed(1) : "—";
  g.fillText(`${big}`, 6 * dpr, 16 * dpr);
  if (!live && last) {
    g.font = font(11, true); g.fillStyle = "#ffce75";
    g.fillText("DEPTH UNAVAILABLE", 6 * dpr, 3 * dpr);
    g.font = font(Math.max(16, Math.min(40, cv.clientHeight / 4)), true);
  }
  const bw = g.measureText(big).width;
  g.font = font(11); g.fillStyle = "rgba(255,255,255,.8)"; g.fillText("m", 6 * dpr + bw + 3 * dpr, 20 * dpr + 14 * dpr);
  if (live && last.w != null) { g.font = font(13, true); g.fillStyle = "#7fe0ff"; g.fillText(`${last.w.toFixed(1)} °C`, 6 * dpr, (22 + Math.max(16, Math.min(40, cv.clientHeight / 4))) * dpr); }
  g.shadowBlur = 0;
  if (!pts.length) {
    g.textAlign = "center"; g.textBaseline = "middle"; g.font = font(13); g.fillStyle = "rgba(255,255,255,.7)";
    g.fillText("Waiting for depth data", W / 2, H / 2);
  }
}

function renderSounder(host, _raw, ctx) {
  if (host.dataset.ready !== "true") {
    host.classList.add("snd-host");
    host.innerHTML = `<canvas class="snd-cv"></canvas><div class="snd-ctl"><button class="snd-win" title="How much history fits across the screen"></button><button class="snd-rng" title="How deep the screen goes (Auto follows the bottom)"></button></div>`;
    const win = host.querySelector(".snd-win"), rng = host.querySelector(".snd-rng");
    const labels = () => {
      win.textContent = sndPrefs.win >= 60 ? `${sndPrefs.win / 60} h` : `${sndPrefs.win} min`;
      rng.textContent = sndPrefs.range ? `0–${sndPrefs.range} m` : "Auto";
    };
    const cycle = (list, key) => (e) => {
      e.stopPropagation();
      sndPrefs[key] = list[(list.indexOf(sndPrefs[key]) + 1) % list.length] ?? list[0];
      try { localStorage.setItem(SND_KEY, JSON.stringify(sndPrefs)); } catch { /* ignore */ }
      labels();
      host._ctx && drawSounder(host.querySelector("canvas"), host._ctx);
    };
    win.onclick = cycle(SND_WINDOWS, "win");
    rng.onclick = cycle(SND_RANGES, "range");
    labels();
    host.dataset.ready = "true";
  }
  host._ctx = ctx;
  const cv = host.querySelector("canvas"), now = Date.now(), size = `${cv.clientWidth}x${cv.clientHeight}`;
  // the sounder is redrawn twice a second (it is asked ten times a second), and at once when its size changes
  if (host._drawnAt && now - host._drawnAt < 500 && host._drawnSize === size) return;
  host._drawnAt = now; host._drawnSize = size;
  drawSounder(cv, ctx);
}

ITEMS.SOUNDER = {
  label: "Sounder history", unit: "m", group: "Graphic", live: true, graphic: true, wide: true, sounder: true,
  get: () => true, fmt: () => "", render: renderSounder,
};

// ---------- history plots (history.js): one per recorded reading ------------------------------
for (const [k, ch] of Object.entries(HIST)) {
  ITEMS[`H_${k}`] = { label: `${ch.label} history`, unit: "", group: "History", live: true, graphic: true, wide: true, history: k, tint: ch.tint,
    get: () => true, fmt: () => "", render: renderHistoryTile(k) };
}
// the pressure tendency as a single reading: change over three hours, with the words for it
ITEMS.PTEND = { short: "Baro 3 h", label: "Pressure 3 h", unit: "hPa", group: "Weather", live: true,
  get: () => histChange("PRES"), fmt: (d) => `${d > 0 ? "+" : d < 0 ? "−" : "±"}${Math.abs(d).toFixed(1)}`,
  state: (d) => (d <= -6 ? "alarm" : d <= -3.6 ? "warn" : "") };
for (const [k, it] of Object.entries(ITEMS)) it.key = k;

/** The history channel behind a reading (TWS → TWS, H_PRES → PRES, Pressure 3 h → PRES), if any. */
export function historyKey(itemKey) {
  const it = ITEMS[itemKey];
  if (!it) return null;
  if (it.history) return it.history;
  if (itemKey === "PTEND") return "PRES";
  if (itemKey === "HDM") return null;
  return HIST[itemKey] ? itemKey : null;
}

function highwayData(c) {
  let range = null, bearingToTarget = null, track = null, cts = null, xte = null;
  if (c.mob && c.v.lat != null && c.v.lon != null) {
    const leg = mobLeg(c);
    if (leg) {
      range = leg.nm;
      bearingToTarget = leg.brg;
      track = leg.brg;
      cts = leg.brg;
      xte = 0;
    }
  } else if (c.nav && c.wp) {
    range = c.wp.dtw;
    bearingToTarget = c.wp.btw;
    track = c.nav.from ? bearing(c.nav.from, [c.nav.next.lon, c.nav.next.lat]) : c.wp.btw;
    const correction = c.wp.xte == null ? 0 : Math.min(30, Math.atan2(Math.abs(c.wp.xte), Math.max(c.wp.dtw, 0.1)) * 180 / Math.PI);
    cts = (track - Math.sign(c.wp.xte ?? 0) * correction + 360) % 360;
    xte = c.wp.xte;
  }
  const sog = c.v.sog;
  const vmg = sog != null && c.v.cog != null && bearingToTarget != null
    ? sog * Math.cos(rad(c.v.cog - bearingToTarget))
    : c.mob ? null : c.wp?.vmc ?? null;
  const ttg = range != null && vmg != null && vmg > 0.1 ? (range / vmg) * 3600 : null;
  const groundTrackOffset = c.v.heading != null && c.v.cog != null
    ? ((c.v.cog - c.v.heading + 540) % 360) - 180
    : 0;
  const targetHdg = cts == null ? null : (cts - groundTrackOffset + 360) % 360;
  return {
    // no heading sensor: steer by COG (the arrow then shows the turn on the ground track)
    mob: !!c.mob, hdg: c.v.heading ?? c.v.cog, hdgFromCog: c.v.heading == null && c.v.cog != null,
    cog: c.v.cog, sog, range, bearing: bearingToTarget,
    track, cts, targetHdg, xte, vmg, ttg, eta: ttg == null ? null : c.now + ttg * 1000,
    groundTrackOffset,
  };
}

function highwayMarkup() {
  return `<div class="hw-dashboard">
    <div class="hw-metrics hw-top">
      <div class="hw-metric"><span data-hw="hdg-label" data-short="HDG">Heading · HDG</span><strong data-hw="hdg">—</strong></div>
      <div class="hw-metric hw-target"><span data-short="CTS">Course to steer · CTS</span><strong data-hw="cts">—</strong></div>
      <div class="hw-metric"><span data-short="SOG kn">Speed over ground</span><strong data-hw="sog">—</strong><small>SOG · kn</small></div>
      <div class="hw-metric"><span data-hw="dtw-label" data-short="DTW NM">Distance to waypoint</span><strong data-hw="dtw">—</strong><small>DTW · NM</small></div>
    </div>
    <div class="hw-ribbon">
      <svg viewBox="0 0 600 50" preserveAspectRatio="none" role="img" aria-label="Heading compass ribbon">
        <g class="hw-ribbon-scale"></g>
        <path d="M300 0L294 9H306Z" class="hw-ribbon-marker"/>
        <text x="300" y="20" text-anchor="middle" class="hw-ribbon-current" data-hw="ribbon-heading">—</text>
      </svg>
    </div>
    <div class="hw-canvas">
      <svg viewBox="0 0 600 300" preserveAspectRatio="none" role="img" aria-label="Perspective navigation highway">
        <g class="hw-runway">
          <path d="M235 26L32 290H568L365 26Z" class="hw-road"/>
          <path d="M235 26L32 290M365 26L568 290" class="hw-rail"/>
          <path d="M300 26V290" class="hw-centerline"/>
          <path d="M195 70H405M157 112H443M95 178H505M43 250H557" class="hw-gridline"/>
          <path d="M260 70L120 290M280 112L205 290M320 112L395 290M340 70L480 290" class="hw-lane"/>
          <path d="M300 40V88" class="hw-aim"/>
        </g>
        <g class="hw-vessel">
          <path d="M300 188L286 216L290 233H310L314 216Z" class="hw-boat"/>
          <path d="M273 197A30 30 0 0 1 327 197" class="hw-boat-wake"/>
        </g>
      </svg>
    </div>
    <div class="hw-steer-row">
      <svg viewBox="0 0 600 60" preserveAspectRatio="none" role="img" aria-label="Required steering direction">
        <path class="hw-steer-shaft"/>
        <path class="hw-steer-arrow"/>
      </svg>
    </div>
    <div class="hw-metrics hw-bottom">
      <div class="hw-metric hw-xte"><span data-short="XTE">Cross-track error · XTE</span><strong data-hw="xte">—</strong></div>
      <div class="hw-metric"><span data-short="TTG">Time to go · TTG</span><strong data-hw="ttg">—</strong><small data-hw="vmg">VMG —</small></div>
      <div class="hw-metric"><span data-short="ETA">Estimated arrival · ETA</span><strong data-hw="eta">—</strong></div>
    </div>
  </div>`;
}

const highwaySmoothing = new WeakMap();
const HIGHWAY_ANGLE_FIELDS = new Set(["hdg", "cog", "bearing", "track", "cts", "targetHdg"]);
const highwayRibbonAngles = new WeakMap();
function smoothHighway(host, value, now) {
  if (!value) return value;
  const previous = highwaySmoothing.get(host);
  if (!previous) {
    highwaySmoothing.set(host, { value, now });
    return value;
  }
  const alpha = 1 - Math.exp(-Math.max(0, now - previous.now) / 450);
  const smoothed = { ...value };
  for (const key of ["hdg", "cog", "sog", "range", "bearing", "track", "cts", "targetHdg", "groundTrackOffset", "xte", "vmg", "ttg", "eta"]) {
    const current = value[key], old = previous.value[key];
    if (current == null || !Number.isFinite(current)) { smoothed[key] = null; continue; }
    if (old == null || !Number.isFinite(old)) { smoothed[key] = current; continue; }
    if (HIGHWAY_ANGLE_FIELDS.has(key)) {
      const difference = ((current - old + 540) % 360) - 180;
      smoothed[key] = (old + difference * alpha + 360) % 360;
    } else {
      smoothed[key] = old + (current - old) * alpha;
    }
  }
  highwaySmoothing.set(host, { value: smoothed, now });
  return smoothed;
}

function highwayTime(seconds) {
  if (seconds == null || !Number.isFinite(seconds)) return "—";
  if (seconds > 99 * 3600) return ">99h";
  const h = Math.floor(seconds / 3600), m = Math.round((seconds % 3600) / 60);
  return h ? `${h}h ${String(m).padStart(2, "0")}m` : `${m} min`;
}

function highwayClock(timestamp) {
  return timestamp == null || !Number.isFinite(timestamp)
    ? "—"
    : new Date(timestamp).toLocaleString("en-NZ", { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false });
}

function renderHighway(host, raw, ctx) {
  host.classList.add("hw-host");
  if (host.dataset.ready !== "true") {
    host.innerHTML = highwayMarkup();
    host.dataset.ready = "true";
  }
  const h = smoothHighway(host, raw, ctx.now ?? Date.now());
  const text = (key, value) => {
    const node = host.querySelector(`[data-hw="${key}"]`);
    if (node && node.textContent !== value) node.textContent = value;
  };
  const angle = (degrees) => degrees == null ? "—" : `${deg3(degrees)}°`;
  text("hdg", angle(h?.hdg));
  text("hdg-label", h?.hdgFromCog ? "Course over ground · COG" : "Heading · HDG");
  host.querySelector('[data-hw="hdg-label"]')?.setAttribute("data-short", h?.hdgFromCog ? "COG" : "HDG");
  host.querySelector('[data-hw="dtw-label"]')?.setAttribute("data-short", h?.mob ? "To MOB NM" : "DTW NM");
  text("cts", angle(h?.cts));
  text("sog", h?.sog == null ? "—" : h.sog.toFixed(1));
  text("dtw", h?.range == null ? "—" : h.range.toFixed(h.range < 10 ? 2 : 1));
  text("dtw-label", h?.mob ? "Distance to MOB" : "Distance to waypoint");
  text("xte", h?.xte == null ? "—" : `${Math.abs(h.xte).toFixed(2)} NM`);
  // no time to go when the range isn't closing: say so rather than show a dash
  const closing = h?.ttg != null || h?.range == null || h?.vmg == null || !Number.isFinite(h.vmg);
  text("ttg", closing ? highwayTime(h?.ttg) : h.vmg < -0.1 ? "Opening" : "Not closing");
  text("vmg", h?.vmg == null ? "VMG —" : `VMG ${h.vmg.toFixed(1)} kn`);
  text("eta", highwayClock(h?.eta));
  text("ribbon-heading", h?.hdg == null ? "—" : `${deg3(h.hdg)}°`);
  const ribbon = host.querySelector(".hw-ribbon-scale");
  if (ribbon && h?.hdg != null) {
    let ribbonHeading = highwayRibbonAngles.get(host);
    if (ribbonHeading == null) {
      ribbonHeading = h.hdg < 70 ? h.hdg + 360 : h.hdg;
    } else {
      ribbonHeading += ((h.hdg - (ribbonHeading % 360) + 540) % 360) - 180;
    }
    highwayRibbonAngles.set(host, ribbonHeading);
    const base = Math.round(ribbonHeading / 10) * 10;
    const fraction = ribbonHeading - base;
    let marks = "";
    for (let i = -9; i <= 9; i++) {
      const x = 300 + i * 30;
      const value = base + i * 10;
      const bearing = ((value % 360) + 360) % 360;
      marks += `<path d="M${x} 0V22" class="hw-ribbon-tick"/><text x="${x}" y="40" text-anchor="middle" class="hw-ribbon-label${value % 20 ? " hw-odd" : ""}">${String(bearing).padStart(3, "0")}</text>`;
    }
    if (ribbon.dataset.base !== String(base)) {
      ribbon.innerHTML = marks;
      ribbon.dataset.base = String(base);
    }
    ribbon.setAttribute("transform", `translate(${(-fraction * 3).toFixed(1)} 0)`);
  }
  const delta = h?.hdg != null && h?.targetHdg != null
    ? ((h.targetHdg - h.hdg + 540) % 360) - 180
    : null;
  const steer = host.querySelector(".hw-steer-row");
  const steerShaft = steer?.querySelector(".hw-steer-shaft");
  const steerArrow = steer?.querySelector(".hw-steer-arrow");
  const length = delta == null || Math.abs(delta) < 1 ? 0 : Math.min(250, Math.max(28, Math.abs(delta) * 5));
  const direction = delta == null ? 0 : Math.sign(delta);
  const tip = 300 + direction * length;
  const headBase = tip - direction * 28;
  if (steerShaft) steerShaft.setAttribute("d", length ? `M300 30L${headBase.toFixed(1)} 30` : "");
  if (steerArrow) steerArrow.setAttribute("d", length
    ? `M${tip.toFixed(1)} 30L${headBase.toFixed(1)} 14L${headBase.toFixed(1)} 46Z`
    : "");
  steer?.classList.toggle("steer-port", delta != null && delta < -1);
  steer?.classList.toggle("steer-stbd", delta != null && delta > 1);
  const runway = host.querySelector(".hw-runway");
  if (runway) {
    const offset = Math.max(-1.5, Math.min(1.5, h?.xte ?? 0)) * -48;
    runway.setAttribute("transform", `translate(${offset.toFixed(1)} 0)`);
  }
  host.parentElement?.classList.toggle("mob-highway", !!h?.mob);
}

/** Rhumb line from the boat to the cursor, or null. */
function mobLeg(c) {
  if (!c.mob || c.v.lat == null || c.v.lon == null) return null;
  return rhumb([c.v.lon, c.v.lat], [c.mob.lon, c.mob.lat]);
}

function cursorLeg(c) {
  if (!c.cursor || c.v.lat == null || c.v.lon == null) return null;
  return rhumb([c.v.lon, c.v.lat], c.cursor);
}

/**
 * The Instruments view's screen of readings (insttiles.js), the readings' shared state (ctx),
 * the history recording, and right-click for a reading's history. The floating instrument panels
 * of earlier versions are retired: their readings live in the data panel (sidepanel.js), which
 * takes the user's own panels over once (floatingPanels()).
 */
export function setupInstruments() {
  const menu = document.getElementById("tile-menu");
  let layout = loadLayout();
  const tiles = setupTiles({ host: document.getElementById("itiles"), menu, items: ITEMS, getLayout: () => layout, save, paint: (t, it, fit) => paintTile(t, it, fit), historyKey });
  let ctx = { v: {}, now: Date.now(), age: Infinity, safetyDepth: 3 };

  function save() { try { localStorage.setItem(KEY, JSON.stringify(layout)); } catch { /* ignore */ } }
  window.addEventListener("racenav-view", () => build());
  // another window (a monitor, or the main window) changed the shared layout
  window.addEventListener("storage", (e) => {
    if (e.key !== KEY || !e.newValue) return;
    layout = loadLayout();
    build();
  });

  function build() {
    tiles.build();
    const lockBtn = document.getElementById("inst-lock");
    if (lockBtn) {
      lockBtn.classList.toggle("on", !!layout.locked);
      lockBtn.textContent = layout.locked ? "Layout locked" : "Lock layout";
    }
  }
  function render() {
    ctx.now = Date.now();
    tiles.render();
  }
  /** Show one reading in its tile; `fit` scales the text (or the highway) to fill the tile. */
  function paintTile(t, it, fit) {
    let val = null;
    try { val = it.get(ctx); } catch { val = null; }
    const has = val !== null && val !== undefined && !(typeof val === "number" && !Number.isFinite(val));
    // a real true/false: classList.toggle(name, undefined) flips the class on every render
    const stale = readingStale(it, ctx);
    const state = has && !stale && it.state ? it.state(val, ctx) : "";
    const v = t.querySelector(".v");
    if (it.render) it.render(v, val, ctx);
    else {
      const html = has ? it.fmt(val) + (it.unit ? `<span class="u">${it.unit}</span>` : "") : "—";
      if (v.innerHTML !== html) v.innerHTML = html;
    }
    const sd = t.querySelector(".sd");
    const side = has && it.side ? it.side(val) : "";
    sd.className = `sd ${side}`;
    sd.textContent = side === "port" ? "PORT" : side === "stbd" ? "STBD" : "";
    t.classList.toggle("stale", stale);
    t.classList.toggle("warn", state === "warn");
    t.classList.toggle("alarm", state === "alarm");
    if (it.highway && fit) fitHighway(v);
    else if (!it.graphic && fit) fitText(t, v);
  }
  setInterval(() => {
    ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000;
    sampleDepth(ctx);
    // the history records live data only: not a replay, not planning, not stale
    const historyV = depthHistoryV(ctx.v, ctx.depthAge ?? (() => null));
    histSample(historyV, ctx.age <= STALE_S && !ctx.replay && (ctx.dataAt ?? 0) > 0);
    render();
  }, 250);

  // right-click a reading: its history in a floating card
  const histFrom = (el) => {
    const t = el?.closest?.("#itiles .itile, #side .sp-row, #side-strip .sp-row");
    if (!t) return null;
    let k = t.dataset.k;
    if (!k && t.classList.contains("itile")) k = layout.tiles?.find((x) => x.id === t.dataset.id)?.k;
    return historyKey(k);
  };
  document.addEventListener("contextmenu", (e) => {
    const h = histFrom(e.target);
    if (!h) return;
    e.preventDefault();
    e.stopPropagation();
    openHistoryCard(h, [e.clientX, e.clientY]);
  }, true);
  document.addEventListener("pointerdown", (e) => { if (!menu.hidden && !menu.contains(e.target) && !e.target.closest(".itile")) menu.hidden = true; });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !menu.hidden) { menu.hidden = true; e.preventDefault(); } });

  // ---------- drawer controls ----------------------------------------------------------
  document.getElementById("it-add").onclick = () => { if (!tiles.add()) ask({ title: "The Instruments screen is full", note: "Make a tile smaller or remove one first.", cancel: "", fields: [] }); };
  document.getElementById("it-reset").onclick = () => tiles.reset();
  const lockBtn = document.getElementById("inst-lock");
  if (lockBtn) lockBtn.onclick = () => { layout.locked = !layout.locked; save(); build(); };

  build();

  return {
    /** Replace the Instruments screen's tiles with a preset: [{k, c, r, w, h}] on the 12 x 8 grid. */
    setScreen(list) {
      layout.tiles = list.filter((t) => ITEMS[t.k]).map((t, i) => ({ id: `t${i + 1}`, ...t }));
      save();
      build();
    },
    /** The floating panels of earlier versions (for the data panel to take over once). */
    floatingPanels: () => layout.panels ?? [],
    /** The readings' shared state: vessel, route, targets, cursor, GRIB, MOB… and its age. */
    ctx: () => ctx,
    /** "042°T · 3.2 NM" from the boat to a chart point (the cursor read-out), or "" without a boat position. */
    legTo(pt) {
      const l = pt ? cursorLeg({ cursor: pt, v: ctx.v }) : null;
      if (!l) return "";
      return `${deg3(l.brg)}°T · ${l.nm < 10 ? l.nm.toFixed(2) : l.nm.toFixed(1)} NM`;
    },
    /** New data: vessel state, route/waypoint and routing targets. */
    update(next) {
      const newMob = next.mob && !ctx.mob;
      ctx = { ...ctx, ...next, now: Date.now() };
      ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000;
      if (newMob) window.dispatchEvent(new Event("racenav-mob-panel"));
      render();
    },
  };
}

/** Grow or shrink a reading's text to fill its tile (measured at 100 px, then scaled). */
function fitText(t, v) {
  const key = `${v.innerHTML}|${t.clientWidth}|${t.clientHeight}`;
  if (t._fit === key) return;
  const lbl = t.querySelector(".lbl");
  const availW = t.clientWidth - 18;
  const availH = t.clientHeight - (lbl ? lbl.offsetHeight : 0) - 10;
  if (availW <= 0 || availH <= 0) return;
  v.style.fontSize = "100px";
  v.style.width = "max-content";
  const r = v.getBoundingClientRect();
  v.style.width = "";
  const k = Math.min(availW / Math.max(1, r.width), availH / Math.max(1, r.height));
  v.style.fontSize = `${Math.max(8, Math.min(400, Math.floor(k * 100)))}px`;
  t._fit = key;
}
/** Scale the highway's drawing, laid out at its design size, to fill its cell. */
function fitHighway(v) {
  const d = v.querySelector(".hw-dashboard");
  if (!d || !v.clientWidth || !v.clientHeight) return;
  const k = Math.min(v.clientWidth / 680, v.clientHeight / 470);
  d.style.transform = `translate(${((v.clientWidth - 680 * k) / 2).toFixed(1)}px, ${((v.clientHeight - 470 * k) / 2).toFixed(1)}px) scale(${k.toFixed(4)})`;
}

/** A graphic reading (dial, highway, sounder, history plot) drawn into `host`, sized by its box. */
export function paintGraphic(host, it, ctx) {
  let val = null;
  try { val = it.get(ctx); } catch { val = null; }
  if (it.render) it.render(host, val, ctx);
  else {
    const html = val != null ? it.fmt(val) : "—";
    if (host._html !== html) { host.innerHTML = html; host._html = html; }
  }
  // in the data panel the highway lays itself out to the panel's width (CSS .hw-fluid), rather than
  // being its full-screen drawing shrunk to fit, which made it unreadably small
  if (it.highway) {
    const fluid = !!host.closest("#side");
    host.classList.toggle("hw-fluid", fluid);
    const d = host.querySelector(".hw-dashboard");
    if (fluid) { if (d) d.style.transform = ""; } else fitHighway(host);
    // narrow: the compass ribbon keeps its proportions (cropped at the sides) so its figures stay readable
    const rib = host.querySelector(".hw-ribbon svg");
    const par = fluid ? "xMidYMid slice" : "none";
    if (rib && rib.getAttribute("preserveAspectRatio") !== par) rib.setAttribute("preserveAspectRatio", par);
  }
}

/** Paint one reading's value: { html, stale, state, side } (the data panel's rows). */
export function readingState(it, ctx) {
  let val = null;
  try { val = it.get(ctx); } catch { val = null; }
  const has = val !== null && val !== undefined && !(typeof val === "number" && !Number.isFinite(val));
  const stale = readingStale(it, ctx);
  const state = has && !stale && it.state ? it.state(val, ctx) : "";
  const side = has && it.side ? it.side(val) : "";
  let html = "—";
  if (has) { try { html = it.fmt(val); } catch { html = "—"; } }
  return { html, unit: has ? it.unit : "", stale, state, side };
}

function readingStale(it, ctx) {
  if (it === ITEMS.POS) return ctx.positionAt == null || Date.now() - ctx.positionAt > STALE_S * 1000;
  if (it === ITEMS.DPT || it === ITEMS.SOUNDER || it === ITEMS.H_DPT) return !depthFresh(ctx.v, ctx.depthAge ?? (() => null));
  return Boolean(it.live) && ctx.age > STALE_S;
}

function loadLayout() {
  let l = null;
  try {
    l = JSON.parse(localStorage.getItem(KEY) || "null");
    if (!l || l.v !== 1) l = null;
  } catch { l = null; }
  l ??= { v: 1, locked: false };
  // the floating panels of earlier versions are kept only for the data panel to take over
  if (!Array.isArray(l.panels)) l.panels = [];
  return l;
}

function hm(s) {
  if (s > 99 * 3600) return "&gt;99h";
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return h ? `${h}<span class="u">h</span>${String(m).padStart(2, "0")}` : `${m}<span class="u">min</span>`;
}

function ddm(deg, pos, neg) {
  if (pos === "E" && Number.isFinite(deg)) deg = ((((deg + 180) % 360) + 360) % 360) - 180; // map past ±180
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}

function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]); }

// ---------- route / waypoint maths ----------------------------------------------------------
const R_NM = 3440.065;
const rad = (d) => (d * Math.PI) / 180;
export function distNm(a, b) {
  const [l1, p1] = [rad(a[0]), rad(a[1])], [l2, p2] = [rad(b[0]), rad(b[1])];
  const h = Math.sin((p2 - p1) / 2) ** 2 + Math.cos(p1) * Math.cos(p2) * Math.sin((l2 - l1) / 2) ** 2;
  return 2 * Math.asin(Math.min(1, Math.sqrt(h))) * R_NM;
}
export function bearing(a, b) {
  const [l1, p1] = [rad(a[0]), rad(a[1])], [l2, p2] = [rad(b[0]), rad(b[1])];
  const y = Math.sin(l2 - l1) * Math.cos(p2);
  const x = Math.cos(p1) * Math.sin(p2) - Math.sin(p1) * Math.cos(p2) * Math.cos(l2 - l1);
  return ((Math.atan2(y, x) * 180) / Math.PI + 360) % 360;
}

/**
 * Waypoint numbers for the boat at `pos` ([lon, lat]) sailing `sog`/`cog`
 * towards `to`, on the leg that started at `from`.
 */
export function waypointData(pos, sog, cog, from, to) {
  const dtw = distNm(pos, to);
  const btw = bearing(pos, to);
  const vmc = sog != null && cog != null ? sog * Math.cos(rad(cog - btw)) : null;
  const ttg = vmc != null && vmc > 0.1 ? (dtw / vmc) * 3600 : null;
  let xte = null;
  if (from) {
    // cross-track distance from the great circle from → to (+ = boat right of the track)
    const d13 = distNm(from, pos) / R_NM;
    const t13 = rad(bearing(from, pos)), t12 = rad(bearing(from, to));
    xte = Math.asin(Math.sin(d13) * Math.sin(t13 - t12)) * R_NM;
  }
  return { dtw, btw, vmc, ttg, xte };
}

/** The routing's point at time `t` (unix s): heading, TWA, BSP, interpolated. */
export function targetAt(points, t) {
  if (!points?.length || t < points[0].t || t > points.at(-1).t) return null;
  let i = points.findIndex((p) => p.t > t);
  if (i <= 0) return points.at(-1);
  const a = points[i - 1];
  return a; // the leg being sailed: targets hold until the next routing point
}
