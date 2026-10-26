// Weather stations and buoys on the chart (Weather › Weather stations and
// buoys): the latest wind at each, and an info box with the last 24 h against the
// loaded GRIBs (real versus forecast).
import { infoBox } from "./infobox.js";
import { invoke } from "@tauri-apps/api/core";
import { PALETTE } from "./weather.js";
import { ddm } from "./navmath.js";

const KEY = "racenav.stations";
const DEFAULTS = { show: false, sources: { metservice: true, metar: true, ndbc: false } };
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const SRC = { metservice: "MetService (open access)", metar: "aviationweather.gov METAR", ndbc: "NOAA NDBC" };
const COMPASS = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
const point = (d) => COMPASS[Math.round(((d % 360) + 360) % 360 / 22.5) % 16];

function windColour(kn) {
  let c = PALETTE[0][1];
  for (const [k, col] of PALETTE) if (kn >= k) c = col;
  return c;
}

export function setupStations(map, { weather }) {
  let s = { ...DEFAULTS, sources: { ...DEFAULTS.sources } };
  try { const j = JSON.parse(localStorage.getItem(KEY) || "{}"); Object.assign(s, j); s.sources = { ...DEFAULTS.sources, ...(j.sources || {}) }; } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ } };
  let obs = [];
  let busy = false;

  // arrow pointing up (rotated to the downwind direction)
  const c = document.createElement("canvas");
  c.width = c.height = 32;
  const g = c.getContext("2d");
  g.fillStyle = "#fff";
  g.beginPath(); g.moveTo(16, 2); g.lineTo(24, 16); g.lineTo(18.5, 15); g.lineTo(18.5, 30); g.lineTo(13.5, 30); g.lineTo(13.5, 15); g.lineTo(8, 16); g.closePath(); g.fill();
  map.addImage("stn-arrow", g.getImageData(0, 0, 32, 32), { sdf: true });
  map.addSource("stations", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "stn-dot", type: "circle", source: "stations",
    paint: { "circle-radius": 7.2, "circle-color": ["get", "col"], "circle-stroke-color": "#1d2a33", "circle-stroke-width": 1.6 } });
  map.addLayer({ id: "stn-arrow", type: "symbol", source: "stations", filter: ["has", "rot"],
    layout: { "icon-image": "stn-arrow", "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map", "icon-allow-overlap": true, "icon-ignore-placement": true,
      "icon-size": ["interpolate", ["linear"], ["get", "kn"], 0, 0.72, 30, 1.52] },
    paint: { "icon-color": ["get", "col"], "icon-halo-color": "#1d2a33", "icon-halo-width": 1.6 } });
  map.addLayer({ id: "stn-label", type: "symbol", source: "stations", minzoom: 6.5, filter: ["has", "lbl"],
    layout: { "icon-image": ["concat", "lbl:", ["get", "lbl"]], "icon-anchor": "left", "icon-offset": [16, 0], "icon-size": 1.0, "icon-allow-overlap": true, "icon-ignore-placement": true } });

  function draw() {
    const vis = s.show ? "visible" : "none";
    for (const id of ["stn-dot", "stn-arrow", "stn-label"]) map.setLayoutProperty(id, "visibility", vis);
    const f = obs.map((o, i) => {
      const kn = o.wind_kn;
      const p = { i, col: kn != null ? windColour(kn) : "#b0bac2", kn: kn ?? 0 };
      if (o.wind_dir != null && kn != null && kn >= 1) p.rot = (o.wind_dir + 180) % 360;
      const w = kn != null ? `${Math.round(kn)}${o.gust_kn != null && o.gust_kn > kn + 2 ? "G" + Math.round(o.gust_kn) : ""}` : "";
      const wave = o.wave_m != null ? ` ${o.wave_m.toFixed(1)}m` : "";
      if (w || wave) p.lbl = `${w}${wave}`;
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
  map.on("click", "stn-arrow", (e) => { const f = e.features?.[0]; if (f) open(obs[f.properties.i]); });
  for (const l of ["stn-dot", "stn-arrow"]) {
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
