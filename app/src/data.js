// Chart › Chart data: what's installed, and downloads of the world coastline
// (GSHHG) and detailed depths (ETOPO 2022) for the chart view. The world depths
// (Natural Earth) come with the app.

import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
import { listen } from "@tauri-apps/api/event";

const $ = (id) => document.getElementById(id);
const LDS = "https://data.linz.govt.nz/";

const DETAIL_KEY = "racenav.coastDetail";

export function setupData(map, { toast, reloadTiles } = {}) {
  let status = null;
  let busy = false;

  const fmtMB = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(b >= 1e8 ? 0 : 1)} MB` : `${Math.max(1, Math.round(b / 1e3))} kB`);
  const fmtArea = (a) => {
    const lat = (v) => `${Math.abs(v).toFixed(1)}°${v < 0 ? "S" : "N"}`;
    const lon = (v) => { const w = ((v + 540) % 360) - 180; return `${Math.abs(w).toFixed(1)}°${w < 0 ? "W" : "E"}`; };
    return `${lat(a[1])}–${lat(a[3])} ${lon(a[0])}–${lon(a[2])}`;
  };
  // the drawer's line, and an activity card for each download (seen with the drawer closed):
  // the first message while busy starts the card, a good/warn/bad one ends it
  let job = null;
  const say = (t, kind = "") => {
    const el = $("data-status"); el.textContent = t; el.className = `grow wx-note ${kind}`;
    if (busy && !job && !kind) { job = activity.start(t.replace(/…$/, ""), { cancel: () => invoke("data_cancel").catch(() => {}) }); return; }
    if (job) {
      if (!kind) job.progress(t.replace(/…$/, ""));
      else { if (kind === "bad") job.fail(t); else job.done(t, { keep: kind === "warn" }); job = null; }
      return;
    }
    if (kind === "bad") activity.start("Data").fail(t);
    else if (kind) activity.note(t);
  };
  const dot = (id, state) => { $(id).className = `dot ${state}`; };

  /** The chart view as [w, s, e, n] (east may pass 180). */
  function viewArea() {
    const b = map.getBounds();
    let w = b.getWest(), e = b.getEast();
    if (e - w >= 360) { w = -180; e = 180; }
    return [w, Math.max(-89.9, b.getSouth()), e, Math.min(89.9, b.getNorth())];
  }

  async function refresh() {
    try { status = await invoke("data_status"); } catch { return; }
    if (!status) return;
    dot("data-wd-dot", status.world_depth ? "ok" : "");
    dot("data-gshhg-dot", status.world_land ? "ok" : status.gshhg ? "part" : "");
    $("data-gshhg-state").textContent = status.world_land ? "GSHHG, every island"
      : status.gshhg ? "GSHHG downloaded, being prepared…" : `coarse outline only (GSHHG not downloaded, about ${fmtMB(status.gshhg_bytes)})`;
    $("data-gshhg").hidden = status.gshhg;
    $("data-coast-detail").disabled = !status.world_land;
    const mb = status.boundaries || {};
    dot("data-mb-dot", mb.lines > 0 ? "ok" : mb.zips > 0 ? "part" : "");
    $("data-mb-state").textContent = mb.lines > 0 ? `${mb.zips} files` : mb.zips > 0 ? "downloaded, being prepared…" : "not downloaded";
    $("data-mb").textContent = mb.zips > 0 ? "Update" : "Download";
    const mpi = status.mpi || { files: [] };
    const when = mpi.files.map((f) => f.fetched).filter(Boolean).sort()[0];
    dot("data-mpi-dot", mpi.pieces > 0 ? "ok" : mpi.files.length ? "part" : "");
    $("data-mpi-state").textContent = mpi.files.length
      ? `${mpi.files.length} sets, ${fmtMB(mpi.files.reduce((a, f) => a + f.bytes, 0))}${when ? `, from ${new Date(when).toLocaleDateString()}` : ""}${mpi.pieces > 0 ? "" : " (being prepared…)"}`
      : "not downloaded (NZ only)";
    $("data-mpi").textContent = mpi.files.length ? "Update" : "Download";
    dot("data-linz-dot", status.linz_tables > 0 ? "ok" : "");
    $("data-linz-state").textContent = status.linz_tables > 0 ? `${status.linz_tables} layers` : "none (optional, NZ only)";
    $("data-linz").title = `Open the LINZ Data Service in your browser (free account) and download the hydrographic layers as GeoPackage into\n${status.dirs.charts}`;
    const list = $("data-etopo-list");
    list.innerHTML = "";
    dot("data-etopo-dot", status.etopo.length ? "ok" : "");
    for (const a of status.etopo) {
      const li = document.createElement("li");
      li.innerHTML = `<span></span>${a.old ? `<button data-act="redo" class="warn" title="Downloaded by an older TweedPlot: its shading breaks into stripes when zoomed out. Download this area again">Redo</button>` : ""}<button data-act="zoom" title="Show this area">Show</button><button data-act="del" title="Delete this area">✕</button>`;
      li.firstChild.textContent = `${fmtArea(a.area)} · ${Math.round(a.spacing)}″ · ${fmtMB(a.bytes)}${a.old ? " · old format" : ""}`;
      li.querySelector('[data-act="redo"]')?.addEventListener("click", async () => {
        if (busy) return;
        setBusy(true);
        say(`Downloading ${fmtArea(a.area)} again…`);
        try {
          await invoke("data_etopo_delete", { file: a.file }); // the old file is open: remove it before writing the new one
          await invoke("data_etopo", { area: a.area, arcsec: Math.round(a.spacing) || 15 });
          say("Depths downloaded again.", "good");
        } catch (e) {
          say(String(e) === "cancelled" ? "Cancelled: the old area was removed, press Get for chart view to fetch it." : `Download failed (the old area was removed): ${e}`, "bad");
        } finally { setBusy(false); refresh(); }
      });
      li.querySelector('[data-act="zoom"]').onclick = () => map.fitBounds([[a.area[0], a.area[1]], [a.area[2], a.area[3]]], { padding: 40, duration: 600 });
      li.querySelector('[data-act="del"]').onclick = async () => {
        try { await invoke("data_etopo_delete", { file: a.file }); say("Depth area deleted."); } catch (e) { say(`Could not delete: ${e}`, "bad"); }
        refresh();
      };
      list.append(li);
    }
    plan();
  }

  // ---------- high-resolution depths (LINZ coastal elevation) ----------------------------
  let hrPlanning = 0;
  async function refreshHires() {
    let h = null;
    try { h = await invoke("hires_status"); } catch { return; }
    if (!h) return;
    const bytes = h.tiles.reduce((a, t) => a + t.bytes, 0);
    dot("data-hr-dot", h.tiles.length ? "ok" : "");
    $("data-hr-state").textContent = h.tiles.length ? `${h.tiles.length} tiles, ${fmtMB(bytes)}` : "none (NZ coasts, where surveyed)";
    $("data-hr-del").hidden = !h.tiles.length;
  }
  async function planHires() {
    const area = viewArea();
    const z = map.getZoom();
    const me = ++hrPlanning;
    if (z < 10) { $("data-hr-plan").textContent = "Zoom in (to about 5 NM range) to get the high-resolution depths for the chart view."; $("data-hr").disabled = true; return; }
    $("data-hr").disabled = busy;
    $("data-hr-plan").textContent = "Looking up the tiles for the chart view…";
    try {
      const p = await invoke("hires_plan", { area });
      if (me !== hrPlanning) return;
      $("data-hr-plan").textContent = p.available === 0 ? "No high-resolution depths here (not surveyed yet)."
        : `Chart view: ${p.available} tile${p.available === 1 ? "" : "s"}${p.have ? `, ${p.have} downloaded` : ""} (Topo50 sheets, each 24 × 36 km; size depends on how much was surveyed).`;
      $("data-hr").disabled = busy || p.available === p.have;
    } catch (e) {
      if (me === hrPlanning) $("data-hr-plan").textContent = `Can't look up the tiles: ${e}`;
    }
  }
  $("data-hr-show").checked = localStorage.getItem("racenav.hires.show") !== "0";
  $("data-hr-show").onchange = (e) => {
    try { localStorage.setItem("racenav.hires.show", e.target.checked ? "1" : "0"); } catch { /* ignore */ }
    if (map.getLayer("hires")) map.setLayoutProperty("hires", "visibility", e.target.checked ? "visible" : "none");
  };
  const reloadHires = () => { const src = map.getSource("hires"); if (src?.setTiles) src.setTiles(src.tiles.map((t) => t.replace(/[?&]r=\d+/, "") + `${t.includes("?") ? "&" : "?"}r=${Date.now()}`)); };
  $("data-hr").onclick = async () => {
    if (busy) return;
    setBusy(true);
    say("Downloading high-resolution depths…");
    try {
      const n = await invoke("hires_download", { area: viewArea() });
      say(n ? `High-resolution depths: ${n} tile${n === 1 ? "" : "s"} downloaded.` : "Already downloaded.", "good");
      reloadHires();
    } catch (e) {
      say(String(e) === "cancelled" ? "Download cancelled." : `Download failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally { setBusy(false); refreshHires(); planHires(); }
  };
  $("data-hr-del").onclick = async () => {
    try { await invoke("hires_delete", { name: "" }); say("High-resolution depths deleted."); reloadHires(); } catch (e) { say(`Could not delete: ${e}`, "bad"); }
    refreshHires(); planHires();
  };

  // ---------- NIWA bathymetry grid (imported from NIWA's zip) ------------------------------
  async function refreshNz() {
    let n = null;
    try { n = await invoke("nzbathy_status"); } catch { return; }
    if (!n) return;
    dot("data-nz-dot", n.have ? "ok" : "");
    $("data-nz-state").textContent = n.have ? `imported, ${fmtMB(n.bytes)}` : "not imported (NIWA 2016 grid)";
    $("data-nz-del").hidden = !n.have;
  }
  const setNzLayer = (on) => {
    try { localStorage.setItem("racenav.nzbathy.show", on ? "1" : "0"); } catch { /* ignore */ }
    $("data-nz-show").checked = on;
    if (map.getLayer("nzbathy")) map.setLayoutProperty("nzbathy", "visibility", on ? "visible" : "none");
  };
  const reloadNz = () => { const src = map.getSource("nzbathy"); if (src?.setTiles) src.setTiles(src.tiles.map((t) => t.replace(/[?&]r=\d+/, "") + `${t.includes("?") ? "&" : "?"}r=${Date.now()}`)); };
  try { $("data-nz-show").checked = localStorage.getItem("racenav.nzbathy.show") === "1"; } catch { /* storage unavailable */ }
  $("data-nz-show").onchange = (e) => setNzLayer(e.target.checked);
  $("data-nz").onclick = async () => {
    if (busy) return;
    let path = null;
    try { path = await invoke("nzbathy_pick_file"); } catch (e) { say(`Could not open the file dialog: ${e}`, "bad"); return; }
    if (!path) return;
    setBusy(true);
    say("Importing the NIWA bathymetry grid…");
    try {
      await invoke("nzbathy_import", { path });
      say("NIWA bathymetry imported. Shown in Chart › Layers (not for navigation).", "good");
      setNzLayer(true);
      reloadNz();
    } catch (e) {
      say(String(e) === "cancelled" ? "Import cancelled." : `Import failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally { setBusy(false); refreshNz(); }
  };
  $("data-nz-del").onclick = async () => {
    try { await invoke("nzbathy_delete"); say("NIWA bathymetry deleted."); reloadNz(); } catch (e) { say(`Could not delete: ${e}`, "bad"); }
    refreshNz();
  };

  async function plan() {
    planHires();
    const area = viewArea();
    const res = +$("data-etopo-res").value;
    try {
      const p = await invoke("data_etopo_plan", { area, arcsec: res });
      const thinned = p.spacing > res ? ` (thinned to ${p.spacing}″ for an area this size; zoom in for more detail)` : "";
      $("data-etopo-plan").textContent = `Chart view ${fmtArea(area)}: about ${fmtMB(p.bytes)} to download${thinned}.`;
    } catch { /* not ready */ }
  }

  function setBusy(on) {
    busy = on;
    for (const id of ["data-gshhg", "data-etopo", "data-mb", "data-mpi", "data-hr", "data-nz"]) $(id).disabled = on;
    $("data-cancel").disabled = !on;
    if (!on) { $("data-progress").hidden = true; if (job) { job.done(""); job = null; } }
  }

  listen("data-progress", ({ payload }) => {
    const p = $("data-progress");
    p.hidden = false;
    const frac = payload.total ? payload.done / payload.total : null;
    p.querySelector(".bar").style.width = frac == null ? "100%" : `${(frac * 100).toFixed(1)}%`;
    p.classList.toggle("indeterminate", frac == null);
    p.querySelector("span").textContent = `${payload.stage}${payload.done ? ` · ${fmtMB(payload.done)}${payload.total ? ` of ${fmtMB(payload.total)}` : ""}` : ""}`;
    job?.progress(payload.stage, payload.done || 0, payload.total || 0, p.querySelector("span").textContent);
  });

  $("data-gshhg").onclick = async () => {
    if (busy) return;
    setBusy(true);
    say("Downloading the world coastline…");
    try {
      await invoke("data_gshhg");
      say("World coastline downloaded. Preparing it (a minute or two the first time)…", "good");
    } catch (e) {
      say(String(e) === "cancelled" ? "Download cancelled." : `Download failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally { setBusy(false); refresh(); }
  };

  $("data-mb").onclick = async () => {
    if (busy) return;
    setBusy(true);
    say("Downloading the maritime boundaries…");
    try {
      const r = await invoke("data_boundaries");
      say(`Maritime boundaries downloaded (${r.files} files${r.failed.length ? `; not available: ${r.failed.length}` : ""}).`, r.failed.length ? "warn" : "good");
      if (r.failed.length) console.warn("maritime boundaries:", r.failed);
    } catch (e) {
      say(String(e) === "cancelled" ? "Download cancelled." : `Download failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally { setBusy(false); refresh(); }
  };

  $("data-mpi").onclick = async () => {
    if (busy) return;
    setBusy(true);
    say("Downloading MPI's fishing rules…");
    try {
      const r = await invoke("data_mpi");
      const failed = r.failed.length ? `; some sets failed: ${r.failed.join("; ")}` : "";
      say(`Fishing rules downloaded: ${r.features} areas${failed}. Preparing them…`, r.failed.length ? "warn" : "good");
      // the charts reopen with the rules built into them: say so when they're ready
      const ready = (e) => {
        if (e.detail?.ready === false) return;
        window.removeEventListener("racenav-charts", ready);
        say(`Fishing rules ready: ${r.features} areas${failed}. Chart › Boundaries and fishing rules.`, r.failed.length ? "warn" : "good");
        toast?.("Fishing rules updated", 3000);
      };
      window.addEventListener("racenav-charts", ready);
    } catch (e) {
      say(String(e) === "cancelled" ? "Download cancelled." : `Download failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally { setBusy(false); refresh(); }
  };

  $("data-etopo").onclick = async () => {
    if (busy) return;
    const area = viewArea();
    setBusy(true);
    say(`Downloading depths for ${fmtArea(area)}…`);
    try {
      await invoke("data_etopo", { area, arcsec: +$("data-etopo-res").value });
      say("Depths downloaded: drawn under the charts from zoom 4, contours from zoom 7.", "good");
      toast?.("Detailed depths added", 3000);
    } catch (e) {
      say(String(e) === "cancelled" ? "Download cancelled." : `Download failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally { setBusy(false); refresh(); }
  };

  // Coastline detail: kept per PC, sent to the chart store at start and on change
  let detail = 0;
  try { detail = Math.min(2, Math.max(0, +localStorage.getItem(DETAIL_KEY) || 0)); } catch { /* storage unavailable */ }
  $("data-coast-detail").value = String(detail);
  invoke("chart_land_detail", { level: detail }).then(() => { if (detail) reloadTiles?.(); }).catch(() => {});
  $("data-coast-detail").onchange = async () => {
    detail = +$("data-coast-detail").value;
    try { localStorage.setItem(DETAIL_KEY, String(detail)); } catch { /* ignore */ }
    try { await invoke("chart_land_detail", { level: detail }); reloadTiles?.(); } catch (e) { say(`Could not change the detail: ${e}`, "bad"); }
  };

  // ---------- ENC chart sets: each exchange set (NOAA's, the IHO's test data …) in or out of use
  const PRODUCERS = { US: "NOAA (USA)", NZ: "LINZ (New Zealand)", AU: "Australia", GB: "UKHO", FR: "SHOM (France)", CA: "Canada", JP: "Japan", NO: "Norway", AA: "IHO test data", "1B": "IHO test data" };
  const fmtProducers = (codes) => {
    const by = new Map();
    for (const [c, n] of codes) by.set(PRODUCERS[c] ?? c, (by.get(PRODUCERS[c] ?? c) ?? 0) + n);
    const p = [...by].sort((a, b) => b[1] - a[1]);
    return p.slice(0, 3).map(([name, n]) => `${name}${p.length > 1 ? ` ${n.toLocaleString()}` : ""}`).join(", ") + (p.length > 3 ? ", …" : "");
  };
  async function refreshSets() {
    const ul = $("data-sets");
    $("data-sets-state").textContent = "looking…";
    let sets = [];
    try { sets = await invoke("chart_sets_list"); } catch (e) { $("data-sets-state").textContent = String(e); return; }
    const on = sets.filter((x) => x.in_use);
    dot("data-sets-dot", on.length ? "ok" : "");
    $("data-sets-state").textContent = sets.length
      ? `${on.length} of ${sets.length} in use, ${on.reduce((a, x) => a + x.cells, 0).toLocaleString()} cells`
      : "none (put S-57 cells or exchange sets in the enc folder)";
    ul.innerHTML = "";
    for (const x of sets) {
      const li = document.createElement("li");
      li.innerHTML = `<label><input type="checkbox"> <b></b></label><small></small>`;
      const cb = li.querySelector("input");
      cb.checked = x.in_use;
      li.querySelector("b").textContent = x.name;
      li.querySelector("small").textContent = `${x.cells.toLocaleString()} cell${x.cells === 1 ? "" : "s"} · ${fmtProducers(x.producers)} · ${x.folder}`;
      li.title = x.path;
      cb.onchange = async () => {
        cb.disabled = true;
        try {
          await invoke("chart_set_use", { path: x.path, on: cb.checked });
          activity.note(`Chart set ${x.name} ${cb.checked ? "in use" : "left out"}: opening the charts again`);
        } catch (e) { cb.checked = !cb.checked; say(String(e), "bad"); }
        cb.disabled = false;
        refreshSets();
      };
      ul.append(li);
    }
  }
  $("data-sets-refresh").onclick = refreshSets;
  // only when the Chart data box is first opened: listing the sets walks every cell
  let setsLooked = false;
  const watch = new MutationObserver(() => {
    if (setsLooked || $("data-box").closest(".pane")?.hidden) return;
    setsLooked = true;
    watch.disconnect();
    refreshSets();
  });
  watch.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["hidden", "open"] });

  $("data-cancel").onclick = () => invoke("data_cancel").catch(() => {});
  $("data-linz").onclick = () => invoke("open_url", { url: LDS }).catch(() => {});
  $("data-etopo-res").onchange = plan;
  map.on("moveend", () => { if (!document.getElementById("data-box").closest(".pane")?.hidden) plan(); });
  window.addEventListener("racenav-charts", refresh);
  refresh();
  refreshHires();
  refreshNz();

  return { refresh };
}
