// The cockpit window: a second window (for a screen at the helm) with large,
// touch-friendly readings. The chart window sends it the same numbers its
// instrument panels show, once a second, plus the active alarms.
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { setWindowDark, installFullscreenKey } from "./winchrome.js";
import { ITEMS } from "./instruments.js";

const KEY = "racenav.cockpit";
const DEFAULTS = { cols: 3, tiles: ["SOG", "COG", "BSP", "WDIAL", "COMPASS", "DPT", "TWS", "TWA", "TWD", "DTW", "BTW", "TTG"], pal: "day" };
const PALS = ["day", "dusk", "night-red", "night-green"];
const PAL_NAMES = { day: "Day", dusk: "Dusk", "night-red": "Night red", "night-green": "Night green" };
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

let s = { ...DEFAULTS };
try { Object.assign(s, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ } };
let ctx = { v: {}, now: Date.now(), age: Infinity, safetyDepth: 3 };
let lastMsg = 0;

/** Column widths and row heights as fractions (dragged at the handles). */
function sizes() {
  const rows = Math.max(1, Math.ceil(s.tiles.length / s.cols));
  const fit = (a, n) => { a = Array.isArray(a) ? a.slice(0, n) : []; while (a.length < n) a.push(1); return a; };
  s.colW = fit(s.colW, s.cols);
  s.rowH = fit(s.rowH, rows);
  return rows;
}
function applySizes(handles = true) {
  const g = $("ck-grid");
  g.style.gridTemplateColumns = s.colW.map((f) => `minmax(0, ${f}fr)`).join(" ");
  g.style.gridTemplateRows = s.rowH.map((f) => `minmax(0, ${f}fr)`).join(" ");
  if (handles) requestAnimationFrame(placeHandles);
}
/** Drag handles on the boundaries between columns and between rows. */
function placeHandles() {
  const g = $("ck-grid"), box = $("ck-handles");
  box.innerHTML = "";
  const tiles = [...g.children];
  if (!tiles.length) return;
  const gr = g.getBoundingClientRect();
  const first = (i) => tiles[i]?.getBoundingClientRect();
  for (let c = 0; c < s.cols - 1; c++) {
    const a = first(c), b = first(c + 1);
    if (!a || !b) continue;
    const h = document.createElement("div");
    h.className = "ck-h col";
    h.style.left = `${(a.right + b.left) / 2 - gr.left - 6}px`;
    h.title = "Drag to widen or narrow these columns";
    drag(h, "x", c, gr.width);
    box.append(h);
  }
  for (let r = 0; r < s.rowH.length - 1; r++) {
    const a = first(r * s.cols), b = first((r + 1) * s.cols);
    if (!a || !b) continue;
    const h = document.createElement("div");
    h.className = "ck-h row";
    h.style.top = `${(a.bottom + b.top) / 2 - gr.top - 6}px`;
    h.title = "Drag to make these rows taller or shallower";
    drag(h, "y", r, gr.height);
    box.append(h);
  }
}
function drag(h, axis, i, total) {
  h.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    h.setPointerCapture(e.pointerId);
    const arr = axis === "x" ? s.colW : s.rowH;
    const sum = arr.reduce((a, b) => a + b, 0);
    const start = axis === "x" ? e.clientX : e.clientY;
    const a0 = arr[i], b0 = arr[i + 1];
    const move = (ev) => {
      const d = (((axis === "x" ? ev.clientX : ev.clientY) - start) / total) * sum;
      const min = 0.15 * sum / arr.length;
      const a = Math.max(min, Math.min(a0 + b0 - min, a0 + d));
      arr[i] = a; arr[i + 1] = a0 + b0 - a;
      applySizes(false); // keep this handle while dragging; it follows the pointer
      if (axis === "x") h.style.left = `${parseFloat(h.dataset.p0) + ((ev.clientX - start))}px`;
      else h.style.top = `${parseFloat(h.dataset.p0) + ((ev.clientY - start))}px`;
    };
    const up = () => { h.removeEventListener("pointermove", move); h.removeEventListener("pointerup", up); save(); placeHandles(); };
    h.dataset.p0 = axis === "x" ? h.style.left : h.style.top;
    h.addEventListener("pointermove", move);
    h.addEventListener("pointerup", up);
  });
}
window.addEventListener("resize", () => requestAnimationFrame(placeHandles));

function build() {
  const g = $("ck-grid");
  sizes();
  g.innerHTML = "";
  s.tiles.forEach((k, i) => {
    const it = ITEMS[k];
    const t = document.createElement("div");
    t.className = `tile ck${it?.graphic ? " graphic" : ""}${it?.wide ? " wide" : ""}${it?.highway ? " highway-card" : ""}`;
    t.dataset.i = i;
    t.innerHTML = `<div class="lbl"><span>${esc(it ? it.label : k)}</span><span class="sd"></span></div><div class="v">—</div>`;
    t.onclick = () => menu(i, t.getBoundingClientRect());
    g.append(t);
  });
  $("ck-cols").textContent = `Columns: ${s.cols}`;
  applySizes();
  render();
}

// man overboard: bearing, range and time since, big, while a MOB is set in the chart window
const MOB_TILES = ["MOBBRG", "MOBRNG", "MOBAGO", "MOBPOS"];
function buildMob() {
  const m = $("ck-mob");
  m.innerHTML = "";
  MOB_TILES.forEach((k, i) => {
    const t = document.createElement("div");
    t.className = `tile ck alarm${ITEMS[k].wide ? " wide" : ""}`;
    t.dataset.k = k;
    t.innerHTML = `<div class="lbl"><span>${esc(ITEMS[k].label)}</span><span class="sd"></span></div><div class="v">—</div>`;
    m.append(t);
  });
}

function render() {
  ctx.now = Date.now();
  ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000;
  const mob = $("ck-mob");
  if (mob.hidden !== !ctx.mob) { mob.hidden = !ctx.mob; requestAnimationFrame(placeHandles); }
  $("ck-mob-btn").classList.toggle("on", !!ctx.mob);
  const tiles = [...$("ck-grid").children].map((t) => [t, ITEMS[s.tiles[Number(t.dataset.i)]]]);
  if (ctx.mob) tiles.push(...[...mob.children].map((t) => [t, ITEMS[t.dataset.k]]));
  const highway = $("ck-highway");
  highway.hidden = false;
  tiles.push([highway, ITEMS.HIGHWAY]);
  for (const [t, it] of tiles) {
    if (!it) continue;
    let val = null;
    try { val = it.get(ctx); } catch { val = null; }
    const has = val !== null && val !== undefined && !(typeof val === "number" && !Number.isFinite(val));
    const stale = Boolean(it.live) && ctx.age > 10;
    const state = has && !stale && it.state ? it.state(val, ctx) : "";
    const v = t.querySelector(".v");
    if (it.render) it.render(v, val, ctx);
    else {
      const html = has ? it.fmt(val) + (it.unit ? `<span class="u">${it.unit}</span>` : "") : "—";
      if (v.innerHTML !== html) v.innerHTML = html;
    }
    const side = has && it.side ? it.side(val) : "";
    const sd = t.querySelector(".sd");
    if (sd) {
      sd.className = `sd ${side}`;
      sd.textContent = side === "port" ? "PORT" : side === "stbd" ? "STBD" : "";
    }
    t.classList.toggle("stale", stale);
    t.classList.toggle("warn", state === "warn");
    t.classList.toggle("alarm", state === "alarm");
  }
  const quiet = Date.now() - lastMsg > 5000;
  $("ck-state").textContent = quiet ? "Waiting for the chart window…" : ctx.age > 10 ? "No live data" : "Live";
}
setInterval(render, 250);

function menu(i, rect) {
  const m = $("ck-menu");
  const groups = {};
  for (const [k, it] of Object.entries(ITEMS)) (groups[it.group] ??= []).push([k, it]);
  let html = "";
  for (const [gname, items] of Object.entries(groups)) {
    html += `<div class="grp">${esc(gname)}</div><div class="opts">${items.map(([k, it]) => `<button data-k="${k}" class="${k === s.tiles[i] ? "on" : ""}">${esc(it.label)}${it.unit ? ` <small>${esc(it.unit)}</small>` : ""}</button>`).join("")}</div>`;
  }
  html += `<div class="opts"><button data-act="left">◀ Move</button><button data-act="right">Move ▶</button><button data-act="del" class="bad">Remove</button><button data-act="close">Close</button></div>`;
  m.innerHTML = html;
  m.hidden = false;
  m.onclick = (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    if (b.dataset.k) s.tiles[i] = b.dataset.k;
    else if (b.dataset.act === "del") s.tiles.splice(i, 1);
    else if (b.dataset.act === "left" && i > 0) [s.tiles[i - 1], s.tiles[i]] = [s.tiles[i], s.tiles[i - 1]];
    else if (b.dataset.act === "right" && i < s.tiles.length - 1) [s.tiles[i + 1], s.tiles[i]] = [s.tiles[i], s.tiles[i + 1]];
    m.hidden = true;
    save();
    build();
  };
}

function pal() {
  document.documentElement.dataset.palette = s.pal;
  document.documentElement.style.setProperty("--pal-dim", s.pal === "day" ? "1" : s.pal === "dusk" ? "0.75" : "0.6");
  $("ck-pal").textContent = PAL_NAMES[s.pal];
  setWindowDark(s.pal !== "day");
}
$("ck-pal").onclick = () => { s.pal = PALS[(PALS.indexOf(s.pal) + 1) % PALS.length]; save(); pal(); };
$("ck-cols").onclick = () => { s.cols = (s.cols % 5) + 1; s.colW = null; s.rowH = null; save(); build(); };
$("ck-even").onclick = () => { s.colW = null; s.rowH = null; save(); build(); };
$("ck-add").onclick = () => { s.tiles.push("SOG"); save(); build(); menu(s.tiles.length - 1); };

// MOB at the helm: the chart window marks it (as its own MOB button does) and sends it back
$("ck-mob-btn").onclick = () => { emit("cockpit-mob", {}).catch(() => {}); };
$("ck-full").onclick = async () => { try { const w = getCurrentWindow(); await w.setFullscreen(!(await w.isFullscreen())); } catch { /* not in Tauri */ } };

// alarms from the chart window: shown large; a tone while any is not silenced there
let tone = null;
function beep() {
  try {
    tone ||= new AudioContext();
    const o = tone.createOscillator(), g = tone.createGain();
    o.type = "square"; o.frequency.value = 880; g.gain.value = 0.08;
    o.connect(g).connect(tone.destination); o.start(); o.stop(tone.currentTime + 0.25);
  } catch { /* no audio */ }
}
let loud = false;
setInterval(() => { if (loud) beep(); }, 2000);

listen("cockpit", ({ payload }) => {
  lastMsg = Date.now();
  ctx = { ...ctx, ...payload };
  const a = (payload.alarms || []).filter((alarm) => !alarm.acked);
  const box = $("ck-alarms");
  box.hidden = !a.length;
  box.innerHTML = a.map((x) => `<div class="alarm ${x.level}${x.acked ? " acked" : ""}"><b>${esc(x.text)}</b>${x.detail ? `<small>${esc(x.detail)}</small>` : ""}</div>`).join("");
  loud = a.some((x) => !x.acked && x.level === "alarm");
});

installFullscreenKey();
pal();
buildMob();
build();
