// The chart in the IHO dusk and night colours (S-52). The page's colour filter can't
// be used for the map then: it would change the IHO colours. So the map is left out of
// the filter (style.css), the chart layers are given the IHO palette's colours (main.js,
// chartfeatures.js), and everything else drawn on the map (routes, tracks, AIS, marks,
// weather, labels) is darkened here to suit: its paint colours as they are set, and the
// images it draws (their pixels), so a white halo goes dark and black ink goes grey.
// Nothing changes in day colours or with the paper chart symbols (the filter does it).

const COLOUR_PROPS = new Set(["background-color", "fill-color", "fill-outline-color", "line-color", "circle-color", "circle-stroke-color",
  "text-color", "text-halo-color", "icon-color", "icon-halo-color", "fill-extrusion-color"]);
const NAMED = { white: "#ffffff", black: "#000000", red: "#ff0000", green: "#008000", blue: "#0000ff", yellow: "#ffff00", orange: "#ffa500",
  grey: "#808080", gray: "#808080", magenta: "#ff00ff", cyan: "#00ffff" };

function parse(s) {
  if (typeof s !== "string") return null;
  const t = NAMED[s.trim().toLowerCase()] ?? s.trim();
  let m = /^#([0-9a-f]{3,8})$/i.exec(t);
  if (m) {
    let h = m[1];
    if (h.length <= 4) h = [...h].map((c) => c + c).join("");
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16), h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1];
  }
  m = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:[\s,/]+([\d.]+))?\s*\)$/i.exec(t);
  if (m) return [+m[1], +m[2], +m[3], m[4] === undefined ? 1 : +m[4]];
  return null;
}

/** The dark-palette version of a day colour: saturated colours keep their hue and are
 *  dimmed (`k`), greys swap light for dark (`kn`: what black becomes), as the IHO dusk
 *  and night palettes do with the day ones. */
export function darken(r, g, b, k, kn) {
  const mx = Math.max(r, g, b) / 255, mn = Math.min(r, g, b) / 255;
  const l = (mx + mn) / 2;
  const s = mx === mn ? 0 : (mx - mn) / (1 - Math.abs(2 * l - 1));
  const sat = Math.min(1, s * 1.4);
  // the target lightness: a mix of "dimmed" (colours) and "inverted" (greys)
  const l2 = sat * l * k + (1 - sat) * (1 - l) * kn;
  const f = l > 0 ? l2 / l : 0;
  if (sat < 0.05 || l === 0) { const v = Math.round(l2 * 255); return [v, v, v]; }
  return [Math.min(255, Math.round(r * f)), Math.min(255, Math.round(g * f)), Math.min(255, Math.round(b * f))];
}

const PROFILES = { Dusk: { k: 0.6, kn: 0.45, raster: 0.45 }, Night: { k: 0.25, kn: 0.17, raster: 0.2 } };

export function setupMapPalette(map) {
  const rawSet = map.setPaintProperty.bind(map);
  const rawAdd = map.addLayer.bind(map);
  const rawImage = map.addImage.bind(map);
  const orig = new Map(); // `${layer}\u0000${prop}` → the colour the app gave it
  const images = new Map(); // id → { img, opts }: images the app drew, to darken
  let prof = null; // the dark profile in use, or null
  let exempt = () => false; // layers drawn in the IHO colours already

  const colour = (c) => {
    const p = parse(c);
    if (!p) return c;
    const [r, g, b] = darken(p[0], p[1], p[2], prof.k, prof.kn);
    return p[3] < 1 ? `rgba(${r},${g},${b},${p[3]})` : `rgb(${r},${g},${b})`;
  };
  const deep = (v) => (Array.isArray(v) ? v.map(deep) : typeof v === "string" ? colour(v) : v);
  const out = (id, prop, v) => (prof && COLOUR_PROPS.has(prop) && !exempt(id) ? deep(v) : v);

  map.setPaintProperty = (id, prop, v, o) => {
    orig.set(`${id}\u0000${prop}`, v);
    return rawSet(id, prop, out(id, prop, v), o);
  };
  map.addLayer = (layer, before) => {
    if (layer.paint) {
      const paint = { ...layer.paint };
      for (const [k, v] of Object.entries(layer.paint)) {
        orig.set(`${layer.id}\u0000${k}`, v);
        paint[k] = out(layer.id, k, v);
      }
      layer = { ...layer, paint };
    }
    return rawAdd(layer, before);
  };
  const isData = (img) => img && typeof img.width === "number" && img.data && img.data.length === img.width * img.height * 4;
  const skipImage = (id) => /^(s52|sh:|dp:|cl:|pat:|dpat:)/.test(id);
  const darkImage = (img) => {
    const d = new Uint8ClampedArray(img.data);
    for (let i = 0; i < d.length; i += 4) {
      if (!d[i + 3]) continue;
      const [r, g, b] = darken(d[i], d[i + 1], d[i + 2], prof.k, prof.kn);
      d[i] = r; d[i + 1] = g; d[i + 2] = b;
    }
    return { width: img.width, height: img.height, data: d };
  };
  map.addImage = (id, img, opts = {}) => {
    if (!opts.sdf && isData(img) && !skipImage(id)) {
      images.set(id, { img, opts });
      if (prof) img = darkImage(img);
    }
    return rawImage(id, img, opts);
  };

  /** Darken for `palette` ("Dusk", "Night"), or back to the app's own colours (null).
   *  `exemptLayer(id)`: layers the caller colours itself. */
  function apply(palette, exemptLayer = () => false) {
    prof = PROFILES[palette] ?? null;
    exempt = exemptLayer;
    for (const l of map.getStyle()?.layers ?? []) {
      if (l.type === "raster") {
        const k = `${l.id}\u0000raster-brightness-max`;
        if (!orig.has(k)) orig.set(k, map.getPaintProperty(l.id, "raster-brightness-max") ?? 1);
        rawSet(l.id, "raster-brightness-max", prof && !exempt(l.id) ? prof.raster : orig.get(k));
        continue;
      }
      for (const prop of Object.keys(l.paint ?? {})) {
        if (!COLOUR_PROPS.has(prop)) continue;
        const k = `${l.id}\u0000${prop}`;
        if (!orig.has(k)) orig.set(k, map.getPaintProperty(l.id, prop));
        rawSet(l.id, prop, out(l.id, prop, orig.get(k)));
      }
    }
    for (const [id, { img }] of images) {
      if (!map.hasImage(id)) { images.delete(id); continue; }
      try { map.updateImage(id, prof ? darkImage(img) : img); } catch { /* changed size: leave it */ }
    }
  }
  return { apply };
}
