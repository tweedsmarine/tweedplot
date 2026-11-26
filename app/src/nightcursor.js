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
 * palette's colour from brightness, then the page's dimming, which dims it as it dims the page):
 * every colour but the black edge → the grey that gives the night colour, the black edge → white.
 */
export function recolour(url, palette) {
  const [level] = COLOUR[palette] ?? COLOUR["night-red"];
  const grey = `%23${(255 - level).toString(16).padStart(2, "0").repeat(3)}`;
  return url
    .replace(/(fill|stroke)='%23(?!000')([0-9a-fA-F]{6}|[0-9a-fA-F]{3})'/g, (_, a) => `${a}='${grey}'`)
    .replace(/stroke='%23000'/g, "stroke='%23fff'");
}

/** Every colour in a cursor's SVG scaled by `dim` (0..1): the night cursors follow Brightness. Exported for tests. */
export function dimmed(url, dim) {
  const k = Math.max(0.05, Math.min(1, dim));
  return url.replace(/%23([0-9a-fA-F]{6}|[0-9a-fA-F]{3})(?=')/g, (_, h) => {
    const full = h.length === 3 ? h.split("").map((c) => c + c).join("") : h;
    return `%23${[0, 2, 4].map((i) => Math.round(parseInt(full.slice(i, i + 2), 16) * k).toString(16).padStart(2, "0")).join("")}`;
  });
}

// a system cursor by name → the night cursor that draws it
const NAMED = {
  pointer: "hand", "zoom-in": "hand", "zoom-out": "hand",
  grab: "grab", move: "grab", "all-scroll": "grab", grabbing: "grabbing",
  crosshair: "cross", cell: "cross",
  "ns-resize": "ns", "n-resize": "ns", "s-resize": "ns", "row-resize": "ns",
  "ew-resize": "ew", "e-resize": "ew", "w-resize": "ew", "col-resize": "ew",
  "nwse-resize": "nwse", "nw-resize": "nwse", "se-resize": "nwse", "nesw-resize": "nwse", "ne-resize": "nwse", "sw-resize": "nwse",
  default: "arrow", auto: "arrow", "not-allowed": "arrow", "no-drop": "arrow", help: "arrow", "context-menu": "arrow", wait: "arrow", progress: "arrow",
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

const NIGHT_NAMES = ["arrow", "hand", "grab", "grabbing", "cross", "ns", "ew", "nwse"];

/**
 * Night versions of a style sheet's cursor rules: each rule that names a system cursor gets a copy,
 * same selector and so the same weight, that uses the night cursor (`--ncur-*`, set only in the
 * night palettes) and falls back to the named one by day. Added after the originals, so the
 * copies win. Rules whose selector can't be repeated (pseudo-elements) are left. Exported for tests.
 */
export function nightCopies(rules) {
  const out = [];
  const walk = (list) => {
    for (const r of list) {
      if (r.cssRules && !r.selectorText) { walk(r.cssRules); continue; }
      const c = r.style?.getPropertyValue?.("cursor")?.trim();
      if (!r.selectorText || !c || !NAMED[c] || /::|data-palette|data-ncur/.test(r.selectorText)) continue;
      const imp = r.style.getPropertyPriority("cursor") === "important" ? " !important" : "";
      out.push(`${r.selectorText} { cursor: var(--ncur-${NAMED[c]}, ${c})${imp}; }`);
    }
  };
  walk(rules);
  return out;
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

  // ---- night copies of every cursor rule: the page's sheets and each OpenBridge shadow root's ----
  const sheetFor = (texts) => {
    const sh = new CSSStyleSheet();
    for (const t of texts) { try { sh.insertRule(t, sh.cssRules.length); } catch { /* a selector this browser won't take */ } }
    return sh;
  };
  const rulesOf = (sheets) => { const all = []; for (const sh of sheets) { try { all.push(...sh.cssRules); } catch { /* another origin's sheet */ } } return all; };
  let pageSheet = null;
  function coverPage() {
    const mine = pageSheet;
    const sheets = [...document.styleSheets].filter((x) => x !== mine);
    const sh = sheetFor(nightCopies(rulesOf([...sheets, ...document.adoptedStyleSheets.filter((x) => x !== mine)])));
    document.adoptedStyleSheets = [...document.adoptedStyleSheets.filter((x) => x !== mine), sh];
    pageSheet = sh;
  }
  const covered = new WeakSet();
  function coverShadow(root) {
    if (!root || covered.has(root)) return;
    covered.add(root);
    const own = [...root.adoptedStyleSheets, ...[...root.querySelectorAll("style")].map((x) => x.sheet).filter(Boolean)];
    const copies = nightCopies(rulesOf(own));
    // near the window's edges the page draws the cursor (see above): none inside the component either
    copies.unshift(":host-context(html.cur-edge), :host-context(html.cur-edge) * { cursor: none !important; }");
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, sheetFor(copies)];
  }
  function coverShadows(from = document) {
    for (const el of from.querySelectorAll("*")) if (el.shadowRoot) { coverShadow(el.shadowRoot); coverShadows(el.shadowRoot); }
  }
  let coverTimer = null;
  const coverSoon = () => { clearTimeout(coverTimer); coverTimer = setTimeout(() => { coverPage(); coverShadows(); }, 300); };
  coverPage(); coverShadows();
  // the components render after start-up; later ones are met under the pointer (below)
  setTimeout(coverSoon, 1500);
  setTimeout(coverSoon, 6000);
  new MutationObserver(coverSoon).observe(document.head, { childList: true, subtree: true });
  // a shadow root met under the pointer that hasn't been covered yet
  document.addEventListener("pointerover", (e) => { for (const n of e.composedPath()) if (n instanceof ShadowRoot) coverShadow(n); }, { capture: true, passive: true });

  // ---- the night cursors follow Brightness (the page is dimmed by its filter; the cursor isn't) ----
  let dimKey = "";
  function dimCursors() {
    const key = `${root.dataset.palette}|${root.style.getPropertyValue("--pal-dim")}`;
    if (key === dimKey) return;
    dimKey = key;
    for (const n of NIGHT_NAMES) { root.style.removeProperty(`--cur-${n}`); root.style.removeProperty(`--ncur-${n}`); }
    if (!night()) return;
    const dim = Number(getComputedStyle(root).getPropertyValue("--pal-dim")) || (root.dataset.palette === "iho-night" ? 1 : 0.6);
    // IHO night is dimmed only below 100 %; the red and green ones always by their dimming
    const k = root.dataset.palette === "iho-night" && !root.hasAttribute("data-pal-dimmed") ? 1 : dim;
    const cs = getComputedStyle(root);
    for (const n of NIGHT_NAMES) {
      const v = cs.getPropertyValue(`--cur-${n}`).trim();
      if (!v) continue;
      root.style.setProperty(`--cur-${n}`, k < 1 ? dimmed(v, k) : v);
      root.style.setProperty(`--ncur-${n}`, `var(--cur-${n})`);
    }
  }
  dimCursors();
  let wasNight = night();
  new MutationObserver(() => {
    dimCursors();
    if (night() && !wasNight) coverShadows(); // components drawn by day
    wasNight = night();
  }).observe(root, { attributes: true, attributeFilter: ["data-palette", "data-pal-dimmed", "style"] });

  // a system cursor by name set straight on an element (style="cursor: pointer"), which no sheet covers:
  // data-ncur on the element under the pointer
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
    if (want) { el.setAttribute("data-ncur", want); fixed = el; }
  }

  // the cursor the page asks for at an element (with the edge rule lifted while reading it)
  function cursorAt(el) {
    if (on) root.classList.remove("cur-edge");
    let v = getComputedStyle(el).cursor;
    if (on) root.classList.add("cur-edge");
    // still a name (an element no rule covers): the night cursor that draws it
    if (!/^url\(/.test(v.trim()) && NAMED[v.trim()]) v = getComputedStyle(root).getPropertyValue(`--cur-${NAMED[v.trim()]}`);
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
    // the element really under the pointer, inside an OpenBridge component too
    const inner = e.composedPath?.()[0];
    const el = inner instanceof Element ? inner : e.target instanceof Element ? e.target : null;
    if (!el) { off(); return; }
    // MapLibre changes the canvas's own cursor (over a feature, while dragging) without a new target
    const inline = el.style?.cursor ?? "";
    if (el !== lastEl || inline !== lastInline) { lastEl = el; lastInline = inline; cur = cursorAt(el); }
    if (!cur) { off(); return; } // a system cursor anyway (text caret)
    const dimVar = Number(getComputedStyle(root).getPropertyValue("--pal-dim")) || 0.6;
    // IHO night has no page filter: its cursor image is drawn as it is (the red/green night palettes'
    // filter inverts and tints, so theirs is recoloured to come out right through it)
    const want = root.dataset.palette === "iho-night" ? cur.url : recolour(cur.url, root.dataset.palette);
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
