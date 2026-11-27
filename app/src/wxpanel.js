// The pop-up Weather panel: time slider, what to show (every parameter in the loaded GRIBs, each with its sources),
// the low / target / high filter, a cursor readout and presets, and a snapshot.
import { invoke } from "@tauri-apps/api/core";
import { fmt, weatherSelectionSummary } from "./weather.js";
import * as activity from "./activity.js";
import { ask } from "./ask.js";
import { speciesList, speciesConfig as spConfig, speciesNote, editSpecies } from "./species.js";

const KEY = "racenav.wxpanel";
const PRESETS = {
  Fishing: { layer: "sst", overlays: { arrows: true, waveArrows: false, currArrows: true, isobars: false, isotherms: true, breaks: true } },
  Passage: { layer: "wind", overlays: { arrows: true, waveArrows: true, currArrows: true, isobars: true, isotherms: false, breaks: false } },
  Anchorage: { layer: "gust", overlays: { arrows: true, waveArrows: false, currArrows: false, isobars: true, isotherms: false, breaks: false } },
};
// Species presets (built-in figures, the user's changes, the editor): species.js
export { SPECIES, speciesConfig } from "./species.js";

export function setupWeatherPanel(map, weather, sidePanel, ui) {
  const $ = (id) => document.getElementById(id);
  const st = { open: false, x: null, y: null, collapsed: false, sec: { show: true, over: true }, presets: {}, envStart: null };
  try { Object.assign(st, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable or damaged: defaults */ }
  if (!st.sec || typeof st.sec !== "object") st.sec = { show: true, over: true };
  if (!st.presets || typeof st.presets !== "object") st.presets = {};
  // species presets: changed figures for built-in species, and the user's own species (species.js)
  if (!st.speciesEdits || typeof st.speciesEdits !== "object") st.speciesEdits = {};
  if (!Array.isArray(st.speciesOwn)) st.speciesOwn = [];
  const species = () => speciesList(st.speciesEdits, st.speciesOwn);
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch { /* ignore */ } };

  const win = $("wxp");
  win.innerHTML = `<div id="wxp-key" class="wx-key" hidden></div>
    <div class="env-start">
      <label class="env-start-row" title="What the Environment view is showing. Weather: wind, pressure, rain and sea state; Ocean: sea temperature, salinity, chlorophyll, currents and below the surface; Seabed: depth layers, with no weather shading. Each remembers its own layer, overlays and filter">Showing
        <select class="env-start-select" aria-label="What the Environment view is showing">
          <option value="weather">Weather</option><option value="ocean">Ocean</option><option value="seabed">Seabed</option>
        </select></label>
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
    <div id="wxp-show" class="wxp-chooser"></div>
    <div class="wxp-time"><div id="wxp-legend" class="wx-legend"></div><div id="wxp-filter"></div></div>
    <div class="wxp-body" id="wxp-body">
      <details data-sec="over"><summary>Overlays</summary><div id="wxp-over"></div><div id="wxp-isobar-controls"></div></details>
      <details data-sec="cmp" id="wxp-cmp"><summary>Compare GRIBs</summary>
        <h5>GRIBs to compare</h5><div id="wxp-cmp-files"></div>
        <h5>Shade where they disagree</h5><div id="wxp-cmp-show"></div><div id="wxp-cmp-note"></div>
        <h5>Each GRIB at the cursor</h5><div id="wxp-cmp-point"></div>
        <h5>Check against what happened</h5><div id="wxp-cmp-verify"></div>
      </details>
      <div class="wxp-presets"><select id="wxp-preset" title="Presets"></select><button id="wxp-psave" title="Save what is shown as a preset">Save…</button><button id="wxp-pdel" title="Delete the chosen saved preset">Delete</button><button id="wxp-pspecies" title="Change the species presets' temperatures, or add your own species">Species…</button></div>
      <div class="wxp-actions">
        <button id="wxp-data" type="button" title="Download or manage GRIB files (Get data)">GRIBs…</button>
        <button id="wxp-visible" title="Hide or show the weather on the chart">Hide weather</button>
        <button id="wxp-snap" title="Save the chart as a picture" aria-label="Save the chart as a picture"><svg viewBox="0 0 24 24" class="ic"><path d="M4 8h3l2-2.5h6L17 8h3v11H4z"/><circle cx="12" cy="13" r="3.5"/></svg> Picture</button>
      </div>
    </div>`;
  weather.mountLegend($("wxp-legend"));
  weather.mountLegend($("side-wx-key"), { always: true });
  weather.mountLegend($("wxp-key"), { always: true });
  weather.mountFilter($("wxp-filter"));
  // The colour key beside the chart time carries the filter, with its pointers, in every view that
  // shows it (a filter set in Planning, say, with no Weather layers section there); the key's own
  // bar steps aside while the filter shows, and the one in Weather layers and filters too (CSS).
  {
    const box = document.createElement("div");
    box.id = "side-wx-filter";
    $("side-wx-key").after(box);
    weather.mountFilter(box);
    // the key's bar steps aside while the filter shows: a class, not a :has() rule (that one made
    // the browser restyle the whole page on every change anywhere, e.g. each line of Live data)
    const keyAside = () => $("side-wx-key").classList.toggle("filter-shown", !box.hidden);
    new MutationObserver(keyAside).observe(box, { attributes: true, attributeFilter: ["hidden"] });
    keyAside();
    // the depth shown, for the layers at depth when the file has a depth stack: between the key and the filter
    const depth = document.createElement("div");
    depth.id = "side-wx-depth";
    $("side-wx-key").after(depth);
    weather.mountDepth(depth);
  }
  $("wxp-isobar-controls").append($("wx-isobar-controls"));
  // everything for comparing GRIBs in one place: the files, the agreement displays and their note,
  // each GRIB's values at the cursor, and Verify runs
  $("wxp-cmp-note").append($("wx-cmp-note"));
  {
    const old = $("wx-point-holder");
    if (old) { $("wxp-cmp-point").append(...old.querySelectorAll(".wxp-compare-table, #wx-sea-point")); old.remove(); }
    $("wx-verify").textContent = "Verify runs…";
    const row = $("wx-verify").parentElement;
    $("wxp-cmp-verify").append($("wx-verify"), $("wx-sst-verify"));
    if (row && !row.children.length) row.remove();
  }

  for (const d of win.querySelectorAll("details[data-sec]")) {
    d.open = st.sec[d.dataset.sec] ?? d.dataset.sec === "cmp";
    d.ontoggle = () => { st.sec[d.dataset.sec] = d.open; save(); };
  }

  // Chart › Layers › Weather controls… (the toolbar's Weather button has gone: Environment is on the view icons)
  const open = $("ly-wx-open");
  if (open) open.onclick = () => sidePanel.openWeather();
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
  const startSelect = win.querySelector(".env-start-select");
  function syncStartingPoint() {
    startSelect.value = st.envStart ?? "weather";
    // the seabed layers belong to the Seabed mode
    win.querySelector(".env-seabed").hidden = st.envStart !== "seabed";
  }
  // Weather, Ocean and Seabed are modes: each keeps its own layer, sources, overlays and filter, put back when
  // you return to it. The first time, a mode starts from sensible defaults for what is loaded.
  const MODE_LAYERS = { weather: ["wind", "gust", "mslp", "rain", "cape", "temp", "waves", "swell", "period"], ocean: ["sst", "skin", "chl", "sss", "subt", "subs", "subcur", "thermo_depth", "thermo_str", "shear_depth", "shear_str", "current", "ssh", "ice", "mld", "phyc", "nppv", "o2", "ph"] };
  const modeOf = (key) => Object.keys(MODE_LAYERS).find((m) => MODE_LAYERS[m].includes(key)) ?? null;
  st.modes ??= {};
  function setMode(key) {
    if (!["weather", "ocean", "seabed"].includes(key)) return;
    if (st.envStart && st.envStart !== key) st.modes[st.envStart] = weather.config();
    st.envStart = key;
    const kept = st.modes[key];
    const options = weather.layerOptions();
    weather.setVisible(true);
    let said = "";
    if (key === "seabed") {
      weather.applyConfig({ layer: "none", overlays: { arrows: false, waveArrows: false, currArrows: false, isobars: false, isotherms: false, breaks: false } });
      said = "Choose the seabed layers below.";
    } else if (kept && (kept.layer === "none" || options.some((o) => o.key === kept.layer && o.paths.length))) {
      weather.applyConfig(kept);
    } else {
      const choices = key === "weather" ? ["wind", "gust", "waves"] : ["sst", "skin", "chl", "sss", "current", "ice"];
      const choice = choices.map((name) => options.find((o) => o.key === name && o.paths.length)).find(Boolean);
      if (choice) weather.pickLayer(choice.key, choice.path ?? choice.paths[0]);
      weather.applyConfig({ layer: choice?.key ?? "none", overlays: {
        arrows: key === "weather", waveArrows: key === "weather", currArrows: key === "ocean",
        isobars: key === "weather", isotherms: key === "ocean", breaks: false } });
      if (!choice) said = `No ${key} GRIB loaded. Use Get data to load one.`;
    }
    $("env-start-status").textContent = said;
    win.querySelector(".env-seabed").open = key === "seabed";
    save();
    syncStartingPoint();
    refresh();
  }
  startSelect.addEventListener("change", () => setMode(startSelect.value));
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
  /** Put freshly built rows in a list only when they differ from what is there: the lists are asked to
   *  redraw at every time step, and replacing them would close an open drop-down and drop a click. */
  function put(target, built) {
    const sig = built.innerHTML + "|" + [...built.querySelectorAll("input, select")].map((e) => (e.type === "checkbox" || e.type === "radio" ? +e.checked : e.value)).join(",");
    if (target.dataset.sig === sig) return;
    target.dataset.sig = sig;
    target.replaceChildren(...built.childNodes);
  }
  function renderShow() {
    const box = document.createElement("div");
    buildShow(box);
    put($("wxp-show"), box);
  }
  // The layer chooser: a chip per layer that a loaded GRIB has, in groups, with the source of the chosen layer
  // under it. In Environment the groups follow the mode; elsewhere all are offered.
  const GROUPS = [["Weather", ["wind", "gust", "mslp", "rain", "cape", "temp"]], ["Sea state", ["waves", "swell", "period"]],
    ["Ocean", ["sst", "sst_err", "skin", "chl", "sss", "current", "ssh", "ice", "mld", "phyc", "nppv", "o2", "ph"]],
    ["Below the surface", ["subt", "subs", "subcur", "thermo_depth", "thermo_str", "shear_depth", "shear_str"]]];
  const CHIP = { wind: "Wind", gust: "Gusts", mslp: "Pressure", rain: "Rain", cape: "CAPE", temp: "Air temp", waves: "Waves", swell: "Swell", period: "Period",
    sst: "Sea temp", sst_err: "SST error", skin: "Sea temp (sat)", chl: "Chlorophyll", sss: "Salinity", subt: "Temp at depth", subs: "Salinity at depth", current: "Current", ssh: "Sea height", ice: "Sea ice", mld: "Mixed layer", phyc: "Phytoplankton", nppv: "Production", o2: "Oxygen", ph: "pH",
    subcur: "Current at depth", thermo_depth: "Thermocline", thermo_str: "Thermocline strength", shear_depth: "Shear depth", shear_str: "Shear strength" };
  function buildShow(box) {
    const info = weather.layerInfo();
    const opts = weather.layerOptions().filter((x) => !x.compare && !x.verify && x.paths.length);
    const env = document.documentElement.dataset.view === "env";
    // a layer picked some other way (a preset, a key) decides the mode shown
    if (env && !info.off && modeOf(info.key) && st.envStart !== "seabed" && modeOf(info.key) !== st.envStart) { st.envStart = modeOf(info.key); save(); syncStartingPoint(); }
    if (env && st.envStart === "seabed" && info.off) return; // Seabed: no weather shading to choose
    if (!opts.length) { box.innerHTML = `<div class="wx-note">No GRIB loaded. Open Get data to load one.</div>`; return; }
    const mode = env && MODE_LAYERS[st.envStart] ? st.envStart : null;
    const chip = (key, label, title, on, pick) => {
      const b = document.createElement("button");
      b.type = "button"; b.className = `wxp-chip${on ? " on" : ""}`;
      b.textContent = label; b.title = title;
      b.setAttribute("aria-pressed", String(on));
      b.dataset.key = key;
      b.onclick = pick;
      return b;
    };
    let any = false;
    for (const [name, keys] of GROUPS) {
      const here = keys.map((k) => opts.find((o) => o.key === k)).filter((o) => o && (!mode || MODE_LAYERS[mode].includes(o.key)));
      if (!here.length) continue;
      any = true;
      const g = document.createElement("div");
      g.className = "wxp-group";
      const h = document.createElement("span");
      h.className = "wxp-group-name"; h.textContent = name;
      g.append(h);
      for (const o of here) g.append(chip(o.key, CHIP[o.key] ?? o.label, `${o.label} (${o.unit})${o.help ? `: ${o.help}` : ""}`, !info.off && info.key === o.key,
        () => weather.pickLayer(o.key, o.path ?? o.paths[0])));
      box.append(g);
    }
    if (!any) { box.innerHTML = `<div class="wx-note">No ${mode} GRIB loaded. Open Get data to load one.</div>`; return; }
    const last = box.lastElementChild;
    last.append(chip("none", "None", "No shading: the overlays alone", info.off, () => weather.pickLayer("none")));
    // where the chosen layer comes from
    const cur = info.off ? null : opts.find((o) => o.key === info.key);
    if (cur) {
      const row = document.createElement("div");
      row.className = "wxp-row wxp-source";
      const name = document.createElement("span");
      name.textContent = `${cur.label} from`;
      row.append(name);
      if (cur.paths.length > 1) row.append(sourceSelect(cur.paths, cur.path, (p) => weather.pickLayer(cur.key, p)));
      else { const one = document.createElement("span"); one.className = "wx-note"; one.textContent = short(weather.sources().find((x) => x.path === cur.paths[0])?.name ?? ""); row.append(one); }
      box.append(row);
    }
  }
  // ---------- Compare GRIBs ----------
  let lastPlain = "wind"; // the layer to go back to when an agreement display is switched off
  function renderCompare() {
    const info = weather.layerInfo();
    const comparing = !info.off && !!(info.layer.compare || info.layer.verify);
    if (!comparing && !info.off) lastPlain = info.key;
    const files = weather.compareFiles().filter((f) => f.can); // ocean and satellite files have nothing the agreement displays use
    const box = document.createElement("div");
    if (files.length < 2) box.innerHTML = `<div class="wx-note">Load two or more GRIBs (Get data) to compare them.</div>`;
    else for (const f of files) {
      const row = document.createElement("label");
      row.className = "wxp-row";
      const c = document.createElement("input");
      c.type = "checkbox"; c.checked = f.on;
      c.onchange = () => weather.setCompare(f.path, c.checked);
      const name = document.createElement("span");
      name.textContent = f.name;
      row.append(c, name);
      box.append(row);
    }
    put($("wxp-cmp-files"), box);
    const show = document.createElement("div");
    const opts = weather.layerOptions().filter((o) => o.compare || o.verify);
    if (!opts.length) {
      show.innerHTML = `<div class="wx-note">${files.length < 2 ? "" : "Tick two or more GRIBs that have wind or pressure."}</div>`;
    } else for (const o of [{ key: "off", label: "Off" }, ...opts]) {
      const row = document.createElement("label");
      row.className = "wxp-row";
      const r = document.createElement("input");
      r.type = "radio"; r.name = "wxp-cmp-layer";
      r.checked = o.key === "off" ? !comparing : comparing && info.key === o.key;
      r.onchange = () => {
        if (o.key !== "off") weather.pickLayer(o.key);
        else if (comparing) { const back = weather.layerOptions().find((x) => x.key === lastPlain) ?? weather.layerOptions()[0]; if (back) weather.pickLayer(back.key, back.path ?? back.paths[0]); }
      };
      const name = document.createElement("span");
      name.textContent = o.key === "off" ? o.label : `${o.label} (${o.unit})`;
      if (o.help) row.title = o.help;
      row.append(r, name);
      show.append(row);
    }
    put($("wxp-cmp-show"), show);
    $("wx-verify").disabled = files.length < 2;
  }
  function renderOver() {
    const box = document.createElement("div");
    const isobarControls = $("wx-isobar-controls");
    if (isobarControls.parentElement !== $("wxp-isobar-controls")) $("wxp-isobar-controls").append(isobarControls);
    const opts = weather.overlayOptions();
    // the isobar interval only while a loaded GRIB has pressure to draw them from
    $("wxp-isobar-controls").hidden = !opts.some((o) => o.key === "isobars");
    if (!opts.length) {
      const n = document.createElement("div");
      n.className = "wx-note";
      n.textContent = "None until a GRIB is loaded.";
      box.append(n);
    }
    for (const o of opts) {
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
      // the contour interval: Auto (about ten lines over the band, else over what is in view) or fixed
      if (o.key === "isotherms" && o.on) {
        const r = document.createElement("label");
        r.className = "wxp-row wxp-sub";
        const sel = document.createElement("select");
        sel.title = "Contour interval. Auto: about ten lines across the band, or across the values in view";
        for (const v of ["auto", 0.1, 0.2, 0.25, 0.5, 1, 2]) sel.append(new Option(v === "auto" ? "Auto" : String(v), String(v)));
        sel.value = String(weather.contourStep());
        sel.onchange = () => weather.setContourStep(sel.value);
        const t = document.createElement("span");
        t.textContent = "Interval";
        r.append(t, sel);
        box.append(r);
      }
    }
    put($("wxp-over"), box);
  }
  // ---------- the timeline under the time slider ----------
  // Day ticks, a mark at now, and a bar for each loaded GRIB showing the times it covers (the shown one
  // brighter), on the slider's own scale: where a file runs out is seen before the slider gets there.
  const tl = document.createElement("div");
  tl.id = "side-wx-timeline";
  tl.title = "The times each loaded GRIB covers. Click to go to a time";
  $("side-wx-time").after(tl);
  tl.onclick = (e) => {
    const sl = $("side-wx-time"), a = Number(sl.min), b = Number(sl.max), r = tl.getBoundingClientRect();
    if (b > a && r.width) weather.setTime(a + ((e.clientX - r.left) / r.width) * (b - a));
  };
  function renderTimeline() {
    const sl = $("side-wx-time"), a = Number(sl.min), b = Number(sl.max);
    const srcs = weather.sources().map((x) => ({ ...x, span: weather.span(x.path) })).filter((x) => x.span);
    tl.hidden = !(b > a) || !srcs.length;
    if (tl.hidden) return;
    const pc = (t) => `${Math.min(100, Math.max(0, ((t - a) / (b - a)) * 100)).toFixed(2)}%`;
    const days = (b - a) / 86400;
    // a label every so many days, so the labels (about 46 px each) never run into each other
    const width = tl.clientWidth || 240, LAB = 46;
    const every = Math.max(1, Math.ceil(LAB / (width / days)));
    let html = "";
    // local midnights
    const d0 = new Date(a * 1000); d0.setHours(24, 0, 0, 0);
    for (let d = d0, n = 0; d.getTime() / 1000 < b; d = new Date(d.getTime()), d.setDate(d.getDate() + 1), n++) {
      const t = d.getTime() / 1000;
      html += `<i class="day" style="left:${pc(t)}"></i>`;
      if (n % every === 0 && ((t - a) / (b - a)) * width < width - LAB + 6) html += `<span class="lab" style="left:${pc(t)}">${d.toLocaleDateString(undefined, { weekday: "short" })} ${d.getDate()}</span>`;
    }
    srcs.forEach((x, i) => {
      html += `<b class="cov${x.shown ? " shown" : ""}" style="left:${pc(x.span[0])};width:calc(${pc(x.span[1])} - ${pc(x.span[0])});top:${16 + i * 5}px" title="${x.name.replace(/"/g, "&quot;")}"></b>`;
    });
    const now = Date.now() / 1000;
    if (now > a && now < b) html += `<i class="now" style="left:${pc(now)}" title="Now"></i>`;
    html += `<i class="at" style="left:${pc(weather.current().t)}"></i>`;
    tl.style.height = `${18 + srcs.length * 5}px`;
    if (tl.dataset.sig !== html) { tl.dataset.sig = html; tl.innerHTML = html; }
  }
  weather.onTime(renderTimeline);
  weather.onLoaded?.(renderTimeline);
  // the panel's width decides how many day labels fit
  if (typeof ResizeObserver === "function") {
    let lastWidth = 0;
    new ResizeObserver(() => { if (tl.clientWidth !== lastWidth) { lastWidth = tl.clientWidth; renderTimeline(); } }).observe(tl);
  }
  weather.onChange(renderTimeline);

  function renderPresets() {
    const sel = $("wxp-preset");
    sel.innerHTML = "";
    sel.append(new Option("Presets…", ""));
    for (const n of Object.keys(PRESETS)) sel.append(new Option(n, `b:${n}`));
    const fish = document.createElement("optgroup");
    fish.label = "Species (sea temperature)";
    for (const sp of species()) {
      const o = new Option(`${sp.name} ${sp.blo}–${sp.bhi} °C${sp.edited ? " (yours)" : ""}`, `s:${sp.name}`);
      o.title = speciesNote(sp);
      fish.append(o);
    }
    sel.append(fish);
    for (const n of Object.keys(st.presets)) sel.append(new Option(`★ ${n}`, `u:${n}`));
  }
  $("wxp-preset").onchange = (e) => {
    const [k, n] = [e.target.value.slice(0, 1), e.target.value.slice(2)];
    const sp = k === "s" ? species().find((x) => x.name === n) : null;
    const c = k === "b" ? PRESETS[n] : k === "u" ? st.presets[n] : sp ? spConfig(sp) : null;
    if (c && weather.applyConfig(c) === false) {
      if (c.layer === "subt") { offerDepths(n, c); return; }
      activity.start(`Preset ${n}`).fail("No sea temperature in the loaded files: get one in Get data (MUR, ACSPO, Geo-Polar…).");
    }
  };
  /** A preset needing temperature at depth with none loaded: offer to get Copernicus at that depth for the
   *  chart view (gribdl.js listens for racenav-get-depths) and apply the preset once it is loaded. */
  async function offerDepths(name, c) {
    const depth = c.depth ?? 100;
    const b = map.getBounds();
    const w = b.getWest(), e = b.getEast(), so = b.getSouth(), no = b.getNorth();
    const big = e - w > 20 || no - so > 20;
    const v = await ask({
      title: `${name}: temperature at ${depth} m`,
      note: `None of the loaded files has sea temperature at depth. Get it from the Copernicus ocean model (free, no account) for ${big ? "a 20° box in the middle of the chart" : "the area on the chart now"}: sea temperature, and temperature, salinity and current at ${depth} m. The preset is applied once it has loaded.`,
      fields: [{ key: "days", label: "Days ahead", type: "select", value: "3", options: [["2", "2 days"], ["3", "3 days"], ["5", "5 days"], ["10", "10 days"]] }],
      ok: "Get it",
    });
    if (!v) return;
    let area = [w, so, e, no];
    if (big) {
      const cx = (w + e) / 2, cy = (so + no) / 2;
      area = [cx - 10, Math.max(-80, cy - 10), cx + 10, Math.min(80, cy + 10)];
    }
    window.addEventListener("racenav-depths-loaded", () => {
      if (weather.applyConfig(c) === false) activity.start(`Preset ${name}`).fail("Still no sea temperature at depth: the download didn't finish (see the activity log).");
    }, { once: true });
    window.dispatchEvent(new CustomEvent("racenav-get-depths", { detail: { area, depth, days: Number(v.days) || 3, label: `${name}: Copernicus at ${depth} m` } }));
  }
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
  $("wxp-pspecies").onclick = async () => {
    const v = await editSpecies(st.speciesEdits, st.speciesOwn);
    if (!v) return;
    st.speciesEdits = v.edits;
    st.speciesOwn = v.own;
    save(); renderPresets();
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
    // the folded "Loaded GRIBs" in the panel turns amber when the run is old or has no data at chart time
    queueMicrotask(() => $("side-wx-gribs")?.classList.toggle("stale", run.classList.contains("stale")));
    if (!s) { run.textContent = "No GRIB loaded"; return; }
    const now = Date.now() / 1000;
    const ageH = s.ref ? (now - s.ref) / 3600 : null;
    const [lo, hi] = [info.srcMin ?? info.min, info.srcMax ?? info.max];
    const over = hi != null && now > hi;
    const outside = hi != null && (cur.t > hi || cur.t < lo);
    // satellite products are analyses of the past, not forecast runs
    const analysis = /VIIRS|MUR SST|Himawari|sea ice/i.test(s.model ?? s.name ?? "");
    const age = (h) => (h < 48 ? `${Math.round(h)} h` : `${Math.round(h / 24)} d`);
    if (analysis) {
      run.textContent = `${s.name} · analysis, ${fmt(lo)} to ${fmt(hi)}`;
      const lastH = (now - hi) / 3600;
      // how late each product normally is when published: VIIRS a couple of days, MUR a day or two, Himawari hours
      const slackH = /VIIRS/i.test(s.model ?? s.name ?? "") ? 96 : /MUR|sea ice/i.test(s.model ?? s.name ?? "") ? 72 : 12;
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
      renderHead(); renderShow(); renderOver(); renderCompare();
      $("wxp-visible").textContent = weather.visible() ? "Hide weather" : "Show weather";
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
      layerOptions: weather.layerOptions(), overlayOptions: weather.overlayOptions(), view: document.documentElement.dataset.view,
    });
  }
  weather.onChange(renderChartState);
  weather.onLoaded(renderChartState);
  renderChartState();
  renderPresets();
  refresh();
}
