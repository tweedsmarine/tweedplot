// Chart › Layers › Online maps: OpenStreetMap (replaces the bundled land) and OpenSeaMap
// seamarks, or tile servers of your own. Everything is off until the user turns it on, and
// the Rust side (webmap.rs) sends nothing to any tile server while the master switch is off.

import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);
const KEY = "tweedplot.webmap";
const DEFAULTS = { on: false, osm: false, sea: false, cachedOnly: false, osmUrl: "", seaUrl: "", credit: "" };
// the land the online base map replaces (the bundled land and the chart's world land and coast)
const LAND_FILL = ["land-world", "land-nz", "chart-world-land", "chart-land"];
const LAND_LINE = ["coast-world", "coast-nz", "chart-coast", "chart-wcoast"];
const COPYRIGHT = "https://www.openstreetmap.org/copyright";

export function loadWebMapPrefs() {
  try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) || "{}") }; } catch { return { ...DEFAULTS }; }
}

export function setupWebMap(map) {
  const prefs = loadWebMapPrefs();
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  const openUrl = (url) => invoke("open_url", { url }).catch(() => window.open(url));
  const link = (text, url) => { const a = document.createElement("a"); a.href = "#"; a.textContent = text; a.onclick = (e) => { e.preventDefault(); openUrl(url); }; return a; };
  const osmOn = () => prefs.on && prefs.osm;
  const seaOn = () => prefs.on && prefs.sea;

  // A tile the server could not supply comes back as an error; ask again after a pause so no holes stay.
  // setTiles (public API) reloads the source in place: tiles already shown stay up while they're asked
  // for again, and those come from the disk cache in Rust, so only the missing ones go to the server.
  const retry = {};
  map.on("error", (e) => {
    const id = e.sourceId;
    if ((id !== "osm" && id !== "osea") || retry[id] || !prefs.on || prefs.cachedOnly) return;
    retry[id] = setTimeout(() => {
      retry[id] = null;
      const src = map.getSource(id);
      if (src?.setTiles && prefs.on && !prefs.cachedOnly) src.setTiles([...src.tiles]);
    }, 25000);
  });
  function credit() {
    const el = $("map-attribution");
    if (!el) return;
    el.replaceChildren("Charts: LINZ, CC BY 4.0. Land: ", link("© OpenStreetMap contributors", COPYRIGHT), " (ODbL). ");
    if (osmOn() && !prefs.osmUrl) el.append("Map: ", link("© OpenStreetMap contributors", COPYRIGHT), ". ");
    if (seaOn() && !prefs.seaUrl) el.append("Seamarks: ", link("© OpenSeaMap contributors", "https://www.openseamap.org"), ". ");
    if ((osmOn() && prefs.osmUrl) || (seaOn() && prefs.seaUrl)) el.append(prefs.credit ? `${prefs.credit}. ` : "Online map: your own tile server. ");
    el.append("Not for navigation.");
  }

  function show() {
    const set = (id, on) => { if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", on ? "visible" : "none"); };
    const replace = osmOn();
    for (const id of LAND_FILL) if (map.getLayer(id)) map.setPaintProperty(id, "fill-opacity", replace ? 0 : 1);
    for (const id of LAND_LINE) if (map.getLayer(id)) map.setPaintProperty(id, "line-opacity", replace ? 0 : 1);
    set("osm", osmOn());
    set("osea", seaOn());
    window.dispatchEvent(new CustomEvent("webmap-changed"));
  }

  function status(text, kind = "") { const n = $("wm-note"); if (n) { n.textContent = text; n.className = `wx-note ${kind}`; } }
  const fmtMB = (b) => (b >= 1e6 ? `${(b / 1e6).toFixed(b >= 1e8 ? 0 : 1)} MB` : `${Math.max(0, Math.round(b / 1e3))} kB`);

  async function cacheLine() {
    try {
      const s = await invoke("webmap_status");
      const el = $("wm-cache"); if (el) el.textContent = `Cached map tiles: ${s.tiles.toLocaleString()} (${fmtMB(s.bytes)})`;
    } catch { /* ignore */ }
  }

  function reload() {
    for (const id of ["osm", "osea"]) {
      const src = map.getSource(id);
      if (src?.setTiles) src.setTiles(src.tiles.map((t) => t.replace(/[?&]r=\d+/, "") + `${t.includes("?") ? "&" : "?"}r=${Date.now()}`));
    }
  }

  // Layers go off first, then Rust is told; turning on: Rust first, so the first tiles are served.
  async function apply({ reloadTiles = false } = {}) {
    const on = prefs.on && (prefs.osm || prefs.sea);
    if (!on) show();
    try {
      await invoke("webmap_set", { cfg: { enabled: on, cachedOnly: prefs.cachedOnly, osmUrl: prefs.osmUrl.trim(), seaUrl: prefs.seaUrl.trim() } });
    } catch (e) {
      status(String(e), "bad");
      prefs.on = false; save(); sync(); show(); credit();
      return false;
    }
    if (on) { show(); if (reloadTiles) reload(); }
    credit();
    status(!prefs.on ? "Online maps are off: TweedPlot sends no map requests."
      : prefs.cachedOnly ? "Online maps: showing tiles already saved on this computer. No requests are sent."
      : "Online maps are on: the tile servers see this computer's IP address and the area you look at. Tiles are saved on this computer.");
    cacheLine();
    return true;
  }

  function sync() {
    $("wm-on").checked = prefs.on;
    $("wm-osm").checked = prefs.osm;
    $("wm-sea").checked = prefs.sea;
    $("wm-cached").checked = prefs.cachedOnly;
    for (const id of ["wm-osm", "wm-sea", "wm-cached"]) $(id).disabled = !prefs.on;
    $("wm-osm-url").value = prefs.osmUrl;
    $("wm-sea-url").value = prefs.seaUrl;
    $("wm-credit").value = prefs.credit;
  }

  const bind = (id, key, { text = false, reloadTiles = false } = {}) => {
    $(id).onchange = async (e) => {
      prefs[key] = text ? e.target.value.trim() : e.target.checked;
      save(); sync();
      await apply({ reloadTiles });
    };
  };
  bind("wm-on", "on", { reloadTiles: true });
  bind("wm-osm", "osm", { reloadTiles: true });
  bind("wm-sea", "sea", { reloadTiles: true });
  bind("wm-cached", "cachedOnly", { reloadTiles: true });
  bind("wm-osm-url", "osmUrl", { text: true, reloadTiles: true });
  bind("wm-sea-url", "seaUrl", { text: true, reloadTiles: true });
  bind("wm-credit", "credit", { text: true });
  $("wm-clear").onclick = async () => {
    try { await invoke("webmap_clear_cache"); reload(); } catch (e) { status(String(e), "bad"); return; }
    cacheLine();
  };

  sync();
  show();
  apply();
}
