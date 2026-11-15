// Magnetic variation diagram: a true north line with the magnetic north vector beside it and
// the variation and its annual change as text. It is fixed to the screen, not the chart: 25 mm
// high by the screen calibration at any zoom, dragged to wherever you want it, and it turns
// with the chart. The variation is that of the chart position under it. Position and on/off are
// saved between runs.
//
// It is a plain element in the page (not in the map), so the day, dusk and night palettes treat
// it like the other panels: drawn dark on light here, the colour filters do the rest.
import { fmtVariation } from "./wmm.js";
import { s52PxPerMm } from "./chartfeatures.js";

const KEY = "racenav.vardiagram";
const INK = "#a3134d";
const HEIGHT_MM = 25;
const NS = "http://www.w3.org/2000/svg";

export function setupVarDiagram(map, { getVariation, getAnnualChange }) {
  let state = null; // { x, y }: the diagram's anchor (the foot of the lines), in page pixels
  try { state = JSON.parse(localStorage.getItem(KEY) || "null"); } catch { /* none */ }
  const save = () => { try { if (state) localStorage.setItem(KEY, JSON.stringify(state)); else localStorage.removeItem(KEY); } catch { /* ignore */ } };

  const svg = document.createElementNS(NS, "svg");
  svg.id = "vardiagram";
  svg.style.cssText = "display:none;position:fixed;z-index:5;pointer-events:none;overflow:visible;touch-action:none;user-select:none";
  const el = (name, attrs, parent = svg) => {
    const e = document.createElementNS(NS, name);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    parent.append(e);
    return e;
  };
  const stroke = { fill: "none", "stroke-linecap": "round", "stroke-linejoin": "round" };
  // a white halo under each mark so it reads on any chart colour
  const trueHalo = el("polyline", { ...stroke, stroke: "#fff", "stroke-width": 5, "stroke-opacity": 0.9 });
  const magHalo = el("polyline", { ...stroke, stroke: "#fff", "stroke-width": 5, "stroke-opacity": 0.9 });
  const trueLine = el("polyline", { ...stroke, stroke: INK, "stroke-width": 1.6 });
  const magLine = el("polyline", { ...stroke, stroke: INK, "stroke-width": 2.2 });
  const label = el("text", { "text-anchor": "middle", "font-size": 13, "font-weight": 600, "font-family": "Segoe UI, system-ui, sans-serif",
    fill: INK, stroke: "#fff", "stroke-width": 4, "paint-order": "stroke", "stroke-linejoin": "round" });
  // wide invisible strokes to grab it by
  const grab = el("polyline", { ...stroke, stroke: "transparent", "stroke-width": 18 });
  const grab2 = el("polyline", { ...stroke, stroke: "transparent", "stroke-width": 18 });
  for (const g of [grab, grab2, label]) { g.style.pointerEvents = "all"; g.style.cursor = "move"; }
  document.body.append(svg);

  const pts = (a) => a.map((p) => `${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(" ");
  /** The point `px` pixels from `o` at `deg` clockwise of true north: the map may be turned. */
  const offset = (o, deg, px) => {
    const a = (deg - map.getBearing()) * Math.PI / 180;
    return [o[0] + px * Math.sin(a), o[1] - px * Math.cos(a)];
  };

  function clamp() {
    const len = HEIGHT_MM * s52PxPerMm();
    state.x = Math.max(len * 0.3, Math.min(window.innerWidth - len * 0.3, state.x));
    state.y = Math.max(len * 1.2, Math.min(window.innerHeight - len * 0.5, state.y));
  }

  function draw() {
    if (!state) { svg.style.display = "none"; return; }
    svg.style.display = "";
    clamp();
    const len = HEIGHT_MM * s52PxPerMm();
    const head = len * 0.14;
    const o = [state.x, state.y];
    // the variation where the diagram sits on the chart
    const rect = map.getContainer().getBoundingClientRect();
    const ll = map.unproject([o[0] - rect.left, o[1] - rect.top]);
    const at = [ll.lng, ll.lat];
    const v = getVariation(at);
    const n = offset(o, 0, len);
    const tShape = [o, n, offset(n, 155, head), n, offset(n, -155, head)];
    trueLine.setAttribute("points", pts(tShape));
    trueHalo.setAttribute("points", pts(tShape));
    grab.setAttribute("points", pts([o, n]));
    let text = "True north";
    if (v != null) {
      // magnetic north: the variation vector, a half arrow head
      const m = offset(o, v, len);
      const mShape = [o, m, offset(m, v + 155, head)];
      magLine.setAttribute("points", pts(mShape));
      magHalo.setAttribute("points", pts(mShape));
      grab2.setAttribute("points", pts([o, m]));
      const ch = getAnnualChange?.(at);
      text = `Var ${fmtVariation(v, 1)}${Number.isFinite(ch) ? ` ${ch >= 0 ? "inc" : "dec"} ${Math.abs(ch * 60).toFixed(0)}′ annually` : ""}`;
    } else {
      for (const p of [magLine, magHalo, grab2]) p.setAttribute("points", "");
    }
    label.textContent = text;
    label.setAttribute("x", o[0].toFixed(1));
    label.setAttribute("y", (o[1] + 17).toFixed(1));
    svg.style.left = "0"; svg.style.top = "0";
    svg.setAttribute("width", "1"); svg.setAttribute("height", "1");
  }

  // dragging
  let drag = null;
  svg.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || !state || drag) return;
    drag = { id: e.pointerId, dx: state.x - e.clientX, dy: state.y - e.clientY };
    svg.setPointerCapture(e.pointerId);
    e.preventDefault();
  });
  svg.addEventListener("pointermove", (e) => {
    if (!drag || e.pointerId !== drag.id || !state) return;
    state.x = e.clientX + drag.dx; state.y = e.clientY + drag.dy;
    draw();
  });
  const end = (e) => { if (drag?.id === e.pointerId) { drag = null; save(); } };
  svg.addEventListener("pointerup", end);
  svg.addEventListener("pointercancel", end);
  svg.addEventListener("lostpointercapture", end);

  // a right-click on it goes to the chart's menu (to remove it); `over` says it was on the diagram
  let over = false;
  svg.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    const rect = map.getContainer().getBoundingClientRect();
    const point = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    over = true;
    try {
      map.fire("contextmenu", { originalEvent: e, point, lngLat: map.unproject([point.x, point.y]) });
    } finally { over = false; }
  });

  // fixed on the screen, but the chart under it changes: redraw (it turns with the chart, and its variation follows the chart)
  map.on("move", draw);
  map.on("rotate", draw);
  window.addEventListener("resize", draw);
  window.addEventListener("racenav-calibration", draw);
  setInterval(draw, 30000);
  draw();

  return {
    /** Put the diagram at a screen point of the map (shown if it wasn't). */
    place(point) {
      const rect = map.getContainer().getBoundingClientRect();
      state = { x: point.x + rect.left, y: point.y + rect.top + HEIGHT_MM * s52PxPerMm() * 0.5 };
      save(); draw();
    },
    /** Whether the right-click being handled is on the diagram. */
    hit: () => over,
    remove() {
      const pointerId = drag?.id;
      drag = null;
      if (pointerId != null && svg.hasPointerCapture(pointerId)) svg.releasePointerCapture(pointerId);
      state = null; save(); draw();
    },
    shown: () => !!state,
    refresh: draw,
  };
}
