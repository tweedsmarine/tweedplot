// Night cursors at the edges of the window, and wherever the page's own rules miss one.
//
// Night cursors are the standard shapes (arrow, pointing hand, open and closed hand, crosshair,
// resize arrows) in the night colour, so nothing new to learn. style.css gives them to the
// controls it knows; anything else that asks for a system cursor by name (cursor: pointer on a
// heading, say) would get Windows' own, bright white: fixNamed() gives it the night one (and its
// children the same, as by day they inherit it).
//
// The night palettes draw their own cursors (style.css, --cur-*), because the colour filter
// doesn't reach the system cursor. But Chromium (WebView2) only shows a page's cursor image when
// the whole image fits inside the window: it stops covering anything outside the page that way.
// Near the edges, where part of the image would hang over, it shows the system cursor instead,
// bright white, whatever the image's size.
//
// So near the edges (EDGE px) the real cursor is hidden (cursor: none, which has no image to
// check) and the same shape is drawn by the page at the pointer, in colours that come out as
// the night cursor's red or green through the palette filter. Away from the edges the real
// cursor is used, as it follows the mouse with no delay at all.

const EDGE = 48; // px from any edge: room for the cursor image, with Windows' larger pointer sizes
const COLOUR = { "night-red": [0xb0, "%23b00000"], "night-green": [0xa0, "%2300a000"] };

/** A cursor value "url("…") x y, fallback" → { url, hx, hy, size }, or null for a keyword cursor. */
export function parseCursor(v) {
  const m = /url\("?([^")]+)"?\)\s*(-?\d+(?:\.\d+)?)?\s*(-?\d+(?:\.\d+)?)?/.exec(v || "");
  if (!m) return null;
  const size = Number(/width='(\d+)'/.exec(m[1])?.[1] ?? 24);
  return { url: m[1], hx: Number(m[2] ?? 0), hy: Number(m[3] ?? 0), size };
}

/**
 * The cursor's SVG recoloured to come out right through the night filter (invert, then the
 * palette's colour from brightness, then dimming `dim`): the night colour → the grey that
 * gives it, the black edge → white.
 */
export function recolour(url, palette, dim) {
  const [level, code] = COLOUR[palette] ?? COLOUR["night-red"];
  const g = Math.max(0, Math.round(255 - Math.min(255, level / Math.max(0.05, dim))));
  const grey = `%23${g.toString(16).padStart(2, "0").repeat(3)}`;
  return url.split(code).join(grey).replace(/stroke='%23000'/g, "stroke='%23fff'");
}

// a system cursor by name → the night cursor that draws it
const NAMED = {
  pointer: "hand", "zoom-in": "hand", "zoom-out": "hand",
  grab: "grab", move: "grab", "all-scroll": "grab", grabbing: "grabbing",
  crosshair: "cross", cell: "cross",
  "ns-resize": "ns", "n-resize": "ns", "s-resize": "ns", "row-resize": "ns",
  "ew-resize": "ew", "e-resize": "ew", "w-resize": "ew", "col-resize": "ew",
  "nwse-resize": "nwse", "nw-resize": "nwse", "se-resize": "nwse", "nesw-resize": "nwse", "ne-resize": "nwse", "sw-resize": "nwse",
  default: "arrow", "not-allowed": "arrow", "no-drop": "arrow", help: "arrow", "context-menu": "arrow", wait: "arrow", progress: "arrow",
};
/** The night cursor for an element from its own and its ancestors' computed cursors (nearest first):
 * a name in NAMED decides; the night arrow (the default every element has at night) looks further up;
 * any other image is already a night cursor. Exported for tests. */
export function namedFor(cursors) {
  for (const v of cursors) {
    const k = String(v || "").trim();
    if (!k.startsWith("url(")) return NAMED[k] ?? null;
    if (!k.includes("M3 2v17")) return null; // a night cursor other than the arrow
  }
  return null;
}

/** Whether a pointer at x, y is within `edge` px of the window's edges. */
export const nearEdge = (x, y, w, h, edge = EDGE) => x < edge || y < edge || x > w - edge || y > h - edge;

export function setupNightCursor() {
  const root = document.documentElement;
  const img = document.createElement("img");
  img.id = "night-cursor";
  img.alt = "";
  img.hidden = true;
  img.setAttribute("aria-hidden", "true");
  document.body.append(img);

  let on = false;
  let lastEl = null, lastInline = "", cur = null, src = "";
  const night = () => /^(iho-)?night/.test(String(root.dataset.palette || ""));

  // a system cursor by name where the night rules miss it: data-ncur on the element under the pointer
  let fixed = null, fixedFor = null;
  function unfix() { if (fixed) { fixed.removeAttribute("data-ncur"); fixed = null; } fixedFor = null; }
  function fixNamed(el) {
    if (el === fixedFor) return;
    unfix();
    fixedFor = el;
    if (!el || el.tagName === "CANVAS" || el.closest?.(".maplibregl-map")) return; // the chart has its own rules
    const list = [];
    for (let e = el, i = 0; e && e !== document.documentElement && i < 8; e = e.parentElement, i++) list.push(getComputedStyle(e).cursor);
    const want = namedFor(list);
    if (want && want !== "arrow") { el.setAttribute("data-ncur", want); fixed = el; }
  }

  // the cursor the page asks for at an element (with the edge rule lifted while reading it)
  function cursorAt(el) {
    if (on) root.classList.remove("cur-edge");
    const v = getComputedStyle(el).cursor;
    if (on) root.classList.add("cur-edge");
    return parseCursor(v);
  }
  function off() {
    if (on) { root.classList.remove("cur-edge"); on = false; }
    img.hidden = true;
  }
  function update(e) {
    if (!night() || e.pointerType === "touch" || e.pointerType === "pen") { off(); unfix(); return; }
    fixNamed(e.target instanceof Element ? e.target : null);
    const x = e.clientX, y = e.clientY;
    if (!nearEdge(x, y, window.innerWidth, window.innerHeight)) { off(); lastEl = null; return; }
    const el = e.target instanceof Element ? e.target : null;
    if (!el) { off(); return; }
    // MapLibre changes the canvas's own cursor (over a feature, while dragging) without a new target
    const inline = el.style?.cursor ?? "";
    if (el !== lastEl || inline !== lastInline) { lastEl = el; lastInline = inline; cur = cursorAt(el); }
    if (!cur) { off(); return; } // a system cursor anyway (text caret)
    const dimVar = Number(getComputedStyle(root).getPropertyValue("--pal-dim")) || 0.6;
    // IHO night has no page filter: its cursor image is drawn as it is (the red/green night palettes'
    // filter inverts and tints, so theirs is recoloured to come out right through it)
    const want = root.dataset.palette === "iho-night" ? cur.url : recolour(cur.url, root.dataset.palette, dimVar);
    if (want !== src) { src = want; img.src = want; }
    // IHO night dimmed (Brightness under 100 %) dims the whole page, this image with it, but not the
    // real cursor: lift it back by the same amount so the two look alike (it all but vanished)
    const dimmed = root.dataset.palette === "iho-night" && root.hasAttribute("data-pal-dimmed");
    img.style.filter = dimmed ? `brightness(${(1 / Math.max(0.05, dimVar)).toFixed(2)})` : "";
    img.style.width = img.style.height = `${cur.size}px`;
    img.style.transform = `translate(${x - cur.hx}px, ${y - cur.hy}px)`;
    if (!on) { root.classList.add("cur-edge"); on = true; }
    img.hidden = false;
  }
  document.addEventListener("pointermove", update, { capture: true, passive: true });
  // pressing and letting go can change the cursor (grab → grabbing) on the same element
  for (const t of ["pointerdown", "pointerup"]) document.addEventListener(t, (e) => { lastEl = null; update(e); }, { capture: true, passive: true });
  document.addEventListener("mouseout", (e) => { if (!e.relatedTarget) off(); });
  window.addEventListener("blur", off);
  new MutationObserver(() => { if (!night()) { off(); unfix(); } lastEl = null; fixedFor = null; }).observe(root, { attributes: true, attributeFilter: ["data-palette"] });
}
