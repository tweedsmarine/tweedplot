// The data panel's starting layouts, one for each view with a chart (Navigation, Planning) and
// each Use (Cruising, Fishing, Racing), and the one-time move of the old floating instrument
// panels into it. No screen here, so it can be tested (sidepresets.test.js).
//
// A layout is a list of sections:
//   { id, kind: "readings", title, cols: 1 | 2, items: [reading keys] }
//   { id, kind: "graphic", title, item: "WDIAL" | "HIGHWAY" | "SOUNDER" | "H_<history>" | "COMPASS", h: mm }
// The own-ship block (top), the MOB section and the alerts (bottom) are not in the list: they
// are always there.

export const PANEL_VIEWS = ["nav", "plan"];
export const USES = ["cruise", "fish", "race"];

/** The fixed own-ship block, in S-Mode's order (MSC.1/Circ.1609 App. 3), plus depth. */
export const OWN_SHIP = ["HDG", "COG", "SOG", "BSP", "DPT", "POS", "TIME"];

const R = (id, title, items, cols = 1) => ({ id, kind: "readings", title, cols, items });
const G = (id, title, item, h = 45) => ({ id, kind: "graphic", title, item, h });

const ROUTE = () => R("route", "Route", ["NEXT", "BTW", "DTW", "XTE", "TTG", "ETA", "VMC"], 2);
const WIND = () => R("wind", "Wind", ["TWS", "TWD", "AWS", "AWA", "TWA", "VMG"], 2);
const WEATHER = () => R("wx", "Weather", ["PRES", "PTEND", "AIRT", "WTEMP"], 2);
const CURSOR = () => R("cursor", "Cursor", ["CPOS", "CBRG", "CRNG", "CSND"], 2);
const GRIB = () => R("grib", "Forecast at the cursor", ["GTIME", "GTWS", "GTWD", "GGUST", "GMSLP", "GHS", "GTP", "GRAIN"], 2);

export const PRESETS = {
  nav: {
    cruise: () => [ROUTE(), WIND(), WEATHER()],
    race: () => [
      R("perf", "Performance", ["BSP", "TBSP", "PERF", "TWA", "TTWA", "VMG", "VMC", "THDG"], 2),
      G("dial", "Wind", "WDIAL", 48),
      R("route", "Route", ["NEXT", "BTW", "DTW", "TTG"], 2),
      R("wx", "Weather", ["TWD", "PRES", "PTEND"], 2),
    ],
    fish: () => [
      R("fish", "Sea", ["WTEMP", "TRIP", "PRES", "PTEND"], 2),
      G("snd", "Sounder", "SOUNDER", 40),
      R("wind", "Wind", ["TWS", "TWD"], 2),
      R("route", "Route", ["NEXT", "BTW", "DTW", "TTG"], 2),
    ],
  },
  plan: {
    cruise: () => [CURSOR(), GRIB(), R("route", "Route", ["NEXT", "BTW", "DTW", "TTG", "ETA"], 2)],
    race: () => [CURSOR(), GRIB(), R("route", "Route", ["NEXT", "BTW", "DTW", "TTG", "ETA"], 2), R("perf", "Targets", ["TTWA", "TBSP", "THDG"], 2)],
    fish: () => [CURSOR(), R("sea", "Sea at the cursor", ["GSST", "GSKIN", "GCHL", "GCUR", "GSET", "GSUBT"], 2), GRIB()],
  },
};

/** A fresh copy of the preset for a view and Use (Cruising when the Use is not set). */
export function preset(view, use) {
  const v = PRESETS[view] ?? PRESETS.nav;
  return (v[use] ?? v.cruise)().map((s) => ({ ...s }));
}

/** Only the readings and graphics that exist (an older layout may name a reading since removed). */
export function clean(sections, items) {
  const out = [];
  for (const s of Array.isArray(sections) ? sections : []) {
    if (!s || typeof s !== "object") continue;
    if (s.kind === "readings") {
      const list = (Array.isArray(s.items) ? s.items : []).filter((k) => items[k]);
      out.push({ id: String(s.id || `s${out.length}`), kind: "readings", title: String(s.title ?? ""), cols: s.cols === 2 ? 2 : 1, items: list, collapsed: !!s.collapsed });
    } else if (s.kind === "graphic" && items[s.item]) {
      const h = Number(s.h);
      out.push({ id: String(s.id || `s${out.length}`), kind: "graphic", title: String(s.title ?? ""), item: s.item, h: Number.isFinite(h) ? Math.max(20, Math.min(120, h)) : 45, collapsed: !!s.collapsed });
    }
  }
  return out;
}

// the floating panels TweedPlot itself put up (their readings are in the presets now)
const BUILT_IN = new Set(["p1", "p2", "pg", "pc", "ps", "pm", "phw", "prace", "pck", "pfish", "pdepth", "psounder"]);
const GRAPHIC_KEYS = new Set(["WDIAL", "COMPASS", "HIGHWAY", "SOUNDER"]);

/**
 * The user's own floating panels as sections, for each view they were shown in: a panel's
 * readings become a readings section (two columns when it had more than three), and its
 * graphics (wind dial, highway, sounder, history plots) a graphic section each.
 * Hidden panels, the ones TweedPlot added itself, and Instruments-only panels are left out.
 */
export function fromFloating(panels, items) {
  const out = { nav: [], plan: [] };
  for (const p of Array.isArray(panels) ? panels : []) {
    if (!p || BUILT_IN.has(p.id) || p.hidden || !Array.isArray(p.tiles)) continue;
    const views = (Array.isArray(p.views) ? p.views : PANEL_VIEWS).filter((v) => PANEL_VIEWS.includes(v));
    if (!views.length) continue;
    const name = String(p.name || "").trim();
    const readings = p.tiles.filter((k) => items[k] && !GRAPHIC_KEYS.has(k) && !items[k].graphic);
    const graphics = p.tiles.filter((k) => items[k] && (GRAPHIC_KEYS.has(k) || items[k].graphic));
    const secs = [];
    if (readings.length) secs.push(R(`m-${p.id}`, name || "My readings", readings, readings.length > 3 ? 2 : 1));
    graphics.forEach((k, i) => secs.push(G(`m-${p.id}-${i}`, items[k].label, k, k === "HIGHWAY" ? 70 : 45)));
    for (const v of views) out[v].push(...secs.map((s) => ({ ...s, items: s.items ? [...s.items] : undefined })));
  }
  return out;
}

/** Widths in millimetres (scaled by the text size), from the screen calibration. */
export const WIDTHS_MM = { narrow: 52, normal: 66, wide: 86 };
/** The width nearest a dragged one (px), as a width name. */
export function snapWidth(px, pxPerMm, scale = 1) {
  let best = "normal", d = Infinity;
  for (const [k, mm] of Object.entries(WIDTHS_MM)) {
    const dd = Math.abs(mm * pxPerMm * scale - px);
    if (dd < d) { d = dd; best = k; }
  }
  return best;
}
