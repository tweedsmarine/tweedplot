// Chart › Chart data › Own soundings: the boat's own soundings gridded into a seabed,
// drawn in Rust as depth images (/pbathy/z/x/y.png).
// Built from the library's tracks with depth, and from TimeZero PBG and XYZ files.
// Not for navigation. Depths reduced to chart datum (pbathy.rs, Datum): the tracks' depth reference and LINZ tides.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as activity from "./activity.js";
import { floatDrag } from "./floatdrag.js";
import { askDepthRef, setDepthRef } from "./depthref.js";

const $ = (id) => document.getElementById(id);
const SHOW = "racenav.pbathy.show"; // before 30 Oct 2026: one setting for every view
const VIEWS_KEY = "racenav.pbathy.views";
/** Shown in a view to begin with: on when planning, off when navigating (the chart's own depths come first). */
const VIEW_DEFAULT = { nav: false, plan: true, inst: false };
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
  mono: [[0, [20, 20, 20]], [1, [235, 235, 235]]],
  bands: [[0, [20, 20, 20]], [0.1666, [20, 20, 20]], [0.1667, [65, 65, 65]], [0.3333, [65, 65, 65]], [0.3334, [110, 110, 110]], [0.5, [110, 110, 110]], [0.5001, [150, 150, 150]], [0.6666, [150, 150, 150]], [0.6667, [190, 190, 190]], [0.8333, [190, 190, 190]], [0.8334, [235, 235, 235]], [1, [235, 235, 235]]],
};
const NIGHT_PALETTES = ["mono", "bands"];
const PALETTE_OPTIONS = `<option value="rainbow">Rainbow</option><option value="blues">Blues</option><option value="mono">Night mono</option><option value="bands">Night bands</option>`;
const PALETTE_HELP = "Rainbow: red shallow to violet deep. Blues: as charts. Night mono and Night bands: grey ramps for the night colours (shallow brightest). Tracks coloured by Depth (own soundings colours) use these colours too";
/** The target depth in use, if it lies between the shallow and deep limits. */
const targetOf = (s) => (s.target != null && s.target > s.shallow && s.target < s.deep ? +s.target : null);
/** The own soundings colours as a scale over depth (m, positive down): { lo, hi, tg, stops: [[depth, css], …] }.
 * With a target, lo..target takes the first half of the palette and target..hi the second (as render.rs). */
export function ownSoundingsScale() {
  const lo = +current.shallow || 0, hi = Math.max(lo + 1, +current.deep || lo + 1);
  const tg = targetOf({ ...current, shallow: lo, deep: hi });
  const base = STOPS[current.palette] || STOPS.rainbow;
  const st = current.reverse ? base.map(([t, c]) => [1 - t, c]).reverse() : base;
  const at = (t) => (tg == null ? lo + t * (hi - lo) : t <= 0.5 ? lo + (t / 0.5) * (tg - lo) : tg + ((t - 0.5) / 0.5) * (hi - tg));
  // with a target the halves need a stop at 0.5 exactly
  const pts = tg == null ? st : [...st.filter(([t]) => t < 0.5), [0.5, colourAt(st, 0.5)], ...st.filter(([t]) => t > 0.5)];
  return { lo, hi, tg, stops: pts.map(([t, c]) => [at(t), `rgb(${c.map(Math.round).join(",")})`]) };
}
function colourAt(st, t) {
  for (let i = 1; i < st.length; i++) {
    if (t <= st[i][0]) {
      const f = (t - st[i - 1][0]) / (st[i][0] - st[i - 1][0]);
      return st[i][1].map((v, k) => st[i - 1][1][k] + (v - st[i - 1][1][k]) * f);
    }
  }
  return st.at(-1)[1];
}
const visible = () => mapRef?.getLayer("pbathy") && mapRef.getLayoutProperty("pbathy", "visibility") === "visible";

// the sounding under the cursor, asked for at most every 150 ms: `show(text, sounding)`, the
// text for the position line (only with the layer shown), the sounding for the instruments
let atBusy = false, atNext = null;
/** How near the cursor (screen pixels) a sounding is picked up. */
const SNAP_PX = 8;
const WHY = { 16: "spike", 32: "too shallow for the chart", 64: "too deep for the chart", 128: "in a run mostly off the chart" };
export function pbathyAt(lon, lat, show) {
  if (mapRef && mapRef.getZoom() < 7) { show("", null); return; }
  atNext = [lon, lat, show];
  if (atBusy) return;
  atBusy = true;
  const run = async () => {
    const [lo, la, cb] = atNext; atNext = null;
    try {
      // snap: the nearest sounding within SNAP_PX pixels at the zoom shown
      const mpp = 40075016.686 * Math.cos(la * Math.PI / 180) / (512 * 2 ** (mapRef ? mapRef.getZoom() : 12));
      const radiusM = Math.max(2, SNAP_PX * mpp);
      const s = await invoke("pbathy_at", { lon: lo, lat: la, radiusM });
      const parts = [];
      if (s && s.z < 0 && visible()) {
        const src = Object.entries(SRC).filter(([b]) => s.src & +b).map(([, n]) => n).join("+");
        const spread = s.n > 1 && s.spread >= 0.05 ? ` ±${s.spread.toFixed(1)}` : "";
        const when = s.newest ? `, ${new Date(s.newest * 1000).toLocaleDateString()}` : "";
        const away = s.off_m >= 3 ? `, ${Math.round(s.off_m)} m away` : "";
        parts.push(`${(-s.z).toFixed(1)} m${spread} (${src}, ${s.n}×${when}${away})`);
      }
      // with Show left out: a left-out sounding here, and why
      if (visible() && current.rejected) {
        const l = await invoke("pbathy_left_at", { lon: lo, lat: la, radiusM }).catch(() => null);
        if (l) {
          const why = (l.src & 80) === 80 ? "second echo (about twice the depth)" : Object.entries(WHY).filter(([b]) => l.src & +b).map(([, n]) => n).join(", ") || "off the chart";
          const when = l.newest ? `, ${new Date(l.newest * 1000).toLocaleDateString()}` : "";
          const away = l.off_m >= 3 ? `, ${Math.round(l.off_m)} m away` : "";
          parts.push(`left out: ${(-l.z).toFixed(1)} m, ${why}${when}${away}`);
        }
      }
      cb(parts.join("  ·  "), s && s.z < 0 ? s : null);
    } catch { cb("", null); }
    setTimeout(() => { if (atNext) run(); else atBusy = false; }, 150);
  };
  run();
}

export function setupPbathy(map, { ui } = {}) {
  mapRef = map;
  let job = null;
  const fmt = (n) => Number(n).toLocaleString();
  const fmtMB = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.max(0.1, b / 1e6).toFixed(b >= 1e8 ? 0 : 1)} MB`);
  const note = (t, kind = "") => { const el = $("pb-note"); el.textContent = t; el.className = `grow wx-note ${kind}`; };

  let style = { palette: "rainbow", shallow: 0, deep: 500, target: null, contours: true, shade: true, fill: true, fill_m: 4, rejected: false, reverse: false };
  try { Object.assign(style, JSON.parse(localStorage.getItem(STYLE) || "{}")); } catch { /* defaults */ }
  current = style;
  window.dispatchEvent(new CustomEvent("own-soundings-colours"));

  const reload = () => {
    const src = map.getSource("pbathy");
    if (src?.setTiles) src.setTiles(src.tiles.map((t) => t.replace(/[?&]r=\d+/, "") + `${t.includes("?") ? "&" : "?"}r=${Date.now()}`));
  };
  const showForm = () => {
    $("pb-palette").value = style.palette;
    $("pb-reverse").checked = !!style.reverse;
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
    $("pb-target").value = style.target == null ? "" : Math.round(style.target);
    card.draw();
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
    const tv = $("pb-target").value.trim();
    const tg = tv === "" ? null : Math.min(dp - 0.5, Math.max(sh + 0.5, +tv));
    style = { ...style, palette: $("pb-palette").value, reverse: $("pb-reverse").checked, shallow: sh, deep: dp, target: tg, contours: $("pb-contours").checked, shade: $("pb-shade").checked, fill: $("pb-fill").checked, fill_m: FILL_STEPS[+$("pb-fill-m").value] ?? 4, rejected: $("pb-rejected").checked };
    card.draw();
    $("pb-fill-v").textContent = `${style.fill_m} m`;
    $("pb-fill-m").disabled = !style.fill;
    applyStyle();
  };
  for (const id of ["pb-palette", "pb-reverse", "pb-shallow", "pb-deep", "pb-target", "pb-contours", "pb-shade", "pb-fill", "pb-fill-m", "pb-rejected"]) $(id).onchange = readForm;
  // check soundings against the chart's depth areas (on unless turned off)
  const CHECK = "racenav.pbathy.check";
  $("pb-check").checked = localStorage.getItem(CHECK) !== "0";
  $("pb-check").onchange = (e) => { try { localStorage.setItem(CHECK, e.target.checked ? "1" : "0"); } catch { /* ignore */ } };
  const check = () => $("pb-check").checked;
  // the value while dragging; the tiles are drawn again on release
  $("pb-fill-m").oninput = () => { $("pb-fill-v").textContent = `${FILL_STEPS[+$("pb-fill-m").value]} m`; };

  // shown or not per view (Navigation, Planning …): the checkbox, the toolbar button and the card
  let perView = {};
  try { perView = JSON.parse(localStorage.getItem(VIEWS_KEY) || "{}"); } catch { /* defaults */ }
  if (!Object.keys(perView).length && localStorage.getItem(SHOW) === "1") perView.plan = true; // the old single setting
  const viewNow = () => document.documentElement.dataset.view || "plan";
  const shownIn = (v) => perView[v] ?? VIEW_DEFAULT[v] ?? false;
  let hasData = false; // the depth card is only useful once there are soundings to colour
  const applyShow = () => {
    const on = shownIn(viewNow());
    $("pb-show").checked = on;
    $("pb-tb")?.classList.toggle("on", on);
    if (map.getLayer("pbathy")) map.setLayoutProperty("pbathy", "visibility", on ? "visible" : "none");
    card.show(on && hasData);
  };
  const setShow = (on) => {
    perView[viewNow()] = on;
    try { localStorage.setItem(VIEWS_KEY, JSON.stringify(perView)); } catch { /* ignore */ }
    applyShow();
  };
  $("pb-show").onchange = (e) => setShow(e.target.checked);
  $("pb-tb").onclick = () => setShow(!shownIn(viewNow()));
  window.addEventListener("racenav-view", applyShow);

  async function refresh() {
    let st = null;
    try { st = await invoke("pbathy_status"); } catch (e) { $("pb-state").textContent = String(e); return; }
    const i = st.info;
    hasData = i.cells > 0;
    applyShow();
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
    // tracks whose depths don't say what they're measured below: ask first
    const unknown = await invoke("tracks_depth_unknown", { newestImport: false }).catch(() => []);
    if (unknown.length) {
      const r = await askDepthRef({
        title: "Depths in your tracks",
        note: `${fmt(unknown.length)} track${unknown.length === 1 ? " has" : "s have"} depths that don't say what they are measured from (recorded before TweedPlot noted it, or imported from a file that doesn't say).`,
      });
      if (!r) return;
      await setDepthRef(unknown, r).catch((e) => note(String(e), "bad"));
    }
    startJob("Own soundings from my tracks");
    try {
      const r = await invoke("pbathy_from_tracks", { check: check() });
      let t;
      if (!r.with_depth) t = "No tracks in the library have depth logged.";
      else if (!r.tracks) t = `All ${fmt(r.with_depth)} tracks with depth are in already.${r.back ? ` ${fmt(r.back)} left-out soundings were put back on a second look.` : ""}`;
      else {
        const c = r.counts || {};
        t = `${fmt(r.tracks)} tracks: ${fmt(r.kept)} soundings added. Left out: ${fmt(c.spikes ?? r.dropped)} spikes${c.echoes ? `, ${fmt(c.echoes)} second echoes` : ""}`;
        t += r.checked ? `, ${fmt(c.off_chart)} well off the charted depth, and ${fmt(c.in_runs)} more in ${fmt(c.runs)} runs mostly off it (the sounder locked onto something else).`
          + (r.back ? ` ${fmt(r.back)} of those left out were put back: the soundings beside them agree (the chart's bands were too coarse there).` : "")
          + (c.unchecked ? ` ${fmt(c.unchecked)} had no chart depth to check against.` : "")
          : check() ? ". Not checked against the chart: no chart depth areas are open." : ".";
      }
      if (r.tracks) {
        // every sounding is one of: already at chart datum (TimeZero's), reduced with the tide, or kept below the surface
        const parts = [
          r.already && `${fmt(r.already)} already at chart datum (TimeZero), added as they were`,
          r.reduced && `${fmt(r.reduced)} reduced to chart datum with the tide`,
          r.no_tide && `${fmt(r.no_tide)} kept below the surface: ${r.tides ? "no LINZ port within 30 km, or no tide data for the time" : "no tide data (Weather and tides › Tides › Download)"}`,
        ].filter(Boolean);
        if (parts.length) t += ` Depths: ${parts.join("; ")}.`;
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

  async function fitToView() {
    const b = map.getBounds();
    try {
      const r = await invoke("pbathy_fit", { area: [b.getWest(), Math.max(-85, b.getSouth()), b.getEast(), Math.min(85, b.getNorth())] });
      if (!r) { note("No soundings in the chart view to fit the colours to."); return; }
      style.shallow = Math.floor(r[0]); style.deep = Math.ceil(r[1]);
      if (style.target != null && (style.target <= style.shallow || style.target >= style.deep)) style.target = null;
      card.fitScale();
      showForm(); applyStyle();
      note(`Colours: ${style.shallow} m to ${style.deep} m (the depths in view).`);
    } catch (e) { note(`Could not fit: ${e}`, "bad"); }
  }
  $("pb-fit").onclick = fitToView;

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

  // ---------- the card over the chart: the depth range with three pointers ----------
  // shallow and deep set the colour range, the middle (red) pointer the target depth: the
  // colours split there and it gets a bold contour. Shown with the layer, in any view.
  const card = makeCard({
    get: () => style,
    set: (s) => { style = { ...style, ...s }; showForm(); applyStyle(); },
    preview: (s) => { current = { ...style, ...s }; window.dispatchEvent(new CustomEvent("own-soundings-colours")); },
    fit: () => fitToView(),
    hide: () => setShow(false),
    settings: () => {
      // the drawer may be hidden in this view (Navigation): bring the panels back first
      window.dispatchEvent(new Event("racenav-need-tools"));
      ui?.open("chart");
      requestAnimationFrame(() => $("pb-dot")?.scrollIntoView({ block: "start", behavior: "smooth" }));
    },
  });
  applyShow();

  // Night colours switch to a night palette; going back to day or dusk restores the one in use before.
  const syncNight = () => {
    const night = (document.documentElement.dataset.palette || "").startsWith("night");
    if (night && !NIGHT_PALETTES.includes(style.palette)) {
      style = { ...style, dayPalette: style.palette, palette: "mono" };
    } else if (!night && style.dayPalette) {
      const { dayPalette, ...rest } = style;
      style = { ...rest, palette: dayPalette };
    } else return;
    showForm(); applyStyle();
  };
  new MutationObserver(syncNight).observe(document.documentElement, { attributes: true, attributeFilter: ["data-palette"] });
  syncNight();

  // ---------- live soundings ----------
  // (wrapped below to update the Record soundings button too)
  let liveRefresh = async function () {
    let st = null;
    try { st = await invoke("pbathy_live_status"); } catch { return; }
    $("pb-live").checked = !!st.on;
    if (document.activeElement !== $("pb-live-sog")) $("pb-live-sog").value = st.max_sog || 0;
    const parts = [];
    if (st.on) parts.push(`Live: ${fmt(st.added || 0)} soundings added today${st.left ? `, ${fmt(st.left)} left out` : ""}${st.transducer_m != null ? ` (transducer ${st.transducer_m} m below the waterline)` : ""}.`);
    for (const w of st.warnings || []) parts.push(w);
    const el = $("pb-live-state");
    el.textContent = parts.join(" ");
    el.className = `wx-note ${(st.warnings || []).length ? "warn" : ""}`;
    card.live(st.on ? `Live: ${fmt(st.added || 0)} today${(st.warnings || []).length ? " ⚠" : ""}` : "", (st.warnings || []).join(" "));
  }
  const liveSet = async () => {
    try { await invoke("pbathy_live_set", { on: $("pb-live").checked, maxSog: +$("pb-live-sog").value || 0 }); } catch (e) { note(`Could not change live soundings: ${e}`, "bad"); }
    liveRefresh();
  };
  $("pb-live").onchange = liveSet;
  $("pb-live-sog").onchange = liveSet;
  listen("pbathy-live", () => { reload(); liveRefresh(); });
  setInterval(() => liveRefresh(), 15000);

  // ---------- Record soundings: one click on the toolbar (Fishing; any use if set) ----------
  // Skippers switch recording on over ground they trust and off over ground they don't, so it's
  // a toolbar switch, with a count while it runs, as TimeZero's and Olex's record buttons are.
  let recording = false;
  async function record(on) {
    try { await invoke("pbathy_live_set", { on, maxSog: +$("pb-live-sog").value || 0 }); } catch (e) { note(`Could not change live soundings: ${e}`, "bad"); return; }
    await liveRefresh();
    import("./activity.js").then((a) => a.note(on ? "Recording own soundings from the depth sounder." : "Own soundings recording stopped."));
  }
  const recSync = (st) => {
    recording = !!st?.on;
    const b = $("pb-rec");
    if (!b) return;
    b.classList.toggle("rec", recording);
    b.querySelector(".tb-count").textContent = recording ? shortCount(st.added || 0) : "";
    b.title = recording
      ? `Recording own soundings: ${fmt(st.added || 0)} added today${(st.warnings || []).length ? ` (${st.warnings.join(" ")})` : ""}. Click to stop (D)`
      : "Record own soundings from the depth sounder (D). Right-click for the speed limit and settings";
  };
  const shortCount = (n) => (n >= 10000 ? `${Math.round(n / 1000)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
  const liveRefreshPlain = liveRefresh;
  liveRefresh = async () => { await liveRefreshPlain(); try { recSync(await invoke("pbathy_live_status")); } catch { /* not in Tauri */ } };
  liveRefresh();
  $("pb-rec")?.addEventListener("click", () => record(!recording));
  $("pb-rec")?.addEventListener("contextmenu", (e) => { e.preventDefault(); ui?.openTab?.("chart", null); document.getElementById("pb-live")?.scrollIntoView({ block: "center" }); });
  try { if (localStorage.getItem("racenav.recAll") === "1") document.documentElement.dataset.recAll = ""; } catch { /* storage unavailable */ }
  const recAll = $("set-rec-all");
  if (recAll) {
    recAll.checked = "recAll" in document.documentElement.dataset;
    recAll.onchange = () => {
      if (recAll.checked) document.documentElement.dataset.recAll = ""; else delete document.documentElement.dataset.recAll;
      try { localStorage.setItem("racenav.recAll", recAll.checked ? "1" : "0"); } catch { /* storage unavailable */ }
    };
  }

  showForm();
  invoke("pbathy_style", { style }).catch(() => {}).finally(() => { reload(); refresh(); });
  return { record, recording: () => recording };
}

/** The floating card: title (drag to move), palette, Fit, settings, close; and the three-pointer
 * depth range. `opts`: get() the style, set(changes) apply them, preview(changes) while dragging,
 * fit(), hide(), settings(). */
function makeCard(opts) {
  const el = document.createElement("div");
  el.id = "pb-card";
  el.className = "pb-card";
  el.hidden = true;
  el.innerHTML = `<div class="pb-head"><b>Own soundings</b><select class="pal" title="${PALETTE_HELP}">${PALETTE_OPTIONS}</select>
      <label class="rev" title="Reverse the colours: deep takes the shallow end of the palette"><input type="checkbox" class="rev-cb"> ⇅</label>
      <button class="fit" title="Fit the depth range to the soundings in the chart view">Fit</button>
      <button class="more" title="All the own soundings settings (Chart › Get data)">⚙</button>
      <button class="close" title="Hide own soundings in this view">✕</button></div>
    <div class="seafilter"><div class="track"><div class="grad"></div><span class="h lo"></span><span class="h tg"></span><span class="h hi"></span></div>
      <div class="vals"><span class="v min"></span><span class="v lo"></span><span class="v tg"></span><span class="v hi"></span><span class="v max"></span></div></div>
    <div class="pb-hint">Drag the pointers: shallow and deep set the colours, the red one a target depth (double-click it to clear).</div>
    <div class="pb-live" hidden></div>`;
  const parent = document.getElementById("map") || document.body;
  parent.append(el);
  floatDrag(el, "own-soundings", el.querySelector(".pb-head"));
  const track = el.querySelector(".track");
  // The track runs min..max m. After each drag it is refitted around the shallow and deep
  // pointers (a quarter of the range either side), so drawing them together zooms the scale
  // in and drawing them apart (or to an end) zooms it out.
  let min = 0, max = 0;
  const refit = (shallow, deep) => {
    const span = Math.max(deep - shallow, 1);
    const pad = Math.max(span * 0.25, 1);
    const lo0 = Math.max(0, shallow - pad), hi0 = deep + pad;
    const u = 10 ** Math.floor(Math.log10(Math.max(hi0 - lo0, 1))) / 2;
    min = Math.max(0, Math.floor(lo0 / u) * u);
    max = Math.ceil(hi0 / u) * u;
    if (max - min < 5) { min = Math.max(0, Math.floor((shallow + deep) / 2 - 2.5)); max = min + 5; }
  };
  const fitScale = () => { const s = opts.get(); refit(s.shallow, s.deep); };
  let live = null; // the values while dragging
  function draw() {
    const s = { ...opts.get(), ...(live || {}) };
    if (!max || s.deep > max || s.shallow < min) fitScale();
    el.querySelector(".pal").value = s.palette;
    el.querySelector(".rev-cb").checked = !!s.reverse;
    const pc = (v) => `${Math.min(100, Math.max(0, ((v - min) / (max - min)) * 100))}%`;
    const tg = s.target != null && s.target > s.shallow && s.target < s.deep ? s.target : null;
    const sc = scaleFor(s);
    const w = Math.max(s.deep - s.shallow, 1e-6);
    const grad = el.querySelector(".grad");
    grad.style.left = pc(s.shallow);
    grad.style.width = `calc(${pc(s.deep)} - ${pc(s.shallow)})`;
    grad.style.background = `linear-gradient(to right, ${sc.map(([v, c]) => `${c} ${((v - s.shallow) / w) * 100}%`).join(", ")})`;
    el.querySelector(".h.lo").style.left = pc(s.shallow);
    el.querySelector(".h.hi").style.left = pc(s.deep);
    const th = el.querySelector(".h.tg");
    th.style.left = pc(tg ?? (s.shallow + s.deep) / 2);
    th.classList.toggle("off", tg == null);
    el.querySelector(".v.min").textContent = `${fmtM(min)} m`;
    el.querySelector(".v.lo").textContent = `${fmtM(s.shallow)} m`;
    el.querySelector(".v.tg").textContent = tg == null ? "no target" : `target ${fmtM(tg)} m`;
    el.querySelector(".v.hi").textContent = `${fmtM(s.deep)} m`;
    el.querySelector(".v.max").textContent = `${fmtM(max)} m`;
  }
  // which pointer, and dragging it
  let drag = null;
  const valueAt = (e) => min + ((e.clientX - drag.rect.left) / drag.rect.width) * (max - min);
  const step = () => { const r = max - min; return r > 500 ? 5 : r > 100 ? 1 : r > 20 ? 0.5 : 0.1; };
  // whole metres, or one decimal on a fine scale
  const fmtM = (v) => (max - min > 20 || Number.isInteger(v) ? String(Math.round(v)) : v.toFixed(1));
  track.addEventListener("pointerdown", (e) => {
    const s = opts.get();
    drag = { rect: track.getBoundingClientRect() };
    const v = valueAt(e);
    const tg = s.target ?? (s.shallow + s.deep) / 2;
    drag.k = e.target.classList.contains("h") ? [...e.target.classList].find((c) => ["lo", "tg", "hi"].includes(c))
      : [["lo", s.shallow], ["tg", tg], ["hi", s.deep]].reduce((a, b) => (Math.abs(b[1] - v) < Math.abs(a[1] - v) ? b : a))[0];
    live = { shallow: s.shallow, deep: s.deep, target: s.target };
    track.setPointerCapture(e.pointerId);
    move(e);
  });
  const move = (e) => {
    if (!drag) return;
    const q = step();
    const v = Math.min(max, Math.max(min, Math.round(valueAt(e) / q) * q));
    if (drag.k === "lo") live.shallow = Math.min(v, live.deep - q);
    else if (drag.k === "hi") live.deep = Math.max(v, live.shallow + q);
    else live.target = Math.min(Math.max(v, live.shallow + q), live.deep - q);
    if (live.target != null && (live.target <= live.shallow || live.target >= live.deep)) live.target = null;
    draw();
    opts.preview(live);
  };
  track.addEventListener("pointermove", move);
  track.addEventListener("pointerup", () => {
    if (!drag) return;
    const r = max - min;
    const atHi = drag.k === "hi" && live.deep >= max;
    const atLo = drag.k === "lo" && live.shallow <= min && min > 0;
    drag = null;
    const s = live; live = null;
    // refit the scale to the new range; a pointer pushed to an end gives room to go further
    refit(s.shallow, s.deep);
    if (atHi) max = Math.max(max, Math.ceil((s.deep + r * 0.6) / step()) * step());
    if (atLo) min = Math.max(0, Math.min(min, Math.floor((s.shallow - r * 0.6) / step()) * step()));
    opts.set(s);
    draw();
  });
  // the track captures the pointer while dragging, so a double-click arrives at the track, not at the pointer
  track.addEventListener("dblclick", (e) => {
    const r = el.querySelector(".h.tg").getBoundingClientRect();
    if (opts.get().target != null && Math.abs(e.clientX - (r.left + r.width / 2)) <= Math.max(14, r.width)) opts.set({ target: null });
  });
  el.querySelector(".pal").onchange = (e) => opts.set({ palette: e.target.value });
  el.querySelector(".rev-cb").onchange = (e) => opts.set({ reverse: e.target.checked });
  el.querySelector(".fit").onclick = () => opts.fit();
  el.querySelector(".close").onclick = () => opts.hide();
  el.querySelector(".more").onclick = () => opts.settings();
  return {
    draw,
    fitScale: () => { max = 0; },
    show(on) { el.hidden = !on; if (on) draw(); },
    live(text, warn) { const l = el.querySelector(".pb-live"); l.hidden = !text; l.textContent = text; l.title = warn || ""; l.classList.toggle("warn", !!warn); },
  };
}

/** [[depth, css colour], …] for a style (the same as ownSoundingsScale, for any style). */
function scaleFor(s) {
  const keep = current;
  current = s;
  try { return ownSoundingsScale().stops; } finally { current = keep; }
}
