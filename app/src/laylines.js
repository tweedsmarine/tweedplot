// Laylines and the tacking cone to the target mark: the mark chosen with "Go to here", else the
// next mark of the route.
//
// Upwind laylines leave the mark downwind of it at the true wind direction + 180° ± the best upwind
// true wind angle (from the polar at the current true wind speed; else half the tacking angle set
// in Settings); sail on the one for your tack until you reach the other. Downwind laylines do the
// same for gybing. With the boat known, only the pair that applies is shown. The tacking cone is
// a wedge from the mark back toward the boat: tack at its edge and the tacks are long far out
// and short near the mark.
import { invoke } from "@tauri-apps/api/core";
import { rhumb, rhumbDestination, fmtBrg, fmtNm } from "./navmath.js";

const PORT = "#d7262b", STBD = "#1f9a3a", CONE = "#2a62c9", GOTO = "#7b1fa2";
const empty = { type: "FeatureCollection", features: [] };
const norm360 = (d) => ((d % 360) + 360) % 360;
const diff180 = (a, b) => ((a - b + 540) % 360) - 180;

export function setupLaylines(map, { before, getTarget, getBoat, getWind, getSettings, getVariation }) {
  for (const s of ["lay-lines", "lay-cone", "lay-labels", "lay-goto"]) map.addSource(s, { type: "geojson", data: empty });
  map.addLayer({ id: "lay-cone-fill", type: "fill", source: "lay-cone", filter: ["==", ["geometry-type"], "Polygon"],
    paint: { "fill-color": CONE, "fill-opacity": 0.12 } }, before);
  map.addLayer({ id: "lay-cone-line", type: "line", source: "lay-cone",
    paint: { "line-color": CONE, "line-width": 1.4, "line-dasharray": [4, 3] } }, before);
  map.addLayer({ id: "lay-goto-line", type: "line", source: "lay-goto", filter: ["==", ["geometry-type"], "LineString"],
    paint: { "line-color": GOTO, "line-width": 1.6, "line-dasharray": [1, 2] } }, before);
  map.addLayer({ id: "lay-line", type: "line", source: "lay-lines", layout: { "line-cap": "round" },
    paint: { "line-color": ["get", "colour"], "line-width": 2, "line-opacity": 0.9,
             "line-dasharray": ["case", ["get", "gybe"], ["literal", [3, 2]], ["literal", [1, 0]]] } }, before);
  map.addLayer({ id: "lay-goto-pt", type: "circle", source: "lay-goto", filter: ["==", ["geometry-type"], "Point"],
    paint: { "circle-radius": 9, "circle-color": "rgba(0,0,0,0)", "circle-stroke-color": GOTO, "circle-stroke-width": 2.5 } }, before);
  map.addLayer({ id: "lay-lbl", type: "symbol", source: "lay-labels",
    layout: { "icon-image": ["concat", "txt:r:", ["get", "lbl"]], "icon-anchor": ["get", "anchor"], "icon-offset": ["get", "offset"],
              "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);

  const layers = ["lay-cone-fill", "lay-cone-line", "lay-goto-line", "lay-line", "lay-goto-pt", "lay-lbl"];
  let seq = 0;

  function clear() {
    for (const s of ["lay-lines", "lay-cone", "lay-labels", "lay-goto"]) map.getSource(s)?.setData(empty);
  }

  /** The upwind and downwind true wind angles to lay: the polar's best VMG, else from the angles in Settings. */
  async function angles(set, tws) {
    if (set.polar && Number.isFinite(tws)) {
      const a = await invoke("polar_vmg_angles", { path: set.polar, tws }).catch(() => null);
      if (a) return { up: a.up, down: a.down, from: "polar" };
    }
    const tack = Number(set.layTack) || 120, gybe = Number(set.layGybe) || 60;
    return { up: tack / 2, down: 180 - gybe / 2, from: "set" };
  }

  const line = (a, b, props) => ({ type: "Feature", properties: props, geometry: { type: "LineString", coordinates: [a, b] } });
  const label = (at, lbl, anchor = "bottom-left", offset = [6, -4]) =>
    ({ type: "Feature", properties: { lbl, anchor, offset }, geometry: { type: "Point", coordinates: at } });
  const bearingText = (brg, at) => {
    const v = getVariation?.(at);
    return `${fmtBrg(brg)}°T${Number.isFinite(v) ? ` ${fmtBrg(brg - v)}°M` : ""}`;
  };

  async function refresh() {
    const my = ++seq;
    const set = getSettings();
    const target = getTarget();
    if (!target) { clear(); return; }
    const mark = [target.lon, target.lat];
    const boat = getBoat();
    const labels = [], goto = [];

    // the mark itself, and the line to it from the boat
    if (target.goto) {
      goto.push({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: mark } });
      if (boat) {
        goto.push(line(boat, mark, {}));
        const { brg, nm } = rhumb(boat, mark);
        labels.push(label(mark, `${target.name}  ${bearingText(brg, boat)}  ${fmtNm(nm)}`, "top-left", [10, 8]));
      } else labels.push(label(mark, target.name, "top-left", [10, 8]));
    }
    map.getSource("lay-goto").setData({ type: "FeatureCollection", features: goto });

    const showLines = set.layShow, showCone = set.layConeShow;
    const finish = (lines, cone) => {
      map.getSource("lay-lines").setData(lines);
      map.getSource("lay-cone").setData(cone);
      map.getSource("lay-labels").setData({ type: "FeatureCollection", features: labels });
    };
    if (!showLines && !showCone) { finish(empty, empty); return; }

    const wind = await getWind(target);
    if (my !== seq) return;
    if (!wind || !Number.isFinite(wind.twd)) { finish(empty, empty); return; }
    const { up, down } = await angles(set, wind.tws);
    if (my !== seq) return;

    // how far the lines run: past the boat, and across the view
    const b = map.getBounds();
    const view = rhumb([b.getWest(), b.getSouth()], [b.getEast(), b.getNorth()]).nm;
    const toBoat = boat ? rhumb(mark, boat) : null;
    // the lines are as long as the range from the boat to the mark, so their labels stay on screen
    const len = toBoat ? Math.max(0.05, toBoat.nm) : Math.min(300, Math.max(1, view * 0.6));

    // which pair applies: the boat downwind of the mark beats up to it; upwind of it, it runs down
    const downwindSide = toBoat ? Math.abs(diff180(toBoat.brg, wind.twd + 180)) < 90 : null;
    const pairs = [];
    if (downwindSide !== false) pairs.push({ twa: up, gybe: false });
    if (downwindSide !== true) pairs.push({ twa: down, gybe: true });

    const lines = [];
    for (const p of showLines ? pairs : []) {
      for (const stbd of [true, false]) {
        // starboard tack: the wind on the starboard side, heading = TWD − TWA; port tack: TWD + TWA
        const heading = norm360(wind.twd + (stbd ? -p.twa : p.twa));
        const end = rhumbDestination(mark, heading + 180, len);
        lines.push(line(mark, end, { colour: stbd ? STBD : PORT, gybe: p.gybe }));
        const word = p.gybe ? (stbd ? "Stbd gybe" : "Port gybe") : (stbd ? "Stbd tack" : "Port tack");
        labels.push(label(rhumbDestination(mark, heading + 180, len * 0.5), `${word} ${bearingText(heading, mark)}`, "bottom", [0, -6]));
      }
    }
    const lineData = { type: "FeatureCollection", features: lines };

    // the tacking cone: apex at the mark, axis toward the boat (else toward where the leg began)
    const axisTo = boat ?? target.from ?? null;
    const half = Number(set.layCone);
    let cone = empty;
    if (showCone && axisTo && half > 0) {
      const axis = rhumb(mark, axisTo);
      const r = Math.max(0.1, axis.nm);
      const arc = [];
      for (let a = -half; a <= half + 1e-9; a += Math.max(1, half / 6)) arc.push(rhumbDestination(mark, axis.brg + a, r));
      cone = { type: "FeatureCollection", features: [{ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [[mark, ...arc, mark]] } }] };
      labels.push(label(arc[Math.floor(arc.length / 2)], `Cone ±${half}°`, "top", [0, 6]));
    }
    finish(lineData, cone);
  }

  map.on("moveend", () => { if (getTarget()) refresh(); }); // the lines run across the view
  return { refresh, clear, layers };
}
