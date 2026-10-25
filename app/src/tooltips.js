// Tooltips drawn by the page. The system's own title tooltips are white and cannot be
// coloured, so in night mode they spoil night vision; these follow the palette.
const DELAY = 450;

export function setupTooltips() {
  const tip = document.createElement("div");
  tip.id = "tip";
  tip.hidden = true;
  tip.setAttribute("role", "tooltip");
  document.body.append(tip);

  let owner = null; // the element whose title is borrowed while its tip is pending or shown
  let timer = null;
  let x = 0, y = 0;

  const restore = () => {
    clearTimeout(timer);
    tip.hidden = true;
    if (owner) {
      if (owner.dataset.tip && !owner.hasAttribute("title")) owner.setAttribute("title", owner.dataset.tip);
      delete owner.dataset.tip;
      owner = null;
    }
  };

  const place = () => {
    const w = tip.offsetWidth, h = tip.offsetHeight;
    tip.style.left = `${Math.max(4, Math.min(x + 12, window.innerWidth - w - 4))}px`;
    tip.style.top = `${y + 22 + h > window.innerHeight ? Math.max(4, y - h - 8) : y + 22}px`;
  };

  document.addEventListener("mouseover", (e) => {
    const el = e.target instanceof Element ? e.target.closest("[title], [data-tip]") : null;
    if (el === owner) return;
    restore();
    if (!el) return;
    const text = el.getAttribute("title") || el.dataset.tip;
    if (!text) return;
    el.dataset.tip = text;
    el.removeAttribute("title");
    owner = el;
    x = e.clientX; y = e.clientY;
    timer = setTimeout(() => {
      tip.textContent = text;
      tip.hidden = false;
      place();
    }, DELAY);
  });
  document.addEventListener("mousemove", (e) => { x = e.clientX; y = e.clientY; if (!tip.hidden) place(); });
  for (const ev of ["mousedown", "keydown", "wheel", "blur"]) window.addEventListener(ev, restore, true);
  document.addEventListener("mouseleave", restore);
}
