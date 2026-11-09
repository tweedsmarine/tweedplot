// Survey depths: LINZ's multibeam and lidar depth models from TweedPlot depth packages
// (.tpdepth, made by tweedplot-depthpack; depthpacks.rs). The layer (main.js "depthpack"),
// the Get data rows (import, list, delete), the survey outlines with a card for each, and the
// depth under the cursor.
import { invoke } from "@tauri-apps/api/core";
import { infoBox } from "./infobox.js";

const SHOW = "racenav.surveydepths.show";
const OUTLINES = "racenav.surveydepths.outlines";
const $ = (id) => document.getElementById(id);
const flag = (k, dflt) => { try { const v = localStorage.getItem(k); return v == null ? dflt : v === "1"; } catch { return dflt; } };
const setFlag = (k, v) => { try { localStorage.setItem(k, v ? "1" : "0"); } catch { /* ignore */ } };
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const fmtMB = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1e3))} kB`);

let have = false;

export function setupSurveyDepths(map, { toast } = {}) {
  map.addSource("sd-outlines", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "sd-outline", type: "line", source: "sd-outlines", layout: { visibility: flag(OUTLINES, false) ? "visible" : "none" },
    paint: { "line-color": "#2b6ca3", "line-width": 1.2, "line-dasharray": [3, 2], "line-opacity": 0.85 } }, "track");
  map.addLayer({ id: "sd-outline-hit", type: "line", source: "sd-outlines", layout: { visibility: flag(OUTLINES, false) ? "visible" : "none" },
    paint: { "line-color": "#000", "line-opacity": 0.01, "line-width": 12 } }, "track");

  const show = $("sd-show"), outl = $("sd-outlines");
  show.checked = flag(SHOW, true);
  outl.checked = flag(OUTLINES, false);
  const applyShow = () => { if (map.getLayer("depthpack")) map.setLayoutProperty("depthpack", "visibility", show.checked ? "visible" : "none"); };
  show.onchange = () => { setFlag(SHOW, show.checked); applyShow(); };
  outl.onchange = () => {
    setFlag(OUTLINES, outl.checked);
    for (const id of ["sd-outline", "sd-outline-hit"]) map.setLayoutProperty(id, "visibility", outl.checked ? "visible" : "none");
  };
  applyShow();

  const reload = () => {
    const src = map.getSource("depthpack");
    if (src?.setTiles) src.setTiles(src.tiles.map((t) => t.replace(/[?&]r=\d+/, "") + `${t.includes("?") ? "&" : "?"}r=${Date.now()}`));
  };
  async function loadOutlines() {
    try { map.getSource("sd-outlines")?.setData(await invoke("depthpack_surveys")); } catch { /* none */ }
  }

  async function refresh() {
    let st = null;
    try { st = await invoke("depthpack_status"); } catch { return; }
    if (!st) return;
    const pk = st.packages || [];
    have = pk.length > 0;
    $("sd-dot").className = `dot ${have ? "ok" : ""}`;
    const editions = [...new Set(pk.map((p) => p.edition).filter(Boolean))].sort();
    const bytes = pk.reduce((a, p) => a + (p.bytes || 0), 0);
    $("sd-state").textContent = have ? `${pk.length} package${pk.length === 1 ? "" : "s"}, ${st.surveys} surveys, edition ${editions.join(", ")}, ${fmtMB(bytes)}` : "none (import a package)";
    $("sd-del").hidden = !have;
    const ul = $("sd-list");
    ul.innerHTML = "";
    // national first, then the regions by name
    pk.sort((a, b) => (a.tier === "national" ? -1 : b.tier === "national" ? 1 : (a.region_name || "").localeCompare(b.region_name || "")));
    for (const p of pk) {
      const li = document.createElement("li");
      li.innerHTML = `<span class="grow"><b>${esc(p.region_name || p.region || p.file)}</b> <small>${p.tier === "national" ? "national, to ~7 m" : "full detail"} · ${esc(p.edition)} · ${p.surveys} surveys · ${fmtMB(p.bytes)}</small></span><button data-act="del" title="Delete this package">✕</button>`;
      li.title = p.file;
      li.querySelector('[data-act="del"]').onclick = async () => {
        try { await invoke("depthpack_delete", { file: p.file }); } catch (e) { toast?.(`Could not delete: ${e}`, 6000); }
        refresh(); reload(); loadOutlines();
      };
      ul.append(li);
    }
    for (const pr of st.problems || []) {
      const li = document.createElement("li");
      li.className = "bad";
      li.textContent = `${pr.file}: ${pr.error}`;
      ul.append(li);
    }
  }

  async function doImport(folder) {
    let r = null;
    try { r = await invoke("depthpack_import", { folder }); } catch (e) { toast?.(`Import failed: ${e}`, 8000); return; }
    if (r?.cancelled) return;
    const added = r.added || [], skipped = r.skipped || [];
    const parts = [];
    if (added.length) parts.push(`Imported ${added.map((a) => `${a.region} (${a.edition})`).join(", ")}.`);
    if (skipped.length) parts.push(`Not imported: ${skipped.map((s) => `${s.file}: ${s.why}`).join("; ")}.`);
    toast?.(parts.join(" ") || "Nothing imported.", 9000);
    await refresh(); reload(); loadOutlines();
  }
  $("sd-import").onclick = () => doImport(false);
  $("sd-import-dir").onclick = () => doImport(true);
  $("sd-del").onclick = async () => {
    if (!confirmDelete()) return;
    try { await invoke("depthpack_delete", { file: "" }); } catch (e) { toast?.(`Could not delete: ${e}`, 6000); }
    refresh(); reload(); loadOutlines();
  };
  // a second press within 4 s deletes (no browser dialogs)
  let armedAt = 0;
  function confirmDelete() {
    if (Date.now() - armedAt < 4000) return true;
    armedAt = Date.now();
    $("sd-del").textContent = "Press again to delete all";
    setTimeout(() => { $("sd-del").textContent = "✕ All"; }, 4000);
    return false;
  }

  // a survey's card: click its outline
  const box = infoBox(map, "survey", { width: 330, height: 230 });
  map.on("click", (e) => {
    if (!outl.checked || document.querySelector(".maplibregl-map.drawing")) return;
    const f = map.queryRenderedFeatures([[e.point.x - 6, e.point.y - 6], [e.point.x + 6, e.point.y + 6]], { layers: ["sd-outline-hit"] })[0];
    if (!f) return;
    const p = f.properties;
    const el = document.createElement("div");
    el.innerHTML = `<table class="sd-card">
      <tr><td>Survey</td><td>${esc(p.title)}</td></tr>
      ${p.year ? `<tr><td>Year</td><td>${esc(p.year)}</td></tr>` : ""}
      <tr><td>Grid</td><td>${esc(p.res)} m</td></tr>
      <tr><td>Datum</td><td>${esc(p.datum)}</td></tr>
      ${p.accuracy ? `<tr><td>Accuracy</td><td>${esc(p.accuracy)}</td></tr>` : ""}
      ${p.published ? `<tr><td>Published</td><td>${esc(p.published)}</td></tr>` : ""}
      <tr><td>LINZ layer</td><td>${esc(p.id)}</td></tr></table>
      <div class="wx-note">Sourced from the LINZ Data Service and licensed for reuse under the CC BY 4.0 licence. Depths are shoal-biased. Not for navigation.</div>`;
    box.open(el, { title: "Survey", at: [e.lngLat.lng, e.lngLat.lat] });
  });

  refresh();
  loadOutlines();
  return { refresh };
}

// the depth under the cursor, asked for at most one at a time: `show(text)` ("" for none)
let busy = false, next = null;
export function surveyDepthAt(map, lon, lat, show) {
  if (!have || map.getZoom() < 8 || map.getLayer("depthpack") == null || map.getLayoutProperty("depthpack", "visibility") === "none") { show(""); return; }
  next = [lon, lat, show];
  if (busy) return;
  busy = true;
  (async () => {
    while (next) {
      const [lo, la, cb] = next; next = null;
      let h = null;
      try { h = await invoke("depthpack_height", { lon: lo, lat: la }); } catch { h = null; }
      cb(h == null ? "" : h <= 0 ? `survey ${(-h).toFixed(1)} m (LAT)` : `survey dries ${h.toFixed(1)} m`);
    }
    busy = false;
  })();
}
