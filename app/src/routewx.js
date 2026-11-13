// Weather along the route (dock view): the forecast the boat meets on the way, from
// the active routing (its own times and positions) or, with no routing, the course
// sailed at a set speed from a fixed departure. Stacked strips like the
// meteogram: wind and gusts with direction arrows, waves (Hs, Tp, direction) by
// default; pressure, rain, air temperature, CAPE, sea temperature and current to tick.
// Select a point to inspect it without changing the route's departure.
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { positionAt } from "./routing.js";
import { courseTrack, sampleTrack, coverage, createRouteWeatherRequest } from "./routewx-core.js";

const NS = "http://www.w3.org/2000/svg";
const KEY = "racenav.routewx";
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
  let opts = { speed: 6, departure: null, wind: true, waves: true, extras: [] };
  try { Object.assign(opts, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  if (!Number.isFinite(opts.speed) || opts.speed < 0.5 || opts.speed > 40) opts.speed = 6;
  if (!Number.isFinite(opts.departure) || opts.departure <= 0) opts.departure = null;
  if (!Array.isArray(opts.extras)) opts.extras = [];
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(opts)); } catch { /* ignore */ } };
  let selected = -1;
  let timer = null;
  let lastPath = weather.current().path;
  const request = createRouteWeatherRequest(() => dock.refresh("routewx"));

  map.addSource("rw-point", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "rw-point", type: "circle", source: "rw-point",
    paint: { "circle-radius": 7, "circle-color": "rgba(255,194,41,0.35)", "circle-stroke-color": "#00274d", "circle-stroke-width": 2.5 } });
  const mark = (p) => map.getSource("rw-point").setData({ type: "FeatureCollection",
    features: p ? [{ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: [p.lon, p.lat] } }] : [] });
  function invalidate(status = "stale") {
    clearTimeout(timer);
    selected = -1;
    mark(null);
    request.invalidate(status);
  }
  function refetch() {
    clearTimeout(timer);
    selected = -1;
    mark(null);
    const current = weather.current();
    if (!current.path || !weather.sources().length) { request.invalidate("no-forecast"); return; }
    if (opts.departure == null && current.t > 0) { opts.departure = current.t; save(); }
    const c = course.get();
    const active = routing.activePoints();
    if (!(active?.length > 1) && (!c.start || !c.marks.length)) { request.invalidate("no-route"); return; }
    const sources = weather.sources();
    request.run(async () => {
      const from = active?.length > 1 ? "routing" : "course";
      const pts = from === "routing" ? active.map((p) => ({ ...p })) : courseTrack(c, opts.departure, opts.speed);
      const rows = sampleTrack(pts, positionAt, from === "routing");
      const vals = await invoke("grib_along", { first: current.path, pts: rows.map((r) => [r.lat, r.lon, r.t]) });
      if (!Array.isArray(vals) || vals.length !== rows.length || vals.some((v) => !v || typeof v !== "object" || Array.isArray(v))) {
        throw new Error("The forecast returned an incomplete result.");
      }
      rows.forEach((r, i) => Object.assign(r, vals[i]));
      return { from, name: from === "routing" ? routing.activeName() || "Active routing" : c.name || "Unsaved route",
        kn: opts.speed, rows, sources, path: current.path };
    });
  }
  const later = () => {
    if (dock.current() !== "routewx") return;
    invalidate();
    timer = setTimeout(refetch, 300);
  };
  dock.onChange((cur) => { if (cur !== "routewx") invalidate("empty"); else refetch(); });
  weather.onLoaded(later);
  weather.onChange(() => { const path = weather.current().path; if (path !== lastPath) { lastPath = path; later(); } });
  routing.onDraw?.(later);
  course.onChange?.(later);

  dock.register("routewx", {
    title: "Weather on route",
    render(body) {
      const state = request.state(), data = state.data;
      const context = document.createElement("div");
      context.className = "rw-context";
      const active = routing.activePoints();
      const routeName = active?.length > 1 ? routing.activeName() || "Active routing" : course.get().name || "Unsaved route";
      const title = document.createElement("strong");
      title.textContent = `${active?.length > 1 ? "Routing" : "Route"}: ${routeName}`;
      context.append(title);
      if (active?.length > 1) {
        const info = document.createElement("span");
        info.textContent = "Uses the active routing's own times and positions.";
        context.append(info);
      } else {
        const departure = document.createElement("label");
        departure.textContent = "Departure (local): ";
        const input = document.createElement("input");
        input.type = "datetime-local";
        input.value = opts.departure ? toLocalInput(opts.departure) : "";
        departure.append(input);
        const speed = document.createElement("label");
        speed.textContent = "Speed (kn): ";
        const sp = document.createElement("input");
        sp.type = "number"; sp.min = "0.5"; sp.max = "40"; sp.step = "0.5"; sp.value = String(opts.speed);
        speed.append(sp);
        const apply = document.createElement("button");
        apply.textContent = "Apply and recalculate";
        apply.onclick = () => {
          const t = Date.parse(input.value) / 1000, kn = Number(sp.value);
          if (!input.value || !Number.isFinite(t) || t <= 0 || !sp.value || !Number.isFinite(kn) || kn < 0.5 || kn > 40) {
            input.setCustomValidity(!input.value || !Number.isFinite(t) || t <= 0 ? "Enter a valid departure time." : "");
            sp.setCustomValidity(!sp.value || !Number.isFinite(kn) || kn < 0.5 || kn > 40 ? "Enter a speed from 0.5 to 40 kn." : "");
            if (!input.reportValidity()) return;
            sp.reportValidity();
            return;
          }
          input.setCustomValidity(""); sp.setCustomValidity("");
          opts.departure = Math.round(t); opts.speed = kn; save(); refetch();
        };
        input.oninput = () => input.setCustomValidity("");
        sp.oninput = () => sp.setCustomValidity("");
        context.append(departure, speed, apply);
      }
      body.append(context);
      if (active?.length > 1 && routing.activeStale()) {
        const warning = document.createElement("div");
        warning.className = "rw-status";
        warning.textContent = "The active routing was computed for a different route. Recompute it before relying on these results.";
        body.append(warning);
      }
      const source = document.createElement("div");
      source.className = "rw-source wx-note";
      const sources = data?.sources ?? weather.sources();
      source.textContent = sources.length ? `Forecast sources (shown file first): ${sources.slice().sort((a, b) => Number(b.path === (data?.path ?? weather.current().path)) - Number(a.path === (data?.path ?? weather.current().path))).map((s) => `${s.name}${s.ref ? ` · run ${runTime(s.ref)}` : " · run time unavailable"}`).join("; ")}` : "No forecast loaded.";
      body.append(source);
      if (state.status !== "ready") {
        const status = document.createElement("div");
        status.className = `rw-status${state.status === "error" ? " error" : ""}`;
        status.setAttribute("role", "status");
        status.textContent = {
          loading: "Calculating weather along the route. Previous results are hidden.",
          stale: "Route or forecast changed. Previous results are hidden while the forecast is updated.",
          "no-route": "Set a route with a start and marks, or activate a weather routing.",
          "no-forecast": "Load a forecast in Weather before viewing weather on the route.",
          error: `Could not calculate weather along the route: ${state.error}`,
          empty: "No route forecast calculated.",
        }[state.status];
        body.append(status);
        if (state.status === "error" || state.status === "no-route" || state.status === "no-forecast") {
          const retry = document.createElement("button");
          retry.textContent = "Retry";
          retry.onclick = refetch;
          body.append(retry);
        }
        return;
      }
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
      body.append(pick);
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
      const fields = [...(opts.wind ? [{ key: "tws", label: "Wind" }] : []), ...(opts.waves ? [{ key: "wave", label: "Waves" }] : []), ...EXTRAS.filter((x) => opts.extras.includes(x.key))];
      const gaps = coverage(data.rows, fields).filter((x) => x.count < x.total);
      if (gaps.length) {
        const warning = document.createElement("div");
        warning.className = "rw-status";
        warning.textContent = `Forecast coverage: ${gaps.map((x) => `${x.label} ${x.count === 0 ? "not available" : `missing at ${x.total - x.count} of ${x.total} sampled points`}`).join("; ")}. Gaps do not mean calm conditions.`;
        body.append(warning);
      }
      const readout = document.createElement("div");
      readout.className = "rw-readout";
      const inspect = document.createElement("div");
      inspect.className = "rw-inspect";
      const previous = document.createElement("button"), next = document.createElement("button");
      previous.textContent = "◀ Previous"; next.textContent = "Next ▶";
      const update = (index, moveTime = true) => {
        selected = Math.max(0, Math.min(data.rows.length - 1, index));
        const r = data.rows[selected];
        mark(r);
        const used = [{ key: "tws", label: "Wind" }, { key: "gust", label: "Gusts" }, { key: "wave", label: "Waves" },
          { key: "period", label: "Wave period" }, ...EXTRAS].filter((f) => r[f.key] != null && r._sources?.[f.key])
          .map((f) => `${f.label}: ${data.sources.find((s) => s.path === r._sources[f.key])?.name ?? r._sources[f.key]}`);
        const { min, max } = weather.layerInfo();
        const inRange = min != null && max != null && r.t >= min && r.t <= max;
        const note = inRange ? "" : " · Outside the available chart forecast times; chart time and cursor forecast unchanged";
        if (moveTime) {
          if (inRange) {
            weather.setTime(r.t);
            weather.setCursor([r.lon, r.lat]);
          }
        }
        const noValues = fields.length && fields.every((f) => r[f.key] == null);
        readout.textContent = `Inspecting: ${describe(r)}${noValues ? " · No selected forecast data at this point" : ""}${used.length ? ` · Sources: ${used.join(", ")}` : ""}${note}`;
        previous.disabled = selected === 0; next.disabled = selected === data.rows.length - 1;
        svg.cursor(r.t);
      };
      const svg = draw(data.rows, strips, Math.max(520, body.clientWidth - 20), {
        pick: (r) => update(data.rows.indexOf(r)),
      });
      previous.onclick = () => update(selected < 0 ? 0 : selected - 1);
      next.onclick = () => update(selected < 0 ? 0 : selected + 1);
      inspect.append(readout, previous, next);
      body.append(svg.el, inspect);
      if (selected >= 0 && selected < data.rows.length) update(selected, false);
      else readout.textContent = "Select a point on the graph, or use Previous / Next, to inspect it.";
    },
  });

  return {
    /** Open the view (Routing drawer button). */
    open() { if (dock.current() === "routewx") return; dock.show("routewx"); },
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
function draw(rows, strips, W, { pick }) {
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
  const at = (e) => {
    const r = svg.getBoundingClientRect();
    const t = t0 + ((e.clientX - r.left - pad.l) / (W - pad.l - pad.r)) * (t1 - t0);
    return t >= t0 && t <= t1 ? t : null;
  };
  const nearest = (t) => rows.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a));
  svg.addEventListener("click", (e) => { const t = at(e); if (t != null) pick(nearest(t)); });
  return {
    el: svg,
    cursor(t) {
      const show = t != null && t >= t0 && t <= t1;
      cur.style.display = show ? "" : "none";
      if (show) { cur.setAttribute("x1", x(t)); cur.setAttribute("x2", x(t)); }
    },
  };
}

function toLocalInput(sec) {
  const d = new Date(sec * 1000), z = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())}T${z(d.getHours())}:${z(d.getMinutes())}`;
}
function runTime(sec) {
  const age = Math.floor((Date.now() / 1000 - sec) / 3600);
  return `${new Date(sec * 1000).toISOString().replace("T", " ").slice(0, 16)} UTC${age >= 0 ? ` · ${age} h old${age > 24 ? " (old)" : ""}` : " · future run time"}`;
}
function hm(s) {
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return `${h} h ${String(m).padStart(2, "0")}`;
}
