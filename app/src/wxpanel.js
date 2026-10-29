// The pop-up Weather panel: time slider, what to show (every parameter in the loaded GRIBs, each with its sources),
// the low / target / high filter, a cursor readout and presets, and a snapshot.
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";

const KEY = "racenav.wxpanel";
const PRESETS = {
  Fishing: { layer: "sst", overlays: { arrows: true, waveArrows: false, currArrows: true, isobars: false, isotherms: true, breaks: true } },
  Passage: { layer: "wind", overlays: { arrows: true, waveArrows: true, currArrows: true, isobars: true, isotherms: false, breaks: false } },
  Anchorage: { layer: "gust", overlays: { arrows: true, waveArrows: false, currArrows: false, isobars: true, isotherms: false, breaks: false } },
};
const DIRS = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
const compass = (d) => DIRS[Math.round((((d % 360) + 360) % 360) / 22.5) % 16];
const deg3 = (d) => String(Math.round(d) % 360).padStart(3, "0");

/** Readout rows: label, the GRIB parameters that supply it, and how to print the point values. */
const ROWS = [
  ["Wind", ["WindU", "WindSpeed"], (r) => r.tws != null && `${deg3(r.twd)}° ${compass(r.twd)} ${r.tws.toFixed(0)} kn`],
  ["Gusts", ["Gust"], (r) => r.gust != null && `${r.gust.toFixed(0)} kn`],
  ["Waves", ["WaveHeight"], (r) => r.wave != null && `${r.wave.toFixed(1)} m${r.wavedir != null ? ` from ${compass(r.wavedir)}` : ""}`],
  ["Wave period", ["WavePeriod"], (r) => r.period != null && `${r.period.toFixed(0)} s`],
  ["Pressure", ["Mslp"], (r, trend) => r.mslp != null && `${r.mslp.toFixed(0)} hPa${trend != null ? ` · ${trend >= 0 ? "+" : ""}${trend.toFixed(1)} /3 h` : ""}`],
  ["Air temp", ["AirTemp"], (r) => r.temp != null && `${r.temp.toFixed(1)} °C`],
  ["Rain", ["Rain"], (r) => r.rain != null && `${r.rain.toFixed(1)} mm/h`],
  ["Sea temp", ["SeaTemp"], (r) => r.sst != null && `${r.sst.toFixed(1)} °C`],
  ["Sea temp (sat.)", ["SkinTemp"], (r) => r.skin != null && `${r.skin.toFixed(1)} °C`],
  ["Temp at depth", ["SubTemp"], (r) => r.subt != null && `${r.subt.toFixed(1)} °C${r.depth != null ? ` at ${Math.abs(r.depth).toFixed(0)} m` : ""}`],
  ["Salinity", ["Salinity"], (r) => r.sss != null && `${r.sss.toFixed(2)} PSU`],
  ["Salinity at depth", ["SubSalinity"], (r) => r.subs != null && `${r.subs.toFixed(2)} PSU${r.depth != null ? ` at ${Math.abs(r.depth).toFixed(0)} m` : ""}`],
  ["Chlorophyll", ["Chlorophyll"], (r) => r.chl != null && `${(10 ** r.chl).toFixed(2)} mg/m³`],
  ["Sea surface height", ["SeaLevel"], (r) => r.ssh != null && `${r.ssh >= 0 ? "+" : ""}${r.ssh.toFixed(2)} m`],
  ["Current", ["CurrentU"], (r) => r.cur && `${r.cur.kn.toFixed(1)} kn toward ${deg3(r.cur.set)}° ${compass(r.cur.set)}`],
];

export function setupWeatherPanel(map, weather) {
  const $ = (id) => document.getElementById(id);
  const st = { open: false, x: null, y: null, collapsed: false, sec: { show: true, over: true }, presets: {}, ...JSON.parse(localStorage.getItem(KEY) || "{}") };
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch { /* ignore */ } };

  const win = document.createElement("div");
  win.id = "wxp";
  win.className = "float-win";
  win.hidden = true;
  win.innerHTML = `
    <header><b>Weather</b><span class="wx-note" id="wxp-when"></span><span class="grow"></span>
      <button id="wxp-snap" title="Save the chart as a picture">📷</button>
      <button id="wxp-fold" title="Fold down to the time slider"></button><button id="wxp-x" title="Close">✕</button></header>
    <div class="wxp-run" id="wxp-run"></div>
    <div class="wxp-time"><div id="wxp-slider"></div><div id="wxp-legend" class="wx-legend"></div><div id="wxp-filter"></div></div>
    <div class="wxp-body" id="wxp-body">
      <div class="wxp-head">At the cursor</div><div id="wxp-point" class="wxp-point"></div>
      <details data-sec="show"><summary>Show on the chart</summary><div id="wxp-show"></div>
        <div class="wxp-presets"><select id="wxp-preset" title="Presets"></select><button id="wxp-psave" title="Save what is shown as a preset">Save…</button><button id="wxp-pdel" title="Delete the chosen saved preset">Delete</button></div></details>
      <details data-sec="over"><summary>Overlays</summary><div id="wxp-over"></div></details>
    </div>`;
  (($("tgt-win") ?? map.getContainer()).parentElement).append(win);
  weather.mountControls($("wxp-slider"), { bare: true });
  weather.mountLegend($("wxp-legend"));
  weather.mountFilter($("wxp-filter"));

  for (const d of win.querySelectorAll("details")) {
    d.open = !!st.sec[d.dataset.sec];
    d.ontoggle = () => { st.sec[d.dataset.sec] = d.open; save(); };
  }

  function place() {
    if (st.x == null) return;
    const m = map.getContainer().getBoundingClientRect();
    win.style.left = `${Math.max(0, Math.min(m.width - 140, st.x))}px`;
    win.style.top = `${Math.max(0, Math.min(m.height - 40, st.y))}px`;
    win.style.right = "auto";
  }
  function fold() {
    win.classList.toggle("folded", st.collapsed);
    $("wxp-fold").textContent = st.collapsed ? "▾" : "▴";
  }
  function show(on) {
    st.open = on;
    win.hidden = !on;
    $("wx-pop-btn")?.classList.toggle("on", on);
    save();
    if (on) { place(); fold(); refresh(); }
  }
  $("wx-pop-btn").onclick = () => show(win.hidden);
  $("wxp-x").onclick = () => show(false);
  $("wxp-fold").onclick = () => { st.collapsed = !st.collapsed; save(); fold(); };
  win.querySelector("header").addEventListener("pointerdown", (e) => {
    if (e.target.closest("select, button")) return;
    const r = win.getBoundingClientRect(), m = map.getContainer().getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    const move = (ev) => { st.x = ev.clientX - m.left - dx; st.y = ev.clientY - m.top - dy; place(); };
    const up = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); save(); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });

  // ---------- what is loaded: layers and overlays, each with its sources ----------
  const short = (n) => (n.length > 28 ? `${n.slice(0, 27)}…` : n);
  function sourceSelect(paths, current, onPick) {
    const names = new Map(weather.sources().map((s) => [s.path, s.name]));
    const sel = document.createElement("select");
    sel.title = "Which loaded GRIB this comes from";
    for (const p of paths) sel.append(new Option(short(names.get(p) ?? p), p));
    sel.value = current ?? paths[0];
    sel.onchange = () => onPick(sel.value);
    return sel;
  }
  function renderShow() {
    const box = $("wxp-show");
    box.innerHTML = "";
    const info = weather.layerInfo();
    const opts = weather.layerOptions();
    if (!opts.length) { box.innerHTML = `<div class="wx-note">No GRIB loaded. Load one in the Weather drawer.</div>`; return; }
    for (const o of [{ key: "none", label: "No shading", paths: [] }, ...opts]) {
      const row = document.createElement("label");
      row.className = "wxp-row";
      const r = document.createElement("input");
      r.type = "radio"; r.name = "wxp-layer";
      r.checked = o.key === "none" ? info.off : !info.off && info.key === o.key;
      r.onchange = () => weather.pickLayer(o.key, o.path ?? o.paths[0]);
      const name = document.createElement("span");
      name.textContent = o.key === "none" ? o.label : `${o.label} (${o.unit})`;
      row.append(r, name);
      if (o.paths.length > 1) {
        const sel = sourceSelect(o.paths, o.path, (p) => weather.pickLayer(o.key, p));
        sel.onclick = (e) => e.stopPropagation();
        row.append(sel);
      } else if (o.paths.length === 1 && weather.sources().length > 1) {
        const s = document.createElement("span");
        s.className = "wx-note";
        s.textContent = short(weather.sources().find((x) => x.path === o.paths[0])?.name ?? "");
        row.append(s);
      }
      box.append(row);
    }
  }
  function renderOver() {
    const box = $("wxp-over");
    box.innerHTML = "";
    for (const o of weather.overlayOptions()) {
      const row = document.createElement("label");
      row.className = "wxp-row";
      const c = document.createElement("input");
      c.type = "checkbox"; c.checked = o.on;
      c.onchange = () => weather.setOverlay(o.key, c.checked);
      const name = document.createElement("span");
      name.textContent = o.label;
      row.append(c, name);
      if (o.paths.length > 1) row.append(sourceSelect(o.paths, o.path, (p) => weather.setOverlay(o.key, null, p)));
      box.append(row);
    }
  }
  function renderPresets() {
    const sel = $("wxp-preset");
    sel.innerHTML = "";
    sel.append(new Option("Presets…", ""));
    for (const n of Object.keys(PRESETS)) sel.append(new Option(n, `b:${n}`));
    for (const n of Object.keys(st.presets)) sel.append(new Option(`★ ${n}`, `u:${n}`));
  }
  $("wxp-preset").onchange = (e) => {
    const [k, n] = [e.target.value.slice(0, 1), e.target.value.slice(2)];
    const c = k === "b" ? PRESETS[n] : k === "u" ? st.presets[n] : null;
    if (c) weather.applyConfig(c);
  };
  $("wxp-psave").onclick = () => {
    const name = prompt("Name for this preset (layer, sources, overlays and filter)")?.trim();
    if (!name) return;
    st.presets[name] = weather.config();
    save(); renderPresets();
    $("wxp-preset").value = `u:${name}`;
  };
  $("wxp-pdel").onclick = () => {
    const v = $("wxp-preset").value;
    if (!v.startsWith("u:")) return;
    delete st.presets[v.slice(2)];
    save(); renderPresets();
  };

  // ---------- header: time, run and how old the data is ----------
  function renderHead() {
    const cur = weather.current();
    const s = weather.sources().find((x) => x.path === cur.path);
    $("wxp-when").textContent = cur.path ? fmt(cur.t) : "";
    const run = $("wxp-run");
    run.className = "wxp-run";
    if (!s) { run.textContent = "No GRIB loaded"; return; }
    const now = Date.now() / 1000;
    const ageH = s.ref ? (now - s.ref) / 3600 : null;
    const info = weather.layerInfo();
    const over = info.max != null && now > info.max;
    run.textContent = `${s.name}${s.ref ? ` · run ${fmt(s.ref, true)} (${ageH < 48 ? `${Math.round(ageH)} h` : `${Math.round(ageH / 24)} d`} old)` : ""}`;
    if (over) { run.classList.add("stale"); run.textContent += " · the forecast has ended: it is past the last time"; }
    else if (ageH > 24) { run.classList.add("stale"); run.textContent += " · old: download a fresh run"; }
  }

  // ---------- cursor readout ----------
  let lastPoint = null, trend = null, trendKey = "";
  weather.onPoint((row, at) => { lastPoint = row; renderPoint(); if (row) fetchTrend(row, at); });
  async function fetchTrend(row, at) {
    const key = `${at[0].toFixed(3)},${at[1].toFixed(3)},${row.t}`;
    if (key === trendKey || row.mslp == null) return;
    trendKey = key;
    const rows = await invoke("grib_point", { lat: at[1], lon: at[0], t: row.t - 10800 }).catch(() => []);
    const was = (rows || []).find((r) => r.path === weather.current().path)?.mslp;
    trend = was != null ? row.mslp - was : null;
    renderPoint();
  }
  function renderPoint() {
    const box = $("wxp-point");
    if (win.hidden) return;
    const have = weather.loadedParams();
    const rows = ROWS.filter(([, params]) => params.some((p) => have.has(p)));
    if (!rows.length) { box.innerHTML = `<div class="wx-note">No GRIB loaded.</div>`; return; }
    const r = lastPoint;
    // one fixed row per parameter in the loaded GRIBs, so the list never changes size as the cursor moves
    const cells = rows.map(([label, , f]) => `<span>${label}</span><b>${(r && f(r, trend)) || "—"}</b>`);
    let warn = "";
    if (r?.cur && r.tws != null && r.cur.kn >= 1 && r.tws >= 15) {
      const diff = Math.abs((((r.twd - r.cur.set) % 360) + 540) % 360 - 180);
      if (diff < 45) warn = "Wind against current: expect steep, short seas.";
    }
    box.innerHTML = `<div class="wxp-grid">${cells.join("")}</div><div class="wxp-warn">${warn}</div>`;
  }

  // ---------- snapshot ----------
  $("wxp-snap").onclick = () => {
    map.once("render", () => {
      map.getCanvas().toBlob(async (blob) => {
        if (!blob) return;
        const data = Array.from(new Uint8Array(await blob.arrayBuffer()));
        const stamp = new Date(weather.current().t * 1000).toISOString().slice(0, 16).replace(/[-:T]/g, "");
        invoke("weather_save_png", { data, suggest: `weather-${stamp}.png` }).catch(() => {});
      }, "image/png");
    });
    map.triggerRepaint();
  };

  // ---------- keep it current ----------
  let timer = null;
  function refresh() {
    clearTimeout(timer);
    timer = setTimeout(() => {
      if (win.hidden) return;
      renderHead(); renderShow(); renderOver(); renderPoint();
    }, 60);
  }
  weather.onChange(refresh);
  weather.onLoaded(refresh);
  weather.onTime(() => { if (!win.hidden) renderHead(); });
  renderPresets();
  fold();
  show(!!st.open);
  return { show, toggle: () => show(win.hidden) };
}
