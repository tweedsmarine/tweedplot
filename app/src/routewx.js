// Weather along the route (dock view): the forecast the boat meets on the way, from
// the active routing (its own times and positions) or, with no routing, the course
// sailed at a set speed from a fixed departure. Stacked strips like the
// meteogram: wind and gusts with direction arrows, waves (Hs, Tp, direction) by
// default; pressure, rain, air temperature, CAPE, sea temperature and current to tick.
// Select a point to inspect it without changing the route's departure.
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { positionAt } from "./routing.js";
import { courseTrack, sampleTrack, gaps as fieldGaps, createRouteWeatherRequest } from "./routewx-core.js";

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
  let opts = { speed: 6, departure: null, wind: true, waves: true, extras: [], settings: false, layout: "below" };
  try { Object.assign(opts, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  if (!Number.isFinite(opts.speed) || opts.speed < 0.5 || opts.speed > 40) opts.speed = 6;
  if (!Number.isFinite(opts.departure) || opts.departure <= 0) opts.departure = null;
  if (!Array.isArray(opts.extras)) opts.extras = [];
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(opts)); } catch { /* ignore */ } };
  let selected = -1;
  let timer = null;
  let lastPath = weather.current().path;
  const request = createRouteWeatherRequest(() => dock.refresh("routewx"));
  const sideButton = document.getElementById("side-route-wx");
  function syncSideButton(cur) {
    if (!sideButton) return;
    const active = cur === "routewx";
    sideButton.classList.toggle("on", active);
    sideButton.setAttribute("aria-pressed", String(active));
    sideButton.title = `${active ? "Close" : "Open"} weather along the active routing or planned route`;
  }
  if (sideButton) sideButton.onclick = () => dock.toggle("routewx");
  syncSideButton(dock.current());

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
    // a departure saved on an earlier day lies outside today's forecast and would give empty strips:
    // start again from the chart time (or the start of the forecast)
    const sp = weather.span?.(current.path);
    if (opts.departure != null && sp && (opts.departure > sp[1] || opts.departure < sp[0] - 12 * 3600)) opts.departure = null;
    if (opts.departure == null && current.t > 0) { opts.departure = sp ? Math.min(sp[1], Math.max(sp[0], current.t)) : current.t; save(); }
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
  let playTimer = 0; // Play on the dock: steps along the route's forecast points
  const stopPlay = () => { clearInterval(playTimer); playTimer = 0; };
  let follow = null, internal = false; // follow(t): move the graph's cursor to a chart time set elsewhere
  weather.onChange(() => { if (follow && dock.current() === "routewx") follow(weather.current().t); });
  const later = () => {
    if (dock.current() !== "routewx") return;
    invalidate();
    timer = setTimeout(refetch, 300);
  };
  dock.onChange((cur) => { if (cur !== "routewx") stopPlay(); syncSideButton(cur); if (cur !== "routewx") invalidate("empty"); else refetch(); });
  weather.onLoaded(later);
  weather.onChange(() => { const path = weather.current().path; if (path !== lastPath) { lastPath = path; later(); } });
  routing.onDraw?.(later);
  course.onChange?.(later);

  dock.register("routewx", {
    title: "Weather on route",
    // beside the chart, the plots turn to run down the screen (for routes that run north-south): the dock
    // moves to the chart's right side, wide enough for a column per plot
    placement: () => (opts.layout === "beside"
      ? { side: "right", width: 110 + 190 * ((opts.wind ? 1 : 0) + (opts.waves ? 1 : 0) + opts.extras.length) } : { side: "bottom" }),
    render(body, tools) {
      const beside = opts.layout === "beside";
      follow = null;
      stopPlay();
      if (tools) {
        const lay = document.createElement("button");
        lay.textContent = beside ? "Plots below" : "Plots beside";
        lay.title = beside ? "Put the plots under the chart, time running left to right" : "Put the plots beside the chart, time running down the screen: best for a route that runs north-south";
        lay.onclick = () => { opts.layout = beside ? "below" : "beside"; save(); dock.show("routewx"); };
        tools.append(lay);
      }
      if (!body._rwObs) {
        body._rwObs = new ResizeObserver(() => {
          const w = body.clientWidth;
          const h = body.clientHeight;
          if (body._rwW != null && (Math.abs(w - body._rwW) > 8 || (opts.layout === "beside" && Math.abs(h - (body._rwH ?? h)) > 8))) { body._rwW = w; body._rwH = h; dock.refresh("routewx"); }
        });
        body._rwObs.observe(body);
      }
      const state = request.state(), data = state.data;
      const context = document.createElement("div");
      context.className = "rw-context";
      const active = routing.activePoints();
      const routeName = active?.length > 1 ? routing.activeName() || "Active routing" : course.get().name || "Unsaved route";
      const title = document.createElement("strong");
      title.textContent = `${active?.length > 1 ? "Routing" : "Route"}: ${routeName}`;
      context.append(title);
      const timeNote = document.createElement("span");
      timeNote.textContent = "Passage times are independent of the chart forecast time.";
      context.append(timeNote);
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
      // the settings and details fold away so the plots have the dock; what matters shows on the summary line
      const set = document.createElement("details");
      set.className = "rw-settings";
      set.open = !!opts.settings;
      set.ontoggle = () => { if (opts.settings !== set.open) { opts.settings = set.open; save(); } };
      const sum = document.createElement("summary");
      sum.textContent = "Settings and details";
      set.append(sum, context);
      body.append(set);
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
      set.append(source);
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
      set.append(pick);
      const r0 = data.rows[0], r1 = data.rows.at(-1);
      const head = document.createElement("div");
      head.className = "mg-head";
      head.textContent = data.from === "routing"
        ? `Along the active routing: ${fmt(r0.t)} to ${fmt(r1.t)}, ${r1.nm.toFixed(0)} NM`
        : `Along the course at ${data.kn} kn from ${fmt(r0.t)}: ${r1.nm.toFixed(0)} NM, ${hm(r1.t - r0.t)}`;
      const lastWith = data.rows.findLast((r) => ["tws", "gust", "wave", "mslp", "temp", "rain", "cape", "sst"].some((k) => r[k] != null) || r.cur?.kn != null);
      if (lastWith && lastWith !== r1) head.textContent += ` · The forecast ends ${fmt(lastWith.t)}, ${lastWith.nm.toFixed(0)} NM along: the last ${(r1.nm - lastWith.nm).toFixed(0)} NM have no forecast`;
      set.append(head);
      sum.textContent = `Settings and details · ${data.from === "routing" ? data.name : `${data.kn} kn from ${fmt(r0.t)}`} · ${r1.nm.toFixed(0)} NM · ${[...(opts.wind ? ["Wind"] : []), ...(opts.waves ? ["Waves"] : []), ...EXTRAS.filter((x) => opts.extras.includes(x.key)).map((x) => x.label)].join(", ") || "no plots"}`;
      const strips = [];
      if (opts.wind) strips.push({ key: "wind", title: "Wind kn (gusts dashed) · direction", h: 120 });
      if (opts.waves) strips.push({ key: "waves", title: "Waves Hs m · Tp s · direction", h: 90 });
      for (const x of EXTRAS) if (opts.extras.includes(x.key)) strips.push({ ...x, title: `${x.label} ${x.unit}`, h: 64 });
      const fields = [...(opts.wind ? [{ key: "tws", label: "Wind" }] : []), ...(opts.waves ? [{ key: "wave", label: "Waves" }] : []), ...EXTRAS.filter((x) => opts.extras.includes(x.key))];
      // a route that outlasts the forecast is not a gap (the heading says where the forecast ends): warn only about
      // fields that are missing altogether, start late, have holes, or end before the others
      const warn = fieldGaps(data.rows, fields).flatMap((g) => {
        const n = (c) => `${c} of ${g.total} sampled points`;
        if (g.count === 0) return [`${g.label} is not in the loaded GRIBs`];
        return [
          ...(g.lead ? [`${g.label} starts late: none at the first ${n(g.lead)}`] : []),
          ...(g.inner ? [`${g.label} has holes at ${n(g.inner)} (over land, or outside that GRIB's area)`] : []),
          ...(g.earlier ? [`${g.label} ends ${n(g.earlier)} before the other fields`] : []),
        ];
      });
      if (warn.length) {
        const warning = document.createElement("div");
        warning.className = "rw-status";
        warning.textContent = `Forecast coverage: ${warn.join("; ")}. Gaps do not mean calm conditions.`;
        body.append(warning);
      }
      const readout = document.createElement("div");
      readout.className = "rw-readout";
      const inspect = document.createElement("div");
      inspect.className = "rw-inspect";
      const previous = document.createElement("button"), next = document.createElement("button");
      previous.textContent = "◀ Previous"; next.textContent = "Next ▶";
      // dragging moves the cursor, values and boat marker at once; the chart's weather follows when the drag
      // pauses for a moment (and on release), so a big GRIB is not asked for a new picture at every step
      let pending = null, timer = 0;
      const moveChart = (r) => {
        clearTimeout(timer); pending = null;
        internal = true;
        try { weather.setTime(r.t); weather.setCursor([r.lon, r.lat]); } finally { internal = false; }
      };
      const flush = () => { if (pending) moveChart(pending); };
      const update = (index, moveTime = true, deferred = false) => {
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
          if (inRange && !deferred) moveChart(r);
          else if (inRange) { pending = r; clearTimeout(timer); timer = setTimeout(flush, 150); }
        }
        const noValues = fields.length && fields.every((f) => r[f.key] == null);
        readout.textContent = `Inspecting: ${describe(r)}${noValues ? " · No selected forecast data at this point" : ""}${used.length && !beside ? ` · Sources: ${used.join(", ")}` : ""}${note}`;
        previous.disabled = selected === 0; next.disabled = selected === data.rows.length - 1;
        svg.cursor(r.t);
      };
      body._rwW = body.clientWidth; body._rwH = body.clientHeight;
      const handlers = { pick: (r) => update(data.rows.indexOf(r), true, true), release: flush };
      // what is already above the plots (settings, warnings) and the readout row below take their room from the height
      const above = [...body.children].reduce((a, c) => a + c.offsetHeight + 6, 0);
      const play = document.createElement("button"), nowBtn = document.createElement("button");
      const playLabel = () => { play.textContent = playTimer ? "❚❚ Pause" : "▶ Play"; };
      const halt = () => { stopPlay(); playLabel(); };
      play.title = "Step along the route's forecast points, moving the chart time with them"; playLabel();
      play.onclick = () => {
        if (playTimer) { halt(); return; }
        if (selected >= data.rows.length - 1) selected = -1; // at the end: start again
        playTimer = setInterval(() => {
          if (selected >= data.rows.length - 1) { halt(); return; }
          update(selected + 1);
        }, 350);
        playLabel();
      };
      nowBtn.textContent = "Now";
      const nowT = Date.now() / 1000;
      nowBtn.disabled = !(nowT >= data.rows[0].t && nowT <= data.rows.at(-1).t);
      nowBtn.title = nowBtn.disabled ? "The real time now is outside this route's times" : "Go to the point on the route where the boat is at the real time now";
      nowBtn.onclick = () => {
        const t = Date.now() / 1000;
        if (!(t >= data.rows[0].t && t <= data.rows.at(-1).t)) return;
        halt(); update(data.rows.reduce((b, r, k) => (Math.abs(r.t - t) < Math.abs(data.rows[b].t - t) ? k : b), 0));
      };
      previous.onclick = () => { halt(); update(selected < 0 ? 0 : selected - 1); };
      next.onclick = () => { halt(); update(selected < 0 ? 0 : selected + 1); };
      handlers.pick = (r) => { halt(); update(data.rows.indexOf(r), true, true); };
      inspect.append(readout, previous, play, next, nowBtn);
      body.append(inspect); // measured with its readout and buttons, so the plots take all the rest
      const below = inspect.offsetHeight + 8;
      const make = (H) => (beside
        ? drawV(data.rows, strips, Math.max(300, body.clientWidth - 20), H, handlers)
        : draw(data.rows, strips, Math.max(520, body.clientWidth - 20), handlers));
      let svg = make(Math.max(320, body.clientHeight - above - below - 16));
      body.insertBefore(svg.el, inspect);
      if (beside) { // margins and padding add up to a little more than the sum: trim what still overflows
        const over = body.scrollHeight - body.clientHeight;
        if (over > 0) { svg.el.remove(); svg = make(Math.max(320, parseFloat(svg.el.getAttribute("height")) - over - 2)); body.insertBefore(svg.el, inspect); }
      }
      // the chart time moved elsewhere (the Explore slider, play): the cursor follows it
      follow = (t) => {
        if (internal) return;
        if (!(t >= data.rows[0].t && t <= data.rows.at(-1).t)) { svg.cursor(null); return; }
        const i = data.rows.reduce((b, r, k) => (Math.abs(r.t - t) < Math.abs(data.rows[b].t - t) ? k : b), 0);
        if (i !== selected) update(i, false);
      };
      if (selected >= 0 && selected < data.rows.length) update(selected, false);
      else readout.textContent = "Press on the graph and drag, or use Previous / Next, to inspect a point.";
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
/** The route's course at a forecast point (towards the next one), degrees true; null when it does not move. */
function courseAt(rows, i) {
  const [a, b] = i < rows.length - 1 ? [rows[i], rows[i + 1]] : [rows[i - 1], rows[i]];
  if (!a || !b || (a.lat === b.lat && a.lon === b.lon)) return null;
  const R = Math.PI / 180, dl = (b.lon - a.lon) * R;
  const y = Math.sin(dl) * Math.cos(b.lat * R), x = Math.cos(a.lat * R) * Math.sin(b.lat * R) - Math.sin(a.lat * R) * Math.cos(b.lat * R) * Math.cos(dl);
  return (Math.atan2(y, x) / R + 360) % 360;
}
const crs3 = (c) => (c == null ? "" : `${String(Math.round(c) % 360).padStart(3, "0")}°`);

function draw(rows, strips, W, { pick, release }) {
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
  // the axis ends where the forecast does (the route may go on longer: the heading says so)
  const hasData = (r) => ["tws", "gust", "wave", "mslp", "temp", "rain", "cape", "sst"].some((k) => r[k] != null) || r.cur?.kn != null;
  const lastWith = rows.findLast(hasData) ?? rows.at(-1);
  const t0 = rows[0].t, t1 = Math.max(lastWith.t, t0 + 600);
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
  const bands = []; // each plot's top and height: the time grid is drawn within them only
  const series = []; // what the draggable cursor reads out: { key, yv, col, unit, above, y0, h }
  for (const st of strips) {
    bands.push({ y0, h: st.h });
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
    // yv and yp read y0, which moves on to the next strip: the cursor reads them later, so undo that shift
    const base = y0;
    const reg = (key, col, unit, yfn = yv, above = true) => series.push({ key, yv: (v) => yfn(v) - (y0 - base), col, unit, above, y0: base, h: st.h });
    if (st.key === "wind") {
      reg("gust", "#c0392b", "kn"); reg("tws", "#006994", "kn", yv, false);
      add("path", { d: path("gust"), stroke: "#c0392b", fill: "none", "stroke-width": 1.2, "stroke-dasharray": "4 3" }, g);
      add("path", { d: path("tws"), stroke: "#006994", fill: "none", "stroke-width": 2 }, g);
      arrows(g, y0 + 10, "twd", "#00274d");
    } else if (st.key === "waves") {
      reg("wave", "#2a62c9", "m");
      add("path", { d: path("wave"), stroke: "#2a62c9", fill: "none", "stroke-width": 2 }, g);
      // Tp: its own scale on the right
      const ps = rows.map((r) => r.period).filter((v) => v != null);
      if (ps.length) {
        const pl = 0, ph = Math.max(4, Math.ceil(Math.max(...ps) / 4) * 4);
        const yp = (v) => y0 + st.h - ((v - pl) / (ph - pl)) * (st.h - top2);
        reg("period", "#5d6b75", "s", yp, false);
        let d = "", pen = false;
        for (const r of rows) { if (r.period == null) { pen = false; continue; } d += `${pen ? "L" : "M"}${x(r.t).toFixed(1)},${yp(r.period).toFixed(1)}`; pen = true; }
        add("path", { d, stroke: "#8a96a0", fill: "none", "stroke-width": 1.3, "stroke-dasharray": "2 2" }, g);
        for (const v of [0, ph / 2, ph]) add("text", { x: W - pad.r + 4, y: yp(v) + 4, class: "mg-tick" }, g).textContent = `${v} s`;
      }
      arrows(g, y0 + 10, "wavedir", "#2a62c9");
    } else if (st.bars) {
      reg(st.key, st.col, st.unit ?? "");
      const bw = Math.max(1, (W - pad.l - pad.r) / rows.length - 0.5);
      for (const r of rows) {
        const v = val(r, st.key);
        if (!v) continue;
        add("rect", { x: x(r.t) - bw / 2, y: yv(v), width: bw, height: yv(lo) - yv(v), fill: st.col, opacity: 0.7 }, g);
      }
    } else {
      reg(st.key, st.col, st.unit ?? "");
      add("path", { d: path(st.key), stroke: st.col, fill: "none", "stroke-width": 1.8 }, g);
    }
    y0 += st.h + gap;
  }
  // time along the bottom (local days and 6-hourly ticks), distance below it
  const span = t1 - t0;
  const minor = span > 5 * 86400 ? 21600 : span > 36 * 3600 ? 10800 : 3600;
  for (let t = Math.ceil(t0 / minor) * minor; t <= t1; t += minor) for (const b of bands) add("line", { x1: x(t), x2: x(t), y1: b.y0, y2: b.y0 + b.h, class: "mg-vgrid" });
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
    add("text", { x: x(t), y: y0 - gap + 27, class: "mg-tick", "text-anchor": "middle" }).textContent = `${r.nm.toFixed(0)} NM · ${crs3(courseAt(rows, rows.indexOf(r)))}`.replace(/ · $/, "");
  }
  const cur = add("line", { x1: 0, x2: 0, y1: top, y2: y0 - gap, class: "mg-cursor" });
  const marks = add("g", { class: "mg-marks" }); // a dot and a value where the cursor crosses each plot
  const at = (e) => {
    const r = svg.getBoundingClientRect();
    return t0 + ((e.clientX - r.left - pad.l) / (W - pad.l - pad.r)) * (t1 - t0);
  };
  const nearest = (t) => rows.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a));
  // press anywhere on the plots and drag: the cursor follows, snapping to the route's forecast points
  let last = null;
  const follow = (e) => {
    const r = nearest(Math.max(t0, Math.min(t1, at(e))));
    if (r !== last) { last = r; pick(r); }
  };
  svg.style.touchAction = "none";
  svg.addEventListener("pointerdown", (e) => {
    if (e.button > 0) return;
    const r = svg.getBoundingClientRect();
    if (e.clientX - r.left < pad.l - 6 || e.clientX - r.left > W - pad.r + 6 || e.clientY - r.top < top) return;
    svg.setPointerCapture?.(e.pointerId);
    last = null; follow(e);
    const move = (ev) => follow(ev);
    const up = () => { release?.(); svg.removeEventListener("pointermove", move); svg.removeEventListener("pointerup", up); svg.removeEventListener("pointercancel", up); };
    svg.addEventListener("pointermove", move); svg.addEventListener("pointerup", up); svg.addEventListener("pointercancel", up);
    e.preventDefault();
  });
  return {
    el: svg,
    cursor(t) {
      const show = t != null && t >= t0 && t <= t1;
      cur.style.display = show ? "" : "none";
      marks.replaceChildren();
      if (!show) return;
      const cx = x(t);
      cur.setAttribute("x1", cx); cur.setAttribute("x2", cx);
      const r = nearest(t);
      const shown = [];
      for (const sr of series) {
        const v = val(r, sr.key);
        if (v == null) continue;
        const cy = sr.yv(v);
        add("circle", { cx, cy, r: 3.5, fill: sr.col, stroke: "#fff", "stroke-width": 1.2 }, marks);
        const txt = `${+v.toFixed(Math.abs(v) < 10 ? 1 : 0)}${sr.unit ? ` ${sr.unit}` : ""}`;
        const right = cx + 12 + txt.length * 6.2 < W - pad.r;
        shown.push({ sr, txt, right, y: cy + (sr.above ? -7 : 14) });
      }
      // values in one plot keep 13 px apart, within the plot
      for (const sid of new Set(shown.map((l) => l.sr.y0))) {
        const g = shown.filter((l) => l.sr.y0 === sid).sort((p, q) => p.y - q.y);
        const lo = g[0].sr.y0 + 11, hi = g[0].sr.y0 + g[0].sr.h - 3;
        g.forEach((l, i) => { l.y = Math.max(lo, i ? Math.max(l.y, g[i - 1].y + 13) : l.y); });
        for (let i = g.length - 1; i >= 0; i--) { g[i].y = Math.min(g[i].y, i < g.length - 1 ? g[i + 1].y - 13 : hi); }
      }
      for (const l of shown) add("text", { x: l.right ? cx + 8 : cx - 8, y: l.y, class: "mg-val", fill: l.sr.col, "text-anchor": l.right ? "start" : "end" }, marks).textContent = l.txt;
    },
  };
}

/** The same plots turned to run down the screen: time (and the route with it) from top to bottom, one column per plot.
 *  For a route that runs north-south, beside the chart. Returns the same { el, cursor } as draw(). */
function drawV(rows, strips, W, H, { pick, release }) {
  const padL = 88, padR = 10, padT = 46, padB = 40, gap = 16, arrowCol = 18; // padB: the last time label is three lines
  const Hp = Math.max(120, H - padT - padB);
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("width", W); svg.setAttribute("height", H);
  svg.classList.add("mg", "mg-v");
  const add = (tag, attrs, parent = svg) => {
    const e = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    parent.append(e);
    return e;
  };
  const hasData = (r) => ["tws", "gust", "wave", "mslp", "temp", "rain", "cape", "sst"].some((k) => r[k] != null) || r.cur?.kn != null;
  const lastWith = rows.findLast(hasData) ?? rows.at(-1);
  const t0 = rows[0].t, t1 = Math.max(lastWith.t, t0 + 600);
  const y = (t) => padT + ((t - t0) / (t1 - t0)) * Hp;
  const val = (r, k) => (k === "cur" ? r.cur?.kn ?? null : r[k] ?? null);
  const n = Math.max(1, strips.length);
  const sw = (W - padL - padR - gap * (n - 1)) / n;
  const arrows = (g, x0, key, col) => {
    let lastY = -1e9;
    for (const r of rows) {
      const d = r[key];
      if (d == null || y(r.t) - lastY < 26) continue;
      lastY = y(r.t);
      const a = ((d + 180) % 360) * Math.PI / 180, cy = y(r.t), cx = x0 + 9, L = 7;
      const dx = Math.sin(a) * L, dy = -Math.cos(a) * L;
      add("line", { x1: cx - dx, y1: cy - dy, x2: cx + dx, y2: cy + dy, stroke: col, "stroke-width": 1.6 }, g);
      add("circle", { cx: cx + dx, cy: cy + dy, r: 1.8, fill: col }, g);
    }
  };
  const series = [];
  const bands = [];
  strips.forEach((st, i) => {
    const x0 = padL + i * (sw + gap);
    bands.push({ x0, w: sw });
    const g = add("g", {});
    add("rect", { x: x0, y: padT, width: sw, height: Hp, class: "mg-bg" }, g);
    const title = st.key === "wind" ? "Wind kn (gust dashed)" : st.key === "waves" ? "Waves Hs m · Tp s" : st.title;
    add("text", { x: x0, y: 11, class: "mg-title" }, g).textContent = title;
    const keys = st.key === "wind" ? ["tws", "gust"] : st.key === "waves" ? ["wave"] : [st.key];
    const vs = rows.flatMap((r) => keys.map((k) => val(r, k))).filter((v) => v != null);
    if (!vs.length) { add("text", { x: x0 + 6, y: padT + 18, class: "mg-tick" }, g).textContent = "not in the loaded GRIBs"; return; }
    let lo = ["mslp", "temp", "sst"].includes(st.key) ? Math.min(...vs) : 0, hi = Math.max(...vs);
    const raw = (hi - lo) / 3 || 1, p10 = 10 ** Math.floor(Math.log10(raw));
    const step = [1, 2, 2.5, 5, 10].map((m) => m * p10).find((q) => q >= raw);
    lo = Math.floor(lo / step) * step;
    hi = Math.max(lo + step, Math.ceil(hi / step) * step);
    const left = st.key === "wind" || st.key === "waves" ? arrowCol : 0; // room for the arrows
    const xv = (v) => x0 + left + ((v - lo) / (hi - lo)) * (sw - left - 4);
    for (let v = lo; v <= hi + 1e-9; v += step) {
      add("line", { x1: xv(v), x2: xv(v), y1: padT, y2: padT + Hp, class: "mg-grid" }, g);
      add("text", { x: xv(v), y: padT - 5, class: "mg-tick", "text-anchor": "middle" }, g).textContent = String(+v.toFixed(2));
    }
    const path = (k, fx = xv) => {
      let d = "", pen = false;
      for (const r of rows) {
        const v = val(r, k);
        if (v == null) { pen = false; continue; }
        d += `${pen ? "L" : "M"}${fx(v).toFixed(1)},${y(r.t).toFixed(1)}`;
        pen = true;
      }
      return d;
    };
    const reg = (key, col, unit, fx = xv, above = true) => series.push({ key, xv: fx, col, unit, above, x0, w: sw });
    if (st.key === "wind") {
      reg("gust", "#c0392b", "kn"); reg("tws", "#006994", "kn", xv, false);
      add("path", { d: path("gust"), stroke: "#c0392b", fill: "none", "stroke-width": 1.2, "stroke-dasharray": "4 3" }, g);
      add("path", { d: path("tws"), stroke: "#006994", fill: "none", "stroke-width": 2 }, g);
      arrows(g, x0, "twd", "#00274d");
    } else if (st.key === "waves") {
      reg("wave", "#2a62c9", "m");
      add("path", { d: path("wave"), stroke: "#2a62c9", fill: "none", "stroke-width": 2 }, g);
      const ps = rows.map((r) => r.period).filter((v) => v != null);
      if (ps.length) {
        const ph = Math.max(4, Math.ceil(Math.max(...ps) / 4) * 4);
        const xp = (v) => x0 + left + (v / ph) * (sw - left - 4);
        reg("period", "#5d6b75", "s", xp, false);
        add("path", { d: path("period", xp), stroke: "#8a96a0", fill: "none", "stroke-width": 1.3, "stroke-dasharray": "2 2" }, g);
        for (const v of [0, ph / 2, ph]) add("text", { x: xp(v), y: padT - 17, class: "mg-tick", "text-anchor": "middle", fill: "#5d6b75" }, g).textContent = `${v} s`;
      }
      arrows(g, x0, "wavedir", "#2a62c9");
    } else if (st.bars) {
      reg(st.key, st.col, st.unit ?? "");
      const bh = Math.max(1, Hp / rows.length - 0.5);
      for (const r of rows) {
        const v = val(r, st.key);
        if (!v) continue;
        add("rect", { x: xv(lo), y: y(r.t) - bh / 2, width: xv(v) - xv(lo), height: bh, fill: st.col, opacity: 0.7 }, g);
      }
    } else {
      reg(st.key, st.col, st.unit ?? "");
      add("path", { d: path(st.key), stroke: st.col, fill: "none", "stroke-width": 1.8 }, g);
    }
  });
  // time down the left (local days and hours), with the distance along the route and its course
  const span = t1 - t0;
  const minor = span > 5 * 86400 ? 21600 : span > 36 * 3600 ? 10800 : 3600;
  for (let t = Math.ceil(t0 / minor) * minor; t <= t1; t += minor) for (const b of bands) add("line", { x1: b.x0, x2: b.x0 + b.w, y1: y(t), y2: y(t), class: "mg-vgrid" });
  const off = -new Date(t0 * 1000).getTimezoneOffset() * 60;
  const fits = (sec) => (sec / span) * Hp >= 50;
  const lab = [3600, 10800, 21600, 43200, 86400, 2 * 86400, 4 * 86400, 7 * 86400].find(fits) ?? 7 * 86400;
  const label = (t) => {
    const d = new Date(t * 1000);
    if (lab >= 86400 || (((t + off) % 86400) + 86400) % 86400 === 0) return d.toLocaleDateString("en-NZ", { weekday: "short", day: "numeric", month: "short" });
    return d.toLocaleTimeString("en-NZ", { hour: "2-digit", minute: "2-digit", hour12: false });
  };
  const first = Math.ceil((t0 + off) / Math.min(lab, 86400)) * Math.min(lab, 86400) - off;
  for (let t = first; t <= t1; t += lab) {
    const yy = y(t);
    add("line", { x1: padL - 8, x2: padL, y1: yy, y2: yy, class: "mg-vgrid", stroke: "#8a96a0" });
    add("text", { x: padL - 10, y: yy + 4, class: "mg-tick", "text-anchor": "end" }).textContent = label(t);
    const i = rows.reduce((b, r, k) => (Math.abs(r.t - t) < Math.abs(rows[b].t - t) ? k : b), 0);
    const c = courseAt(rows, i);
    add("text", { x: padL - 10, y: yy + 16, class: "mg-tick", "text-anchor": "end" }).textContent = `${rows[i].nm.toFixed(0)} NM`;
    if (c != null) {
      const gg = add("g", { transform: `translate(${padL - 14},${yy + 27}) rotate(${c.toFixed(0)})` });
      add("path", { d: "M0,-5 L3,2 L0,0 L-3,2 Z", fill: "#5d6b75" }, gg);
      add("text", { x: padL - 24, y: yy + 30, class: "mg-tick", "text-anchor": "end" }).textContent = crs3(c);
    }
  }
  const cur = add("line", { x1: padL - 4, x2: W - padR, y1: 0, y2: 0, class: "mg-cursor" });
  const marks = add("g", { class: "mg-marks" });
  const at = (e) => t0 + ((e.clientY - svg.getBoundingClientRect().top - padT) / Hp) * (t1 - t0);
  const nearest = (t) => rows.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a));
  let last = null;
  const follow = (e) => {
    const r = nearest(Math.max(t0, Math.min(t1, at(e))));
    if (r !== last) { last = r; pick(r); }
  };
  svg.style.touchAction = "none";
  svg.addEventListener("pointerdown", (e) => {
    if (e.button > 0) return;
    const r = svg.getBoundingClientRect();
    if (e.clientX - r.left < padL - 6 || e.clientX - r.left > W - padR + 6 || e.clientY - r.top < padT - 6) return;
    svg.setPointerCapture?.(e.pointerId);
    last = null; follow(e);
    const move = (ev) => follow(ev);
    const up = () => { release?.(); svg.removeEventListener("pointermove", move); svg.removeEventListener("pointerup", up); svg.removeEventListener("pointercancel", up); };
    svg.addEventListener("pointermove", move); svg.addEventListener("pointerup", up); svg.addEventListener("pointercancel", up);
    e.preventDefault();
  });
  return {
    el: svg,
    cursor(t) {
      const show = t != null && t >= t0 && t <= t1;
      cur.style.display = show ? "" : "none";
      marks.replaceChildren();
      if (!show) return;
      const cy = y(t);
      cur.setAttribute("y1", cy); cur.setAttribute("y2", cy);
      const r = nearest(t);
      const shown = [];
      for (const sr of series) {
        const v = val(r, sr.key);
        if (v == null) continue;
        const cx = sr.xv(v);
        add("circle", { cx, cy, r: 3.5, fill: sr.col, stroke: "#fff", "stroke-width": 1.2 }, marks);
        const txt = `${+v.toFixed(Math.abs(v) < 10 ? 1 : 0)}${sr.unit ? ` ${sr.unit}` : ""}`;
        const right = cx + 12 + txt.length * 6.2 < sr.x0 + sr.w;
        shown.push({ sr, txt, right, cx, y: cy + (sr.above ? -7 : 14) });
      }
      // values in one plot keep 13 px apart, within the plot
      for (const sid of new Set(shown.map((l) => l.sr.x0))) {
        const g = shown.filter((l) => l.sr.x0 === sid).sort((p, q) => p.y - q.y);
        g.forEach((l, i) => { l.y = Math.max(padT + 11, i ? Math.max(l.y, g[i - 1].y + 13) : l.y); });
        for (let i = g.length - 1; i >= 0; i--) g[i].y = Math.min(g[i].y, i < g.length - 1 ? g[i + 1].y - 13 : padT + Hp - 3);
      }
      for (const l of shown) add("text", { x: l.right ? l.cx + 8 : l.cx - 8, y: l.y, class: "mg-val", fill: l.sr.col, "text-anchor": l.right ? "start" : "end" }, marks).textContent = l.txt;
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
