// Floating instrument panels: rows or columns of readings, dragged anywhere,
// always on top of the chart. Each reading can show a state ("stale", "warn",
// "alarm") that styles the tile, ready for user-set warnings and alarms later.

import { rhumb } from "./navmath.js";
import { VIEWS } from "./views.js";
import { setupTiles } from "./insttiles.js";
import { columns, spanFromDrag, cleanSpan, moveReading, dropIndex, CELL, ROW } from "./gridmath.js";

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
  UTC: { label: "UTC", unit: "", group: "Navigation", get: (c) => c.now, fmt: (t) => new Date(t).toISOString().slice(11, 19) },
  LOCAL: { label: "Local", unit: "", group: "Navigation", get: (c) => c.now, fmt: (t) => new Date(t).toLocaleTimeString("en-NZ", { hour12: false }) },

  AWS: { label: "AWS", unit: "kn", group: "Wind", live: true, get: (c) => c.v.aws, fmt: f1 },
  AWA: { label: "AWA", unit: "°", group: "Wind", live: true, get: (c) => c.v.awa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  TWS: { label: "TWS", unit: "kn", group: "Wind", live: true, get: (c) => c.v.tws, fmt: f1 },
  TWA: { label: "TWA", unit: "°", group: "Wind", live: true, get: (c) => c.v.twa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  TWD: { label: "TWD", unit: "°T", group: "Wind", live: true, get: (c) => c.v.twd, fmt: deg3 },
  VMG: { label: "VMG", unit: "kn", group: "Wind", live: true, get: (c) => (c.v.twa != null && (c.v.stw ?? c.v.sog) != null ? (c.v.stw ?? c.v.sog) * Math.cos((c.v.twa * Math.PI) / 180) : null), fmt: (x) => f1(Math.abs(x)) },

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
  CRNG: { label: "Rng to cursor", unit: "NM", group: "Cursor", get: (c) => cursorLeg(c)?.nm ?? null, fmt: (x) => (x < 10 ? x.toFixed(2) : x.toFixed(1)) },

  // the GRIB shown on the chart, at the cursor and the slider time (Weather panel)
  GTWS: { label: "GRIB TWS", unit: "kn", group: "GRIB at cursor", get: (c) => c.grib?.tws, fmt: f1 },
  GTWD: { label: "GRIB TWD", unit: "°T", group: "GRIB at cursor", get: (c) => c.grib?.twd, fmt: deg3 },
  GGUST: { label: "GRIB gust", unit: "kn", group: "GRIB at cursor", get: (c) => c.grib?.gust, fmt: (x) => x.toFixed(0) },
  GMSLP: { label: "GRIB MSLP", unit: "hPa", group: "GRIB at cursor", get: (c) => c.grib?.mslp, fmt: (x) => x.toFixed(0) },
  GHS: { label: "GRIB Hs", unit: "m", group: "GRIB at cursor", get: (c) => c.grib?.wave, fmt: f1 },
  GTP: { label: "GRIB period", unit: "s", group: "GRIB at cursor", get: (c) => c.grib?.period, fmt: (x) => x.toFixed(0) },
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

// ---------- graphical instruments ----------------------------------------------------------
const polar = (r, deg) => [50 + r * Math.sin((deg * Math.PI) / 180), 50 - r * Math.cos((deg * Math.PI) / 180)];
const pt = (p) => `${p[0].toFixed(2)},${p[1].toFixed(2)}`;
function ticks(step, rIn, rOut, cls) {
  let d = "";
  for (let a = 0; a < 360; a += step) d += `M${pt(polar(rIn, a))}L${pt(polar(rOut, a))}`;
  return `<path d="${d}" class="${cls}"/>`;
}
/** Wind dial: boat up, apparent (orange) and true (blue) wind angle pointers, speeds in the middle. */
function windDial(w) {
  const sector = (a0, a1, cls) => {
    const p0 = polar(44, a0), p1 = polar(44, a1);
    return `<path d="M${pt(polar(38, a0))}L${pt(p0)}A44,44 0 0 ${a1 > a0 ? 1 : 0} ${pt(p1)}L${pt(polar(38, a1))}A38,38 0 0 ${a1 > a0 ? 0 : 1} ${pt(polar(38, a0))}z" class="${cls}"/>`;
  };
  const ptr = (a, cls, len) => (a == null ? "" : `<path d="M${pt(polar(len, a))}L${pt(polar(10, a - 9))}L${pt(polar(10, a + 9))}z" class="${cls}"/>`);
  return `<svg viewBox="-6 -1 112 108" class="dial">
    ${sector(20, 60, "close-stbd")}${sector(-60, -20, "close-port")}
    <circle cx="50" cy="50" r="44" class="ring"/>${ticks(10, 41, 44, "tk")}${ticks(30, 37, 44, "tk2")}
    <path d="M50,32 L56,47 L56,62 L50,68 L44,62 L44,47z" class="hull"/>
    ${ptr(w.twa, "ptr-t", 40)}${ptr(w.awa, "ptr-a", 43)}
    <text x="50" y="84" class="num a">${w.aws != null ? w.aws.toFixed(1) : "—"}</text>
    <text x="50" y="22" class="num t">${w.tws != null ? w.tws.toFixed(1) : "—"}</text>
    <text x="-5" y="105.5" class="cap">AWA ${w.awa != null ? Math.round(Math.abs(w.awa)) + "°" : "—"}</text><text x="105" y="105.5" class="cap end">TWA ${w.twa != null ? Math.round(Math.abs(w.twa)) + "°" : "—"}</text>
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
      <div class="hw-metric"><span data-hw="hdg-label">Heading · HDG</span><strong data-hw="hdg">—</strong></div>
      <div class="hw-metric hw-target"><span>Course to steer · CTS</span><strong data-hw="cts">—</strong></div>
      <div class="hw-metric"><span>Speed over ground</span><strong data-hw="sog">—</strong><small>SOG · kn</small></div>
      <div class="hw-metric"><span data-hw="dtw-label">Distance to waypoint</span><strong data-hw="dtw">—</strong><small>DTW · NM</small></div>
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
      <div class="hw-metric hw-xte"><span>Cross-track error · XTE</span><strong data-hw="xte">—</strong></div>
      <div class="hw-metric"><span>Time to go · TTG</span><strong data-hw="ttg">—</strong><small data-hw="vmg">VMG —</small></div>
      <div class="hw-metric"><span>Estimated arrival · ETA</span><strong data-hw="eta">—</strong></div>
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
  text("cts", angle(h?.cts));
  text("sog", h?.sog == null ? "—" : h.sog.toFixed(1));
  text("dtw", h?.range == null ? "—" : h.range.toFixed(h.range < 10 ? 2 : 1));
  text("dtw-label", h?.mob ? "Distance to MOB" : "Distance to waypoint");
  text("xte", h?.xte == null ? "—" : `${Math.abs(h.xte).toFixed(2)} NM`);
  text("ttg", highwayTime(h?.ttg));
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
      marks += `<path d="M${x} 0V22" class="hw-ribbon-tick"/><text x="${x}" y="40" text-anchor="middle" class="hw-ribbon-label">${String(bearing).padStart(3, "0")}</text>`;
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

const CURSOR_PANEL = { id: "pc", x: 0.45, y: 0.02, dir: "row", size: "s", tiles: ["CPOS", "CBRG", "CRNG"] };
const GRIB_PANEL = { id: "pg", x: 0.45, y: 0.11, dir: "row", size: "s", tiles: ["GTWS", "GTWD", "GGUST", "GMSLP", "GHS"] };
// shown only while an ocean file is loaded (`sea`)
// shown only while a man overboard is set (`mob`): big, near the top, below the alarm banner
const MOB_PANEL = { id: "pm", x: 0.4, y: 0.25, dir: "row", size: "l", mob: true, name: "Man overboard", tiles: ["MOBBRG", "MOBRNG", "MOBAGO", "MOBPOS"] };
const HIGHWAY_PANEL = { id: "phw", x: 0.35, y: 0.48, dir: "row", size: "l", mob: true, name: "Highway", tiles: ["HIGHWAY"] };
const SEA_PANEL = { id: "ps", x: 0.36, y: 0.2, dir: "row", size: "s", sea: true, name: "Sea at cursor", tiles: ["GSST", "GSKIN", "GSSS", "GCUR", "GSET", "GCHL"] };

const DEFAULT_LAYOUT = {
  v: 1,
  locked: false,
  panels: [
    { id: "p1", x: 0.3, y: 0.86, dir: "row", size: "m", tiles: ["SOG", "COG", "HDG", "DPT", "TWS", "TWA"] },
    { id: "p2", x: 0.86, y: 0.12, dir: "column", size: "s", tiles: ["NEXT", "BTW", "DTW", "TTG", "XTE"] },
    CURSOR_PANEL,
    GRIB_PANEL,
    SEA_PANEL,
    MOB_PANEL,
    HIGHWAY_PANEL,
  ],
  added: ["cursor", "grib", "sea", "mob", "highway"],
};

export function setupInstruments() {
  const root = document.getElementById("inst");
  const rail = document.getElementById("rail");
  const menu = document.getElementById("tile-menu");
  let layout = loadLayout();
  const tiles = setupTiles({ host: document.getElementById("itiles"), menu, items: ITEMS, getLayout: () => layout, save, paint: (t, it, fit) => paintTile(t, it, fit) });
  let ctx = { v: {}, now: Date.now(), age: Infinity, safetyDepth: 3 };

  function save() { try { localStorage.setItem(KEY, JSON.stringify(layout)); } catch { /* ignore */ } }
  const view = () => document.documentElement.dataset.view || "plan";
  // a panel without a list of views shows in every view; a man overboard shows everywhere
  // the Instruments view has no groups: it is one screen of single readings (insttiles.js)
  const inView = (p) => p.mob || (view() !== "inst" && (!p.views || p.views.includes(view())));
  window.addEventListener("racenav-view", () => build());
  // another window (a monitor, or the main window) changed the shared layout
  window.addEventListener("storage", (e) => {
    if (e.key !== KEY || !e.newValue) return;
    layout = loadLayout();
    build();
  });
  // position and size are kept per view: the chart overlay and the instruments screen differ
  const geo = (p) => p.vp?.[view()] ?? p;
  const own = (p) => { p.vp ??= {}; return (p.vp[view()] ??= { x: p.x, y: p.y }); };

  // ---------- build ----------------------------------------------------------
  function build() {
    tiles.build();
    root.innerHTML = "";
    root.classList.toggle("locked", !!layout.locked);
    root.hidden = !!layout.hideAll;
    for (const p of layout.panels) {
      if (p.hidden || !inView(p)) continue;
      const el = document.createElement("div");
      const g = geo(p);
      // a panel holding the highway is always a free grid: its fixed size would be far too big
      const hasHw = p.tiles.some((k) => ITEMS[k]?.highway);
      const avail = Math.max(120, window.innerHeight - (parseInt(getComputedStyle(document.documentElement).getPropertyValue("--dock-h")) || 0) - 8);
      const gw = g.w > 0 ? g.w : hasHw ? 420 : 0;
      const free = !p.collapsed && gw > 0;
      el.className = `ipanel ${p.collapsed ? "row collapsed" : free ? "free" : p.dir} ${p.size}`;
      if (free) {
        const maxW = Math.max(80, window.innerWidth - rail.getBoundingClientRect().right);
        const w = Math.min(gw, maxW);
        const cols = columns(w, p.size);
        const base = ROW[p.size] ?? ROW.m;
        // default spans: the highway fills the panel's width, three rows tall
        p.span = p.tiles.map((k, i) => p.span?.[i] ?? (ITEMS[k]?.highway ? [cols, 3] : null));
        const area = p.tiles.reduce((n, _, i) => { const [c, r] = cleanSpan(p.span[i]); return n + Math.min(c, cols) * r; }, 0);
        const rows = Math.max(1, Math.ceil(area / cols));
        // too many rows for the screen: make the rows shorter, never taller than the screen
        const rh = Math.max(28, Math.min(base, Math.floor((avail - 24 - rows * 3) / rows)));
        el.style.width = `${w}px`;
        el.style.maxHeight = `${avail}px`;
        if (g.h > 0) el.style.minHeight = `${Math.min(g.h, avail)}px`;
        el.style.setProperty("--cols", cols);
        el.style.setProperty("--rh", `${rh}px`);
      }
      el.dataset.id = p.id;
      const grip = document.createElement("div");
      grip.className = "grip";
      grip.textContent = "⋮⋮";
      grip.title = p.collapsed ? "Double-click (or double-tap) to open this panel; drag to move it"
        : "Drag to move this panel; double-click (or double-tap) to fold it away";
      el.append(grip);
      if (p.collapsed) {
        const t = document.createElement("div");
        t.className = "ptitle";
        t.textContent = panelTitle(p);
        t.title = "Double-click to open";
        t.ondblclick = () => toggleCollapsed(p);
        el.append(t);
      }
      if (!p.collapsed) p.tiles.forEach((key, i) => {
        const it = ITEMS[key];
        const t = document.createElement("div");
        t.className = `tile ${p.size}${it?.wide ? " wide" : ""}${it?.graphic ? " graphic" : ""}${it?.highway ? " highway-card" : ""}`;
        t.dataset.i = i;
        t.innerHTML = `<div class="lbl"><span>${it ? it.label : key}</span><span class="sd"></span></div><div class="v">—</div>`;
        const sp = cleanSpan(p.span?.[i]);
        if (free) {
          const cs = Math.min(sp[0], Number(el.style.getPropertyValue("--cols")) || sp[0]);
          if (cs > 1) t.style.gridColumn = `span ${cs}`;
          if (sp[1] > 1) t.style.gridRow = `span ${sp[1]}`;
          if (!layout.locked) {
            const h = document.createElement("div");
            h.className = "trsz";
            h.title = "Drag to make this reading span more or fewer cells";
            h.addEventListener("click", (e) => e.stopPropagation());
            t.append(h);
          }
        }
        el.append(t);
      });
      if (!p.collapsed && !layout.locked) {
        const h = document.createElement("div");
        h.className = "prsz";
        h.title = "Drag to resize this panel; the readings re-flow to fit";
        el.append(h);
        resizePanel(el, h, p);
      }
      root.append(el);
      place(el, p);
      dragByGrip(el, grip, p);
    }
    renderList();
    render();
  }

  function place(el, p) {
    const W = window.innerWidth, H = window.innerHeight;
    const w = el.offsetWidth, h = el.offsetHeight;
    const minX = rail.getBoundingClientRect().right;
    // keep clear of the dock at the bottom when it is open
    const dockH = parseInt(getComputedStyle(document.documentElement).getPropertyValue("--dock-h")) || 0;
    const gg = geo(p);
    const x = Math.max(minX, Math.min(Math.max(minX, W - w), gg.x * W));
    const y = Math.max(0, Math.min(H - dockH - h, gg.y * H));
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
  }
  const replace = () => { for (const el of root.children) place(el, panel(el.dataset.id)); };
  window.addEventListener("resize", replace);
  window.addEventListener("racenav-layout", replace);
  const panel = (id) => layout.panels.find((p) => p.id === id);
  const panelTitle = (p) => p.name || p.tiles.slice(0, 3).map((t) => ITEMS[t]?.label ?? t).join(" · ") + (p.tiles.length > 3 ? " …" : "");
  function toggleCollapsed(p) { p.collapsed = !p.collapsed; save(); build(); }

  function dragByGrip(el, grip, p) {
    // A tap that doesn't move is a tap; two taps close together fold or open
    // the panel (works with a mouse double-click and a finger double-tap).
    let lastTap = 0;
    grip.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      grip.setPointerCapture(e.pointerId);
      const r = el.getBoundingClientRect();
      const dx = e.clientX - r.left, dy = e.clientY - r.top;
      const x0 = e.clientX, y0 = e.clientY;
      let moved = false;
      if (!layout.locked) grip.style.cursor = "grabbing";
      const move = (ev) => {
        if (Math.abs(ev.clientX - x0) + Math.abs(ev.clientY - y0) > 6) moved = true;
        if (layout.locked || !moved) return;
        const W = window.innerWidth, H = window.innerHeight;
        let x = ev.clientX - dx, y = ev.clientY - dy;
        const minX = rail.getBoundingClientRect().right;
        // snap to the screen edges
        if (x - minX < 12) x = minX;
        if (y < 12) y = 0;
        if (W - (x + r.width) < 12) x = W - r.width;
        if (H - (y + r.height) < 12) y = H - r.height;
        x = Math.max(minX, Math.min(Math.max(minX, W - r.width), x));
        const o = own(p);
        o.x = x / W; o.y = y / H;
        place(el, p);
      };
      const up = () => {
        grip.removeEventListener("pointermove", move);
        grip.removeEventListener("pointerup", up);
        grip.style.cursor = "";
        if (moved) { save(); return; }
        const now = Date.now();
        if (now - lastTap < 400) { lastTap = 0; toggleCollapsed(p); } else lastTap = now;
      };
      grip.addEventListener("pointermove", move);
      grip.addEventListener("pointerup", up);
    });
  }

  // Resize a panel from its corner: the width sets how many columns of readings fit.
  function resizePanel(el, handle, p) {
    handle.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      handle.setPointerCapture(e.pointerId);
      const r = el.getBoundingClientRect();
      const x0 = e.clientX, y0 = e.clientY;
      el.classList.add("free");
      el.style.setProperty("--rh", `${ROW[p.size] ?? ROW.m}px`);
      const limit = () => ({
        w: Math.max(60, Math.min(window.innerWidth - r.left, r.width + (lastX - x0))),
        h: Math.max(40, Math.min(window.innerHeight - 8 - r.top, r.height + (lastY - y0))),
      });
      let lastX = x0, lastY = y0;
      const move = (ev) => {
        lastX = ev.clientX; lastY = ev.clientY;
        const s = limit();
        el.style.width = `${s.w}px`;
        el.style.minHeight = `${s.h}px`;
        el.style.setProperty("--cols", columns(s.w, p.size));
      };
      const up = () => {
        handle.removeEventListener("pointermove", move);
        handle.removeEventListener("pointerup", up);
        const s = limit();
        const o = own(p);
        o.w = Math.round(s.w); o.h = Math.round(s.h);
        save(); build();
      };
      handle.addEventListener("pointermove", move);
      handle.addEventListener("pointerup", up);
    });
  }

  // A reading's corner handle sets how many columns and rows it covers; dragging
  // the reading itself moves it to another place in this panel or in another one.
  let swallowClick = false;
  root.addEventListener("click", (e) => { if (swallowClick) { swallowClick = false; e.stopPropagation(); e.preventDefault(); } }, true);
  root.addEventListener("pointerdown", (e) => {
    if (layout.locked || e.button > 0) return;
    const t = e.target.closest(".tile");
    if (!t) return;
    const p = panel(t.parentElement.dataset.id);
    const i = Number(t.dataset.i);
    if (!p) return;
    if (e.target.closest(".trsz")) return startTileResize(e, t, p, i);
    startTileDrag(e, t, p, i);
  });

  function startTileResize(e, t, p, i) {
    e.preventDefault();
    e.stopPropagation();
    const h = e.target;
    h.setPointerCapture(e.pointerId);
    const r = t.getBoundingClientRect();
    const [c0, r0] = cleanSpan(p.span?.[i]);
    const x0 = e.clientX, y0 = e.clientY;
    let cur = [c0, r0];
    const move = (ev) => {
      cur = [spanFromDrag(r.width, ev.clientX - x0, r.width / c0), spanFromDrag(r.height, ev.clientY - y0, r.height / r0)];
      t.style.gridColumn = `span ${cur[0]}`;
      t.style.gridRow = `span ${cur[1]}`;
    };
    const up = () => {
      h.removeEventListener("pointermove", move);
      h.removeEventListener("pointerup", up);
      p.span = p.tiles.map((_, k) => (k === i ? cur : cleanSpan(p.span?.[k])));
      save(); build();
    };
    h.addEventListener("pointermove", move);
    h.addEventListener("pointerup", up);
  }

  function startTileDrag(e, t, p, i) {
    const x0 = e.clientX, y0 = e.clientY;
    let dragging = false, mark = null, place_ = null;
    const clear = () => { mark?.classList.remove("drop-before", "drop-after"); mark = null; };
    const target = (ev) => {
      const u = document.elementFromPoint(ev.clientX, ev.clientY);
      const tt = u?.closest?.(".tile");
      if (tt && root.contains(tt) && tt !== t) {
        const q = panel(tt.parentElement.dataset.id);
        const b = tt.getBoundingClientRect();
        const wide = !tt.parentElement.classList.contains("column");
        return { el: tt, q, idx: dropIndex(Number(tt.dataset.i), (ev.clientX - b.left) / b.width, (ev.clientY - b.top) / b.height, wide) };
      }
      const pe = u?.closest?.(".ipanel");
      if (pe && root.contains(pe) && !tt) {
        const q = panel(pe.dataset.id);
        return q && !q.collapsed ? { el: null, q, idx: q.tiles.length } : null;
      }
      return null;
    };
    const move = (ev) => {
      if (!dragging) {
        if (Math.abs(ev.clientX - x0) + Math.abs(ev.clientY - y0) < 8) return;
        dragging = true;
        t.classList.add("dragging");
      }
      clear();
      place_ = target(ev);
      if (place_?.el) {
        mark = place_.el;
        mark.classList.add(place_.idx > Number(mark.dataset.i) ? "drop-after" : "drop-before");
      }
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      clear();
      t.classList.remove("dragging");
      if (!dragging) return;
      swallowClick = true;
      setTimeout(() => { swallowClick = false; }, 50);
      if (!place_) return;
      const { q, idx } = place_;
      if (q === p) {
        const to = idx > i ? idx - 1 : idx;
        if (to === i) return;
        const m = moveReading(p.tiles, p.span, i, to);
        p.tiles = m.tiles; p.span = m.spans;
      } else {
        const key = p.tiles[i], sp = p.span?.[i] ?? null;
        p.tiles.splice(i, 1);
        if (p.span) p.span.splice(i, 1);
        q.span = q.tiles.map((_, k) => q.span?.[k] ?? null);
        q.tiles.splice(idx, 0, key);
        q.span.splice(idx, 0, sp);
        if (!p.tiles.length) layout.panels = layout.panels.filter((x) => x !== p);
      }
      save(); build();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  }

  // ---------- values ----------------------------------------------------------
  function render() {
    ctx.now = Date.now();
    for (const el of root.children) {
      const p = panel(el.dataset.id);
      if (!p) continue;
      // the sea panel: only while an ocean file is loaded; the MOB panel: only during a MOB
      const off = (!!p.sea && !ctx.seaLoaded) || (!!p.mob && !ctx.mob);
      if ((el.style.display === "none") !== off) {
        el.style.display = off ? "none" : "";
        if (!off) place(el, p);
      }
      for (const t of el.querySelectorAll(".tile")) {
        const it = ITEMS[p.tiles[Number(t.dataset.i)]];
        if (it) paintTile(t, it, el.classList.contains("free"));
      }
    }
    tiles.render();
  }
  /** Show one reading in its tile; `fit` scales the text (or the highway) to fill the tile. */
  function paintTile(t, it, fit) {
    let val = null;
    try { val = it.get(ctx); } catch { val = null; }
    const has = val !== null && val !== undefined && !(typeof val === "number" && !Number.isFinite(val));
    // a real true/false: classList.toggle(name, undefined) flips the class on every render
    const stale = Boolean(it.live) && ctx.age > STALE_S;
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
  setInterval(() => { ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000; render(); }, 250);

  // ---------- tile menu ----------------------------------------------------------
  root.addEventListener("click", (e) => {
    const t = e.target.closest(".tile");
    if (!t) return;
    const p = panel(t.parentElement.dataset.id);
    openMenu(p, Number(t.dataset.i), t.getBoundingClientRect());
  });
  function openMenu(p, i, rect) {
    const cur = p.tiles[i];
    const groups = {};
    for (const [k, it] of Object.entries(ITEMS)) (groups[it.group] ??= []).push([k, it]);
    let html = "";
    for (const [g, items] of Object.entries(groups)) {
      html += `<div class="grp">${g}</div><div class="items">`;
      for (const [k, it] of items) html += `<button data-set="${k}" class="${k === cur ? "cur" : ""}">${it.label}${it.unit ? ` <small>${it.unit}</small>` : ""}</button>`;
      html += "</div>";
    }
    html += `<hr><button data-act="add">Add a reading after this one</button><button data-act="remove">Remove this reading</button>`;
    html += `<hr><div class="grp">This panel</div><div class="sizes">${["s", "m", "l"].map((s) => `<button data-size="${s}" class="${p.size === s ? "cur" : ""}">${{ s: "Small", m: "Medium", l: "Large" }[s]}</button>`).join("")}</div>`;
    html += `<button data-act="dir">${p.dir === "row" ? "Stack vertically" : "Lay out horizontally"}</button>`;
    if (geo(p).w > 0 || p.span?.some(Boolean)) html += `<button data-act="auto">Automatic size and spans</button>`;
    html += `<button data-act="delpanel">Remove panel</button>`;
    menu.innerHTML = html;
    menu.hidden = false;
    // keep it on screen
    const mw = 300, mh = Math.min(menu.scrollHeight, window.innerHeight * 0.8);
    let x = rect.right + 6, y = rect.top;
    if (x + mw > window.innerWidth) x = rect.left - mw - 6;
    if (y + mh > window.innerHeight) y = window.innerHeight - mh - 8;
    menu.style.left = `${Math.max(4, x)}px`;
    menu.style.top = `${Math.max(4, y)}px`;
    menu.onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      if (b.dataset.set) p.tiles[i] = b.dataset.set;
      if (b.dataset.size) p.size = b.dataset.size;
      if (b.dataset.act === "add") { p.tiles.splice(i + 1, 0, cur); p.span?.splice(i + 1, 0, null); }
      if (b.dataset.act === "remove") { p.tiles.splice(i, 1); p.span?.splice(i, 1); if (!p.tiles.length) layout.panels = layout.panels.filter((x) => x !== p); }
      if (b.dataset.act === "dir") { p.dir = p.dir === "row" ? "column" : "row"; if (p.vp?.[view()]) { delete p.vp[view()].w; delete p.vp[view()].h; } }
      if (b.dataset.act === "auto") { const o = p.vp?.[view()]; if (o) { delete o.w; delete o.h; } p.span = undefined; }
      if (b.dataset.act === "delpanel") layout.panels = layout.panels.filter((x) => x !== p);
      menu.hidden = true;
      save();
      build();
    };
  }
  document.addEventListener("pointerdown", (e) => { if (!menu.hidden && !menu.contains(e.target) && !e.target.closest(".tile")) menu.hidden = true; });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !menu.hidden) { menu.hidden = true; e.preventDefault(); } });

  // ---------- drawer controls ----------------------------------------------------------
  const lockBtn = document.getElementById("inst-lock");
  function renderList() {
    document.getElementById("inst-fold").textContent = layout.panels.some((p) => !p.collapsed) ? "Fold all" : "Open all";
    lockBtn.classList.toggle("on", !!layout.locked);
    lockBtn.textContent = layout.locked ? "Layout locked" : "Lock layout";
    document.getElementById("inst-all").checked = !layout.hideAll;
    const ul = document.getElementById("inst-panels");
    ul.innerHTML = "";
    if (!layout.panels.length) ul.innerHTML = `<li class="empty">No panels. Add one.</li>`;
    layout.panels.forEach((p, k) => {
      const li = document.createElement("li");
      li.innerHTML = `<input type="checkbox" title="Show this panel"><div class="main"><input type="text" class="pname" maxlength="40" title="Rename this group of instruments"><small></small><div class="pviews"></div></div><button data-act="fold">${p.collapsed ? "Open" : "Fold"}</button><button data-act="dir">${p.dir === "row" ? "Vertical" : "Horizontal"}</button><button data-act="del">Remove</button>`;
      const nm = li.querySelector(".pname");
      nm.value = p.name || "";
      nm.placeholder = `Panel ${k + 1}`;
      nm.onclick = (e) => e.stopPropagation();
      nm.onchange = () => { p.name = nm.value.trim() || undefined; save(); build(); };
      const pv = li.querySelector(".pviews");
      for (const v of VIEWS.filter((x) => x.id !== "inst")) {
        const lab = document.createElement("label");
        lab.title = `Show this group in the ${v.name} view`;
        const c = document.createElement("input");
        c.type = "checkbox";
        c.checked = !p.views || p.views.includes(v.id);
        c.onchange = () => {
          const set = new Set(p.views ?? VIEWS.map((x) => x.id));
          if (c.checked) set.add(v.id); else set.delete(v.id);
          p.views = VIEWS.map((x) => x.id).filter((id) => set.has(id));
          save(); build();
        };
        lab.append(c, ` ${v.name}`);
        pv.append(lab);
      }
      li.querySelector('[data-act="fold"]').onclick = () => toggleCollapsed(p);
      const cb = li.querySelector("input");
      cb.checked = !p.hidden;
      cb.onchange = () => { p.hidden = !cb.checked; save(); build(); };
      li.querySelector("small").textContent = p.tiles.map((t) => ITEMS[t]?.label ?? t).join(", ");
      li.querySelector('[data-act="dir"]').onclick = () => { p.dir = p.dir === "row" ? "column" : "row"; const o = p.vp?.[view()]; if (o) { delete o.w; delete o.h; } save(); build(); };
      li.querySelector('[data-act="del"]').onclick = () => { layout.panels.splice(k, 1); save(); build(); };
      ul.append(li);
    });
  }
  document.getElementById("inst-add").onclick = () => {
    layout.panels.push({ id: `p${Date.now().toString(36)}`, x: 0.45, y: 0.4, dir: "row", size: "m", tiles: ["SOG", "TWS"] });
    save(); build();
  };
  document.getElementById("inst-fold").onclick = () => {
    const fold = layout.panels.some((p) => !p.collapsed);
    for (const p of layout.panels) p.collapsed = fold;
    save(); build();
  };
  document.getElementById("it-add").onclick = () => { if (!tiles.add()) alert("The Instruments screen is full: make a tile smaller or remove one first."); };
  document.getElementById("it-reset").onclick = () => tiles.reset();
  lockBtn.onclick = () => { layout.locked = !layout.locked; save(); build(); };
  document.getElementById("inst-all").onchange = (e) => { layout.hideAll = !e.target.checked; save(); build(); };
  document.getElementById("inst-reset").onclick = () => { layout = structuredClone(DEFAULT_LAYOUT); save(); build(); };

  build();

  return {
    /** New data: vessel state, route/waypoint and routing targets. */
    update(next) {
      // a new man overboard: its panel comes up even if it was folded or hidden
      const newMob = next.mob && !ctx.mob;
      ctx = { ...ctx, ...next, now: Date.now() };
      if (newMob) {
        let p = layout.panels.find((x) => x.mob);
        if (!p) { p = structuredClone(MOB_PANEL); layout.panels.push(p); }
        p.hidden = false;
        p.collapsed = false;
        if (!p.tiles.includes("MOBPOS")) p.tiles.push("MOBPOS");
        let highwayPanel = layout.panels.find((x) => x.id === HIGHWAY_PANEL.id);
        if (!highwayPanel) { highwayPanel = structuredClone(HIGHWAY_PANEL); layout.panels.push(highwayPanel); }
        highwayPanel.hidden = false;
        highwayPanel.collapsed = false;
        if (layout.hideAll) layout.hideAll = false;
        save();
        build();
      }
      ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000;
      render();
    },
  };
}

function loadLayout() {
  let l = null;
  try {
    l = JSON.parse(localStorage.getItem(KEY) || "null");
    if (l && l.v === 1 && Array.isArray(l.panels)) {
      // panels added in later versions appear once in an existing layout
      l.added ??= [];
      if (!l.added.includes("cursor")) { l.panels.push(structuredClone(CURSOR_PANEL)); l.added.push("cursor"); }
      if (!l.added.includes("grib")) { l.panels.push(structuredClone(GRIB_PANEL)); l.added.push("grib"); }
      if (!l.added.includes("sea")) { l.panels.push(structuredClone(SEA_PANEL)); l.added.push("sea"); }
      if (!l.added.includes("mob")) { l.panels.push(structuredClone(MOB_PANEL)); l.added.push("mob"); }
      if (!l.added.includes("highway")) { l.panels.push(structuredClone(HIGHWAY_PANEL)); l.added.push("highway"); }
    } else l = null;
  } catch { l = null; }
  l ??= structuredClone(DEFAULT_LAYOUT);
  addCockpitGroup(l);
  return l;
}

/** The old cockpit window's readings become a group for the Instruments view, once. */
function addCockpitGroup(l) {
  l.added ??= [];
  if (l.added.includes("cockpit")) return;
  l.added.push("cockpit");
  try {
    const old = JSON.parse(localStorage.getItem("racenav.cockpit") || "null");
    const tiles = (old?.tiles ?? []).filter((k) => ITEMS[k]);
    if (!tiles.length) return;
    const cols = Math.max(1, Math.min(5, Number(old.cols) || 3));
    l.panels.push({
      id: "pck", x: 0.03, y: 0.04, dir: "row", size: "l", name: "Cockpit", views: ["inst"], tiles,
      vp: { inst: { x: 0.03, y: 0.04, w: Math.round(cols * CELL.l) } },
    });
  } catch { /* ignore */ }
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
