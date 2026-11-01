// Tidal streams from the LINZ charts (Weather and tides › Tides › Tidal streams):
// each charted tidal diamond (S-57 TS_PAD) with the stream it predicts now, worked out
// in Rust from the reference port's high water (tidal_streams), and the charted flood
// and ebb arrows (TS_FEB). The predicted stream is drawn as IMO SN.1/Circ.243 asks: a
// single long-dashed line with three arrowheads, labelled with the rate and the time
// (hours from high water at the reference port), on opposite sides.
import { placing } from "./modes.js";
import { invoke } from "@tauri-apps/api/core";
import { infoBox } from "./infobox.js";

const MAGENTA = "#b5359a";
const MINZOOM = 9;
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** Images: "tsd:<letter>" the chart's tidal diamond; "tsa" the predicted stream (pointing
 *  up, from the bottom centre); "tsf:1" flood and "tsf:2" ebb arrows (pointing up). */
export function tidalImage(id) {
  const R = 2;
  if (id.startsWith("tsd:")) {
    const letter = id.slice(4) || "?";
    const S = 26, c = document.createElement("canvas");
    c.width = c.height = S * R;
    const g = c.getContext("2d"); g.scale(R, R);
    const m = S / 2, h = 10.5;
    const d = () => { g.beginPath(); g.moveTo(m, m - h); g.lineTo(m + h, m); g.lineTo(m, m + h); g.lineTo(m - h, m); g.closePath(); };
    g.lineJoin = "round";
    d(); g.fillStyle = "rgba(255,255,255,0.9)"; g.fill(); g.lineWidth = 3.5; g.strokeStyle = "rgba(255,255,255,0.9)"; g.stroke();
    d(); g.lineWidth = 1.4; g.strokeStyle = MAGENTA; g.stroke();
    g.fillStyle = MAGENTA; g.font = `bold ${letter.length > 1 ? 8 : 10}px "Segoe UI", system-ui, sans-serif`;
    g.textAlign = "center"; g.textBaseline = "middle"; g.fillText(letter.slice(0, 2), m, m + 0.5);
    return g.getImageData(0, 0, c.width, c.height);
  }
  if (id === "tsa" || id.startsWith("tsf:")) {
    const W = 22, H = 2 * 46, c = document.createElement("canvas"); // drawn up from the centre
    c.width = W * R; c.height = H * R;
    const g = c.getContext("2d"); g.scale(R, R);
    const cx = W / 2, y0 = H / 2 - 8, y1 = 4; // leaves the diamond, runs up
    const head = (y) => { g.moveTo(cx - 5, y + 6); g.lineTo(cx, y); g.lineTo(cx + 5, y + 6); };
    const draw = (col, w, dash) => {
      g.strokeStyle = col; g.lineWidth = w; g.lineCap = "round"; g.lineJoin = "round";
      g.setLineDash(dash); g.beginPath(); g.moveTo(cx, y0); g.lineTo(cx, y1); g.stroke(); g.setLineDash([]);
      g.beginPath();
      if (id === "tsa") { head(y1); head(y1 + 8); head(y1 + 16); }
      else {
        head(y1);
        if (id === "tsf:1") { for (const y of [y0 - 6, y0 - 1]) { g.moveTo(cx - 4, y + 4); g.lineTo(cx, y); g.lineTo(cx + 4, y + 4); } } // flood: feathered tail
      }
      g.stroke();
    };
    draw("rgba(255,255,255,0.9)", 4.5, id === "tsa" ? [7, 4] : []);
    draw(MAGENTA, 1.6, id === "tsa" ? [7, 4] : []);
    return g.getImageData(0, 0, c.width, c.height);
  }
  return null;
}

export function setupTidalStreams(map, { before, getTime, onTime, isOn, status = () => {} }) {
  const empty = { type: "FeatureCollection", features: [] };
  map.addSource("tstreams", { type: "geojson", data: empty });
  const src = { source: "tstreams" };
  map.addLayer({ id: "ts-feb", type: "symbol", ...src, minzoom: MINZOOM, filter: ["==", ["get", "k"], "feb"],
    layout: { "icon-image": ["concat", "tsf:", ["get", "cat"]], "icon-rotate": ["get", "dir"], "icon-rotation-alignment": "map", "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);
  map.addLayer({ id: "ts-feb-lbl", type: "symbol", ...src, minzoom: MINZOOM + 1, filter: ["all", ["==", ["get", "k"], "feb"], ["has", "lbl"]],
    layout: { "icon-image": ["concat", "txt:l:", ["get", "lbl"]], "icon-anchor": "left", "icon-offset": [8, 0], "icon-allow-overlap": false } }, before);
  map.addLayer({ id: "ts-arrow", type: "symbol", ...src, minzoom: MINZOOM, filter: ["all", ["==", ["get", "k"], "pad"], ["has", "dir"]],
    layout: { "icon-image": "tsa", "icon-rotate": ["get", "dir"], "icon-rotation-alignment": "map", "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);
  map.addLayer({ id: "ts-diamond", type: "symbol", ...src, minzoom: MINZOOM, filter: ["==", ["get", "k"], "pad"],
    layout: { "icon-image": ["concat", "tsd:", ["get", "name"]], "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);
  map.addLayer({ id: "ts-rate", type: "symbol", ...src, minzoom: MINZOOM, filter: ["all", ["==", ["get", "k"], "pad"], ["has", "rate"]],
    layout: { "icon-image": ["concat", "txt:l:", ["get", "rate"]], "icon-anchor": "left", "icon-offset": [12, 0], "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);
  map.addLayer({ id: "ts-time", type: "symbol", ...src, minzoom: MINZOOM, filter: ["all", ["==", ["get", "k"], "pad"], ["has", "when"]],
    layout: { "icon-image": ["concat", "txt:l:", ["get", "when"]], "icon-anchor": "right", "icon-offset": [-12, 0], "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);
  const LAYERS = ["ts-feb", "ts-feb-lbl", "ts-arrow", "ts-diamond", "ts-rate", "ts-time"];

  let panels = [];
  // how many diamonds the charts have at all, and roughly where (once the charts are open)
  let counted = false;
  async function count() {
    if (counted || !isOn()) return;
    try {
      const r = await invoke("tidal_streams", { bbox: [160, -53, 190, -28], t: Math.floor(getTime()) });
      const ps = r.panels || [];
      if (!ps.length && !(r.arrows || []).length) { status("No tidal diamonds in the chart data: they come from LINZ's \"Tidal stream panel data points\" layers (in the hydrographic downloads)."); return; }
      counted = true;
      const refs = [...new Set(ps.map((p) => p.ref_name).filter(Boolean))].sort();
      status(`${ps.length} tidal diamonds and ${(r.arrows || []).length} flood/ebb arrows in the charts${refs.length ? `, from high water at ${refs.join(", ")}` : ""}. Zoom in near those ports (from about 1:500,000) to see them.`);
    } catch { /* charts not ready */ }
  }
  let busy = false, again = false;
  async function refresh() {
    const on = isOn();
    for (const l of LAYERS) map.setLayoutProperty(l, "visibility", on ? "visible" : "none");
    if (!on || map.getZoom() < MINZOOM - 0.5) return;
    if (busy) { again = true; return; }
    busy = true;
    try {
      const b = map.getBounds();
      const r = await invoke("tidal_streams", { bbox: [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()], t: Math.floor(getTime()) });
      panels = r.panels || [];
      const f = [];
      panels.forEach((p, i) => {
        const props = { k: "pad", i, name: p.name || "?" };
        if (p.dir != null && p.rate != null) {
          if (p.rate >= 0.1) { props.dir = p.dir; props.rate = `${p.rate.toFixed(1)} kn`; }
          else props.rate = "slack";
          const h = Math.round(p.hour);
          props.when = `${p.rel || "HW"}${h === 0 ? "" : h > 0 ? `+${h}` : `−${-h}`}`;
        }
        f.push({ type: "Feature", properties: props, geometry: { type: "Point", coordinates: [p.lon, p.lat] } });
      });
      for (const a of r.arrows || []) {
        const props = { k: "feb", cat: a.cat === 1 ? 1 : 2, dir: a.dir };
        if (a.rate != null) props.lbl = `${a.rate.toFixed(1)} kn`;
        f.push({ type: "Feature", properties: props, geometry: { type: "Point", coordinates: [a.lon, a.lat] } });
      }
      map.getSource("tstreams").setData({ type: "FeatureCollection", features: f });
      if (box.isOpen && shown != null) showPanel(shown, false);
    } catch { /* charts not ready */ }
    busy = false;
    if (again) { again = false; refresh(); }
  }

  // ---------- a diamond's table in an info window ----------------------------------
  const box = infoBox(map, "tidal", { width: 330, height: 430 });
  let shown = null; // index into panels
  const fmtT = (t) => new Date(t * 1000).toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit" });
  function showPanel(i, open = true) {
    const p = panels[i];
    if (!p) return;
    shown = i;
    const cur = p.hour != null ? Math.round(p.hour) : null;
    const rows = p.hours.map(([d, r], k) => {
      const h = k - 6;
      return `<tr${h === cur ? ' class="on"' : ""}><td>${p.rel}${h === 0 ? "" : h > 0 ? "+" + h : "−" + -h}</td><td>${String(Math.round(d)).padStart(3, "0")}°</td><td>${r.toFixed(1)} kn</td></tr>`;
    }).join("");
    const now = p.rate != null
      ? `<p><b>Now: ${p.rate < 0.1 ? "slack" : `${p.rate.toFixed(1)} kn setting ${String(Math.round(p.dir)).padStart(3, "0")}°T`}</b><br>
         ${p.hour >= 0 ? `${p.hour.toFixed(1)} h after` : `${(-p.hour).toFixed(1)} h before`} ${p.rel === "LW" ? "low" : "high"} water at ${esc(p.ref_name)} (${fmtT(p.ref_t)})${p.factor != null ? `; spring rates × ${p.factor.toFixed(2)} for this tide's range` : ""}.</p>`
      : `<p class="wx-note warn">${esc(p.note || "No prediction")}</p>`;
    const html = `<div class="feat-pop ts-pop">${now}
      <table class="ts-table"><thead><tr><th>Time</th><th>Sets</th><th>Spring rate</th></tr></thead><tbody>${rows}</tbody></table>
      <div class="wx-note">Tidal diamond ${esc(p.name)} from the LINZ chart: directions the stream runs (°T) and spring rates for each hour from ${p.rel === "LW" ? "low" : "high"} water at ${esc(p.ref_name)}. Neap rates are about half; the rate now is scaled by the tide's range. Wind and pressure change streams. Not for navigation.</div></div>`;
    if (open) box.open(html, { title: `Tidal stream ${p.name}`, at: [p.lon, p.lat], onClose: () => { shown = null; } });
    else box.update(html);
  }
  for (const l of ["ts-diamond", "ts-arrow"]) {
    map.on("click", l, (e) => { if (placing()) return; const f = e.features?.[0]; if (f) showPanel(f.properties.i); });
    map.on("mouseenter", l, () => { if (!placing()) map.getCanvas().style.cursor = "pointer"; });
    map.on("mouseleave", l, () => { if (!placing()) map.getCanvas().style.cursor = ""; });
  }

  let t0 = null;
  map.on("moveend", () => { clearTimeout(t0); t0 = setTimeout(refresh, 300); });
  onTime?.(() => { clearTimeout(t0); t0 = setTimeout(refresh, 250); });
  window.addEventListener("racenav-charts", (e) => { if (e.detail?.ready !== false) { counted = false; setTimeout(() => { refresh(); count(); }, 500); } });
  setInterval(refresh, 60_000);
  refresh();
  count();
  return { refresh: () => { refresh(); count(); } };
}
