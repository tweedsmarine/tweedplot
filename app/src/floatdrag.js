// A small box over the chart (a colour key) that can be dragged anywhere and stays where
// it was put (as a share of the chart's size, so it survives resizing). Double-click puts
// it back in its first place.
// `handle`: the part to drag it by (default the whole box), for a box with controls in it.
export function floatDrag(el, key, handle = el) {
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
  // the drag cursor only where it can be dragged (the title of a card with controls in it)
  handle.classList.add("float-drag");
  handle.title = "Drag to move; double-click to put it back";
  handle.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || (handle !== el && e.target.closest("button, select, input"))) return;
    e.preventDefault();
    const p = parent().getBoundingClientRect(), r = el.getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    handle.setPointerCapture(e.pointerId);
    el.classList.add("dragging");
    const move = (ev) => {
      pos = { x: (ev.clientX - dx - p.left) / p.width, y: (ev.clientY - dy - p.top) / p.height };
      place();
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      el.classList.remove("dragging");
      try { localStorage.setItem(KEY, JSON.stringify(pos)); } catch { /* ignore */ }
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  });
  handle.addEventListener("dblclick", (e) => {
    if (handle !== el && e.target.closest("button, select, input")) return; pos = null; try { localStorage.removeItem(KEY); } catch { /* ignore */ } place(); });
  window.addEventListener("resize", place);
  new ResizeObserver(place).observe(el);
  return { place };
}
