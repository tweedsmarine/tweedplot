// Instrument history on screen: the recorder every window feeds once a second (histcore.js),
// kept over a restart by the main window, drawn as a trend plot in an instrument tile or in a
// floating card (right-click a reading). For nowcasting: a day of wind and pressure at a glance,
// with the three-hour pressure tendency in words.
import { Recorder, CHANNELS, KEEP_HOURS, tendencyWords, unwrap, wrap360, wrap180, MIN_MS } from "./histcore.js";
import { PRIMARY, own } from "./role.js";
import { floatResize } from "./floatresize.js";

export { CHANNELS };

const STORE = "racenav.history"; // the main window's minute bins (all windows read it)
const PREFS = "racenav.history.prefs";
const CARDS = own("racenav.history.cards");
export const WINDOWS = [0.25, 1, 3, 6, 12, 24, 48, 72]; // hours across a plot

const prefs = { keep: 24 };
try { Object.assign(prefs, JSON.parse(localStorage.getItem(PREFS) || "{}")); } catch { /* defaults */ }
if (!KEEP_HOURS.includes(prefs.keep)) prefs.keep = 24;

export const recorder = new Recorder({ keepHours: prefs.keep });
try { recorder.load(JSON.parse(localStorage.getItem(STORE) || "null")); } catch { /* damaged: start empty */ }

let dirty = false, savedAt = 0;
function save(force = false) {
  if (!PRIMARY || !dirty) return;
  // a minute's new bins are written every five minutes (and when the window closes)
  if (!force && Date.now() - savedAt < 5 * MIN_MS) return;
  savedAt = Date.now();
  dirty = false;
  try { localStorage.setItem(STORE, JSON.stringify(recorder.toJSON())); } catch { /* full or unavailable: memory only */ }
}
if (PRIMARY) {
  window.addEventListener("beforeunload", () => save(true));
  window.addEventListener("pagehide", () => save(true));
} else {
  // a monitor window takes the main window's record when it is written
  window.addEventListener("storage", (e) => {
    if (e.key !== STORE || !e.newValue) return;
    try { recorder.load(JSON.parse(e.newValue)); } catch { /* ignore */ }
  });
}

/** Feed the recorder (called four times a second; it samples once a second). Stale or
 *  replayed data is not recorded: a gap is left. */
export function sample(v, fresh) {
  if (recorder.sample(fresh ? v : {})) { dirty = true; save(); }
}

export const keepHours = () => prefs.keep;
export function setKeepHours(h) {
  if (!KEEP_HOURS.includes(h)) return;
  prefs.keep = h;
  recorder.keepHours = h;
  recorder.trim();
  dirty = true;
  try { localStorage.setItem(PREFS, JSON.stringify(prefs)); } catch { /* ignore */ }
  save(true);
}
export function clearHistory() {
  for (const k of Object.keys(CHANNELS)) { recorder.bins[k] = []; recorder.raw[k] = []; recorder.cur[k] = {}; }
  dirty = true;
  save(true);
}

// ---------- drawing ------------------------------------------------------------------
const cssVar = (name, d) => getComputedStyle(document.documentElement).getPropertyValue(name).trim() || d;
const colourOf = (ch) => (ch.tint === "t" ? cssVar("--wind-true", "#2f8fe6") : ch.tint === "a" ? cssVar("--wind-app", "#f57c00") : "#4fd1c5");
const MIN_SPAN = { TWS: 6, AWS: 6, SOG: 2, BSP: 2, PRES: 6, AIRT: 3, WTEMP: 1, HUM: 10, DPT: 5, TWD: 30, AWA: 30 };

const niceStep = (span, n = 4) => {
  const raw = span / n, p = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= raw) ?? raw;
};
const tickStep = (h) => (h <= 0.25 ? 5 : h <= 1 ? 10 : h <= 3 ? 30 : h <= 6 ? 60 : h <= 12 ? 120 : h <= 24 ? 180 : 360) * 60e3;
const hhmm = (t) => new Date(t).toLocaleTimeString("en-NZ", { hour: "2-digit", minute: "2-digit", hour12: false });
const fmtV = (ch, x) => (ch.dir === "true" ? String(Math.round(wrap360(x)) % 360).padStart(3, "0") : ch.dir === "rel" ? `${Math.round(Math.abs(wrap180(x)))}` : x.toFixed(ch.dp));

/** The headline: value, and for pressure the tendency, for wind the strongest gust. */
export function headline(k, now = Date.now()) {
  const ch = CHANNELS[k];
  const v = recorder.latest(k);
  let note = "";
  if (ch.tendency) {
    const d = recorder.change(k, 3, now);
    note = d == null ? "Tendency after 3 h of data" : `${d > 0 ? "+" : d < 0 ? "−" : "±"}${Math.abs(d).toFixed(1)} hPa / 3 h · ${tendencyWords(d)}`;
  } else if (k === "TWS" || k === "AWS") {
    const s = recorder.series(k, now - 10 * MIN_MS, now);
    const g = s.reduce((m, p) => Math.max(m, p.hi ?? p.m), -Infinity);
    if (Number.isFinite(g)) note = `max ${g.toFixed(1)} kn in 10 min`;
  }
  const side = ch.dir === "rel" && v != null ? (wrap180(v) < 0 ? " P" : " S") : "";
  return { value: v == null ? "—" : fmtV(ch, v) + side, unit: ch.unit, note, falling: ch.tendency && recorder.change(k, 3, now) <= -3.6 };
}

/** Draw reading `k` over the last `hours` into canvas `cv` (sized by CSS). */
export function drawHistory(cv, k, hours, { compact = false } = {}) {
  const ch = CHANNELS[k];
  const dpr = window.devicePixelRatio || 1;
  const W = Math.max(1, Math.round(cv.clientWidth * dpr)), H = Math.max(1, Math.round(cv.clientHeight * dpr));
  if (cv.width !== W || cv.height !== H) { cv.width = W; cv.height = H; }
  const g = cv.getContext("2d");
  g.clearRect(0, 0, W, H);
  const font = (px, bold) => `${bold ? "bold " : ""}${Math.round(px * dpr)}px system-ui, sans-serif`;
  const quiet = cssVar("--tile-quiet", "#9fb2bd"), col = colourOf(ch);
  const now = Date.now(), span = hours * 3600e3, t0 = now - span;
  const pts = recorder.series(k, t0, now);
  const T = (compact ? 4 : 6) * dpr, B = 14 * dpr, L = 6 * dpr;

  // values (directions unwrapped so a veer through north is one line)
  let ms = pts.map((p) => p.m);
  if (ch.dir) ms = unwrap(ms);
  const vals = [];
  pts.forEach((p, i) => { vals.push(ms[i]); if (!ch.dir && p.lo != null) vals.push(p.lo, p.hi); });
  let lo = Math.min(...vals), hi = Math.max(...vals);
  if (!vals.length) { lo = 0; hi = 1; }
  const minSpan = MIN_SPAN[k] ?? 1;
  if (hi - lo < minSpan) { const c = (hi + lo) / 2; lo = c - minSpan / 2; hi = c + minSpan / 2; }
  if (!ch.dir && ["TWS", "AWS", "SOG", "BSP", "DPT"].includes(k)) lo = Math.max(0, lo);
  const st = ch.dir ? [10, 15, 30, 45, 90].find((s) => s >= (hi - lo) / 4) ?? 90 : niceStep(hi - lo);
  lo = Math.floor(lo / st) * st; hi = Math.ceil(hi / st) * st;
  // depth reads downwards, like the sounder
  const down = k === "DPT";
  // room on the right for the widest value label
  g.font = font(10);
  let lw = 0;
  for (let v = lo; v <= hi + 1e-9; v += st) lw = Math.max(lw, g.measureText(fmtV(ch, v)).width);
  const R = lw + 8 * dpr;
  const pw = Math.max(1, W - L - R), ph = Math.max(1, H - T - B);
  const X = (t) => L + ((t - t0) / span) * pw;
  const Y = (v) => T + (down ? (v - lo) / (hi - lo) : (hi - v) / (hi - lo)) * ph;

  // grid: values on the right, local times along the bottom
  g.lineWidth = Math.max(1, dpr * 0.6);
  g.font = font(10); g.textAlign = "left"; g.textBaseline = "middle";
  for (let v = lo; v <= hi + 1e-9; v += st) {
    g.strokeStyle = "rgba(160,180,190,.18)"; g.beginPath(); g.moveTo(L, Y(v)); g.lineTo(L + pw, Y(v)); g.stroke();
    g.fillStyle = quiet; g.fillText(fmtV(ch, v), L + pw + 4 * dpr, Y(v));
  }
  const tk = tickStep(hours);
  g.textAlign = "center"; g.textBaseline = "top";
  // ticks on the local clock (whole hours for steps of an hour or more)
  const off = new Date(now).getTimezoneOffset() * 60e3;
  for (let t = Math.ceil((t0 - off) / tk) * tk + off; t < now; t += tk) {
    const x = X(t);
    g.strokeStyle = "rgba(160,180,190,.13)"; g.beginPath(); g.moveTo(x, T); g.lineTo(x, T + ph); g.stroke();
    g.fillStyle = quiet; g.fillText(hhmm(t), Math.min(L + pw - 14 * dpr, Math.max(L + 14 * dpr, x)), T + ph + 2 * dpr);
  }

  if (!pts.length) {
    g.textAlign = "center"; g.textBaseline = "middle"; g.font = font(12); g.fillStyle = quiet;
    g.fillText(recorder.has(k) ? `No ${ch.label.toLowerCase()} in this time` : `Waiting for ${ch.label.toLowerCase()} data`, L + pw / 2, T + ph / 2);
    return;
  }

  // runs of points without a gap: more than three steps (15 s between five-second points,
  // 3 min between minute bins) leaves a gap, never a made-up line
  const runs = [];
  pts.forEach((p, i) => {
    const q = pts[i - 1];
    if (!i || p.t - q.t > (p.raw && q.raw ? 15_000 : 3 * MIN_MS)) runs.push([]);
    runs.at(-1).push(i);
  });
  const mid = (p) => p.t + (p.raw ? 2500 : MIN_MS / 2);
  // the range (gusts and lulls) as a band
  if (!ch.dir) {
    g.fillStyle = col; g.globalAlpha = 0.22;
    for (const r of runs) {
      const rr = r.filter((i) => pts[i].lo != null);
      if (rr.length < 2) continue;
      g.beginPath();
      rr.forEach((i, j) => (j ? g.lineTo(X(mid(pts[i])), Y(pts[i].hi)) : g.moveTo(X(mid(pts[i])), Y(pts[i].hi))));
      for (let j = rr.length - 1; j >= 0; j--) g.lineTo(X(mid(pts[rr[j]])), Y(pts[rr[j]].lo));
      g.closePath(); g.fill();
    }
    g.globalAlpha = 1;
  }
  // the mean
  g.strokeStyle = col; g.lineWidth = 2 * dpr; g.lineJoin = "round";
  for (const r of runs) {
    g.beginPath();
    r.forEach((i, j) => (j ? g.lineTo(X(mid(pts[i])), Y(ms[i])) : g.moveTo(X(mid(pts[i])), Y(ms[i]))));
    if (r.length === 1) { g.arc(X(mid(pts[r[0]])), Y(ms[r[0]]), 1.5 * dpr, 0, 7); g.fillStyle = col; g.fill(); }
    g.stroke();
  }
}

// ---------- a tile (Instruments view or a panel) ------------------------------------------
const tilePrefs = {};
try { Object.assign(tilePrefs, JSON.parse(localStorage.getItem(`${PREFS}.tiles`) || "{}")); } catch { /* ignore */ }
const winLabel = (h) => (h < 1 ? `${Math.round(h * 60)} min` : `${h} h`);
const nextWin = (h) => {
  const list = WINDOWS.filter((w) => w <= prefs.keep);
  return list[(list.indexOf(h) + 1) % list.length] ?? list[0];
};

function head(k) {
  const h = headline(k);
  return `<b class="hv">${h.value}</b><small>${h.unit}</small><span class="hn${h.falling ? " warn" : ""}">${h.note}</span>`;
}

/** An instrument tile's render(): built once, redrawn every two seconds or on resize. */
export function renderHistoryTile(k) {
  return (host) => {
    if (host.dataset.hist !== k) {
      host.dataset.hist = k;
      host.classList.add("hist-host");
      host.innerHTML = `<div class="hh">${head(k)}</div><canvas class="hist-cv"></canvas><button class="hist-win" title="How much history across the plot (click to change)"></button>`;
      const b = host.querySelector(".hist-win");
      b.textContent = winLabel(tilePrefs[k] ?? 6);
      b.onclick = (e) => {
        e.stopPropagation();
        tilePrefs[k] = nextWin(tilePrefs[k] ?? 6);
        b.textContent = winLabel(tilePrefs[k]);
        try { localStorage.setItem(`${PREFS}.tiles`, JSON.stringify(tilePrefs)); } catch { /* ignore */ }
        host._at = 0;
      };
      host._at = 0;
    }
    const cv = host.querySelector("canvas");
    const size = `${cv.clientWidth}x${cv.clientHeight}`;
    if (Date.now() - (host._at ?? 0) < 2000 && host._size === size) return;
    host._at = Date.now(); host._size = size;
    host.querySelector(".hh").innerHTML = head(k);
    drawHistory(cv, k, Math.min(tilePrefs[k] ?? 6, prefs.keep), { compact: true });
  };
}

// ---------- floating cards (right-click a reading) ----------------------------------------
const open = new Map(); // k → { el, hours }
function saveCards() {
  const list = [...open.entries()].map(([k, c]) => ({ k, h: c.hours, w: c.el.offsetWidth, ht: c.el.offsetHeight, x: c.el.offsetLeft, y: c.el.offsetTop }));
  try { localStorage.setItem(CARDS, JSON.stringify(list)); } catch { /* ignore */ }
}

/** Open (or bring forward) the floating history card for reading `k`, near a point. */
export function openHistoryCard(k, at = null, { hours = 6, w = 380, h = 220, pos = null } = {}) {
  if (!CHANNELS[k]) return;
  const have = open.get(k);
  if (have) { have.el.classList.remove("flash"); void have.el.offsetWidth; have.el.classList.add("flash"); document.body.append(have.el); return; }
  const ch = CHANNELS[k];
  const el = document.createElement("div");
  el.className = `hist-float${ch.tint ? ` wt-${ch.tint}` : ""}`;
  el.style.width = `${w}px`; el.style.height = `${h}px`;
  el.innerHTML = `<header><span class="ht">${ch.label} history</span><span class="hh"></span><span class="hsp"></span>
    <select class="hist-sel" title="How much history across the plot"></select><button class="hist-x" title="Close">✕</button></header>
    <div class="hn"></div><canvas class="hist-cv"></canvas>`;
  document.body.append(el);
  const card = { el, hours: Math.min(hours, prefs.keep) };
  open.set(k, card);
  const sel = el.querySelector(".hist-sel");
  const fill = () => {
    sel.innerHTML = WINDOWS.filter((x) => x <= prefs.keep).map((x) => `<option value="${x}" ${x === card.hours ? "selected" : ""}>${winLabel(x)}</option>`).join("");
  };
  fill();
  sel.onchange = () => { card.hours = Number(sel.value); draw(); saveCards(); };
  el.querySelector(".hist-x").onclick = () => { el.remove(); open.delete(k); clearInterval(timer); ro.disconnect(); saveCards(); };
  const header = el.querySelector("header");
  floatResize(el);
  const put = (x, y) => {
    el.style.left = `${Math.max(0, Math.min(window.innerWidth - 60, x))}px`;
    el.style.top = `${Math.max(0, Math.min(window.innerHeight - 30, y))}px`;
  };
  // opened from the data panel: beside the panel, over the chart's edge, not over the panel
  const panel = document.getElementById("side");
  const pr = panel && !panel.hidden ? panel.getBoundingClientRect() : null;
  if (at && pr && at[0] >= pr.left && at[0] <= pr.right) {
    const x = pr.left > window.innerWidth / 2 ? pr.left - w - 8 : pr.right + 8;
    put(x, Math.min(window.innerHeight - h - 4, Math.max(4, at[1] - 20)));
  } else if (at) put(Math.min(window.innerWidth - w - 4, at[0] + 12), Math.min(window.innerHeight - h - 4, at[1] - 20));
  else if (pos) put(pos[0], pos[1]);
  else put(window.innerWidth / 2 - w / 2, window.innerHeight / 3);
  // drag by the title
  header.classList.add("float-drag");
  header.title = "Drag to move";
  header.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || e.target.closest("button, select")) return;
    e.preventDefault();
    const r = el.getBoundingClientRect(), dx = e.clientX - r.left, dy = e.clientY - r.top;
    header.setPointerCapture(e.pointerId);
    document.body.append(el); // to the front
    const move = (ev) => put(ev.clientX - dx, ev.clientY - dy);
    const up = () => { header.removeEventListener("pointermove", move); header.removeEventListener("pointerup", up); saveCards(); };
    header.addEventListener("pointermove", move);
    header.addEventListener("pointerup", up);
  });
  const draw = () => {
    if (!el.isConnected) return;
    const hd = headline(k);
    el.querySelector(".hh").innerHTML = `<b>${hd.value}</b> ${hd.unit}`;
    const n = el.querySelector(".hn");
    n.textContent = hd.note;
    n.classList.toggle("warn", !!hd.falling);
    drawHistory(el.querySelector("canvas"), k, card.hours);
  };
  const ro = new ResizeObserver(() => { draw(); saveCards(); });
  ro.observe(el);
  const timer = setInterval(draw, 5000);
  card.refill = fill;
  draw();
  saveCards();
}

/** Reopen the cards that were open when the window closed. */
export function restoreHistoryCards() {
  let list = [];
  try { list = JSON.parse(localStorage.getItem(CARDS) || "[]"); } catch { /* none */ }
  for (const c of Array.isArray(list) ? list : []) if (CHANNELS[c.k]) openHistoryCard(c.k, null, { hours: c.h, w: c.w || 380, h: c.ht || 220, pos: Number.isFinite(c.x) ? [c.x, c.y] : null });
}

/** The history settings row in the Instruments drawer. */
export function setupHistorySettings(sel, info, clear) {
  if (!sel) return;
  sel.innerHTML = KEEP_HOURS.map((h) => `<option value="${h}" ${h === prefs.keep ? "selected" : ""}>${h} hours</option>`).join("");
  const say = () => {
    const n = Object.keys(CHANNELS).filter((k) => recorder.has(k)).map((k) => CHANNELS[k].label);
    if (info) info.textContent = n.length ? `Recording: ${n.join(", ")}.` : "Nothing recorded yet: history starts when live data comes in.";
  };
  sel.onchange = () => { setKeepHours(Number(sel.value)); for (const c of open.values()) c.refill?.(); say(); };
  if (clear) clear.onclick = () => { clearHistory(); say(); };
  say();
  setInterval(say, 30_000);
}
