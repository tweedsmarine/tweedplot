// The cockpit window: a second window (for a screen at the helm) with large,
// touch-friendly readings. The chart window sends it the same numbers its
// instrument panels show, once a second, plus the active alarms.
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
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

function build() {
  const g = $("ck-grid");
  g.style.gridTemplateColumns = `repeat(${s.cols}, 1fr)`;
  g.innerHTML = "";
  s.tiles.forEach((k, i) => {
    const it = ITEMS[k];
    const t = document.createElement("div");
    t.className = `tile ck${it?.graphic ? " graphic" : ""}${it?.wide ? " wide" : ""}`;
    t.dataset.i = i;
    t.innerHTML = `<div class="lbl"><span>${esc(it ? it.label : k)}</span><span class="sd"></span></div><div class="v">—</div>`;
    t.onclick = () => menu(i, t.getBoundingClientRect());
    g.append(t);
  });
  $("ck-cols").textContent = `Columns: ${s.cols}`;
  render();
}

function render() {
  ctx.now = Date.now();
  ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000;
  for (const t of $("ck-grid").children) {
    const it = ITEMS[s.tiles[Number(t.dataset.i)]];
    if (!it) continue;
    let val = null;
    try { val = it.get(ctx); } catch { val = null; }
    const has = val !== null && val !== undefined && !(typeof val === "number" && !Number.isFinite(val));
    const stale = Boolean(it.live) && ctx.age > 10;
    const state = has && !stale && it.state ? it.state(val, ctx) : "";
    const html = has ? it.fmt(val) + (it.unit ? `<span class="u">${it.unit}</span>` : "") : "—";
    const v = t.querySelector(".v");
    if (v.innerHTML !== html) v.innerHTML = html;
    const side = has && it.side ? it.side(val) : "";
    const sd = t.querySelector(".sd");
    sd.className = `sd ${side}`;
    sd.textContent = side === "port" ? "PORT" : side === "stbd" ? "STBD" : "";
    t.classList.toggle("stale", stale);
    t.classList.toggle("warn", state === "warn");
    t.classList.toggle("alarm", state === "alarm");
  }
  const quiet = Date.now() - lastMsg > 5000;
  $("ck-state").textContent = quiet ? "Waiting for the chart window…" : ctx.age > 10 ? "No live data" : "Live";
}
setInterval(render, 1000);

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
}
$("ck-pal").onclick = () => { s.pal = PALS[(PALS.indexOf(s.pal) + 1) % PALS.length]; save(); pal(); };
$("ck-cols").onclick = () => { s.cols = (s.cols % 5) + 1; save(); build(); };
$("ck-add").onclick = () => { s.tiles.push("SOG"); save(); build(); menu(s.tiles.length - 1); };
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
  const a = payload.alarms || [];
  const box = $("ck-alarms");
  box.hidden = !a.length;
  box.innerHTML = a.map((x) => `<div class="alarm ${x.level}${x.acked ? " acked" : ""}"><b>${esc(x.text)}</b>${x.detail ? `<small>${esc(x.detail)}</small>` : ""}</div>`).join("");
  loud = a.some((x) => !x.acked && x.level === "alarm");
});

pal();
build();
