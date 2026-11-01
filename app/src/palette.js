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
// Chromium drops a page's own cursor for the system one (white) when the cursor image is over
// 32 pixels and reaches past the edge of the window, which happens near the edges with display
// scaling above 100 %: 24 px drawn at 150 % is 36. The night cursors are redrawn small enough to
// stay under that at this window's scaling (24 px at up to 133 %, 21 at 150 %, 16 at 200 %).
const CURSORS = ["--cur-arrow", "--cur-ring", "--cur-cross", "--cur-grab", "--cur-grabbing", "--cur-nwse", "--cur-ew", "--cur-ns"];
/** The size (CSS px) that keeps a cursor at or under 32 device pixels. */
export const cursorPx = (dpr) => Math.max(12, Math.min(24, Math.floor(32 / (dpr || 1))));
/** A 24 px cursor value (url(...) x y, fallback) redrawn at `px`, hot spot scaled with it. */
export function scaleCursor(v, px) {
  if (px >= 24) return v;
  return v
    .replace(/width='24' height='24'/, `width='${px}' height='${px}' viewBox='0 0 24 24'`)
    .replace(/\)\s+(\d+(?:\.\d+)?)\s+(\d+(?:\.\d+)?)\s*,/, (_, x, y) => `) ${Math.round((x * px) / 24)} ${Math.round((y * px) / 24)},`);
}
function fitCursors() {
  const root = document.documentElement;
  for (const n of CURSORS) root.style.removeProperty(n);
  const px = cursorPx(window.devicePixelRatio);
  if (px >= 24 || !String(root.dataset.palette).startsWith("night")) return;
  const cs = getComputedStyle(root);
  for (const n of CURSORS) {
    const v = cs.getPropertyValue(n).trim();
    if (v) root.style.setProperty(n, scaleCursor(v, px));
  }
}
/** Refit when the window moves to a screen with other scaling. */
function watchScaling() {
  const mq = matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
  mq.addEventListener("change", () => { fitCursors(); watchScaling(); }, { once: true });
}

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
    $("palette-btn").title = `Colours for this window: ${NAMES[p.mode]} (click for ${NAMES[next]}; right-click or hold for brightness)`;
    $("palette-btn").classList.toggle("night", p.mode !== "day");
    setWindowDark(p.mode !== "day");
    fitCursors();
    if (persist) { try { localStorage.setItem(own(KEY), JSON.stringify(p)); } catch { /* ignore */ } }
  }
  $("palette-btn").onclick = () => { p.mode = MODES[(MODES.indexOf(p.mode) + 1) % MODES.length]; apply(); };
  $("pal-mode").onchange = (e) => { p.mode = e.target.value; apply(); };
  $("pal-dim").oninput = (e) => { if (p.mode !== "day") { p.dim[p.mode] = +e.target.value; apply(); } };
  $("pal-full").onclick = () => toggleFullscreen();

  // right-click or long-press the colours button: colours and brightness at once,
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
    popRange.disabled = p.mode === "day";
    popVal.textContent = p.mode === "day" ? "" : `${p.dim[p.mode]} %`;
  }
  function openPop() {
    const r = $("palette-btn").getBoundingClientRect();
    pop.style.left = `${r.right + 8}px`;
    pop.style.top = `${r.top}px`;
    syncPop();
    pop.hidden = false;
  }
  pop.addEventListener("click", (e) => { const b = e.target.closest("button[data-m]"); if (b) { p.mode = b.dataset.m; apply(); syncPop(); } });
  popRange.oninput = () => { if (p.mode !== "day") { p.dim[p.mode] = +popRange.value; apply(); syncPop(); } };
  $("palette-btn").addEventListener("contextmenu", (e) => { e.preventDefault(); openPop(); });
  let press = null;
  $("palette-btn").addEventListener("pointerdown", (e) => { if (e.pointerType !== "mouse") press = setTimeout(() => { press = "done"; openPop(); }, 550); });
  $("palette-btn").addEventListener("pointerup", () => { if (press && press !== "done") clearTimeout(press); });
  $("palette-btn").addEventListener("click", (e) => { if (press === "done") { e.stopImmediatePropagation(); press = null; } }, true);
  document.addEventListener("pointerdown", (e) => { if (!pop.hidden && !pop.contains(e.target) && e.target.closest("#palette-btn") == null) pop.hidden = true; });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !pop.hidden) { pop.hidden = true; e.preventDefault(); } });
  installFullscreenKey();
  apply();
  watchScaling();
  return { mode: () => p.mode, set: (m) => { if (MODES.includes(m)) { p.mode = m; apply(); } } };
}
