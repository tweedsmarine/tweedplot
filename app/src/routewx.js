// Weather along the route (dock view): the forecast the boat meets on the way, from
// the active routing (its own times and positions) or, with no routing, the course
// sailed at a set speed from the weather slider's time. Stacked strips like the
// meteogram: wind and gusts with direction arrows, waves (Hs, Tp, direction) by
// default; pressure, rain, air temperature, CAPE, sea temperature and current to tick.
// Hover a strip: the boat's place on the chart and the values there; click: the slider
// moves to that time.
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { positionAt } from "./routing.js";

const NS = "http://www.w3.org/2000/svg";
const KEY = "racenav.routewx";
const NM = 1852;
/** The extra strips, ticked as wanted. */
const EXTRAS = [
  { key: "mslp", label: "Pressure", unit: "hPa", col: "#5b6b75" },
  { key: "rain", label: "Rain", unit: "mm/h", col: "#2a62c9", bars: true },
  { key: "temp", label: "Air temp", unit: "°C", col: "#c0392b" },
  { key: "cape", label: "CAPE", unit: "J/kg", col: "#8e44ad", bars: true },
  { key: "sst", label: "Sea temp", unit: "°C", col: "#006994" },
  { key: "cur", label: "Current", unit: "kn", col: "#1abc9c" },
];

export function setupRouteWeather(map, { weather, routing, course, dock }) {
  let opts = { speed: 6, wind: true, waves: true, extras: [] };
  try { Object.assign(opts, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(opts)); } catch { /* ignore */ } };
  let data = null; // { from, rows: [{ t, lon, lat, nm, twa?, … values }] }
  let busy = false;

  map.addSource("rw-point", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "rw-point", type: "circle", source: "rw-point",
    paint: { "circle-radius": 7, "circle-color": "rgba(255,194,41,0.35)", "circle-stroke-color": "#00274d", "circle-stroke-width": 2.5 } });
  const mark = (p) => map.getSource("rw-point").setData({ type: "FeatureCollection",
    features: p ? [{ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: [p.lon, p.lat] } }] : [] });
  dock.onChange((cur) => { if (cur !== "routewx") mark(null); else refetch(); });

  /** The track to sample: routing points, else the course at the set speed. */
  function track() {
    const pts = routing.activePoints();
    if (pts?.length > 1) return { from: "routing", pts };
    const c = course.get();
    const seq = [c.start ? { lon: c.start[0] ?? c.start.lon, lat: c.start[1] ?? c.start.lat } : null, ...c.marks].filter(Boolean);
    if (seq.length < 2) return null;
    const t0 = weather.current().t;
    if (t0 == null) return null;
    const kn = Math.max(0.5, Number(opts.speed) || 6);
    let t = t0;
    const out = [{ t, lon: seq[0].lon, lat: seq[0].lat }];
    for (let i = 1; i < seq.length; i++) {
      t += (dist(seq[i - 1], seq[i]) / NM / kn) * 3600;
      out.push({ t: Math.round(t), lon: seq[i].lon, lat: seq[i].lat });
    }
    return { from: "course", pts: out, kn };
  }

  async function refetch() {
    const tr = track();
    if (!tr) { data = null; dock.refresh("routewx"); return; }
    const t0 = tr.pts[0].t, t1 = tr.pts.at(-1).t;
    // samples at even times along the track: about 240 of them, at least every 10 min
    const step = Math.max(600, Math.ceil((t1 - t0) / 240 / 300) * 300);
    const rows = [];
    let nm = 0, prev = null;
    for (let t = t0; ; t = Math.min(t + step, t1)) {
      const p = positionAt(tr.pts, t);
      if (p) {
        if (prev) nm += dist(prev, p) / NM;
        rows.push({ t, lon: p.lon, lat: p.lat, nm, twa: tr.from === "routing" ? p.twa : null });
        prev = p;
      }
      if (t >= t1) break;
    }
    busy = true;
    dock.refresh("routewx");
    const vals = await invoke("grib_along", { first: weather.current().path ?? null, pts: rows.map((r) => [r.lat, r.lon, r.t]) }).catch(() => []);
    busy = false;
    rows.forEach((r, i) => Object.assign(r, vals[i] || {}));
    data = { from: tr.from, kn: tr.kn, rows };
    dock.refresh("routewx");
  }
  let timer = null;
  const later = () => { clearTimeout(timer); timer = setTimeout(() => { if (dock.current() === "routewx") refetch(); }, 300); };
  weather.onLoaded(later);
  routing.onDraw?.(later);
  course.onChange?.(later);
  weather.onTime(() => { if (data?.from === "course") later(); else cursorLine?.(weather.current().t); });
  let cursorLine = null;

  dock.register("routewx", {
    title: "Weather on route",
    render(body, tools) {
      cursorLine = null;
      // which strips
      const pick = document.createElement("div");
      pick.className = "rw-pick";
      const box = (label, on, set, title = "") => {
        const l = document.createElement("label");
        l.title = title;
        l.innerHTML = `<input type="checkbox"${on ? " checked" : ""}> ${label}`;
        l.querySelector("input").onchange = (e) => { set(e.target.checked); save(); dock.refresh("routewx"); };
        return l;
      };
      pick.append(box("Wind", opts.wind, (v) => (opts.wind = v)), box("Waves", opts.waves, (v) => (opts.waves = v)));
      for (const x of EXTRAS) pick.append(box(x.label, opts.extras.includes(x.key), (v) => { opts.extras = v ? [...new Set([...opts.extras, x.key])] : opts.extras.filter((k) => k !== x.key); }));
      const sp = document.createElement("label");
      sp.title = "With no routing: the course sailed at this speed from the weather slider's time";
      sp.innerHTML = `Course at <input type="number" min="0.5" max="40" step="0.5" value="${opts.speed}"> kn`;
      sp.querySelector("input").onchange = (e) => { opts.speed = Number(e.target.value) || 6; save(); refetch(); };
      pick.append(sp);
      body.append(pick);
      if (busy && !data) { body.innerHTML = `<p class="wx-note">Reading the GRIBs along the route…</p>`; return; }
      if (!data) { body.innerHTML = `<p class="wx-note">Compute a routing, or set a course (start and marks), and load a GRIB.</p>`; return; }
      const r0 = data.rows[0], r1 = data.rows.at(-1);
      const head = document.createElement("div");
      head.className = "mg-head";
      head.textContent = data.from === "routing"
        ? `Along the active routing: ${fmt(r0.t)} to ${fmt(r1.t)}, ${r1.nm.toFixed(0)} NM`
        : `Along the course at ${data.kn} kn from ${fmt(r0.t)}: ${r1.nm.toFixed(0)} NM, ${hm(r1.t - r0.t)}`;
      body.append(head);
      const strips = [];
      if (opts.wind) strips.push({ key: "wind", title: "Wind kn (gusts dashed) · direction", h: 120 });
      if (opts.waves) strips.push({ key: "waves", title: "Waves Hs m · Tp s · direction", h: 90 });
      for (const x of EXTRAS) if (opts.extras.includes(x.key)) strips.push({ ...x, title: `${x.label} ${x.unit}`, h: 64 });
      if (!data.rows.some((r) => r.tws != null || r.wave != null)) {
        body.append(Object.assign(document.createElement("p"), { className: "wx-note", textContent: "No loaded GRIB covers this route at these times." }));
      }
      const readout = document.createElement("div");
      readout.className = "rw-readout";
      const svg = draw(data.rows, strips, Math.max(520, body.clientWidth - 20), {
        hover: (r) => { mark(r); readout.textContent = r ? describe(r) : ""; },
        pick: (t) => weather.setTime(t),
      });
      body.append(svg.el, readout);
      cursorLine = svg.cursor;
      cursorLine(weather.current().t);
    },
  });

  return {
    /** Open the view (Routing drawer button). */
    open() { if (dock.current() === "routewx") refetch(); else dock.show("routewx"); },
  };
}

function describe(r) {
  const d3 = (x) => String(Math.round(x) % 360).padStart(3, "0");
  const parts = [fmt(r.t), `${r.nm.toFixed(1)} NM`];
  if (r.tws != null) parts.push(`${r.tws.toFixed(1)} kn${r.gust != null ? ` (gust ${r.gust.toFixed(0)})` : ""} from ${d3(r.twd)}°`);
  if (r.twa != null) parts.push(`TWA ${Math.round(Math.abs(r.twa))}° ${r.twa < 0 ? "port" : "stbd"}`);
  if (r.wave != null) parts.push(`Hs ${r.wave.toFixed(1)} m${r.period != null ? ` / ${r.period} s` : ""}${r.wavedir != null ? ` from ${d3(r.wavedir)}°` : ""}`);
  if (r.mslp != null) parts.push(`${r.mslp.toFixed(0)} hPa`);
  if (r.rain != null) parts.push(`rain ${r.rain.toFixed(1)} mm/h`);
  if (r.temp != null) parts.push(`air ${r.temp.toFixed(1)}°C`);
  if (r.cape != null) parts.push(`CAPE ${Math.round(r.cape)}`);
  if (r.sst != null) parts.push(`sea ${r.sst.toFixed(1)}°C`);
  if (r.cur) parts.push(`current ${r.cur.kn.toFixed(1)} kn ${d3(r.cur.set)}°`);
  return parts.join(" · ");
}

/** The strips as SVG over time, with distance marked along the bottom. */
function draw(rows, strips, W, { hover, pick }) {
  const pad = { l: 46, r: 40 };
  const gap = 20, top = 16, axisH = 34;
  const H = top + strips.reduce((a, p) => a + p.h + gap, 0) + axisH;
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
  const t0 = rows[0].t, t1 = Math.max(rows.at(-1).t, t0 + 600);
  const x = (t) => pad.l + ((t - t0) / (t1 - t0)) * (W - pad.l - pad.r);
  const val = (r, k) => (k === "cur" ? r.cur?.kn ?? null : r[k] ?? null);
  // arrows along the top of a strip every so many px: pointing where the wind/waves go
  const arrows = (g, y, key, col) => {
    let lastX = -1e9;
    for (const r of rows) {
      const d = r[key];
      if (d == null || x(r.t) - lastX < 26) continue;
      lastX = x(r.t);
      const a = ((d + 180) % 360) * Math.PI / 180, cx = x(r.t), L = 7;
      const dx = Math.sin(a) * L, dy = -Math.cos(a) * L;
      add("line", { x1: cx - dx, y1: y - dy, x2: cx + dx, y2: y + dy, stroke: col, "stroke-width": 1.6 }, g);
      add("circle", { cx: cx + dx, cy: y + dy, r: 1.8, fill: col }, g);
    }
  };
  let y0 = top;
  for (const st of strips) {
    const g = add("g", {});
    add("rect", { x: pad.l, y: y0, width: W - pad.l - pad.r, height: st.h, class: "mg-bg" }, g);
    add("text", { x: pad.l, y: y0 - 4, class: "mg-title" }, g).textContent = st.title;
    const keys = st.key === "wind" ? ["tws", "gust"] : st.key === "waves" ? ["wave"] : [st.key];
    const vs = rows.flatMap((r) => keys.map((k) => val(r, k))).filter((v) => v != null);
    if (!vs.length) {
      add("text", { x: pad.l + 8, y: y0 + st.h / 2 + 4, class: "mg-tick" }, g).textContent = "not in the loaded GRIBs";
      y0 += st.h + gap;
      continue;
    }
    let lo = ["mslp", "temp", "sst"].includes(st.key) ? Math.min(...vs) : 0, hi = Math.max(...vs);
    const raw = (hi - lo) / 4 || 1, p10 = 10 ** Math.floor(Math.log10(raw));
    const step = [1, 2, 2.5, 5, 10].map((m) => m * p10).find((s) => s >= raw);
    lo = Math.floor(lo / step) * step;
    hi = Math.max(lo + step, Math.ceil(hi / step) * step);
    const top2 = st.key === "wind" || st.key === "waves" ? 18 : 0; // room for the arrows
    const yv = (v) => y0 + st.h - ((v - lo) / (hi - lo)) * (st.h - top2);
    for (let v = lo; v <= hi + 1e-9; v += step) {
      add("line", { x1: pad.l, x2: W - pad.r, y1: yv(v), y2: yv(v), class: "mg-grid" }, g);
      add("text", { x: pad.l - 4, y: yv(v) + 4, class: "mg-tick", "text-anchor": "end" }, g).textContent = String(+v.toFixed(2));
    }
    const path = (k) => {
      let d = "", pen = false;
      for (const r of rows) {
        const v = val(r, k);
        if (v == null) { pen = false; continue; }
        d += `${pen ? "L" : "M"}${x(r.t).toFixed(1)},${yv(v).toFixed(1)}`;
        pen = true;
      }
      return d;
    };
    if (st.key === "wind") {
      add("path", { d: path("gust"), stroke: "#c0392b", fill: "none", "stroke-width": 1.2, "stroke-dasharray": "4 3" }, g);
      add("path", { d: path("tws"), stroke: "#006994", fill: "none", "stroke-width": 2 }, g);
      arrows(g, y0 + 10, "twd", "#00274d");
    } else if (st.key === "waves") {
      add("path", { d: path("wave"), stroke: "#2a62c9", fill: "none", "stroke-width": 2 }, g);
      // Tp: its own scale on the right
      const ps = rows.map((r) => r.period).filter((v) => v != null);
      if (ps.length) {
        const pl = 0, ph = Math.max(4, Math.ceil(Math.max(...ps) / 4) * 4);
        const yp = (v) => y0 + st.h - ((v - pl) / (ph - pl)) * (st.h - top2);
        let d = "", pen = false;
        for (const r of rows) { if (r.period == null) { pen = false; continue; } d += `${pen ? "L" : "M"}${x(r.t).toFixed(1)},${yp(r.period).toFixed(1)}`; pen = true; }
        add("path", { d, stroke: "#8a96a0", fill: "none", "stroke-width": 1.3, "stroke-dasharray": "2 2" }, g);
        for (const v of [0, ph / 2, ph]) add("text", { x: W - pad.r + 4, y: yp(v) + 4, class: "mg-tick" }, g).textContent = `${v} s`;
      }
      arrows(g, y0 + 10, "wavedir", "#2a62c9");
    } else if (st.bars) {
      const bw = Math.max(1, (W - pad.l - pad.r) / rows.length - 0.5);
      for (const r of rows) {
        const v = val(r, st.key);
        if (!v) continue;
        add("rect", { x: x(r.t) - bw / 2, y: yv(v), width: bw, height: yv(lo) - yv(v), fill: st.col, opacity: 0.7 }, g);
      }
    } else {
      add("path", { d: path(st.key), stroke: st.col, fill: "none", "stroke-width": 1.8 }, g);
    }
    y0 += st.h + gap;
  }
  // time along the bottom (local days and 6-hourly ticks), distance below it
  const span = t1 - t0;
  const minor = span > 5 * 86400 ? 21600 : span > 36 * 3600 ? 10800 : 3600;
  for (let t = Math.ceil(t0 / minor) * minor; t <= t1; t += minor) add("line", { x1: x(t), x2: x(t), y1: top, y2: y0 - gap, class: "mg-vgrid" });
  // labels at local midnights (date) or local hours, spaced so they never overlap
  const off = -new Date(t0 * 1000).getTimezoneOffset() * 60; // local time = UTC + off
  const fits = (sec) => (sec / span) * (W - pad.l - pad.r) >= 96;
  const lab = [3600, 10800, 21600, 43200, 86400, 2 * 86400, 4 * 86400, 7 * 86400].find(fits) ?? 7 * 86400;
  const label = (t) => {
    const d = new Date(t * 1000);
    if (lab >= 86400 || (((t + off) % 86400) + 86400) % 86400 === 0) return d.toLocaleDateString("en-NZ", { weekday: "short", day: "numeric", month: "short" });
    return d.toLocaleTimeString("en-NZ", { hour: "2-digit", minute: "2-digit", hour12: false });
  };
  const first = Math.ceil((t0 + off) / Math.min(lab, 86400)) * Math.min(lab, 86400) - off;
  for (let t = first; t <= t1; t += lab) {
    add("text", { x: x(t), y: y0 - gap + 14, class: "mg-tick", "text-anchor": "middle" }).textContent = label(t);
    const r = rows.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a));
    add("text", { x: x(t), y: y0 - gap + 27, class: "mg-tick", "text-anchor": "middle" }).textContent = `${r.nm.toFixed(0)} NM`;
  }
  const cur = add("line", { x1: 0, x2: 0, y1: top, y2: y0 - gap, class: "mg-cursor" });
  const hov = add("line", { x1: 0, x2: 0, y1: top, y2: y0 - gap, stroke: "#ffc229", "stroke-width": 2, display: "none" });
  const at = (e) => {
    const r = svg.getBoundingClientRect();
    const t = t0 + ((e.clientX - r.left - pad.l) / (W - pad.l - pad.r)) * (t1 - t0);
    return t >= t0 && t <= t1 ? t : null;
  };
  const nearest = (t) => rows.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a));
  svg.addEventListener("mousemove", (e) => {
    const t = at(e);
    if (t == null) { hov.setAttribute("display", "none"); hover(null); return; }
    const r = nearest(t);
    hov.setAttribute("x1", x(r.t)); hov.setAttribute("x2", x(r.t)); hov.setAttribute("display", "");
    hover(r);
  });
  svg.addEventListener("mouseleave", () => { hov.setAttribute("display", "none"); hover(null); });
  svg.addEventListener("click", (e) => { const t = at(e); if (t != null) pick(Math.round(nearest(t).t / 300) * 300); });
  return {
    el: svg,
    cursor(t) {
      const show = t != null && t >= t0 && t <= t1;
      cur.style.display = show ? "" : "none";
      if (show) { cur.setAttribute("x1", x(t)); cur.setAttribute("x2", x(t)); }
    },
  };
}

/** Great-circle distance in metres between {lon, lat} points. */
function dist(a, b) {
  const R = 6371008.8, r = Math.PI / 180;
  const dLat = (b.lat - a.lat) * r, dLon = (b.lon - a.lon) * r;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a.lat * r) * Math.cos(b.lat * r) * Math.sin(dLon / 2) ** 2;
  return 2 * R * Math.asin(Math.min(1, Math.sqrt(h)));
}
function hm(s) {
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return `${h} h ${String(m).padStart(2, "0")}`;
}
