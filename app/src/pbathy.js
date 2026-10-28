// Chart › Chart data › Personal bathymetry: the boat's own soundings gridded into a
// seabed (as Olex and TimeZero PBG), drawn in Rust as depth images (/pbathy/z/x/y.png).
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
const visible = () => mapRef?.getLayer("pbathy") && mapRef.getLayoutProperty("pbathy", "visibility") === "visible";

// the sounding under the cursor, asked for at most every 150 ms
let atBusy = false, atNext = null;
export function pbathyAt(lon, lat, show) {
  if (!visible() || (mapRef && mapRef.getZoom() < 9)) { show(""); return; }
  atNext = [lon, lat, show];
  if (atBusy) return;
  atBusy = true;
  const run = async () => {
    const [lo, la, cb] = atNext; atNext = null;
    try {
      const s = await invoke("pbathy_at", { lon: lo, lat: la });
      if (!s || s.z >= 0) cb("");
      else {
        const src = Object.entries(SRC).filter(([b]) => s.src & +b).map(([, n]) => n).join("+");
        const spread = s.n > 1 && s.spread >= 0.05 ? ` ±${s.spread.toFixed(1)}` : "";
        const when = s.newest ? `, ${new Date(s.newest * 1000).toLocaleDateString()}` : "";
        cb(`${(-s.z).toFixed(1)} m${spread} (${src}, ${s.n}×${when})`);
      }
    } catch { cb(""); }
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

  let style = { palette: "rainbow", shallow: 0, deep: 500, contours: true, shade: true, fill: true };
  try { Object.assign(style, JSON.parse(localStorage.getItem(STYLE) || "{}")); } catch { /* defaults */ }

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
  };
  async function applyStyle() {
    try { localStorage.setItem(STYLE, JSON.stringify(style)); } catch { /* ignore */ }
    try { await invoke("pbathy_style", { style }); reload(); } catch (e) { note(`Could not change the colours: ${e}`, "bad"); }
  }
  const readForm = () => {
    const sh = Math.max(0, +$("pb-shallow").value || 0);
    const dp = Math.max(sh + 1, +$("pb-deep").value || sh + 1);
    style = { palette: $("pb-palette").value, shallow: sh, deep: dp, contours: $("pb-contours").checked, shade: $("pb-shade").checked, fill: $("pb-fill").checked };
    applyStyle();
  };
  for (const id of ["pb-palette", "pb-shallow", "pb-deep", "pb-contours", "pb-shade", "pb-fill"]) $(id).onchange = readForm;

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
    $("pb-state").textContent = i.cells > 0 ? `${fmt(i.cells)} cells (about 2 m), ${fmtMB(i.bytes)}` : "empty";
    $("pb-zoom").hidden = !i.bounds;
    $("pb-del").hidden = !(i.cells > 0);
    const ul = $("pb-sources");
    ul.innerHTML = "";
    for (const s of i.sources) {
      const li = document.createElement("li");
      li.textContent = `${s.kind}: ${s.name} · ${fmt(s.soundings)} soundings · ${new Date(s.added * 1000).toLocaleDateString()}`;
      ul.append(li);
    }
    $("pb-zoom").onclick = () => i.bounds && map.fitBounds([[i.bounds[0], i.bounds[1]], [i.bounds[2], i.bounds[3]]], { padding: 40, duration: 600 });
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
        const r = await invoke("pbathy_import", { path, again });
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
        const t = `${name}: ${fmt(r.added)} soundings added in ${Math.round(r.seconds)} s.`;
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
    startJob("Personal bathymetry from my tracks");
    try {
      const r = await invoke("pbathy_from_tracks");
      let t;
      if (!r.with_depth) t = "No tracks in the library have depth logged.";
      else if (!r.tracks) t = `All ${fmt(r.with_depth)} tracks with depth are in already.`;
      else t = `${fmt(r.tracks)} tracks: ${fmt(r.kept)} soundings added, ${fmt(r.dropped)} spikes left out.`;
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
      note("This deletes all the personal bathymetry (own soundings and imported files). The tracks and the files stay; it can be built again.", "warn");
      delArmed = setTimeout(() => { delArmed = null; b.textContent = "✕ All"; }, 5000);
      return;
    }
    clearTimeout(delArmed); delArmed = null; b.textContent = "✕ All";
    try { await invoke("pbathy_clear"); note("Personal bathymetry deleted."); } catch (e) { note(`Could not delete: ${e}`, "bad"); }
    refresh(); reload();
  };

  showForm();
  invoke("pbathy_style", { style }).catch(() => {}).finally(() => { reload(); refresh(); });
}
