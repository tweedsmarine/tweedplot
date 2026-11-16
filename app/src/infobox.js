// Floating info boxes: what a click on the chart tells you (AIS targets, fishing rules,
// chart features, tide stations, weather stations, tracks) in a window that can be moved (by its title bar), resized (its corner)
// and scrolled, instead of a popup tied to the point that covers the chart round it and
// can run off the screen. Each kind of box keeps its own place and size. A small ring
// marks the place the box is about while it's open.

import * as maplibregl from "maplibre-gl";
import { floatResize } from "./floatresize.js";

const KEY = "racenav.infobox.";
const boxes = new Set();

/**
 * A box for one kind of information (`kind`: "ais", "rules", ...).
 * `open(content, { title, at })`: content is an element or HTML; `at` [lon, lat] is marked.
 */
export function createInfoBox(map, kind, { title = "", width = 340, height = 320, showRing = true } = {}) {
  let st = { w: width, h: height };
  try { Object.assign(st, JSON.parse(localStorage.getItem(KEY + kind) || "{}")); } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY + kind, JSON.stringify(st)); } catch { /* ignore */ } };

  const el = document.createElement("div");
  el.className = "float-win info-box";
  el.hidden = true;
  el.innerHTML = `<header><b class="ib-title"></b><span class="grow"></span><button class="ib-close" title="Close (Esc)">✕</button></header><div class="ib-body"></div>`;
  document.body.append(el); // over the chart, placed within the chart's box, like the target list
  floatResize(el);
  const body = el.querySelector(".ib-body"), head = el.querySelector("header"), titleEl = el.querySelector(".ib-title");
  let onClose = null, ring = null;

  const area = () => map.getContainer().getBoundingClientRect();
  function place() {
    const r = area();
    const w = Math.min(st.w, r.width - 16), h = Math.min(st.h, r.height - 16);
    el.style.width = `${w}px`;
    // as tall as its content, up to the height it was last dragged to (it was always that tall, mostly empty)
    el.style.height = "auto";
    el.style.maxHeight = `${h}px`;
    el.style.right = "auto";
    // st.x, st.y are within the chart; the box is on the page, and the chart starts after the toolbar
    el.style.left = `${r.left + Math.max(4, Math.min(r.width - w - 4, st.x ?? r.width - w - 70))}px`;
    el.style.top = `${r.top + Math.max(4, Math.min(r.height - h - 4, st.y ?? 70))}px`;
  }

  /** A box opening over another open box moves aside (to its right, else left, else below) so neither hides the other.
   *  Only this showing is moved: the position it was dragged to is kept. */
  function clear() {
    const r = area();
    const others = [...boxes].filter((x) => x !== api && x.el && !x.el.hidden).map((x) => x.el.getBoundingClientRect());
    for (let pass = 0; pass < others.length + 1; pass++) {
      const me = el.getBoundingClientRect();
      const hit = others.find((o) => me.left < o.right && me.right > o.left && me.top < o.bottom && me.bottom > o.top);
      if (!hit) return;
      const gap = 8;
      let left = hit.right + gap, top = me.top;
      if (left + me.width > r.right - 4) left = hit.left - gap - me.width;
      if (left < r.left + 4) { left = Math.max(r.left + 4, Math.min(me.left, r.right - me.width - 4)); top = hit.bottom + gap; }
      if (top + me.height > r.bottom - 4) top = Math.max(r.top + 4, r.bottom - 4 - me.height);
      el.style.left = `${left}px`; el.style.top = `${top}px`;
    }
  }

  // first time: beside the point (not on it), on the side with room
  function besidePoint(at) {
    if (st.x != null || !at) return;
    const p = map.project(at), r = area();
    const w = Math.min(st.w, r.width - 16);
    st.x = p.x + 30 + w < r.width ? p.x + 30 : p.x - 30 - w;
    st.y = p.y - 60;
  }

  head.addEventListener("pointerdown", (e) => {
    if (e.target.closest("button, select, input")) return;
    const b = el.getBoundingClientRect(), m = area();
    const dx = e.clientX - b.left, dy = e.clientY - b.top;
    head.setPointerCapture?.(e.pointerId);
    const move = (ev) => { st.x = ev.clientX - m.left - dx; st.y = ev.clientY - m.top - dy; place(); };
    const up = () => { head.removeEventListener("pointermove", move); head.removeEventListener("pointerup", up); save(); };
    head.addEventListener("pointermove", move);
    head.addEventListener("pointerup", up);
    e.preventDefault();
  });
  // the corner handle (CSS resize) changes the size: keep it
  let sizing = null;
  new ResizeObserver(() => {
    if (el.hidden) return;
    const w = el.offsetWidth, h = el.offsetHeight;
    if (Math.abs(w - st.w) < 2 && Math.abs(h - st.h) < 2) return;
    st.w = w; st.h = h;
    clearTimeout(sizing);
    sizing = setTimeout(save, 300);
  }).observe(el);
  window.addEventListener("resize", () => !el.hidden && place());
  window.addEventListener("racenav-layout", () => !el.hidden && place());

  function mark(at) {
    if (!at || !showRing) { ring?.remove(); ring = null; return; }
    if (!ring) {
      const d = document.createElement("div");
      d.className = "ib-ring";
      ring = new maplibregl.Marker({ element: d });
    }
    ring.setLngLat(at).addTo(map);
  }

  function setContent(content) {
    const top = body.scrollTop;
    if (typeof content === "string") body.innerHTML = content;
    else body.replaceChildren(content);
    // a heading that only repeats the window's title is not shown twice
    const first = body.firstElementChild?.matches("h1, h2, h3, h4, h5, b, strong, .ib-h") ? body.firstElementChild : body.firstElementChild?.querySelector?.(":scope > :is(h1, h2, h3, h4, h5, b, strong):first-child");
    if (first && first.textContent.trim() === titleEl.textContent.trim()) first.hidden = true;
    body.scrollTop = top;
  }

  const api = {
    element: el,
    body,
    el,
    get isOpen() { return !el.hidden; },
    open(content, { title: t = title, at = null, onClose: oc = null } = {}) {
      const was = !el.hidden;
      if (was && onClose && onClose !== oc) { const f = onClose; onClose = null; f(); }
      titleEl.textContent = t;
      setContent(content);
      if (!was) body.scrollTop = 0;
      besidePoint(at);
      el.hidden = false;
      place();
      if (!was) clear();
      mark(at);
      onClose = oc;
      boxes.add(api);
    },
    /** New content (and title, place) without moving the box or its scroll. */
    update(content, { title: t, at } = {}) {
      if (el.hidden) return;
      if (t != null) titleEl.textContent = t;
      setContent(content);
      if (at) mark(at);
    },
    close() {
      if (el.hidden) return;
      el.hidden = true;
      mark(null);
      boxes.delete(api);
      const f = onClose; onClose = null; f?.();
    },
  };
  el.querySelector(".ib-close").onclick = () => api.close();
  return api;
}

// Esc closes the box opened last
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || !boxes.size || /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName ?? "")) return;
  [...boxes].pop().close();
});

const made = new Map();
/** The one box of a kind (made on first use), for modules that share it. */
export function infoBox(map, kind, opts) {
  if (!made.has(kind)) made.set(kind, createInfoBox(map, kind, opts));
  return made.get(kind);
}
