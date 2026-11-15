// The pop-up Weather panel: time slider, what to show (every parameter in the loaded GRIBs, each with its sources),
// the low / target / high filter, a cursor readout and presets, and a snapshot.
import { invoke } from "@tauri-apps/api/core";
import { fmt, weatherSelectionSummary } from "./weather.js";
import * as activity from "./activity.js";
import { ask } from "./ask.js";

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
const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const ROWS = [
  ["Wind", ["WindU", "WindSpeed"], (r) => r.tws != null && `${deg3(r.twd)}° ${compass(r.twd)} ${r.tws.toFixed(0)} kn`, "tws"],
  ["Gusts", ["Gust"], (r) => r.gust != null && `${r.gust.toFixed(0)} kn`, "gust"],
  ["Waves", ["WaveHeight"], (r) => r.wave != null && `${r.wave.toFixed(1)} m${r.wavedir != null ? ` from ${compass(r.wavedir)}` : ""}`, "wave"],
  ["Wave period", ["WavePeriod"], (r) => r.period != null && `${r.period.toFixed(0)} s`, "period"],
  ["Pressure", ["Mslp"], (r, trend) => r.mslp != null && `${r.mslp.toFixed(0)} hPa${trend != null ? ` · ${trend >= 0 ? "+" : ""}${trend.toFixed(1)} /3 h` : ""}`, "mslp"],
  ["Air temp", ["AirTemp"], (r) => r.temp != null && `${r.temp.toFixed(1)} °C`, "temp"],
  ["Rain", ["Rain"], (r) => r.rain != null && `${r.rain.toFixed(1)} mm/h`, "rain"],
  ["Sea temp", ["SeaTemp"], (r) => r.sst != null && `${r.sst.toFixed(1)} °C`, "sst"],
  ["Sea temp (sat.)", ["SkinTemp"], (r) => r.skin != null && `${r.skin.toFixed(1)} °C`, "skin"],
  ["Temp at depth", ["SubTemp"], (r) => r.subt != null && `${r.subt.toFixed(1)} °C${r.depth != null ? ` at ${Math.abs(r.depth).toFixed(0)} m` : ""}`, "subt"],
  ["Salinity", ["Salinity"], (r) => r.sss != null && `${r.sss.toFixed(2)} PSU`, "sss"],
  ["Salinity at depth", ["SubSalinity"], (r) => r.subs != null && `${r.subs.toFixed(2)} PSU${r.depth != null ? ` at ${Math.abs(r.depth).toFixed(0)} m` : ""}`, "subs"],
  ["Chlorophyll", ["Chlorophyll"], (r) => r.chl != null && `${r.chl.toFixed(2)} mg/m³`, "chl"],
  ["Sea surface height", ["SeaLevel"], (r) => r.ssh != null && `${r.ssh >= 0 ? "+" : ""}${r.ssh.toFixed(2)} m`, "ssh"],
  ["Current", ["CurrentU"], (r) => r.cur && `${r.cur.kn.toFixed(1)} kn toward ${deg3(r.cur.set)}° ${compass(r.cur.set)}`, "cur"],
];

export function setupWeatherPanel(map, weather, sidePanel, ui) {
  const $ = (id) => document.getElementById(id);
  const st = { open: false, x: null, y: null, collapsed: false, sec: { show: true, over: true }, presets: {}, envStart: null };
  try { Object.assign(st, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable or damaged: defaults */ }
  if (!st.sec || typeof st.sec !== "object") st.sec = { show: true, over: true };
  if (!st.presets || typeof st.presets !== "object") st.presets = {};
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch { /* ignore */ } };

  const win = $("wxp");
  win.innerHTML = `
    <div class="env-start">
      <div class="wx-note">Last starting point selected; adjust any layer or overlay afterwards.</div>
      <div class="env-start-buttons" role="group" aria-label="Environment starting points">
        <button type="button" data-env-preset="weather" aria-pressed="false">Weather</button>
        <button type="button" data-env-preset="ocean" aria-pressed="false">Ocean</button>
        <button type="button" data-env-preset="seabed" aria-pressed="false">Seabed</button>
      </div>
      <div id="env-start-status" class="wx-note" role="status"></div>
      <details class="env-seabed"><summary>Seabed layers</summary>
        <label><input type="checkbox" data-seabed="sd-show"> Survey depths (LINZ)</label>
        <label><input type="checkbox" data-seabed="sd-outlines"> Survey outlines</label>
        <label><input type="checkbox" data-seabed="pb-show"> Own soundings (not for navigation)</label>
        <label><input type="checkbox" data-seabed="data-hr-show"> High-resolution depths (shared)</label>
        <label><input type="checkbox" data-seabed="data-nz-show"> NIWA bathymetry (shared; not for navigation)</label>
        <label><input type="checkbox" data-seabed="lv-etopo"> Detailed depths (shared; not for navigation)</label>
        <label><input type="checkbox" data-seabed="lv-wdepth"> World depths (shared; not for navigation)</label>
        <div class="wx-note">Manage seabed data in Chart → Get data. Shared layers also affect other views.</div>
      </details>
    </div>
    <button id="wxp-data" type="button">Download or manage GRIBs…</button>
    <div class="wxp-time"><div id="wxp-legend" class="wx-legend"></div><div id="wxp-filter"></div></div>
    <div class="wxp-body" id="wxp-body">
      <details data-sec="show"><summary>Show on the chart</summary><div id="wxp-show"></div>
        <div class="wxp-presets"><select id="wxp-preset" title="Presets"></select><button id="wxp-psave" title="Save what is shown as a preset">Save…</button><button id="wxp-pdel" title="Delete the chosen saved preset">Delete</button></div></details>
      <details data-sec="over"><summary>Overlays</summary><div id="wxp-over"></div><div id="wxp-isobar-controls"></div></details>
      <button id="wxp-visible">Hide weather on chart</button>
      <button id="wxp-snap" title="Save the chart as a picture" aria-label="Save the chart as a picture"><svg viewBox="0 0 24 24" class="ic"><path d="M4 8h3l2-2.5h6L17 8h3v11H4z"/><circle cx="12" cy="13" r="3.5"/></svg> Save picture</button>
    </div>`;
  weather.mountLegend($("wxp-legend"));
  weather.mountFilter($("wxp-filter"));
  $("wxp-isobar-controls").append($("wx-isobar-controls"));

  for (const d of win.querySelectorAll("details[data-sec]")) {
    d.open = !!st.sec[d.dataset.sec];
    d.ontoggle = () => { st.sec[d.dataset.sec] = d.open; save(); };
  }

  $("wx-pop-btn").title = "Open weather controls in Environment";
  $("wx-pop-btn").onclick = () => sidePanel.openWeather();
  $("wx-open-controls").onclick = () => sidePanel.openWeather();
  $("wxp-data").onclick = () => sidePanel.openEnvironmentData();
  $("wxp-visible").onclick = () => weather.setVisible(!weather.visible());
  const seabed = [...win.querySelectorAll("[data-seabed]")];
  const syncSeabed = () => { for (const input of seabed) input.checked = !!$(input.dataset.seabed)?.checked; };
  for (const input of seabed) {
    input.addEventListener("change", () => {
      const source = $(input.dataset.seabed);
      source.checked = input.checked;
      source.dispatchEvent(new Event("change", { bubbles: true }));
      syncSeabed();
    });
  }
  document.addEventListener("change", (event) => {
    if (seabed.some((input) => input.dataset.seabed === event.target.id)) syncSeabed();
  });
  window.addEventListener("racenav-view", () => queueMicrotask(syncSeabed));
  const startButtons = win.querySelector(".env-start-buttons");
  function syncStartingPoint() {
    for (const button of startButtons.querySelectorAll("[data-env-preset]")) {
      const selected = button.dataset.envPreset === st.envStart;
      button.classList.toggle("on", selected);
      button.setAttribute("aria-pressed", String(selected));
    }
  }
  startButtons.addEventListener("click", (event) => {
    const key = event.target.closest("[data-env-preset]")?.dataset.envPreset;
    if (!key) return;
    const choices = key === "weather" ? ["wind", "gust", "waves"] : key === "ocean" ? ["sst", "skin", "chl", "sss", "current"] : [];
    const options = weather.layerOptions();
    const choice = choices.map((name) => options.find((option) => option.key === name)).find(Boolean);
    weather.setVisible(true);
    if (choice) weather.pickLayer(choice.key, choice.path ?? choice.paths[0]);
    weather.applyConfig({
      layer: choice?.key ?? "none",
      overlays: {
        arrows: key === "weather", waveArrows: key === "weather", currArrows: key === "ocean",
        isobars: key === "weather", isotherms: key === "ocean", breaks: false,
      },
    });
    $("env-start-status").textContent = key === "seabed" ? "Choose the seabed layers below." :
      choice ? `Starting with ${choice.label}.` : `No ${key} GRIB loaded. Use Get data to load one.`;
    win.querySelector(".env-seabed").open = key === "seabed";
    st.envStart = key;
    save();
    syncStartingPoint();
  });
  syncStartingPoint();
  syncSeabed();

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
    if (!opts.length) { box.innerHTML = `<div class="wx-note">No GRIB loaded. Open Get data to load one.</div>`; return; }
    for (const o of [{ key: "none", label: "No shading", paths: [] }, ...opts]) {
      const row = document.createElement("label");
      row.className = "wxp-row";
      const r = document.createElement("input");
      r.type = "radio"; r.name = "wxp-layer";
      r.checked = o.key === "none" ? info.off : !info.off && info.key === o.key;
      r.onchange = () => weather.pickLayer(o.key, o.path ?? o.paths[0]);
      const name = document.createElement("span");
      name.textContent = o.key === "none" ? o.label : `${o.label} (${o.unit})`;
      if (o.help) row.title = o.help;
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
    const isobarControls = $("wx-isobar-controls");
    $("wxp-isobar-controls").append(isobarControls);
    box.innerHTML = "";
    for (const o of weather.overlayOptions()) {
      const row = document.createElement("label");
      row.className = "wxp-row";
      const c = document.createElement("input");
      c.type = "checkbox"; c.checked = o.on;
      c.onchange = () => weather.setOverlay(o.key, c.checked);
      const name = document.createElement("span");
      name.textContent = o.label + (o.on && o.timeStatus?.state === "past" ? ` · past analysis ${fmt(o.timeStatus.time)}` :
        o.on && o.timeStatus?.state === "missing" ? " · no data at chart time" : "");
      row.append(c, name);
      if (o.paths.length > 1) row.append(sourceSelect(o.paths, o.path, (p) => weather.setOverlay(o.key, null, p)));
      box.append(row);
      if (o.key === "isobars") box.append(isobarControls);
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
  $("wxp-psave").onclick = async () => {
    const v = await ask({ title: "Save a preset", note: "The layer, sources, overlays and filter shown now, under a name.", ok: "Save",
      fields: [{ key: "name", label: "Name", placeholder: "e.g. Tuna fronts" }],
      check: (x) => (!x.name.trim() ? "Give it a name." : PRESETS[x.name.trim()] ? "That name is taken by a built-in preset." : "") });
    const name = v?.name.trim();
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
    const info = weather.layerInfo();
    const s = weather.sources().find((x) => x.path === (info.path ?? cur.path));
    const run = $("wxp-run");
    run.className = "wxp-run";
    if (!s) { run.textContent = "No GRIB loaded"; return; }
    const now = Date.now() / 1000;
    const ageH = s.ref ? (now - s.ref) / 3600 : null;
    const [lo, hi] = [info.srcMin ?? info.min, info.srcMax ?? info.max];
    const over = hi != null && now > hi;
    const outside = hi != null && (cur.t > hi || cur.t < lo);
    // satellite products are analyses of the past, not forecast runs
    const analysis = /VIIRS|MUR SST|Himawari/i.test(s.model ?? s.name ?? "");
    const age = (h) => (h < 48 ? `${Math.round(h)} h` : `${Math.round(h / 24)} d`);
    if (analysis) {
      run.textContent = `${s.name} · analysis, ${fmt(lo)} to ${fmt(hi)}`;
      const lastH = (now - hi) / 3600;
      // how late each product normally is when published: VIIRS a couple of days, MUR a day or two, Himawari hours
      const slackH = /VIIRS/i.test(s.model ?? s.name ?? "") ? 96 : /MUR/i.test(s.model ?? s.name ?? "") ? 72 : 12;
      if (info.timeStatus?.state === "past") { run.classList.add("stale"); run.textContent += ` · using past analysis from ${fmt(info.timeStatus.time)} where available`; }
      else if (info.timeStatus?.state === "missing" || outside) { run.classList.add("stale"); run.textContent += " · no data at this time"; }
      else if (lastH > slackH) { run.classList.add("stale"); run.textContent += ` · latest data is ${age(lastH)} old: download a newer analysis`; }
      return;
    }
    run.textContent = `${s.name}${s.ref ? ` · run ${fmt(s.ref, true)} (${age(ageH)} old)` : ""}`;
    if (info.timeStatus?.state === "missing" || outside) { run.classList.add("stale"); run.textContent += ` · no data at this time: it covers ${fmt(lo)} to ${fmt(hi)}`; }
    else if (over) { run.classList.add("stale"); run.textContent += ` · the data ends ${fmt(hi)}, before now`; }
    else if (ageH > 24) { run.classList.add("stale"); run.textContent += " · old: download a fresh run"; }
  }

  // ---------- cursor readout ----------
  let lastPoint = null, trend = null, trendKey = "";
  weather.onPoint((row, at) => { lastPoint = row; if (row && at) fetchTrend(row, at); else { trendKey = ""; trend = null; } renderPoint(); });
  async function fetchTrend(row, at) {
    const source = row.src?.mslp ?? weather.current().name;
    const key = `${at[0].toFixed(3)},${at[1].toFixed(3)},${row.t},${source}`;
    if (key === trendKey || row.mslp == null) return;
    trendKey = key;
    trend = null; // not the last point's trend while this one is fetched
    const rows = await invoke("grib_point", { lat: at[1], lon: at[0], t: row.t - 10800 }).catch(() => []);
    if (key !== trendKey) return; // the cursor has moved on: a newer request will answer
    const was = (rows || []).find((r) => r.name === source)?.mslp;
    trend = was != null ? row.mslp - was : null;
    renderPoint();
  }
  function renderPoint() {
    const box = $("wxp-point");
    const have = weather.loadedParams();
    const rows = ROWS.filter(([, params]) => params.some((p) => have.has(p)));
    if (!rows.length) {
      box.innerHTML = `<div class="wx-note">${weather.sources().length ? "No supported cursor fields in the loaded GRIBs." : "No GRIB loaded."}</div>`;
      return;
    }
    const r = lastPoint;
    // one fixed row per parameter in the loaded GRIBs, so the list never changes size as the cursor moves
    // a value borrowed from another loaded file (the shown one lacks it) is marked a, b … with the file below
    const notes = [];
    const mark = (k) => {
      const past = r?.past?.[k];
      const source = r?.src?.[k] ?? (past != null ? r.name : null);
      const name = source && `${source}${past != null ? ` · past analysis ${fmt(past)}` : ""}`;
      if (!name) return "";
      let i = notes.indexOf(name);
      if (i < 0) { notes.push(name); i = notes.length - 1; }
      return `<sup class="wxp-from" title="From ${esc(name)}">${String.fromCharCode(97 + i)}</sup>`;
    };
    const cells = rows.map(([label, , f, k]) => {
      const v = r && f(r, trend);
      return `<span>${label}</span><b>${v ? `${v}${mark(k)}` : "—"}</b>`;
    });
    const from = notes.length ? `<div class="wxp-srcs">${notes.map((n, i) => `<span><sup>${String.fromCharCode(97 + i)}</sup> from ${esc(n)}</span>`).join("")}</div>` : "";
    let warn = "";
    if (r?.cur && r.tws != null && r.cur.kn >= 1 && r.tws >= 15) {
      const diff = Math.abs((((r.twd - r.cur.set) % 360) + 540) % 360 - 180);
      if (diff < 45) warn = "Wind against current: expect steep, short seas.";
    }
    const context = r ? `<div class="wxp-point-context">Chart time ${esc(fmt(r.t))} · shown GRIB: ${esc(r.name)}</div>` : "";
    box.innerHTML = `${context}<div class="wxp-grid">${cells.join("")}</div>${from}<div class="wxp-warn">${warn}</div>`;
  }

  // ---------- snapshot ----------
  $("wxp-snap").onclick = () => {
    // drawn in the frame just rendered (the WebGL buffer is only readable then), with the
    // credits and "not for navigation" along the bottom, as the licences of the map data ask
    map.once("render", () => {
      const src = map.getCanvas();
      const credit = ($("map-attribution")?.textContent || "Not for navigation.").replace(/\s+/g, " ").trim();
      const px = Math.max(11, Math.round(12 * (window.devicePixelRatio || 1)));
      const strip = Math.round(px * 1.8);
      const out = document.createElement("canvas");
      out.width = src.width;
      out.height = src.height + strip;
      const g = out.getContext("2d");
      g.drawImage(src, 0, 0);
      g.fillStyle = "#fff";
      g.fillRect(0, src.height, out.width, strip);
      g.fillStyle = "#333";
      g.font = `${px}px system-ui, sans-serif`;
      g.textBaseline = "middle";
      const when = weather.current().path ? `Weather ${fmt(weather.current().t)}. ` : "";
      g.fillText(`${when}${credit}`, Math.round(px / 2), src.height + strip / 2, out.width - px);
      out.toBlob(async (blob) => {
        if (!blob) { activity.start("Saving the weather picture").fail("The chart could not be read as a picture."); return; }
        const data = new Uint8Array(await blob.arrayBuffer());
        const stamp = new Date(weather.current().t * 1000).toISOString().slice(0, 16).replace(/[-:T]/g, "");
        try {
          // the picture goes as raw bytes, not as a list of numbers
          const path = await invoke("weather_save_png", data, { headers: { "x-suggest": `weather-${stamp}.png` } });
          if (path) activity.note("Weather picture saved", { details: [path] });
        } catch (e) { activity.start("Saving the weather picture").fail(String(e)); }
      }, "image/png");
    });
    map.triggerRepaint();
  };

  // ---------- keep it current ----------
  let timer = null;
  function refresh() {
    clearTimeout(timer);
    timer = setTimeout(() => {
      renderHead(); renderShow(); renderOver(); renderPoint();
      $("wxp-visible").textContent = weather.visible() ? "Hide weather on chart" : "Show weather on chart";
      $("wxp-visible").setAttribute("aria-pressed", String(weather.visible()));
    }, 60);
  }
  weather.onChange(refresh);
  weather.onLoaded(refresh);
  let lastOverlayTime = null;
  weather.onTime((time) => {
    renderHead();
    if (document.documentElement.dataset.view === "env" && time !== lastOverlayTime) renderOver();
    lastOverlayTime = time;
  });
  function renderChartState() {
    $("side-weather-state").textContent = weatherSelectionSummary({
      visible: weather.visible(), sources: weather.sources(), layerInfo: weather.layerInfo(),
      layerOptions: weather.layerOptions(), overlayOptions: weather.overlayOptions(),
    });
  }
  weather.onChange(renderChartState);
  weather.onLoaded(renderChartState);
  renderChartState();
  renderPresets();
  refresh();
}
