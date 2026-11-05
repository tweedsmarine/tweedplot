// A corner handle to resize a floating window. The browser's own resize corner (CSS `resize`)
// shows the system cursor, which the night palettes can't recolour; this one is the page's, so
// the night cursors (style.css) apply. `widthOnly`: the window's height follows its content.
export function floatResize(el, { widthOnly = false } = {}) {
  const h = document.createElement("div");
  h.className = widthOnly ? "fw-rsz fw-rsz-w" : "fw-rsz";
  h.setAttribute("aria-hidden", "true");
  el.append(h);
  h.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const r = el.getBoundingClientRect();
    const x0 = e.clientX, y0 = e.clientY;
    const cs = getComputedStyle(el);
    const minW = parseFloat(cs.minWidth) || 0, minH = parseFloat(cs.minHeight) || 0;
    const maxW = parseFloat(cs.maxWidth) || Infinity, maxH = parseFloat(cs.maxHeight) || Infinity;
    h.setPointerCapture(e.pointerId);
    const move = (ev) => {
      el.style.width = `${Math.min(maxW, Math.max(minW, r.width + ev.clientX - x0))}px`;
      if (!widthOnly) el.style.height = `${Math.min(maxH, Math.max(minH, r.height + ev.clientY - y0))}px`;
    };
    const up = () => {
      h.removeEventListener("pointermove", move);
      h.removeEventListener("pointerup", up);
      h.removeEventListener("pointercancel", up);
    };
    h.addEventListener("pointermove", move);
    h.addEventListener("pointerup", up);
    h.addEventListener("pointercancel", up);
  });
  return h;
}
