// Weather panel: GRIB files, time slider, wind barbs and speed shading,
// and a table comparing every loaded run at the cursor.
import { invoke } from "@tauri-apps/api/core";

// Wind speed shading (knots).
// Palette sampled every 2 kn so light-air differences show too.
const PALETTE = [
  [0, "#e8f4ff"], [2, "#c4e2ff"], [4, "#9fd0f7"], [6, "#79bdec"], [8, "#5ab9cf"], [10, "#5cc6a3"],
  [12, "#7fd27a"], [14, "#b3dc5c"], [16, "#e2e04f"], [18, "#f7cd45"], [20, "#f7aa3b"], [22, "#f38536"],
  [25, "#e85c3a"], [28, "#d6404f"], [32, "#b8357a"], [36, "#9334a3"], [40, "#6c2fae"], [50, "#45247f"],
];

// Display settings (per viewer)
const WX_DEFAULTS = { density: 5, size: 1.0, shading: "2", opacity: 0.45, land: "land" };
const DENSITY_POINTS = [80, 150, 300, 500, 900, 1400, 2200, 3500, 6000, 1e9];
function loadWx() {
  try { return { ...WX_DEFAULTS, ...JSON.parse(localStorage.getItem("racenav.wx") || "{}") }; } catch { return { ...WX_DEFAULTS }; }
}

/** Colour of a wind speed on the palette (linear between stops). */
function paletteAt(kn) {
  for (let i = 1; i < PALETTE.length; i++) {
    const [k1, c1] = PALETTE[i - 1], [k2, c2] = PALETTE[i];
    if (kn <= k2) {
      const f = Math.max(0, (kn - k1) / (k2 - k1));
      const a = [1, 3, 5].map((o) => parseInt(c1.slice(o, o + 2), 16));
      const b = [1, 3, 5].map((o) => parseInt(c2.slice(o, o + 2), 16));
      return "#" + a.map((v, j) => Math.round(v + (b[j] - v) * f).toString(16).padStart(2, "0")).join("");
    }
  }
  return PALETTE.at(-1)[1];
}

/** fill-color expression: smooth, or bands of `step` knots. */
function shadingExpression(mode) {
  if (mode === "smooth") return ["interpolate", ["linear"], ["get", "tws"], ...PALETTE.flat()];
  const step = Number(mode);
  const expr = ["step", ["get", "tws"], paletteAt(step / 2)];
  for (let k = step; k <= 50; k += step) expr.push(k, paletteAt(k + step / 2));
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
    paint: { "fill-color": shadingExpression(wx.shading === "off" ? "2" : wx.shading), "fill-opacity": wx.opacity, "fill-antialias": false },
    layout: { visibility: wx.shading === "off" ? "none" : "visible" },
  }, shadingBefore());
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

  async function load(path) {
    if (!path) return;
    $("wx-load").disabled = true;
    $("wx-load").textContent = "…";
    try {
      const s = await invoke("grib_load", { path });
      loaded.set(path, s);
      if (!display) display = path;
      renderLoaded();
      setTimes();
      await refreshFiles();
    } catch (e) {
      alertLine(`Could not read GRIB: ${e}`);
    } finally {
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
      const wind = s.params.find((p) => p.param === "WindU");
      const span = wind ? `${fmt(wind.times[0], true)} → ${fmt(wind.times.at(-1), true)}` : "no wind";
      row.innerHTML = `<input type="radio" name="wx-show"><label></label><button title="Unload">×</button>`;
      const [radio, label, x] = row.children;
      radio.checked = path === display;
      label.textContent = s.name;
      label.title = `${path}\n${span}`;
      radio.onchange = () => { display = path; setTimes(); };
      label.onclick = () => { radio.checked = true; display = path; setTimes(); };
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
  }

  // ---------- time -------------------------------------------------------------
  function setTimes() {
    const s = display && loaded.get(display);
    const wind = s && s.params.find((p) => p.param === "WindU");
    times = wind ? wind.times : [];
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
  const stepBy = (n) => {
    const s = $("wx-slider");
    t = Math.min(Number(s.max), Math.max(Number(s.min), t + n * Number(s.step)));
    s.value = t;
    draw();
  };
  $("wx-prev").onclick = () => stepBy(-1);
  $("wx-next").onclick = () => stepBy(1);
  $("wx-play").onclick = () => {
    if (playing) { clearInterval(playing); playing = null; $("wx-play").textContent = "▶"; return; }
    $("wx-play").textContent = "❚❚";
    playing = setInterval(() => {
      const s = $("wx-slider");
      if (t >= Number(s.max)) { t = Number(s.min) - Number(s.step); }
      stepBy(1);
    }, 700);
  };

  // ---------- drawing --------------------------------------------------------------
  let pending = false;
  async function draw() {
    $("wx-time").textContent = times.length ? fmt(t) : "—";
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
      // Barbs at the chosen density; shading at full GRIB resolution (up to 20 000 cells).
      const [g, gs] = await Promise.all([
        invoke("grib_wind_grid", { path: display, t, bbox, maxPoints: Math.round(DENSITY_POINTS[wx.density - 1]) }).catch(() => null),
        wx.shading === "off" ? null : invoke("grib_wind_grid", { path: display, t, bbox, maxPoints: 20000 }).catch(() => null),
      ]);
      if (!g) return;
      const cells = [], barbs = [];
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
      if (gs) {
        const hx = gs.dlon / 2, hy = gs.dlat / 2;
        for (let j = 0; j < gs.nj; j++) {
          for (let i = 0; i < gs.ni; i++) {
            const tws = gs.tws[j * gs.ni + i];
            if (tws == null) continue;
            const lon = gs.lon0 + i * gs.dlon, lat = gs.lat0 + j * gs.dlat;
            cells.push({ type: "Feature", properties: { tws },
              geometry: { type: "Polygon", coordinates: [[[lon - hx, lat - hy], [lon + hx, lat - hy], [lon + hx, lat + hy], [lon - hx, lat + hy], [lon - hx, lat - hy]]] } });
          }
        }
      }
      map.getSource("wx-cells").setData(fc(cells));
      map.getSource("wx-barbs").setData(fc(barbs));
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
        const cells = [r.name, n(r.tws, 1), r.twd == null ? "—" : String(r.twd).padStart(3, "0") + "°", n(r.gust, 0), n(r.mslp, 0), n(r.wave, 1)];
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
  shade.onchange = () => {
    wx.shading = shade.value; saveWx();
    map.setLayoutProperty("wx-cells", "visibility", wx.shading === "off" ? "none" : "visible");
    if (wx.shading !== "off") map.setPaintProperty("wx-cells", "fill-color", shadingExpression(wx.shading));
    draw();
    drawLegend();
  };
  function drawLegend() {
    const el = $("wx-legend");
    el.innerHTML = "";
    if (wx.shading === "off") return;
    const step = wx.shading === "smooth" ? 5 : Math.max(Number(wx.shading), wx.shading === "1" ? 2 : Number(wx.shading));
    for (let k = 0; k <= 40; k += step) {
      const sw = document.createElement("span");
      sw.style.background = paletteAt(wx.shading === "smooth" ? k : k + Number(wx.shading) / 2);
      sw.title = `${k} kn`;
      sw.textContent = k % 10 === 0 ? k : "";
      el.append(sw);
    }
  }
  drawLegend();

  refreshFiles();
  invoke("grib_loaded").then((list) => {
    for (const s of list || []) loaded.set(s.file, s);
    if (loaded.size) { display = loaded.keys().next().value; renderLoaded(); setTimes(); }
  }).catch(() => {});

  return {
    setCursor: (lonlat) => { cursor = lonlat; updatePoint(); },
    /** The GRIB shown on the chart and the slider time. */
    current: () => ({ path: display, t, name: display && loaded.get(display)?.name }),
    /** Move the slider to a time (clamped to the shown GRIB). */
    setTime: (sec) => {
      const s = $("wx-slider");
      if (!times.length) return;
      t = Math.min(Number(s.max), Math.max(Number(s.min), sec));
      s.value = t;
      draw();
    },
    onTime: (fn) => { timeListeners.push(fn); },
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
