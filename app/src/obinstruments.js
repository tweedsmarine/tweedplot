// The Instruments view in OpenBridge's dress: the compass is OpenBridge's own instrument
// (obc-compass: heading up, heading and COG markers, the true wind from where it blows, the bearing
// to the next mark as the setpoint marker when navigating); the other
// tiles keep TweedPlot's drawings in OpenBridge's colours and type (style.css). The tiles, their
// layout and what they show are TweedPlot's (insttiles.js, instruments.js).

import "@oicl/openbridge-webcomponents/dist/navigation-instruments/compass/compass.js";
import { ITEMS } from "./instruments.js";

const twdOf = (c) => c.v.twd ?? ((c.v.heading ?? c.v.cog) != null && c.v.twa != null ? ((c.v.heading ?? c.v.cog) + c.v.twa + 360) % 360 : null);

// Rate of turn for the compass's ring of dots (they turn with the boat, faster the faster it turns):
// a rate of turn from the boat's instruments when there is one, otherwise worked out here from the
// heading over the last 10 seconds (or from the COG when there is no heading and the boat is moving).
const WINDOW_MS = 10_000;
const turnHist = new WeakMap(); // host -> { pts: [{ t, a (unwrapped °) }], rot }
function rateOfTurn(host, ctx) {
  if (Number.isFinite(ctx?.v?.rot)) return ctx.v.rot;
  const fromCog = ctx?.v?.heading == null;
  const a = fromCog ? ((ctx?.v?.sog ?? 0) > 2 ? ctx?.v?.cog : null) : ctx.v.heading;
  if (a == null || !Number.isFinite(a)) { turnHist.delete(host); return undefined; }
  const now = ctx.now ?? Date.now();
  const h = turnHist.get(host) ?? { pts: [], rot: 0 };
  const prev = h.pts[h.pts.length - 1];
  if (prev && now - prev.t > 5000) h.pts = []; // a gap in the data: start again
  const unwrapped = prev && h.pts.length ? prev.a + ((((a - prev.a) % 360) + 540) % 360) - 180 : a;
  if (!prev || now - prev.t >= 200) h.pts.push({ t: now, a: unwrapped });
  while (h.pts.length > 2 && now - h.pts[0].t > WINDOW_MS) h.pts.shift();
  const first = h.pts[0], last = h.pts[h.pts.length - 1];
  const span = (last.t - first.t) / 60_000;
  if (span > 2 / 60) {
    const raw = (last.a - first.a) / span; // ° per minute, + to starboard
    h.rot += (raw - h.rot) * 0.3;
  }
  turnHist.set(host, h);
  return Math.abs(h.rot) < 3 ? 0 : h.rot; // below 3°/min: steady (heading noise), the dots still
}

export function setupOpenBridgeInstruments() {
  const compass = ITEMS.COMPASS;
  if (!compass) return;
  compass.render = (host, val, ctx) => {
    let el = host.querySelector(":scope > obc-compass");
    if (!el) {
      host.innerHTML = "";
      host._html = null;
      el = document.createElement("obc-compass");
      el.direction = "headingUp";
      el.showLabels = true;
      host.append(el);
    }
    const hdg = val?.hdg ?? val?.cog;
    el.state = hdg == null ? "off" : "active";
    if (hdg == null) return;
    el.heading = hdg;
    el.courseOverGround = val.cog ?? hdg;
    const twd = ctx ? twdOf(ctx) : null;
    el.windFromDirection = twd;
    el.currentWindSpeedKnots = ctx?.v?.tws ?? null;
    el.rateOfTurnDegreesPerMinute = rateOfTurn(host, ctx);
    // the bearing to the next mark of the navigated route, as the ring's setpoint marker (only when
    // navigating: a planned route isn't a course to steer)
    const btw = ctx?.navigating && Number.isFinite(ctx?.wp?.btw) ? ctx.wp.btw : null;
    el.headingSetpoint = btw;
    el.title = btw == null ? "" : `Marker on the ring: bearing to ${ctx.nav?.next?.name || "the next mark"} ${Math.round(btw)}°T`;
  };
}
