// The data panel's starting layouts, one for each view with a chart (Navigation, Planning) and
// each Use (Cruising, Fishing, Racing), and the one-time move of the old floating instrument
// panels into it. No screen here, so it can be tested (sidepresets.test.js).
//
// A layout is a list of sections:
//   { id, kind: "readings", title, cols: 1 | 2, items: [reading keys], trend: show a 3-hour trend line under readings that have a history }
//   { id, kind: "graphic", title, item: "WDIAL" | "HIGHWAY" | "SOUNDER" | "H_<history>" | "COMPASS", h: mm }
//   { id, kind: "marks", title, n }   the route's next n marks, with distance and ETA
//   { id, kind: "targets", title, n } the n AIS targets that matter most (distress, danger, CPA)
// The own-ship block (top), the MOB section and the alerts (bottom) are not in the list: they
// are always there.

export const PANEL_VIEWS = ["nav", "plan", "env"];
export const USES = ["cruise", "fish", "race"];

/** The fixed own-ship block, in S-Mode's order (MSC.1/Circ.1609 App. 3), plus depth. */
export const OWN_SHIP = ["HDG", "COG", "SOG", "BSP", "DPT", "POS", "TIME"];

const R = (id, title, items, cols = 1, trend = false) => ({ id, kind: "readings", title, cols, items, ...(trend ? { trend: true } : {}) });
const G = (id, title, item, h = 45) => ({ id, kind: "graphic", title, item, h });
const M = (n = 5) => ({ id: "marks", kind: "marks", title: "Marks ahead", n });
const T = (n = 4) => ({ id: "targets", kind: "targets", title: "Targets", n });

const ROUTE = () => R("route", "Route", ["BTW", "DTW", "XTE", "VMC"], 2);
const WIND = () => R("wind", "Wind", ["TWS", "TWD", "AWS", "AWA", "TWA", "VMG"], 2, true);
const WEATHER = () => R("wx", "Weather", ["PRES", "PTEND", "AIRT", "WTEMP"], 2, true);
const CURSOR = () => R("cursor", "Cursor", ["CPOS", "CBRG", "CRNG", "CSND"], 2);
export const WEATHER_SECTIONS = {
  weatherTime: "Chart time",
  weather: "Weather layers and filters",
  // ("GRIB data at the cursor" was here: it is the readout on the chart now, readout.js)
};
export const weatherSection = (kind) => ({ id: kind, kind, title: WEATHER_SECTIONS[kind] });

export const PRESETS = {
  nav: {
    cruise: () => [ROUTE(), M(5), T(4), WIND(), WEATHER()],
    race: () => [
      R("perf", "Performance", ["BSP", "TBSP", "PERF", "TWA", "TTWA", "VMG", "VMC", "THDG"], 2),
      G("dial", "Wind", "WDIAL", 48),
      R("route", "Route", ["BTW", "DTW", "TTG"], 2),
      M(3),
      R("wx", "Weather", ["TWD", "PRES", "PTEND"], 2, true),
      T(3),
    ],
    fish: () => [
      R("fish", "Sea", ["WTEMP", "TRIP", "PRES", "PTEND"], 2, true),
      R("wind", "Wind", ["TWS", "TWD"], 2, true),
      R("route", "Route", ["NEXT", "BTW", "DTW", "TTG"], 2),
      T(5),
    ],
  },
  plan: {
    cruise: () => [weatherSection("weatherTime"), CURSOR(), R("route", "Route", ["BTW", "DTW", "TTG"], 2), M(8)],
    race: () => [weatherSection("weatherTime"), CURSOR(), R("route", "Route", ["BTW", "DTW", "TTG"], 2), M(8), R("perf", "Routing targets", ["TTWA", "TBSP", "THDG"], 2)],
    fish: () => [weatherSection("weatherTime"), CURSOR()],
  },
  env: {
    cruise: () => [weatherSection("weatherTime"), weatherSection("weather")],
    race: () => [weatherSection("weatherTime"), weatherSection("weather")],
    fish: () => [weatherSection("weatherTime"), weatherSection("weather")],
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
  const special = new Set();
  for (const s of Array.isArray(sections) ? sections : []) {
    if (!s || typeof s !== "object") continue;
    if (Object.hasOwn(WEATHER_SECTIONS, s.kind)) {
      if (!special.has(s.kind)) out.push(weatherSection(s.kind));
      special.add(s.kind);
    } else if (s.kind === "readings") {
      const list = (Array.isArray(s.items) ? s.items : []).filter((k) => items[k]);
      out.push({ id: String(s.id || `s${out.length}`), kind: "readings", title: String(s.title ?? ""), cols: s.cols === 2 ? 2 : 1, items: list, collapsed: !!s.collapsed, ...(s.trend ? { trend: true } : {}) });
    } else if (s.kind === "marks" || s.kind === "targets") {
      const n = Math.round(Number(s.n));
      out.push({ id: String(s.id || `s${out.length}`), kind: s.kind, title: String(s.title ?? ""), n: Number.isFinite(n) ? Math.max(1, Math.min(12, n)) : 5, collapsed: !!s.collapsed });
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
    const views = (Array.isArray(p.views) ? p.views : ["nav", "plan"]).filter((v) => Object.hasOwn(out, v));
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

// ---------- upgrading saved layouts ----------
// the section ids the starting layouts have used (a layout made only of these hasn't had sections added)
const PRESET_IDS = new Set(["route", "wind", "wx", "perf", "dial", "fish", "snd", "cursor", "grib", "sea", "marks", "targets"]);
export const PRESETS_VERSION = 8;
const GRIB_READINGS = new Set(["GTIME", "GSRC", "GTWS", "GTWD", "GGUST", "GMSLP", "GHS", "GTP", "GWVD", "GRAIN", "GTEMP", "GSST", "GSKIN", "GCHL", "GSSS", "GSUBT", "GSUBS", "GCUR", "GSET"]);
/**
 * Bring saved layouts up to the current presets (v2: no sounder in Navigation · Fishing; marks
 * ahead, targets and trend lines added). A layout the user hasn't added sections to is replaced by
 * the new preset, keeping which sections were folded; one with sections of their own only loses
 * the old preset's sounder section from Navigation · Fishing. Returns true if anything changed.
 */
export function upgrade(data) {
  if (!data || (data.presetsVersion ?? 1) >= PRESETS_VERSION) return false;
  const oldVersion = data.presetsVersion ?? 1;
  for (const [k, L] of Object.entries(data.layouts ?? {})) {
    if (!Array.isArray(L)) continue;
    const [view, use] = k.split(".");
    const own = L.some((sec) => !PRESET_IDS.has(sec?.id));
    if (oldVersion < 2 && !own && PRESETS[view] && view !== "plan") {
      const folded = new Set(L.filter((sec) => sec?.collapsed).map((sec) => sec.id));
      data.layouts[k] = preset(view, use).map((sec) => (folded.has(sec.id) ? { ...sec, collapsed: true } : sec));
    } else if (oldVersion < 2 && k === "nav.fish") {
      data.layouts[k] = L.filter((sec) => !(sec?.id === "snd" && sec.item === "SOUNDER"));
    }
    if (oldVersion < 3 && view === "plan") {
      data.layouts[k] = data.layouts[k].flatMap((sec) => {
        if (sec?.kind !== "readings" || !Array.isArray(sec.items)) return [sec];
        const items = sec.items.filter((item) => !GRIB_READINGS.has(item));
        if (!items.length && (sec.id === "grib" || sec.id === "sea")) return [];
        return [{ ...sec, items }];
      });
    }
    if (oldVersion < 4 && view === "plan") {
      data.layouts[k] = [weatherSection("weather"), ...data.layouts[k]];
    }
    if (oldVersion < 5 && data.layouts[k].some((sec) => sec?.kind === "weather") && !data.layouts[k].some((sec) => sec?.kind === "weatherTime")) {
      const i = data.layouts[k].findIndex((sec) => sec?.kind === "weather");
      data.layouts[k].splice(i, 0, weatherSection("weatherTime"));
    }
    if (oldVersion < 7) data.layouts[k] = data.layouts[k].filter((sec) => sec?.kind !== "gribFiles");
    if (oldVersion < 8 && view === "plan") {
      data.layouts[k] = data.layouts[k].filter((sec) => sec?.kind !== "weather" && sec?.kind !== "gribPoint");
      if (!data.layouts[k].some((sec) => sec?.kind === "weatherTime")) data.layouts[k].unshift(weatherSection("weatherTime"));
    }
  }
  data.presetsVersion = PRESETS_VERSION;
  return true;
}
