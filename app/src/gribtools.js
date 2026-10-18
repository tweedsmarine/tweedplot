// GRIB tools in the dock:
//  - Meteogram: every loaded GRIB's wind speed, direction and pressure at one
//    point over time (right-click > Meteogram here). Ensembles show their mean
//    with the members' range as a band.
//  - Verify: how well earlier runs forecast the wind, scored against a later
//    run's hour 0 over the chart view, and the track record by model and lead.
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { ddm } from "./navmath.js";

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
const num = (v, d = 1, plus = false) => (v == null ? "—" : `${plus && v > 0 ? "+" : ""}${v.toFixed(d)}`);

export function setupGribTools(map, { weather, dock }) {
  // ---------- meteogram ----------
  let point = null; // [lon, lat]
  let series = null; // last fetch
  let busy = false;
  map.addSource("mg-point", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "mg-point", type: "circle", source: "mg-point",
    paint: { "circle-radius": 6, "circle-color": "rgba(0,0,0,0)", "circle-stroke-color": "#7b2cbf", "circle-stroke-width": 2.5 } });
  function setPoint(p) {
    point = p;
    map.getSource("mg-point").setData({ type: "FeatureCollection",
      features: p ? [{ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: p } }] : [] });
  }

  async function fetchSeries() {
    if (!point) return;
    busy = true;
    series = await invoke("grib_meteogram", { paths: [], lat: point[1], lon: point[0] }).catch(() => []);
    series = (series || []).filter((s) => s.points?.some((q) => q.tws != null || q.mslp != null));
    busy = false;
    dock.refresh("meteogram");
  }

  let cursorLine = null; // the slider-time line in the shown meteogram: (t) => void
  weather.onTime((t) => cursorLine?.(t));
  weather.onLoaded(() => {
    if (point && dock.current() === "meteogram") fetchSeries();
    if (dock.current() === "verify") { vtimes = null; dock.refresh("verify"); }
  });

  dock.register("meteogram", {
    title: "Meteogram",
    render(body, tools) {
      cursorLine = null;
      if (!point) { body.innerHTML = `<p class="wx-note">Right-click the chart and choose Meteogram here.</p>`; return; }
      const clear = document.createElement("button");
      clear.textContent = "Clear point";
      clear.onclick = () => { setPoint(null); series = null; dock.refresh("meteogram"); };
      tools.append(clear);
      const head = document.createElement("div");
      head.className = "mg-head";
      head.textContent = `${ddm(point[1], "N", "S")} ${ddm(((point[0] + 540) % 360) - 180, "E", "W")}`;
      body.append(head);
      if (busy || !series) { body.append(Object.assign(document.createElement("p"), { className: "wx-note", textContent: "Reading the GRIBs…" })); return; }
      if (!series.length) { body.append(Object.assign(document.createElement("p"), { className: "wx-note", textContent: "No loaded GRIB covers this point." })); return; }
      const legend = document.createElement("div");
      legend.className = "mg-legend";
      series.forEach((s, k) => {
        const sp = document.createElement("span");
        sp.innerHTML = `<i style="background:${COLOURS[k % COLOURS.length]}"></i>${esc(label(s))}`;
        legend.append(sp);
      });
      body.append(legend);
      const svg = drawMeteogram(series, Math.max(500, body.clientWidth - 20), (t) => weather.setTime(t));
      body.append(svg.el);
      cursorLine = svg.cursor;
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

  return {
    /** Show the meteogram for a chart point. */
    meteogramAt(p) {
      setPoint(p);
      series = null;
      dock.show("meteogram");
      fetchSeries();
    },
    openVerify() { vtimes = null; dock.show("verify"); },
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

/** The meteogram as SVG: wind speed (with gusts and ensemble range), direction and pressure panels. */
function drawMeteogram(series, W, onPick) {
  const pad = { l: 46, r: 12 };
  const panels = [{ key: "tws", h: 130, title: "TWS kn" }, { key: "twd", h: 90, title: "TWD °T" }, { key: "mslp", h: 80, title: "MSLP hPa" }];
  const gap = 18, top = 6, axisH = 22;
  const H = top + panels.reduce((a, p) => a + p.h + gap, 0) + axisH;
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
  let t0 = Infinity, t1 = -Infinity;
  for (const s of series) for (const p of s.points) { t0 = Math.min(t0, p.t); t1 = Math.max(t1, p.t); }
  if (t1 <= t0) t1 = t0 + 3600;
  const x = (t) => pad.l + ((t - t0) / (t1 - t0)) * (W - pad.l - pad.r);

  // direction: one common unwrapped "mean direction" line through time (circular
  // mean of every GRIB at each time, unwrapped step by step); each GRIB is drawn
  // at its angle from that mean, so GRIBs that agree are drawn together even when
  // the wind swings through north
  const dirs = unwrapAroundMean(series);

  let y0 = top;
  for (const pn of panels) {
    const vals = [];
    series.forEach((s, k) => s.points.forEach((p, i) => {
      const v = pn.key === "twd" ? dirs[k][i] : p[pn.key];
      if (v != null) vals.push(v);
      if (pn.key === "tws") { if (p.gust != null) vals.push(p.gust); if (p.hi != null) vals.push(p.hi, p.lo); }
    }));
    const g = add("g", {});
    add("rect", { x: pad.l, y: y0, width: W - pad.l - pad.r, height: pn.h, class: "mg-bg" }, g);
    add("text", { x: pad.l + 5, y: y0 + 12, class: "mg-title" }, g).textContent = pn.title;
    if (!vals.length) {
      add("text", { x: pad.l + 8, y: y0 + pn.h / 2, class: "mg-tick" }, g).textContent = "no data";
      y0 += pn.h + gap;
      continue;
    }
    let lo = Math.min(...vals), hi = Math.max(...vals);
    if (pn.key === "tws") lo = 0;
    const step = pn.key === "twd" ? (hi - lo > 180 ? 90 : hi - lo > 60 ? 45 : 15) : pn.key === "mslp" ? (hi - lo > 20 ? 10 : hi - lo > 8 ? 4 : 2) : hi > 40 ? 10 : 5;
    lo = Math.floor(lo / step) * step;
    hi = Math.max(lo + step, Math.ceil(hi / step) * step);
    const yv = (v) => y0 + pn.h - ((v - lo) / (hi - lo)) * pn.h;
    for (let v = lo; v <= hi + 1e-9; v += step) {
      add("line", { x1: pad.l, x2: W - pad.r, y1: yv(v), y2: yv(v), class: "mg-grid" }, g);
      const txt = pn.key === "twd" ? String(((Math.round(v) % 360) + 360) % 360).padStart(3, "0") : String(Math.round(v));
      add("text", { x: pad.l - 4, y: yv(v) + 4, class: "mg-tick", "text-anchor": "end" }, g).textContent = txt;
    }
    series.forEach((s, k) => {
      const col = COLOURS[k % COLOURS.length];
      const pts = s.points;
      if (pn.key === "tws" && pts.some((p) => p.hi != null)) {
        const up = pts.filter((p) => p.hi != null).map((p) => `${x(p.t)},${yv(p.hi)}`);
        const dn = pts.filter((p) => p.lo != null).reverse().map((p) => `${x(p.t)},${yv(p.lo)}`);
        add("polygon", { points: [...up, ...dn].join(" "), fill: col, "fill-opacity": 0.15, stroke: "none" }, g);
      }
      const path = (val) => {
        let d = "", pen = false, last = null;
        pts.forEach((p, i) => {
          const v = val(p, i);
          if (v == null) { pen = false; return; }
          if (pn.key === "twd" && last != null && Math.abs(v - last) > 180) pen = false; // wrapped
          last = v;
          d += `${pen ? "L" : "M"}${x(p.t).toFixed(1)},${yv(v).toFixed(1)}`;
          pen = true;
        });
        return d;
      };
      if (pn.key === "tws" && pts.some((p) => p.gust != null)) add("path", { d: path((p) => p.gust), stroke: col, fill: "none", "stroke-width": 1, "stroke-dasharray": "3 3", opacity: 0.8 }, g);
      add("path", { d: path((p, i) => (pn.key === "twd" ? dirs[k][i] : p[pn.key])), stroke: col, fill: "none", "stroke-width": 1.8 }, g);
    });
    y0 += pn.h + gap;
  }
  // time axis: a tick each day at 00 local, light ticks every 6 h
  const span = t1 - t0;
  const minor = span > 10 * 86400 ? 86400 : span > 3 * 86400 ? 21600 : 10800;
  for (let t = Math.ceil(t0 / minor) * minor; t <= t1; t += minor) {
    add("line", { x1: x(t), x2: x(t), y1: top, y2: y0 - gap, class: "mg-vgrid" });
  }
  const lab = span > 10 * 86400 ? 2 * 86400 : 86400;
  for (let t = Math.ceil(t0 / lab) * lab; t <= t1; t += lab) {
    add("text", { x: x(t), y: y0 - gap + 15, class: "mg-tick", "text-anchor": "middle" }).textContent = fmt(t, true).replace(/, \d\d:\d\d$/, "");
  }
  const cur = add("line", { x1: 0, x2: 0, y1: top, y2: y0 - gap, class: "mg-cursor" });
  const readout = add("text", { x: W - pad.r, y: H - 4, class: "mg-tick", "text-anchor": "end" });
  svg.addEventListener("click", (e) => {
    const r = svg.getBoundingClientRect();
    const t = t0 + ((e.clientX - r.left - pad.l) / (W - pad.l - pad.r)) * (t1 - t0);
    if (t >= t0 && t <= t1) onPick(Math.round(t / 900) * 900);
  });
  return {
    el: svg,
    cursor(t) {
      const show = t != null && t >= t0 && t <= t1;
      cur.style.display = show ? "" : "none";
      if (!show) { readout.textContent = ""; return; }
      cur.setAttribute("x1", x(t));
      cur.setAttribute("x2", x(t));
      readout.textContent = fmt(t);
    },
  };
}
