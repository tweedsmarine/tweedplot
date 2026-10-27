// Libraries alongside: another library (a mate's marks, another season's) drawn on the chart
// with the one in use, read-only, never mixed in. Each has its own colour, as a ring round
// its marks; clicking one shows what it is. The Whole library filters apply to them too, and
// their tracks are drawn when those filters pick 200 or fewer. The list of libraries shown
// is kept (racenav.overlays) and opened again at start.

import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
import { addMarkLayers, plainLibrary } from "./library.js";
import { sharedTest, sharedActive, onShared } from "./libfilter.js";
import { markInTrip, trackInTrip } from "./tripstore.js";
import { infoBox } from "./infobox.js";

const KEY = "racenav.overlays";
const COLOURS = ["#8e44ad", "#16a085", "#d35400", "#2c3e50", "#c2185b", "#7f8c00"];
const MAX_TRACKS = 200;
const $ = (id) => document.getElementById(id);

export function setupOverlays(map, { library }) {
  let list = [];
  try { list = JSON.parse(localStorage.getItem(KEY) || "[]"); } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(list.map(({ path, name, colour, shown, tracks }) => ({ path, name, colour, shown, tracks })))); } catch { /* ignore */ } };
  const data = new Map(); // path → { marks, tracks, error }
  const trackCache = new Map(); // `${path}|${id}` → track

  // ---------- chart: their marks (with a ring in their colour) and tracks, under the library's ----------
  const empty = { type: "FeatureCollection", features: [] };
  map.addSource("ovl-tracks", { type: "geojson", data: empty });
  map.addLayer({ id: "ovl-tracks", type: "line", source: "ovl-tracks", layout: { "line-cap": "round", "line-join": "round" },
    paint: { "line-color": ["get", "oc"], "line-width": 2, "line-opacity": 0.75, "line-dasharray": [3, 2] } }, "lib-buoys");
  addMarkLayers(map, "ovl", "lib-buoys");
  map.addLayer({ id: "ovl-ring", type: "circle", source: "ovl-marks",
    paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], 5, 5, 10, 13], "circle-color": "rgba(0,0,0,0)",
             "circle-stroke-color": ["get", "oc"], "circle-stroke-width": ["interpolate", ["linear"], ["zoom"], 5, 1, 10, 2.5], "circle-stroke-opacity": 0.9 } }, "ovl-buoys");
  for (const id of ["ovl-buoys", "ovl-poi", "ovl-sym", "ovl-labels"]) map.setPaintProperty(id, "icon-opacity", 0.85);

  const SHARED_MARK = { time: (m) => (m.date_unknown ? null : m.created || null), inTrip: markInTrip };
  const SHARED_TRK = { time: (t) => (t.stats?.start ? [t.stats.start, t.stats.end ?? t.stats.start] : null), inTrip: trackInTrip };

  let drawing = 0;
  async function draw() {
    const run = ++drawing;
    const marks = [], lines = [];
    for (const [i, o] of list.entries()) {
      const d = data.get(o.path);
      if (!o.shown || !d?.marks) continue;
      for (const m of d.marks) {
        if (sharedActive() && !sharedTest(m, SHARED_MARK)) continue;
        const p = { id: m.id, name: m.name, kind: m.kind, oc: o.colour, ovl: i };
        if (m.colour) p.colour = m.colour;
        marks.push({ type: "Feature", properties: p, geometry: { type: "Point", coordinates: [m.lon, m.lat] } });
      }
    }
    map.getSource("ovl-marks").setData({ type: "FeatureCollection", features: marks });
    for (const o of list) {
      const d = data.get(o.path);
      if (!o.shown || !o.tracks || !d?.tracks || !sharedActive()) continue;
      const ts = d.tracks.filter((t) => sharedTest(t, SHARED_TRK));
      if (ts.length > MAX_TRACKS) continue;
      for (const t of ts) {
        const k = `${o.path}|${t.id}`;
        if (!trackCache.has(k)) { try { trackCache.set(k, await invoke("overlay_track", { path: o.path, id: t.id })); } catch { continue; } }
        if (run !== drawing) return; // a newer draw started
        for (const seg of trackCache.get(k).segments) if (seg.length > 1) lines.push({ type: "Feature", properties: { oc: o.colour }, geometry: { type: "LineString", coordinates: seg.map((p) => [p[0], p[1]]) } });
      }
    }
    if (run === drawing) map.getSource("ovl-tracks").setData({ type: "FeatureCollection", features: lines });
    renderChip();
  }
  onShared(() => draw());

  // ---------- clicking one of their marks: what it is (read-only) ----------
  map.on("click", "ovl-ring", (e) => {
    if (document.querySelector(".armed")) return;
    const f = e.features[0];
    const o = list[f.properties.ovl];
    const m = data.get(o?.path)?.marks?.find((x) => x.id === f.properties.id);
    if (!m) return;
    const rows = [["Library", `${o.name} (alongside, read-only)`]];
    if (m.group) rows.push(["Group", m.group]);
    rows.push(["Made", m.date_unknown ? "date unknown" : m.created ? new Date(m.created * 1000).toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" }) : "–"]);
    if (m.depth != null) rows.push(["Depth", `${m.depth.toFixed(1)} m`]);
    if (m.temp != null) rows.push(["Sea temperature", `${m.temp.toFixed(1)} °C`]);
    if (m.notes) rows.push(["Notes", m.notes]);
    const el = document.createElement("div");
    el.className = "feat-pop";
    el.innerHTML = `<table>${rows.map(() => "<tr><td></td><td></td></tr>").join("")}</table>`;
    el.querySelectorAll("tr").forEach((tr, i) => { tr.cells[0].textContent = rows[i][0]; tr.cells[1].textContent = rows[i][1]; });
    infoBox(map, "overlay-mark", { width: 280, height: 200 }).open(el, { title: m.name || "Mark", at: [m.lon, m.lat] });
  });
  map.on("mouseenter", "ovl-ring", () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "ovl-ring", () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = ""; });

  // ---------- opening them ----------
  async function load(o) {
    try {
      const r = await invoke("overlay_add", { path: o.path });
      o.name = r.name;
      const [marks, tracks] = await Promise.all([invoke("overlay_marks", { path: o.path }), invoke("overlay_tracks", { path: o.path })]);
      data.set(o.path, { marks, tracks });
    } catch (e) {
      data.set(o.path, { error: String(e) });
    }
  }
  async function add() {
    let p = await invoke("library_pick_db", { save: false, title: "Show a library alongside", suggest: null }).catch(() => null);
    if (p) p = await plainLibrary(p, false);
    if (!p) return;
    if (list.some((o) => o.path === p)) { activity.note("That library is already shown alongside"); return; }
    const used = new Set(list.map((o) => o.colour));
    const o = { path: p, name: p.split(/[\\/]/).pop().replace(/\.gpkg$/i, ""), colour: COLOURS.find((c) => !used.has(c)) ?? COLOURS[list.length % COLOURS.length], shown: true, tracks: true };
    const job = activity.start(`Showing ${o.name} alongside`);
    await load(o);
    const d = data.get(o.path);
    if (d.error) { job.fail(d.error); data.delete(o.path); return; }
    list.push(o);
    save();
    job.done(`${d.marks.length.toLocaleString()} marks and ${d.tracks.length.toLocaleString()} tracks, read-only.`, { details: [p, "Its tracks are drawn when a Whole library filter (dates, months, trip) picks 200 or fewer."] });
    render(); draw();
  }
  async function remove(o) {
    list = list.filter((x) => x !== o);
    data.delete(o.path);
    for (const k of [...trackCache.keys()]) if (k.startsWith(`${o.path}|`)) trackCache.delete(k);
    await invoke("overlay_remove", { path: o.path }).catch(() => {});
    save(); render(); draw();
  }

  // ---------- Library > Libraries alongside ----------
  function render() {
    const ul = $("ovl-list");
    ul.innerHTML = "";
    if (!list.length) ul.innerHTML = `<li class="wx-note">None shown yet.</li>`;
    for (const o of list) {
      const d = data.get(o.path) ?? {};
      const li = document.createElement("li");
      li.className = "ovl-row";
      li.innerHTML = `<input type="checkbox" class="ovl-shown" title="Show it on the chart"><input type="color" class="ovl-colour" title="Its colour on the chart">
        <div class="main"><b></b><small></small></div><label class="ovl-trk" title="Draw its tracks (when a Whole library filter picks 200 or fewer)"><input type="checkbox"> Tracks</label><button class="ovl-rm" title="Stop showing it (the library itself isn't changed)">Remove</button>`;
      li.querySelector("b").textContent = o.name;
      li.querySelector("small").textContent = d.error ? `Can't open it: ${d.error}` : `${(d.marks?.length ?? 0).toLocaleString()} marks · ${(d.tracks?.length ?? 0).toLocaleString()} tracks · ${o.path}`;
      const shown = li.querySelector(".ovl-shown");
      shown.checked = o.shown;
      shown.onchange = () => { o.shown = shown.checked; save(); draw(); };
      const col = li.querySelector(".ovl-colour");
      col.value = o.colour;
      col.onchange = () => { o.colour = col.value; save(); draw(); };
      const trk = li.querySelector(".ovl-trk input");
      trk.checked = !!o.tracks;
      trk.onchange = () => { o.tracks = trk.checked; save(); draw(); };
      li.querySelector(".ovl-rm").onclick = () => remove(o);
      ul.append(li);
    }
    renderChip();
  }
  function renderChip() {
    const chip = $("lib-ovl-chip");
    const on = list.filter((o) => o.shown && !data.get(o.path)?.error);
    chip.hidden = !list.length;
    chip.textContent = on.length ? `+ ${on.map((o) => o.name).join(", ")}` : "+ alongside (hidden)";
    chip.classList.toggle("on", on.length > 0);
  }
  $("ovl-add").onclick = add;
  $("lib-ovl-chip").onclick = () => { library.showSub("overlays"); render(); };
  window.addEventListener("racenav-overlays-open", render);

  (async () => {
    for (const o of list) await load(o);
    const bad = list.filter((o) => data.get(o.path)?.error);
    if (bad.length) activity.start("Libraries alongside").fail(`${bad.length === 1 ? `${bad[0].name} couldn't be opened` : `${bad.length} couldn't be opened`}`, { details: bad.map((o) => `${o.path}: ${data.get(o.path).error}`) });
    render(); draw();
  })();
  return { draw };
}
