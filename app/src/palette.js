// Bright, day, dusk and night colours. The panels, drawers, buttons and windows use the
// OpenBridge palettes (openbridge/palettes.css): data-obc-theme on the page picks one. With the IHO chart symbols, dusk and night (IHO) draw the
// chart in the IHO S-52 palettes (mappalette.js) and darken the panels. Night red and night green turn every colour on
// the screen (chart, GRIB shading, panels) into a shade of red or green on
// black, so a glance at the screen doesn't spoil night vision. Done with CSS
// filters on the whole page: dark-on-light becomes light-on-dark, then one hue.
import { setWindowDark, toggleFullscreen, installFullscreenKey } from "./winchrome.js";
import { own } from "./role.js";
import { setupNightCursor } from "./nightcursor.js";

const KEY = "racenav.palette";
const MODES = ["bright", "day", "dusk", "iho-night", "night-red", "night-green"];
const NAMES = { bright: "bright", day: "day", dusk: "dusk", "iho-night": "night (IHO)", "night-red": "night (red)", "night-green": "night (green)" };
const $ = (id) => document.getElementById(id);
/** The OpenBridge palette for a mode. Night (red) and Night (green) invert the Day palette with a filter. */
export const OBC_THEME = { bright: "bright", day: "day", dusk: "dusk", "iho-night": "night", "night-red": "day", "night-green": "day" };
const LIGHT = (m) => m === "day" || m === "bright";

/** Each window (monitor) keeps its own colours: an office well and a fly bridge differ.
 *  Everything shown in a window, whatever its view, uses that window's colours. */
export function setupPalette(onChange = () => {}) {
  // the main window carries on with the setting from before monitors existed
  let p = { mode: "day", dim: { dusk: 100, "iho-night": 100, "night-red": 60, "night-green": 60 } };
  try { Object.assign(p, JSON.parse(localStorage.getItem(own(KEY)) || "{}")); } catch { /* storage unavailable */ }
  p.dim = { dusk: 100, "iho-night": 100, "night-red": 60, "night-green": 60, ...(p.dim || {}) };
  if (!MODES.includes(p.mode)) p.mode = "day";

  function apply(persist = true) {
    const root = document.documentElement;
    root.dataset.palette = p.mode;
    root.dataset.obcTheme = OBC_THEME[p.mode];
    const dim = p.dim[p.mode] ?? 100;
    root.style.setProperty("--pal-dim", String(dim / 100));
    $("pal-mode").value = p.mode;
    $("pal-dim").value = String(dim);
    $("pal-dim").disabled = LIGHT(p.mode);
    $("pal-dim-v").textContent = LIGHT(p.mode) ? "" : `${dim} %`;
    const next = MODES[(MODES.indexOf(p.mode) + 1) % MODES.length];
    $("palette-btn").title = `Colours for this window: ${NAMES[p.mode]} (click for ${NAMES[next]}; right-click for brightness)`;
    $("palette-btn").classList.toggle("night", !LIGHT(p.mode));
    setWindowDark(!LIGHT(p.mode));
    if (persist) { try { localStorage.setItem(own(KEY), JSON.stringify(p)); } catch { /* ignore */ } }
    if (p.mode !== lastMode) { lastMode = p.mode; onChange(p.mode); }
  }
  let lastMode = null;
  $("palette-btn").onclick = () => { p.mode = MODES[(MODES.indexOf(p.mode) + 1) % MODES.length]; apply(); };
  $("pal-mode").onchange = (e) => { p.mode = e.target.value; apply(); };
  $("pal-dim").oninput = (e) => { if (!LIGHT(p.mode)) { p.dim[p.mode] = +e.target.value; apply(); } };
  $("pal-full").onclick = () => toggleFullscreen();

  // right-click the colours button: colours and brightness at once,
  // without opening a drawer (S-Mode keeps day/night and brilliance one action away)
  const pop = document.createElement("div");
  pop.id = "pal-pop";
  pop.className = "ctx pal-pop";
  pop.hidden = true;
  pop.innerHTML = `<div class="pal-modes">${MODES.map((m) => `<button data-m="${m}">${NAMES[m][0].toUpperCase() + NAMES[m].slice(1)}</button>`).join("")}</div>
    <label class="pal-b">Brightness <input type="range" min="20" max="100" step="5"><span></span></label>`;
  document.body.append(pop);
  const popRange = pop.querySelector("input"), popVal = pop.querySelector("span");
  function syncPop() {
    for (const b of pop.querySelectorAll("button")) b.classList.toggle("on", b.dataset.m === p.mode);
    popRange.value = String(p.dim[p.mode] ?? 100);
    popRange.disabled = LIGHT(p.mode);
    popVal.textContent = LIGHT(p.mode) ? "" : `${p.dim[p.mode]} %`;
  }
  function openPop() {
    const r = $("palette-btn").getBoundingClientRect();
    pop.style.left = `${r.right + 8}px`;
    pop.style.top = `${r.top}px`;
    syncPop();
    pop.hidden = false;
    // the button is low on the toolbar: keep the whole menu on the screen (it ran off the bottom)
    const h = pop.getBoundingClientRect().height;
    if (r.top + h > window.innerHeight - 8) pop.style.top = `${Math.max(8, window.innerHeight - 8 - h)}px`;
  }
  pop.addEventListener("click", (e) => { const b = e.target.closest("button[data-m]"); if (b) { p.mode = b.dataset.m; apply(); syncPop(); } });
  popRange.oninput = () => { if (!LIGHT(p.mode)) { p.dim[p.mode] = +popRange.value; apply(); syncPop(); } };
  $("palette-btn").addEventListener("contextmenu", (e) => { e.preventDefault(); openPop(); });
  document.addEventListener("pointerdown", (e) => { if (!pop.hidden && !pop.contains(e.target) && e.target.closest("#palette-btn") == null) pop.hidden = true; });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !pop.hidden) { pop.hidden = true; e.preventDefault(); } });
  installFullscreenKey();
  apply();
  setupNightCursor();
  return { mode: () => p.mode, set: (m) => { if (MODES.includes(m)) { p.mode = m; apply(); } } };
}
