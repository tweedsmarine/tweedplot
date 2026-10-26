// Gate sweep: a row of gates (points along a meridian or a parallel) on the
// way to one of the course marks, and one routing through each gate, with the
// GRIB shown or with every loaded GRIB, using the start time and settings of
// a normal Compute. The dock's Gates table compares them: arrival per gate and
// GRIB, the time lost against the best gate, and (with several GRIBs) which
// gate holds up best across all of them.
import { fmt } from "./weather.js";
import { hm } from "./stats.js";

const KEY = "racenav.gates";
const DEFAULTS = { axis: "lon", line: "", from: "", to: "", n: 6, wx: "shown" };
const PARALLEL = 2; // routings run at once (each already uses every core)
const VIOLET = "#7b2cbf";
const NS = "http://www.w3.org/2000/svg";
const LINE_COLOURS = ["#1f77b4", "#d62728", "#2ca02c", "#9467bd", "#ff7f0e", "#17becf", "#8c564b", "#e377c2"];

/** "35S", "35 30S", "35°30.5'S", "-35.5", "S35", "005 00E" → degrees (negative S / W); null if not one. */
export function parseCoord(text, axis) {
  let s = String(text ?? "").trim().toUpperCase().replace(/[°'"′″]/g, " ");
  if (!s) return null;
  const hemi = s.match(/[NSEW]/g);
  if (hemi && hemi.length > 1) return null;
  const h = hemi?.[0];
  if (h && (axis === "lat") !== (h === "N" || h === "S")) return null;
  s = s.replace(/[NSEW]/, " ").trim();
  let neg = false;
  if (s.startsWith("-")) { neg = true; s = s.slice(1); }
  if (s.startsWith("+")) s = s.slice(1);
  const parts = s.split(/[\s,:]+/).filter(Boolean);
  if (!parts.length || parts.length > 3 || parts.some((p) => !/^\d+(\.\d+)?$/.test(p))) return null;
  const [d, m = 0, sec = 0] = parts.map(Number);
  if (parts.length > 1 && (m >= 60 || sec >= 60 || !Number.isInteger(d))) return null;
  let v = d + m / 60 + sec / 3600;
  if (neg || h === "S" || h === "W") v = -v;
  const lim = axis === "lat" ? 90 : 180;
  return Math.abs(v) <= lim + 1e-9 ? v : null;
}

/** 35°00'S, 5°30'E, 35°07.5'S */
export function fmtDM(v, axis) {
  if (Math.abs(v) < 1 / 1200) v = 0;
  const h = axis === "lat" ? (v < 0 ? "S" : "N") : v < 0 ? "W" : "E";
  const a = Math.abs(v);
  let d = Math.floor(a + 1e-9);
  let m = (a - d) * 60;
  if (m > 59.95) { d += 1; m = 0; }
  const ms = Math.abs(m - Math.round(m)) < 0.05 ? String(Math.round(m)).padStart(2, "0") : m.toFixed(1).padStart(4, "0");
  return `${d}°${ms}'${h}`;
}

export function setupGates(map, rt) {
  const { weather, course, dock, ui } = rt;
  const $ = (id) => document.getElementById(id);
  let st = { ...DEFAULTS };
  try { Object.assign(st, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch { /* ignore */ } };
  let sweep = null; // the last sweep: { title, axis, gates, gribs, target, cells: { "gi|path": { key?, state, error? } }, t0 }
  let running = false, stopping = false;

  // ---------- chart preview ----------
  const empty = { type: "FeatureCollection", features: [] };
  map.addSource("gt-gates", { type: "geojson", data: empty });
  map.addLayer({ id: "gt-row", type: "line", source: "gt-gates", filter: ["==", ["geometry-type"], "LineString"],
    paint: { "line-color": VIOLET, "line-width": 1.2, "line-dasharray": [3, 3], "line-opacity": 0.7 } }, "track");
  map.addLayer({ id: "gt-pts", type: "circle", source: "gt-gates", filter: ["==", ["geometry-type"], "Point"],
    paint: { "circle-radius": ["case", ["get", "best"], 7, 5], "circle-color": ["case", ["get", "best"], VIOLET, "#fff"],
             "circle-stroke-color": VIOLET, "circle-stroke-width": 2 } }, "track");

  const tabOpen = () => ui.current() === "routing" && !document.querySelector('#drawer .pane[data-pane="routing"] .tab[data-tab="gates"]')?.hidden;

  // ---------- the form ----------
  const marks = () => course.get().marks;
  function renderTargets() {
    const sel = $("gt-target");
    const ms = marks();
    const keep = sel.value;
    sel.innerHTML = "";
    if (!ms.length) sel.append(new Option("Add marks in the Route drawer", ""));
    ms.forEach((m, i) => sel.append(new Option(`${i + 1}: ${m.name || `${fmtDM(m.lat, "lat")} ${fmtDM(m.lon, "lon")}`}${i === ms.length - 1 ? " (finish)" : ""}`, i)));
    sel.value = keep !== "" && Number(keep) < ms.length ? keep : String(Math.max(0, ms.length - 1));
  }
  /** Where the leg into the target starts: the previous mark, else the start. */
  function legFrom(ti) {
    const c = course.get();
    return ti > 0 ? [c.marks[ti - 1].lon, c.marks[ti - 1].lat] : c.start;
  }
  function fillDefaults() {
    if (st.line || !marks().length) return;
    const ti = marks().length - 1;
    const a = legFrom(ti), b = [marks()[ti].lon, marks()[ti].lat];
    if (!a) return;
    // across the middle of the leg into the target, ±5° either side of the rhumb line
    if (st.axis === "lon") {
      const lon = Math.round((a[0] + b[0]) / 2), lat = Math.round((a[1] + b[1]) / 2);
      st.line = fmtDM(lon, "lon"); st.from = fmtDM(lat + 5, "lat"); st.to = fmtDM(lat - 5, "lat");
    } else {
      const lat = Math.round((a[1] + b[1]) / 2), lon = Math.round((a[0] + b[0]) / 2);
      st.line = fmtDM(lat, "lat"); st.from = fmtDM(lon - 5, "lon"); st.to = fmtDM(lon + 5, "lon");
    }
  }
  function labels() {
    $("gt-line-lab").textContent = st.axis === "lon" ? "Longitude" : "Latitude";
    $("gt-range-lab").textContent = st.axis === "lon" ? "Latitudes from" : "Longitudes from";
    $("gt-line").placeholder = st.axis === "lon" ? "5E" : "35S";
    $("gt-from").placeholder = st.axis === "lon" ? "30S" : "10E";
    $("gt-to").placeholder = st.axis === "lon" ? "40S" : "20E";
  }
  /** The gates from the form: { gates: [{ lon, lat, label, along }], error }. */
  function gatesFromForm() {
    const other = st.axis === "lon" ? "lat" : "lon";
    const line = parseCoord(st.line, st.axis), a = parseCoord(st.from, other), b = parseCoord(st.to, other);
    const n = Math.round(Number(st.n));
    if (line == null) return { error: `Type the ${st.axis === "lon" ? "longitude" : "latitude"} of the gate line, e.g. ${st.axis === "lon" ? "5E" : "35S"}.` };
    if (a == null || b == null) return { error: `Type the ${other === "lat" ? "latitudes" : "longitudes"} the gates run between.` };
    if (!(n >= 2 && n <= 40)) return { error: "Choose 2 to 40 gates." };
    if (Math.abs(a - b) < 1e-6) return { error: "The two ends of the gate row are the same." };
    const ti = Number($("gt-target").value);
    const tgt = marks()[ti];
    // the long way round never: put longitudes on the target's side of the date line
    const near = (lon) => { if (!tgt) return lon; let x = lon; while (x - tgt.lon > 180) x -= 360; while (tgt.lon - x > 180) x += 360; return x; };
    let lo = a, hi = b;
    if (other === "lon") { lo = near(a); hi = lo + ((((b - a) % 360) + 540) % 360 - 180); }
    const gates = [];
    for (let i = 0; i < n; i++) {
      const v = lo + ((hi - lo) * i) / (n - 1);
      const lon = st.axis === "lon" ? near(line) : v;
      const lat = st.axis === "lon" ? v : line;
      const norm = ((lon + 540) % 360) - 180;
      gates.push({ lon, lat, along: v, label: `${fmtDM(lat, "lat")} ${fmtDM(norm, "lon")}` });
    }
    return { gates, spacing: Math.abs(hi - lo) / (n - 1), ti };
  }
  function gribs() {
    if (st.wx === "all") return weather.all().map((g) => ({ path: g.path, name: g.name }));
    const w = weather.current();
    return w.path ? [{ path: w.path, name: w.name }] : [];
  }
  function update() {
    labels();
    const g = gatesFromForm();
    const sp = $("gt-spacing");
    sp.textContent = g.spacing ? `every ${g.spacing >= 1 ? `${+g.spacing.toFixed(2)}°` : `${+(g.spacing * 60).toFixed(1)}'`}` : "";
    const nG = gribs().length;
    $("gt-count").textContent = g.error ?? `${g.gates.length} gates × ${nG || "no"} GRIB${nG === 1 ? "" : "s"} = ${g.gates.length * nG} routings.`;
    $("gt-count").classList.toggle("warn", !!g.error || !nG);
    drawPreview(g.gates);
  }
  function drawPreview(gates) {
    // the form's gates while the Gates tab is open; else the last sweep's while it runs or its table is open
    const useSweep = !!sweep && (running || (!tabOpen() && dock.current() === "gates"));
    const gs = useSweep ? sweep.gates : tabOpen() ? gates ?? [] : [];
    const best = useSweep ? bestGate() : null;
    const feats = gs.map((g, i) => ({ type: "Feature", properties: { i, best: i === best }, geometry: { type: "Point", coordinates: [g.lon, g.lat] } }));
    if (gs.length > 1) feats.push({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: gs.map((g) => [g.lon, g.lat]) } });
    map.getSource("gt-gates").setData({ type: "FeatureCollection", features: feats });
  }

  for (const [id, k] of [["gt-axis", "axis"], ["gt-line", "line"], ["gt-from", "from"], ["gt-to", "to"], ["gt-n", "n"], ["gt-wx", "wx"]]) {
    $(id).value = st[k];
    $(id).addEventListener(id === "gt-line" || id === "gt-from" || id === "gt-to" ? "input" : "change", () => {
      const was = st.axis;
      st[k] = $(id).value;
      if (k === "axis" && st.axis !== was) {
        // swap the meaning of the fields: start again from the course
        st.line = st.from = st.to = "";
        fillDefaults();
        for (const [i2, k2] of [["gt-line", "line"], ["gt-from", "from"], ["gt-to", "to"]]) $(i2).value = st[k2];
      }
      save();
      update();
    });
  }
  $("gt-target").onchange = update;
  function refill() {
    renderTargets();
    if (!st.line) { fillDefaults(); for (const [i2, k2] of [["gt-line", "line"], ["gt-from", "from"], ["gt-to", "to"]]) $(i2).value = st[k2]; save(); }
    update();
  }
  course.onChange(refill);
  ui.onChange(refill);
  weather.onLoaded?.(update);

  // ---------- run ----------
  $("gt-go").onclick = async () => {
    if (running) return;
    const g = gatesFromForm();
    if (g.error) { rt.msg(g.error, "warn"); return; }
    const gr = gribs();
    if (!gr.length) { rt.msg("Load a GRIB in the Weather panel first.", "warn"); return; }
    if (!rt.ready()) return;
    course.stopEditing?.();
    const tgt = marks()[g.ti];
    const tName = tgt.name || `mark ${g.ti + 1}`;
    sweep = {
      axis: st.axis, gates: g.gates, gribs: gr, target: g.ti, t0: rt.startTime(),
      title: `Gates on ${fmtDM(parseCoord(st.line, st.axis), st.axis)}, ${g.gates[0].label.split(" ")[st.axis === "lon" ? 0 : 1]} → ` +
        `${g.gates.at(-1).label.split(" ")[st.axis === "lon" ? 0 : 1]}, before ${tName}`,
      cells: {},
    };
    const tasks = [];
    for (const grib of gr) g.gates.forEach((gate, gi) => tasks.push({ gi, gate, grib }));
    for (const t of tasks) sweep.cells[`${t.gi}|${t.grib.path}`] = { state: "waiting" };
    running = true; stopping = false;
    $("gt-go").disabled = true; $("gt-stop").disabled = false;
    const line = rt.msg(`Gate sweep: 0 of ${tasks.length}…`);
    dock.show("gates");
    let done = 0, failed = 0;
    const one = async (t) => {
      const cell = sweep.cells[`${t.gi}|${t.grib.path}`];
      cell.state = "running";
      dock.refresh("gates");
      const ms = course.get().marks;
      ms.splice(sweep.target, 0, { id: null, name: `Gate ${t.gate.label}`, lon: t.gate.lon, lat: t.gate.lat, rounding: "" });
      try {
        const it = await rt.compute(t.grib, ms, `Gate ${t.gate.label}${gr.length > 1 ? ` · ${t.grib.name}` : ""}`);
        cell.key = it.key;
        cell.state = "done";
      } catch (e) {
        cell.state = "failed"; cell.error = String(e); failed += 1;
      }
      done += 1;
      line.set(`Gate sweep: ${done} of ${tasks.length}${failed ? ` (${failed} failed)` : ""}…`);
      dock.refresh("gates");
      update();
    };
    const queue = [...tasks];
    await Promise.all(Array.from({ length: Math.min(PARALLEL, queue.length) }, async () => {
      while (queue.length && !stopping) await one(queue.shift());
    }));
    for (const t of queue) sweep.cells[`${t.gi}|${t.grib.path}`].state = "stopped";
    running = false;
    $("gt-go").disabled = false; $("gt-stop").disabled = true;
    const b = bestGate();
    line.set(stopping ? `Gate sweep stopped after ${done} of ${tasks.length}.`
      : `Gate sweep done: ${done} routings${failed ? `, ${failed} failed` : ""}.${b != null ? ` Best: gate ${sweep.gates[b].label}.` : ""}`, stopping || failed ? "warn" : "good");
    dock.refresh("gates");
    update();
  };
  $("gt-stop").onclick = () => { stopping = true; $("gt-stop").disabled = true; };
  $("gt-table").onclick = () => { dock.toggle("gates"); update(); };

  // ---------- results ----------
  /** Per GRIB: elapsed seconds of each gate's routing (null: not arrived), and the best. */
  function results() {
    if (!sweep) return null;
    const cols = sweep.gribs.map((grib) => {
      const el = sweep.gates.map((_, gi) => {
        const c = sweep.cells[`${gi}|${grib.path}`];
        const it = c?.key && rt.item(c.key);
        return it && it.r.status === "arrived" ? it.r.arrival - it.stats.t0 : null;
      });
      const ok = el.filter((v) => v != null);
      return { grib, el, best: ok.length ? Math.min(...ok) : null };
    });
    // per gate: time lost against the best gate under each GRIB, its mean and worst
    const rows = sweep.gates.map((gate, gi) => {
      const d = cols.map((c) => (c.el[gi] != null && c.best != null ? c.el[gi] - c.best : null));
      const ok = d.filter((v) => v != null);
      return { gate, gi, d, missing: d.length - ok.length, mean: ok.length ? ok.reduce((a, b) => a + b, 0) / ok.length : null,
        worst: ok.length && ok.length === d.length ? Math.max(...ok) : null };
    });
    const ranked = rows.filter((r) => r.mean != null).sort((a, b) => a.missing - b.missing || a.mean - b.mean || (a.worst ?? 1e18) - (b.worst ?? 1e18));
    ranked.forEach((r, k) => { r.rank = k + 1; });
    return { cols, rows };
  }
  function bestGate() {
    const r = results();
    return r?.rows.find((x) => x.rank === 1)?.gi ?? null;
  }
  const lost = (s) => (s == null ? "—" : s < 60 ? "best" : `+${hm(s)}`);

  dock.register("gates", {
    title: "Gates",
    render(body, tools) {
      drawPreview(gatesFromForm().gates);
      if (!sweep) { body.innerHTML = `<p class="wx-note">Set up a row of gates in Routing › Gates and press Run gate sweep.</p>`; return; }
      const keys = Object.values(sweep.cells).map((c) => c.key).filter(Boolean);
      const only = document.createElement("button");
      only.textContent = "Show only these";
      only.title = "Show this sweep's routings on the chart and hide the others";
      only.onclick = () => rt.setVisible(keys.filter((k) => rt.item(k)), true);
      const rm = document.createElement("button");
      rm.textContent = "Remove these routings";
      rm.disabled = running;
      rm.onclick = () => { rt.remove(keys); sweep = null; dock.refresh("gates"); update(); };
      tools.append(only, rm);

      const res = results();
      const multi = sweep.gribs.length > 1;
      const head = document.createElement("div");
      head.className = "gt-head";
      head.textContent = `${sweep.title} · start ${fmt(sweep.t0, true)}${multi ? ` · ${sweep.gribs.length} GRIBs` : ` · ${sweep.gribs[0].name}`}`;
      body.append(head);
      const wrap = document.createElement("div");
      wrap.className = "gt-wrap";
      body.append(wrap);

      const t = document.createElement("table");
      t.className = "cmp gt-table";
      t.innerHTML = `<thead><tr><th>Gate</th>${sweep.gribs.map((g) => `<th title="${esc(g.name)}">${esc(multi ? shortGrib(g.name) : "Arrival")}</th>`).join("")}` +
        `${multi ? "<th title=\"Mean time lost against the best gate of each GRIB\">Mean lost</th><th title=\"Most time lost under any GRIB\">Worst</th>" : "<th>Lost</th>"}<th>Rank</th></tr></thead><tbody></tbody>`;
      for (const row of res.rows) {
        const tr = document.createElement("tr");
        tr.classList.toggle("gt-best", row.rank === 1);
        const tdG = document.createElement("td");
        tdG.className = "nm";
        tdG.textContent = row.gate.label;
        tr.append(tdG);
        sweep.gribs.forEach((grib, k) => {
          const c = sweep.cells[`${row.gi}|${grib.path}`];
          const it = c?.key && rt.item(c.key);
          const td = document.createElement("td");
          if (it) {
            const sw = `<span class="sw" style="background:${it.colour}"></span>`;
            if (it.r.status === "arrived") {
              const d = res.cols[k].el[row.gi] - res.cols[k].best;
              td.innerHTML = `${sw}${esc(fmt(it.r.arrival, true))}${multi ? ` <small>${esc(lost(d))}</small>` : ""}`;
              td.classList.toggle("best", d < 60 && sweep.gates.length > 1);
            } else td.innerHTML = `${sw}<small>${esc(it.r.status === "grib end" ? `GRIB ends, ${it.stats.toGo?.toFixed(0) ?? "?"} NM to go` : it.r.status)}</small>`;
            td.title = `${it.name}: click to show in detail`;
            td.classList.add("click");
            td.onclick = (e) => { e.stopPropagation(); rt.activate(it.key); dock.refresh("gates"); };
          } else if (c?.key) td.innerHTML = "<small>removed</small>";
          else td.innerHTML = `<small${c?.error ? ` title="${esc(c.error)}"` : ""}>${{ waiting: "…", running: "routing…", failed: "failed", stopped: "not run" }[c?.state] ?? ""}</small>`;
          tr.append(td);
        });
        tr.insertAdjacentHTML("beforeend", multi
          ? `<td>${esc(row.mean == null ? "—" : row.mean < 60 ? "0" : `+${hm(row.mean)}`)}${row.missing ? ` <small>(${row.missing} not arrived)</small>` : ""}</td><td>${esc(row.worst == null ? "—" : row.worst < 60 ? "0" : `+${hm(row.worst)}`)}</td>`
          : `<td>${esc(lost(row.d[0]))}</td>`);
        tr.insertAdjacentHTML("beforeend", `<td>${row.rank ?? "—"}</td>`);
        tr.onclick = () => {
          const c = sweep.gribs.map((g) => sweep.cells[`${row.gi}|${g.path}`]).find((x) => x?.key && rt.item(x.key));
          if (c) { rt.activate(c.key); dock.refresh("gates"); }
        };
        t.tBodies[0].append(tr);
      }
      wrap.append(t);
      const plot = drawPlot(res, sweep);
      if (plot) wrap.append(plot);
      const note = document.createElement("p");
      note.className = "wx-note";
      note.textContent = multi
        ? "Lost = later than the best gate under the same GRIB. Ranked by mean time lost: a gate that is close to the best under every GRIB is the safer choice when the GRIBs disagree."
        : "Lost = later than the best gate. Click a cell to show that routing in detail.";
      body.append(note);
    },
  });
  rt.onDraw(() => { if (dock.current() === "gates") dock.refresh("gates"); });

  renderTargets();
  fillDefaults();
  for (const [i2, k2] of [["gt-line", "line"], ["gt-from", "from"], ["gt-to", "to"]]) $(i2).value = st[k2];
  update();
  return { gatesFromForm, parseCoord };
}

/** Time lost (hours) against the gate position, one line per GRIB. */
function drawPlot(res, sweep) {
  const pts = res.cols.map((c) => c.el.map((v, gi) => (v != null && c.best != null ? { x: sweep.gates[gi].along, y: (v - c.best) / 3600 } : null)));
  const all = pts.flat().filter(Boolean);
  if (all.length < 2) return null;
  const W = 340, H = 170, l = 40, r = 10, top = 10, b = 26;
  const xs = sweep.gates.map((g) => g.along);
  const x0 = Math.min(...xs), x1 = Math.max(...xs);
  const yMax = Math.max(1, ...all.map((p) => p.y));
  const step = yMax > 24 ? 12 : yMax > 12 ? 6 : yMax > 4 ? 2 : 1;
  const yTop = Math.ceil(yMax / step) * step;
  const X = (x) => l + ((x - x0) / (x1 - x0 || 1)) * (W - l - r);
  const Y = (y) => top + (1 - y / yTop) * (H - top - b);
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("width", W);
  svg.setAttribute("height", H);
  svg.classList.add("mg", "gt-plot");
  const add = (tag, attrs) => { const e = document.createElementNS(NS, tag); for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v); svg.append(e); return e; };
  add("rect", { x: l, y: top, width: W - l - r, height: H - top - b, class: "mg-bg" });
  for (let y = 0; y <= yTop + 1e-9; y += step) {
    add("line", { x1: l, x2: W - r, y1: Y(y), y2: Y(y), class: "mg-grid" });
    add("text", { x: l - 4, y: Y(y) + 4, class: "mg-tick", "text-anchor": "end" }).textContent = `${y} h`;
  }
  const axis = sweep.axis === "lon" ? "lat" : "lon";
  const every = Math.max(1, Math.ceil(xs.length / 6));
  xs.forEach((x, i) => {
    if (i % every && i !== xs.length - 1) return;
    add("text", { x: X(x), y: H - 8, class: "mg-tick", "text-anchor": "middle" }).textContent = fmtDM(axis === "lon" ? ((x + 540) % 360) - 180 : x, axis).replace(/°00'/, "°");
  });
  pts.forEach((line, k) => {
    const col = LINE_COLOURS[k % LINE_COLOURS.length];
    let d = "", pen = false;
    for (const p of line) {
      if (!p) { pen = false; continue; }
      d += `${pen ? "L" : "M"}${X(p.x).toFixed(1)},${Y(p.y).toFixed(1)}`;
      pen = true;
    }
    add("path", { d, stroke: col, fill: "none", "stroke-width": 1.8 });
    for (const p of line) if (p) add("circle", { cx: X(p.x), cy: Y(p.y), r: 2.5, fill: col });
  });
  if (res.cols.length > 1) {
    res.cols.forEach((c, k) => {
      add("text", { x: l + 6, y: top + 12 + k * 12, class: "mg-tick", fill: LINE_COLOURS[k % LINE_COLOURS.length], style: `fill:${LINE_COLOURS[k % LINE_COLOURS.length]}` }).textContent = shortGrib(c.grib.name);
    });
  }
  return svg;
}

const shortGrib = (name) => name;
function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]); }
