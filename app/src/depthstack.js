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

/** The water column as an SVG: temperature (red, bottom axis) and current (blue, top axis) against depth. */
export function profileSvg(p, { w = 280, h = 230 } = {}) {
  const m = { l: 34, r: 10, t: 22, b: 22 };
  const deep = Math.max(...p.depths.filter((_, i) => p.temp[i] != null || p.kn[i] != null), 10);
  const ts = finite(p.temp), ks = finite(p.kn);
  const tlo = ts.length ? Math.floor(Math.min(...ts)) : 0, thi = ts.length ? Math.ceil(Math.max(...ts)) : 1;
  const khi = ks.length ? Math.max(0.5, Math.ceil(Math.max(...ks) * 4) / 4) : 1;
  const y = (d) => m.t + (d / deep) * (h - m.t - m.b);
  const xT = (v) => m.l + ((v - tlo) / Math.max(thi - tlo, 0.5)) * (w - m.l - m.r);
  const xK = (v) => m.l + (v / khi) * (w - m.l - m.r);
  const line = (vals, x) => {
    let d = "", pen = false;
    p.depths.forEach((z, i) => { const v = vals[i]; if (v == null) { pen = false; return; } d += `${pen ? "L" : "M"}${x(v).toFixed(1)},${y(z).toFixed(1)}`; pen = true; });
    return d;
  };
  let s = `<svg class="wx-profile-svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" role="img" aria-label="Temperature and current against depth">`;
  for (const d of niceTicks(0, deep, 5)) s += `<line x1="${m.l}" x2="${w - m.r}" y1="${y(d)}" y2="${y(d)}" class="grid"/><text x="${m.l - 4}" y="${y(d) + 3}" text-anchor="end">${d}</text>`;
  for (const v of niceTicks(tlo, thi, 4)) s += `<text x="${xT(v)}" y="${h - 6}" text-anchor="middle" class="t">${v}°</text>`;
  if (ks.length) for (const v of niceTicks(0, khi, 3)) s += `<text x="${xK(v)}" y="${m.t - 8}" text-anchor="middle" class="k">${v}</text>`;
  if (p.thermocline) s += `<line x1="${m.l}" x2="${w - m.r}" y1="${y(p.thermocline[0])}" y2="${y(p.thermocline[0])}" class="thermo"/>`;
  if (p.shear) s += `<line x1="${m.l}" x2="${w - m.r}" y1="${y(p.shear[0])}" y2="${y(p.shear[0])}" class="shear"/>`;
  if (ks.length) s += `<path d="${line(p.kn, xK)}" class="kn"/>`;
  if (ts.length) s += `<path d="${line(p.temp, xT)}" class="temp"/>`;
  s += `<text x="${m.l - 4}" y="${m.t - 8}" text-anchor="end" class="u">m</text>`;
  if (ks.length) s += `<text x="${w - m.r}" y="10" text-anchor="end" class="k">current kn</text>`;
  return s + `</svg>`;
}

export function setupDepthStack(map, { weather, dock }) {
  const $ = (id) => document.getElementById(id);
  // ---------- the water column at the cursor ----------------------------------------------
  const box = document.createElement("div");
  box.id = "wx-profile";
  box.className = "wx-profile";
  box.hidden = true;
  box.innerHTML = `<div class="wx-profile-head"><b>Water column at the cursor</b></div><div class="wx-profile-plot"></div><div class="wx-profile-note"></div>
    <button type="button" class="wx-section-btn" title="Draw a line on the chart: a section along it shows the water from the surface down">Section along a line…</button>`;
  ($("wxp-point") ?? $("side-grib-point"))?.after(box);
  let busy = false, again = null;
  async function showProfile(at) {
    const path = weather.stackPath();
    if (!path || !at) { box.hidden = !path; if (!at) { box.querySelector(".wx-profile-plot").innerHTML = ""; box.querySelector(".wx-profile-note").textContent = path ? "Move the cursor over the sea." : ""; } return; }
    if (busy) { again = at; return; }
    busy = true;
    try {
      const p = await invoke("grib_profile", { path, lat: at[1], lon: at[0], t: weather.current().t });
      box.hidden = false;
      const plot = box.querySelector(".wx-profile-plot"), note = box.querySelector(".wx-profile-note");
      if (!p) { plot.innerHTML = ""; note.textContent = "No water column here (land, or outside the file)."; }
      else {
        plot.innerHTML = profileSvg(p);
        const top = p.temp.find((v) => v != null);
        note.textContent = [
          p.thermocline ? `Thermocline ${Math.round(p.thermocline[0])} m (${p.thermocline[1].toFixed(2)} °C per 10 m)` : "No thermocline to speak of (well mixed)",
          p.shear ? `shear ${Math.round(p.shear[0])} m (${p.shear[1].toFixed(2)} kn per 10 m)` : null,
          top != null ? `surface ${top.toFixed(1)} °C` : null,
        ].filter(Boolean).join(" · ");
      }
    } catch { /* not loaded any more */ }
    busy = false;
    if (again) { const a = again; again = null; showProfile(a); }
  }
  let lastAt = null;
  weather.onPoint((_row, at) => { lastAt = at; showProfile(at); });
  weather.onLoaded(() => { box.hidden = !weather.stackPath(); showProfile(lastAt); });
  box.querySelector(".wx-section-btn").onclick = () => startDrawing();

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
    box.querySelector(".wx-profile-note").textContent = "Click along the line; double-click (or Enter) to finish, Esc to cancel.";
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
