// True and apparent wind arrows: on the boat when live (from the instruments),
// on the cursor when planning (true wind from the GRIB shown; apparent wind for
// the heading and polar speed of the active routing near the cursor, else for
// a course towards the mark of the nearest course leg).
//
// Each arrow points the way the wind blows, its tip on the boat (or cursor) and
// its tail upwind; its length grows with the wind speed.

const KEY = "racenav.windarrows";
const TRUE_COLOUR = "#1565c0";
const APP_COLOUR = "#e65100";
const rad = (d) => (d * Math.PI) / 180;
const deg = (r) => ((r * 180) / Math.PI + 360) % 360;

/** Apparent wind (from-direction °T, speed kn) for true wind from `twd` at `tws` and the boat on `hdg` at `bsp`. */
export function apparent(twd, tws, hdg, bsp) {
  // air movement over the ground (blowing towards twd + 180), minus the boat's own movement
  const ax = Math.sin(rad(twd + 180)) * tws - Math.sin(rad(hdg)) * bsp;
  const ay = Math.cos(rad(twd + 180)) * tws - Math.cos(rad(hdg)) * bsp;
  return { awd: deg(Math.atan2(-ax, -ay)), aws: Math.hypot(ax, ay) };
}

function arrowImage() {
  const w = 36, h = 110, c = document.createElement("canvas");
  c.width = w; c.height = h;
  const g = c.getContext("2d");
  g.fillStyle = "#000";
  g.beginPath(); // head (tip at the top)
  g.moveTo(w / 2, 2); g.lineTo(w - 3, 34); g.lineTo(w / 2 + 4.5, 28); g.lineTo(w / 2 + 4.5, h - 3);
  g.lineTo(w / 2 - 4.5, h - 3); g.lineTo(w / 2 - 4.5, 28); g.lineTo(3, 34); g.closePath();
  g.fill();
  return g.getImageData(0, 0, w, h);
}

export function setupWindArrows(map, { before }) {
  let on = true;
  try { on = JSON.parse(localStorage.getItem(KEY) ?? "true"); } catch { /* ignore */ }
  map.addImage("wind-arrow", arrowImage(), { sdf: true, pixelRatio: 2 });
  map.addSource("wind-arrows", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "wind-arrows", type: "symbol", source: "wind-arrows",
    layout: { "icon-image": "wind-arrow", "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map", "icon-anchor": "top",
              "icon-size": ["get", "size"], "icon-allow-overlap": true, "icon-ignore-placement": true },
    paint: { "icon-color": ["get", "colour"], "icon-halo-color": "#ffffff", "icon-halo-width": 1.2, "icon-opacity": 0.9 } }, before);

  const size = (kn) => 0.55 + (Math.min(Math.max(kn, 0), 40) / 40) * 1.25;
  function show(pos, tw, aw) {
    const f = [];
    if (on && pos) {
      if (tw && Number.isFinite(tw.twd) && Number.isFinite(tw.tws)) f.push({ type: "Feature", properties: { rot: (tw.twd + 180) % 360, size: size(tw.tws), colour: TRUE_COLOUR, kind: "true" }, geometry: { type: "Point", coordinates: pos } });
      if (aw && Number.isFinite(aw.awd) && Number.isFinite(aw.aws)) f.push({ type: "Feature", properties: { rot: (aw.awd + 180) % 360, size: size(aw.aws), colour: APP_COLOUR, kind: "apparent" }, geometry: { type: "Point", coordinates: pos } });
    }
    map.getSource("wind-arrows").setData({ type: "FeatureCollection", features: f });
  }

  return {
    enabled: () => on,
    setEnabled(v) { on = !!v; try { localStorage.setItem(KEY, JSON.stringify(on)); } catch { /* ignore */ } if (!on) show(null); },
    /** Live: from the instruments (v.twd/v.tws, v.awa/v.aws relative to the heading). */
    live(pos, v) {
      const hdg = v.heading ?? v.cog;
      const twd = v.twd ?? (hdg != null && v.twa != null ? (hdg + v.twa + 360) % 360 : null);
      const tw = twd != null && v.tws != null ? { twd, tws: v.tws } : null;
      const aw = hdg != null && v.awa != null && v.aws != null ? { awd: (hdg + v.awa + 360) % 360, aws: v.aws } : null;
      show(pos, tw, aw);
    },
    /** Planning: true wind at the cursor, and apparent wind for a heading and boat speed if known. */
    planning(pos, tw, hdg, bsp) {
      const aw = tw && hdg != null && bsp != null ? apparent(tw.twd, tw.tws, hdg, bsp) : null;
      show(pos, tw, aw);
    },
    clear() { show(null); },
  };
}
