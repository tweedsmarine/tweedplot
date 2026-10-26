// Weather stations and buoys on the chart (Weather › Weather stations and
// buoys): the latest wind at each, and an info box with the last 24 h against the
// loaded GRIBs (real versus forecast).
import { infoBox } from "./infobox.js";
import { invoke } from "@tauri-apps/api/core";
import { ddm } from "./navmath.js";

const KEY = "racenav.stations";
const DEFAULTS = { show: false, sources: { metservice: true, metar: true, ndbc: false } };
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const SRC = { metservice: "MetService (open access)", metar: "aviationweather.gov METAR", ndbc: "NOAA NDBC" };
const COMPASS = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
const point = (d) => COMPASS[Math.round(((d % 360) + 360) % 360 / 22.5) % 16];


// The station symbol, after IMO SN.1/Circ.243 (ECDIS symbols, "weather station"):
// a circle with W in it, and the reported wind as a shaft from the direction the
// wind comes from to the circle, with barbs (full 10 kn, half 5 kn) and pennants
// (50 kn) on the clockwise side in the northern hemisphere, anticlockwise in the
// southern (WMO No. 485). Calm: a second circle round it; a direction without a
// speed: an x at the end of the shaft. Drawn in the AIS AtoN colour.
const STN_COL = "#6a1b9a";
const D = 18; // circle diameter, px (the circular says no more than 6 mm)
const SHAFT = 2.6 * D; // no more than three diameters
/** Station image ids: "stnc" (the circle with W) or "stnc:calm" (with the calm ring),
 *  upright; and the wind shaft, turned to the wind: "stnw:<kn rounded to 5>:<n|s>" or
 *  "stnw:x" (a direction without a speed). The shaft points up (wind from the north),
 *  drawn about the circle's centre so both share an anchor. */
export function stationImage(id) {
  const circle = id === "stnc" || id === "stnc:calm";
  if (!circle && !id.startsWith("stnw:")) return null;
  const [, kind, hem] = id.split(":");
  const R = 2, half = circle ? D / 2 + 7 : SHAFT + D / 2 + 6, W = 2 * half;
  const c = document.createElement("canvas");
  c.width = c.height = W * R;
  const g = c.getContext("2d");
  g.scale(R, R);
  const cx = half, cy = half, r = D / 2;
  const side = hem === "s" ? -1 : 1; // clockwise of the shaft (pointing up) is to the right
  g.lineCap = "round"; g.lineJoin = "round";
  if (circle) {
    const ring = (w, col) => { g.strokeStyle = col; g.lineWidth = w; g.beginPath(); g.arc(cx, cy, r, 0, 7); if (kind === "calm") { g.moveTo(cx + r + 4, cy); g.arc(cx, cy, r + 4, 0, 7); } g.stroke(); };
    ring(4.5, "rgba(255,255,255,0.92)");
    g.fillStyle = "rgba(255,255,255,0.85)"; g.beginPath(); g.arc(cx, cy, r, 0, 7); g.fill();
    ring(1.3, STN_COL);
    g.fillStyle = STN_COL;
    g.font = `bold ${Math.round(D * 0.62)}px sans-serif`; g.textAlign = "center"; g.textBaseline = "middle";
    g.fillText("W", cx, cy + 0.5);
    return g.getImageData(0, 0, c.width, c.height);
  }
  const top = cy - r - SHAFT;
  const shape = (stroke) => {
    // the shaft from the circle's edge out towards where the wind comes from
    g.beginPath(); g.moveTo(cx, cy - r); g.lineTo(cx, top); stroke();
    if (kind === "x") { g.beginPath(); g.moveTo(cx - 5, top - 5); g.lineTo(cx + 5, top + 5); g.moveTo(cx + 5, top - 5); g.lineTo(cx - 5, top + 5); stroke(); return; }
    let k = Math.round(Number(kind) / 5) * 5, y = top;
    const L = D * 0.95, ang = (60 * Math.PI) / 180; // barbs at 120° to the shaft towards the station
    const dx = side * L * Math.sin(ang), dy = -L * Math.cos(ang);
    const pennants = [];
    while (k >= 50) { pennants.push(y); y += 9; k -= 50; }
    if (pennants.length) y += 2;
    g.beginPath();
    while (k >= 10) { g.moveTo(cx, y); g.lineTo(cx + dx, y + dy); y += 6; k -= 10; }
    if (k >= 5) { if (y === top) y += 6; g.moveTo(cx, y); g.lineTo(cx + dx / 2, y + dy / 2); }
    stroke();
    for (const py of pennants) { g.beginPath(); g.moveTo(cx, py); g.lineTo(cx + side * L * 0.9, py + 1); g.lineTo(cx, py + 8); g.closePath(); g.fill(); }
  };
  g.strokeStyle = g.fillStyle = "rgba(255,255,255,0.92)";
  shape(() => { g.lineWidth = 5.5; g.stroke(); });
  g.strokeStyle = g.fillStyle = STN_COL;
  shape(() => { g.lineWidth = 2.4; g.stroke(); });
  return g.getImageData(0, 0, c.width, c.height);
}
const shaftIcon = (o) => {
  if (o.wind_dir == null || (o.wind_kn != null && o.wind_kn < 1)) return null;
  if (o.wind_kn == null) return "stnw:x";
  return `stnw:${Math.round(o.wind_kn / 5) * 5}:${o.lat < 0 ? "s" : "n"}`;
};

export function setupStations(map, { weather }) {
  let s = { ...DEFAULTS, sources: { ...DEFAULTS.sources } };
  try { const j = JSON.parse(localStorage.getItem(KEY) || "{}"); Object.assign(s, j); s.sources = { ...DEFAULTS.sources, ...(j.sources || {}) }; } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ } };
  let obs = [];
  let busy = false;

  // arrow pointing up (rotated to the downwind direction)
  map.addSource("stations", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  // the symbol (drawn on demand by stationImage), turned to the direction the wind comes from
  map.addLayer({ id: "stn-sym", type: "symbol", source: "stations", filter: ["has", "shaft"],
    layout: { "icon-image": ["get", "shaft"], "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map", "icon-allow-overlap": true, "icon-ignore-placement": true },
  });
  // the circle with W stays upright
  map.addLayer({ id: "stn-w", type: "symbol", source: "stations",
    layout: { "icon-image": ["get", "icon"], "icon-rotation-alignment": "viewport", "icon-allow-overlap": true, "icon-ignore-placement": true },
  });
  // click target round the circle
  map.addLayer({ id: "stn-dot", type: "circle", source: "stations",
    paint: { "circle-radius": 11, "circle-color": "#000", "circle-opacity": 0.01 } });
  // the label: wind (and gust, waves); the station's name too when zoomed in
  map.addLayer({ id: "stn-label", type: "symbol", source: "stations", minzoom: 6.5, maxzoom: 9, filter: ["has", "lbl"],
    layout: { "icon-image": ["concat", "lbl:", ["get", "lbl"]], "icon-anchor": "left", "icon-offset": [14, 8], "icon-size": 1.0, "icon-allow-overlap": true, "icon-ignore-placement": true } });
  map.addLayer({ id: "stn-label-name", type: "symbol", source: "stations", minzoom: 9, filter: ["has", "lbl2"],
    layout: { "icon-image": ["concat", "lbl:", ["get", "lbl2"]], "icon-anchor": "left", "icon-offset": [14, 8], "icon-size": 1.0, "icon-allow-overlap": true, "icon-ignore-placement": true } });

  function draw() {
    const vis = s.show ? "visible" : "none";
    for (const id of ["stn-dot", "stn-sym", "stn-w", "stn-label", "stn-label-name"]) map.setLayoutProperty(id, "visibility", vis);
    const f = obs.map((o, i) => {
      const kn = o.wind_kn;
      const p = { i, icon: o.wind_kn != null && o.wind_kn < 1 ? "stnc:calm" : "stnc", rot: o.wind_dir ?? 0 };
      const sh = shaftIcon(o);
      if (sh) p.shaft = sh;
      const w = kn != null ? `${Math.round(kn)}${o.gust_kn != null && o.gust_kn > kn + 2 ? "G" + Math.round(o.gust_kn) : ""}` : "";
      const wave = o.wave_m != null ? ` ${o.wave_m.toFixed(1)}m` : "";
      if (w || wave) p.lbl = `${w}${wave}`;
      const name = String(o.name ?? "").trim();
      if (name || p.lbl) p.lbl2 = [name, p.lbl].filter(Boolean).join(" ");
      return { type: "Feature", properties: p, geometry: { type: "Point", coordinates: [o.lon, o.lat] } };
    });
    map.getSource("stations").setData({ type: "FeatureCollection", features: f });
  }

  async function refresh(force = false) {
    if (!s.show || busy) return;
    const sources = Object.keys(s.sources).filter((k) => s.sources[k]);
    if (!sources.length) { obs = []; draw(); $("stn-status").textContent = "Choose at least one source."; return; }
    busy = true;
    $("stn-status").textContent = "Downloading observations…";
    const b = map.getBounds();
    let w = b.getWest(), e = b.getEast();
    if (e - w >= 360) { w = -180; e = 180; }
    const pad = 0.15 * (e - w);
    try {
      const r = await invoke("stations_latest", { bbox: [w - pad, Math.max(-90, b.getSouth() - 1), e + pad, Math.min(90, b.getNorth() + 1)], sources, refresh: force });
      obs = r.obs || [];
      const errs = Object.entries(r.errors || {});
      const newest = obs.reduce((m, o) => Math.max(m, o.t), 0);
      $("stn-status").textContent = `${obs.length} station${obs.length === 1 ? "" : "s"} in view${newest ? `, newest ${new Date(newest * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}` : ""}.` +
        (errs.length ? ` Not available: ${errs.map(([k, v]) => `${SRC[k] ?? k} (${v})`).join("; ")}` : "");
    } catch (err) { $("stn-status").textContent = `Could not download: ${err}`; }
    busy = false;
    draw();
  }

  // ---------- info box: latest and the last 24 h against the GRIBs -----------------------------
  function table(o) {
    const rows = [];
    const add = (k, v) => { if (v != null) rows.push(`<tr><th>${k}</th><td>${esc(v)}</td></tr>`); };
    add("Wind", o.wind_kn != null ? `${o.wind_dir != null ? point(o.wind_dir) + " (" + Math.round(o.wind_dir) + "°) " : ""}${o.wind_kn.toFixed(0)} kn${o.gust_kn != null ? `, gusting ${o.gust_kn.toFixed(0)}` : ""}` : null);
    add("Pressure", o.pressure_hpa != null ? `${o.pressure_hpa.toFixed(1)} hPa` : null);
    add("Air / dew point", o.temp_c != null ? `${o.temp_c.toFixed(1)} °C${o.dewpoint_c != null ? ` / ${o.dewpoint_c.toFixed(1)} °C` : ""}` : null);
    add("Waves", o.wave_m != null ? `${o.wave_m.toFixed(1)} m${o.wave_period_s ? `, ${o.wave_period_s} s` : ""}${o.wave_dir != null ? ` from ${Math.round(o.wave_dir)}°` : ""}` : null);
    add("Sea temperature", o.water_temp_c != null ? `${o.water_temp_c.toFixed(1)} °C` : null);
    add("Visibility", o.vis_km != null ? `${o.vis_km >= 10 ? "10+" : o.vis_km.toFixed(1)} km` : null);
    add("Observed", `${new Date(o.t * 1000).toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit" })} (${Math.round((Date.now() / 1000 - o.t) / 60)} min ago)`);
    add("Position", `${ddm(o.lat, "N", "S")} ${ddm(o.lon, "E", "W")}`);
    add("Source", SRC[o.source] ?? o.source);
    return rows.join("");
  }
  /** SVG: observed wind (solid) and gust (dots) against each GRIB's wind (thin lines), and pressure. */
  function chart(hist, grib) {
    const W = 300, H = 120, PH = 52, L = 26, R = 6, T = 6;
    const t0 = hist[0].t, t1 = Math.max(hist.at(-1).t, Date.now() / 1000);
    const x = (t) => L + ((t - t0) / Math.max(1, t1 - t0)) * (W - L - R);
    const ws = [...hist.flatMap((o) => [o.wind_kn, o.gust_kn]), ...grib.flatMap((g) => g.points.filter((p) => p.t >= t0 && p.t <= t1).map((p) => p.tws))].filter((v) => v != null);
    const wmax = Math.max(10, Math.ceil(Math.max(...ws, 0) / 5) * 5);
    const y = (v) => T + (1 - v / wmax) * (H - T - 14);
    const path = (pts) => pts.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join("");
    const cols = ["#1f6fb2", "#c0392b", "#27ae60", "#8e44ad", "#e67e22"];
    let svg = `<svg viewBox="0 0 ${W} ${H + PH}" class="stn-chart">`;
    for (let v = 0; v <= wmax; v += wmax > 30 ? 10 : 5) svg += `<line x1="${L}" x2="${W - R}" y1="${y(v)}" y2="${y(v)}" class="grid"/><text x="${L - 3}" y="${y(v) + 3}" class="ax">${v}</text>`;
    // hours
    for (let t = Math.ceil(t0 / 21600) * 21600; t <= t1; t += 21600) {
      const d = new Date(t * 1000);
      svg += `<line x1="${x(t)}" x2="${x(t)}" y1="${T}" y2="${H - 14}" class="grid"/><text x="${x(t)}" y="${H - 3}" class="ax mid">${d.getHours().toString().padStart(2, "0")}h</text>`;
    }
    grib.forEach((gr, k) => {
      const pts = gr.points.filter((p) => p.tws != null && p.t >= t0 - 3600 && p.t <= t1 + 3600).map((p) => [x(Math.min(t1, Math.max(t0, p.t))), y(p.tws)]);
      if (pts.length > 1) svg += `<path d="${path(pts)}" fill="none" stroke="${cols[k % cols.length]}" stroke-width="1.3" stroke-dasharray="4 2"/>`;
    });
    const wpts = hist.filter((o) => o.wind_kn != null).map((o) => [x(o.t), y(o.wind_kn)]);
    if (wpts.length) svg += `<path d="${path(wpts)}" fill="none" stroke="#1d2a33" stroke-width="2"/>`;
    for (const o of hist) if (o.gust_kn != null) svg += `<circle cx="${x(o.t)}" cy="${y(o.gust_kn)}" r="1.6" fill="#1d2a33"/>`;
    // pressure panel
    const ps = [...hist.map((o) => o.pressure_hpa), ...grib.flatMap((g) => g.points.filter((p) => p.t >= t0 && p.t <= t1).map((p) => p.mslp))].filter((v) => v != null);
    if (ps.length) {
      const pmin = Math.floor(Math.min(...ps)) - 1, pmax = Math.ceil(Math.max(...ps)) + 1;
      const py = (v) => H + 4 + (1 - (v - pmin) / (pmax - pmin)) * (PH - 10);
      svg += `<text x="${L - 3}" y="${py(pmax) + 6}" class="ax">${pmax}</text><text x="${L - 3}" y="${py(pmin)}" class="ax">${pmin}</text>`;
      grib.forEach((gr, k) => {
        const pts = gr.points.filter((p) => p.mslp != null && p.t >= t0 && p.t <= t1).map((p) => [x(p.t), py(p.mslp)]);
        if (pts.length > 1) svg += `<path d="${path(pts)}" fill="none" stroke="${cols[k % cols.length]}" stroke-width="1" stroke-dasharray="4 2"/>`;
      });
      const pp = hist.filter((o) => o.pressure_hpa != null).map((o) => [x(o.t), py(o.pressure_hpa)]);
      if (pp.length) svg += `<path d="${path(pp)}" fill="none" stroke="#1d2a33" stroke-width="1.5"/>`;
      svg += `<text x="${W - R}" y="${H + 12}" class="ax end">hPa</text>`;
    }
    svg += `<text x="${W - R}" y="${T + 9}" class="ax end">kn</text></svg>`;
    const legend = `<div class="stn-legend"><span><i style="background:#1d2a33"></i>Observed (dots: gusts)</span>${grib.map((gr, k) => `<span><i class="dash" style="border-color:${cols[k % cols.length]}"></i>${esc(gr.name)}</span>`).join("")}</div>`;
    return svg + legend;
  }
  /** Mean wind difference forecast − observed per GRIB, at the observation times. */
  function bias(hist, grib) {
    const out = [];
    for (const gr of grib) {
      const pts = gr.points.filter((p) => p.tws != null);
      if (pts.length < 2) continue;
      let sum = 0, n = 0, dsum = 0, dn = 0;
      for (const o of hist) {
        if (o.wind_kn == null) continue;
        const i = pts.findIndex((p) => p.t >= o.t);
        if (i <= 0) continue;
        const a = pts[i - 1], b = pts[i];
        const f = (o.t - a.t) / (b.t - a.t);
        sum += a.tws + f * (b.tws - a.tws) - o.wind_kn; n++;
        if (o.wind_dir != null && a.twd != null && o.wind_kn > 5) { dsum += ((a.twd - o.wind_dir + 540) % 360) - 180; dn++; }
      }
      if (n >= 2) out.push(`${esc(gr.name)}: ${sum / n >= 0 ? "+" : ""}${(sum / n).toFixed(1)} kn${dn ? `, ${dsum / dn >= 0 ? "+" : ""}${Math.round(dsum / dn)}° veer` : ""} (${n} obs)`);
    }
    return out.length ? `<div class="stn-bias"><b>Forecast minus observed, mean</b><br>${out.join("<br>")}</div>` : "";
  }
  async function open(o) {
    const box = infoBox(map, "station", { width: 360, height: 460 });
    box.open(`<div class="stn-pop"><table>${table(o)}</table><div class="stn-hist">Loading the last 24 h…</div>${o.raw ? `<code>${esc(o.raw)}</code>` : ""}</div>`, { title: o.name, at: [o.lon, o.lat] });
    const el = box.body.querySelector(".stn-hist");
    try {
      const [hist, grib] = await Promise.all([
        invoke("station_history", { id: o.id, lat: o.lat, lon: o.lon, hours: 24 }),
        (async () => { const paths = (weather?.all?.() || []).map((g) => g.path); return paths.length ? invoke("grib_meteogram", { paths, lat: o.lat, lon: o.lon }).catch(() => []) : []; })(),
      ]);
      if (!el.isConnected) return;
      if (!hist?.length) { el.textContent = "No history from this source."; return; }
      el.innerHTML = chart(hist, grib || []) + bias(hist, grib || []) + (grib?.length ? "" : `<div class="wx-note">Load a GRIB covering this place to compare it with the forecast.</div>`);
    } catch (err) { if (el.isConnected) el.textContent = `No history: ${err}`; }
  }
  map.on("click", "stn-dot", (e) => { const f = e.features?.[0]; if (f) open(obs[f.properties.i]); });
  map.on("click", "stn-sym", (e) => { const f = e.features?.[0]; if (f) open(obs[f.properties.i]); });
  for (const l of ["stn-dot", "stn-sym"]) {
    map.on("mouseenter", l, () => { map.getCanvas().style.cursor = "pointer"; });
    map.on("mouseleave", l, () => { map.getCanvas().style.cursor = ""; });
  }

  // ---------- settings ----------------------------------------------------------------
  $("stn-show").checked = s.show;
  for (const k of Object.keys(DEFAULTS.sources)) {
    $(`stn-src-${k}`).checked = !!s.sources[k];
    $(`stn-src-${k}`).onchange = (e) => { s.sources[k] = e.target.checked; save(); refresh(); };
  }
  $("stn-show").onchange = (e) => { s.show = e.target.checked; save(); draw(); if (s.show) refresh(); else $("stn-status").textContent = ""; };
  $("stn-refresh").onclick = () => refresh(true);
  let moveTimer = null;
  map.on("moveend", () => { clearTimeout(moveTimer); moveTimer = setTimeout(() => refresh(), 1500); });
  setInterval(() => refresh(), 10 * 60 * 1000);
  draw();
  if (s.show) setTimeout(() => refresh(), 2000);
  return { refresh, obs: () => obs };
}
