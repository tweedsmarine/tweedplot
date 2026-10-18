// Chart › Chart data: what's installed, and downloads of the world coastline
// (GSHHG) and detailed depths (ETOPO 2022) for the chart view. The world depths
// (Natural Earth) come with the app.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const $ = (id) => document.getElementById(id);
const LDS = "https://data.linz.govt.nz/";

export function setupData(map, { toast } = {}) {
  let status = null;
  let busy = false;

  const fmtMB = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(b >= 1e8 ? 0 : 1)} MB` : `${Math.max(1, Math.round(b / 1e3))} kB`);
  const fmtArea = (a) => {
    const lat = (v) => `${Math.abs(v).toFixed(1)}°${v < 0 ? "S" : "N"}`;
    const lon = (v) => { const w = ((v + 540) % 360) - 180; return `${Math.abs(w).toFixed(1)}°${w < 0 ? "W" : "E"}`; };
    return `${lat(a[1])}–${lat(a[3])} ${lon(a[0])}–${lon(a[2])}`;
  };
  const say = (t, kind = "") => { const el = $("data-status"); el.textContent = t; el.className = `grow wx-note ${kind}`; };
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
    dot("data-wd-dot", status.world_depth ? "ok" : "");
    dot("data-gshhg-dot", status.world_land ? "ok" : status.gshhg ? "part" : "");
    $("data-gshhg-state").textContent = status.world_land ? "GSHHG, every island"
      : status.gshhg ? "GSHHG downloaded, being prepared…" : `coarse outline only (GSHHG not downloaded, about ${fmtMB(status.gshhg_bytes)})`;
    $("data-gshhg").hidden = status.gshhg;
    dot("data-linz-dot", status.linz_tables > 0 ? "ok" : "");
    $("data-linz-state").textContent = status.linz_tables > 0 ? `${status.linz_tables} layers` : "none (optional, NZ only)";
    $("data-linz").title = `Open the LINZ Data Service in your browser (free account) and download the hydrographic layers as GeoPackage into\n${status.dirs.charts}`;
    const list = $("data-etopo-list");
    list.innerHTML = "";
    dot("data-etopo-dot", status.etopo.length ? "ok" : "");
    for (const a of status.etopo) {
      const li = document.createElement("li");
      li.innerHTML = `<span></span><button data-act="zoom" title="Show this area">Show</button><button data-act="del" title="Delete this area">✕</button>`;
      li.firstChild.textContent = `${fmtArea(a.area)} · ${Math.round(a.spacing)}″ · ${fmtMB(a.bytes)}`;
      li.querySelector('[data-act="zoom"]').onclick = () => map.fitBounds([[a.area[0], a.area[1]], [a.area[2], a.area[3]]], { padding: 40, duration: 600 });
      li.querySelector('[data-act="del"]').onclick = async () => {
        try { await invoke("data_etopo_delete", { file: a.file }); say("Depth area deleted."); } catch (e) { say(`Could not delete: ${e}`, "bad"); }
        refresh();
      };
      list.append(li);
    }
    plan();
  }

  async function plan() {
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
    for (const id of ["data-gshhg", "data-etopo"]) $(id).disabled = on;
    $("data-cancel").disabled = !on;
    if (!on) $("data-progress").hidden = true;
  }

  listen("data-progress", ({ payload }) => {
    const p = $("data-progress");
    p.hidden = false;
    const frac = payload.total ? payload.done / payload.total : null;
    p.querySelector(".bar").style.width = frac == null ? "100%" : `${(frac * 100).toFixed(1)}%`;
    p.classList.toggle("indeterminate", frac == null);
    p.querySelector("span").textContent = `${payload.stage}${payload.done ? ` · ${fmtMB(payload.done)}${payload.total ? ` of ${fmtMB(payload.total)}` : ""}` : ""}`;
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

  $("data-cancel").onclick = () => invoke("data_cancel").catch(() => {});
  $("data-linz").onclick = () => invoke("open_url", { url: LDS }).catch(() => {});
  $("data-etopo-res").onchange = plan;
  map.on("moveend", () => { if (!document.getElementById("data-box").closest(".pane")?.hidden) plan(); });
  window.addEventListener("racenav-charts", refresh);
  refresh();

  return { refresh };
}
