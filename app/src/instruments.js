// Floating instrument panels: rows or columns of readings, dragged anywhere,
// always on top of the chart. Each reading can show a state ("stale", "warn",
// "alarm") that styles the tile, ready for user-set warnings and alarms later.

import { rhumb } from "./navmath.js";

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
  ],
  added: ["cursor", "grib", "sea"],
};

export function setupInstruments() {
  const root = document.getElementById("inst");
  const menu = document.getElementById("tile-menu");
  let layout = loadLayout();
  let ctx = { v: {}, now: Date.now(), age: Infinity, safetyDepth: 3 };

  function save() { try { localStorage.setItem(KEY, JSON.stringify(layout)); } catch { /* ignore */ } }

  // ---------- build ----------------------------------------------------------
  function build() {
    root.innerHTML = "";
    root.classList.toggle("locked", !!layout.locked);
    root.hidden = !!layout.hideAll;
    for (const p of layout.panels) {
      if (p.hidden) continue;
      const el = document.createElement("div");
      el.className = `ipanel ${p.collapsed ? "row collapsed" : p.dir} ${p.size}`;
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
        t.className = `tile ${p.size}${it?.wide ? " wide" : ""}${it?.graphic ? " graphic" : ""}`;
        t.dataset.i = i;
        t.innerHTML = `<div class="lbl"><span>${it ? it.label : key}</span><span class="sd"></span></div><div class="v">—</div>`;
        el.append(t);
      });
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
    // keep clear of the dock at the bottom when it is open
    const dockH = parseInt(getComputedStyle(document.documentElement).getPropertyValue("--dock-h")) || 0;
    const x = Math.max(0, Math.min(W - w, p.x * W));
    const y = Math.max(0, Math.min(H - dockH - h, p.y * H));
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
        // snap to the screen edges
        if (x < 12) x = 0;
        if (y < 12) y = 0;
        if (W - (x + r.width) < 12) x = W - r.width;
        if (H - (y + r.height) < 12) y = H - r.height;
        p.x = x / W; p.y = y / H;
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

  // ---------- values ----------------------------------------------------------
  function render() {
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
        if (!it) continue;
        let val = null;
        try { val = it.get(ctx); } catch { val = null; }
        const has = val !== null && val !== undefined && !(typeof val === "number" && !Number.isFinite(val));
        // a real true/false: classList.toggle(name, undefined) flips the class on every render
        const stale = Boolean(it.live) && ctx.age > STALE_S;
        const state = has && !stale && it.state ? it.state(val, ctx) : "";
        const html = has ? it.fmt(val) + (it.unit ? `<span class="u">${it.unit}</span>` : "") : "—";
        const v = t.querySelector(".v");
        if (v.innerHTML !== html) v.innerHTML = html;
        const sd = t.querySelector(".sd");
        const side = has && it.side ? it.side(val) : "";
        sd.className = `sd ${side}`;
        sd.textContent = side === "port" ? "PORT" : side === "stbd" ? "STBD" : "";
        t.classList.toggle("stale", stale);
        t.classList.toggle("warn", state === "warn");
        t.classList.toggle("alarm", state === "alarm");
      }
    }
  }
  setInterval(() => { ctx.now = Date.now(); ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000; render(); }, 1000);

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
    html += `<button data-act="dir">${p.dir === "row" ? "Stack vertically" : "Lay out horizontally"}</button><button data-act="delpanel">Remove panel</button>`;
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
      if (b.dataset.act === "add") p.tiles.splice(i + 1, 0, cur);
      if (b.dataset.act === "remove") { p.tiles.splice(i, 1); if (!p.tiles.length) layout.panels = layout.panels.filter((x) => x !== p); }
      if (b.dataset.act === "dir") p.dir = p.dir === "row" ? "column" : "row";
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
      li.innerHTML = `<input type="checkbox" title="Show this panel"><div class="main"><b>Panel ${k + 1}</b><small></small></div><button data-act="fold">${p.collapsed ? "Open" : "Fold"}</button><button data-act="dir">${p.dir === "row" ? "Vertical" : "Horizontal"}</button><button data-act="del">Remove</button>`;
      li.querySelector('[data-act="fold"]').onclick = () => toggleCollapsed(p);
      const cb = li.querySelector("input");
      cb.checked = !p.hidden;
      cb.onchange = () => { p.hidden = !cb.checked; save(); build(); };
      li.querySelector("small").textContent = p.tiles.map((t) => ITEMS[t]?.label ?? t).join(", ");
      li.querySelector('[data-act="dir"]').onclick = () => { p.dir = p.dir === "row" ? "column" : "row"; save(); build(); };
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
  try {
    const l = JSON.parse(localStorage.getItem(KEY) || "null");
    if (l && l.v === 1 && Array.isArray(l.panels)) {
      // panels added in later versions appear once in an existing layout
      l.added ??= [];
      if (!l.added.includes("cursor")) { l.panels.push(structuredClone(CURSOR_PANEL)); l.added.push("cursor"); }
      if (!l.added.includes("grib")) { l.panels.push(structuredClone(GRIB_PANEL)); l.added.push("grib"); }
      if (!l.added.includes("sea")) { l.panels.push(structuredClone(SEA_PANEL)); l.added.push("sea"); }
      if (!l.added.includes("mob")) { l.panels.push(structuredClone(MOB_PANEL)); l.added.push("mob"); }
      return l;
    }
  } catch { /* ignore */ }
  return structuredClone(DEFAULT_LAYOUT);
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
