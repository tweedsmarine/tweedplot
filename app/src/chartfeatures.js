// LINZ hydrographic features on the chart: lights (with sectors), buoys, beacons,
// rocks, wrecks, obstructions, restricted areas, marine farms, cables, leading
// lines, names. The tiles come from the Rust chart store (layers fpt, fln, far,
// lsec); symbols and labels are drawn here on demand, so nothing needs fetching.
// Click a feature for all its details.
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";

const MAGENTA = "#b5359a";
const INK = "#1d2a33";
const COLOURS = { 1: "#ffffff", 2: "#111111", 3: "#d7262b", 4: "#1f9a3a", 5: "#2a62c9", 6: "#f4c20d", 7: "#8a8f94", 8: "#7a5230", 9: "#f0a020", 10: "#7f3fbf", 11: "#f07c18", 12: MAGENTA, 13: "#f39ac0" };
const LIGHT = { r: "#d7262b", g: "#1f9a3a", w: "#f4c20d", o: MAGENTA, m: MAGENTA };

const KEY = "racenav.chartfeatures";
const DEFAULTS = { show: true, sectors: true, names: true, avoidDangers: true, avoidFarms: true };

/** Point symbols; underwater dangers deeper than the safety contour show as their depth only. */
const pointFilter = (contour) => ["all", minz, ["!", ["in", ["get", "cls"], ["literal", ["LIGHTS", "SEAARE", "LNDRGN"]]]],
  ["any", ["!", ["has", "dep"]], ["<", ["get", "dep"], contour]]];

const labelFilter = (contour) => ["all", minz, ["has", "lbl"], ["!", ["in", ["get", "cls"], ["literal", ["SEAARE", "LNDRGN"]]]],
  ["any", ["!", ["has", "dep"]], ["<", ["get", "dep"], contour]]];

/** Zoom filter: a feature shows from its SCAMIN zoom. */
const minz = ["any", ["!", ["has", "minz"]], [">=", ["zoom"], ["get", "minz"]]];

export function setupChartFeatures(map, { before, safetyContour = 5 }) {
  const $ = (id) => document.getElementById(id);
  const opts = { ...DEFAULTS };
  try { Object.assign(opts, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(opts)); } catch { /* ignore */ } };

  const src = { source: "chart" };
  const cls = (...c) => ["in", ["get", "cls"], ["literal", c]];
  const layers = [
    // areas
    { id: "f-area-fill", type: "fill", ...src, "source-layer": "far", filter: ["all", minz, ["any", cls("MARCUL", "OBSTRN"), ["==", ["get", "entry"], 1]]],
      paint: { "fill-color": ["match", ["get", "cls"], "OBSTRN", "#7fb8e8", MAGENTA], "fill-opacity": ["match", ["get", "cls"], "OBSTRN", 0.5, 0.08] } },
    { id: "f-area-hit", type: "fill", ...src, "source-layer": "far", filter: ["all", minz, ["!", cls("SEAARE")]],
      paint: { "fill-color": "#000", "fill-opacity": 0.01 } },
    { id: "f-area-dash", type: "line", ...src, "source-layer": "far", filter: ["all", minz, cls("RESARE", "ACHARE", "MARCUL", "CBLARE", "CTNARE", "PRCARE")],
      paint: { "line-color": MAGENTA, "line-width": ["match", ["get", "cls"], "RESARE", 1.4, 1], "line-dasharray": [4, 2.5] } },
    { id: "f-area-grey", type: "line", ...src, "source-layer": "far", filter: ["all", minz, cls("DMPGRD", "FERYRT", "OBSTRN")],
      paint: { "line-color": "#5b6b75", "line-width": 0.9, "line-dasharray": [2, 2] } },
    // lines
    { id: "f-line-mag", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("CBLSUB", "PIPSOL", "RDOCAL")],
      paint: { "line-color": MAGENTA, "line-width": 1.1, "line-dasharray": [3, 2] } },
    { id: "f-line-solid", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("NAVLNE")],
      paint: { "line-color": INK, "line-width": 1 } },
    { id: "f-line-dash", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("RECTRC", "CBLOHD", "FSHFAC")],
      paint: { "line-color": ["match", ["get", "cls"], "RECTRC", INK, "#5b6b75"], "line-width": 1, "line-dasharray": [5, 3] } },
    // light sectors
    { id: "f-sec-leg", type: "line", ...src, "source-layer": "lsec", filter: ["all", minz, ["==", ["get", "k"], "leg"]],
      paint: { "line-color": INK, "line-width": 0.7, "line-dasharray": [4, 3] } },
    { id: "f-sec-arc", type: "line", ...src, "source-layer": "lsec", filter: ["all", minz, ["==", ["get", "k"], "arc"]],
      paint: { "line-color": ["match", ["get", "col"], "r", LIGHT.r, "g", LIGHT.g, "w", LIGHT.w, MAGENTA], "line-width": 4, "line-opacity": 0.9 } },
    // names of sea areas and bays
    { id: "f-names", type: "symbol", ...src, "source-layer": "far", filter: ["all", cls("SEAARE"), ["has", "name"]], minzoom: 9,
      layout: { "icon-image": ["concat", "txt:n:", ["get", "name"]], "icon-allow-overlap": false, "icon-padding": 4, "symbol-placement": "point" } },
    { id: "f-pnames", type: "symbol", ...src, "source-layer": "fpt", filter: ["all", cls("SEAARE", "LNDRGN"), ["has", "lbl"]], minzoom: 9,
      layout: { "icon-image": ["concat", "txt:n:", ["get", "lbl"]], "icon-allow-overlap": false, "icon-padding": 4 } },
    // point symbols: lights under the marks they sit on
    { id: "f-lights", type: "symbol", ...src, "source-layer": "fpt", filter: ["all", minz, cls("LIGHTS")],
      layout: { "icon-image": ["concat", "s57:", ["get", "icon"]], "icon-anchor": "top-left", "icon-offset": [-2, -2],
                "icon-allow-overlap": true, "icon-ignore-placement": true } },
    { id: "f-points", type: "symbol", ...src, "source-layer": "fpt", filter: pointFilter(safetyContour),
      layout: { "icon-image": ["concat", "s57:", ["get", "icon"]],
                "icon-anchor": ["match", ["slice", ["get", "icon"], 0, 2], ["b|", "c|"], "bottom", "center"],
                "icon-allow-overlap": true, "icon-ignore-placement": true,
                "symbol-sort-key": ["match", ["get", "cls"], ["UWTROC", "WRECKS", "OBSTRN"], 2, 1] } },
    // labels: light characteristics, names of marks, depths over dangers
    { id: "f-labels", type: "symbol", ...src, "source-layer": "fpt", filter: labelFilter(safetyContour),
      layout: { "icon-image": ["concat", "txt:", ["match", ["get", "cls"], "LIGHTS", "l", ["UWTROC", "WRECKS", "OBSTRN"], "d", "b"], ":", ["get", "lbl"]],
                "icon-anchor": "left", "icon-offset": ["match", ["get", "cls"], "LIGHTS", ["literal", [12, 8]], ["literal", [8, -6]]],
                "icon-allow-overlap": false, "icon-padding": 1,
                "symbol-sort-key": ["match", ["get", "cls"], "LIGHTS", 0, 1] } },
  ];
  for (const l of layers) map.addLayer(l, before);
  // labels only once there is room
  map.setLayerZoomRange("f-labels", 11, 24);

  const SECTOR = ["f-sec-leg", "f-sec-arc"];
  const NAMES = ["f-names", "f-pnames", "f-labels"];
  function apply() {
    for (const l of layers) {
      let on = opts.show;
      if (SECTOR.includes(l.id)) on = on && opts.sectors;
      if (NAMES.includes(l.id)) on = on && opts.names;
      map.setLayoutProperty(l.id, "visibility", on ? "visible" : "none");
    }
  }
  for (const [id, key] of [["set-feat", "show"], ["set-sectors", "sectors"], ["set-featnames", "names"], ["set-avoid-dangers", "avoidDangers"], ["set-avoid-farms", "avoidFarms"]]) {
    const el = $(id);
    if (!el) continue;
    el.checked = opts[key];
    el.onchange = () => { opts[key] = el.checked; save(); apply(); };
  }
  apply();

  // ---------- details on a click ----------------------------------------------------------
  const POINTS = ["f-points", "f-lights"];
  const LINES = ["f-line-mag", "f-line-solid", "f-line-dash"];
  let popup = null;
  map.on("click", async (e) => {
    if (!opts.show || document.querySelector(".armed")) return;
    // leave clicks on the app's own things (course, routings, fleet, marks) to them
    const pad = 6;
    const box = [[e.point.x - pad, e.point.y - pad], [e.point.x + pad, e.point.y + pad]];
    const own = map.queryRenderedFeatures(box).some((f) => /^(rt-|lib-|fl-|dl-|boat)/.test(f.layer.id));
    if (own) return;
    const hit = map.queryRenderedFeatures(box, { layers: POINTS }).concat(
      map.queryRenderedFeatures(box, { layers: LINES }), map.queryRenderedFeatures(e.point, { layers: ["f-area-hit"] }));
    if (!hit.length) return;
    // several features at one spot (a beacon with its light): show them all
    const seen = new Set();
    const list = hit.filter((f) => { const k = `${f.properties.t}/${f.properties.fid}`; if (seen.has(k)) return false; seen.add(k); return true; }).slice(0, 4);
    const infos = (await Promise.all(list.map((f) => invoke("chart_feature", { t: f.properties.t, fid: f.properties.fid }).catch(() => null)))).filter(Boolean);
    if (!infos.length) return;
    popup?.remove();
    const el = document.createElement("div");
    el.className = "feat-pop";
    for (const info of infos) {
      const h = document.createElement("h4");
      const name = info.attrs.find(([k]) => k === "Name")?.[1];
      h.textContent = name ? `${info.name}: ${name}` : info.name;
      el.append(h);
      const tbl = document.createElement("table");
      for (const [k, v] of info.attrs) {
        if (k === "Name") continue;
        const tr = tbl.insertRow();
        tr.insertCell().textContent = k;
        tr.insertCell().textContent = v;
      }
      el.append(tbl);
    }
    const note = document.createElement("div");
    note.className = "wx-note";
    note.textContent = "LINZ chart data, CC BY 4.0. Not for navigation.";
    el.append(note);
    popup = new maplibregl.Popup({ maxWidth: "340px", closeButton: true }).setLngLat(e.lngLat).setDOMContent(el).addTo(map);
  });
  for (const id of [...POINTS, ...LINES]) {
    map.on("mouseenter", id, () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = "pointer"; });
    map.on("mouseleave", id, () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = ""; });
  }

  return {
    /** Underwater rocks, wrecks and obstructions deeper than this lose their danger symbol. */
    setSafetyContour: (m) => { map.setFilter("f-points", pointFilter(m)); map.setFilter("f-labels", labelFilter(m)); },
    /** Routing options: which charted hazards to keep off. */
    avoid: () => ({ dangers: opts.avoidDangers, farms: opts.avoidFarms }),
  };
}

// ---------- symbols ----------------------------------------------------------
// Drawn at twice the size (pixelRatio 2). Simplified paper-chart (INT 1) symbols.

function canvas(w, h) {
  const c = document.createElement("canvas");
  c.width = w; c.height = h;
  const g = c.getContext("2d");
  g.lineJoin = "round"; g.lineCap = "round";
  return [c, g];
}
const colourList = (s) => (s || "").split(",").map((x) => COLOURS[Number(x)]).filter(Boolean);

/** Fill a path with the colours of a mark: horizontal bands, vertical stripes, or one colour. */
function paint(g, path, cols, pat, bbox) {
  const [x0, y0, x1, y1] = bbox;
  g.save();
  g.clip(path);
  if (!cols.length) cols = ["#ffffff"];
  const n = cols.length;
  for (let i = 0; i < n; i++) {
    g.fillStyle = cols[i];
    if (pat === "2" || pat === "3") g.fillRect(x0 + ((x1 - x0) * i) / n, y0, (x1 - x0) / n + 0.5, y1 - y0);
    else if (pat === "1" || n > 1) g.fillRect(x0, y0 + ((y1 - y0) * i) / n, x1 - x0, (y1 - y0) / n + 0.5);
    else g.fillRect(x0, y0, x1 - x0, y1 - y0);
  }
  g.restore();
  g.strokeStyle = INK; g.lineWidth = 2.2;
  g.stroke(path);
}

function cone(g, cx, cy, s, up, fill) {
  g.beginPath();
  if (up) { g.moveTo(cx, cy - s); g.lineTo(cx + s * 0.85, cy + s * 0.5); g.lineTo(cx - s * 0.85, cy + s * 0.5); }
  else { g.moveTo(cx, cy + s); g.lineTo(cx + s * 0.85, cy - s * 0.5); g.lineTo(cx - s * 0.85, cy - s * 0.5); }
  g.closePath();
  g.fillStyle = fill; g.fill();
}

/** Topmark centred at (cx, cy), about 22 px tall. */
function topmark(g, top, cx, cy) {
  const black = "#111";
  switch (top) {
    case "n": cone(g, cx, cy - 6, 6, true, black); cone(g, cx, cy + 7, 6, true, black); break;
    case "s": cone(g, cx, cy - 7, 6, false, black); cone(g, cx, cy + 6, 6, false, black); break;
    case "e": cone(g, cx, cy - 7, 6, true, black); cone(g, cx, cy + 7, 6, false, black); break;
    case "w": cone(g, cx, cy - 6, 6, false, black); cone(g, cx, cy + 6, 6, true, black); break;
    case "isd": for (const dy of [-6, 6]) { g.beginPath(); g.arc(cx, cy + dy, 5, 0, 7); g.fillStyle = black; g.fill(); } break;
    case "saw": g.beginPath(); g.arc(cx, cy, 6, 0, 7); g.fillStyle = COLOURS[3]; g.fill(); g.strokeStyle = INK; g.lineWidth = 1.5; g.stroke(); break;
    case "spp": g.strokeStyle = COLOURS[6]; g.lineWidth = 5; g.beginPath(); g.moveTo(cx - 6, cy - 6); g.lineTo(cx + 6, cy + 6); g.moveTo(cx + 6, cy - 6); g.lineTo(cx - 6, cy + 6); g.stroke();
      g.strokeStyle = INK; g.lineWidth = 1; g.stroke(); break;
    case "can": g.fillStyle = COLOURS[3]; g.fillRect(cx - 6, cy - 6, 12, 12); g.strokeStyle = INK; g.lineWidth = 1.5; g.strokeRect(cx - 6, cy - 6, 12, 12); break;
    case "cone": cone(g, cx, cy, 8, true, COLOURS[4]); g.strokeStyle = INK; g.lineWidth = 1.5; g.stroke(); break;
    default: break;
  }
}

function buoy(shape, colours, pat, top) {
  const W = 48, H = 72;
  const [c, g] = canvas(W, H);
  const cols = colourList(colours);
  const bx = W / 2, by = H - 5; // position: centre of the small base circle
  const p = new Path2D();
  // buoys lean, as on paper charts
  switch (shape) {
    case "1": p.moveTo(bx - 13, by - 2); p.lineTo(bx + 11, by - 2); p.lineTo(bx + 3, by - 30); p.closePath(); break; // conical
    case "2": p.moveTo(bx - 12, by - 2); p.lineTo(bx + 11, by - 2); p.lineTo(bx + 15, by - 24); p.lineTo(bx - 6, by - 26); p.closePath(); break; // can
    case "3": p.ellipse(bx, by - 13, 12, 11, 0, 0, Math.PI * 2); break; // spherical
    case "5": p.moveTo(bx - 4, by - 2); p.lineTo(bx + 4, by - 2); p.lineTo(bx + 12, by - 38); p.lineTo(bx + 6, by - 39); p.closePath(); break; // spar
    case "6": p.ellipse(bx, by - 12, 15, 9, -0.2, 0, Math.PI * 2); break; // barrel
    case "7": p.moveTo(bx - 18, by - 2); p.lineTo(bx + 18, by - 2); p.lineTo(bx + 14, by - 18); p.lineTo(bx - 14, by - 18); p.closePath(); break; // super-buoy
    default: p.moveTo(bx - 11, by - 2); p.lineTo(bx + 11, by - 2); p.lineTo(bx + 9, by - 36); p.lineTo(bx + 1, by - 37); p.closePath(); // pillar
  }
  paint(g, p, cols, pat, [0, 0, W, H]);
  // waterline and position circle
  g.strokeStyle = INK; g.lineWidth = 2;
  g.beginPath(); g.moveTo(bx - 17, by); g.lineTo(bx - 6, by); g.moveTo(bx + 6, by); g.lineTo(bx + 17, by); g.stroke();
  g.beginPath(); g.arc(bx, by, 3.5, 0, 7); g.stroke();
  if (top) topmark(g, top, bx + (shape === "3" || shape === "6" ? 0 : 6), shape === "5" ? by - 52 : by - 50);
  return g.getImageData(0, 0, W, H);
}

function beacon(shape, colours, pat, top) {
  const W = 44, H = 76;
  const [c, g] = canvas(W, H);
  const cols = colourList(colours);
  const bx = W / 2, by = H - 5;
  const p = new Path2D();
  if (shape === "3" || shape === "4" || shape === "6") { p.moveTo(bx - 10, by - 2); p.lineTo(bx + 10, by - 2); p.lineTo(bx + 5, by - 34); p.lineTo(bx - 5, by - 34); p.closePath(); }
  else { p.rect(bx - 3.5, by - 38, 7, 36); } // stake, pole, pile
  paint(g, p, cols.length ? cols : ["#111111"], pat, [0, 0, W, H]);
  g.strokeStyle = INK; g.lineWidth = 2;
  g.beginPath(); g.moveTo(bx - 12, by); g.lineTo(bx + 12, by); g.stroke();
  if (top) topmark(g, top, bx, by - 50);
  return g.getImageData(0, 0, W, H);
}

function light(col) {
  // the flare: a teardrop from the light's position towards the lower right
  const W = 56, H = 56;
  const [c, g] = canvas(W, H);
  g.translate(4, 4);
  g.rotate(Math.PI / 4);
  g.beginPath();
  g.moveTo(0, 0);
  g.bezierCurveTo(10, -8, 40, -8, 46, 0);
  g.bezierCurveTo(40, 8, 10, 8, 0, 0);
  g.fillStyle = LIGHT[col] || MAGENTA; g.fill();
  g.strokeStyle = INK; g.lineWidth = 1.5; g.stroke();
  return g.getImageData(0, 0, W, H);
}

function rock(watlev) {
  const [c, g] = canvas(30, 30);
  g.strokeStyle = INK; g.lineWidth = 2.4; g.fillStyle = INK;
  const cx = 15, cy = 15;
  if (watlev === "4" || watlev === "1") { // covers and uncovers: asterisk
    for (let i = 0; i < 4; i++) { const a = (i * Math.PI) / 4; g.beginPath(); g.moveTo(cx - 9 * Math.cos(a), cy - 9 * Math.sin(a)); g.lineTo(cx + 9 * Math.cos(a), cy + 9 * Math.sin(a)); g.stroke(); }
  } else if (watlev === "5") { // awash: cross with dots
    g.beginPath(); g.moveTo(cx - 9, cy); g.lineTo(cx + 9, cy); g.moveTo(cx, cy - 9); g.lineTo(cx, cy + 9); g.stroke();
    for (const [dx, dy] of [[-6, -6], [6, -6], [-6, 6], [6, 6]]) { g.beginPath(); g.arc(cx + dx, cy + dy, 1.8, 0, 7); g.fill(); }
  } else if (watlev === "2") { // always dry
    g.beginPath(); g.arc(cx, cy, 6, 0, 7); g.fill();
  } else { // underwater: cross in a dotted danger ring
    g.beginPath(); g.moveTo(cx - 6, cy); g.lineTo(cx + 6, cy); g.moveTo(cx, cy - 6); g.lineTo(cx, cy + 6); g.stroke();
    dottedRing(g, cx, cy, 12);
  }
  return g.getImageData(0, 0, 30, 30);
}

function dottedRing(g, cx, cy, r) {
  g.save(); g.setLineDash([2, 3]); g.lineWidth = 1.6; g.strokeStyle = INK;
  g.beginPath(); g.arc(cx, cy, r, 0, 7); g.stroke(); g.restore();
}

function wreck(cat) {
  const [c, g] = canvas(44, 30);
  g.strokeStyle = INK; g.fillStyle = INK; g.lineWidth = 2.2;
  const cx = 22, cy = 15;
  if (cat === "5") { // hull showing
    g.beginPath(); g.moveTo(cx - 14, cy - 2); g.lineTo(cx + 14, cy - 2); g.lineTo(cx + 9, cy + 5); g.lineTo(cx - 9, cy + 5); g.closePath(); g.fill();
  } else {
    g.beginPath(); g.moveTo(cx - 14, cy); g.lineTo(cx + 14, cy);
    for (const dx of [-6, 0, 6]) { g.moveTo(cx + dx, cy - 6); g.lineTo(cx + dx, cy + 6); }
    if (cat === "4") { g.moveTo(cx + 3, cy); g.lineTo(cx + 8, cy - 10); }
    g.stroke();
    if (cat !== "1" && cat !== "3") { g.save(); g.setLineDash([2, 3]); g.lineWidth = 1.6; g.beginPath(); g.ellipse(cx, cy, 20, 12, 0, 0, 7); g.stroke(); g.restore(); }
  }
  return g.getImageData(0, 0, 44, 30);
}

function obstruction() {
  const [c, g] = canvas(30, 30);
  g.fillStyle = "rgba(127,184,232,0.6)"; g.beginPath(); g.arc(15, 15, 11, 0, 7); g.fill();
  dottedRing(g, 15, 15, 11);
  return g.getImageData(0, 0, 30, 30);
}

function small(kind) {
  const [c, g] = canvas(30, 30);
  const cx = 15, cy = 15;
  g.lineWidth = 2; g.strokeStyle = INK; g.fillStyle = INK;
  switch (kind) {
    case "lm": g.beginPath(); g.arc(cx, cy, 7, 0, 7); g.stroke(); g.beginPath(); g.arc(cx, cy, 2, 0, 7); g.fill(); break;
    case "rc": g.strokeStyle = MAGENTA; g.lineWidth = 2.5; g.beginPath(); g.arc(cx, cy, 11, 0, 7); g.stroke(); break;
    case "pb": g.strokeStyle = MAGENTA; g.beginPath(); g.arc(cx, cy, 10, 0, 7); g.stroke(); g.beginPath(); g.moveTo(cx, cy - 6); g.lineTo(cx + 5, cy); g.lineTo(cx, cy + 6); g.lineTo(cx - 5, cy); g.closePath(); g.fillStyle = MAGENTA; g.fill(); break;
    case "an": case "ab": anchor(g, cx, cy, kind === "an" ? 1 : 0.75); break;
    case "mo": g.beginPath(); g.arc(cx, cy + 3, 5, 0, 7); g.fill(); g.beginPath(); g.moveTo(cx, cy - 2); g.lineTo(cx, cy - 9); g.stroke(); break;
    case "pi": g.beginPath(); g.arc(cx, cy, 3, 0, 7); g.fill(); break;
    case "pf": g.strokeRect(cx - 6, cy - 6, 12, 12); g.beginPath(); g.arc(cx, cy, 2, 0, 7); g.fill(); break;
    case "mf": g.strokeStyle = MAGENTA; g.strokeRect(cx - 8, cy - 6, 16, 12); g.beginPath(); g.moveTo(cx - 8, cy); g.lineTo(cx + 8, cy); g.moveTo(cx, cy - 6); g.lineTo(cx, cy + 6); g.stroke(); break;
    case "ca": g.fillStyle = MAGENTA; g.font = "bold 24px sans-serif"; g.textAlign = "center"; g.textBaseline = "middle"; g.fillText("!", cx, cy + 1); break;
    case "cp": g.strokeStyle = MAGENTA; g.beginPath(); g.arc(cx, cy, 9, 0, 7); g.stroke(); g.beginPath(); g.moveTo(cx - 5, cy); g.lineTo(cx + 5, cy); g.lineTo(cx + 1, cy - 4); g.stroke(); break;
    case "rs": g.fillStyle = COLOURS[3]; g.fillRect(cx - 2.5, cy - 8, 5, 16); g.fillRect(cx - 8, cy - 2.5, 16, 5); break;
    case "ss": g.beginPath(); g.arc(cx, cy + 4, 4, 0, 7); g.stroke(); g.beginPath(); g.moveTo(cx, cy); g.lineTo(cx, cy - 11); g.lineTo(cx + 8, cy - 8); g.lineTo(cx, cy - 5); g.stroke(); break;
    case "hf": anchor(g, cx, cy, 0.7, "#5b6b75"); break;
    case "fg": g.strokeStyle = MAGENTA; for (const r of [5, 9, 13]) { g.beginPath(); g.arc(cx - 6, cy, r, -0.6, 0.6); g.stroke(); } break;
    default: g.beginPath(); g.arc(cx, cy, 3, 0, 7); g.fill();
  }
  return g.getImageData(0, 0, 30, 30);
}

function anchor(g, cx, cy, k, colour = MAGENTA) {
  g.save(); g.translate(cx, cy); g.scale(k, k);
  g.strokeStyle = colour;
  g.lineWidth = 2.4;
  g.beginPath(); g.arc(0, -9, 2.5, 0, 7); g.moveTo(0, -6.5); g.lineTo(0, 9); g.moveTo(-6, -3); g.lineTo(6, -3);
  g.moveTo(-9, 3); g.quadraticCurveTo(-7, 9, 0, 9); g.quadraticCurveTo(7, 9, 9, 3); g.stroke();
  g.restore();
}

function daymark(colours) {
  const [c, g] = canvas(26, 26);
  const p = new Path2D(); p.rect(6, 4, 14, 14);
  paint(g, p, colourList(colours), "", [0, 0, 26, 26]);
  return g.getImageData(0, 0, 26, 26);
}

/** Text drawn as an image: l = light, b = mark name, d = depth, n = area name. */
function text(style, t) {
  const fonts = {
    l: `600 19px "Segoe UI", system-ui, sans-serif`,
    b: `italic 18px "Segoe UI", system-ui, sans-serif`,
    d: `600 19px "Segoe UI", system-ui, sans-serif`,
    n: `italic 600 21px "Segoe UI", system-ui, sans-serif`,
  };
  const ink = { l: MAGENTA, b: INK, d: INK, n: "#2d5f8a" };
  const [c0, g0] = canvas(1, 1);
  g0.font = fonts[style];
  const w = Math.ceil(g0.measureText(t).width) + 10;
  const [c, g] = canvas(w, 28);
  g.font = fonts[style];
  g.textBaseline = "middle";
  g.lineWidth = 4.5; g.strokeStyle = "rgba(255,255,255,0.9)";
  g.strokeText(t, 5, 14);
  g.fillStyle = ink[style];
  g.fillText(t, 5, 14);
  return g.getImageData(0, 0, w, 28);
}

/** Image for an `s57:` icon id or a `txt:` label id, or null. */
export function featureImage(id) {
  if (id.startsWith("txt:")) {
    const style = id[4];
    const t = id.slice(6);
    return t ? text(style, t) : null;
  }
  if (!id.startsWith("s57:")) return null;
  const [k, a, b, c, d] = id.slice(4).split("|");
  switch (k) {
    case "b": return buoy(a, b, c, d);
    case "c": return beacon(a, b, c, d);
    case "l": return light(a);
    case "r": return rock(a);
    case "w": return wreck(a);
    case "o": return obstruction();
    case "d": return daymark(a);
    default: return small(k);
  }
}
