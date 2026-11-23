// GRIB tools in the dock:
//  - Meteogram: one point over time (right-click > Meteogram here). Wind speed,
//    direction and pressure for every loaded GRIB (ensembles as their mean with
//    the members' range as a band); waves, rain, temperatures and current from
//    the first loaded GRIB that has them. The plots are chosen in Plots ▾.
//  - Verify: how well earlier runs forecast the wind, scored against a later
//    run's hour 0 over the chart view, and the track record by model and lead.
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { ddm } from "./navmath.js";

const MG_KEY = "racenav.meteogram";
/** The meteogram's plots: models = each loaded GRIB's own line; along = the first loaded GRIB that has it. */
const PLOTS = [
  { key: "tws", group: "Wind", label: "Wind speed, gusts and direction arrows", title: "TWS kn (gusts dashed, arrows: where the wind goes)", h: 130, src: "models" },
  { key: "twd", group: "Wind", label: "Wind direction", title: "TWD °T", h: 90, src: "models" },
  { key: "waves", group: "Waves", label: "Waves: height, period, direction", short: "Waves", title: "Waves m (period s dashed, right)", h: 100, src: "along" },
  { key: "mslp", group: "Atmosphere", label: "Pressure", title: "MSLP hPa", h: 80, src: "models" },
  { key: "rain", group: "Atmosphere", label: "Rain", short: "Rain", title: "Rain mm/h", unit: "mm/h", h: 60, src: "along", bars: true, col: "#2a62c9" },
  { key: "temp", group: "Atmosphere", label: "Air temperature", short: "Air temp", title: "Air temp °C", unit: "°C", h: 70, src: "along", col: "#c0392b" },
  { key: "sst", group: "Sea", label: "Sea temperature", short: "Sea temp", title: "Sea temp °C", unit: "°C", h: 70, src: "along", col: "#006994" },
  { key: "cur", group: "Sea", label: "Current", short: "Current", title: "Current kn", unit: "kn", h: 70, src: "along", col: "#1abc9c" },
];
const DEFAULT_PLOTS = ["tws", "twd", "waves", "mslp"];
const COLOURS = ["#1f77b4", "#d62728", "#2ca02c", "#9467bd", "#ff7f0e", "#17becf", "#8c564b", "#e377c2", "#7f7f7f", "#bcbd22"];
const NS = "http://www.w3.org/2000/svg";
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const ang = (a, b) => ((((a - b) % 360) + 540) % 360) - 180; // signed difference, -180..180, for any a and b
const shortName = (name) => name.replace(/ \d+ \w+ \d+Z$/, "");
function fmtRun(t) {
  const d = new Date(t * 1000);
  return `${d.getUTCDate()} ${d.toLocaleString("en-NZ", { month: "short", timeZone: "UTC" })} ${String(d.getUTCHours()).padStart(2, "0")}Z`;
}
const label = (f) => `${shortName(f.name)} ${fmtRun(f.run)}${f.members > 1 ? ` (${f.members} members)` : ""}`;
const SV_COLD = "#2166ac", SV_WARM = "#b2182b"; // product colder / warmer than the boat (red–blue: safe for red-green colour blindness)
const num = (v, d = 1, plus = false) => (v == null ? "—" : `${plus && v > 0 ? "+" : ""}${v.toFixed(d)}`);

export function setupGribTools(map, { weather, dock }) {
  // ---------- meteogram ----------
  // The plots to show are chosen in the Plots menu (kept between sessions). Wind and pressure are drawn for
  // every loaded GRIB that has them (to compare models); waves, rain, temperatures and current come from the
  // first loaded GRIB that has each (the shown one first), like Weather on route.
  let point = null; // [lon, lat]
  let series = null; // the models' wind and pressure (last fetch)
  let along = null; // { rows: [{ t, wave, period, wavedir, rain, temp, sst, cur, _sources }], step }
  let busy = false;
  let plots = new Set(DEFAULT_PLOTS);
  try { const s = JSON.parse(localStorage.getItem(MG_KEY) || "null"); if (Array.isArray(s?.plots)) plots = new Set(s.plots.filter((k) => PLOTS.some((p) => p.key === k))); } catch { /* ignore */ }
  const savePlots = () => { try { localStorage.setItem(MG_KEY, JSON.stringify({ plots: [...plots] })); } catch { /* ignore */ } };
  map.addSource("mg-point", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "mg-point", type: "circle", source: "mg-point",
    paint: { "circle-radius": 6, "circle-color": "rgba(0,0,0,0)", "circle-stroke-color": "#7b2cbf", "circle-stroke-width": 2.5 } });
  // the point's ring shows only while its meteogram is open
  map.setLayoutProperty("mg-point", "visibility", "none");
  dock.onChange((cur) => map.setLayoutProperty("mg-point", "visibility", cur === "meteogram" ? "visible" : "none"));
  function setPoint(p) {
    point = p;
    map.getSource("mg-point").setData({ type: "FeatureCollection",
      features: p ? [{ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: p } }] : [] });
  }

  async function fetchSeries() {
    if (!point) return;
    busy = true;
    const at = point;
    const [ms, rows] = await Promise.all([
      invoke("grib_meteogram", { paths: [], lat: at[1], lon: at[0] }).catch(() => []),
      fetchAlong(at),
    ]);
    if (at !== point) return; // another point was chosen meanwhile
    series = (ms || []).filter((s) => s.points?.some((q) => q.tws != null || q.mslp != null));
    along = rows;
    busy = false;
    dock.refresh("meteogram");
  }
  /** Every field at the point through the loaded GRIBs' combined times. */
  async function fetchAlong(at) {
    const { min, max } = weather.layerInfo();
    if (min == null || max == null || max <= min) return null;
    const span = max - min;
    const step = span <= 5 * 86400 ? 3600 : 10800;
    const pts = [];
    for (let t = min; t <= max && pts.length < 600; t += step) pts.push([at[1], at[0], t]);
    const vals = await invoke("grib_along", { first: weather.current().path ?? null, pts }).catch(() => null);
    return vals ? { step, rows: vals.map((v, i) => ({ ...v, t: pts[i][2] })) } : null;
  }

  let cursorLine = null; // the slider-time line in the shown meteogram: (t) => void
  let internal = false; // the meteogram moved the slider itself: its cursor is already there
  weather.onTime((t) => { if (!internal) cursorLine?.(t); });
  weather.onLoaded(() => {
    if (point && dock.current() === "meteogram") fetchSeries();
    if (dock.current() === "verify") { vtimes = null; dock.refresh("verify"); }
  });

  // the Plots menu: a list of the plots to tick, grouped; plots the loaded GRIBs don't have at the point are greyed
  let menu = null;
  function closeMenu() { menu?.remove(); menu = null; document.removeEventListener("pointerdown", outside, true); }
  function outside(e) { if (menu && !menu.contains(e.target) && !e.target.closest?.(".mg-plots-btn")) closeMenu(); }
  function openMenu(btn) {
    if (menu) { closeMenu(); return; }
    menu = document.createElement("div");
    menu.className = "ctx checks mg-plots";
    let group = "";
    for (const p of PLOTS) {
      if (p.group !== group) { group = p.group; menu.append(Object.assign(document.createElement("div"), { className: "grp", textContent: group })); }
      const has = hasPlot(p);
      const lab = document.createElement("label");
      lab.title = has ? "" : "Not in the loaded GRIBs at this point";
      lab.innerHTML = `<input type="checkbox"> <span></span>`;
      lab.querySelector("span").textContent = has ? p.label : `${p.label} (no data)`;
      lab.classList.toggle("off", !has);
      const box = lab.querySelector("input");
      box.checked = plots.has(p.key);
      box.onchange = () => { if (box.checked) plots.add(p.key); else plots.delete(p.key); savePlots(); dock.refresh("meteogram"); openMenu(document.querySelector(".mg-plots-btn")); };
      menu.append(lab);
    }
    const reset = document.createElement("button");
    reset.type = "button";
    reset.textContent = "Standard plots";
    reset.title = "Wind speed, wind direction, waves and pressure";
    reset.onclick = () => { plots = new Set(DEFAULT_PLOTS); savePlots(); dock.refresh("meteogram"); closeMenu(); };
    menu.append(reset);
    document.body.append(menu);
    const r = btn.getBoundingClientRect();
    menu.style.left = `${Math.max(4, Math.min(r.left, window.innerWidth - menu.offsetWidth - 4))}px`;
    menu.style.top = `${Math.max(4, r.top - menu.offsetHeight - 4 < 4 ? r.bottom + 4 : r.top - menu.offsetHeight - 4)}px`;
    document.addEventListener("pointerdown", outside, true);
  }
  function hasPlot(p) {
    if (p.src === "models") return !!series?.some((s) => s.points.some((q) => (p.key === "tws" ? q.tws ?? q.gust : q[p.key]) != null));
    const k = p.key === "waves" ? "wave" : p.key;
    return !!along?.rows.some((r) => (k === "cur" ? r.cur?.kn : r[k]) != null);
  }

  dock.register("meteogram", {
    title: "Meteogram",
    render(body, tools) {
      cursorLine = null;
      closeMenu();
      if (!point) { body.innerHTML = `<p class="wx-note">Right-click the chart and choose Meteogram here.</p>`; return; }
      const plotsBtn = document.createElement("button");
      plotsBtn.className = "mg-plots-btn";
      plotsBtn.textContent = "Plots ▾";
      plotsBtn.title = "Choose what the meteogram shows: wind, waves, atmosphere, sea";
      plotsBtn.onclick = () => openMenu(plotsBtn);
      const clear = document.createElement("button");
      clear.textContent = "Clear point";
      clear.onclick = () => { setPoint(null); series = null; along = null; dock.refresh("meteogram"); };
      tools.append(plotsBtn, clear);
      const head = document.createElement("div");
      head.className = "mg-head";
      head.textContent = `${ddm(point[1], "N", "S")} ${ddm(((point[0] + 540) % 360) - 180, "E", "W")}`;
      body.append(head);
      if (busy || !series) { body.append(Object.assign(document.createElement("p"), { className: "wx-note", textContent: "Reading the GRIBs…" })); return; }
      const shown = PLOTS.filter((p) => plots.has(p.key));
      const anyData = PLOTS.some(hasPlot);
      if (!anyData) { body.append(Object.assign(document.createElement("p"), { className: "wx-note", textContent: "No loaded GRIB covers this point." })); return; }
      if (!shown.length) { body.append(Object.assign(document.createElement("p"), { className: "wx-note", textContent: "No plots chosen: tick some in Plots ▾." })); return; }
      const legend = document.createElement("div");
      legend.className = "mg-legend";
      if (shown.some((p) => p.src === "models")) series.forEach((s, k) => {
        const sp = document.createElement("span");
        sp.innerHTML = `<i style="background:${COLOURS[k % COLOURS.length]}"></i>${esc(label(s))}`;
        legend.append(sp);
      });
      // where the other plots come from
      const names = new Map(weather.all().map((g) => [g.path, g.name]));
      const from = [];
      for (const p of shown.filter((q) => q.src === "along" && hasPlot(q))) {
        const k = p.key === "waves" ? "wave" : p.key;
        const src = along.rows.find((r) => r._sources?.[k])?._sources[k];
        if (src) from.push(`${p.short}: ${shortName(names.get(src) ?? src.split(/[\\/]/).pop())}`);
      }
      if (from.length) legend.append(Object.assign(document.createElement("span"), { className: "mg-from", textContent: from.join(" · ") }));
      body.append(legend);
      // inspecting: press and drag on the plots, or step with Previous / Next; the chart's weather follows
      const inspect = document.createElement("div");
      inspect.className = "rw-inspect";
      const readout = document.createElement("div");
      readout.className = "rw-readout";
      const prev = document.createElement("button"), next = document.createElement("button"), nowBtn = document.createElement("button");
      prev.textContent = "◀ Previous"; next.textContent = "Next ▶"; nowBtn.textContent = "Now";
      inspect.append(readout, prev, next, nowBtn);
      let pending = null, timer = 0;
      const moveChart = (t) => {
        clearTimeout(timer); pending = null;
        internal = true;
        try { weather.setTime(t); } finally { internal = false; }
      };
      const flush = () => { if (pending != null) moveChart(pending); };
      let svg = null, sel = null;
      const show = (t, deferred = false) => {
        sel = t;
        svg.cursor(t);
        const { min, max } = weather.layerInfo();
        const inRange = min != null && t >= min && t <= max;
        readout.textContent = `Inspecting: ${fmt(t)}${inRange ? "" : " · outside the loaded forecast times: the chart stays where it is"}`;
        if (!inRange) return;
        if (deferred) { pending = t; clearTimeout(timer); timer = setTimeout(flush, 150); } else moveChart(t);
      };
      svg = drawMeteogram(series, along, shown, Math.max(500, body.clientWidth - 20), { pick: (t) => show(t, true), release: flush, primary: Math.max(0, series.findIndex((s) => s.name === weather.current().name)) });
      body.append(svg.el, inspect);
      const stepS = along?.step ?? 3600;
      const clampT = (t) => Math.max(svg.t0, Math.min(svg.t1, t));
      prev.onclick = () => show(clampT(Math.ceil((sel ?? weather.current().t ?? svg.t0) / stepS) * stepS - stepS));
      next.onclick = () => show(clampT(Math.floor((sel ?? weather.current().t ?? svg.t0) / stepS) * stepS + stepS));
      const nowT = Date.now() / 1000;
      nowBtn.disabled = !(nowT >= svg.t0 && nowT <= svg.t1);
      nowBtn.title = nowBtn.disabled ? "The time now is outside these forecasts" : "Go to the time now";
      nowBtn.onclick = () => show(Math.round(Date.now() / 1000 / 900) * 900);
      cursorLine = (t) => { sel = t; svg.cursor(t); readout.textContent = t != null && t >= svg.t0 && t <= svg.t1 ? `Inspecting: ${fmt(t)}` : "Press on the plots and drag, or use Previous / Next, to look along the forecast."; };
      cursorLine(weather.current().t);
    },
  });

  // ---------- verify ----------
  let vtimes = null; // [{ t, refs, forecasts }]
  let vt = null; // chosen verify time
  let scores = null, track = null;
  let kind = "vector";
  const bbox = () => { const b = map.getBounds(); return [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()]; };

  async function loadVerify() {
    vtimes = await invoke("grib_verify_times", { paths: [] }).catch(() => []) || [];
    vtimes = vtimes.filter((v) => v.forecasts.length);
    if (!vtimes.some((v) => v.t === vt)) vt = vtimes.at(-1)?.t ?? null;
    await loadScores();
  }
  async function loadScores() {
    scores = track = null;
    dock.refresh("verify");
    if (vt == null) return;
    const bb = bbox();
    [scores, track] = await Promise.all([
      invoke("grib_verify_scores", { paths: [], t: vt, bbox: bb }).catch(() => []),
      invoke("grib_verify_track", { paths: [], bbox: bb }).catch(() => []),
    ]);
    dock.refresh("verify");
  }

  dock.register("verify", {
    title: "Verify",
    render(body, tools) {
      if (vtimes == null) { body.innerHTML = `<p class="wx-note">Reading the GRIBs…</p>`; loadVerify(); return; }
      if (!vtimes.length) {
        body.innerHTML = `<p class="wx-note">Nothing to verify yet. Load a run together with an earlier run of the same area:` +
          ` the newer run's hour 0 is the best estimate of what really happened, and the earlier run is scored against it.</p>`;
        return;
      }
      const re = document.createElement("button");
      re.textContent = "Rescore this view";
      re.title = "Score again over the area now shown on the chart";
      re.onclick = loadScores;
      tools.append(re);
      const target = weather.verifyTarget();
      if (target) {
        const off = document.createElement("button");
        off.textContent = "Stop shading";
        off.onclick = () => { weather.setVerify(null); dock.refresh("verify"); };
        tools.append(off);
      }

      const bar = document.createElement("div");
      bar.className = "vf-bar";
      const sel = document.createElement("select");
      for (const v of vtimes) sel.append(new Option(`${fmt(v.t)}: hour 0 of ${v.refs.map((r) => shortName(r.name)).join(", ")}`, v.t));
      sel.value = vt;
      sel.onchange = () => { vt = Number(sel.value); loadScores(); };
      bar.append(Object.assign(document.createElement("span"), { textContent: "Verify at" }), sel);
      const kinds = document.createElement("span");
      kinds.className = "seg";
      for (const [k, txt] of [["vector", "Vector"], ["speed", "Speed"], ["direction", "Direction"]]) {
        const b = document.createElement("button");
        b.textContent = txt;
        b.classList.toggle("on", kind === k);
        b.title = "What the chart shading shows when you click a forecast";
        b.onclick = () => { kind = k; if (target) weather.setVerify({ ...target, kind }); dock.refresh("verify"); };
        kinds.append(b);
      }
      bar.append(kinds);
      body.append(bar);

      const note = document.createElement("p");
      note.className = "wx-note";
      note.textContent = "Scored over sea points in the chart view. RMS = vector error; bias = forecast speed minus actual " +
        "(+ = too windy); veer = mean direction error (+ = forecast veered). Click a forecast to shade its error on the chart.";
      body.append(note);
      if (!scores) { body.append(Object.assign(document.createElement("p"), { className: "wx-note", textContent: "Scoring…" })); return; }

      const t1 = document.createElement("table");
      t1.className = "cmp vf-table";
      t1.innerHTML = `<thead><tr><th>Forecast</th><th>Lead</th><th>RMS kn</th><th>Bias kn</th><th>|Dir| °</th><th>Veer °</th><th>MSL hPa</th><th>Points</th></tr></thead><tbody></tbody>`;
      const ranked = [...scores].sort((a, b) => a.rms - b.rms);
      for (const [i, s] of ranked.entries()) {
        const tr = document.createElement("tr");
        tr.className = "click";
        tr.classList.toggle("best", i === 0 && ranked.length > 1);
        tr.classList.toggle("on", target?.path === s.path && target?.t === vt);
        tr.innerHTML = `<td>${esc(label(s))}</td><td>+${Math.round(s.lead_h)} h</td><td>${num(s.rms)}</td><td>${num(s.bias, 1, true)}</td>` +
          `<td>${num(s.dir, 0)}</td><td>${num(s.veer, 0, true)}</td><td>${num(s.msl, 1, true)}</td><td>${s.n}</td>`;
        tr.onclick = () => {
          weather.setVerify({ t: vt, path: s.path, name: label(s), lead: s.lead_h, kind });
          dock.refresh("verify");
        };
        t1.tBodies[0].append(tr);
      }
      if (!ranked.length) t1.tBodies[0].innerHTML = `<tr><td colspan="8">No sea points scored in this view.</td></tr>`;
      body.append(t1);

      if (track?.length) {
        const h = document.createElement("h4");
        h.textContent = "Track record (every verify time, this view)";
        body.append(h);
        const t2 = document.createElement("table");
        t2.className = "cmp vf-table";
        t2.innerHTML = `<thead><tr><th>Model</th><th>Lead</th><th>Times</th><th>RMS kn</th><th>Bias kn</th><th>|Dir| °</th><th>Veer °</th><th>MSL hPa</th></tr></thead><tbody></tbody>`;
        for (const r of track) {
          const tr = document.createElement("tr");
          tr.innerHTML = `<td>${esc(r.model)}</td><td>${esc(r.band)}</td><td>${r.count}</td><td>${num(r.rms)}</td><td>${num(r.bias, 1, true)}</td>` +
            `<td>${num(r.dir, 0)}</td><td>${num(r.veer, 0, true)}</td><td>${num(r.msl, 1, true)}</td>`;
          t2.tBodies[0].append(tr);
        }
        body.append(t2);
      }
    },
  });

  // ---------- sea temperature products against the boat (P1.4) ----------
  // Every sea temperature file (loaded, and with "files not loaded" those in the weather
  // folders) read at the boat's logged positions and times, scored per product.
  const SV_KEY = "racenav.sstcheck";
  let sv = { scope: "all", disk: true, age: 36 };
  try { sv = { ...sv, ...JSON.parse(localStorage.getItem(SV_KEY) || "{}") }; } catch { /* ignore */ }
  const svSave = () => { try { localStorage.setItem(SV_KEY, JSON.stringify(sv)); } catch { /* ignore */ } };
  let svRes = null; // the last result, or { error }
  let svPick = null; // the product plotted and shown on the chart
  let svBusy = false;
  map.addSource("sstv-pts", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "sstv-pts", type: "circle", source: "sstv-pts", layout: { visibility: "none" },
    paint: { "circle-radius": 4, "circle-stroke-width": 0.75, "circle-stroke-color": "#222",
      "circle-color": ["interpolate", ["linear"], ["get", "d"], -2, SV_COLD, -0.5, "#92c5de", 0, "#f7f7f7", 0.5, "#f4a582", 2, SV_WARM] } });
  dock.onChange((cur) => map.setLayoutProperty("sstv-pts", "visibility", cur === "sstcheck" && svPick ? "visible" : "none"));
  function svShow() {
    const p = svRes?.products?.find((x) => x.product === svPick);
    map.getSource("sstv-pts").setData({ type: "FeatureCollection", features: (p?.pairs ?? []).map(([t, boat, prod, lat, lon]) =>
      ({ type: "Feature", properties: { t, d: Math.round((prod - boat) * 100) / 100 }, geometry: { type: "Point", coordinates: [lon, lat] } })) });
    map.setLayoutProperty("sstv-pts", "visibility", p && dock.current() === "sstcheck" ? "visible" : "none");
  }
  async function svLoad() {
    svBusy = true;
    svRes = null;
    dock.refresh("sstcheck");
    const b = map.getBounds();
    const now = Math.floor(Date.now() / 1000);
    const args = {
      trackIds: [],
      bbox: sv.scope === "view" ? [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()] : null,
      since: sv.scope === "7d" ? now - 7 * 86400 : sv.scope === "30d" ? now - 30 * 86400 : null,
      fromDisk: !!sv.disk,
      maxAgeH: Number(sv.age) || 36,
    };
    svRes = await invoke("sst_verify", args).catch((e) => ({ error: String(e) }));
    svBusy = false;
    if (!svRes.products?.some((p) => p.product === svPick)) svPick = svRes.products?.[0]?.product ?? null;
    svShow();
    dock.refresh("sstcheck");
  }
  const pct = (x) => `${Math.round(x * 100)} %`;
  const day = (t) => new Date(t * 1000).toLocaleDateString("en-NZ", { day: "numeric", month: "short", year: "numeric" });

  dock.register("sstcheck", {
    title: "Sea temp v boat",
    render(body, tools) {
      const scope = document.createElement("select");
      scope.title = "Which of the boat's tracks to use";
      for (const [v, t] of [["all", "All tracks"], ["view", "Tracks in the chart view"], ["7d", "Last 7 days"], ["30d", "Last 30 days"]]) scope.append(new Option(t, v));
      scope.value = sv.scope;
      scope.onchange = () => { sv.scope = scope.value; svSave(); svLoad(); };
      const age = document.createElement("select");
      age.title = "The oldest sea temperature field matched to a reading";
      for (const h of [12, 24, 36, 72]) age.append(new Option(`Data up to ${h} h old`, h));
      age.value = sv.age;
      age.onchange = () => { sv.age = Number(age.value); svSave(); svLoad(); };
      const disk = document.createElement("label");
      disk.title = "Also read the sea temperature files in the weather folders that cover the tracks, without loading them";
      disk.innerHTML = `<input type="checkbox"> Files not loaded`;
      disk.firstChild.checked = !!sv.disk;
      disk.firstChild.onchange = (e) => { sv.disk = e.target.checked; svSave(); svLoad(); };
      const again = document.createElement("button");
      again.textContent = "Score again";
      again.onclick = svLoad;
      tools.append(scope, age, disk, again);

      if (svRes == null) { body.innerHTML = `<p class="wx-note">Reading the tracks and sea temperature files…</p>`; if (!svBusy) svLoad(); return; }
      if (svRes.error) { body.innerHTML = `<p class="wx-note">${esc(svRes.error)}</p>`; return; }
      if (!svRes.readings) {
        body.innerHTML = `<p class="wx-note">No water temperature readings to score: none of these tracks logged a sea temperature at the times ` +
          `your sea temperature files cover${sv.disk ? "" : " (tick Files not loaded to use the ones in the weather folders too)"}. Download MUR, ACSPO or Geo-Polar ` +
          `for days you were out. TweedPlot logs the sea temperature with the track whenever an instrument sends one (a sounder or a temperature sensor, through NMEA 0183 MTW or Signal K).</p>`;
        return;
      }
      const head = document.createElement("p");
      head.className = "wx-note";
      head.textContent = `${svRes.readings.toLocaleString("en-NZ")} readings from ${svRes.tracks} track${svRes.tracks === 1 ? "" : "s"}, ` +
        `${day(svRes.first)} to ${day(svRes.last)}; ${svRes.files} sea temperature file${svRes.files === 1 ? "" : "s"} read` +
        (svRes.skipped ? ` (${svRes.skipped} could not be read)` : "") + ".";
      body.append(head);
      if (!svRes.products.length) {
        body.append(Object.assign(document.createElement("p"), { className: "wx-note",
          textContent: "None of the sea temperature files cover these tracks' places and times. Download MUR, ACSPO or Geo-Polar for the dates you were out " +
            "(Get data › Ocean, with past dates), or tick Files not loaded." }));
        return;
      }
      const note = document.createElement("p");
      note.className = "wx-note";
      note.textContent = "Bias = product minus the boat (+ = product too warm). SD = the spread once the bias is taken off: how well the product " +
        "follows the changes the boat saw, which is what finds temperature breaks. A hull sensor reads 0.5–2 m down, so a steady bias on calm, sunny " +
        "days is normal. Coverage = readings that got a value (clouds and coastlines leave gaps). Click a product to plot it and show its error on the chart.";
      body.append(note);
      const t = document.createElement("table");
      t.className = "cmp vf-table";
      t.innerHTML = `<thead><tr><th>Product</th><th>Bias °C</th><th>SD °C</th><th>RMS °C</th><th>Mean |error| °C</th><th>Coverage</th><th>Data age h</th><th>Readings</th><th>Files</th></tr></thead><tbody></tbody>`;
      for (const [i, p] of svRes.products.entries()) {
        const tr = document.createElement("tr");
        tr.className = "click";
        tr.classList.toggle("best", i === 0 && svRes.products.length > 1);
        tr.classList.toggle("on", p.product === svPick);
        tr.title = p.files.join("\n");
        tr.innerHTML = `<td>${esc(p.product)}</td><td>${num(p.bias, 2, true)}</td><td>${num(p.sd, 2)}</td><td>${num(p.rms, 2)}</td><td>${num(p.mae, 2)}</td>` +
          `<td>${pct(p.coverage)}</td><td>${num(p.age_h, 0)}</td><td>${p.n}</td><td>${p.files.length}</td>`;
        tr.onclick = () => { svPick = p.product; svShow(); dock.refresh("sstcheck"); };
        t.tBodies[0].append(tr);
      }
      body.append(t);
      const p = svRes.products.find((x) => x.product === svPick);
      if (p) body.append(drawSstPlot(p, Math.max(480, body.clientWidth - 20)));
      const key = document.createElement("p");
      key.className = "wx-note sv-key";
      key.innerHTML = `On the chart: <i style="background:${SV_COLD}"></i> product colder · <i style="background:#f7f7f7"></i> right · ` +
        `<i style="background:${SV_WARM}"></i> warmer than the boat (±2 °C at full colour).`;
      if (p) body.append(key);
    },
  });

  return {
    /** Show the meteogram for a chart point. */
    meteogramAt(p) {
      setPoint(p);
      series = null;
      dock.show("meteogram");
      fetchSeries();
    },
    openVerify() { vtimes = null; dock.show("verify"); },
    /** Score the sea temperature products against the boat's own water temperature. */
    openSstCheck() { dock.show("sstcheck"); svLoad(); },
  };
}

/** Each series' TWD as continuous values (may pass 360 or go below 0), placed about a shared unwrapped mean. */
export function unwrapAroundMean(series) {
  const ts = [...new Set(series.flatMap((s) => s.points.filter((p) => p.twd != null).map((p) => p.t)))].sort((a, b) => a - b);
  const mean = new Map();
  let prev = null;
  for (const t of ts) {
    let x = 0, y = 0;
    for (const s of series) {
      const p = s.points.find((q) => q.t === t && q.twd != null);
      if (p) { x += Math.sin((p.twd * Math.PI) / 180); y += Math.cos((p.twd * Math.PI) / 180); }
    }
    let m = (Math.atan2(x, y) * 180) / Math.PI;
    if (Math.hypot(x, y) < 1e-9) m = prev ?? 0; // directions cancel out: keep the line where it was
    prev = prev == null ? (m + 360) % 360 : prev + ang(m, prev);
    mean.set(t, prev);
  }
  // keep the line inside one 360° band round the overall mean direction, so a
  // wind that keeps turning the same way (light airs, sea breezes) cannot drift
  // the axis through several turns; the line breaks where it wraps
  let cx = 0, cy = 0;
  for (const m of mean.values()) { cx += Math.sin((m * Math.PI) / 180); cy += Math.cos((m * Math.PI) / 180); }
  const c = ((Math.atan2(cx, cy) * 180) / Math.PI + 360) % 360;
  for (const [t, m] of mean) mean.set(t, c + ang(m, c));
  // a time a GRIB has but no other: the nearest mean in time
  const near = (t) => mean.get(t) ?? mean.get(ts.reduce((a, b) => (Math.abs(b - t) < Math.abs(a - t) ? b : a), ts[0]));
  return series.map((s) => s.points.map((p) => (p.twd == null ? null : near(p.t) + ang(p.twd, near(p.t)))));
}

/** The meteogram as SVG: the chosen plots stacked over one time axis. Wind speed (gusts dashed, ensemble
 *  range as a band), direction and pressure for each model; waves (height, period on the right, direction
 *  arrows), rain, temperatures and current from the merged GRIBs. Press and drag to move the cursor:
 *  handlers.pick(t) as it moves, handlers.release() when let go. Returns { el, cursor(t), t0, t1 }. */
function drawMeteogram(series, along, plots, W, { pick, release, primary = 0 }) {
  const pad = { l: 46, r: 34 };
  const gap = 20, top = 16, axisH = 22;
  const rows = along?.rows ?? [];
  const has = (p) => (p.src === "models"
    ? series.some((s) => s.points.some((q) => (p.key === "tws" ? q.tws ?? q.gust : q[p.key]) != null))
    : rows.some((r) => (p.key === "waves" ? r.wave : p.key === "cur" ? r.cur?.kn : r[p.key]) != null));
  const heights = plots.map((p) => (has(p) ? p.h : 26));
  const H = top + heights.reduce((a, h) => a + h + gap, 0) + axisH;
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("width", W);
  svg.setAttribute("height", H);
  svg.classList.add("mg");
  const add = (tag, attrs, parent = svg) => {
    const e = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    parent.append(e);
    return e;
  };
  // the time axis: every model's times, and the merged fields' times where they have data
  let t0 = Infinity, t1 = -Infinity;
  if (plots.some((p) => p.src === "models")) for (const s of series) for (const p of s.points) { t0 = Math.min(t0, p.t); t1 = Math.max(t1, p.t); }
  const alongKeys = plots.filter((p) => p.src === "along").map((p) => (p.key === "waves" ? "wave" : p.key));
  for (const r of rows) if (alongKeys.some((k) => (k === "cur" ? r.cur?.kn : r[k]) != null)) { t0 = Math.min(t0, r.t); t1 = Math.max(t1, r.t); }
  if (!Number.isFinite(t0)) for (const s of series) for (const p of s.points) { t0 = Math.min(t0, p.t); t1 = Math.max(t1, p.t); }
  if (!Number.isFinite(t0)) { t0 = rows[0]?.t ?? 0; t1 = rows.at(-1)?.t ?? 3600; }
  if (t1 <= t0) t1 = t0 + 3600;
  const x = (t) => pad.l + ((t - t0) / (t1 - t0)) * (W - pad.l - pad.r);
  const dirs = unwrapAroundMean(series);
  const val = (r, k) => (k === "cur" ? r.cur?.kn ?? null : r[k] ?? null);
  const nearest = (pts, t) => pts.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a), pts[0]);
  // arrows along the top of a plot: pointing where the wind or waves go
  const arrows = (g, y, pts, get, col) => {
    let lastX = -1e9;
    for (const p of pts) {
      const d = get(p);
      if (d == null || p.t < t0 || p.t > t1 || x(p.t) - lastX < 26) continue;
      lastX = x(p.t);
      const a = ((d + 180) % 360) * Math.PI / 180, cx = x(p.t), L = 7;
      const dx = Math.sin(a) * L, dy = -Math.cos(a) * L;
      add("line", { x1: cx - dx, y1: y - dy, x2: cx + dx, y2: y + dy, stroke: col, "stroke-width": 1.6 }, g);
      add("circle", { cx: cx + dx, cy: y + dy, r: 1.8, fill: col }, g);
    }
  };
  const readers = []; // what the cursor reads out: { y0, h, at(t) => [{ v, y, col, unit, above }] }
  const bands = [];
  let y0 = top;
  plots.forEach((pn, n) => {
    const h = heights[n];
    const g = add("g", {});
    bands.push({ y0, h });
    add("rect", { x: pad.l, y: y0, width: W - pad.l - pad.r, height: h, class: "mg-bg" }, g);
    add("text", { x: pad.l, y: y0 - 4, class: "mg-title" }, g).textContent = pn.title;
    if (!has(pn)) {
      add("text", { x: pad.l + 8, y: y0 + h / 2 + 4, class: "mg-tick" }, g).textContent = pn.src === "models" ? "not in the loaded GRIBs" : `not in the loaded GRIBs at this point${pn.key === "waves" ? ": load a wave GRIB (GFS-Wave, ECMWF wave)" : ""}`;
      y0 += h + gap;
      return;
    }
    const vals = [];
    if (pn.src === "models") series.forEach((s, k) => s.points.forEach((p, i) => {
      const v = pn.key === "twd" ? dirs[k][i] : p[pn.key];
      if (v != null) vals.push(v);
      if (pn.key === "tws") { if (p.gust != null) vals.push(p.gust); if (p.hi != null) vals.push(p.hi, p.lo); }
    }));
    else for (const r of rows) { const v = val(r, pn.key === "waves" ? "wave" : pn.key); if (v != null) vals.push(v); }
    let lo = ["tws", "waves", "rain", "cur"].includes(pn.key) ? 0 : Math.min(...vals), hi = Math.max(...vals);
    let step;
    if (pn.key === "twd") step = hi - lo > 180 ? 90 : hi - lo > 60 ? 45 : 15;
    else if (pn.key === "mslp") step = hi - lo > 20 ? 10 : hi - lo > 8 ? 4 : 2;
    else { const raw = (hi - lo) / 4 || 1, p10 = 10 ** Math.floor(Math.log10(raw)); step = [1, 2, 2.5, 5, 10].map((m) => m * p10).find((s) => s >= raw); }
    lo = Math.floor(lo / step) * step;
    hi = Math.max(lo + step, Math.ceil(hi / step) * step);
    const room = pn.key === "tws" || pn.key === "waves" ? 18 : 0; // the arrows
    const base = y0;
    const yv = (v) => base + h - ((v - lo) / (hi - lo)) * (h - room);
    for (let v = lo; v <= hi + 1e-9; v += step) {
      add("line", { x1: pad.l, x2: W - pad.r, y1: yv(v), y2: yv(v), class: "mg-grid" }, g);
      const txt = pn.key === "twd" ? String(((Math.round(v) % 360) + 360) % 360).padStart(3, "0") : String(+v.toFixed(2));
      add("text", { x: pad.l - 4, y: yv(v) + 4, class: "mg-tick", "text-anchor": "end" }, g).textContent = txt;
    }
    const line = (pts, get, attrs, wrap = false) => {
      let d = "", pen = false, last = null;
      pts.forEach((p, i) => {
        const v = get(p, i);
        if (v == null) { pen = false; return; }
        if (wrap && last != null && Math.abs(v - last) > 180) pen = false;
        last = v;
        d += `${pen ? "L" : "M"}${x(p.t).toFixed(1)},${yv(v).toFixed(1)}`;
        pen = true;
      });
      add("path", { d, fill: "none", ...attrs }, g);
    };
    if (pn.src === "models") {
      series.forEach((s, k) => {
        const col = COLOURS[k % COLOURS.length];
        const pts = s.points;
        if (pn.key === "tws" && pts.some((p) => p.hi != null)) {
          const up = pts.filter((p) => p.hi != null).map((p) => `${x(p.t)},${yv(p.hi)}`);
          const dn = pts.filter((p) => p.lo != null).reverse().map((p) => `${x(p.t)},${yv(p.lo)}`);
          add("polygon", { points: [...up, ...dn].join(" "), fill: col, "fill-opacity": 0.15, stroke: "none" }, g);
        }
        if (pn.key === "tws" && pts.some((p) => p.gust != null)) line(pts, (p) => p.gust, { stroke: col, "stroke-width": 1, "stroke-dasharray": "3 3", opacity: 0.8 });
        line(pts, (p, i) => (pn.key === "twd" ? dirs[k][i] : p[pn.key]), { stroke: col, "stroke-width": 1.8 }, pn.key === "twd");
      });
      // direction arrows along the top, as on Weather on route: the shown GRIB's wind (else the first), in its colour
      if (pn.key === "tws" && series[primary]) arrows(g, base + 10, series[primary].points, (p) => p.twd, series.length > 1 ? COLOURS[primary % COLOURS.length] : "#00274d");
      const unit = pn.key === "tws" ? "kn" : pn.key === "twd" ? "°" : "hPa";
      readers.push({ y0: base, h, at: (t) => series.map((s, k) => {
        const i = s.points.indexOf(nearest(s.points, t));
        const p = s.points[i];
        const v = pn.key === "twd" ? dirs[k][i] : p?.[pn.key];
        if (v == null || Math.abs(p.t - t) > 6 * 3600) return null;
        const shownV = pn.key === "twd" ? ((Math.round(v) % 360) + 360) % 360 : v;
        return { v: shownV, y: yv(v), col: COLOURS[k % COLOURS.length], unit, above: true, d: pn.key === "twd" ? 0 : null };
      }).filter(Boolean) });
    } else if (pn.key === "waves") {
      line(rows, (r) => r.wave, { stroke: "#2a62c9", "stroke-width": 2 });
      const ps = rows.map((r) => r.period).filter((v) => v != null);
      let yp = null;
      if (ps.length) {
        const ph = Math.max(4, Math.ceil(Math.max(...ps) / 4) * 4);
        yp = (v) => base + h - (v / ph) * (h - room);
        let d = "", pen = false;
        for (const r of rows) { if (r.period == null) { pen = false; continue; } d += `${pen ? "L" : "M"}${x(r.t).toFixed(1)},${yp(r.period).toFixed(1)}`; pen = true; }
        add("path", { d, stroke: "#8a96a0", fill: "none", "stroke-width": 1.3, "stroke-dasharray": "2 2" }, g);
        for (const v of [0, ph / 2, ph]) add("text", { x: W - pad.r + 4, y: yp(v) + 4, class: "mg-tick" }, g).textContent = `${v} s`;
      }
      arrows(g, base + 10, rows, (r) => r.wavedir, "#2a62c9");
      readers.push({ y0: base, h, at: (t) => {
        const r = nearest(rows, t);
        if (!r) return [];
        return [r.wave != null && { v: r.wave, y: yv(r.wave), col: "#2a62c9", unit: "m", above: true },
          r.period != null && yp && { v: r.period, y: yp(r.period), col: "#5d6b75", unit: "s", above: false }].filter(Boolean);
      } });
    } else {
      const k = pn.key;
      if (pn.bars) {
        const bw = Math.max(1, (W - pad.l - pad.r) / Math.max(1, rows.length) - 0.5);
        for (const r of rows) { const v = val(r, k); if (!v || r.t < t0 || r.t > t1) continue; add("rect", { x: x(r.t) - bw / 2, y: yv(v), width: bw, height: yv(lo) - yv(v), fill: pn.col, opacity: 0.7 }, g); }
      } else line(rows, (r) => val(r, k), { stroke: pn.col, "stroke-width": 1.8 });
      if (k === "cur") arrows(g, base + 10, rows, (r) => (r.cur?.set == null ? null : (r.cur.set + 180) % 360), pn.col); // arrows show where it flows
      readers.push({ y0: base, h, at: (t) => { const r = nearest(rows, t); const v = r && val(r, k); return v == null ? [] : [{ v, y: yv(v), col: pn.col, unit: pn.unit, above: true }]; } });
    }
    y0 += h + gap;
  });
  // the time axis: grid in each plot, labels at local days or hours, never overlapping
  const span = t1 - t0;
  const minor = span > 5 * 86400 ? 21600 : span > 36 * 3600 ? 10800 : 3600;
  for (let t = Math.ceil(t0 / minor) * minor; t <= t1; t += minor) for (const b of bands) add("line", { x1: x(t), x2: x(t), y1: b.y0, y2: b.y0 + b.h, class: "mg-vgrid" });
  const off = -new Date(t0 * 1000).getTimezoneOffset() * 60;
  const fits = (sec) => (sec / span) * (W - pad.l - pad.r) >= 96;
  const lab = [3600, 10800, 21600, 43200, 86400, 2 * 86400, 4 * 86400, 7 * 86400].find(fits) ?? 7 * 86400;
  const labelAt = (t) => {
    const d = new Date(t * 1000);
    if (lab >= 86400 || (((t + off) % 86400) + 86400) % 86400 === 0) return d.toLocaleDateString("en-NZ", { weekday: "short", day: "numeric", month: "short" });
    return d.toLocaleTimeString("en-NZ", { hour: "2-digit", minute: "2-digit", hour12: false });
  };
  const unitL = Math.min(lab, 86400);
  for (let t = Math.ceil((t0 + off) / unitL) * unitL - off; t <= t1; t += lab) {
    add("text", { x: x(t), y: y0 - gap + 15, class: "mg-tick", "text-anchor": "middle" }).textContent = labelAt(t);
  }
  const cur = add("line", { x1: 0, x2: 0, y1: top, y2: y0 - gap, class: "mg-cursor" });
  const marks = add("g", { class: "mg-marks" });
  // press anywhere on the plots and drag: the cursor follows (to the quarter hour)
  const at = (e) => { const r = svg.getBoundingClientRect(); return t0 + ((e.clientX - r.left - pad.l) / (W - pad.l - pad.r)) * (t1 - t0); };
  let last = null;
  const follow = (e) => {
    const t = Math.max(t0, Math.min(t1, Math.round(at(e) / 900) * 900));
    if (t !== last) { last = t; pick(t); }
  };
  svg.style.touchAction = "none";
  svg.style.cursor = "ew-resize";
  svg.addEventListener("pointerdown", (e) => {
    if (e.button > 0) return;
    const r = svg.getBoundingClientRect();
    if (e.clientX - r.left < pad.l - 6 || e.clientX - r.left > W - pad.r + 6) return;
    svg.setPointerCapture?.(e.pointerId);
    last = null; follow(e);
    const move = (ev) => follow(ev);
    const up = () => { release?.(); svg.removeEventListener("pointermove", move); svg.removeEventListener("pointerup", up); svg.removeEventListener("pointercancel", up); };
    svg.addEventListener("pointermove", move); svg.addEventListener("pointerup", up); svg.addEventListener("pointercancel", up);
    e.preventDefault();
  });
  return {
    el: svg, t0, t1,
    cursor(t) {
      const show = t != null && t >= t0 && t <= t1;
      cur.style.display = show ? "" : "none";
      marks.replaceChildren();
      if (!show) return;
      const cx = x(t);
      cur.setAttribute("x1", cx); cur.setAttribute("x2", cx);
      for (const rd of readers) {
        const ls = rd.at(t).map((l) => {
          const txt = `${l.unit === "°" ? String(Math.round(l.v)).padStart(3, "0") : +l.v.toFixed(Math.abs(l.v) < 10 ? 1 : 0)}${l.unit === "°" ? "°" : l.unit ? ` ${l.unit}` : ""}`;
          add("circle", { cx, cy: l.y, r: 3.5, fill: l.col, stroke: "#fff", "stroke-width": 1.2 }, marks);
          return { ...l, txt, ty: l.y + (l.above ? -7 : 14) };
        }).sort((p, q) => p.ty - q.ty);
        // values in one plot keep 13 px apart, within the plot
        const lo = rd.y0 + 11, hi = rd.y0 + rd.h - 3;
        ls.forEach((l, i) => { l.ty = Math.max(lo, i ? Math.max(l.ty, ls[i - 1].ty + 13) : l.ty); });
        for (let i = ls.length - 1; i >= 0; i--) ls[i].ty = Math.min(ls[i].ty, i < ls.length - 1 ? ls[i + 1].ty - 13 : hi);
        for (const l of ls) {
          const right = cx + 12 + l.txt.length * 6.2 < W - pad.r;
          add("text", { x: right ? cx + 8 : cx - 8, y: l.ty, class: "mg-val", fill: l.col, "text-anchor": right ? "start" : "end" }, marks).textContent = l.txt;
        }
      }
    },
  };
}


/** The boat's water temperature (black) and one product (blue) at the matched readings, over time; lines break at gaps over 3 h. */
export function drawSstPlot(p, width) {
  const W = width, H = 170, L = 44, R = 10, T = 10, B = 26;
  const pts = p.pairs;
  const ts = pts.map((q) => q[0]);
  const t0 = Math.min(...ts), t1 = Math.max(...ts, t0 + 1);
  const vals = pts.flatMap((q) => [q[1], q[2]]);
  let lo = Math.floor(Math.min(...vals) * 2) / 2, hi = Math.ceil(Math.max(...vals) * 2) / 2;
  if (hi - lo < 1) { lo -= 0.5; hi += 0.5; }
  const x = (t) => L + ((t - t0) / (t1 - t0)) * (W - L - R);
  const y = (v) => T + ((hi - v) / (hi - lo)) * (H - T - B);
  const path = (k) => {
    let d = "", prev = null;
    for (const q of pts) { d += `${prev == null || q[0] - prev > 3 * 3600 ? "M" : "L"}${x(q[0]).toFixed(1)},${y(q[k]).toFixed(1)}`; prev = q[0]; }
    return d;
  };
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("width", W); svg.setAttribute("height", H); svg.setAttribute("class", "sv-plot");
  const step = hi - lo > 6 ? 2 : hi - lo > 3 ? 1 : 0.5;
  let g = "";
  for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9; v += step) g += `<line x1="${L}" x2="${W - R}" y1="${y(v)}" y2="${y(v)}" class="grid"/><text x="${L - 4}" y="${y(v) + 4}" text-anchor="end">${v.toFixed(step < 1 ? 1 : 0)}</text>`;
  const days = Math.max(1, Math.round((t1 - t0) / 86400));
  const every = Math.max(1, Math.ceil(days / Math.max(1, Math.floor((W - L - R) / 70))));
  // local midnights
  const d = new Date(t0 * 1000);
  d.setHours(24, 0, 0, 0);
  for (; d.getTime() / 1000 <= t1; d.setDate(d.getDate() + every)) {
    const t = d.getTime() / 1000;
    g += `<line x1="${x(t)}" x2="${x(t)}" y1="${T}" y2="${H - B}" class="grid"/><text x="${x(t)}" y="${H - 8}" text-anchor="middle">${d.getDate()} ${d.toLocaleString("en-NZ", { month: "short" })}</text>`;
  }
  svg.innerHTML = `<g class="axes">${g}<text x="4" y="${T + 4}">°C</text></g>` +
    `<path d="${path(1)}" fill="none" stroke="currentColor" stroke-width="1.6"/>` +
    `<path d="${path(2)}" fill="none" stroke="#1f77b4" stroke-width="1.6" stroke-dasharray="5 3"/>` +
    `<text x="${W - R}" y="${T + 10}" text-anchor="end" class="lab">— boat   <tspan fill="#1f77b4">- - ${esc(p.product)}</tspan></text>`;
  return svg;
}
