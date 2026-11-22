// The ocean below the surface, from a GRIB with a depth stack (Copernicus ocean model,
// "Every depth to 500 m"): the water column at the cursor (temperature and current against
// depth, with the thermocline and the shear marked) and a section along a line drawn on the
// chart (distance across, depth down), shown in the dock. Maps at a depth, and the
// thermocline and shear maps, are weather layers (weather.js).
import { invoke } from "@tauri-apps/api/core";

const empty = { type: "FeatureCollection", features: [] };
const TURBO = ["#30123b", "#4145ab", "#4675ed", "#39a2fc", "#1bcfd4", "#24eca6", "#61fc6c", "#a4fc3b", "#d1e834", "#f3c63a", "#fe9b2d", "#f36315", "#d93806", "#a51301"];
const CURRENT = ["#f7fbff", "#c6dbef", "#9ecae1", "#6baed6", "#4292c6", "#2171b5", "#08519c", "#08306b"];

/** A colour from a list, for t in 0..1. */
export function ramp(cols, t) {
  const x = Math.max(0, Math.min(1, t)) * (cols.length - 1);
  const i = Math.min(cols.length - 2, Math.floor(x)), f = x - i;
  const a = cols[i], b = cols[i + 1];
  const c = (s, k) => parseInt(s.slice(1 + 2 * k, 3 + 2 * k), 16);
  const mix = (k) => Math.round(c(a, k) + (c(b, k) - c(a, k)) * f);
  return `rgb(${mix(0)},${mix(1)},${mix(2)})`;
}

/** Tidy axis ticks over lo..hi, about n of them. */
export function niceTicks(lo, hi, n = 5) {
  if (!(hi > lo)) return [lo];
  const raw = (hi - lo) / n, p = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= raw);
  const out = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + step * 1e-9; v += step) out.push(+v.toFixed(6));
  return out;
}

const finite = (a) => a.filter((v) => v != null && Number.isFinite(v));

/** The water column as an SVG, three panels side by side on one depth axis: temperature (°C),
 *  salinity (PSU) and current (kn), with the thermocline (dashed) and the shear (dotted) across them. */
export function profileSvg(p, { w = 560, h = 240 } = {}) {
  const m = { l: 40, r: 8, t: 20, b: 18 }, gap = 14;
  const deep = Math.max(...p.depths.filter((_, i) => p.temp[i] != null || p.kn[i] != null || p.sal[i] != null), 10);
  const y = (d) => m.t + (d / deep) * (h - m.t - m.b);
  const panels = [
    { key: "temp", label: "Temperature °C", cls: "temp", dp: 1 },
    { key: "sal", label: "Salinity PSU", cls: "sal", dp: 2 },
    { key: "kn", label: "Current kn", cls: "kn", dp: 1, zero: true },
  ].filter((q) => finite(p[q.key]).length);
  const pw = (w - m.l - m.r - gap * (panels.length - 1)) / Math.max(panels.length, 1);
  let s = `<svg class="wx-profile-svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" role="img" aria-label="Temperature, salinity and current against depth">`;
  for (const d of niceTicks(0, deep, 5)) s += `<text x="${m.l - 4}" y="${y(d) + 3}" text-anchor="end">${d}</text>`;
  s += `<text x="${m.l - 4}" y="${m.t - 8}" text-anchor="end" class="u">m</text>`;
  panels.forEach((q, k) => {
    const x0 = m.l + k * (pw + gap), x1 = x0 + pw;
    const vs = finite(p[q.key]);
    let lo = q.zero ? 0 : Math.min(...vs), hi = Math.max(...vs);
    const ticks = niceTicks(lo, hi > lo ? hi : lo + 1, 3);
    lo = Math.min(lo, ticks[0]); hi = Math.max(hi, ticks.at(-1), lo + 1e-6);
    const x = (v) => x0 + ((v - lo) / (hi - lo)) * pw;
    s += `<rect x="${x0}" y="${m.t}" width="${pw}" height="${h - m.t - m.b}" class="frame"/>`;
    for (const d of niceTicks(0, deep, 5)) s += `<line x1="${x0}" x2="${x1}" y1="${y(d)}" y2="${y(d)}" class="grid"/>`;
    for (const v of ticks) s += `<text x="${x(v)}" y="${h - 5}" text-anchor="middle" class="${q.cls}">${+v.toFixed(q.dp + 1)}</text>`;
    s += `<text x="${x0 + pw / 2}" y="${m.t - 7}" text-anchor="middle" class="${q.cls} h">${q.label}</text>`;
    if (p.thermocline) s += `<line x1="${x0}" x2="${x1}" y1="${y(p.thermocline[0])}" y2="${y(p.thermocline[0])}" class="thermo"/>`;
    if (p.shear) s += `<line x1="${x0}" x2="${x1}" y1="${y(p.shear[0])}" y2="${y(p.shear[0])}" class="shear"/>`;
    let d = "", pen = false;
    p.depths.forEach((z, i) => { const v = p[q.key][i]; if (v == null) { pen = false; return; } d += `${pen ? "L" : "M"}${x(v).toFixed(1)},${y(z).toFixed(1)}`; pen = true; });
    s += `<path d="${d}" class="${q.cls}"/>`;
  });
  s += `<line class="hover" x1="${m.l}" x2="${w - m.r}" y1="0" y2="0" visibility="hidden"/>`;
  return s + `</svg>`;
}
/** The depth (m) under a y position of profileSvg's picture. */
export function profileDepthAt(p, py, h = 240) {
  const m = { t: 20, b: 18 };
  const deep = Math.max(...p.depths.filter((_, i) => p.temp[i] != null || p.kn[i] != null || p.sal[i] != null), 10);
  return ((py - m.t) / (h - m.t - m.b)) * deep;
}

export function setupDepthStack(map, { weather, dock }) {
  const $ = (id) => document.getElementById(id);
  // ---------- the water column at the cursor (a dock view) ------------------------------------
  // In the side panel: a one-line summary at the cursor and the buttons; the picture is in the dock,
  // where it can be seen while the mouse is on the chart. It follows the cursor and keeps the last
  // place with water when the cursor leaves the chart; a click on the chart pins it there.
  const box = document.createElement("div");
  box.id = "wx-profile";
  box.className = "wx-profile";
  box.hidden = true;
  box.innerHTML = `<div class="wx-profile-head"><b>Below the surface</b> <span class="wx-profile-note"></span></div>
    <div class="wx-profile-btns"><button type="button" class="wx-wc-btn" title="Temperature, salinity and current against depth at the cursor, in the dock">Water column</button>
    <button type="button" class="wx-section-btn" title="Draw a line on the chart: a section along it shows the water from the surface down, in the dock">Section along a line…</button></div>`;
  ($("wxp-point") ?? $("side-grib-point"))?.after(box);
  const note = box.querySelector(".wx-profile-note");
  map.addSource("wc-pin", { type: "geojson", data: empty });
  map.addLayer({ id: "wc-pin", type: "circle", source: "wc-pin", paint: { "circle-radius": 6, "circle-color": "#e4572e", "circle-stroke-color": "#fff", "circle-stroke-width": 2 } });
  let prof = null, profAt = null, pinned = null, busy = false, again = null, wcBody = null;
  const fmtPos = (c) => `${Math.abs(c[1]).toFixed(3)}°${c[1] < 0 ? "S" : "N"} ${Math.abs(c[0]).toFixed(3)}°${c[0] < 0 ? "W" : "E"}`;
  const summary = (p) => !p ? "" : [
    p.thermocline ? `thermocline ${Math.round(p.thermocline[0])} m (${p.thermocline[1].toFixed(2)} °C/10 m)` : "well mixed: no thermocline",
    p.shear ? `shear ${Math.round(p.shear[0])} m (${p.shear[1].toFixed(2)} kn/10 m)` : null,
  ].filter(Boolean).join(" · ");
  async function fetchProfile(at) {
    const path = weather.stackPath();
    box.hidden = !path;
    if (!path || !at) return;
    if (busy) { again = at; return; }
    busy = true;
    try {
      const p = await invoke("grib_profile", { path, lat: at[1], lon: at[0], t: weather.current().t });
      // over land the last place with water stays (unless pinned there)
      if (p || pinned) { prof = p; profAt = at; }
      note.textContent = p ? `· ${summary(p)}` : "";
      drawProfile();
    } catch { /* unloaded */ }
    busy = false;
    if (again) { const a = again; again = null; fetchProfile(a); }
  }
  weather.onPoint((_row, at) => { if (!pinned && at) fetchProfile(at); });
  weather.onTime(() => { if (profAt) fetchProfile(profAt); });
  weather.onLoaded(() => { box.hidden = !weather.stackPath(); if (!box.hidden && profAt) fetchProfile(profAt); });
  box.querySelector(".wx-wc-btn").onclick = () => dock.show("watercol");
  box.querySelector(".wx-section-btn").onclick = () => startDrawing();
  map.on("click", (e) => {
    if (drawing || dock.current() !== "watercol") return;
    pinned = [e.lngLat.lng, e.lngLat.lat];
    map.getSource("wc-pin").setData({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: pinned } });
    fetchProfile(pinned);
    dock.refresh("watercol");
  });
  const unpin = () => { pinned = null; map.getSource("wc-pin").setData(empty); };
  function drawProfile() {
    if (!wcBody?.isConnected) return;
    const host = wcBody.parentElement ?? wcBody;
    const W = Math.max(360, host.clientWidth - 24), H = Math.max(150, host.clientHeight - 34);
    const head = `<div class="wc-head">${profAt ? `${pinned ? "Pinned at" : "At the cursor,"} ${fmtPos(profAt)}` : "Move the cursor over the sea"}${prof ? ` · ${summary(prof)}` : ""}</div>`;
    if (!prof) { wcBody.innerHTML = head + `<p class="wx-note">${profAt ? "No water column here (land, or outside the file)." : ""}</p>`; return; }
    wcBody.innerHTML = head + `<div class="wc-plot">${profileSvg(prof, { w: W, h: H })}<div class="sect-tip" hidden></div></div>`;
    const svg = wcBody.querySelector("svg"), tip = wcBody.querySelector(".sect-tip"), line = svg.querySelector(".hover");
    svg.onmousemove = (e) => {
      const r = svg.getBoundingClientRect(), py = ((e.clientY - r.top) / r.height) * H;
      const z = profileDepthAt(prof, py, H);
      const i = prof.depths.reduce((b, d, k) => (Math.abs(d - z) < Math.abs(prof.depths[b] - z) ? k : b), 0);
      const yy = 20 + (prof.depths[i] / Math.max(...prof.depths.filter((_, k) => prof.temp[k] != null || prof.kn[k] != null || prof.sal[k] != null), 10)) * (H - 38);
      line.setAttribute("y1", yy); line.setAttribute("y2", yy); line.setAttribute("visibility", "visible");
      const v = (x, dp, u) => (x == null ? null : `${x.toFixed(dp)} ${u}`);
      tip.hidden = false;
      tip.style.left = `${e.clientX - r.left + 12}px`; tip.style.top = `${e.clientY - r.top + 8}px`;
      tip.textContent = [`${prof.depths[i] < 10 ? prof.depths[i].toFixed(1) : Math.round(prof.depths[i])} m`, v(prof.temp[i], 1, "°C"), v(prof.sal[i], 2, "PSU"),
        prof.kn[i] == null ? null : `${prof.kn[i].toFixed(1)} kn toward ${String(prof.set[i]).padStart(3, "0")}°`].filter(Boolean).join(" · ");
    };
    svg.onmouseleave = () => { tip.hidden = true; line.setAttribute("visibility", "hidden"); };
  }
  dock.register("watercol", {
    title: "Water column",
    render(body, tools) {
      const follow = document.createElement("button");
      follow.textContent = pinned ? "Follow the cursor" : "Click the chart to pin";
      follow.disabled = !pinned;
      follow.title = "The water column follows the cursor; a click on the chart pins it to that place";
      follow.onclick = () => { unpin(); dock.refresh("watercol"); };
      tools.append(follow);
      wcBody = document.createElement("div");
      wcBody.className = "wc-body";
      body.append(wcBody);
      requestAnimationFrame(drawProfile);
    },
  });
  dock.onChange((cur) => { if (cur !== "watercol") { unpin(); wcBody = null; } });
  // the dock resized: draw again to fit
  let fitTimer = null;
  window.addEventListener("racenav-layout", () => {
    clearTimeout(fitTimer);
    fitTimer = setTimeout(() => { if (dock.current() === "watercol") drawProfile(); else if (dock.current() === "section" && section) dock.refresh("section"); }, 150);
  });

  // ---------- a section along a line --------------------------------------------------------
  map.addSource("sect-line", { type: "geojson", data: empty });
  map.addSource("sect-here", { type: "geojson", data: empty });
  map.addLayer({ id: "sect-line", type: "line", source: "sect-line", layout: { "line-cap": "round" }, paint: { "line-color": "#b1005d", "line-width": 2.5, "line-dasharray": [2, 1] } });
  map.addLayer({ id: "sect-pts", type: "circle", source: "sect-line", filter: ["==", ["geometry-type"], "Point"], paint: { "circle-radius": 4, "circle-color": "#fff", "circle-stroke-color": "#b1005d", "circle-stroke-width": 2 } });
  map.addLayer({ id: "sect-here", type: "circle", source: "sect-here", paint: { "circle-radius": 6, "circle-color": "#b1005d", "circle-stroke-color": "#fff", "circle-stroke-width": 2 } });
  let drawing = false, line = [], rubber = null, section = null, kind = "temp";
  const showLine = () => {
    const pts = rubber ? [...line, rubber] : line;
    map.getSource("sect-line").setData({ type: "FeatureCollection", features: [
      ...(pts.length > 1 ? [{ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: pts } }] : []),
      ...line.map((c) => ({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: c } })),
    ] });
  };
  function startDrawing() {
    if (!weather.stackPath()) return;
    drawing = true; line = []; rubber = null; showLine();
    map.getCanvas().style.cursor = "crosshair";
    map.doubleClickZoom.disable();
    note.textContent = "· click along the line; double-click (or Enter) to finish, Esc to cancel";
  }
  function stopDrawing(keep) {
    drawing = false; rubber = null;
    map.getCanvas().style.cursor = "";
    setTimeout(() => map.doubleClickZoom.enable(), 0);
    if (keep && line.length > 1) { showLine(); section = null; dock.show("section"); fetchSection(); }
    else { line = []; showLine(); }
  }
  map.on("click", (e) => { if (drawing) { line.push([e.lngLat.lng, e.lngLat.lat]); showLine(); } });
  map.on("mousemove", (e) => { if (drawing && line.length) { rubber = [e.lngLat.lng, e.lngLat.lat]; showLine(); } });
  map.on("dblclick", (e) => { if (!drawing) return; e.preventDefault(); stopDrawing(true); });
  window.addEventListener("keydown", (e) => {
    if (!drawing) return;
    if (e.key === "Escape") { e.preventDefault(); stopDrawing(false); }
    if (e.key === "Enter") { e.preventDefault(); stopDrawing(true); }
  });

  let fetching = 0;
  async function fetchSection() {
    const path = weather.stackPath();
    if (!path || line.length < 2) return;
    const n = ++fetching;
    try {
      const s = await invoke("grib_section", { path, line, t: weather.current().t, n: 200 });
      if (n !== fetching) return;
      section = s;
    } catch { section = null; }
    if (dock.current() === "section") dock.refresh("section");
  }
  weather.onTime(() => { if (dock.current() === "section" && line.length > 1) fetchSection(); });
  dock.onChange((cur) => {
    if (cur !== "section" && !drawing) { line = []; showLine(); map.getSource("sect-here").setData(empty); }
  });

  const KINDS = { temp: ["Temperature", "°C"], sal: ["Salinity", "PSU"], kn: ["Current", "kn"] };
  dock.register("section", {
    title: "Section",
    render(body, tools) {
      tools.innerHTML = "";
      const sel = document.createElement("select");
      for (const [k, [label]] of Object.entries(KINDS)) sel.append(new Option(label, k));
      sel.value = kind;
      sel.onchange = () => { kind = sel.value; dock.refresh("section"); };
      const redraw = document.createElement("button");
      redraw.textContent = "Draw another line";
      redraw.onclick = () => { dock.show(null); startDrawing(); };
      tools.append(sel, redraw);
      if (!section) { body.innerHTML = `<p class="wx-note">${line.length > 1 ? "Reading the section…" : "Draw a line on the chart (Section along a line…, under GRIB data at the cursor)."}</p>`; return; }
      drawSection(body);
    },
  });

  function drawSection(body) {
    const s = section;
    const vals = s[kind];
    const all = finite(vals.flat());
    body.innerHTML = "";
    const wrap = document.createElement("div");
    wrap.className = "sect-wrap";
    const cv = document.createElement("canvas");
    const tip = document.createElement("div");
    tip.className = "sect-tip";
    const key = document.createElement("div");
    key.className = "sect-key";
    wrap.append(cv, tip);
    body.append(wrap, key);
    if (!all.length) { key.textContent = "No values along this line (land, or outside the file)."; return; }
    const lo = Math.min(...all), hi = Math.max(...all);
    const cols = kind === "kn" ? CURRENT : TURBO;
    const colour = (v) => ramp(cols, (v - (kind === "kn" ? 0 : lo)) / Math.max(hi - (kind === "kn" ? 0 : lo), 1e-6));
    // deepest level with a value anywhere: the picture stops there
    let nz = s.depths.length;
    while (nz > 1 && vals[nz - 1].every((v) => v == null)) nz--;
    const deep = s.depths[nz - 1];
    const W = Math.max(300, body.clientWidth - 8), H = Math.max(120, body.clientHeight - 34);
    const dpr = window.devicePixelRatio || 1;
    cv.width = W * dpr; cv.height = H * dpr; cv.style.width = `${W}px`; cv.style.height = `${H}px`;
    const g = cv.getContext("2d");
    g.scale(dpr, dpr);
    const m = { l: 40, r: 8, t: 6, b: 18 };
    const total = s.dist.at(-1) || 1;
    const X = (d) => m.l + (d / total) * (W - m.l - m.r);
    const Y = (z) => m.t + (z / deep) * (H - m.t - m.b);
    // each cell from half-way to the previous level/point to half-way to the next
    const edge = (arr, i, f) => (i <= 0 ? f(arr[0]) - (f(arr[1]) - f(arr[0])) / 2 : i >= arr.length ? f(arr.at(-1)) + (f(arr.at(-1)) - f(arr.at(-2))) / 2 : (f(arr[i - 1]) + f(arr[i])) / 2);
    g.fillStyle = "#8a7a4a"; // below the sea floor
    g.fillRect(m.l, m.t, W - m.l - m.r, H - m.t - m.b);
    for (let z = 0; z < nz; z++) {
      const y0 = Math.max(m.t, edge(s.depths, z, Y)), y1 = Math.min(H - m.b, edge(s.depths, z + 1, Y));
      for (let x = 0; x < s.dist.length; x++) {
        const v = vals[z][x];
        if (v == null) continue;
        g.fillStyle = colour(v);
        const x0 = Math.max(m.l, edge(s.dist, x, X)), x1 = Math.min(W - m.r, edge(s.dist, x + 1, X));
        g.fillRect(x0, y0, x1 - x0 + 0.6, y1 - y0 + 0.6);
      }
    }
    // the thermocline along the line
    if (kind !== "sal") {
      g.strokeStyle = "#fff"; g.lineWidth = 1.6; g.setLineDash([5, 3]);
      g.beginPath();
      let pen = false;
      s.thermocline.forEach((z, x) => { if (z == null) { pen = false; return; } if (pen) g.lineTo(X(s.dist[x]), Y(z)); else g.moveTo(X(s.dist[x]), Y(z)); pen = true; });
      g.stroke(); g.setLineDash([]);
    }
    g.fillStyle = "#1d2a33"; g.font = "11px system-ui, sans-serif"; g.textAlign = "right";
    for (const z of niceTicks(0, deep, 5)) { g.fillText(`${z} m`, m.l - 4, Y(z) + 4); }
    g.textAlign = "center";
    for (const d of niceTicks(0, total, 6)) g.fillText(`${d} NM`, X(d), H - 4);
    const [label, unit] = KINDS[kind];
    const fmt = (v) => (kind === "sal" ? v.toFixed(2) : v.toFixed(1));
    key.innerHTML = `<span>${label} along the line, ${fmt(kind === "kn" ? 0 : lo)}</span><span class="sect-bar" style="background:linear-gradient(to right,${cols.join(",")})"></span><span>${fmt(hi)} ${unit}</span>` +
      (kind !== "sal" ? `<span class="sect-thermo">- - thermocline</span>` : "") + `<span class="wx-note">at ${new Date(weather.current().t * 1000).toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit" })}</span>`;
    cv.onmousemove = (e) => {
      const r = cv.getBoundingClientRect(), px = e.clientX - r.left, py = e.clientY - r.top;
      const d = ((px - m.l) / (W - m.l - m.r)) * total, z = ((py - m.t) / (H - m.t - m.b)) * deep;
      if (d < 0 || d > total || z < 0 || z > deep) { tip.hidden = true; map.getSource("sect-here").setData(empty); return; }
      const xi = s.dist.reduce((b, v, i) => (Math.abs(v - d) < Math.abs(s.dist[b] - d) ? i : b), 0);
      const zi = s.depths.reduce((b, v, i) => (Math.abs(v - z) < Math.abs(s.depths[b] - z) ? i : b), 0);
      const v = vals[zi]?.[xi];
      tip.hidden = false;
      tip.style.left = `${px + 12}px`; tip.style.top = `${py + 8}px`;
      tip.textContent = `${v == null ? "sea floor" : `${fmt(v)} ${unit}`} · ${Math.round(s.depths[zi])} m · ${s.dist[xi].toFixed(1)} NM`;
      map.getSource("sect-here").setData({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: s.pts[xi] } });
    };
    cv.onmouseleave = () => { tip.hidden = true; map.getSource("sect-here").setData(empty); };
  }

  return { isDrawing: () => drawing };
}
