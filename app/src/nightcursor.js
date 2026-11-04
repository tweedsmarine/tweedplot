// Night cursors at the edges of the window.
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
    if (!night() || e.pointerType === "touch" || e.pointerType === "pen") { off(); return; }
    const x = e.clientX, y = e.clientY;
    if (!nearEdge(x, y, window.innerWidth, window.innerHeight)) { off(); lastEl = null; return; }
    const el = e.target instanceof Element ? e.target : null;
    if (!el) { off(); return; }
    // MapLibre changes the canvas's own cursor (over a feature, while dragging) without a new target
    const inline = el.style?.cursor ?? "";
    if (el !== lastEl || inline !== lastInline) { lastEl = el; lastInline = inline; cur = cursorAt(el); }
    if (!cur) { off(); return; } // a system cursor anyway (text caret)
    const dimVar = Number(getComputedStyle(root).getPropertyValue("--pal-dim")) || 0.6;
    const want = recolour(cur.url, root.dataset.palette, root.dataset.palette === "iho-night" ? dimVar * 0.6 : dimVar);
    if (want !== src) { src = want; img.src = want; }
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
  new MutationObserver(() => { if (!night()) off(); lastEl = null; }).observe(root, { attributes: true, attributeFilter: ["data-palette"] });
}
