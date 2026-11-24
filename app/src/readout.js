// The readout: what the loaded GRIBs say at the cursor, in a small floating panel over the chart
// that the skipper turns on and off (Readout on the toolbar, Q). Optional and temporary: it shows
// only the readings chosen for the session (⚙: from what the loaded files hold, whatever is under
// the cursor), with "Show all" for the rest. It can sit where it is put (drag its title) or ride
// beside the cursor. Pinned (Shift+Q, or right-click › Pin the readout here) it stays on one place
// as the chart time moves. A reading can also be compared across every loaded GRIB.
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { floatDrag } from "./floatdrag.js";

const DIRS = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
const compass = (d) => DIRS[Math.round((((d % 360) + 360) % 360) / 22.5) % 16];
const deg3 = (d) => String(Math.round(d) % 360).padStart(3, "0");
const at = (r) => (r.depth != null ? ` at ${Math.abs(r.depth).toFixed(0)} m` : "");

/** Every reading the readout knows: { key, label, params (any of them in a loaded file makes it
 *  available), value(row, trend) → text or false }. */
export const READINGS = [
  { key: "tws", label: "Wind", params: ["WindU", "WindSpeed"], value: (r) => r.tws != null && `${deg3(r.twd)}° ${compass(r.twd)} ${r.tws.toFixed(0)} kn` },
  { key: "gust", label: "Gusts", params: ["Gust"], value: (r) => r.gust != null && `${r.gust.toFixed(0)} kn` },
  { key: "wave", label: "Waves", params: ["WaveHeight"], value: (r) => r.wave != null && `${r.wave.toFixed(1)} m${r.wavedir != null ? ` from ${compass(r.wavedir)}` : ""}` },
  { key: "period", label: "Wave period", params: ["WavePeriod"], value: (r) => r.period != null && `${r.period.toFixed(0)} s` },
  { key: "mslp", label: "Pressure", params: ["Mslp"], value: (r, trend) => r.mslp != null && `${r.mslp.toFixed(0)} hPa${trend != null ? ` · ${trend >= 0 ? "+" : ""}${trend.toFixed(1)} /3 h` : ""}` },
  { key: "temp", label: "Air temp", params: ["AirTemp"], value: (r) => r.temp != null && `${r.temp.toFixed(1)} °C` },
  { key: "rain", label: "Rain", params: ["Rain"], value: (r) => r.rain != null && `${r.rain.toFixed(1)} mm/h` },
  { key: "sst", label: "Sea temp", params: ["SeaTemp"], value: (r) => r.sst != null && `${r.sst.toFixed(1)} °C` },
  { key: "sst_err", label: "Sea temp error", params: ["SeaTempError"], value: (r) => r.sst_err != null && `±${r.sst_err.toFixed(2)} °C` },
  { key: "skin", label: "Sea temp (sat.)", params: ["SkinTemp"], value: (r) => r.skin != null && `${r.skin.toFixed(1)} °C` },
  { key: "subt", label: "Temp at depth", params: ["SubTemp"], value: (r) => r.subt != null && `${r.subt.toFixed(1)} °C${at(r)}` },
  { key: "sss", label: "Salinity", params: ["Salinity"], value: (r) => r.sss != null && `${r.sss.toFixed(2)} PSU` },
  { key: "subs", label: "Salinity at depth", params: ["SubSalinity"], value: (r) => r.subs != null && `${r.subs.toFixed(2)} PSU${at(r)}` },
  { key: "cur", label: "Current", params: ["CurrentU"], value: (r) => r.cur && `${r.cur.kn.toFixed(1)} kn toward ${deg3(r.cur.set)}° ${compass(r.cur.set)}` },
  { key: "subcur", label: "Current at depth", params: ["SubCurrentU"], value: (r) => r.subcur != null && `${r.subcur.kn.toFixed(1)} kn toward ${deg3(r.subcur.set)}°${at(r)}` },
  { key: "mld", label: "Mixed layer", params: ["MixedLayer"], value: (r) => r.mld != null && `${Math.round(r.mld)} m` },
  { key: "chl", label: "Chlorophyll", params: ["Chlorophyll"], value: (r) => r.chl != null && `${r.chl.toFixed(2)} mg/m³` },
  { key: "phyc", label: "Phytoplankton", params: ["Phytoplankton"], value: (r) => r.phyc != null && `${r.phyc.toFixed(2)} mmol/m³` },
  { key: "nppv", label: "Primary production", params: ["PrimaryProduction"], value: (r) => r.nppv != null && `${r.nppv.toFixed(1)} mg C/m³/d` },
  { key: "o2", label: "Oxygen", params: ["Oxygen"], value: (r) => r.o2 != null && `${Math.round(r.o2)} mmol/m³` },
  { key: "ph", label: "pH", params: ["Ph"], value: (r) => r.ph != null && r.ph.toFixed(3) },
  { key: "ssh", label: "Sea surface height", params: ["SeaLevel"], value: (r) => r.ssh != null && `${r.ssh >= 0 ? "+" : ""}${r.ssh.toFixed(2)} m` },
  { key: "ice", label: "Sea ice", params: ["SeaIce"], value: (r) => r.ice != null && `${Math.round(r.ice)} %` },
];
/** The readings shown until the skipper chooses: the everyday ones the loaded files have. */
export const DEFAULT_KEYS = ["tws", "gust", "wave", "mslp", "sst", "cur", "subt"];

/** The readings available from the loaded files (`have`: their parameter names). */
export const available = (have) => READINGS.filter((x) => x.params.some((p) => have.has(p)));

/** The readings to show: the chosen ones the files have, else the defaults they have, else the
 *  first four they have; with `all`, every one they have (chosen first). */
export function chosenReadings(have, keys, all = false) {
  const av = available(have);
  const pick = (ks) => ks.map((k) => av.find((x) => x.key === k)).filter(Boolean);
  let out = keys?.length ? pick(keys) : [];
  if (!out.length) out = pick(DEFAULT_KEYS);
  if (!out.length) out = av.slice(0, 4);
  return all ? [...out, ...av.filter((x) => !out.includes(x))] : out;
}

/** Where the cursor balloon goes: beside the cursor (down and right), flipped to the other side
 *  near the right or bottom edge. Sizes in pixels; returns the box's top-left. */
export function balloonPlace(cx, cy, w, h, W, H, gap = 18) {
  const x = cx + gap + w > W ? Math.max(0, cx - gap - w) : cx + gap;
  const y = cy + gap + h > H ? Math.max(0, cy - gap - h) : cy + gap;
  return [x, y];
}

const KEY = "racenav.readout";
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export function setupReadout(map, { weather }) {
  const st = { on: false, keys: null, compare: [], follow: false, all: false };
  try { Object.assign(st, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* defaults */ }
  if (!Array.isArray(st.compare)) st.compare = [];
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch { /* ignore */ } };

  const el = document.createElement("div");
  el.id = "readout";
  el.className = "readout";
  el.hidden = true;
  el.innerHTML = `<div class="ro-head"><b class="ro-title">At the cursor</b>
      <button type="button" class="ro-unpin" hidden title="Follow the cursor again (Shift+Q)">Unpin</button>
      <button type="button" class="ro-follow" title="Ride beside the cursor, or stay where it is put">Follow</button>
      <button type="button" class="ro-cfg" title="Choose the readings" aria-haspopup="true">⚙</button>
      <button type="button" class="ro-close" title="Close the readout (Q)">✕</button></div>
    <div class="ro-body"></div>
    <div class="ro-foot"></div>
    <div class="ro-menu" hidden></div>`;
  (document.getElementById("map") || document.body).append(el);
  const head = el.querySelector(".ro-head"), body = el.querySelector(".ro-body"), foot = el.querySelector(".ro-foot"), menu = el.querySelector(".ro-menu");
  const drag = floatDrag(el, "readout", head);
  const btn = document.getElementById("readout-btn");

  // the pinned place, its marker, and the values there
  map.addSource("ro-pin", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "ro-pin", type: "circle", source: "ro-pin",
    paint: { "circle-radius": 6, "circle-color": "#ffd400", "circle-stroke-color": "#1d2a33", "circle-stroke-width": 2 } });
  let pinned = null, pinQ = null, pinBusy = 0;
  let live = null, liveAt = null, liveRows = [];
  let trend = null, trendKey = "";

  function show(on) {
    st.on = on; save();
    el.hidden = !on;
    btn?.classList.toggle("on", on);
    btn?.setAttribute("aria-pressed", String(on));
    if (!on) { menu.hidden = true; unpin(); }
    applyFollow();
    render();
  }
  function applyFollow() {
    el.classList.toggle("follow", st.follow && !pinned);
    el.querySelector(".ro-follow").classList.toggle("on", st.follow);
    el.querySelector(".ro-follow").title = st.follow ? "Riding beside the cursor: click to leave it where it is put" : "Ride beside the cursor";
    if (!(st.follow && !pinned)) { el.style.transform = ""; drag.place(); }
  }
  function pinAt(lngLat) {
    pinned = lngLat;
    if (!st.on) show(true);
    map.getSource("ro-pin").setData({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: pinned } });
    el.querySelector(".ro-unpin").hidden = false;
    applyFollow();
    fetchPinned();
  }
  function unpin() {
    pinned = null; pinQ = null;
    map.getSource("ro-pin")?.setData({ type: "FeatureCollection", features: [] });
    el.querySelector(".ro-unpin").hidden = true;
    applyFollow();
    render();
  }
  async function fetchPinned() {
    if (!pinned) return;
    const n = ++pinBusy;
    const q = await weather.queryPoint(pinned).catch(() => null);
    if (n !== pinBusy || !pinned) return;
    pinQ = q;
    fetchTrend(q?.shown, pinned);
    render();
  }

  // the pressure's change over the last 3 hours, at the place shown
  async function fetchTrend(row, place) {
    if (!row || !place || row.mslp == null) { trend = null; trendKey = ""; return; }
    const source = row.src?.mslp ?? row.name;
    const key = `${place[0].toFixed(3)},${place[1].toFixed(3)},${row.t},${source}`;
    if (key === trendKey) return;
    trendKey = key; trend = null;
    const rows = await invoke("grib_point", { lat: place[1], lon: place[0], t: row.t - 10800 }).catch(() => []);
    if (key !== trendKey) return;
    const was = (rows || []).find((r) => r.name === source)?.mslp;
    trend = was != null ? row.mslp - was : null;
    render();
  }

  weather.onPoint((row, place, rows) => {
    live = row; liveAt = place; liveRows = rows ?? [];
    if (!st.on || pinned) return;
    fetchTrend(row, place);
    render();
  });
  weather.onTime(() => { if (pinned) fetchPinned(); });
  weather.onLoaded(() => { if (pinned) fetchPinned(); else render(); });
  weather.onChange(() => { if (pinned) fetchPinned(); });

  // ---------- drawing ----------
  let frame = 0;
  function render() {
    if (!st.on || frame) return;
    frame = requestAnimationFrame(() => { frame = 0; draw(); });
  }
  function draw() {
    const have = weather.loadedParams();
    const row = pinned ? pinQ?.shown : live;
    const rows = pinned ? (pinQ?.rows ?? []) : liveRows;
    const place = pinned ?? liveAt;
    el.querySelector(".ro-title").textContent = pinned ? "Pinned" : "At the cursor";
    if (!weather.sources().length) {
      body.innerHTML = `<div class="ro-note">No GRIB loaded. Get data in the Environment view.</div>`;
      foot.textContent = "";
      return;
    }
    const list = chosenReadings(have, st.keys, st.all);
    const notes = [];
    const mark = (k) => {
      const past = row?.past?.[k];
      const source = row?.src?.[k] ?? (past != null ? row.name : null);
      const name = source && `${source}${past != null ? ` · past analysis ${fmt(past)}` : ""}`;
      if (!name) return "";
      let i = notes.indexOf(name);
      if (i < 0) { notes.push(name); i = notes.length - 1; }
      return `<sup title="From ${esc(name)}">${String.fromCharCode(97 + i)}</sup>`;
    };
    let html = "";
    for (const x of list) {
      const v = row && x.value(row, trend);
      html += `<span class="ro-k">${esc(x.label)}</span><span class="ro-v">${v ? `${esc(v)}${mark(x.key)}` : "—"}</span>`;
      // compared across the loaded GRIBs: one line for each file that has it
      if (st.compare.includes(x.key)) {
        for (const r of rows) {
          const w = x.value(r, null);
          if (!w) continue;
          html += `<span class="ro-k ro-sub" title="${esc(r.fileName ?? r.name)}">${esc(r.fileName ?? r.name)}</span><span class="ro-v ro-sub">${esc(w)}</span>`;
        }
      }
    }
    const more = available(have).length - chosenReadings(have, st.keys).length;
    body.innerHTML = `<div class="ro-grid">${html}</div>` +
      (more > 0 || st.all ? `<button type="button" class="ro-all">${st.all ? "Show fewer" : `Show all (${more} more)`}</button>` : "") +
      (notes.length ? `<div class="ro-srcs">${notes.map((n, i) => `<span><sup>${String.fromCharCode(97 + i)}</sup> ${esc(n)}</span>`).join("")}</div>` : "");
    const pos = place ? `${Math.abs(place[1]).toFixed(3)}°${place[1] < 0 ? "S" : "N"} ${Math.abs(place[0]).toFixed(3)}°${place[0] < 0 ? "W" : "E"}` : "move the cursor over the chart";
    foot.textContent = `${pos}${row ? ` · ${fmt(row.t)} · ${row.name}` : ""}`;
    body.querySelector(".ro-all")?.addEventListener("click", () => { st.all = !st.all; save(); render(); });
    if (st.follow && !pinned) placeBalloon();
  }

  // ---------- riding beside the cursor ----------
  let cursorPx = null;
  map.on("mousemove", (e) => {
    cursorPx = [e.point.x, e.point.y];
    el.classList.remove("away");
    if (st.on && st.follow && !pinned) placeBalloon();
  });
  // off the chart, the balloon goes (it comes back with the cursor)
  map.getCanvasContainer().addEventListener("mouseleave", () => { if (st.follow && !pinned) el.classList.add("away"); });
  new ResizeObserver(() => { if (st.on && st.follow && !pinned) placeBalloon(); }).observe(el);
  function placeBalloon() {
    if (!cursorPx) return;
    const c = map.getContainer();
    const [x, y] = balloonPlace(cursorPx[0], cursorPx[1], el.offsetWidth, el.offsetHeight, c.clientWidth, c.clientHeight);
    el.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }

  // ---------- choosing the readings ----------
  function openMenu() {
    const have = weather.loadedParams();
    const av = available(have);
    const shown = new Set(chosenReadings(have, st.keys).map((x) => x.key));
    if (!av.length) { menu.innerHTML = `<div class="ro-note">Load a GRIB to choose its readings.</div>`; menu.hidden = false; return; }
    menu.innerHTML = `<div class="ro-menu-h">Readings for this session</div>` +
      av.map((x) => `<label class="ro-opt"><input type="checkbox" data-k="${x.key}"${shown.has(x.key) ? " checked" : ""}> ${esc(x.label)}` +
        `<span class="ro-cmp" title="Show it from each loaded GRIB as well"><input type="checkbox" data-c="${x.key}"${st.compare.includes(x.key) ? " checked" : ""}> each GRIB</span></label>`).join("") +
      `<div class="ro-menu-foot"><button type="button" data-act="defaults">Defaults</button><button type="button" data-act="done">Done</button></div>`;
    menu.hidden = false;
  }
  menu.addEventListener("change", (e) => {
    const k = e.target.dataset.k, c = e.target.dataset.c;
    if (k) {
      const keys = [...menu.querySelectorAll("input[data-k]")].filter((i) => i.checked).map((i) => i.dataset.k);
      st.keys = keys;
    }
    if (c) st.compare = e.target.checked ? [...new Set([...st.compare, c])] : st.compare.filter((x) => x !== c);
    save(); render();
  });
  menu.addEventListener("click", (e) => {
    const act = e.target.dataset.act;
    if (act === "defaults") { st.keys = null; st.compare = []; save(); openMenu(); render(); }
    if (act === "done") menu.hidden = true;
  });
  el.querySelector(".ro-cfg").onclick = () => { if (menu.hidden) openMenu(); else menu.hidden = true; };
  el.querySelector(".ro-close").onclick = () => show(false);
  el.querySelector(".ro-unpin").onclick = () => unpin();
  el.querySelector(".ro-follow").onclick = () => { st.follow = !st.follow; save(); applyFollow(); render(); };
  if (btn) btn.onclick = () => show(!st.on);
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !menu.hidden) { menu.hidden = true; e.stopPropagation(); } }, true);

  show(!!st.on);
  return {
    toggle: () => show(!st.on),
    shown: () => st.on,
    /** Ride beside the cursor, or stay where it is put. */
    toggleFollow: () => { st.follow = !st.follow; save(); applyFollow(); render(); },
    following: () => st.follow,
    /** Pin the readout at a place ([lon, lat]), or at the cursor when none is given; again: unpin. */
    pinAt: (lngLat) => (lngLat ? pinAt(lngLat) : pinned ? unpin() : liveAt && pinAt(liveAt)),
    unpin,
  };
}
