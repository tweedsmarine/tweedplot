// Screen-control preferences for this window: zoom buttons and large touch-size
// controls. "auto" follows whether a touch screen is present.
import { own } from "./role.js";

const KEY = own("racenav.ui");
const touch = window.matchMedia("(any-pointer: coarse)");

function load() {
  try { return { zoom: "auto", large: "auto", ...JSON.parse(localStorage.getItem(KEY) || "{}") }; }
  catch { return { zoom: "auto", large: "auto" }; }
}

const on = (mode) => (mode === "auto" ? touch.matches : mode === "on");

function apply(p) {
  const root = document.documentElement;
  root.classList.toggle("ui-zoom", on(p.zoom));
  root.classList.toggle("ui-large", on(p.large));
}

export function setupUiPrefs() {
  const p = load();
  apply(p);
  touch.addEventListener("change", () => apply(p));
  for (const k of ["zoom", "large"]) {
    const el = document.getElementById(`ui-${k}`);
    if (!el) continue;
    el.value = p[k];
    el.onchange = () => {
      p[k] = el.value;
      localStorage.setItem(KEY, JSON.stringify(p));
      apply(p);
    };
  }
}
