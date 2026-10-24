// GRIB download (Weather drawer > Download): model, parameters, area (drawn on
// the chart, the chart view, or round the course), forecast length and step.
// The Rust side finds the newest complete run on NOAA NOMADS, downloads it cut
// to the area, and the file is loaded as soon as it is written.
import { SAILDOCS, SD_MODELS, SD_PARAM_NAMES, saildocsRequest, saildocsBytes } from "./saildocs.js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const KEY = "racenav.gribdl";
const DEFAULTS = { model: "gfs0p25", params: { gfs: ["wind", "gust", "mslp"], wave: ["waves", "period"], mur_sst: ["sst"], hycom_espc: ["sst", "sss", "current"],
  himawari: ["skin"], chl_viirs: ["chl"] },
  area: null, days: 7, every: 3, depth: 50, himHours: 12, himEvery: 1 };
const OCEAN = ["mur_sst", "hycom_espc", "himawari", "chl_viirs"];
// daily satellite analyses: "the latest N days", no step
const DAILY = ["mur_sst", "chl_viirs"];
/** Which saved parameter list a model uses. */
const paramKey = (id) => (OCEAN.includes(id) ? id : id.includes("wave") ? "wave" : "gfs");

export function setupGribDownload(map, { weather, course }) {
  const $ = (id) => document.getElementById(id);
  let prefs = structuredClone(DEFAULTS);
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  prefs.params = { ...DEFAULTS.params, ...prefs.params };
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  let models = [];
  const status = (t, kind = "") => { const el = $("dl-status"); el.textContent = t; el.className = `wx-note ${kind}`; };
  let sdReady = false; // the Saildocs request is set up (end of this function)
  const model = () => models.find((m) => m.id === prefs.model) ?? models[0];

  // ---------- the area on the chart ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  map.addSource("dl-area", { type: "geojson", data: empty });
  map.addLayer({ id: "dl-area-fill", type: "fill", source: "dl-area", paint: { "fill-color": "#0b5cad", "fill-opacity": 0.06 } });
  map.addLayer({ id: "dl-area-line", type: "line", source: "dl-area", paint: { "line-color": "#0b5cad", "line-width": 2, "line-dasharray": [4, 2] } });
  function showArea() {
    const a = prefs.area;
    const open = $("dl-box").open && !$("drawer").hidden;
    map.getSource("dl-area").setData(a && open ? { type: "Feature", properties: {}, geometry: { type: "Polygon",
      coordinates: [[[a[0], a[1]], [a[2], a[1]], [a[2], a[3]], [a[0], a[3]], [a[0], a[1]]]] } } : empty);
    for (const [id, i] of [["dl-w", 0], ["dl-s", 1], ["dl-e", 2], ["dl-n", 3]]) $(id).value = a ? String(+a[i].toFixed(2)) : "";
    if (sdReady) sdUpdate();
  }
  function setArea(a) {
    if (!a) return;
    // tidy: whole tenths, south < north, and east after west (may pass 180 to cross the date line)
    let [w, s, e, n] = a.map((x) => Math.round(x * 10) / 10);
    if (s > n) [s, n] = [n, s];
    s = Math.max(-90, s); n = Math.min(90, n);
    if (e <= w) e += 360;
    prefs.area = [w, s, e, n];
    save();
    showArea();
    estimate();
  }
  $("dl-box").addEventListener("toggle", showArea);
  new MutationObserver(showArea).observe($("drawer"), { attributes: true, attributeFilter: ["hidden"] });

  $("dl-view").onclick = () => {
    const b = map.getBounds();
    setArea([b.getWest(), b.getSouth(), b.getEast(), b.getNorth()]);
  };
  $("dl-course").onclick = () => {
    const c = course.get();
    const pts = [c.start, ...c.marks.map((m) => [m.lon, m.lat])].filter(Boolean);
    if (!pts.length) { status("Set a course first (Course panel).", "warn"); return; }
    // keep longitudes continuous so a course across the date line gives a small box
    const lons = [];
    for (const p of pts) { let x = p[0]; if (lons.length) { while (x - lons.at(-1) > 180) x -= 360; while (x - lons.at(-1) < -180) x += 360; } lons.push(x); }
    const lats = pts.map((p) => p[1]);
    const span = Math.max(Math.max(...lons) - Math.min(...lons), Math.max(...lats) - Math.min(...lats));
    const m = Math.max(1, Math.min(10, span * 0.3));
    setArea([Math.min(...lons) - m, Math.min(...lats) - m, Math.max(...lons) + m, Math.max(...lats) + m]);
    fitArea();
  };
  for (const id of ["dl-w", "dl-s", "dl-e", "dl-n"]) {
    $(id).onchange = () => {
      const v = ["dl-w", "dl-s", "dl-e", "dl-n"].map((k) => parseFloat($(k).value));
      if (v.every(Number.isFinite)) setArea(v); else status("Type the four edges in degrees (south and west negative).", "warn");
    };
  }
  // draw: drag a rectangle
  let drawing = false, from = null;
  $("dl-draw").onclick = () => {
    drawing = !drawing;
    $("dl-draw").classList.toggle("armed", drawing);
    map.getCanvas().style.cursor = drawing ? "crosshair" : "";
    map.getContainer().classList.toggle("drawing", drawing);
    if (drawing) { map.dragPan.disable(); status("Drag a rectangle on the chart."); } else map.dragPan.enable();
  };
  map.on("mousedown", (e) => { if (drawing) { from = e.lngLat; e.preventDefault(); } });
  let to = null;
  map.on("mousemove", (e) => {
    if (!drawing || !from) return;
    to = e.lngLat;
    prefs.area = [Math.min(from.lng, to.lng), Math.min(from.lat, to.lat), Math.max(from.lng, to.lng), Math.max(from.lat, to.lat)];
    showArea();
  });
  // finish on mouse up anywhere (it may be released over a panel)
  window.addEventListener("mouseup", () => {
    if (!drawing || !from || !to) return;
    const a = [Math.min(from.lng, to.lng), Math.min(from.lat, to.lat), Math.max(from.lng, to.lng), Math.max(from.lat, to.lat)];
    to = null;
    from = null;
    drawing = false;
    $("dl-draw").classList.remove("armed");
    map.getCanvas().style.cursor = "";
    map.getContainer().classList.remove("drawing");
    map.dragPan.enable();
    if (a[2] - a[0] < 0.2 || a[3] - a[1] < 0.2) { status("That area is too small: drag a larger rectangle.", "warn"); showArea(); return; }
    setArea(a);
    status("");
  });
  function fitArea() {
    const a = prefs.area;
    if (a) map.fitBounds([[a[0], a[1]], [a[2], a[3]]], { padding: { top: 60, bottom: 80, left: 440, right: 80 }, duration: 500 });
  }

  // ---------- model, parameters, time ----------------------------------------------------------
  function renderParams() {
    const m = model();
    const key = paramKey(m.id);
    const mur = DAILY.includes(m.id), hycom = m.id === "hycom_espc", him = m.id === "himawari";
    const box = $("dl-params");
    box.innerHTML = "";
    for (const p of m.params) {
      const lab = document.createElement("label");
      lab.innerHTML = `<input type="checkbox"> <span></span>`;
      lab.querySelector("span").textContent = p.name;
      const cb = lab.querySelector("input");
      cb.checked = prefs.params[key].includes(p.id);
      cb.onchange = () => {
        const s = new Set(prefs.params[key]);
        if (cb.checked) s.add(p.id); else s.delete(p.id);
        prefs.params[key] = [...s];
        save(); estimate(); showDepth();
      };
      box.append(lab);
    }
    const days = $("dl-days");
    days.innerHTML = "";
    // MUR: how many of the latest daily analyses; the others: forecast length
    days.previousElementSibling.textContent = him ? "Hours back" : mur ? "Days of analyses" : "Forecast length";
    if (him) {
      // Himawari: the last N hours (values in hours)
      for (const h of [3, 6, 12, 24, 48, 72]) days.append(new Option(`last ${h} hours`, String(h)));
      days.value = String(prefs.himHours ?? 12);
    } else {
      for (const d of (mur ? [1, 2, 3, 5, 7, 14] : [1, 2, 3, 5, 7, 8, 10, 16])) if (d * 24 <= m.max_hours) days.append(new Option(mur ? `last ${d} day${d > 1 ? "s" : ""}` : `${d} day${d > 1 ? "s" : ""}`, String(d)));
      days.value = String(Math.min(mur ? Math.min(prefs.days, 3) : prefs.days, 16));
    }
    if (!days.value) days.value = days.options[days.options.length - 1]?.value ?? "1";
    $("dl-every-row").hidden = mur;
    $("dl-depth").value = String(prefs.depth ?? 50);
    showDepth();
    void hycom;
    const ev = $("dl-every");
    ev.innerHTML = "";
    const fine = m.steps[0][1];
    for (const h of (him ? [1, 2, 3, 6] : [1, 3, 6, 12])) if (h >= fine) ev.append(new Option(h === 1 ? "1 hour" : `${h} hours`, String(h)));
    ev.value = String(him ? prefs.himEvery ?? 1 : Math.max(prefs.every, fine));
    if (!ev.value) ev.value = String(fine);
  }
  function showDepth() {
    const m = model();
    $("dl-depth-row").hidden = !(m?.id === "hycom_espc" && prefs.params.hycom_espc.includes("depth"));
  }
  $("dl-depth").onchange = (e) => { prefs.depth = Number(e.target.value); save(); };
  function request() {
    const m = model();
    const key = paramKey(m.id);
    const him = m.id === "himawari";
    return { model: m.id, area: prefs.area, params: prefs.params[key], hours: Number($("dl-days").value) * (him ? 1 : 24),
      every: DAILY.includes(m.id) ? 24 : Number($("dl-every").value), depth: Number($("dl-depth").value) || 50 };
  }
  async function estimate() {
    if (!models.length) return;
    if (!prefs.area) { $("dl-size").textContent = "Choose an area: draw it, use the chart view or the course."; return; }
    try {
      const r = await invoke("gribdl_estimate", { req: request() });
      const size = (b) => { const mb = b / 1e6; return mb < 1 ? `${Math.max(1, Math.round(b / 1000))} kB` : `${mb.toFixed(mb < 10 ? 1 : 0)} MB`; };
      const ecmwf = request().model.startsWith("ecmwf");
      const mur = DAILY.includes(request().model);
      const hycom = request().model === "hycom_espc";
      const him = request().model === "himawari";
      const reqs = Math.ceil(Number($("dl-days").value)) * (request().params.filter((p) => p !== "depth").length ? 1 : 0) + (request().params.includes("depth") ? Math.ceil(Number($("dl-days").value)) : 0);
      const hrs = Math.floor(request().hours / request().every);
      $("dl-size").textContent = him
        ? `About ${size(r.bytes)} if clear · up to ${hrs} hourly view${hrs === 1 ? "" : "s"} (one request each; under cloud there is no value, and each hour keeps the last clear value seen)`
        : hycom
        ? `About ${size(r.bytes)} · ${r.steps} times to +${r.last} h · ${reqs} request${reqs === 1 ? "" : "s"} (one a day; HYCOM's server takes from a few seconds to a minute or two for each)`
        : mur
        ? `About ${size(r.bytes)} · the latest ${Number($("dl-days").value)} daily analys${Number($("dl-days").value) > 1 ? "es" : "is"}${r.bytes > 20e6 && request().model === "mur_sst" ? " · a big area: it comes down thinned (MUR is 1 km)" : ""}`
        : ecmwf
        ? `File about ${size(r.bytes)}; downloads about ${size(r.download)} (ECMWF publishes whole-globe fields, cut to the area here) · ${r.steps} forecast times to +${r.last} h` +
          (r.last > 144 && !request().model.includes("aifs") ? " · only the 00Z and 12Z runs go past 144 h" : "")
        : `About ${size(r.bytes)} · ${r.steps} forecast times to +${r.last} h`;
    } catch (e) { $("dl-size").textContent = String(e); }
  }
  const SOURCE_NOTE = {
    gfs: "Source: NOAA NOMADS (GFS, GFS-Wave), cut to the area on the server. The newest complete run is used.",
    ecmwf: "Source: ECMWF open data (CC BY 4.0, © ECMWF), from data.ecmwf.int or its AWS mirror. The free 0.1° IFS (~9 km) data arrives about two hours after the 0.25° products; ECMWF does not document the surface currents as tidal-stream predictions. The newest complete run is used.",
    mur_sst: "Source: NASA JPL MUR SST (free to use and share) via NOAA CoastWatch ERDDAP: a daily 1 km analysis from all the satellites, without cloud gaps, usually a day or two old. Shown until four days after its date.",
    himawari: "Source: Himawari-9 (JMA) AHI, processed by NOAA STAR (ACSPO L3C, GHRSST: free and open) via NOAA CoastWatch: hourly, about 2 km, clear sky only (best-quality pixels), two or three hours after the hour. Cloud leaves gaps; each hour keeps the last clear value seen in the hours downloaded.",
    chl_viirs: "Source: NOAA CoastWatch chlorophyll-a from the VIIRS satellites (S-NPP, NOAA-20), gap-filled (DINEOF), daily at about 9 km (free to use and share). Near real time, about three days old. Shown until six days after its date.",
    hycom_espc: "Source: US Navy HYCOM ESPC global ocean model (1/12°, about 9 km) via hycom.org: from the latest analysis out to 8 days. A demonstration service with no guarantee it is up. An area across 180° isn't offered yet.",
  };
  const showSource = () => status(SOURCE_NOTE[OCEAN.includes(prefs.model) ? prefs.model : String(prefs.model).startsWith("ecmwf") ? "ecmwf" : "gfs"]);
  $("dl-model").onchange = (e) => { prefs.model = e.target.value; save(); renderParams(); estimate(); showSource(); };
  $("dl-days").onchange = (e) => { if (prefs.model === "himawari") prefs.himHours = Number(e.target.value); else prefs.days = Number(e.target.value); save(); estimate(); };
  $("dl-every").onchange = (e) => { if (prefs.model === "himawari") prefs.himEvery = Number(e.target.value); else prefs.every = Number(e.target.value); save(); estimate(); };

  // ---------- download ----------------------------------------------------------
  const prog = $("dl-progress");
  listen("gribdl-progress", ({ payload: p }) => {
    prog.hidden = false;
    const pc = p.total ? Math.round((100 * p.done) / p.total) : 2;
    prog.querySelector(".bar").style.width = `${pc}%`;
    prog.querySelector("span").textContent = p.total ? `${p.stage} · ${p.done}/${p.total} · ${(p.bytes / 1e6).toFixed(1)} MB` : `${p.stage}…`;
  }).catch(() => {});
  $("dl-go").onclick = async () => {
    const req = request();
    if (!req.area) { status("Choose an area first.", "warn"); return; }
    if (!req.params.length) { status("Tick at least one parameter.", "warn"); return; }
    $("dl-go").disabled = true;
    $("dl-cancel").disabled = false;
    status("Downloading…");
    const t0 = performance.now();
    try {
      const r = await invoke("gribdl_start", { req });
      status(`Downloaded ${r.name} in ${((performance.now() - t0) / 1000).toFixed(0)} s. Loading…`, "good");
      await weather.refreshFiles();
      await weather.load(r.path);
      const file = r.path.split(/[\\/]/).pop();
      if (r.note) status(`${r.name}: downloaded and loaded (${file}). ${r.note}`, "warn");
      else status(`${r.name}: downloaded and loaded (${file}).`, "good");
    } catch (e) {
      status(String(e) === "cancelled" ? "Download cancelled." : `Download failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally {
      $("dl-go").disabled = false;
      $("dl-cancel").disabled = true;
      prog.hidden = true;
    }
  };
  $("dl-cancel").onclick = () => invoke("gribdl_cancel").catch(() => {});

  // ---------- Saildocs email request ----------------------------------------------------------
  prefs.sd ??= { model: "GFS", res: {}, params: {} };
  const sdSel = $("sd-model"), sdRes = $("sd-res");
  for (const [k, m] of Object.entries(SD_MODELS)) sdSel.append(new Option(m.name, k));
  sdSel.value = SD_MODELS[prefs.sd.model] ? prefs.sd.model : "GFS";
  function sdRender() {
    const m = SD_MODELS[sdSel.value];
    sdRes.innerHTML = "";
    for (const r of m.res) sdRes.append(new Option(`${r}°`, String(r)));
    sdRes.value = String(prefs.sd.res[sdSel.value] ?? m.res[0]);
    const box = $("sd-params");
    box.innerHTML = "";
    const on = prefs.sd.params[sdSel.value] ?? m.def;
    for (const p of m.params) {
      const lab = document.createElement("label");
      lab.innerHTML = `<input type="checkbox" value="${p}"> ${SD_PARAM_NAMES[p] ?? p}`;
      lab.querySelector("input").checked = on.includes(p);
      box.append(lab);
    }
    sdUpdate();
  }
  function sdReq() {
    const params = [...$("sd-params").querySelectorAll("input:checked")].map((x) => x.value);
    return { model: sdSel.value, area: prefs.area, res: Number(sdRes.value), hours: Number($("dl-days").value) * 24, every: Math.max(3, Number($("dl-every").value)), params };
  }
  function sdUpdate() {
    const r = sdReq();
    const ok = !!r.area && r.params.length > 0;
    $("sd-text").value = ok ? saildocsRequest(r) : r.area ? "Choose at least one parameter." : "Choose an area above.";
    $("sd-copy").disabled = $("sd-mail").disabled = !ok;
    if (!ok) { $("sd-size").textContent = ""; return; }
    const kb = saildocsBytes(r) / 1000;
    const hrs = Math.min(r.hours, SD_MODELS[r.model].max);
    $("sd-size").textContent = `Reply about ${kb < 1000 ? `${Math.max(1, Math.round(kb))} kB` : `${(kb / 1000).toFixed(1)} MB`}` +
      (hrs < r.hours ? ` · ${r.model} goes to ${hrs} h` : "");
    $("sd-size").classList.toggle("warn", kb > 500);
  }
  sdSel.onchange = () => { prefs.sd.model = sdSel.value; save(); sdRender(); };
  sdRes.onchange = () => { prefs.sd.res[sdSel.value] = Number(sdRes.value); save(); sdUpdate(); };
  $("sd-params").onchange = () => { prefs.sd.params[sdSel.value] = sdReq().params; save(); sdUpdate(); };
  for (const id of ["dl-days", "dl-every", "dl-n", "dl-s", "dl-w", "dl-e"]) $(id).addEventListener("change", sdUpdate);
  $("sd-copy").onclick = async () => {
    try { await navigator.clipboard.writeText($("sd-text").value); $("sd-copy").textContent = "Copied"; } catch { $("sd-text").select(); }
    setTimeout(() => { $("sd-copy").textContent = "Copy"; }, 1500);
  };
  $("sd-mail").onclick = () => {
    const url = `mailto:${SAILDOCS}?subject=${encodeURIComponent("GRIB request")}&body=${encodeURIComponent($("sd-text").value)}`;
    invoke("open_url", { url }).catch(() => { window.location.href = url; });
  };
  sdReady = true;
  sdRender();

  (async () => {
    models = (await invoke("gribdl_models").catch(() => [])) || [];
    const sel = $("dl-model");
    for (const m of models) sel.append(new Option(m.name, m.id));
    if (!models.some((m) => m.id === prefs.model)) prefs.model = models[0]?.id ?? "gfs0p25";
    showSource();
    sel.value = prefs.model;
    if (models.length) renderParams();
    showArea();
    estimate();
  })();
}
