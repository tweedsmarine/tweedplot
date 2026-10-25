// Day, dusk and night colours. Night red and night green turn every colour on
// the screen (chart, GRIB shading, panels) into a shade of red or green on
// black, so a glance at the screen doesn't spoil night vision. Done with CSS
// filters on the whole page: dark-on-light becomes light-on-dark, then one hue.
import { setWindowDark, toggleFullscreen, installFullscreenKey } from "./winchrome.js";
import { own } from "./role.js";

const KEY = "racenav.palette";
const MODES = ["day", "dusk", "night-red", "night-green"];
const NAMES = { day: "day", dusk: "dusk", "night-red": "night (red)", "night-green": "night (green)" };
const $ = (id) => document.getElementById(id);

/** Each window (monitor) keeps its own colours: an office well and a fly bridge differ.
 *  Everything shown in a window, whatever its view, uses that window's colours. */
export function setupPalette() {
  // the main window carries on with the setting from before monitors existed
  let p = { mode: "day", dim: { dusk: 75, "night-red": 60, "night-green": 60 } };
  try { Object.assign(p, JSON.parse(localStorage.getItem(own(KEY)) || "{}")); } catch { /* storage unavailable */ }
  p.dim = { dusk: 75, "night-red": 60, "night-green": 60, ...(p.dim || {}) };
  if (!MODES.includes(p.mode)) p.mode = "day";

  function apply(persist = true) {
    const root = document.documentElement;
    root.dataset.palette = p.mode;
    const dim = p.dim[p.mode] ?? 100;
    root.style.setProperty("--pal-dim", String(dim / 100));
    $("pal-mode").value = p.mode;
    $("pal-dim").value = String(dim);
    $("pal-dim").disabled = p.mode === "day";
    $("pal-dim-v").textContent = p.mode === "day" ? "" : `${dim} %`;
    const next = MODES[(MODES.indexOf(p.mode) + 1) % MODES.length];
    $("palette-btn").title = `Colours for this window: ${NAMES[p.mode]} (click for ${NAMES[next]})`;
    $("palette-btn").classList.toggle("night", p.mode !== "day");
    setWindowDark(p.mode !== "day");
    if (persist) { try { localStorage.setItem(own(KEY), JSON.stringify(p)); } catch { /* ignore */ } }
  }
  $("palette-btn").onclick = () => { p.mode = MODES[(MODES.indexOf(p.mode) + 1) % MODES.length]; apply(); };
  $("pal-mode").onchange = (e) => { p.mode = e.target.value; apply(); };
  $("pal-dim").oninput = (e) => { if (p.mode !== "day") { p.dim[p.mode] = +e.target.value; apply(); } };
  $("pal-full").onclick = () => toggleFullscreen();
  installFullscreenKey();
  apply();
  return { mode: () => p.mode, set: (m) => { if (MODES.includes(m)) { p.mode = m; apply(); } } };
}
