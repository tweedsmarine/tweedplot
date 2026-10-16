// Fleet: a YB-tracked race. The Rust side fetches the race setup (boats,
// classes, course) and polls the light Expedition position file, keeping every
// fix on disk, so tracks grow as the race goes on. Here: the boats on the chart
// (at their latest fix, or where they were at the weather slider's time), the
// Fleet drawer and a Fleet table in the dock.
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { fmt } from "./weather.js";
import { distNm, bearing } from "./instruments.js";

const KEY = "racenav.fleet";
const DEFAULTS = { race: "", auto: 15, at: "latest", tracks: "all", labels: true, cls: "", starred: {}, hidden: {}, showCourse: true };
const PALETTE = ["#1565c0", "#c62828", "#2e7d32", "#ef6c00", "#6a1b9a", "#00838f", "#5d4037", "#ad1457", "#455a64", "#9e9d24", "#283593", "#00695c"];

export function setupFleet(map, { weather, course, routing, dock, ui }) {
  const $ = (id) => document.getElementById(id);
  let prefs = { ...DEFAULTS };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };

  let race = null; // { key, setup, fixes: { id: [{t,lat,lon}] }, meta }
  let timer = null;
  let busy = false;
  const status = (t, kind = "") => { const el = $("fl-status"); el.textContent = t; el.className = `wx-note ${kind}`; };

  // ---------- layers ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  for (const id of ["fl-course", "fl-tracks", "fl-boats"]) map.addSource(id, { type: "geojson", data: empty });
  map.addLayer({ id: "fl-course", type: "line", source: "fl-course",
    paint: { "line-color": "#6c7a83", "line-width": 1.5, "line-dasharray": [6, 4], "line-opacity": 0.8 } }, "track");
  map.addLayer({ id: "fl-tracks", type: "line", source: "fl-tracks",
    paint: { "line-color": ["get", "colour"], "line-width": ["case", ["get", "star"], 2.6, 1.4], "line-opacity": ["case", ["get", "star"], 0.95, 0.6] } }, "track");
  map.addLayer({ id: "fl-boats", type: "symbol", source: "fl-boats",
    layout: { "icon-image": "rt-tri", "icon-rotate": ["get", "cog"], "icon-rotation-alignment": "map",
              "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-size": ["case", ["get", "star"], 1.25, 0.95] },
    paint: { "icon-color": ["get", "colour"], "icon-halo-color": "#ffffff", "icon-halo-width": 1.4, "icon-opacity": ["case", ["get", "stale"], 0.5, 1] } });
  map.addLayer({ id: "fl-labels", type: "symbol", source: "fl-boats", minzoom: 4,
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "left", "icon-offset": [14, 0], "icon-allow-overlap": false } });

  // ---------- derived numbers per boat ----------------------------------------------------------
  const teams = () => race?.setup?.teams ?? [];
  const teamOf = (id) => teams().find((t) => t.id === Number(id));
  // YB's own colours where given; the rest from a palette, skipping colours already in use
  let colours = new Map();
  function assignColours() {
    colours = new Map();
    const used = new Set(teams().map((t) => t.colour).filter(Boolean));
    let k = 0;
    for (const id of boatIds()) {
      const c = teamOf(id)?.colour;
      if (c) { colours.set(id, c); continue; }
      let pick = PALETTE.find((p) => !used.has(p));
      if (!pick) pick = PALETTE[k++ % PALETTE.length];
      used.add(pick);
      colours.set(id, pick);
    }
  }
  const colourOf = (id) => colours.get(Number(id)) || PALETTE[Number(id) % PALETTE.length];
  const nameOf = (id) => teamOf(id)?.name || `Boat ${id}`;
  const starred = () => new Set(prefs.starred[race?.key] ?? []);
  const hiddenSet = () => new Set(prefs.hidden[race?.key] ?? []);
  function inClass(id) {
    if (!prefs.cls) return true;
    const t = teamOf(id);
    return !t || t.tags.includes(Number(prefs.cls));
  }
  function boatIds() {
    const ids = new Set(Object.keys(race?.fixes ?? {}).map(Number));
    for (const t of teams()) ids.add(t.id);
    return [...ids].sort((a, b) => nameOf(a).localeCompare(nameOf(b), undefined, { numeric: true }));
  }
  /** Where a boat is at time t (or its latest fix): position, course and speed made good. */
  function stateAt(id, t) {
    const f = race?.fixes?.[id];
    if (!f?.length) return null;
    const last = f.at(-1);
    let i = f.length - 1, pos = last, stale = false;
    if (t != null) {
      if (t < f[0].t) return null;
      if (t >= last.t) { stale = t - last.t > 6 * 3600; }
      else {
        let lo = 0, hi = f.length - 1;
        while (hi - lo > 1) { const m = (lo + hi) >> 1; if (f[m].t <= t) lo = m; else hi = m; }
        const a = f[lo], b = f[hi], k = (t - a.t) / (b.t - a.t || 1);
        pos = { t, lat: a.lat + (b.lat - a.lat) * k, lon: a.lon + (b.lon - a.lon) * k,
                dtf: a.dtf != null && b.dtf != null ? a.dtf + (b.dtf - a.dtf) * k : null };
        i = hi;
      }
    }
    const prev = i > 0 ? f[i - 1] : null, cur = f[i];
    const dt = prev ? (cur.t - prev.t) / 3600 : 0;
    const sog = prev && dt > 0 ? distNm([prev.lon, prev.lat], [cur.lon, cur.lat]) / dt : null;
    const cog = prev && sog > 0.05 ? bearing([prev.lon, prev.lat], [cur.lon, cur.lat]) : null;
    // distance sailed in the last 24 h (between fixes)
    const day = f.filter((x) => x.t >= cur.t - 86400 && x.t <= cur.t);
    let d24 = 0;
    for (let j = 1; j < day.length; j++) d24 += distNm([day[j - 1].lon, day[j - 1].lat], [day[j].lon, day[j].lat]);
    return { lat: pos.lat, lon: pos.lon, t: pos.t, fixT: cur.t, sog, cog, stale, d24: day.length > 1 ? d24 : null, dtf: pos.dtf ?? dtf(pos), ybDtf: pos.dtf != null };
  }
  /** Index of the YB course point a boat at p is heading for (end of the nearest leg). */
  function nextNode(p) {
    const c = race?.setup?.course ?? [];
    let best = { off: Infinity, i: 0 };
    for (let i = 0; i + 1 < c.length; i++) {
      const off = crossTrack([p.lon, p.lat], [c[i].lon, c[i].lat], [c[i + 1].lon, c[i + 1].lat]);
      if (off < best.off) best = { off, i };
    }
    return best.i + 1;
  }
  /** Distance to finish along the YB course: to the end of the nearest course leg, then the legs after it. */
  function dtf(p) {
    const c = race?.setup?.course ?? [];
    if (c.length < 2) return null;
    let best = null;
    for (let i = 0; i + 1 < c.length; i++) {
      const a = [c[i].lon, c[i].lat], b = [c[i + 1].lon, c[i + 1].lat];
      const off = crossTrack([p.lon, p.lat], a, b);
      if (!best || off < best.off) best = { off, i };
    }
    let d = distNm([p.lon, p.lat], [c[best.i + 1].lon, c[best.i + 1].lat]);
    for (let j = best.i + 1; j + 1 < c.length; j++) d += distNm([c[j].lon, c[j].lat], [c[j + 1].lon, c[j + 1].lat]);
    return d;
  }
  function crossTrack(p, a, b) {
    // distance from p to the segment a-b (flat approximation, fine at leg scale)
    const k = Math.cos((p[1] * Math.PI) / 180);
    const ax = (a[0] - p[0]) * k, ay = a[1] - p[1], bx = (b[0] - p[0]) * k, by = b[1] - p[1];
    const dx = bx - ax, dy = by - ay, L = dx * dx + dy * dy;
    const u = L > 0 ? Math.max(0, Math.min(1, -(ax * dx + ay * dy) / L)) : 0;
    return Math.hypot(ax + u * dx, ay + u * dy) * 60;
  }

  // ---------- drawing ----------------------------------------------------------
  const atTime = () => (prefs.at === "weather" ? weather.current().t || null : null);
  function draw() {
    if (!race) { for (const id of ["fl-course", "fl-tracks", "fl-boats"]) map.getSource(id).setData(empty); return; }
    const t = atTime(), st = starred(), hid = hiddenSet();
    const tracks = [], boats = [];
    boatIds().forEach((id, i) => {
      if (hid.has(id) || !inClass(id)) return;
      const s = stateAt(id, t);
      const colour = colourOf(id, i), star = st.has(id);
      const f = race.fixes[id] ?? [];
      if (prefs.tracks === "all" || (prefs.tracks === "starred" && star)) {
        const pts = (t == null ? f : f.filter((x) => x.t <= t)).map((x) => [x.lon, x.lat]);
        if (s && t != null) pts.push([s.lon, s.lat]);
        if (pts.length > 1) tracks.push({ type: "Feature", properties: { colour, star }, geometry: { type: "LineString", coordinates: unwrap(pts) } });
      }
      if (s) boats.push({ type: "Feature", properties: { id, colour, star, stale: s.stale, cog: s.cog ?? 0, name: prefs.labels ? nameOf(id) : "" },
        geometry: { type: "Point", coordinates: [s.lon, s.lat] } });
    });
    map.getSource("fl-tracks").setData({ type: "FeatureCollection", features: tracks });
    map.getSource("fl-boats").setData({ type: "FeatureCollection", features: boats });
    const c = race.setup?.course ?? [];
    map.getSource("fl-course").setData(prefs.showCourse && c.length > 1
      ? { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: unwrap(c.map((p) => [p.lon, p.lat])) } } : empty);
    renderList();
    dock.refresh("fleet");
  }
  weather.onTime(() => { if (prefs.at === "weather") { draw(); } });

  // ---------- drawer ----------------------------------------------------------
  function renderClasses() {
    const sel = $("fl-class");
    sel.innerHTML = "";
    sel.append(new Option("All classes", ""));
    for (const tg of [...(race?.setup?.tags ?? [])].sort((a, b) => a.sort - b.sort)) sel.append(new Option(tg.name, String(tg.id)));
    sel.value = [...sel.options].some((o) => o.value === prefs.cls) ? prefs.cls : "";
    prefs.cls = sel.value;
  }
  function renderList() {
    const ul = $("fl-list");
    ul.innerHTML = "";
    if (!race) { ul.innerHTML = `<li class="empty">Type a YB race name (as in yb.tl/<i>race</i>), then Open.</li>`; return; }
    const t = atTime(), st = starred(), hid = hiddenSet(), now = Date.now() / 1000;
    const ids = boatIds().filter(inClass);
    if (!ids.length) ul.innerHTML = `<li class="empty">No boats yet.</li>`;
    ids.forEach((id) => {
      const s = stateAt(id, t), tm = teamOf(id);
      const li = document.createElement("li");
      li.innerHTML = `<input type="checkbox" title="Show on the chart"><button class="star" title="Follow: highlighted, track kept with Tracks: followed">★</button><span class="sw"></span>` +
        `<div class="main" title="Centre on this boat"><b></b><small></small></div><button data-act="route" title="Route from this boat's fix: start position and time">Route</button>`;
      const all = boatIds();
      li.querySelector("input").checked = !hid.has(id);
      li.querySelector("input").onchange = (e) => { toggle(prefs.hidden, id, !e.target.checked); save(); draw(); };
      li.querySelector(".star").classList.toggle("on", st.has(id));
      li.querySelector(".star").onclick = () => { toggle(prefs.starred, id, !st.has(id)); save(); draw(); };
      li.querySelector(".sw").style.background = colourOf(id, all.indexOf(id));
      li.querySelector("b").textContent = `${nameOf(id)}${tm?.sail ? ` · ${tm.sail}` : ""}`;
      li.querySelector("small").textContent = s
        ? [`fix ${age(now - s.fixT)} ago`, s.sog != null ? `${s.sog.toFixed(1)} kn` : "", s.cog != null ? `${pad3(s.cog)}°` : "", s.dtf != null ? `DTF ${s.dtf.toFixed(0)} NM` : "", tm?.status && tm.status !== "RACING" ? tm.status.toLowerCase() : ""].filter(Boolean).join(" · ")
        : (tm?.model || "no position yet");
      li.querySelector(".main").onclick = () => { if (s) map.easeTo({ center: [s.lon, s.lat] }); };
      const rb = li.querySelector('[data-act="route"]');
      rb.disabled = !s;
      rb.onclick = () => routeFrom(id);
      ul.append(li);
    });
  }
  function toggle(obj, id, on) {
    const k = race.key;
    const set = new Set(obj[k] ?? []);
    if (on) set.add(id); else set.delete(id);
    obj[k] = [...set];
  }
  function routeFrom(id) {
    const f = race?.fixes?.[id];
    if (!f?.length) return;
    const last = f.at(-1);
    // If the Course panel holds the YB course, drop the marks this boat has already passed.
    const c = race.setup?.course ?? [];
    const cur = course.get().marks;
    const same = (a, b) => Math.abs(a.lat - b.lat) < 1e-6 && Math.abs(a.lon - b.lon) < 1e-6;
    const isYb = c.length > 1 && cur.length && cur.length <= c.length && cur.every((m, i) => same(m, c[c.length - cur.length + i]));
    if (isYb) {
      const next = nextNode(last);
      course.fromPoints([{ name: nameOf(id), lat: last.lat, lon: last.lon }, ...c.slice(next)], course.get().name || race.setup.title);
    } else {
      course.startAt([last.lon, last.lat]);
    }
    routing.setStartTime(last.t);
    const label = `${nameOf(id)} · fix ${utc(last.t)}`;
    const col = colourOf(id);
    routing.setRunLabel(label, [last.lon, last.lat], last.t, col && col.toLowerCase() !== "#ffffff" ? col : null);
    routing.msg(`Start set to ${nameOf(id)}'s fix at ${utc(last.t)}. Routings from here will be named “${label}”. Compute when ready (polar in Boat).`);
    ui.openTab("routing", "run");
  }

  // ---------- open / refresh ----------------------------------------------------------
  async function open(key) {
    key = key.trim();
    if (!key) return;
    status("Opening…");
    try {
      const r = await invoke("yb_open", { race: key });
      race = { key: r.key, setup: r.setup, fixes: toFixes(r.fixes), meta: r.meta };
      prefs.race = r.key; save();
      $("fl-race").value = r.key;
      $("fl-title").textContent = r.setup?.title || r.key;
      renderClasses();
      assignColours();
      $("fl-course-copy").disabled = !(r.setup?.course?.length > 1);
      draw();
      if (r.error) status(r.error, "warn");
      // the full history when this race has none yet, or once it is 6 hours old
      const h = r.meta?.history_fetched;
      if (!h || Date.now() / 1000 - h > 6 * 3600) await loadHistory();
      await refresh();
      fitFleet();
      schedule();
      loadRaceList();
    } catch (e) { status(`Could not open “${key}”: ${e}`, "bad"); }
  }
  async function refresh() {
    if (!race || busy) return;
    busy = true;
    $("fl-refresh").disabled = true;
    try {
      const since = Math.max(0, ...Object.values(race.fixes).map((f) => f.at(-1)?.t ?? 0));
      const r = await invoke("yb_refresh", { race: race.key, since: 0 });
      race.fixes = toFixes(r.fixes);
      race.meta = r.meta;
      assignColours();
      const newest = Math.max(0, ...Object.values(race.fixes).map((f) => f.at(-1)?.t ?? 0));
      status(`${r.boats} boats · ${r.added} new fixes · latest fix ${newest ? fmt(newest, true) : "—"}${newest > since && since ? "" : ""} · checked ${new Date().toLocaleTimeString("en-NZ", { hour12: false })}`, "good");
      draw();
    } catch (e) {
      status(`Positions: ${e}`, "bad");
    } finally { busy = false; $("fl-refresh").disabled = false; }
  }
  /** Every published fix of every boat (YB's history file). */
  async function loadHistory() {
    if (!race) return;
    status("Downloading the full tracks…");
    $("fl-history").disabled = true;
    try {
      const r = await invoke("yb_history", { race: race.key });
      race.fixes = toFixes(r.fixes);
      race.meta = r.meta;
      assignColours();
      status(`Full tracks: ${r.received} fixes for ${r.boats} boats (${r.added} new).`, "good");
      draw();
    } catch (e) {
      status(`Full tracks not available: ${e}`, "warn");
    } finally { $("fl-history").disabled = false; }
  }

  function schedule() {
    clearInterval(timer);
    if (race && prefs.auto > 0) timer = setInterval(refresh, prefs.auto * 60000);
  }
  function fitFleet() {
    const pts = Object.values(race?.fixes ?? {}).map((f) => f.at(-1)).filter(Boolean).map((x) => [x.lon, x.lat]);
    if (!pts.length) return;
    const b = new maplibregl.LngLatBounds(pts[0], pts[0]);
    for (const p of pts) b.extend(p);
    map.fitBounds(b, { padding: { top: 80, bottom: 140, left: 440, right: 180 }, maxZoom: 11, duration: 600 });
  }
  async function loadRaceList() {
    const list = (await invoke("yb_races").catch(() => [])) || [];
    const dl = $("fl-races");
    dl.innerHTML = "";
    for (const r of list) { const o = document.createElement("option"); o.value = r.key; o.label = r.title || r.key; dl.append(o); }
  }

  $("fl-open").onclick = () => open($("fl-race").value);
  $("fl-race").onkeydown = (e) => { if (e.key === "Enter") open($("fl-race").value); };
  $("fl-refresh").onclick = refresh;
  $("fl-history").onclick = loadHistory;
  const auto = $("fl-auto");
  for (const [v, l] of [[0, "Off"], [5, "5 min"], [10, "10 min"], [15, "15 min"], [30, "30 min"], [60, "1 h"], [180, "3 h"]]) auto.append(new Option(l, String(v)));
  auto.value = String(prefs.auto);
  auto.onchange = () => { prefs.auto = Number(auto.value); save(); schedule(); };
  for (const [id, key] of [["fl-at", "at"], ["fl-tracks-sel", "tracks"]]) {
    $(id).value = prefs[key];
    $(id).onchange = (e) => { prefs[key] = e.target.value; save(); draw(); };
  }
  $("fl-labels").checked = prefs.labels;
  $("fl-labels").onchange = (e) => { prefs.labels = e.target.checked; save(); draw(); };
  $("fl-show-course").checked = prefs.showCourse;
  $("fl-show-course").onchange = (e) => { prefs.showCourse = e.target.checked; save(); draw(); };
  $("fl-class").onchange = (e) => { prefs.cls = e.target.value; save(); draw(); };
  $("fl-table").onclick = () => dock.toggle("fleet");
  $("fl-course-copy").onclick = () => {
    const c = race?.setup?.course ?? [];
    if (c.length < 2) return;
    course.fromPoints(c, race.setup.title || race.key);
    status(`Course copied to the Course panel (${c.length} points). Save it there to keep it in the library.`, "good");
  };

  // hover and click on a boat
  const popup = new maplibregl.Popup({ closeButton: false, closeOnClick: false, offset: 10 });
  map.on("mousemove", "fl-boats", (e) => {
    const id = e.features[0].properties.id, s = stateAt(id, atTime()), tm = teamOf(id);
    if (!s) return;
    map.getCanvas().style.cursor = "pointer";
    popup.setLngLat([s.lon, s.lat]).setHTML(`<div class="rt-popup"><b></b><br><span></span><br>fix ${fmt(s.fixT)}` +
      `${s.sog != null ? `<br>${s.sog.toFixed(1)} kn · ${s.cog != null ? pad3(s.cog) + "°" : "—"} (between fixes)` : ""}` +
      `${s.dtf != null ? `<br>DTF ${s.dtf.toFixed(1)} NM${s.ybDtf ? " (YB)" : ""}` : ""}${s.d24 != null ? ` · 24 h ${s.d24.toFixed(0)} NM` : ""}</div>`).addTo(map);
    const el = popup.getElement();
    el.querySelector("b").textContent = nameOf(id);
    el.querySelector("span").textContent = [tm?.sail, tm?.model, tm?.skipper].filter(Boolean).join(" · ");
  });
  map.on("mouseleave", "fl-boats", () => { map.getCanvas().style.cursor = ""; popup.remove(); });

  // ---------- dock: fleet table ----------------------------------------------------------
  let sortKey = "dtf", sortDir = 1;
  const COLS = [
    ["rank", "#"], ["name", "Boat"], ["cls", "Class"], ["fix", "Fix (UTC)"], ["age", "Age"], ["sog", "SOG"], ["cog", "COG"],
    ["d24", "24 h"], ["dtf", "DTF"], ["dtl", "DTL"], ["pos", "Position"],
  ];
  dock.register("fleet", {
    title: "Fleet",
    render(body) {
      if (!race) { body.innerHTML = `<p class="wx-note">Open a YB race in the Fleet panel.</p>`; return; }
      const t = atTime(), now = Date.now() / 1000, st = starred();
      const rows = boatIds().filter(inClass).map((id) => {
        const s = stateAt(id, t), tm = teamOf(id);
        const cls = (tm?.tags ?? []).map((x) => race.setup.tags.find((y) => y.id === x)?.name).filter((n) => n && !/^all/i.test(n)).join(", ");
        return { id, s, name: nameOf(id), cls, fix: s?.fixT, age: s ? now - s.fixT : null, sog: s?.sog, cog: s?.cog, d24: s?.d24, dtf: s?.dtf, star: st.has(id), status: tm?.status };
      });
      const lead = Math.min(...rows.map((r) => r.dtf).filter((x) => x != null));
      for (const r of rows) r.dtl = r.dtf != null && Number.isFinite(lead) ? r.dtf - lead : null;
      [...rows].filter((r) => r.dtf != null).sort((a, b) => a.dtf - b.dtf).forEach((r, i) => { r.rank = i + 1; });
      rows.sort((a, b) => cmp(a[sortKey], b[sortKey]) * sortDir);
      const tbl = document.createElement("table");
      tbl.className = "cmp";
      tbl.innerHTML = `<thead><tr>${COLS.map(([k, l]) => `<th data-k="${k}" class="${k === sortKey ? (sortDir > 0 ? "asc" : "desc") : ""}">${l}</th>`).join("")}</tr></thead><tbody></tbody>`;
      const all = boatIds();
      for (const r of rows) {
        const tr = document.createElement("tr");
        tr.classList.toggle("active", r.star);
        const cell = {
          rank: r.rank ?? "—", name: "", cls: r.cls || "—", fix: r.fix ? utc(r.fix) : "—",
          age: r.age != null ? age(r.age) : "—", sog: r.sog != null ? `${r.sog.toFixed(1)} kn` : "—", cog: r.cog != null ? `${pad3(r.cog)}°` : "—",
          d24: r.d24 != null ? `${r.d24.toFixed(0)} NM` : "—", dtf: r.dtf != null ? `${r.dtf.toFixed(1)} NM` : "—", dtl: r.dtl != null ? `${r.dtl.toFixed(1)} NM` : "—",
          pos: r.s ? `${ddm(r.s.lat, "N", "S")} ${ddm(r.s.lon, "E", "W")}` : "—",
        };
        tr.innerHTML = COLS.map(([k]) => `<td class="${k === "name" ? "nm" : ""}">${k === "name" ? `<span class="sw"></span><span></span>` : esc(String(cell[k]))}</td>`).join("");
        tr.querySelector(".sw").style.background = colourOf(r.id, all.indexOf(r.id));
        tr.querySelector(".nm span:last-child").textContent = r.name + (r.status && r.status !== "RACING" ? ` (${r.status.toLowerCase()})` : "");
        tr.onclick = () => { if (r.s) map.easeTo({ center: [r.s.lon, r.s.lat], zoom: Math.max(map.getZoom(), 7) }); };
        tbl.tBodies[0].append(tr);
      }
      tbl.tHead.onclick = (e) => {
        const k = e.target.closest("th")?.dataset.k;
        if (!k) return;
        if (sortKey === k) sortDir = -sortDir; else { sortKey = k; sortDir = 1; }
        dock.refresh("fleet");
      };
      const note = document.createElement("p");
      note.className = "wx-note";
      note.textContent = `${prefs.at === "weather" ? `Positions at the weather time, ${fmt(t ?? 0, true)}` : "Latest fixes"}. SOG and COG are made good between the last two fixes; DTF is YB's own where the full tracks give it, otherwise measured along the YB course.`;
      body.append(tbl, note);
    },
  });

  loadRaceList();
  renderList();
  if (prefs.race) { $("fl-race").value = prefs.race; open(prefs.race); }
  return { refresh, current: () => race };
}

function utc(t) {
  const d = new Date(t * 1000);
  return `${d.getUTCDate()} ${d.toLocaleString("en-GB", { month: "short", timeZone: "UTC" })} ${d.toISOString().slice(11, 16)}Z`;
}
function toFixes(o) {
  const out = {};
  for (const [k, v] of Object.entries(o ?? {})) out[k] = v.map((f) => (Array.isArray(f) ? { t: f[0], lat: f[1], lon: f[2] } : f)).sort((a, b) => a.t - b.t);
  return out;
}
/** Keep lines continuous across the date line. */
function unwrap(pts) {
  const out = [];
  let off = 0;
  for (let i = 0; i < pts.length; i++) {
    if (i > 0) { const d = pts[i][0] + off - out[i - 1][0]; if (d > 180) off -= 360; else if (d < -180) off += 360; }
    out.push([pts[i][0] + off, pts[i][1]]);
  }
  return out;
}
function age(s) {
  if (s < 0) s = 0;
  if (s < 3600) return `${Math.round(s / 60)} min`;
  if (s < 86400 * 2) return `${(s / 3600).toFixed(s < 36000 ? 1 : 0)} h`;
  return `${Math.round(s / 86400)} d`;
}
function cmp(a, b) {
  if (a === b) return 0;
  if (a === null || a === undefined) return 1;
  if (b === null || b === undefined) return -1;
  return typeof a === "number" && typeof b === "number" ? a - b : String(a).localeCompare(String(b), undefined, { numeric: true });
}
function pad3(x) { return String(Math.round(x) % 360).padStart(3, "0"); }
function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]); }
function ddm(deg, pos, neg) {
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}
