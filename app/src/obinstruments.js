// The Instruments view in OpenBridge's dress: the compass is OpenBridge's own instrument
// (obc-compass: heading up, heading and COG markers, the true wind from where it blows); the other
// tiles keep TweedPlot's drawings in OpenBridge's colours and type (style.css). The tiles, their
// layout and what they show are TweedPlot's (insttiles.js, instruments.js).

import "@oicl/openbridge-webcomponents/dist/navigation-instruments/compass/compass.js";
import { ITEMS } from "./instruments.js";

const twdOf = (c) => c.v.twd ?? ((c.v.heading ?? c.v.cog) != null && c.v.twa != null ? ((c.v.heading ?? c.v.cog) + c.v.twa + 360) % 360 : null);

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
  };
}
