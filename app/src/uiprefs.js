// Screen-control preferences for this window: the zoom buttons on the chart (for a trackball or
// a mouse without a wheel). TweedPlot is laid out for a mouse and keyboard, at one size.
import { own } from "./role.js";

const KEY = own("racenav.ui");

function load() {
  try { return { zoom: "off", ...JSON.parse(localStorage.getItem(KEY) || "{}") }; }
  catch { return { zoom: "off" }; }
}

function apply(p) {
  document.documentElement.classList.toggle("ui-zoom", p.zoom === "on");
}

export function setupUiPrefs() {
  const p = load();
  if (p.zoom === "auto") p.zoom = "off"; // "auto" followed a touch screen, no longer supported
  apply(p);
  const el = document.getElementById("ui-zoom");
  if (!el) return;
  el.value = p.zoom;
  el.onchange = () => {
    p.zoom = el.value;
    localStorage.setItem(KEY, JSON.stringify(p));
    apply(p);
  };
}
