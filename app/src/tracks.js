// Library > Tracks: tracks recorded from Signal K and tracks imported from GPX.
// Shown on the chart in their own colours; optionally a dot where each track's
// boat was at the weather time (replays and comparisons); click a track for the
// time, SOG and COG there. The recorder itself runs in Rust (it keeps logging
// while plotting is paused); this module starts and stops it and shows its progress.
import { infoBox } from "./infobox.js";
import { seaScale } from "./weather.js";
import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
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
  const prefs = { shown: [], atTime: true, colourBy: "track", filter: {} };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  prefs.filter ||= {};
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
    range = by ? rangeOf(by, shown) : null;
    const f = range && filterOf(range);
    for (const [t, d] of shown) {
      const colour = colourOf(t);
      for (const seg of d.segments) {
        if (seg.length < 2) continue;
        if (!f) { feats.push({ type: "Feature", properties: { id: t.id, colour }, geometry: { type: "LineString", coordinates: seg.map((p) => [p[0], p[1]]) } }); continue; }
        // runs of fixes in the same colour step: one line each, not one per fix. Each run is
        // "in" the band (coloured), "out" of it (thin grey) or without a value (grey).
        const step = Math.max(range.q, (f.hi - f.lo) / 200);
        const key = (p) => {
          const v = p[by.i];
          if (v == null) return "none";
          if (v < f.lo || v > f.hi) return "out";
          return Math.round((v - f.lo) / step);
        };
        let start = 0, k0 = key(seg[0]);
        for (let k = 1; k <= seg.length; k++) {
          const kk = k < seg.length ? key(seg[k]) : undefined;
          if (kk !== k0 || k === seg.length) {
            const run = seg.slice(start, Math.min(k + 1, seg.length)).map((p) => [p[0], p[1]]);
            if (run.length > 1) {
              const props = { id: t.id };
              if (typeof k0 === "number") {
                props.v = Math.min(f.hi, f.lo + k0 * step);
                if (Math.abs(props.v - f.tg) <= step / 2 + 1e-9) props.tg = 1;
              } else props.x = k0;
              feats.push({ type: "Feature", properties: props, geometry: { type: "LineString", coordinates: run } });
            }
            start = k; k0 = kk;
          }
        }
      }
    }
    if (f) {
      const sc = seaScale(f);
      map.setPaintProperty("trk-lines", "line-color", ["case", ["has", "v"], ["interpolate", ["linear"], ["get", "v"], ...sc.flat()], NO_VALUE]);
      map.setPaintProperty("trk-lines", "line-width", ["case", ["has", "tg"], 5, ["has", "v"], 2.6, 1.2]);
      map.setPaintProperty("trk-lines", "line-opacity", ["case", ["has", "v"], 0.95, 0.6]);
    } else {
      map.setPaintProperty("trk-lines", "line-color", ["get", "colour"]);
      map.setPaintProperty("trk-lines", "line-width", 2.2);
      map.setPaintProperty("trk-lines", "line-opacity", 0.9);
    }
    map.getSource("trk-lines").setData({ type: "FeatureCollection", features: feats });
    drawFilter(by, shown.length);
    drawNow();
  }

  // ---------- colour by sea temperature, depth or speed ----------------------------------
  // The same filter as the GRIB sea layers (Weather drawer): the outer pointers set the
  // lowest and highest coloured, the middle one the target (drawn bold); the same colours.
  const NO_VALUE = "#9aa3a8";
  // fix layout from track_get: [lon, lat, t, sog, cog, depth, temp]
  const BY = {
    temp: { i: 6, name: "Sea temperature", unit: "°C" },
    depth: { i: 5, name: "Depth", unit: "m" },
    sog: { i: 3, name: "Speed (SOG)", unit: "kn" },
  };
  let range = null; // { min, max, q } of the shown tracks' values
  const quantum = (lo, hi) => 10 ** Math.floor(Math.log10(Math.max(hi - lo, 0.01) / 20));
  const roundTo = (v, q) => +(Math.round(v / q) * q).toFixed(6);
  /** The span of the shown tracks' values (without the odd wild one). */
  function rangeOf(by, shown) {
    const vals = [];
    let n = 0;
    for (const [, d] of shown) for (const seg of d.segments) n += seg.length;
    const every = Math.max(1, Math.floor(n / 100000));
    let k = 0;
    for (const [, d] of shown) for (const seg of d.segments) for (const p of seg) if (k++ % every === 0 && p[by.i] != null) vals.push(p[by.i]);
    if (!vals.length) return null;
    vals.sort((a, b) => a - b);
    let min = vals[Math.floor(vals.length * 0.005)], max = vals[Math.min(vals.length - 1, Math.floor(vals.length * 0.995))];
    if (!(max > min)) max = min + 1;
    const q = quantum(min, max);
    return { min: roundTo(Math.floor(min / q) * q, q), max: roundTo(Math.ceil(max / q) * q, q), q };
  }
  /** The filter for the shown tracks, kept within their range (whole range at first). */
  function filterOf(r) {
    let f = prefs.filter[prefs.colourBy];
    if (!f || f.hi <= r.min || f.lo >= r.max) f = { lo: r.min, hi: r.max, tg: roundTo((r.min + r.max) / 2, r.q) };
    const lo = Math.max(r.min, Math.min(f.lo, r.max - r.q)), hi = Math.min(r.max, Math.max(f.hi, lo + r.q));
    f = { lo, hi, tg: Math.min(hi, Math.max(lo, f.tg)) };
    prefs.filter[prefs.colourBy] = f;
    return f;
  }
  // the scale on the chart, for when the drawer is closed
  const leg = document.createElement("div");
  leg.className = "trk-legend";
  leg.hidden = true;
  map.getContainer().append(leg);
  function drawFilter(by, count) {
    const box = $("trk-sea"), note = $("trk-colour-note");
    note.textContent = by && count && !range ? `The shown tracks have no ${by.name.toLowerCase()} logged.` : by && !count ? "Tick tracks to show them." : "";
    const f = range && prefs.filter[prefs.colourBy];
    box.hidden = leg.hidden = !f;
    if (!f) return;
    const r = range, span = Math.max(r.max - r.min, 1e-6);
    const pc = (v) => `${Math.min(100, Math.max(0, ((v - r.min) / span) * 100))}%`;
    const sc = seaScale(f), w = Math.max(f.hi - f.lo, 1e-6);
    const css = `linear-gradient(to right, ${sc.map(([v, c]) => `${c} ${((v - f.lo) / w) * 100}%`).join(", ")}, ${sc.at(-1)[1]} 100%)`;
    const grad = box.querySelector(".grad");
    grad.style.left = pc(f.lo);
    grad.style.width = `calc(${pc(f.hi)} - ${pc(f.lo)})`;
    grad.style.background = css;
    for (const k of ["lo", "tg", "hi"]) box.querySelector(`.h.${k}`).style.left = pc(f[k]);
    const dp = Math.max(0, -Math.round(Math.log10(r.q)));
    const fmt = (v) => v.toFixed(dp);
    box.querySelector(".v.lo").textContent = fmt(f.lo);
    box.querySelector(".v.tg").textContent = `target ${fmt(f.tg)}`;
    box.querySelector(".v.hi").textContent = fmt(f.hi);
    box.querySelector(".v.min").textContent = fmt(r.min);
    box.querySelector(".v.max").textContent = `${fmt(r.max)} ${by.unit}`;
    box.title = `${by.name} (${by.unit}): drag the outer pointers to set the lowest and highest coloured, the middle one to the target (drawn bold). Fit: back to the shown tracks' whole range.`;
    leg.innerHTML = `<b></b><div class="bar"></div><div class="ticks"><span></span><span class="tg"></span><span></span></div>`;
    leg.querySelector("b").textContent = `${by.name}, ${by.unit}`;
    leg.querySelector(".bar").style.background = css;
    const s = leg.querySelectorAll(".ticks span");
    s[0].textContent = fmt(f.lo); s[1].textContent = `target ${fmt(f.tg)}`; s[2].textContent = fmt(f.hi);
  }
  // dragging the pointers (as the GRIB sea filter)
  (() => {
    const box = $("trk-sea"), track = box.querySelector(".track");
    let drag = null;
    const valueAt = (e) => range.min + ((e.clientX - drag.rect.left) / drag.rect.width) * (range.max - range.min);
    track.addEventListener("pointerdown", (e) => {
      const f = prefs.filter[prefs.colourBy];
      if (!f || !range) return;
      drag = { rect: track.getBoundingClientRect() };
      const v = valueAt(e);
      drag.k = e.target.classList.contains("h") ? [...e.target.classList].find((c) => ["lo", "tg", "hi"].includes(c))
        : ["tg", "lo", "hi"].reduce((a, b) => (Math.abs(f[b] - v) < Math.abs(f[a] - v) ? b : a));
      track.setPointerCapture(e.pointerId);
      move(e);
    });
    let pending = null;
    const move = (e) => {
      if (!drag) return;
      const f = prefs.filter[prefs.colourBy], q = range.q;
      const v = Math.min(range.max, Math.max(range.min, roundTo(valueAt(e), q)));
      if (drag.k === "lo") f.lo = Math.min(v, f.tg);
      else if (drag.k === "hi") f.hi = Math.max(v, f.tg);
      else f.tg = Math.min(Math.max(v, f.lo), f.hi);
      drawFilter(BY[prefs.colourBy], 1);
      if (!pending) pending = requestAnimationFrame(() => { pending = null; draw(); });
    };
    track.addEventListener("pointermove", move);
    track.addEventListener("pointerup", () => { if (drag) { drag = null; save(); draw(); } });
    box.querySelector(".fit").onclick = () => { delete prefs.filter[prefs.colourBy]; save(); draw(); };
  })();
  function syncColourControls() {
    $("trk-colour-by").value = prefs.colourBy;
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
        activity.note(`Deleted ${n.toLocaleString()} track${n === 1 ? "" : "s"}`);
      } catch (e) { activity.start("Deleting tracks").fail(String(e)); }
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
  $("trk-time").checked = prefs.atTime;
  $("trk-time").onchange = () => { prefs.atTime = $("trk-time").checked; save(); drawNow(); };

  refresh();
  return { refresh };
}
