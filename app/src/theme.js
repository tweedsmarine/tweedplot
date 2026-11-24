// Colours by name, for everything drawn outside the chart: plots, the sounder, colour keys, panels.
// Each palette (bright, day, dusk, night) gives the names its own values in style.css as CSS custom
// properties, --c-<name>, so a plot drawn here follows the palette like the panels around it.
//
//   SVG and HTML: use css("gust") → "var(--c-gust)" in a style; it changes with the palette, no redraw.
//   Canvas:       use col("gust") → the colour now; redraw when onTheme() calls back.
//   Data scales:  ramp("#30123b") → the colour as given by day, dimmed for dusk and night as the
//                 IHO palettes dim colours (the same rule as the overlays on the chart).
//
// The names are the same on every branch; only the values in style.css differ. Night red and night
// green keep the day values: the whole page is filtered into one hue then.

import { darken } from "./mappalette.js";

const page = typeof document !== "undefined"; // false in the unit tests (node)
const root = () => (page ? document.documentElement : { dataset: {} });
let cache = new Map();
let key = "";
const listeners = new Set();

/** The palette now, as the colour names see it: "day", "dusk" or "night". */
export function themeOf() {
  const p = root().dataset.palette ?? "day";
  return p === "dusk" ? "dusk" : p === "iho-night" ? "night" : "day";
}

function fresh() {
  const k = `${root().dataset.palette}|${root().dataset.obcTheme ?? ""}`;
  if (k !== key) { key = k; cache = new Map(); }
}

/** A named colour for SVG or HTML styles: follows the palette without a redraw. */
export const css = (name, fallback) => (fallback ? `var(--c-${name}, ${fallback})` : `var(--c-${name})`);

/** A named colour as it is now (for a canvas). `fallback` when the stylesheet doesn't name it. */
export function col(name, fallback = "#888") {
  fresh();
  let v = cache.get(name);
  if (v === undefined) {
    v = (page ? getComputedStyle(root()).getPropertyValue(`--c-${name}`).trim() : "") || fallback;
    cache.set(name, v);
  }
  return v;
}

/** The same colour with an opacity (0–1), for a canvas: works with #rgb, #rrggbb and rgb(). */
export function alpha(colour, a) {
  const c = colour.trim();
  let m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(c);
  if (m) {
    const h = m[1].length === 3 ? [...m[1]].map((x) => x + x).join("") : m[1];
    return `rgba(${parseInt(h.slice(0, 2), 16)},${parseInt(h.slice(2, 4), 16)},${parseInt(h.slice(4, 6), 16)},${a})`;
  }
  m = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(c);
  if (m) return `rgba(${m[1]},${m[2]},${m[3]},${a})`;
  return c;
}

// the dimming for data colours, as mappalette.js dims the overlays on the chart
const DIM = { dusk: { k: 0.75, kn: 0.5 }, night: { k: 0.45, kn: 0.25 } };
const rampCache = new Map();
/** A data-scale colour (#rrggbb or rgb(); depths, wind speeds, temperatures): its hue kept, dimmed for dusk and night. */
export function ramp(hex) {
  const t = themeOf();
  if (t === "day") return hex;
  const k = `${t}|${hex}`;
  let v = rampCache.get(k);
  if (!v) {
    const m = /^#([0-9a-f]{6})$/i.exec(hex), q = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(hex);
    const rgb = m ? [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16)) : q ? [+q[1], +q[2], +q[3]] : null;
    if (!rgb) return hex;
    const [r, g, b] = darken(rgb[0], rgb[1], rgb[2], DIM[t].k, DIM[t].kn);
    v = `rgb(${r},${g},${b})`;
    rampCache.set(k, v);
  }
  return v;
}

/** Call `fn` after the palette changes (a canvas redraws itself then). Returns a way to stop. */
export function onTheme(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

// palette.js and the OpenBridge theme change data-palette / data-obc-theme on the page
if (page) new MutationObserver(() => {
  const before = key;
  fresh();
  if (key !== before) for (const f of listeners) { try { f(themeOf()); } catch (e) { console.error(e); } }
}).observe(document.documentElement, { attributes: true, attributeFilter: ["data-palette", "data-obc-theme"] });

const SVG_PAINT = new Set(["fill", "stroke", "stop-color", "color"]);
/** Set an SVG attribute; a named colour (css()) goes in the style, where var() works. */
export function svgAttr(e, k, v) {
  if (SVG_PAINT.has(k) && typeof v === "string" && v.startsWith("var(")) e.style.setProperty(k, v);
  else e.setAttribute(k, v);
}
