// A small box over the chart (a colour key) that can be dragged anywhere and stays where
// it was put (as a share of the chart's size, so it survives resizing). Double-click puts
// it back in its first place.
export function floatDrag(el, key) {
  const KEY = `racenav.float.${key}`;
  let pos = null;
  try { pos = JSON.parse(localStorage.getItem(KEY) || "null"); } catch { /* storage unavailable */ }
  const parent = () => el.offsetParent ?? el.parentElement;
  function place() {
    if (!pos) { el.style.left = el.style.top = el.style.right = el.style.bottom = ""; return; }
    const p = parent().getBoundingClientRect();
    const x = Math.min(Math.max(0, pos.x * p.width), Math.max(0, p.width - el.offsetWidth));
    const y = Math.min(Math.max(0, pos.y * p.height), Math.max(0, p.height - el.offsetHeight));
    Object.assign(el.style, { left: `${x}px`, top: `${y}px`, right: "auto", bottom: "auto" });
  }
  el.classList.add("float-drag");
  el.title = "Drag to move; double-click to put it back";
  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const p = parent().getBoundingClientRect(), r = el.getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    el.setPointerCapture(e.pointerId);
    el.classList.add("dragging");
    const move = (ev) => {
      pos = { x: (ev.clientX - dx - p.left) / p.width, y: (ev.clientY - dy - p.top) / p.height };
      place();
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.classList.remove("dragging");
      try { localStorage.setItem(KEY, JSON.stringify(pos)); } catch { /* ignore */ }
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  });
  el.addEventListener("dblclick", () => { pos = null; try { localStorage.removeItem(KEY); } catch { /* ignore */ } place(); });
  window.addEventListener("resize", place);
  new ResizeObserver(place).observe(el);
  return { place };
}
