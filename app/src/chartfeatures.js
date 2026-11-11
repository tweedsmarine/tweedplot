// LINZ hydrographic features on the chart: lights (with sectors), buoys, beacons,
// rocks, wrecks, obstructions, restricted areas, marine farms, cables, leading
// lines, names. The tiles come from the Rust chart store (layers fpt, fln, far,
// lsec); symbols and labels are drawn here on demand, so nothing needs fetching.
// Click a feature for all its details.
import { placing } from "./modes.js";
import { infoBox } from "./infobox.js";
import { invoke } from "@tauri-apps/api/core";
import { linkPublications } from "./publications.js";
import { listen } from "@tauri-apps/api/event";
import { magneticVariation, fmtVariation } from "./wmm.js";

const OBJECT_NAMES = { MAGVAR: "Magnetic variation" };

const MAGENTA = "#b5359a";
const INK = "#1d2a33";
const COLOURS = { 1: "#ffffff", 2: "#111111", 3: "#d7262b", 4: "#1f9a3a", 5: "#2a62c9", 6: "#f4c20d", 7: "#8a8f94", 8: "#7a5230", 9: "#f0a020", 10: "#7f3fbf", 11: "#f07c18", 12: MAGENTA, 13: "#f39ac0" };
const LIGHT = { r: "#d7262b", g: "#1f9a3a", w: "#f4c20d", o: MAGENTA, m: MAGENTA };

const KEY = "racenav.chartfeatures";
const DEFAULTS = { screenMm: 50, cat: "all", sectors: true, names: true, soundings: true, avoidDangers: true, avoidFarms: true, symSize: 1, sym: "s52", info: false, bounds: "plain",
  textImportant: true, textOther: true, dateSym: false };
/** The settings kept separately for each view with a chart (see setup). */
const VIEW_KEYS = ["cat", "sectors", "names", "soundings", "info", "textImportant", "textOther", "dateSym"];
/** The standard display (S-Mode): one press back to it. */
const STANDARD = { cat: "standard", sectors: true, names: true, soundings: true };

/** An underwater danger with a known depth at least as deep as the safety contour. */
const deeper = (contour) => ["all", ["has", "dep"], [">=", ["get", "dep"], contour]];
/** Point symbols. Hazards show at every zoom, except underwater dangers of known depth that
 *  aren't isolated dangers (scaminDanger), which follow the chart's SCAMIN. Symbols
 *  turned with the chart (`rot`: a traffic lane's arrow) are in their own layer. */
const pointFilter = (contour) => ["all", ["!", ["in", ["get", "cls"], ["literal", ["LIGHTS", "SEAARE", "LNDRGN"]]]], ["!", ["has", "rot"]],
  ["any", ["all", HZ, ["!", scaminDanger(contour)]], minzDeep]];

const labelFilter = (contour) => ["all", ["has", "lbl"], ["!", ["in", ["get", "cls"], ["literal", ["SEAARE", "LNDRGN"]]]],
  ["any", ["all", ["!", scaminDanger(contour)], minz], minzDeep]];

/** Isolated danger (S-52 UDWHAZ05): a rock, wreck or obstruction shallower than the safety
 *  contour (or of unknown depth, drying or awash) lying in water at least as deep as the
 *  safety contour (`wd`, the depth area it is in). It gets the isolated danger symbol. With
 *  "isolated dangers in shallow water" on, also those in water from 0 m to the contour. */
let shallowDangers = false;
const isolated = (contour) => ["all", ["has", "wd"], shallowDangers ? [">=", ["get", "wd"], 0] : [">=", ["get", "wd"], contour], ["!=", ["get", "nd"], 1], ["!", deeper(contour)]];
/** Underwater dangers of known depth that aren't isolated dangers (deeper than the safety
 *  contour, or in water already shaded shallower than it) follow the chart's SCAMIN, as on
 *  an ECDIS; isolated dangers, drying and awash ones and those of unknown depth show at
 *  every zoom. */
const scaminDanger = (contour) => ["all", ["has", "dep"], ["!", isolated(contour)], ["!", ["in", "ISODGR01", ["coalesce", ["get", "sy"], ""]]]];
const pointIcon = (contour) => ["case", isolated(contour), "s57:iso", ["concat", "s57:", ["get", "icon"]]];

// ---------- IHO symbols (S-52 / S-101 Portrayal Catalogue) ----------------------------
// The chart store runs the IHO rules and puts the symbols they choose on each point
// feature (`sy`: "BOYPIL24,TOPMAR06", "LIGHTS11@135"). The catalogue's SVGs are loaded
// once; a feature's image is its symbols drawn over each other at their pivot points.
// Lines and areas: the rules' line style (`sl`, drawn `slw` mm tall), colour fill (`cf`,
// transparency `ct`), fill pattern (`af`) and display category (`vg`) are on the tile
// features; the line styles and patterns are drawn here from the catalogue's definitions.
const S52 = { ready: false, imgs: new Map(), colours: {}, lines: {}, fills: {} };
/** Pixels per millimetre for the symbols, drawn at twice the size (pixelRatio 2). */
// IHO symbols are drawn to size in millimetres (S-52): image pixels per mm (images are
// added at pixel ratio 2). 3.78 CSS px per mm is the browser's nominal 96 dpi; the
// mariner's screen calibration (Chart display) corrects it to the real screen.
let S52_PX_MM = 3.78 * 2;
/** CSS pixels per millimetre on this screen, as calibrated. */
export function s52PxPerMm() { return S52_PX_MM / 2; }
/** Each palette's symbols and styles, loaded once (725 SVGs to decode: a second or
 *  two), so switching between day, dusk and night is immediate after the first time. */
const S52_PALETTES = new Map();
export function loadS52Palette(palette = "Day") {
  if (!S52_PALETTES.has(palette)) {
    S52_PALETTES.set(palette, (async () => {
      const [list, styles] = await Promise.all([invoke("s52_symbols", { palette }), invoke("s52_styles", { palette })]);
      const imgs = new Map();
      await Promise.all(list.map(({ id, svg, vb }) => new Promise((resolve) => {
        const img = new Image();
        const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
        img.onload = () => { imgs.set(id, { img, vb }); URL.revokeObjectURL(url); resolve(); };
        img.onerror = () => { URL.revokeObjectURL(url); resolve(); };
        img.src = url;
      })));
      return { palette, imgs, colours: styles?.colours || {}, lines: styles?.lines || {}, fills: styles?.fills || {} };
    })().catch((e) => { S52_PALETTES.delete(palette); throw e; }));
  }
  return S52_PALETTES.get(palette);
}
async function loadS52Symbols(palette = "Day") {
  Object.assign(S52, await loadS52Palette(palette));
  S52.ready = S52.imgs.size > 0;
  // the others in the background, for switching to them later
  if (S52.ready) setTimeout(() => { for (const p of ["Dusk", "Night"]) loadS52Palette(p).catch(() => {}); }, 10000);
  return S52.ready;
}
/** A chart note (an ENC's text file, TXTDSC) for a feature's card: its text once read, or
 *  where to get it. */
function chartNote(name) {
  const box = document.createElement("div");
  box.className = "feat-text";
  box.textContent = `${name}…`;
  invoke("chart_text", { name }).then((r) => {
    if (r?.text != null) {
      // references to the Annual Notices to Mariners and the Almanac open them
      box.replaceChildren(...linkPublications(r.text));
      box.title = r.name;
    } else if (r && "picture" in r) {
      if (!r.picture) {
        box.className = "wx-note";
        box.textContent = `Picture ${name} can't be shown: ${r.error}`;
        return;
      }
      // a picture (PICREP), e.g. the leading marks of a harbour: click for a larger view
      box.className = "feat-pic";
      const img = document.createElement("img");
      img.src = r.picture;
      img.alt = `Chart picture ${r.name}`;
      img.title = `${r.name}: click to enlarge`;
      img.onclick = () => showPicture(r);
      box.replaceChildren(img);
    } else {
      box.className = "wx-note";
      box.textContent = `${name} is a note that comes with the ENC charts (LINZ's NZ ENC service). Chart › Get data › Chart notes adds them from an ENC folder.`;
    }
  }).catch(() => { box.className = "wx-note"; box.textContent = `${name} not available.`; });
  return box;
}

/** A chart picture over the whole window; a click or Escape closes it. */
function showPicture(r) {
  const lb = document.createElement("div");
  lb.className = "feat-lightbox";
  const img = document.createElement("img");
  img.src = r.picture;
  img.alt = r.name;
  const cap = document.createElement("p");
  cap.textContent = `${r.name}, from the ENC charts. Not for navigation.`;
  lb.append(img, cap);
  const close = () => { lb.remove(); document.removeEventListener("keydown", key, true); };
  const key = (e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); } };
  lb.onclick = close;
  document.addEventListener("keydown", key, true);
  document.body.append(lb);
}

/** The image for "s52:<sy>", or null while the symbols are loading. */
function s52Image(sy) {
  if (!S52.ready) return null;
  const parts = sy.split(",").filter(Boolean).map((p) => { const [id, a] = p.split("@"); return { s: S52.imgs.get(id), a: ((Number(a) || 0) * Math.PI) / 180 }; }).filter((p) => p.s);
  if (!parts.length) return null;
  // the canvas is centred on the pivot: as far out as any (turned) corner reaches
  let half = 1;
  for (const { s: { vb }, a } of parts) {
    for (const [x, y] of [[vb[0], vb[1]], [vb[0] + vb[2], vb[1]], [vb[0], vb[1] + vb[3]], [vb[0] + vb[2], vb[1] + vb[3]]]) {
      const rx = x * Math.cos(a) - y * Math.sin(a), ry = x * Math.sin(a) + y * Math.cos(a);
      half = Math.max(half, Math.abs(rx) * S52_PX_MM, Math.abs(ry) * S52_PX_MM);
    }
  }
  const n = Math.ceil(half) * 2 + 2;
  const c = document.createElement("canvas");
  c.width = c.height = n;
  const g = c.getContext("2d");
  for (const { s: { img, vb }, a } of parts) {
    g.save();
    g.translate(n / 2, n / 2);
    g.rotate(a);
    g.drawImage(img, vb[0] * S52_PX_MM, vb[1] * S52_PX_MM, vb[2] * S52_PX_MM, vb[3] * S52_PX_MM);
    g.restore();
  }
  return g.getImageData(0, 0, n, n);
}

/** An IHO text, "colour:size:HV:dx:dy:text" (size in points; H S/C/E start, centre,
 *  end; V T/C/B top, centre, bottom; offset in mm, y up): an image holding the feature's
 *  point and the text where the rules put it, the point at the image's anchor
 *  (`textAnchor`): a corner or edge when the text lies wholly to one side, else the
 *  middle. Plain, as S-52 has it. */
function s52TextImage(spec) {
  const f = spec.split(":");
  if (f.length < 6) return null;
  const [colour, size, hv, dxs, dys] = f;
  const t = f.slice(5).join(":");
  if (!t) return null;
  const px = Math.max(6, Number(size) * 0.3528 * S52_PX_MM); // pt → mm → image pixels
  const font = `${Math.round(px)}px "Segoe UI", system-ui, sans-serif`;
  const g0 = canvas(1, 1)[1];
  g0.font = font;
  const w = Math.ceil(g0.measureText(t).width) + 2, h = Math.ceil(px * 1.2);
  const dx = Number(dxs) || 0, dy = Number(dys) || 0;
  // the text box from the point (y down)
  const x = dx * S52_PX_MM - (hv[0] === "C" ? w / 2 : hv[0] === "E" ? w : 0);
  const y = -dy * S52_PX_MM - (hv[1] === "C" ? h / 2 : hv[1] === "B" ? h : 0);
  // along each axis: from the point when the text is wholly to one side (as textAnchor
  // reckons it), else centred on the point
  const span = (oneSide, lo, hi) => {
    if (oneSide === 1) return [0, Math.ceil(hi)];
    if (oneSide === -1) return [Math.floor(lo), 0];
    const m = Math.ceil(Math.max(Math.abs(lo), Math.abs(hi)));
    return [-m, m];
  };
  const sx = hv[0] === "S" && dx >= 0 ? 1 : hv[0] === "E" && dx <= 0 ? -1 : 0;
  const sy = hv[1] === "T" && dy <= 0 ? 1 : hv[1] === "B" && dy >= 0 ? -1 : 0;
  const [ax, bx] = span(sx, x, x + w), [ay, by] = span(sy, y, y + h);
  const [, g] = canvas(Math.max(1, bx - ax), Math.max(1, by - ay));
  g.font = font;
  g.textBaseline = "middle";
  g.fillStyle = S52.colours[colour] || S52.colours.CHBLK || "#000";
  g.fillText(t, x - ax + 1, y - ay + h / 2);
  return g.getImageData(0, 0, Math.max(1, bx - ax), Math.max(1, by - ay));
}

/** A light's S-52 sectors of fixed size on the screen (chart.rs `sec`): "L bearing mm"
 *  limits and "A radius start extent styles" arcs (radius mm, degrees clockwise from
 *  north, styles "s0.64LITRD+d0.32CHBLK": solid or dashed, width mm, colour token), as an
 *  image centred on the light, north up (the map turns it). */
function s52SectorImage(spec) {
  if (!S52.ready) return null;
  const parts = spec.split("/").map((p) => p.split(" "));
  let reach = 1;
  for (const f of parts) reach = Math.max(reach, (Number(f[f[0] === "A" ? 1 : 2]) || 0) + 1.5);
  const half = Math.ceil(reach * S52_PX_MM) + 2, n = half * 2;
  const [, g] = canvas(n, n);
  const at = (b, r) => [half + r * Math.sin((b * Math.PI) / 180), half - r * Math.cos((b * Math.PI) / 180)];
  const colour = (t) => S52.colours[t] || "#000";
  g.lineCap = "butt";
  const dash = () => g.setLineDash([3.6 * S52_PX_MM, 1.8 * S52_PX_MM]);
  // the arcs, wider styles first (the black edging under the colour)
  const strokes = [];
  for (const f of parts) {
    if (f[0] !== "A") continue;
    const [r, start, extent] = [Number(f[1]), Number(f[2]), Number(f[3])];
    for (const st of (f[4] || "").split("+").filter(Boolean)) {
      const m = /^([sd])([0-9.]+)([A-Z0-9]+)$/.exec(st);
      if (m) strokes.push({ r, start, extent, dashed: m[1] === "d", w: Number(m[2]), tok: m[3] });
    }
  }
  strokes.sort((a, b) => b.w - a.w);
  for (const s of strokes) {
    g.beginPath();
    // canvas angles run clockwise from east; a bearing clockwise from north
    const a0 = ((s.start - 90) * Math.PI) / 180, a1 = ((s.start + s.extent - 90) * Math.PI) / 180;
    g.arc(half, half, s.r * S52_PX_MM, Math.min(a0, a1), Math.max(a0, a1));
    g.lineWidth = Math.max(1, s.w * S52_PX_MM);
    g.strokeStyle = colour(s.tok);
    if (s.dashed) dash(); else g.setLineDash([]);
    g.stroke();
  }
  // the sector limits, 25 mm (or as charted on the screen)
  g.setLineDash([]);
  dash();
  g.lineWidth = Math.max(1, 0.32 * S52_PX_MM);
  g.strokeStyle = colour("CHBLK");
  for (const f of parts) {
    if (f[0] !== "L") continue;
    const [x, y] = at(Number(f[1]), Number(f[2]) * S52_PX_MM);
    g.beginPath(); g.moveTo(half, half); g.lineTo(x, y); g.stroke();
  }
  return g.getImageData(0, 0, n, n);
}

/** Where an IHO text's image (s52TextImage) is anchored on its point. */
function textAnchor() {
  const dx = ["coalesce", ["get", "dx"], 0], dy = ["coalesce", ["get", "dy"], 0];
  const hp = ["case", ["all", ["==", ["get", "h"], "S"], [">=", dx, 0]], "left", ["all", ["==", ["get", "h"], "E"], ["<=", dx, 0]], "right", ""];
  const vp = ["case", ["all", ["==", ["get", "v"], "T"], ["<=", dy, 0]], "top", ["all", ["==", ["get", "v"], "B"], [">=", dy, 0]], "bottom", ""];
  return ["let", "hp", hp, "vp", vp,
    ["case", ["all", ["==", ["var", "hp"], ""], ["==", ["var", "vp"], ""]], "center",
      ["==", ["var", "vp"], ""], ["var", "hp"],
      ["==", ["var", "hp"], ""], ["var", "vp"],
      ["concat", ["var", "vp"], "-", ["var", "hp"]]]];
}

/** A line style as drawn: a catalogue style, or a rule's simple line
 *  "_interval_width_colour[_start-length...]". */
function s52LineStyle(sl) {
  if (!sl.startsWith("_")) return S52.lines[sl] || null;
  const [interval, width, colour, ...dashes] = sl.slice(1).split("_");
  const w = Number(width) || 0.32;
  return { interval: Number(interval) || 0, width: w, colour, height: w, symbols: [], parts: [],
    dashes: dashes.map((d) => d.split("-").map(Number)) };
}
/** Draw symbol `id` with its pivot at x, y (px), and its copies a period away either side. */
function s52Stamp(g, id, x, y, wrapX, wrapY) {
  const s = S52.imgs.get(id);
  if (!s) return;
  const { img, vb } = s, k = S52_PX_MM;
  for (const dx of wrapX ? [-wrapX, 0, wrapX] : [0]) for (const dy of wrapY ? [-wrapY, 0, wrapY] : [0])
    g.drawImage(img, x + dx + vb[0] * k, y + dy + vb[1] * k, vb[2] * k, vb[3] * k);
}
/** The image for "s52l:<sl>": one interval of the line style, the line along its middle,
 *  repeated along the line by the map (line-pattern, as tall as the line is wide). */
function s52LineImage(sl) {
  if (!S52.ready) return null;
  const st = s52LineStyle(sl);
  if (!st) return null;
  const k = S52_PX_MM;
  const W = Math.max(2, Math.round((st.interval || 0.5) * k));
  const H = Math.max(2, Math.round((st.height || st.width) * k));
  const c = document.createElement("canvas");
  c.width = W; c.height = H;
  const g = c.getContext("2d");
  // the map draws a line pattern's image mirrored across the line (its top on the left of
  // the way the line runs); the IHO's symbols sit on the right (inside an area's clockwise
  // boundary): drawn upside down here, they come out the IHO's way round
  g.translate(0, H);
  g.scale(1, -1);
  const y = H / 2;
  for (const p of st.parts?.length ? st.parts : [st]) {
    g.fillStyle = S52.colours[p.colour] || "#ff00ff";
    const lw = Math.max(1, p.width * k);
    if (!p.dashes?.length || !st.interval) g.fillRect(0, y - lw / 2, W, lw);
    else for (const [s, l] of p.dashes) for (const off of [-W, 0]) g.fillRect(s * k + off, y - lw / 2, l * k, lw);
    for (const [id, pos] of p.symbols || []) s52Stamp(g, id, pos * k, y, W, 0);
  }
  return g.getImageData(0, 0, W, H);
}
/** The image for "s52f:<af>": a tile of the area fill's symbol lattice. */
function s52FillImage(af) {
  if (!S52.ready) return null;
  const f = S52.fills[af];
  if (!f) return null;
  const k = S52_PX_MM;
  const W = f.v1[0] || 10;
  // rows shift by v2.x: repeat n rows so the tile wraps (n with the least shift left over)
  let n = 1, best = Infinity;
  for (let m = 1; m <= 4; m++) {
    const r = ((m * f.v2[0]) % W + W) % W, e = Math.min(r, W - r);
    if (e < best - 0.05) { best = e; n = m; }
  }
  const tw = Math.max(2, Math.round(W * k)), th = Math.max(2, Math.round(n * f.v2[1] * k));
  const c = document.createElement("canvas");
  c.width = tw; c.height = th;
  const g = c.getContext("2d");
  for (let i = 0; i < n; i++) s52Stamp(g, f.symbol, (((i * f.v2[0]) % W + W) % W) * k, i * f.v2[1] * k, tw, th);
  return g.getImageData(0, 0, tw, th);
}
/** The colour of a token, as an expression over `cf`. */
const s52FillColour = (prop = "cf") => {
  const pairs = Object.entries(S52.colours).flat();
  return pairs.length ? ["match", ["coalesce", ["get", prop], ""], ...pairs, "#ff00ff"] : "#ff00ff";
};

/** What stays in the Base display (IMO ECDIS performance standard, display base): the
 *  dangers (`hz`), and buoys, beacons and traffic routeing (`db`). */
const BASE = ["any", ["==", ["get", "hz"], 1], ["==", ["get", "db"], 1]];

/** Zoom filter: a feature shows from its SCAMIN zoom, except hazards to the vessel
 * (rocks, wrecks, obstructions, farms, piles, platforms, bridges, overhead cables:
 * `hz` from the chart store), which show at every zoom. Underwater dangers of known
 * depth that aren't isolated dangers still follow SCAMIN (the point filters below). */
const HZ = ["==", ["get", "hz"], 1];
// `maxz`: a coarser chart's copy of a mark, standing in until the finer chart's copy shows
const notReplaced = ["any", ["!", ["has", "maxz"]], ["<", ["zoom"], ["get", "maxz"]]];
const minz = ["all", notReplaced, ["any", HZ, ["!", ["has", "minz"]], [">=", ["zoom"], ["get", "minz"]]]];
const minzDeep = ["all", notReplaced, ["any", ["!", ["has", "minz"]], [">=", ["zoom"], ["get", "minz"]]]];

/** Which point symbols draw over which: the IHO drawing priority (`pp`), then dangers
 *  over other marks and isolated dangers over all. */
function pointSortKey(m) {
  return ["+", ["*", ["coalesce", ["get", "pp"], 24], 10], ["case", isolated(m), 3, ["match", ["get", "cls"], ["UWTROC", "WRECKS", "OBSTRN"], 2, 1]]];
}

// The IHO drawing priorities (S-101's, S-52's ×3): each step's areas, then its area
// boundaries, then its lines, so a higher priority draws over a lower one whatever the
// geometry, and at one priority lines over areas (S-52 10.3.4.1). Features of old tiles
// without priorities: fills at 3, lines at 15.
const S52_PRIORITIES = [0, 3, 6, 9, 12, 15, 18, 21, 24, 27, 30];
const S52_AREA_IDS = [], S52_LINE_IDS = [], S52_FILL_IDS = [], S52_SOLID_IDS = [];
function s52Layers() {
  const src = { source: "chart" };
  const out = [];
  S52_PRIORITIES.forEach((p, i) => {
    const hi = S52_PRIORITIES[i + 1] ?? 1000;
    const band = (key, dflt) => { const v = ["coalesce", ["get", key], dflt]; return ["all", [">=", v, i ? p : -1], ["<", v, hi]]; };
    const lineW = ["*", ["get", "slw"], S52_PX_MM / 2], solidW = ["max", 1, ["*", ["get", "lw"], S52_PX_MM / 2]];
    out.push(
      // colour fills and fill patterns
      { id: `s52-fill-${p}`, type: "fill", ...src, "source-layer": "far", filter: ["all", minz, ["has", "cf"], band("pf", 3)],
        paint: { "fill-color": "#ff00ff", "fill-opacity": ["-", 1, ["coalesce", ["get", "ct"], 0]] } },
      { id: `s52-pat-${p}`, type: "fill", ...src, "source-layer": "far", filter: ["all", minz, ["has", "af"], band("pf", 3)],
        paint: { "fill-pattern": ["concat", "s52f:", ["get", "af"]] } },
      // area boundaries and lines: a line style drawn as a pattern, or a plain solid line
      // (`lc` colour, `lw` mm)
      { id: `s52-aline-${p}`, type: "line", ...src, "source-layer": "far", filter: ["all", minz, ["has", "sl"], band("pl", 15)],
        paint: { "line-pattern": ["concat", "s52l:", ["get", "sl"]], "line-width": lineW } },
      { id: `s52-aline-solid-${p}`, type: "line", ...src, "source-layer": "far", filter: ["all", minz, ["has", "lc"], band("pl", 15)],
        paint: { "line-color": "#000", "line-width": solidW } },
      { id: `s52-line-${p}`, type: "line", ...src, "source-layer": "fln", filter: ["all", minz, ["has", "sl"], band("pl", 15)],
        paint: { "line-pattern": ["concat", "s52l:", ["get", "sl"]], "line-width": lineW } },
      { id: `s52-line-solid-${p}`, type: "line", ...src, "source-layer": "fln", filter: ["all", minz, ["has", "lc"], band("pl", 15)],
        paint: { "line-color": "#000", "line-width": solidW } },
    );
    if (!S52_FILL_IDS.includes(`s52-fill-${p}`)) {
      S52_FILL_IDS.push(`s52-fill-${p}`);
      S52_SOLID_IDS.push(`s52-aline-solid-${p}`, `s52-line-solid-${p}`);
      S52_AREA_IDS.push(`s52-fill-${p}`, `s52-pat-${p}`, `s52-aline-${p}`, `s52-aline-solid-${p}`);
      S52_LINE_IDS.push(`s52-line-${p}`, `s52-line-solid-${p}`);
    }
  });
  return out;
}

export function setupChartFeatures(map, { before, safetyContour = 5, shallow = false, onSymbols = () => {}, onScale = () => {} }) {
  shallowDangers = shallow;
  const $ = (id) => document.getElementById(id);
  const opts = { ...DEFAULTS };
  try { Object.assign(opts, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  // before display categories: "Lights, marks, dangers, areas" off was the base display
  if (opts.show === false) { opts.cat = "base"; delete opts.show; } else delete opts.show;
  // What the chart shows is kept for each view with a chart (Navigation, Planning): the display
  // category and the Chart display switches, so that Planning can be a clean chart under the
  // weather while Navigation keeps the lights, names and soundings. A view not set up yet starts
  // as the one you came from. The symbols, sizes, calibration and safety settings are the same
  // in every view.
  const chartView = () => (document.documentElement.dataset.view === "plan" ? "plan" : "nav");
  const pick = (o) => Object.fromEntries(VIEW_KEYS.filter((k) => o[k] !== undefined).map((k) => [k, o[k]]));
  let curView = chartView();
  if (opts.views?.[curView]) Object.assign(opts, pick(opts.views[curView]));
  const save = () => {
    // only this view's part is this window's to write: another window may be in the other view
    let stored = {};
    try { stored = JSON.parse(localStorage.getItem(KEY) || "{}").views || {}; } catch { /* ignore */ }
    opts.views = { ...stored, [curView]: pick(opts) };
    try { localStorage.setItem(KEY, JSON.stringify(opts)); } catch { /* ignore */ }
  };
  // the screen calibration's scale, before the layers are made with it (applyCalibration)
  if (Number(opts.screenMm) > 5) S52_PX_MM = 3.78 * 2 * (50 / Number(opts.screenMm));
  let enabled = false;
  const useS52 = () => String(opts.sym).startsWith("s52") && S52.ready;

  const src = { source: "chart" };
  const cls = (...c) => ["in", ["get", "cls"], ["literal", c]];
  const layers = [
    // areas
    { id: "f-area-fill", type: "fill", ...src, "source-layer": "far", filter: ["all", minz, ["any", cls("MARCUL", "OBSTRN"), ["==", ["get", "entry"], 1]]],
      paint: { "fill-color": ["match", ["get", "cls"], "OBSTRN", "#7fb8e8", MAGENTA], "fill-opacity": ["match", ["get", "cls"], "OBSTRN", 0.5, 0.08] } },
    { id: "f-area-hit", type: "fill", ...src, "source-layer": "far", filter: ["all", minz, ["!", cls("SEAARE")]],
      paint: { "fill-color": "#000", "fill-opacity": 0.01 } },
    { id: "f-area-dash", type: "line", ...src, "source-layer": "far", filter: ["all", minz, cls("RESARE", "ACHARE", "MARCUL", "CBLARE", "CTNARE", "PRCARE", "ISTZNE", "TSSCRS", "TSSRON", "FAIRWY", "FERYRT", "UNKNWN")],
      paint: { "line-color": ["match", ["get", "cls"], ["FAIRWY", "FERYRT"], "#5b6b75", MAGENTA], "line-width": ["match", ["get", "cls"], "RESARE", 1.4, 1], "line-dasharray": [4, 2.5] } },
    // ships' routeing: separation zones and lines (S-52 TSEZNE, TSELNE: translucent magenta), boundaries
    { id: "f-tss-fill", type: "fill", ...src, "source-layer": "far", filter: ["all", minz, cls("TSEZNE")],
      paint: { "fill-color": MAGENTA, "fill-opacity": 0.22 } },
    { id: "f-tss-line", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("TSELNE")],
      paint: { "line-color": MAGENTA, "line-width": 6, "line-opacity": 0.35 } },
    { id: "f-tss-bnd", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("TSSBND")],
      paint: { "line-color": MAGENTA, "line-width": 2, "line-dasharray": [4, 2.5] } },
    // wharves, breakwaters, pontoons, floating docks, hulks, causeways
    { id: "f-construct", type: "fill", ...src, "source-layer": "far", filter: ["all", minz, cls("SLCONS", "PONTON", "FLODOC", "HULKES", "MORFAC", "CAUSWY")],
      paint: { "fill-color": "#b9b2a3", "fill-outline-color": INK } },
    { id: "f-bridge", type: "fill", ...src, "source-layer": "far", filter: ["all", minz, cls("BRIDGE")],
      paint: { "fill-color": "#9a9a9a", "fill-outline-color": INK } },
    { id: "f-area-grey", type: "line", ...src, "source-layer": "far", filter: ["all", minz, cls("DMPGRD", "OBSTRN", "WEDKLP")],
      paint: { "line-color": "#5b6b75", "line-width": 0.9, "line-dasharray": [2, 2] } },
    // lines
    { id: "f-line-mag", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("CBLSUB", "PIPSOL", "RDOCAL")],
      paint: { "line-color": MAGENTA, "line-width": 1.1, "line-dasharray": [3, 2] } },
    { id: "f-line-solid", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("NAVLNE", "BRIDGE", "SLCONS", "PONTON", "CAUSWY")],
      paint: { "line-color": INK, "line-width": ["match", ["get", "cls"], "BRIDGE", 2.5, "PONTON", 2.2, "SLCONS", 1.6, "CAUSWY", 1.6, 1] } },
    // ENC coastlines (the ENC land fill has no outline of its own)
    { id: "f-coast", type: "line", ...src, "source-layer": "fln", filter: cls("COALNE"),
      paint: { "line-color": "#8a7a4a", "line-width": 0.9 } },
    { id: "f-line-dash", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("RECTRC", "CBLOHD", "PIPOHD", "FSHFAC", "UNKNWN")],
      paint: { "line-color": ["match", ["get", "cls"], "RECTRC", INK, "UNKNWN", MAGENTA, "#5b6b75"], "line-width": 1, "line-dasharray": [5, 3] } },
    // lines of equal magnetic variation (isogonals), labelled with today's variation
    { id: "f-line-magvar", type: "line", ...src, "source-layer": "fln", filter: ["all", minz, cls("MAGVAR")],
      paint: { "line-color": MAGENTA, "line-width": 0.9, "line-dasharray": [1, 3] } },
    { id: "f-magvar-lbl", type: "symbol", ...src, "source-layer": "fln", filter: ["all", minz, cls("MAGVAR"), ["has", "lbl"]], minzoom: 6,
      layout: { "icon-image": ["concat", "txt:n:", ["get", "lbl"]], "symbol-placement": "line", "symbol-spacing": 400,
                "icon-rotation-alignment": "viewport", "icon-allow-overlap": false, "icon-padding": 4 } },
    // the IHO portrayal's areas and lines (Symbols: IHO), in the order of their drawing
    // priorities (S-52 10.3.4.1): s52Layers()
    ...s52Layers(),
    // light sectors as S-52 draws them (with the IHO symbols): limits of a length on the
    // ground (full light lines, or charted) dashed lines; the arcs (in the light's colour
    // over black, obscured ones dashed) and the 25 mm limits an image of fixed size on the
    // screen at the light (`sec`), as an ECDIS draws them, turning with the chart
    { id: "s52-sec-dash", type: "line", ...src, "source-layer": "lsec", filter: ["all", minz, ["==", ["get", "k"], "sdash"]],
      paint: { "line-color": "#000", "line-width": 0.32 * S52_PX_MM / 2, "line-dasharray": [3.6 / 0.32, 1.8 / 0.32] } },
    { id: "s52-sec-img", type: "symbol", ...src, "source-layer": "fpt", filter: ["all", minz, ["has", "sec"]],
      layout: { "icon-image": ["concat", "s52sec:", ["get", "sec"]], "icon-rotation-alignment": "map", "icon-pitch-alignment": "map",
                "icon-allow-overlap": true, "icon-ignore-placement": true } },
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
    // where a light can be clicked for its details, whatever it is drawn with (with the IHO
    // symbols a sector light has no flare, just its sectors)
    { id: "f-light-hit", type: "circle", ...src, "source-layer": "fpt", filter: ["all", minz, cls("LIGHTS")],
      paint: { "circle-radius": 9, "circle-opacity": 0, "circle-stroke-opacity": 0 } },
    // symbols that turn with the chart: a traffic lane's direction arrow
    { id: "f-rot", type: "symbol", ...src, "source-layer": "fpt", filter: ["all", minz, ["has", "rot"]],
      layout: { "icon-image": ["concat", "s57:", ["get", "icon"]], "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map",
                "icon-allow-overlap": true, "icon-ignore-placement": true } },
    // the IHO information symbols (the "i" of a feature with notes): only when asked for, as on an ECDIS
    { id: "s52-info", type: "symbol", ...src, "source-layer": "fpt", filter: ["all", minz, ["has", "si"]],
      layout: { "icon-image": ["concat", "s52:", ["get", "si"]], "icon-allow-overlap": true, "icon-ignore-placement": true } },
    // the IHO date-dependent symbol (CHDATD01) on features charted for a time only: when asked for
    { id: "s52-date", type: "symbol", ...src, "source-layer": "fpt", filter: ["all", minz, ["has", "sd"]],
      layout: { "icon-image": ["concat", "s52:", ["get", "sd"]], "icon-allow-overlap": true, "icon-ignore-placement": true } },
    { id: "f-points", type: "symbol", ...src, "source-layer": "fpt", filter: pointFilter(safetyContour),
      layout: { "icon-image": pointIcon(safetyContour),
                "icon-anchor": ["match", ["slice", ["get", "icon"], 0, 2], ["b|", "c|"], "bottom", "center"],
                "icon-allow-overlap": true, "icon-ignore-placement": true,
                "symbol-sort-key": pointSortKey(safetyContour) } },
    // labels: light characteristics, names of marks, depths over dangers
    { id: "f-labels", type: "symbol", ...src, "source-layer": "fpt", filter: labelFilter(safetyContour),
      layout: { "icon-image": ["concat", "txt:", ["match", ["get", "cls"], "LIGHTS", "l", ["UWTROC", "WRECKS", "OBSTRN"], "d", "b"], ":", ["get", "lbl"]],
                "icon-anchor": "left", "icon-offset": ["match", ["get", "cls"], "LIGHTS", ["literal", [12, 8]], ["literal", [8, -6]]],
                "icon-allow-overlap": false, "icon-padding": 1,
                "symbol-sort-key": ["match", ["get", "cls"], "LIGHTS", 0, 1] } },
    // the IHO rules' texts, placed before TweedPlot's labels (above them) (light descriptions, clearances, bearings, seabed...): each an
    // image anchored on its feature, the text placed in it as the rules align and offset it
    { id: "s52-text", type: "symbol", ...src, "source-layer": "ftx", filter: ["all", minz, ["has", "txt"]],
      layout: { "icon-image": ["concat", "s52t:", ["get", "c"], ":", ["get", "fs"], ":", ["get", "h"], ["get", "v"], ":",
                  ["coalesce", ["get", "dx"], 0], ":", ["coalesce", ["get", "dy"], 0], ":", ["get", "txt"]],
                "icon-anchor": textAnchor(), "icon-rotate": ["coalesce", ["get", "rot"], 0], "icon-allow-overlap": false, "icon-padding": 0,
                "symbol-sort-key": ["get", "g"] } },
  ];
  for (const l of layers) map.addLayer(l, before);
  // hazards stay on the chart when chart features are switched off (like an ECDIS's
  // display base, which can't be removed): these layers keep their hazards then
  const HAZ_LAYERS = ["f-area-fill", "f-area-hit", "f-area-dash", "f-construct", "f-bridge", "f-area-grey", "f-line-solid", "f-line-dash", "f-points",
    "f-tss-fill", "f-tss-line", "f-tss-bnd", "f-rot"];
  const baseFilter = Object.fromEntries(layers.map((l) => [l.id, l.filter]));
  // labels only once there is room
  map.setLayerZoomRange("f-labels", 11, 24);

  // The display category (IMO ECDIS PS App. 2: the display mode is part of the display base, so
  // always shown) is in the data panel's display row (sidepanel.js), with its Base / Standard /
  // All buttons: "racenav-dispcat" tells it the state, "racenav-set-dispcat" sets it.

  const SECTOR = ["f-sec-leg", "f-sec-arc", "s52-sec-img", "s52-sec-dash"];
  const NAMES = ["f-names", "f-pnames", "f-labels"];
  // Display categories, as an ECDIS (IHO S-52): Base is land, depth areas, the safety
  // contour, dangers, buoys and beacons, and traffic separation schemes, and can't be
  // reduced; Standard adds lights, other routeing measures, ferry routes and fairways,
  // restricted and caution areas, constructions; All adds soundings, the other contours,
  // submarine cables and pipelines, dumping grounds and kelp.
  const ALL_ONLY = ["f-line-mag", "f-area-grey"];
  const MAIN_ALL = ["depcnt", "wdcnt", "depcnt-lbl"]; // chart layers in main.js shown only in All
  // With the IHO symbols, the IHO rules draw the lines and areas they portray (`s5`), in
  // the display category their viewing groups give (`vg`: 1 base, 2 standard, 3 other;
  // hazards always); TweedPlot's own layers draw the rest.
  const S52_LAYERS = [...S52_AREA_IDS, ...S52_LINE_IDS, "s52-info", "s52-date", "s52-sec-img", "s52-sec-dash", "s52-text"];
  // S-52 text groups: important (10-19: clearances, bearings...) and other (20-: light
  // descriptions, seabed, heights...), each switched by the mariner
  const textGroups = () => {
    const g = ["get", "g"];
    if (opts.textImportant && opts.textOther) return null;
    if (opts.textImportant) return ["<", g, 20];
    if (opts.textOther) return [">=", g, 20];
    return ["has", "__never__"];
  };
  const TP_DRAWN = ["f-area-fill", "f-area-dash", "f-tss-fill", "f-tss-line", "f-tss-bnd", "f-construct", "f-bridge", "f-area-grey",
    "f-line-mag", "f-line-solid", "f-coast", "f-line-dash", "f-sec-leg", "f-sec-arc"];
  // The viewing date (S-52 10.4.1): today unless the mariner sets another, or a range of
  // dates (`viewDate`, `viewTo`; not kept between runs: the chart always starts on today)
  let viewDate = null, viewTo = null;
  const ymd = (d) => `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
  // x within lo..hi on the year's circle (a period can run over the new year)
  // (two literal strings compared would read as an old-style filter: decided here instead)
  const within = (x, lo, hi) => {
    const inside = ["all", ["<=", lo, x], [">=", hi, x]], wrap = ["any", ["<=", lo, x], [">=", hi, x]];
    if (typeof lo === "string" && typeof hi === "string") return lo <= hi ? inside : wrap;
    return ["case", ["<=", lo, hi], inside, wrap];
  };
  function dateFilter() {
    const a = ymd(viewDate ?? new Date()), b = viewTo && ymd(viewTo) > a ? ymd(viewTo) : a;
    // charted at some time in a..b
    const fixed = ["all", ["any", ["!", ["has", "ds"]], ["<=", ["get", "ds"], b]], ["any", ["!", ["has", "de"]], [">=", ["get", "de"], a]]];
    const ps = ["get", "ps"], pe = ["get", "pe"], ma = a.slice(4), mb = b.slice(4);
    const yearRound = Number(b.slice(0, 4)) - Number(a.slice(0, 4)) > 1 || (b.slice(0, 4) !== a.slice(0, 4) && mb >= ma);
    const periodic = yearRound ? true : ["any", ["!", ["has", "ps"]], within(ma, ps, pe), within(ps, ma, mb)];
    return ["all", fixed, periodic];
  }
  // a new day: the chart moves on with it
  setInterval(() => { if (!viewDate) apply(); }, 3600 * 1000);
  function modeFilter(id, cat) {
    const s52 = useS52();
    const vgOk = cat === "base" ? ["any", ["==", ["get", "vg"], 1], HZ]
      : cat === "standard" ? ["any", ["!", ["has", "vg"]], ["<=", ["get", "vg"], 2], HZ] : null;
    if (id === "s52-text") { const tg = textGroups(); return vgOk && tg ? ["all", vgOk, tg] : (vgOk ?? tg); }
    if (S52_LAYERS.includes(id)) return vgOk;
    if (TP_DRAWN.includes(id)) return s52 ? ["!", ["has", "s5"]] : null;
    // a traffic lane's arrow of TweedPlot's, where the IHO rules draw the lane's own
    if (id === "f-rot") return s52 ? ["!", ["has", "po"]] : null;
    // an area's IHO symbol at its middle (`ac`), or TweedPlot's symbol for a small one (`po`)
    // points in Standard: what the rules put in Base or Standard (landmarks, for one, are
    // "Other"), with hazards and TweedPlot's display base (buoys, beacons) always
    // (and a light's structure, `lt`, whenever its light shows: a lighthouse the rules put
    // in "Other" would leave the light standing alone)
    const ptOk = cat === "standard" ? ["any", ["!", ["has", "vg"]], ["<=", ["get", "vg"], 2], HZ, ["==", ["get", "db"], 1], ["==", ["get", "lt"], 1]] : true;
    // a light the IHO rules drew as sectors only: no flare of TweedPlot's either
    if (id === "f-light-hit") return s52 ? ptOk : null;
    if (id === "f-lights") return s52 ? ["all", ["any", ["has", "sy"], ["!", ["has", "s5"]]], ptOk] : null;
    // the IHO symbol of a danger writes its depth in it: no label beside it as well
    // (and a light's description written by the IHO rules, `itx`, stands for TweedPlot's)
    if (id === "f-labels") return s52 ? ["all", ["!", ["in", "SOUNDS", ["coalesce", ["get", "sy"], ""]]], ptOk, ...(opts.textOther ? [["!", ["has", "itx"]]] : [])] : null;
    if (id === "f-points") return s52 ? ["all", ["!", ["has", "po"]], ptOk, ["any", ["!", ["has", "ac"]], ["all", ["has", "sy"], vgOk ?? true]]] : ["!", ["has", "ac"]];
    return null;
  }
  function apply() {
    const cat = opts.cat;
    for (const l of layers) {
      const haz = HAZ_LAYERS.includes(l.id);
      const s52layer = S52_LAYERS.includes(l.id);
      let on = enabled && (cat !== "base" || haz || s52layer || l.id === "f-coast"); // the ENC coast is part of the chart base
      if (s52layer) on = on && useS52();
      if (l.id === "s52-info") on = on && opts.info;
      if (l.id === "s52-date") on = on && opts.dateSym;
      if (ALL_ONLY.includes(l.id)) on = on && cat === "all";
      if (SECTOR.includes(l.id)) on = on && opts.sectors;
      if (NAMES.includes(l.id)) on = on && opts.names;
      map.setLayoutProperty(l.id, "visibility", on ? "visible" : "none");
      const parts = [baseFilter[l.id]];
      // Base: hazards and TweedPlot's display base, and with the IHO symbols whatever the
      // rules put in the display base (a hulk, a pylon, an islet, a conspicuous structure)
      if (haz && cat === "base") parts.push(useS52() ? ["any", BASE, ["==", ["get", "vg"], 1]] : BASE);
      const m = modeFilter(l.id, cat);
      if (m) parts.push(m);
      // only what is charted for the viewing date (S-52 10.4.1)
      parts.push(dateFilter());
      map.setFilter(l.id, parts.length > 1 ? ["all", ...parts] : parts[0]);
    }
    if (useS52()) {
      for (const id of S52_FILL_IDS) map.setPaintProperty(id, "fill-color", s52FillColour());
      for (const id of S52_SOLID_IDS) map.setPaintProperty(id, "line-color", s52FillColour("lc"));
      map.setPaintProperty("s52-sec-dash", "line-color", s52FillColour("tok"));
    }
    {
      const fmt = (d) => d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
      // the standard display exactly (S-Mode), or the standard category changed by the mariner,
      // which must be indicated (IMO ECDIS PS 10.5)
      const isStd = Object.entries(STANDARD).every(([k, v]) => opts[k] === v);
      window.dispatchEvent(new CustomEvent("racenav-dispcat", { detail: {
        enabled, cat, standard: isStd,
        dated: viewDate ? `Chart date ${fmt(viewDate)}${viewTo ? ` to ${fmt(viewTo)}` : ""}` : "",
      } }));
    }
    for (const id of MAIN_ALL) if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", enabled && cat === "all" ? "visible" : "none");
    if (map.getLayer("soundg")) map.setLayoutProperty("soundg", "visibility", enabled && cat !== "base" && (cat === "all" || opts.soundings) ? "visible" : "none");
    const sel = $("set-dispcat");
    if (sel) sel.value = cat;
    for (const [id, key] of [["set-sectors", "sectors"], ["set-featnames", "names"], ["set-soundings", "soundings"], ["set-info", "info"], ["set-text-important", "textImportant"], ["set-text-other", "textOther"], ["set-datesym", "dateSym"]]) if ($(id)) $(id).checked = opts[key];
    if ($("set-soundings")) $("set-soundings").disabled = cat !== "standard";
    const std = $("set-std");
    if (std) std.classList.toggle("on", Object.entries(STANDARD).every(([k, v]) => opts[k] === v));
  }
  const sel = $("set-dispcat");
  if (sel) sel.onchange = () => { opts.cat = sel.value; save(); apply(); };
  const std = $("set-std");
  if (std) std.onclick = () => { Object.assign(opts, STANDARD); save(); apply(); };
  // the data panel's buttons: Standard is the one-press standard display (S-Mode); Base and All
  // change only the category, as Chart › Display does
  window.addEventListener("racenav-set-dispcat", (e) => {
    const c = e.detail?.cat;
    if (c === "standard") Object.assign(opts, STANDARD);
    else if (c === "base" || c === "all") opts.cat = c;
    else return;
    save(); apply();
  });
  for (const [id, key] of [["set-sectors", "sectors"], ["set-featnames", "names"], ["set-soundings", "soundings"], ["set-info", "info"], ["set-text-important", "textImportant"], ["set-text-other", "textOther"], ["set-datesym", "dateSym"], ["set-avoid-dangers", "avoidDangers"], ["set-avoid-farms", "avoidFarms"]]) {
    const el = $(id);
    if (!el) continue;
    el.checked = opts[key];
    el.onchange = () => { opts[key] = el.checked; save(); apply(); };
  }
  window.addEventListener("racenav-view", () => {
    const v = chartView();
    if (v === curView) return;
    let stored = {};
    try { stored = JSON.parse(localStorage.getItem(KEY) || "{}").views || {}; } catch { /* ignore */ }
    curView = v;
    if (stored[v]) Object.assign(opts, pick(stored[v]));
    save();
    apply();
  });
  const dateIn = $("set-chartdate"), dateTo = $("set-chartdate-to"), dateNow = $("set-chartdate-today");
  const iso = (d) => (d ? `${ymd(d).slice(0, 4)}-${ymd(d).slice(4, 6)}-${ymd(d).slice(6)}` : "");
  const parse = (v) => { const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v || ""); return m ? new Date(+m[1], +m[2] - 1, +m[3]) : null; };
  const showDate = () => { if (dateIn) dateIn.value = iso(viewDate); if (dateTo) dateTo.value = iso(viewTo); };
  const setDates = (from, to) => {
    viewDate = from; viewTo = from && to && ymd(to) > ymd(from) ? to : null;
    if (viewDate && !viewTo && ymd(viewDate) === ymd(new Date())) viewDate = null;
    showDate(); apply();
  };
  if (dateIn) dateIn.onchange = () => setDates(parse(dateIn.value), viewTo);
  if (dateTo) dateTo.onchange = () => setDates(viewDate ?? new Date(), parse(dateTo.value));
  if (dateNow) dateNow.onclick = () => setDates(null, null);
  showDate();
  apply();

  // Screen calibration: a bar 50 mm long by the browser's reckoning (189 CSS px, at its
  // nominal 96 dpi; it never changes), and what the mariner measures it to be on the
  // screen; every IHO symbol, line, pattern and text is drawn to the scale that gives
  function applyCalibration(redraw = true) {
    const mm = Number(opts.screenMm) > 5 ? Number(opts.screenMm) : 50;
    S52_PX_MM = 3.78 * 2 * (50 / mm);
    window.dispatchEvent(new Event("racenav-calibration"));
    if (!redraw) return;
    const lineW = ["*", ["get", "slw"], S52_PX_MM / 2], solidW = ["max", 1, ["*", ["get", "lw"], S52_PX_MM / 2]];
    for (const id of S52_AREA_IDS.concat(S52_LINE_IDS)) {
      if (!map.getLayer(id) || map.getLayer(id).type !== "line") continue;
      map.setPaintProperty(id, "line-width", id.includes("solid") ? solidW : lineW);
    }
    map.setPaintProperty("s52-sec-dash", "line-width", 0.32 * S52_PX_MM / 2);
    for (const id of map.listImages()) if (id.startsWith("s52")) map.removeImage(id);
    map.setLayoutProperty("f-points", "icon-image", pointImage(useS52()));
    onScale();
  }
  const calIn = $("set-screen-mm");
  if (calIn) {
    calIn.value = opts.screenMm;
    calIn.onchange = () => { const v = Number(calIn.value); if (v > 5 && v < 500) { opts.screenMm = v; save(); applyCalibration(); } };
  }
  applyCalibration(false);

  // symbol size: the symbols are drawn at twice size, so up to 2x stays sharp; labels grow at half the rate
  function applySize() {
    const k = opts.symSize, kl = 1 + (k - 1) / 2;
    map.setLayoutProperty("f-lights", "icon-size", k);
    map.setLayoutProperty("f-points", "icon-size", k);
    map.setLayoutProperty("f-rot", "icon-size", k);
    map.setLayoutProperty("s52-info", "icon-size", k);
    map.setLayoutProperty("s52-date", "icon-size", k);
    map.setLayoutProperty("s52-text", "icon-size", k);
    map.setLayoutProperty("f-labels", "icon-size", kl);
    // labels sit beside the symbol: keep them clear of it as it grows
    map.setLayoutProperty("f-labels", "icon-offset", ["match", ["get", "cls"], "LIGHTS", ["literal", [12 * k / kl, 8 * k / kl]], ["literal", [8 * k / kl, -6 * k / kl]]]);
    const v = $("set-symsize-v");
    if (v) v.textContent = `${Math.round(k * 100)}%`;
    map.setLayoutProperty("f-points", "icon-image", pointImage(useS52()));
  }
  // which symbols: the IHO's (S-52) where the chart store has them, else TweedPlot's
  let contour = safetyContour;
  /** The point symbols' image. A rock, wreck or obstruction of known depth has the depth
   *  written in a dotted circle twice the size of the plain symbol (S-52 DANGER01/02/03):
   *  where the nearest other symbol in its tile (`rm`, tile units, 8 to a pixel) is closer
   *  than that circle needs, it gets the plain symbol of its kind without the depth (`syc`)
   *  until zoomed in far enough. Past the tiles' last zoom (16) the room only grows. */
  function pointImage(on) {
    const has = ["has", "sy"];
    if (!on) return pointIcon(contour);
    // the circle's half-width (4.2 mm) and a neighbour's (2.2 mm), with a little gap
    const need = 7 * (S52_PX_MM / 2) * (opts.symSize || 1) * 8;
    const full = ["concat", "s52:", ["get", "sy"]];
    const roomy = ["case", ["all", ["has", "syc"], ["<", ["coalesce", ["get", "rm"], 1e9], need]], ["concat", "s52:", ["get", "syc"]], full];
    return ["step", ["zoom"], ["case", has, roomy, pointIcon(contour)], 17, ["case", has, full, pointIcon(contour)]];
  }
  function applySymbols() {
    const on = useS52();
    const has = ["has", "sy"];
    map.setLayoutProperty("f-points", "icon-image", pointImage(on));
    map.setLayoutProperty("f-points", "icon-anchor", on ? ["case", has, "center", ["match", ["slice", ["get", "icon"], 0, 2], ["b|", "c|"], "bottom", "center"]]
      : ["match", ["slice", ["get", "icon"], 0, 2], ["b|", "c|"], "bottom", "center"]);
    map.setLayoutProperty("f-lights", "icon-image", on ? ["case", has, ["concat", "s52:", ["get", "sy"]], ["concat", "s57:", ["get", "icon"]]] : ["concat", "s57:", ["get", "icon"]]);
    map.setLayoutProperty("f-lights", "icon-anchor", on ? ["case", has, "center", "top-left"] : "top-left");
    map.setLayoutProperty("f-lights", "icon-offset", on ? ["case", has, ["literal", [0, 0]], ["literal", [-2, -2]]] : ["literal", [-2, -2]]);
    const sel = $("set-symbols");
    if (sel) sel.value = opts.sym;
    apply();
    if (on !== lastOn) { lastOn = on; onSymbols(on); }
  }
  let lastOn = null;
  const symSel = $("set-symbols");
  // the IHO rules' mariner options: simplified or paper-chart point symbols, plain or
  // symbolised area boundaries (the chart store portrays again when they change)
  function sendOptions() {
    invoke("chart_s52_context", { id: "SimplifiedSymbols", value: String(opts.sym === "s52s") }).catch(() => {});
    invoke("chart_s52_context", { id: "PlainBoundaries", value: String(opts.bounds !== "symbolised") }).catch(() => {});
  }
  sendOptions();
  if (symSel) symSel.onchange = () => { opts.sym = symSel.value; save(); sendOptions(); applySymbols(); };
  const bndSel = $("set-boundaries");
  if (bndSel) {
    bndSel.value = opts.bounds;
    bndSel.onchange = () => { opts.bounds = bndSel.value; save(); sendOptions(); };
  }
  loadS52Symbols().then(applySymbols).catch((e) => console.warn("IHO symbols:", e));
  applySymbols();

  const sz = $("set-symsize");
  if (sz) {
    sz.value = opts.symSize;
    sz.oninput = () => { opts.symSize = Number(sz.value); save(); applySize(); };
  }
  applySize();

  // ---------- details on a click ----------------------------------------------------------
  const POINTS = ["f-points", "f-lights", "f-light-hit", "f-rot", "s52-info"];
  const LINES = ["f-line-mag", "f-line-magvar", "f-line-solid", "f-line-dash", "f-tss-line", "f-tss-bnd", ...S52_LINE_IDS];
  map.on("click", async (e) => {
    if (!enabled || placing()) return;
    // leave clicks on the app's own things (course, routings, fleet, marks, tide stations and
    // tidal stream diamonds) to them
    const pad = 6;
    const box = [[e.point.x - pad, e.point.y - pad], [e.point.x + pad, e.point.y + pad]];
    const own = map.queryRenderedFeatures(box).some((f) => /^(rt-|lib-|fl-|dl-|boat|tide-|ts-|stn-|ais-|alarm-|ruler-|bar-|trk-|evt-|ovl-|vd-|anc-|lay-)/.test(f.layer.id));
    if (own) return;
    // inside a fishing rule area, the rules box (marine.js) stands in for chart areas
    const inRule = map.getLayer("mar-rules-fill") && map.getLayoutProperty("mar-rules-fill", "visibility") !== "none"
      && map.queryRenderedFeatures(e.point, { layers: ["mar-rules-fill"] }).length > 0;
    const hit = map.queryRenderedFeatures(box, { layers: POINTS }).concat(
      map.queryRenderedFeatures(box, { layers: LINES }), inRule ? [] : map.queryRenderedFeatures(e.point, { layers: ["f-area-hit"] }));
    // several features at one spot (a beacon with its light): show them all; on open water,
    // the depth (or dredged) area of the largest-scale chart there
    // nearest the click first (a mark and its light together), lines and areas after the
    // points: a crowded spot (rocks, a coarser chart's copy) mustn't push the light off
    const dist = (f) => {
      if (f.geometry?.type !== "Point") return 1e6;
      const q = map.project(f.geometry.coordinates);
      return Math.round(Math.hypot(q.x - e.point.x, q.y - e.point.y) / 3);
    };
    const near = hit.map((f, i) => [dist(f), i, f]).sort((a, b) => a[0] - b[0] || a[1] - b[1]).map((x) => x[2]);
    const seen = new Set();
    const list = near.filter((f) => { const k = `${f.properties.t}/${f.properties.fid}`; if (seen.has(k)) return false; seen.add(k); return true; }).slice(0, 5);
    const infos = list.length
      ? (await Promise.all(list.map((f) => invoke("chart_feature", { t: f.properties.t, fid: f.properties.fid }).catch(() => null)))).filter(Boolean)
      : [await invoke("chart_depth_at", { lon: e.lngLat.lng, lat: e.lngLat.lat }).catch(() => null)].filter(Boolean);
    if (!infos.length) return;
    const el = document.createElement("div");
    el.className = "feat-pop";
    for (const info of infos) {
      const h = document.createElement("h4");
      const name = info.attrs.find(([k]) => k === "Name")?.[1];
      // the catch-all table for objects without a symbol: name the object by its class
      const kind = info.code === "CHTOBJ" ? OBJECT_NAMES[info.attrs.find(([k]) => k === "S-57 object class")?.[1]] ?? info.name : info.name;
      h.textContent = name ? `${kind}: ${name}` : kind;
      el.append(h);
      const rows = [...info.attrs];
      if (rows.some(([k]) => k === "Magnetic variation")) {
        const w = magneticVariation(e.lngLat.lat, e.lngLat.lng);
        if (w) rows.push([`World Magnetic Model ${Math.floor(w.year)}`, `${fmtVariation(w.variation)} (${w.annualChange >= 0 ? "+" : "−"}${Math.abs(w.annualChange * 60).toFixed(1)}′ a year)`]);
      }
      const tbl = document.createElement("table");
      for (const [k, v] of rows) {
        if (k === "Name") continue;
        const tr = tbl.insertRow();
        tr.insertCell().textContent = k;
        tr.insertCell().textContent = v;
      }
      el.append(tbl);
      // the chart notes the feature names (TXTDSC, NTXTDS), from the ENC exchange sets
      for (const [k, v] of rows) {
        if (!["Text file", "Text file (local)", "Picture"].includes(k) || !v) continue;
        el.append(chartNote(String(v)));
      }
    }
    const note = document.createElement("div");
    note.className = "wx-note";
    note.textContent = "LINZ chart data, CC BY 4.0. Not for navigation.";
    el.append(note);
    infoBox(map, "chart", { width: 360, height: 360 }).open(el, { title: infos.length > 1 ? `${infos.length} chart features` : infos[0].name, at: [e.lngLat.lng, e.lngLat.lat] });
  });
  for (const id of [...POINTS, ...LINES]) {
    map.on("mouseenter", id, () => { if (!placing()) map.getCanvas().style.cursor = "pointer"; });
    map.on("mouseleave", id, () => { if (!placing()) map.getCanvas().style.cursor = ""; });
  }

  return {
    setEnabled: (on) => { enabled = on; apply(); },
    /** View the chart as at another date (a Date), or today (null). */
    setChartDate: (d, to = null) => setDates(d, to),
    /** Whether the IHO symbols are in use (Symbols: IHO, and loaded). */
    usingS52: () => useS52(),
    /** Draw the IHO symbols, lines and areas in a palette ("Day", "Dusk", "Night"). */
    /** Draw the IHO symbols, lines and areas in a palette ("Day", "Dusk", "Night"), once
     *  loaded (loadS52Palette): the images on the map are redrawn in place, all at once. */
    usePalette: (bundle) => {
      if (!bundle || S52.palette === bundle.palette) return;
      Object.assign(S52, bundle);
      for (const id of map.listImages()) {
        if (!id.startsWith("s52")) continue;
        const img = featureImage(id);
        const old = map.getImage(id);
        if (img && old && img.width === old.data.width && img.height === old.data.height) map.updateImage(id, img);
        else map.removeImage(id);
      }
      apply();
    },
    /** Underwater rocks, wrecks and obstructions deeper than this lose their danger symbol.
     *  `shallow`: the isolated danger symbol also for dangers in water shallower than the
     *  safety contour (S-52 "isolated dangers in shallow water", off unless chosen). */
    setSafetyContour: (m, shallow = false) => {
      shallowDangers = shallow;
      baseFilter["f-points"] = pointFilter(m); baseFilter["f-labels"] = labelFilter(m); map.setFilter("f-labels", labelFilter(m));
      contour = m;
      applySymbols();
      map.setLayoutProperty("f-points", "symbol-sort-key", pointSortKey(m));
      apply();
    },
    /** Routing options: which charted hazards to keep off. */
    avoid: () => ({ dangers: opts.avoidDangers, farms: opts.avoidFarms }),
  };
}

// ---------- symbols ----------------------------------------------------------
// Drawn at twice the size (pixelRatio 2). Simplified paper-chart (INT 1) symbols, with the
// S-52 isolated danger and unknown-object symbols. (The S-52 Presentation Library set is to come.)

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

// information symbols are drawn at twice the size of the old ones (more for the small
// round ones), with thinner lines for the size, so they read beside buoys and rocks
// (the caution "!", marine farm, anchor berth and harbour facility were half the size of the rest)
const SMALL_K = { an: 1.5, ab: 1.8 };
function small(kind) {
  const k = SMALL_K[kind] ?? 2, n = Math.ceil(30 * k);
  const [c, g] = canvas(n, n);
  g.scale(k, k);
  const cx = 15, cy = 15;
  g.lineWidth = 2 * Math.min(1, 1.4 / k); g.strokeStyle = INK; g.fillStyle = INK;
  switch (kind) {
    case "lm": g.beginPath(); g.arc(cx, cy, 7, 0, 7); g.stroke(); g.beginPath(); g.arc(cx, cy, 2, 0, 7); g.fill(); break;
    case "rc": g.strokeStyle = MAGENTA; g.lineWidth = 2.5; g.beginPath(); g.arc(cx, cy, 11, 0, 7); g.stroke(); break;
    case "pb": g.strokeStyle = MAGENTA; g.beginPath(); g.arc(cx, cy, 10, 0, 7); g.stroke(); g.beginPath(); g.moveTo(cx, cy - 6); g.lineTo(cx + 5, cy); g.lineTo(cx, cy + 6); g.lineTo(cx - 5, cy); g.closePath(); g.fillStyle = MAGENTA; g.fill(); break;
    case "an": case "ab": anchor(g, cx, cy, kind === "an" ? 1 : 0.75); break;
    case "mo": g.beginPath(); g.arc(cx, cy + 3, 5, 0, 7); g.fill(); g.beginPath(); g.moveTo(cx, cy - 2); g.lineTo(cx, cy - 9); g.stroke(); break;
    case "pi": g.beginPath(); g.arc(cx, cy, 3, 0, 7); g.fill(); break;
    case "pf": g.strokeRect(cx - 6, cy - 6, 12, 12); g.beginPath(); g.arc(cx, cy, 2, 0, 7); g.fill(); break;
    case "mf": g.strokeStyle = MAGENTA; g.strokeRect(cx - 8, cy - 6, 16, 12); g.beginPath(); g.moveTo(cx - 8, cy); g.lineTo(cx + 8, cy); g.moveTo(cx, cy - 6); g.lineTo(cx, cy + 6); g.stroke(); break;
    case "ca": // caution: a magenta "!" about as tall as the pilot boarding circle
      g.fillStyle = MAGENTA; g.beginPath(); g.moveTo(cx - 2.2, cy - 10); g.lineTo(cx + 2.2, cy - 10); g.lineTo(cx + 1.2, cy + 4); g.lineTo(cx - 1.2, cy + 4); g.closePath(); g.fill();
      g.beginPath(); g.arc(cx, cy + 8.2, 1.9, 0, 7); g.fill(); break;
    case "cp": g.strokeStyle = MAGENTA; g.beginPath(); g.arc(cx, cy, 9, 0, 7); g.stroke(); g.beginPath(); g.moveTo(cx - 5, cy); g.lineTo(cx + 5, cy); g.lineTo(cx + 1, cy - 4); g.stroke(); break;
    case "rs": g.fillStyle = COLOURS[3]; g.fillRect(cx - 2.5, cy - 8, 5, 16); g.fillRect(cx - 8, cy - 2.5, 16, 5); break;
    case "ss": g.beginPath(); g.arc(cx, cy + 4, 4, 0, 7); g.stroke(); g.beginPath(); g.moveTo(cx, cy); g.lineTo(cx, cy - 11); g.lineTo(cx + 8, cy - 8); g.lineTo(cx, cy - 5); g.stroke(); break;
    case "hf": anchor(g, cx, cy, 0.7, "#5b6b75"); break;
    case "fg": g.strokeStyle = MAGENTA; for (const r of [5, 9, 13]) { g.beginPath(); g.arc(cx - 6, cy, r, -0.6, 0.6); g.stroke(); } break;
    case "iso": // isolated danger (S-52 ISODGR01): a magenta ring with a cross, on a white disc so it shows on any depth colour
      g.fillStyle = "rgba(255,255,255,0.85)"; g.beginPath(); g.arc(cx, cy, 8, 0, 7); g.fill();
      g.strokeStyle = MAGENTA; g.lineWidth = 1.8; g.beginPath(); g.arc(cx, cy, 8, 0, 7); g.stroke();
      g.beginPath(); g.moveTo(cx - 5, cy - 5); g.lineTo(cx + 5, cy + 5); g.moveTo(cx + 5, cy - 5); g.lineTo(cx - 5, cy + 5); g.stroke(); break;
    case "uk": // an object TweedPlot has no symbol for (S-52 QUESMRK1): click it for what it is
      g.fillStyle = MAGENTA; g.font = `700 22px "Segoe UI", system-ui, sans-serif`; g.textAlign = "center"; g.textBaseline = "middle";
      g.lineWidth = 3; g.strokeStyle = "rgba(255,255,255,0.9)"; g.strokeText("?", cx, cy + 1); g.fillText("?", cx, cy + 1); break;
    case "mv": // magnetic variation: the half arrow of the paper chart's compass rose, pointing to magnetic north
      g.strokeStyle = MAGENTA; g.lineWidth = 1.8; g.beginPath();
      g.moveTo(cx, cy + 12); g.lineTo(cx, cy - 12); g.lineTo(cx + 6, cy - 3); g.stroke();
      g.beginPath(); g.arc(cx, cy + 12, 1.8, 0, 7); g.fillStyle = MAGENTA; g.fill(); break;
    case "ts": // traffic lane direction (S-52 TSSLPT51): an open magenta arrow pointing up, turned to the lane's ORIENT
      g.strokeStyle = MAGENTA; g.lineWidth = 1.8; g.beginPath();
      g.moveTo(cx, cy - 13); g.lineTo(cx + 7, cy - 4); g.lineTo(cx + 3, cy - 4); g.lineTo(cx + 3, cy + 13); g.lineTo(cx - 3, cy + 13); g.lineTo(cx - 3, cy - 4); g.lineTo(cx - 7, cy - 4); g.closePath(); g.stroke(); break;
    default: g.beginPath(); g.arc(cx, cy, 3, 0, 7); g.fill();
  }
  return g.getImageData(0, 0, n, n);
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

/** Text drawn as an image: l = light, b = mark name, d = depth, n = area name, t = tide station, r = ruler. */
function text(style, t) {
  const fonts = {
    l: `600 19px "Segoe UI", system-ui, sans-serif`,
    b: `italic 18px "Segoe UI", system-ui, sans-serif`,
    d: `600 19px "Segoe UI", system-ui, sans-serif`,
    n: `italic 600 21px "Segoe UI", system-ui, sans-serif`,
    t: `600 18px "Segoe UI", system-ui, sans-serif`,
    r: `700 20px "Segoe UI", system-ui, sans-serif`,
    m: `600 17px "Segoe UI", system-ui, sans-serif`,
    f: `600 17px "Segoe UI", system-ui, sans-serif`,
  };
  // m: maritime boundaries, f: fisheries areas (marine.js)
  const ink = { l: MAGENTA, b: INK, d: INK, n: "#2d5f8a", t: "#1e6fb5", r: "#a3134d", m: "#5b2c83", f: "#2e6f8e" };
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
  if (id.startsWith("s52sec:")) return s52SectorImage(id.slice(7));
  if (id.startsWith("s52t:")) return s52TextImage(id.slice(5));
  if (id.startsWith("s52:")) return s52Image(id.slice(4));
  if (id.startsWith("s52l:")) return s52LineImage(id.slice(5));
  if (id.startsWith("s52f:")) return s52FillImage(id.slice(5));
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
