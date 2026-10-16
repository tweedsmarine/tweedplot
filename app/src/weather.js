// Weather panel: GRIB files, time slider, wind barbs and speed shading,
// and a table comparing every loaded run at the cursor.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Wind speed shading (knots).
// Palette sampled every 2 kn so light-air differences show too.
const PALETTE = [
  [0, "#e8f4ff"], [2, "#c4e2ff"], [4, "#9fd0f7"], [6, "#79bdec"], [8, "#5ab9cf"], [10, "#5cc6a3"],
  [12, "#7fd27a"], [14, "#b3dc5c"], [16, "#e2e04f"], [18, "#f7cd45"], [20, "#f7aa3b"], [22, "#f38536"],
  [25, "#e85c3a"], [28, "#d6404f"], [32, "#b8357a"], [36, "#9334a3"], [40, "#6c2fae"], [50, "#45247f"],
];

// Other quantities: value stops and colours.
const WAVES = [[0, "#eef6fb"], [0.5, "#cfe6f5"], [1, "#a6d2ec"], [1.5, "#7fbde0"], [2, "#5aa8d3"], [3, "#5cc6a3"], [4, "#b3dc5c"],
  [5, "#f7cd45"], [6, "#f38536"], [8, "#d6404f"], [10, "#9334a3"], [14, "#45247f"]];
const PERIOD = [[0, "#eef6fb"], [4, "#cfe6f5"], [6, "#a6d2ec"], [8, "#7fbde0"], [10, "#5cc6a3"], [12, "#b3dc5c"], [14, "#f7cd45"],
  [16, "#f38536"], [18, "#d6404f"], [22, "#9334a3"]];
const RAIN = [[0.1, "#d7ecf7"], [0.5, "#a6d2ec"], [1, "#6fb3dc"], [2, "#3f8fcf"], [4, "#5cc6a3"], [8, "#f7cd45"], [16, "#f38536"],
  [32, "#d6404f"], [64, "#9334a3"]];
const CAPE = [[100, "#f4f0c0"], [250, "#f7e08a"], [500, "#f7cd45"], [1000, "#f7aa3b"], [1500, "#f38536"], [2000, "#e85c3a"],
  [3000, "#d6404f"], [4000, "#9334a3"]];
const PRESS = [[960, "#45247f"], [980, "#6c2fae"], [995, "#5aa8d3"], [1005, "#a6d2ec"], [1013, "#eef6fb"], [1020, "#f7e08a"],
  [1030, "#f7aa3b"], [1040, "#d6404f"]];

/** What can be shaded: the GRIB parameters it needs, unit, colour scale, and values below `min` left clear. */
export const LAYERS = {
  wind: { label: "Wind speed", unit: "kn", params: ["WindU", "WindSpeed"], scale: PALETTE, knots: true },
  gust: { label: "Gusts", unit: "kn", params: ["Gust"], scale: PALETTE, knots: true },
  waves: { label: "Wave height (Hs)", unit: "m", params: ["WaveHeight"], scale: WAVES },
  swell: { label: "Swell height", unit: "m", params: ["SwellHeight"], scale: WAVES },
  period: { label: "Wave period", unit: "s", params: ["WavePeriod"], scale: PERIOD },
  rain: { label: "Rain", unit: "mm/h", params: ["Rain"], scale: RAIN, min: 0.1 },
  cape: { label: "CAPE (squalls)", unit: "J/kg", params: ["Cape"], scale: CAPE, min: 100 },
  mslp: { label: "Pressure", unit: "hPa", params: ["Mslp"], scale: PRESS },
};

// Display settings (per viewer)
const WX_DEFAULTS = { density: 5, size: 1.0, shading: "2", opacity: 0.45, land: "land", layer: "wind", arrows: true, waveArrows: true, isobars: false };
const DENSITY_POINTS = [80, 150, 300, 500, 900, 1400, 2200, 3500, 6000, 1e9];
function loadWx() {
  try { return { ...WX_DEFAULTS, ...JSON.parse(localStorage.getItem("racenav.wx") || "{}") }; } catch { return { ...WX_DEFAULTS }; }
}

/** Colour of a value on a scale (linear between stops); the wind palette by default. */
function paletteAt(kn, scale = PALETTE) {
  if (kn <= scale[0][0]) return scale[0][1];
  for (let i = 1; i < scale.length; i++) {
    const [k1, c1] = scale[i - 1], [k2, c2] = scale[i];
    if (kn <= k2) {
      const f = Math.max(0, (kn - k1) / (k2 - k1));
      const a = [1, 3, 5].map((o) => parseInt(c1.slice(o, o + 2), 16));
      const b = [1, 3, 5].map((o) => parseInt(c2.slice(o, o + 2), 16));
      return "#" + a.map((v, j) => Math.round(v + (b[j] - v) * f).toString(16).padStart(2, "0")).join("");
    }
  }
  return scale.at(-1)[1];
}

/** fill-color expression: smooth, or bands (of `mode` knots for wind; between the scale's stops otherwise). */
function shadingExpression(mode, layer = LAYERS.wind) {
  const scale = layer.scale;
  if (mode === "smooth") return ["interpolate", ["linear"], ["get", "v"], ...scale.flat()];
  if (layer.knots) {
    const step = Number(mode);
    const expr = ["step", ["get", "v"], paletteAt(step / 2)];
    for (let k = step; k <= 50; k += step) expr.push(k, paletteAt(k + step / 2));
    return expr;
  }
  const expr = ["step", ["get", "v"], scale[0][1]];
  for (let i = 1; i < scale.length; i++) expr.push(scale[i][0], paletteAt((scale[i][0] + (scale[i + 1]?.[0] ?? scale[i][0] * 1.2)) / 2, scale));
  return expr;
}

/**
 * `layers.landTop`: first land fill layer (shading goes under it for "land on top");
 * `layers.coastTop`: first coastline layer (shading goes under it for "coastline on top");
 * `layers.top`: barbs go under this one.
 */
export function setupWeather(map, layers) {
  const beforeTop = layers.top;
  const shadingBefore = () => (wx.land === "coast" ? layers.coastTop : layers.landTop);
  const $ = (id) => document.getElementById(id);
  const loaded = new Map(); // path -> summary
  let display = null; // path shown on the chart
  let times = [];
  let t = 0;
  let playing = null;
  let cursor = null; // [lon, lat]
  const wx = loadWx();
  const timeListeners = [];

  map.addSource("wx-cells", { type: "geojson", data: fc([]) });
  map.addSource("wx-barbs", { type: "geojson", data: fc([]) });
  map.addLayer({
    id: "wx-cells", type: "fill", source: "wx-cells",
    paint: { "fill-color": shadingExpression(wx.shading === "off" ? "2" : wx.shading, LAYERS[wx.layer] ?? LAYERS.wind), "fill-opacity": wx.opacity, "fill-antialias": false },
    layout: { visibility: wx.shading === "off" ? "none" : "visible" },
  }, shadingBefore());
  // isobars and wave-direction arrows
  map.addSource("wx-isobars", { type: "geojson", data: fc([]) });
  map.addSource("wx-wavedir", { type: "geojson", data: fc([]) });
  map.addLayer({ id: "wx-isobars", type: "line", source: "wx-isobars",
    paint: { "line-color": "#3d4f5c", "line-width": ["case", ["==", ["%", ["get", "hpa"], 8], 0], 1.6, 0.9], "line-opacity": 0.85 } }, beforeTop);
  map.addLayer({ id: "wx-isobar-labels", type: "symbol", source: "wx-isobars", filter: ["==", ["geometry-type"], "Point"],
    layout: { "icon-image": ["concat", "lbl:", ["to-string", ["get", "hpa"]]], "icon-allow-overlap": false, "icon-size": 0.85 } }, beforeTop);
  if (!map.hasImage("wave-arrow")) map.addImage("wave-arrow", waveArrow(), { pixelRatio: 2 });
  map.addLayer({ id: "wx-wavedir", type: "symbol", source: "wx-wavedir",
    layout: { "icon-image": "wave-arrow", "icon-rotate": ["get", "dir"], "icon-rotation-alignment": "map",
              "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-size": wx.size } }, beforeTop);
  map.addLayer({
    id: "wx-barbs", type: "symbol", source: "wx-barbs",
    layout: {
      "icon-image": ["get", "icon"], "icon-rotate": ["get", "twd"], "icon-rotation-alignment": "map",
      "icon-anchor": "bottom", "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-size": wx.size,
    },
  }, beforeTop);

  // ---------- files ----------------------------------------------------------
  async function refreshFiles() {
    const files = await invoke("grib_files").catch(() => []);
    const sel = $("wx-files");
    sel.innerHTML = "";
    if (!files.length) sel.append(new Option("No GRIB files found", ""));
    for (const f of files) {
      const mb = f.size ? `${(f.size / 1e6).toFixed(f.size < 1e7 ? 1 : 0)} MB` : "";
      const fetched = f.mtime ? `fetched ${fmtLocal(f.mtime)}` : "";
      const desc = f.info ? f.info.label : f.file;
      const opt = new Option(`${f.loaded ? "✓ " : ""}${desc} · ${fetched} · ${mb}`, f.path);
      opt.title = `${f.file}\n${f.dir}`;
      sel.append(opt);
    }
  }

  // Loading progress (the Rust side reports while it reads and decodes).
  const prog = $("wx-progress");
  let loadingPath = null;
  function showProgress(pc, stage) {
    prog.hidden = false;
    prog.querySelector(".bar").style.width = `${pc}%`;
    prog.querySelector("span").textContent = `${stage === "reading" ? "Reading" : stage === "indexing" ? "Indexing" : "Decoding"} … ${pc} %`;
  }
  listen("grib-progress", ({ payload }) => { if (payload.path === loadingPath) showProgress(payload.percent, payload.stage); }).catch(() => {});

  async function load(path) {
    if (!path) return;
    $("wx-load").disabled = true;
    $("wx-load").textContent = "…";
    loadingPath = path;
    showProgress(0, "reading");
    const t0 = performance.now();
    try {
      const s = await invoke("grib_load", { path });
      loaded.set(path, s);
      if (!display) display = path;
      renderLoaded();
      setTimes();
      alertLine(`Loaded ${s.name} in ${((performance.now() - t0) / 1000).toFixed(1)} s`);
      await refreshFiles();
    } catch (e) {
      alertLine(`Could not read GRIB: ${e}`);
    } finally {
      loadingPath = null;
      prog.hidden = true;
      $("wx-load").disabled = false;
      $("wx-load").textContent = "Load";
    }
  }

  function alertLine(msg) {
    $("wx-where").textContent = msg;
  }

  function renderLoaded() {
    const box = $("wx-loaded");
    box.innerHTML = "";
    for (const [path, s] of loaded) {
      const row = document.createElement("div");
      row.className = "run";
      const wind = s.params.find((p) => p.param === "WindU") ?? s.params.find((p) => p.times?.length);
      const span = wind ? `${fmt(wind.times[0], true)} → ${fmt(wind.times.at(-1), true)}` : "no data";
      row.innerHTML = `<input type="radio" name="wx-show"><label></label><button title="Unload">×</button>`;
      const [radio, label, x] = row.children;
      radio.checked = path === display;
      label.textContent = s.name;
      label.title = `${path}\n${span}`;
      radio.onchange = () => showGrib(path);
      label.onclick = () => showGrib(path);
      x.onclick = async () => {
        await invoke("grib_unload", { path });
        loaded.delete(path);
        if (display === path) display = loaded.keys().next().value ?? null;
        renderLoaded();
        setTimes();
        refreshFiles();
      };
      box.append(row);
    }
    syncMirrors();
  }

  /** Show this loaded GRIB on the chart (the time is kept when it is inside the new GRIB). */
  function showGrib(path) {
    if (!loaded.has(path)) return;
    display = path;
    for (const r of $("wx-loaded").querySelectorAll("input[type=radio]")) r.checked = false;
    const i = [...loaded.keys()].indexOf(path);
    const radio = $("wx-loaded").querySelectorAll("input[type=radio]")[i];
    if (radio) radio.checked = true;
    setTimes();
    syncMirrors();
  }

  // ---------- time -------------------------------------------------------------
  /** Layers the shown GRIB has data for. */
  function available(path = display) {
    const s = path && loaded.get(path);
    if (!s) return [];
    const has = new Set(s.params.map((p) => (typeof p.param === "string" ? p.param : "")));
    return Object.keys(LAYERS).filter((k) => LAYERS[k].params.some((p) => has.has(p)));
  }
  const hasParam = (name) => !!loaded.get(display)?.params.some((p) => p.param === name);
  function renderLayers() {
    const av = available();
    if (av.length && !av.includes(wx.layer)) { wx.layer = av[0]; saveWx(); applyLayer(); }
    for (const sel of [$("wx-layer"), ...[...mirrors].filter((m) => m.isConnected).map((m) => m.querySelector(".layer"))]) {
      if (!sel) continue;
      sel.innerHTML = "";
      for (const [k, l] of Object.entries(LAYERS)) {
        const o = new Option(l.label, k);
        o.disabled = !av.includes(k);
        sel.append(o);
      }
      sel.append(new Option("No shading", "none"));
      sel.value = wx.shading === "off" ? "none" : wx.layer;
    }
    $("wx-arrows-row").hidden = !(hasParam("WindU") || hasParam("WindSpeed"));
    $("wx-wavearrows-row").hidden = !hasParam("WaveDir");
    $("wx-isobars-row").hidden = !hasParam("Mslp");
  }
  function setTimes() {
    const s = display && loaded.get(display);
    // the shown layer's times, else the wind's, else whatever the file has
    const want = [...(LAYERS[wx.layer]?.params ?? []), "WindU", "WindSpeed"];
    const series = s && (want.map((w) => s.params.find((p) => p.param === w)).find(Boolean) ?? s.params.find((p) => p.times?.length));
    times = series ? series.times : [];
    renderLayers();
    const slider = $("wx-slider");
    if (!times.length) {
      slider.min = slider.max = 0;
      t = 0;
      draw();
      return;
    }
    const step = times.length > 1 ? Math.min(...times.slice(1).map((v, i) => v - times[i])) : 3600;
    slider.min = times[0];
    slider.max = times.at(-1);
    slider.step = Math.max(900, Math.min(step, 3600));
    if (t < times[0] || t > times.at(-1)) {
      const now = Math.floor(Date.now() / 1000);
      t = now >= times[0] && now <= times.at(-1) ? now - (now % 3600) : times[0];
    }
    slider.value = t;
    draw();
  }

  $("wx-slider").oninput = (e) => { t = Number(e.target.value); draw(); };
  function setT(v) {
    if (!times.length) return;
    const s = $("wx-slider");
    t = Math.min(Number(s.max), Math.max(Number(s.min), v));
    s.value = t;
    draw();
  }
  const stepBy = (n) => {
    const s = $("wx-slider");
    t = Math.min(Number(s.max), Math.max(Number(s.min), t + n * Number(s.step)));
    s.value = t;
    draw();
  };
  $("wx-now").onclick = () => {
    if (!times.length) { alertLine("Load a GRIB first."); return; }
    const now = Math.floor(Date.now() / 1000);
    if (now < times[0] || now > times.at(-1)) { alertLine(`Now is outside this GRIB (${fmt(times[0], true)} → ${fmt(times.at(-1), true)}).`); return; }
    t = now;
    $("wx-slider").value = t;
    draw();
  };
  $("wx-prev").onclick = () => stepBy(-1);
  $("wx-next").onclick = () => stepBy(1);
  function togglePlay() {
    if (playing) { clearInterval(playing); playing = null; }
    else {
      playing = setInterval(() => {
        const s = $("wx-slider");
        if (t >= Number(s.max)) { t = Number(s.min) - Number(s.step); }
        stepBy(1);
      }, 700);
    }
    $("wx-play").textContent = playing ? "❚❚" : "▶";
    syncMirrors();
  }
  $("wx-play").onclick = togglePlay;

  // ---------- copies of the GRIB choice and time slider elsewhere (e.g. the compare dock) ----------
  // Each copy drives the same state as the Weather drawer, so the drawer can stay closed.
  const mirrors = new Set();
  function mountControls(el) {
    el.classList.add("wx-mini");
    el.innerHTML = `<div class="chips"></div>
      <div class="wx-row time"><button data-act="prev" title="Back one step">◀</button><button data-act="play" title="Play / pause">▶</button>` +
      `<button data-act="next" title="Forward one step">▶|</button><button data-act="now" title="Set the time to now">Now</button>` +
      `<select class="layer" title="What the shading shows"></select>` +
      `<input type="range" class="grow"><span class="when">—</span></div>`;
    const sl = el.querySelector("input[type=range]");
    sl.oninput = () => setT(Number(sl.value));
    el.querySelector('[data-act="prev"]').onclick = () => stepBy(-1);
    el.querySelector('[data-act="next"]').onclick = () => stepBy(1);
    el.querySelector('[data-act="play"]').onclick = togglePlay;
    el.querySelector('[data-act="now"]').onclick = () => $("wx-now").click();
    el.querySelector(".layer").onchange = (e) => setLayer(e.target.value);
    mirrors.add(el);
    syncMirror(el, true);
    renderLayers();
  }
  function syncMirror(el, chips) {
    if (chips) {
      const box = el.querySelector(".chips");
      box.innerHTML = "";
      if (!loaded.size) box.innerHTML = `<span class="wx-note">No GRIB loaded: load one in the Weather panel.</span>`;
      for (const [path, s] of loaded) {
        const lab = document.createElement("label");
        lab.className = `chip${path === display ? " on" : ""}`;
        lab.title = `Show ${s.name} on the chart`;
        lab.innerHTML = `<input type="checkbox"><span></span>`;
        lab.querySelector("input").checked = path === display;
        lab.querySelector("span").textContent = s.name;
        lab.querySelector("input").onchange = (e) => { if (e.target.checked) showGrib(path); else e.target.checked = true; };
        box.append(lab);
      }
    }
    const main = $("wx-slider"), sl = el.querySelector("input[type=range]");
    sl.min = main.min; sl.max = main.max; sl.step = main.step; sl.value = t;
    sl.disabled = !times.length;
    el.querySelector(".when").textContent = times.length ? fmt(t) : "—";
    el.querySelector('[data-act="play"]').textContent = playing ? "❚❚" : "▶";
  }
  function syncMirrors(timeOnly) {
    for (const el of mirrors) {
      if (!el.isConnected) { mirrors.delete(el); continue; }
      syncMirror(el, !timeOnly);
    }
  }

  // ---------- drawing --------------------------------------------------------------
  let pending = false;
  async function draw() {
    $("wx-time").textContent = times.length ? fmt(t) : "—";
    syncMirrors(true);
    for (const fn of timeListeners) fn(t);
    updatePoint();
    if (pending) return;
    pending = true;
    requestAnimationFrame(async () => {
      pending = false;
      if (!display || !times.length) {
        map.getSource("wx-cells").setData(fc([]));
        map.getSource("wx-barbs").setData(fc([]));
        return;
      }
      const b = map.getBounds();
      const bbox = [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()];
      const layer = LAYERS[wx.layer] ?? LAYERS.wind;
      const arrowPts = Math.round(DENSITY_POINTS[wx.density - 1]);
      const shade = wx.shading !== "off";
      // Barbs and wave arrows at the chosen density; shading at full GRIB resolution (up to 20 000 cells).
      const [g, gs, gw, gp] = await Promise.all([
        wx.arrows && (hasParam("WindU") || hasParam("WindSpeed")) ? invoke("grib_wind_grid", { path: display, t, bbox, maxPoints: arrowPts }).catch(() => null) : null,
        shade ? invoke("grib_grid", { path: display, layer: wx.layer, t, bbox, maxPoints: 20000 }).catch(() => null) : null,
        wx.waveArrows && hasParam("WaveDir") ? invoke("grib_grid", { path: display, layer: "wavedir", t, bbox, maxPoints: Math.max(40, arrowPts / 2) }).catch(() => null) : null,
        wx.isobars && hasParam("Mslp") ? invoke("grib_grid", { path: display, layer: "mslp", t, bbox, maxPoints: 20000 }).catch(() => null) : null,
      ]);
      const cells = [], barbs = [], waves = [];
      if (g) {
        for (let j = 0; j < g.nj; j++) {
          for (let i = 0; i < g.ni; i++) {
            const k = j * g.ni + i;
            const tws = g.tws[k], twd = g.twd[k];
            if (tws == null) continue;
            const lon = g.lon0 + i * g.dlon, lat = g.lat0 + j * g.dlat;
            barbs.push({ type: "Feature", properties: { tws, twd, icon: barbId(tws, lat) },
              geometry: { type: "Point", coordinates: [lon, lat] } });
          }
        }
      }
      if (gs) {
        const hx = gs.dlon / 2, hy = gs.dlat / 2, min = layer.min ?? -Infinity;
        for (let j = 0; j < gs.nj; j++) {
          for (let i = 0; i < gs.ni; i++) {
            const v = gs.values[j * gs.ni + i];
            if (v == null || v < min) continue;
            const lon = gs.lon0 + i * gs.dlon, lat = gs.lat0 + j * gs.dlat;
            cells.push({ type: "Feature", properties: { v },
              geometry: { type: "Polygon", coordinates: [[[lon - hx, lat - hy], [lon + hx, lat - hy], [lon + hx, lat + hy], [lon - hx, lat + hy], [lon - hx, lat - hy]]] } });
          }
        }
      }
      if (gw) {
        for (let j = 0; j < gw.nj; j++) {
          for (let i = 0; i < gw.ni; i++) {
            const d = gw.values[j * gw.ni + i];
            if (d == null) continue;
            // arrows point the way the waves travel (direction is where they come from)
            waves.push({ type: "Feature", properties: { dir: (d + 180) % 360 }, geometry: { type: "Point", coordinates: [gw.lon0 + i * gw.dlon, gw.lat0 + j * gw.dlat] } });
          }
        }
      }
      map.getSource("wx-cells").setData(fc(cells));
      map.getSource("wx-barbs").setData(fc(barbs));
      map.getSource("wx-wavedir").setData(fc(waves));
      map.getSource("wx-isobars").setData(fc(gp ? isobars(gp, map.getZoom() < 4 ? 8 : 4) : []));
    });
  }
  map.on("moveend", draw);

  // ---------- comparison table --------------------------------------------------------
  let pointTimer = null;
  function updatePoint() {
    if (pointTimer) return;
    pointTimer = setTimeout(async () => {
      pointTimer = null;
      const body = document.querySelector("#wx-point tbody");
      if (!cursor || !loaded.size || !times.length) { body.innerHTML = ""; return; }
      const rows = await invoke("grib_point", { lat: cursor[1], lon: cursor[0], t }).catch(() => []);
      body.innerHTML = "";
      for (const r of rows) {
        const tr = document.createElement("tr");
        const cells = [r.name, n(r.tws, 1), r.twd == null ? "—" : String(r.twd).padStart(3, "0") + "°", n(r.gust, 0), n(r.mslp, 0),
          r.wave == null ? "—" : `${n(r.wave, 1)}${r.period != null ? ` / ${r.period}s` : ""}`];
        for (const c of cells) { const td = document.createElement("td"); td.textContent = c; tr.append(td); }
        if (r.path === display) tr.style.fontWeight = "600";
        body.append(tr);
      }
    }, 120);
  }
  map.on("mousemove", (e) => { cursor = [e.lngLat.lng, e.lngLat.lat]; updatePoint(); });

  $("wx-load").onclick = () => load($("wx-files").value);
  $("wx-refresh").onclick = refreshFiles;

  // ---------- display settings ----------------------------------------------------------
  const saveWx = () => { try { localStorage.setItem("racenav.wx", JSON.stringify(wx)); } catch { /* ignore */ } };
  const dens = $("wx-density"), size = $("wx-size"), shade = $("wx-shading"), opac = $("wx-opacity");
  dens.value = wx.density; size.value = wx.size; shade.value = wx.shading; opac.value = wx.opacity;
  const showVals = () => {
    $("wx-density-v").textContent = wx.density === 10 ? "every point" : String(wx.density);
    $("wx-size-v").textContent = `${Math.round(wx.size * 100)} %`;
    $("wx-opacity-v").textContent = `${Math.round(wx.opacity * 100)} %`;
  };
  showVals();
  dens.oninput = () => { wx.density = Number(dens.value); showVals(); saveWx(); draw(); };
  size.oninput = () => { wx.size = Number(size.value); showVals(); saveWx(); map.setLayoutProperty("wx-barbs", "icon-size", wx.size); };
  opac.oninput = () => { wx.opacity = Number(opac.value); showVals(); saveWx(); map.setPaintProperty("wx-cells", "fill-opacity", wx.opacity); };
  const landSel = $("wx-land");
  landSel.value = wx.land;
  landSel.onchange = () => { wx.land = landSel.value; saveWx(); map.moveLayer("wx-cells", shadingBefore()); };
  function applyLayer() {
    map.setLayoutProperty("wx-cells", "visibility", wx.shading === "off" ? "none" : "visible");
    if (wx.shading !== "off") map.setPaintProperty("wx-cells", "fill-color", shadingExpression(wx.shading, LAYERS[wx.layer] ?? LAYERS.wind));
    $("wx-shading-label").textContent = `${(LAYERS[wx.layer] ?? LAYERS.wind).label} shading`;
    drawLegend();
  }
  shade.onchange = () => {
    if (shade.value === "off") wx.shading = "off";
    else wx.shading = shade.value;
    saveWx();
    applyLayer();
    renderLayers();
    draw();
  };
  function setLayer(v) {
    if (v === "none") { wx.shading = "off"; }
    else { wx.layer = v; if (wx.shading === "off") wx.shading = "smooth"; }
    shade.value = wx.shading;
    saveWx();
    applyLayer();
    setTimes();
  }
  $("wx-layer").onchange = (e) => setLayer(e.target.value);
  for (const [id, key] of [["wx-arrows", "arrows"], ["wx-wavearrows", "waveArrows"], ["wx-isobars", "isobars"]]) {
    $(id).checked = !!wx[key];
    $(id).onchange = (e) => { wx[key] = e.target.checked; saveWx(); draw(); };
  }
  function drawLegend() {
    const el = $("wx-legend");
    el.innerHTML = "";
    if (wx.shading === "off") return;
    const layer = LAYERS[wx.layer] ?? LAYERS.wind;
    if (layer.knots) {
      const step = wx.shading === "smooth" ? 5 : Math.max(Number(wx.shading), wx.shading === "1" ? 2 : Number(wx.shading));
      for (let k = 0; k <= 40; k += step) {
        const sw = document.createElement("span");
        sw.style.background = paletteAt(wx.shading === "smooth" ? k : k + Number(wx.shading) / 2);
        sw.title = `${k} kn`;
        sw.textContent = k % 10 === 0 ? k : "";
        el.append(sw);
      }
    } else {
      for (const [v, c] of layer.scale) {
        const sw = document.createElement("span");
        sw.style.background = c;
        sw.title = `${v} ${layer.unit}`;
        sw.textContent = v;
        el.append(sw);
      }
    }
    el.title = `${layer.label}, ${layer.unit}`;
  }
  applyLayer();

  refreshFiles();
  invoke("grib_loaded").then((list) => {
    for (const s of list || []) loaded.set(s.file, s);
    if (loaded.size) { display = loaded.keys().next().value; renderLoaded(); setTimes(); }
  }).catch(() => {});

  return {
    setCursor: (lonlat) => { cursor = lonlat; updatePoint(); },
    /** The GRIB shown on the chart and the slider time. */
    current: () => ({ path: display, t, name: display && loaded.get(display)?.name }),
    /** Every loaded GRIB: { path, name, start, end } (wind times). */
    all: () => [...loaded].map(([path, s]) => {
      const w = s.params.find((p) => p.param === "WindU") ?? s.params.find((p) => p.times?.length);
      return { path, name: s.name, start: w?.times[0], end: w?.times.at(-1) };
    }),
    /** Move the slider to a time (clamped to the shown GRIB). */
    setTime: (sec) => {
      const s = $("wx-slider");
      if (!times.length) return;
      t = Math.min(Number(s.max), Math.max(Number(s.min), sec));
      s.value = t;
      draw();
    },
    onTime: (fn) => { timeListeners.push(fn); },
    /** Put a GRIB chooser and time slider into `el`, working the same state as the Weather drawer. */
    mountControls,
    showGrib,
    /** Load a GRIB file by path (e.g. one just downloaded), then show it. */
    async load(path) { await load(path); showGrib(path); },
    refreshFiles,
  };
}

/** Barb images are drawn on demand: id "barb:<knots rounded to 5>:<n|s>". */
export function barbImage(id) {
  return id.startsWith("barb:") ? barbIcon(id) : null;
}

function barbId(tws, lat) {
  const k = Math.round(tws / 5) * 5;
  return `barb:${k}:${lat < 0 ? "s" : "n"}`;
}

/** Wind barb pointing up (wind from the north): staff from the station (bottom) to the tip.
 *  Feathers go on the clockwise side in the northern hemisphere, anticlockwise in the southern. */
function barbIcon(id) {
  const [, ks, hem] = id.split(":");
  let k = Number(ks);
  const W = 64, H = 80, cx = W / 2, top = 6, bottom = H - 2;
  const side = hem === "s" ? -1 : 1;
  const c = document.createElement("canvas");
  c.width = W; c.height = H;
  const g = c.getContext("2d");
  const total = k;
  const path = () => {
    let k = total;
    g.beginPath();
    if (k < 3) {
      g.arc(cx, bottom - 8, 6, 0, Math.PI * 2);
      return;
    }
    g.moveTo(cx, bottom);
    g.lineTo(cx, top);
    let y = top;
    const L = 22;
    while (k >= 50) { g.moveTo(cx, y); g.lineTo(cx + side * L, y + 5); g.lineTo(cx, y + 10); y += 13; k -= 50; }
    while (k >= 10) { g.moveTo(cx, y); g.lineTo(cx + side * L, y - 7); y += 8; k -= 10; }
    if (k >= 5) { if (y === top) y += 8; g.moveTo(cx, y); g.lineTo(cx + side * L / 2, y - 4); }
  };
  g.lineCap = "round"; g.lineJoin = "round";
  path(); g.strokeStyle = "rgba(255,255,255,0.9)"; g.lineWidth = 7; g.stroke();
  path(); g.strokeStyle = "#1d2a33"; g.lineWidth = 3; g.stroke();
  // fill pennants
  k = Number(ks);
  let y = top;
  g.fillStyle = "#1d2a33";
  while (k >= 50) { g.beginPath(); g.moveTo(cx, y); g.lineTo(cx + side * 22, y + 5); g.lineTo(cx, y + 10); g.fill(); y += 13; k -= 50; }
  return g.getImageData(0, 0, W, H);
}

/** "12 Oct 16:55" in local time. */
function fmtLocal(sec) {
  return new Date(sec * 1000).toLocaleString("en-NZ", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hour12: false });
}

function fc(features) { return { type: "FeatureCollection", features }; }
function n(v, d) { return v == null ? "—" : v.toFixed(d); }

/** Local time plus UTC, e.g. "Tue 13 Oct 09:00 (20:00Z)". */
export function fmt(sec, short = false) {
  const d = new Date(sec * 1000);
  const local = d.toLocaleString("en-NZ", { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hour12: false });
  if (short) return local;
  const z = d.toISOString().slice(11, 16);
  return `${local} (${z}Z)`;
}

/** Wave arrow: a slim arrow pointing up, drawn once. */
function waveArrow() {
  const w = 18, h = 34, c = document.createElement("canvas");
  c.width = w; c.height = h;
  const g = c.getContext("2d");
  g.strokeStyle = "#0b3d6b"; g.fillStyle = "#0b3d6b"; g.lineWidth = 2.5; g.lineCap = "round";
  g.beginPath(); g.moveTo(w / 2, h - 3); g.lineTo(w / 2, 10); g.stroke();
  g.beginPath(); g.moveTo(w / 2, 2); g.lineTo(w / 2 + 6, 13); g.lineTo(w / 2 - 6, 13); g.closePath(); g.fill();
  return g.getImageData(0, 0, w, h);
}

/**
 * Isobars every `step` hPa from a pressure grid: marching squares, segments
 * joined into lines, with a label point on each longer line.
 */
export function isobars(gp, step) {
  const { ni, nj, values: v } = gp;
  const at = (i, j) => v[j * ni + i];
  const pos = (i, j) => [gp.lon0 + i * gp.dlon, gp.lat0 + j * gp.dlat];
  const lo = Math.min(...v.filter((x) => x != null)), hi = Math.max(...v.filter((x) => x != null));
  const out = [];
  for (let level = Math.ceil(lo / step) * step; level <= hi; level += step) {
    const segs = [];
    for (let j = 0; j + 1 < nj; j++) {
      for (let i = 0; i + 1 < ni; i++) {
        const c = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
        if (c.some((x) => x == null)) continue;
        const p = [pos(i, j), pos(i + 1, j), pos(i + 1, j + 1), pos(i, j + 1)];
        const cross = [];
        for (let e = 0; e < 4; e++) {
          const a = c[e], b = c[(e + 1) % 4];
          if ((a < level) !== (b < level)) {
            const f = (level - a) / (b - a), pa = p[e], pb = p[(e + 1) % 4];
            cross.push([pa[0] + (pb[0] - pa[0]) * f, pa[1] + (pb[1] - pa[1]) * f]);
          }
        }
        if (cross.length === 2) segs.push(cross);
        else if (cross.length === 4) { segs.push([cross[0], cross[1]]); segs.push([cross[2], cross[3]]); }
      }
    }
    for (const line of joinSegments(segs)) {
      out.push({ type: "Feature", properties: { hpa: level }, geometry: { type: "LineString", coordinates: line } });
      if (line.length > 12) out.push({ type: "Feature", properties: { hpa: level }, geometry: { type: "Point", coordinates: line[Math.floor(line.length / 2)] } });
    }
  }
  return out;
}

function joinSegments(segs) {
  const key = (p) => `${p[0].toFixed(5)},${p[1].toFixed(5)}`;
  const ends = new Map();
  segs.forEach((s, i) => { for (const p of s) { const k = key(p); (ends.get(k) ?? ends.set(k, []).get(k)).push(i); } });
  const used = new Array(segs.length).fill(false);
  const lines = [];
  for (let i = 0; i < segs.length; i++) {
    if (used[i]) continue;
    used[i] = true;
    const line = [...segs[i]];
    for (const dir of [1, -1]) {
      for (;;) {
        const tip = dir > 0 ? line.at(-1) : line[0];
        const next = (ends.get(key(tip)) ?? []).find((k) => !used[k]);
        if (next === undefined) break;
        used[next] = true;
        const s = segs[next];
        const other = key(s[0]) === key(tip) ? s[1] : s[0];
        if (dir > 0) line.push(other); else line.unshift(other);
      }
    }
    lines.push(line);
  }
  return lines;
}
