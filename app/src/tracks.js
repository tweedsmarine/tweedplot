// Library > Tracks: tracks recorded from Signal K and tracks imported from GPX.
// Shown on the chart in their own colours; optionally a dot where each track's
// boat was at the weather time (replays and comparisons); click a track for the
// time, SOG and COG there. The recorder itself runs in Rust (it keeps logging
// while plotting is paused); this module starts and stops it and shows its progress.
import { infoBox } from "./infobox.js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const KEY = "racenav.tracks";
const PALETTE = ["#d35400", "#8e44ad", "#16a085", "#c0392b", "#2c3e50", "#27ae60", "#2980b9", "#b7950b"];
const empty = { type: "FeatureCollection", features: [] };

const fmtTime = (t) => (t == null ? "–" : new Date(t * 1000).toLocaleString("en-NZ", { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" }));
const fmtClock = (t) => new Date(t * 1000).toLocaleTimeString("en-NZ", { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" });
function fmtDur(s) {
  if (s == null || s <= 0) return "";
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return h >= 24 ? `${Math.floor(h / 24)} d ${h % 24} h` : h ? `${h} h ${String(m).padStart(2, "0")} m` : `${m} min`;
}

export function setupTracks(map, { before, getTime, onTime, status }) {
  const $ = (id) => document.getElementById(id);
  const prefs = { shown: [], atTime: true, colourBy: "track", range: {} };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };

  let list = []; // summaries
  const cache = new Map(); // id -> { segments: [[lon, lat, t, sog, cog]...] }
  let rec = { on: false };

  // ---------- chart ----------------------------------------------------------
  map.addSource("trk-lines", { type: "geojson", data: empty });
  map.addSource("trk-now", { type: "geojson", data: empty });
  map.addLayer({ id: "trk-lines", type: "line", source: "trk-lines", layout: { "line-join": "round", "line-cap": "round" },
    paint: { "line-color": ["get", "colour"], "line-width": 2.2, "line-opacity": 0.9 } }, before);
  map.addLayer({ id: "trk-now", type: "circle", source: "trk-now",
    paint: { "circle-radius": 5, "circle-color": ["get", "colour"], "circle-stroke-color": "#fff", "circle-stroke-width": 1.5 } }, before);

  const colourOf = (t) => t.colour || PALETTE[Math.abs([...t.id].reduce((a, c) => a * 31 + c.charCodeAt(0), 7)) % PALETTE.length];

  async function load(id) {
    if (!cache.has(id)) cache.set(id, await invoke("track_get", { id }));
    return cache.get(id);
  }

  async function draw() {
    const feats = [];
    const shown = [];
    for (const t of list.filter((x) => prefs.shown.includes(x.id))) {
      try { shown.push([t, await load(t.id)]); } catch { /* gone */ }
    }
    const by = BY[prefs.colourBy];
    const range = by && rangeOf(by, shown);
    for (const [t, d] of shown) {
      const colour = colourOf(t);
      for (const seg of d.segments) {
        if (seg.length < 2) continue;
        if (!range) { feats.push({ type: "Feature", properties: { id: t.id, colour }, geometry: { type: "LineString", coordinates: seg.map((p) => [p[0], p[1]]) } }); continue; }
        // runs of fixes in the same colour step: one line each (not one per fix)
        const [lo, hi] = range, step = (hi - lo) / STEPS || 1;
        const bin = (p) => (p[by.i] == null ? null : Math.max(0, Math.min(STEPS, Math.round((p[by.i] - lo) / step))));
        let start = 0, b = bin(seg[0]);
        for (let k = 1; k <= seg.length; k++) {
          const nb = k < seg.length ? bin(seg[k]) : undefined;
          if (nb !== b || k === seg.length) {
            const run = seg.slice(start, Math.min(k + 1, seg.length)).map((p) => [p[0], p[1]]);
            if (run.length > 1) feats.push({ type: "Feature", properties: b == null ? { id: t.id } : { id: t.id, v: lo + b * step }, geometry: { type: "LineString", coordinates: run } });
            start = k; b = nb;
          }
        }
      }
    }
    map.setPaintProperty("trk-lines", "line-color", range
      ? ["case", ["has", "v"], ["interpolate", ["linear"], ["get", "v"], ...by.ramp.flatMap((c, i) => [range[0] + ((range[1] - range[0]) * i) / (by.ramp.length - 1), c])], NO_VALUE]
      : ["get", "colour"]);
    map.getSource("trk-lines").setData({ type: "FeatureCollection", features: feats });
    legend(by, range, shown.length);
    drawNow();
  }

  // ---------- colour by sea temperature, depth or speed ----------------------------------
  const STEPS = 40;
  const NO_VALUE = "#9aa3a8";
  // fix layout from track_get: [lon, lat, t, sog, cog, depth, temp]
  const BY = {
    temp: { i: 6, name: "Sea temperature", unit: "°C", dp: 1, ramp: ["#313695", "#4575b4", "#74add1", "#abd9e9", "#fee090", "#fdae61", "#f46d43", "#d73027", "#a50026"] },
    depth: { i: 5, name: "Depth", unit: "m", dp: 0, ramp: ["#ffffb2", "#a1dab4", "#41b6c4", "#2c7fb8", "#253494", "#081d58"] },
    sog: { i: 3, name: "Speed (SOG)", unit: "kn", dp: 1, ramp: ["#2c7bb6", "#abd9e9", "#ffffbf", "#fdae61", "#d7191c"] },
  };
  /** The range to colour over: the one set in the drawer, else most of the shown tracks' values. */
  function rangeOf(by, shown) {
    const set = prefs.range[prefs.colourBy] || [];
    const vals = [];
    let n = 0;
    for (const [, d] of shown) for (const seg of d.segments) n += seg.length;
    const every = Math.max(1, Math.floor(n / 100000));
    let k = 0;
    for (const [, d] of shown) for (const seg of d.segments) for (const p of seg) if (k++ % every === 0 && p[by.i] != null) vals.push(p[by.i]);
    if (!vals.length) return set[0] != null && set[1] != null ? [set[0], set[1]] : null;
    vals.sort((a, b) => a - b);
    const lo = set[0] ?? vals[Math.floor(vals.length * 0.02)], hi = set[1] ?? vals[Math.floor(vals.length * 0.98)];
    return hi > lo ? [lo, hi] : [lo, lo + 1];
  }
  // the scale on the chart
  const leg = document.createElement("div");
  leg.className = "trk-legend";
  leg.hidden = true;
  map.getContainer().append(leg);
  function legend(by, range, count) {
    const note = $("trk-colour-note");
    leg.hidden = !by || !count;
    if (note) note.textContent = by && count && !range ? `The shown tracks have no ${by.name.toLowerCase()} logged.` : "";
    if (leg.hidden || !range) { leg.hidden = true; return; }
    const f = (v) => v.toFixed(by.dp);
    leg.innerHTML = `<b></b><div class="bar"></div><div class="ticks"><span></span><span></span><span></span></div>`;
    leg.querySelector("b").textContent = `${by.name}, ${by.unit}`;
    leg.querySelector(".bar").style.background = `linear-gradient(to right, ${by.ramp.join(", ")})`;
    const s = leg.querySelectorAll(".ticks span");
    s[0].textContent = f(range[0]); s[1].textContent = f((range[0] + range[1]) / 2); s[2].textContent = f(range[1]);
  }
  function syncColourControls() {
    $("trk-colour-by").value = prefs.colourBy;
    const by = BY[prefs.colourBy];
    $("trk-range").hidden = !by;
    const r = prefs.range[prefs.colourBy] || [];
    $("trk-min").value = r[0] ?? ""; $("trk-max").value = r[1] ?? "";
    if (by) { $("trk-unit").textContent = by.unit; }
  }

  /** Position of a track at time t (between fixes, straight line), or null outside its times. */
  function at(d, t) {
    for (const seg of d.segments) {
      if (!seg.length || seg[0][2] == null || t < seg[0][2] || t > seg.at(-1)[2]) continue;
      let lo = 0, hi = seg.length - 1;
      while (hi - lo > 1) { const m = (lo + hi) >> 1; if (seg[m][2] <= t) lo = m; else hi = m; }
      const a = seg[lo], b = seg[hi];
      const f = b[2] > a[2] ? (t - a[2]) / (b[2] - a[2]) : 0;
      return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
    }
    return null;
  }
  function drawNow() {
    const feats = [];
    if (prefs.atTime) {
      const t = getTime();
      for (const s of list.filter((x) => prefs.shown.includes(x.id))) {
        const d = cache.get(s.id);
        const p = d && at(d, t);
        if (p) feats.push({ type: "Feature", properties: { id: s.id, colour: colourOf(s) }, geometry: { type: "Point", coordinates: p } });
      }
    }
    map.getSource("trk-now").setData({ type: "FeatureCollection", features: feats });
  }
  onTime?.(drawNow);

  // click a track: time, SOG and COG at the nearest fix
  map.on("click", "trk-lines", async (e) => {
    if (document.querySelector(".armed")) return;
    const id = e.features[0].properties.id;
    const d = cache.get(id);
    const s = list.find((x) => x.id === id);
    if (!d || !s) return;
    let best = null, bd = Infinity;
    const c = e.lngLat;
    const k = Math.cos((c.lat * Math.PI) / 180);
    for (const seg of d.segments) for (const p of seg) {
      const dd = ((p[0] - c.lng) * k) ** 2 + (p[1] - c.lat) ** 2;
      if (dd < bd) { bd = dd; best = p; }
    }
    if (!best) return;
    const el = document.createElement("div");
    el.className = "feat-pop";
    const rows = [["Time", best[2] == null ? "–" : `${fmtTime(best[2])}:${String(new Date(best[2] * 1000).getSeconds()).padStart(2, "0")}`],
      ["SOG", best[3] == null ? "–" : `${best[3].toFixed(1)} kn`], ["COG", best[4] == null ? "–" : `${Math.round(best[4])}°T`]];
    if (best[5] != null) rows.push(["Depth", `${best[5].toFixed(1)} m`]);
    if (best[6] != null) rows.push(["Sea temperature", `${best[6].toFixed(1)} °C`]);
    el.innerHTML = `<table>${rows.map(() => "<tr><td></td><td></td></tr>").join("")}</table>`;
    el.querySelectorAll("tr").forEach((tr, i) => { tr.cells[0].textContent = rows[i][0]; tr.cells[1].textContent = rows[i][1]; });
    infoBox(map, "track", { width: 260, height: 170 }).open(el, { title: s.name, at: [best[0], best[1]] });
  });
  map.on("mouseenter", "trk-lines", () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "trk-lines", () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = ""; });

  // ---------- list ----------------------------------------------------------
  async function refresh() {
    try {
      [list, rec] = await Promise.all([invoke("tracks_list"), invoke("rec_status")]);
      list ||= []; rec ||= { on: false };
    } catch (e) { status(`Tracks: ${e}`); list = []; rec = { on: false }; }
    // the recording track may not be on disk yet: list it anyway
    if (rec.on && !list.some((t) => t.id === rec.id)) list.unshift({ id: rec.id, name: rec.name, notes: "", colour: "", source: "log", stats: rec.stats });
    prefs.shown = prefs.shown.filter((id) => list.some((t) => t.id === id));
    if (rec.on) cache.delete(rec.id); // reload the recording track as the recorder has it
    render();
    showRec();
    draw();
  }

  function render() {
    const ul = $("lib-tracks");
    ul.innerHTML = "";
    if (!list.length) {
      const li = document.createElement("li");
      li.className = "wx-note";
      li.textContent = "No tracks yet: press Record, or import a GPX file.";
      ul.append(li);
    }
    const q = $("trk-search").value.trim().toLowerCase();
    listed = q ? list.filter((t) => `${t.name} ${t.notes} ${fmtTime(t.stats?.start)}`.toLowerCase().includes(q)) : list;
    $("trk-delete").disabled = !listed.length;
    $("trk-bulk").hidden = true;
    if (q && !listed.length) {
      const li = document.createElement("li");
      li.className = "wx-note";
      li.textContent = "No tracks match.";
      ul.append(li);
    }
    // a page at a time: an import from a fishing plotter can bring thousands of tracks
    for (const t of listed.slice(0, pageEnd)) {
      const li = document.createElement("li");
      const st = t.stats || {};
      const recording = rec.on && rec.id === t.id;
      li.innerHTML = `<input type="checkbox" title="Show on the chart"><span class="swatch"></span>
        <div class="main" title="Zoom to this track"><b></b><small></small></div>
        <button data-act="gpx" title="Write this track as GPX">GPX</button><button data-act="edit" title="Name, colour, notes">✎</button><button data-act="del" title="Delete this track">✕</button>`;
      const cb = li.querySelector("input");
      cb.checked = prefs.shown.includes(t.id);
      cb.onchange = () => {
        prefs.shown = cb.checked ? [...prefs.shown, t.id] : prefs.shown.filter((x) => x !== t.id);
        save(); draw();
      };
      li.querySelector(".swatch").style.background = colourOf(t);
      li.querySelector("b").textContent = `${recording ? "● " : ""}${t.name}`;
      li.querySelector("small").textContent = [fmtTime(st.start), fmtDur(st.end - st.start), `${(st.distance_nm ?? 0).toFixed(1)} NM`,
        st.max_sog ? `max ${st.max_sog.toFixed(1)} kn` : "", t.source === "gpx" ? "GPX" : t.source === "timezero" ? "TimeZero" : recording ? "recording" : "log"].filter(Boolean).join(" · ");
      li.querySelector(".main").onclick = () => zoomTo(t);
      li.querySelector('[data-act="gpx"]').onclick = async () => {
        try { status(`Written to ${await invoke("track_export", { id: t.id })}`); } catch (e) { status(String(e)); }
      };
      li.querySelector('[data-act="edit"]').onclick = () => edit(t);
      li.querySelector('[data-act="del"]').onclick = () => {
        if (recording) { status("This track is being recorded: stop recording first."); return; }
        const keep = li.innerHTML;
        li.innerHTML = `<div class="main"><b>Delete ${t.name.replace(/[<&]/g, "")}?</b></div><button data-act="yes" class="bad">Delete</button><button data-act="no">Keep</button>`;
        li.querySelector('[data-act="no"]').onclick = () => { li.innerHTML = keep; render(); };
        li.querySelector('[data-act="yes"]').onclick = async () => {
          try { await invoke("track_delete", { id: t.id }); cache.delete(t.id); } catch (e) { status(String(e)); }
          refresh();
        };
      };
      ul.append(li);
    }
    if (listed.length > pageEnd) {
      const li = document.createElement("li");
      li.className = "more";
      li.innerHTML = `<button>Show ${Math.min(PAGE, listed.length - pageEnd)} more</button><small></small>`;
      li.querySelector("small").textContent = ` ${listed.length - pageEnd} more tracks, newest first.`;
      li.querySelector("button").onclick = () => { pageEnd += PAGE; render(); };
      ul.append(li);
    }
  }
  const PAGE = 100;
  let pageEnd = PAGE, listed = [];
  $("trk-search").oninput = () => { pageEnd = PAGE; render(); };
  $("trk-delete").onclick = () => {
    const ids = listed.filter((t) => !(rec.on && rec.id === t.id)).map((t) => t.id);
    if (!ids.length) return;
    const all = ids.length === list.length;
    $("trk-bulk").querySelector("span").textContent = `Delete ${all ? "all " : ""}${ids.length} track${ids.length === 1 ? "" : "s"}${all ? "" : " listed"}? This can't be undone.`;
    $("trk-bulk").hidden = false;
    $("trk-bulk-yes").onclick = async () => {
      $("trk-bulk-yes").disabled = true;
      try {
        const n = await invoke("tracks_delete", { ids });
        for (const id of ids) cache.delete(id);
        status(`Deleted ${n} track${n === 1 ? "" : "s"}.`);
      } catch (e) { status(String(e)); }
      $("trk-bulk-yes").disabled = false;
      await refresh();
    };
  };
  $("trk-bulk-no").onclick = () => { $("trk-bulk").hidden = true; };

  function zoomTo(t) {
    const b = t.stats?.bbox;
    if (!b) return;
    if (!prefs.shown.includes(t.id)) { prefs.shown.push(t.id); save(); render(); draw(); }
    map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: { top: 60, bottom: 80, left: 440, right: 80 }, duration: 600, maxZoom: 15 });
  }

  // ---------- edit ----------------------------------------------------------
  const form = $("trk-edit");
  let editing = null;
  function edit(t) {
    editing = t;
    form.name.value = t.name;
    form.colour.value = colourOf(t);
    form.notes.value = t.notes || "";
    form.hidden = false;
    form.name.focus();
  }
  form.onsubmit = async (e) => {
    e.preventDefault();
    if (!editing) return;
    try {
      await invoke("track_update", { id: editing.id, name: form.name.value.trim() || editing.name, notes: form.notes.value, colour: form.colour.value });
      cache.delete(editing.id);
    } catch (err) { status(String(err)); }
    form.hidden = true; editing = null;
    refresh();
  };
  form.querySelector('[data-act="cancel"]').onclick = () => { form.hidden = true; editing = null; };

  // ---------- recording ----------------------------------------------------------
  function showRec() {
    const b = $("trk-rec");
    b.textContent = rec.on ? "Stop" : "Record";
    b.classList.toggle("on", !!rec.on);
    $("trk-new").disabled = !rec.on;
    const st = rec.stats || {};
    $("trk-rec-info").textContent = rec.on
      ? `${rec.name}: ${st.points ?? 0} fixes, ${(st.distance_nm ?? 0).toFixed(1)} NM${st.points ? "" : " (waiting for Signal K)"}`
      : "Not recording.";
  }
  $("trk-rec").onclick = async () => {
    try { rec = rec.on ? await invoke("rec_stop") : await invoke("rec_start", { fresh: false }); } catch (e) { status(String(e)); }
    refresh();
  };
  $("trk-new").onclick = async () => {
    try { rec = await invoke("rec_start", { fresh: true }); } catch (e) { status(String(e)); }
    if (rec.on && !prefs.shown.includes(rec.id)) { prefs.shown.push(rec.id); save(); }
    refresh();
  };
  // each fix the recorder keeps: extend the drawn track and the totals
  let redraw = null;
  listen("rec-point", ({ payload: p }) => {
    if (!rec.on) return;
    rec.stats = p.stats;
    const d = cache.get(rec.id);
    if (d) {
      if (!d.segments.length) d.segments.push([]);
      d.segments.at(-1).push([p.lon, p.lat, p.t, null, null]);
    }
    const row = list.find((t) => t.id === rec.id);
    if (row) row.stats = p.stats;
    showRec();
    if (!redraw) redraw = setTimeout(() => { redraw = null; if (prefs.shown.includes(rec.id)) draw(); }, 2000);
  }).catch(() => {});

  // ---------- import and options ----------------------------------------------------------
  $("trk-import").onclick = () => document.getElementById("lib-import").click();
  window.addEventListener("racenav-tracks-imported", async () => {
    const before = new Set(list.map((t) => t.id));
    await refresh();
    // show what came in, unless it's a plotter's whole history (thousands of tracks)
    const fresh = list.filter((t) => !before.has(t.id));
    if (fresh.length <= 10) for (const t of fresh) if (!prefs.shown.includes(t.id)) prefs.shown.push(t.id);
    save(); render(); draw();
    if (fresh.length === 1) zoomTo(fresh[0]);
  });
  syncColourControls();
  $("trk-colour-by").onchange = (e) => { prefs.colourBy = e.target.value; syncColourControls(); save(); draw(); };
  const setRange = () => {
    const num = (v) => (v.trim() === "" || isNaN(+v) ? null : +v);
    prefs.range[prefs.colourBy] = [num($("trk-min").value), num($("trk-max").value)];
    save(); draw();
  };
  $("trk-min").onchange = setRange; $("trk-max").onchange = setRange;
  $("trk-time").checked = prefs.atTime;
  $("trk-time").onchange = () => { prefs.atTime = $("trk-time").checked; save(); drawNow(); };

  refresh();
  return { refresh };
}
