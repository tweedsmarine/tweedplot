// Chart › Chart data › Own soundings: the boat's own soundings gridded into a seabed,
// drawn in Rust as depth images (/pbathy/z/x/y.png).
// Built from the library's tracks with depth, and from TimeZero PBG and XYZ files.
// Not for navigation: depths as logged, not reduced to chart datum.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as activity from "./activity.js";

const $ = (id) => document.getElementById(id);
const SHOW = "racenav.pbathy.show";
const STYLE = "racenav.pbathy.style";
const SRC = { 1: "own", 2: "TimeZero", 4: "XYZ", 8: "other" };

let mapRef = null;
// the colours in use (set up below), for tracks coloured by depth with own soundings colours
let current = { palette: "rainbow", shallow: 0, deep: 500 };
/** Fill distances on the slider (m). */
const FILL_STEPS = [0, 2, 4, 8, 15, 25, 50, 100, 150, 250];
// the same stops as crates/bathy/src/render.rs (position 0–1 along the depth range, RGB)
const STOPS = {
  rainbow: [[0, [196, 32, 38]], [0.14, [236, 112, 36]], [0.28, [244, 206, 52]], [0.42, [120, 200, 80]], [0.56, [40, 186, 170]], [0.70, [40, 130, 216]], [0.84, [70, 62, 196]], [1, [92, 28, 128]]],
  blues: [[0, [205, 240, 236]], [0.2, [150, 210, 234]], [0.45, [96, 164, 220]], [0.7, [58, 110, 184]], [1, [26, 52, 120]]],
};
/** The own soundings colours as a scale over depth (m, positive down): { lo, hi, stops: [[depth, css], …] }. */
export function ownSoundingsScale() {
  const lo = +current.shallow || 0, hi = Math.max(lo + 1, +current.deep || lo + 1);
  const st = STOPS[current.palette] || STOPS.rainbow;
  return { lo, hi, stops: st.map(([t, c]) => [lo + t * (hi - lo), `rgb(${c.join(",")})`]) };
}
const visible = () => mapRef?.getLayer("pbathy") && mapRef.getLayoutProperty("pbathy", "visibility") === "visible";

// the sounding under the cursor, asked for at most every 150 ms: `show(text, sounding)`, the
// text for the position line (only with the layer shown), the sounding for the instruments
let atBusy = false, atNext = null;
const WHY = { 16: "spike", 32: "too shallow for the chart", 64: "too deep for the chart", 128: "in a run mostly off the chart" };
export function pbathyAt(lon, lat, show) {
  if (mapRef && mapRef.getZoom() < 9) { show("", null); return; }
  atNext = [lon, lat, show];
  if (atBusy) return;
  atBusy = true;
  const run = async () => {
    const [lo, la, cb] = atNext; atNext = null;
    try {
      const s = await invoke("pbathy_at", { lon: lo, lat: la });
      const parts = [];
      if (s && s.z < 0 && visible()) {
        const src = Object.entries(SRC).filter(([b]) => s.src & +b).map(([, n]) => n).join("+");
        const spread = s.n > 1 && s.spread >= 0.05 ? ` ±${s.spread.toFixed(1)}` : "";
        const when = s.newest ? `, ${new Date(s.newest * 1000).toLocaleDateString()}` : "";
        parts.push(`${(-s.z).toFixed(1)} m${spread} (${src}, ${s.n}×${when})`);
      }
      // with Show left out: a left-out sounding here, and why
      if (visible() && current.rejected) {
        const l = await invoke("pbathy_left_at", { lon: lo, lat: la }).catch(() => null);
        if (l) {
          const why = Object.entries(WHY).filter(([b]) => l.src & +b).map(([, n]) => n).join(", ") || "off the chart";
          const when = l.newest ? `, ${new Date(l.newest * 1000).toLocaleDateString()}` : "";
          parts.push(`left out: ${(-l.z).toFixed(1)} m, ${why}${when}`);
        }
      }
      cb(parts.join("  ·  "), s && s.z < 0 ? s : null);
    } catch { cb("", null); }
    setTimeout(() => { if (atNext) run(); else atBusy = false; }, 150);
  };
  run();
}

export function setupPbathy(map) {
  mapRef = map;
  let job = null;
  const fmt = (n) => Number(n).toLocaleString();
  const fmtMB = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.max(0.1, b / 1e6).toFixed(b >= 1e8 ? 0 : 1)} MB`);
  const note = (t, kind = "") => { const el = $("pb-note"); el.textContent = t; el.className = `grow wx-note ${kind}`; };

  let style = { palette: "rainbow", shallow: 0, deep: 500, contours: true, shade: true, fill: true, fill_m: 4, rejected: false };
  try { Object.assign(style, JSON.parse(localStorage.getItem(STYLE) || "{}")); } catch { /* defaults */ }
  current = style;
  window.dispatchEvent(new CustomEvent("own-soundings-colours"));

  const reload = () => {
    const src = map.getSource("pbathy");
    if (src?.setTiles) src.setTiles(src.tiles.map((t) => t.replace(/[?&]r=\d+/, "") + `${t.includes("?") ? "&" : "?"}r=${Date.now()}`));
  };
  const showForm = () => {
    $("pb-palette").value = style.palette;
    $("pb-shallow").value = Math.round(style.shallow);
    $("pb-deep").value = Math.round(style.deep);
    $("pb-contours").checked = style.contours;
    $("pb-shade").checked = style.shade;
    $("pb-fill").checked = style.fill;
    const k = FILL_STEPS.reduce((b, v, i) => (Math.abs(v - style.fill_m) < Math.abs(FILL_STEPS[b] - style.fill_m) ? i : b), 0);
    $("pb-fill-m").value = String(k);
    $("pb-fill-v").textContent = `${FILL_STEPS[k]} m`;
    $("pb-fill-m").disabled = !style.fill;
    $("pb-rejected").checked = !!style.rejected;
  };
  async function applyStyle() {
    current = style;
    try { localStorage.setItem(STYLE, JSON.stringify(style)); } catch { /* ignore */ }
    // tracks coloured with these colours follow
    window.dispatchEvent(new CustomEvent("own-soundings-colours"));
    try { await invoke("pbathy_style", { style }); reload(); } catch (e) { note(`Could not change the colours: ${e}`, "bad"); }
  }
  const readForm = () => {
    const sh = Math.max(0, +$("pb-shallow").value || 0);
    const dp = Math.max(sh + 1, +$("pb-deep").value || sh + 1);
    style = { palette: $("pb-palette").value, shallow: sh, deep: dp, contours: $("pb-contours").checked, shade: $("pb-shade").checked, fill: $("pb-fill").checked, fill_m: FILL_STEPS[+$("pb-fill-m").value] ?? 4, rejected: $("pb-rejected").checked };
    $("pb-fill-v").textContent = `${style.fill_m} m`;
    $("pb-fill-m").disabled = !style.fill;
    applyStyle();
  };
  for (const id of ["pb-palette", "pb-shallow", "pb-deep", "pb-contours", "pb-shade", "pb-fill", "pb-fill-m", "pb-rejected"]) $(id).onchange = readForm;
  // check soundings against the chart's depth areas (on unless turned off)
  const CHECK = "racenav.pbathy.check";
  $("pb-check").checked = localStorage.getItem(CHECK) !== "0";
  $("pb-check").onchange = (e) => { try { localStorage.setItem(CHECK, e.target.checked ? "1" : "0"); } catch { /* ignore */ } };
  const check = () => $("pb-check").checked;
  // the value while dragging; the tiles are drawn again on release
  $("pb-fill-m").oninput = () => { $("pb-fill-v").textContent = `${FILL_STEPS[+$("pb-fill-m").value]} m`; };

  const setShow = (on) => {
    $("pb-show").checked = on;
    try { localStorage.setItem(SHOW, on ? "1" : "0"); } catch { /* ignore */ }
    if (map.getLayer("pbathy")) map.setLayoutProperty("pbathy", "visibility", on ? "visible" : "none");
  };
  $("pb-show").checked = localStorage.getItem(SHOW) === "1";
  $("pb-show").onchange = (e) => setShow(e.target.checked);

  async function refresh() {
    let st = null;
    try { st = await invoke("pbathy_status"); } catch (e) { $("pb-state").textContent = String(e); return; }
    const i = st.info;
    $("pb-dot").className = `dot ${i.cells > 0 ? "ok" : ""}`;
    $("pb-state").textContent = i.cells > 0 ? `${fmt(i.cells)} cells (about 2 m), ${fmtMB(i.bytes)}${i.rejected ? `, ${fmt(i.rejected)} soundings left out` : ""}` : "empty";
    $("pb-zoom").hidden = !i.bounds;
    $("pb-del").hidden = !(i.cells > 0);
    const ul = $("pb-sources");
    ul.innerHTML = "";
    for (const s of i.sources) {
      const li = document.createElement("li");
      li.textContent = `${s.kind}: ${s.name} · ${fmt(s.soundings)} soundings${s.rejected ? ` (${fmt(s.rejected)} left out)` : ""} · ${new Date(s.added * 1000).toLocaleDateString()}`;
      ul.append(li);
    }
    // the area with nearly all of them (strays left out), and the layer turned on
    $("pb-zoom").onclick = () => {
      if (!i.bounds) return;
      if (!visible()) setShow(true);
      map.fitBounds([[i.bounds[0], i.bounds[1]], [i.bounds[2], i.bounds[3]]], { padding: 40, duration: 600 });
    };
  }

  const startJob = (title) => {
    job = activity.start(title, { cancel: () => invoke("pbathy_cancel").catch(() => {}) });
    for (const b of ["pb-import", "pb-tracks", "pb-del"]) $(b).disabled = true;
  };
  const endJob = () => {
    job = null;
    for (const b of ["pb-import", "pb-tracks", "pb-del"]) $(b).disabled = false;
    refresh();
    reload();
  };
  listen("pbathy-progress", (e) => {
    const p = e.payload || {};
    job?.progress(p.stage, p.done, p.total);
    note(p.total ? `${p.stage}: ${fmt(p.done)} of ${fmt(p.total)}` : `${p.stage}…`);
  });

  $("pb-import").onclick = async () => {
    const path = await invoke("pbathy_pick_file").catch(() => null);
    if (!path) return;
    const name = path.split(/[\\/]/).pop();
    const run = async (again) => {
      startJob(`Import soundings: ${name}`);
      try {
        const r = await invoke("pbathy_import", { path, again, check: check() });
        if (r.already) {
          const t = `${name} was imported before: not added again (Import again adds it twice).`;
          job.done(t); note(t, "warn");
          endJob();
          const b = $("pb-again");
          b.hidden = false;
          b.onclick = () => { b.hidden = true; run(true); };
          setTimeout(() => { b.hidden = true; }, 15000);
          return;
        }
        const t = `${name}: ${fmt(r.added)} soundings added in ${Math.round(r.seconds)} s.`
          + (r.checked ? ` ${fmt(r.rejected)} well off the charted depth left out${r.back ? `; ${fmt(r.back)} left-out soundings put back where the soundings beside them agree` : ""}.` : check() ? " Not checked: no chart depth areas are open." : "");
        job.done(t); note(t, "good");
        if (!visible()) setShow(true);
      } catch (e) {
        const c = String(e) === "cancelled";
        job.fail(c ? "Cancelled: nothing was added." : `Import failed: ${e}`); note(c ? "Cancelled." : `Import failed: ${e}`, "bad");
      }
      endJob();
    };
    run(false);
  };

  $("pb-tracks").onclick = async () => {
    startJob("Own soundings from my tracks");
    try {
      const r = await invoke("pbathy_from_tracks", { check: check() });
      let t;
      if (!r.with_depth) t = "No tracks in the library have depth logged.";
      else if (!r.tracks) t = `All ${fmt(r.with_depth)} tracks with depth are in already.${r.back ? ` ${fmt(r.back)} left-out soundings were put back on a second look.` : ""}`;
      else {
        const c = r.counts || {};
        t = `${fmt(r.tracks)} tracks: ${fmt(r.kept)} soundings added. Left out: ${fmt(c.spikes ?? r.dropped)} spikes`;
        t += r.checked ? `, ${fmt(c.off_chart)} well off the charted depth, and ${fmt(c.in_runs)} more in ${fmt(c.runs)} runs mostly off it (the sounder locked onto something else).`
          + (r.back ? ` ${fmt(r.back)} of those left out were put back: the soundings beside them agree (the chart's bands were too coarse there).` : "")
          + (c.unchecked ? ` ${fmt(c.unchecked)} had no chart depth to check against.` : "")
          : check() ? ". Not checked against the chart: no chart depth areas are open." : ".";
      }
      if (r.changed) t += ` ${r.changed} track${r.changed === 1 ? " has" : "s have"} changed since being added (not added again; delete all and rebuild to take the changes).`;
      job.done(t, { keep: !!r.changed }); note(t, r.tracks ? "good" : "");
      if (r.tracks && !visible()) setShow(true);
    } catch (e) {
      const c = String(e) === "cancelled";
      job.fail(c ? "Cancelled: nothing was added." : `Failed: ${e}`); note(c ? "Cancelled." : `Failed: ${e}`, "bad");
    }
    endJob();
  };

  $("pb-fit").onclick = async () => {
    const b = map.getBounds();
    try {
      const r = await invoke("pbathy_fit", { area: [b.getWest(), Math.max(-85, b.getSouth()), b.getEast(), Math.min(85, b.getNorth())] });
      if (!r) { note("No soundings in the chart view to fit the colours to."); return; }
      style.shallow = Math.floor(r[0]); style.deep = Math.ceil(r[1]);
      showForm(); applyStyle();
      note(`Colours: ${style.shallow} m to ${style.deep} m (the depths in view).`);
    } catch (e) { note(`Could not fit: ${e}`, "bad"); }
  };

  // two clicks: the first asks
  let delArmed = null;
  $("pb-del").onclick = async () => {
    const b = $("pb-del");
    if (!delArmed) {
      b.textContent = "Delete all? Click again";
      note("This deletes all the own soundings (from your tracks and the imported files). The tracks and the files stay; it can be built again.", "warn");
      delArmed = setTimeout(() => { delArmed = null; b.textContent = "✕ All"; }, 5000);
      return;
    }
    clearTimeout(delArmed); delArmed = null; b.textContent = "✕ All";
    try { await invoke("pbathy_clear"); note("Own soundings deleted."); } catch (e) { note(`Could not delete: ${e}`, "bad"); }
    refresh(); reload();
  };

  showForm();
  invoke("pbathy_style", { style }).catch(() => {}).finally(() => { reload(); refresh(); });
}
