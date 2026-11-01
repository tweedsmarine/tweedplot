// Tooltips drawn by the page. The system's own title tooltips are white and cannot be
// coloured, so in night mode they spoil night vision; these follow the palette.
// Every title is moved to data-tip as soon as it appears (at start-up, when elements are added,
// and when a title is set), so the system never has a title to show, however the pointer or the
// keyboard reaches the control. Code reading a title should use `el.title || el.dataset.tip`.
const DELAY = 450;

export function setupTooltips() {
  const tip = document.createElement("div");
  tip.id = "tip";
  tip.hidden = true;
  tip.setAttribute("role", "tooltip");
  document.body.append(tip);

  let owner = null; // the element whose tip is pending or shown
  let timer = null;
  let x = 0, y = 0;

  const restore = () => {
    clearTimeout(timer);
    tip.hidden = true;
    owner = null;
  };

  const place = () => {
    const w = tip.offsetWidth, h = tip.offsetHeight;
    tip.style.left = `${Math.max(4, Math.min(x + 12, window.innerWidth - w - 4))}px`;
    tip.style.top = `${y + 22 + h > window.innerHeight ? Math.max(4, y - h - 8) : y + 22}px`;
  };

  const take = (el) => {
    const t = el.getAttribute("title");
    if (t === null) return;
    el.removeAttribute("title");
    if (t) el.dataset.tip = t; else delete el.dataset.tip;
    if (el === owner && !tip.hidden) { if (t) tip.textContent = t; else restore(); }
  };
  const takeAll = (root) => {
    if (root.nodeType !== 1) return;
    if (root.hasAttribute("title")) take(root);
    for (const el of root.querySelectorAll("[title]")) take(el);
  };
  takeAll(document.body);
  new MutationObserver((list) => {
    for (const m of list) {
      if (m.type === "attributes") take(m.target);
      else for (const n of m.addedNodes) takeAll(n);
    }
  }).observe(document.body, { attributes: true, attributeFilter: ["title"], childList: true, subtree: true });

  const enter = (e) => {
    let el = e.target instanceof Element ? e.target.closest("[data-tip]") : null;
    // a tip belonging to a container (the draggable card header) is not for the controls inside it
    let ctl = e.target instanceof Element ? e.target.closest("button, select, input, label") : null;
    if (ctl?.tagName === "INPUT" && ctl.closest("label")) ctl = ctl.closest("label");
    if (el && ctl && ctl !== el && el.contains(ctl)) el = null;
    if (el === owner) return;
    restore();
    if (!el) return;
    owner = el;
    x = e.clientX; y = e.clientY;
    timer = setTimeout(() => {
      const key = el.dataset.key;
      const t = el.dataset.tip || "";
      // the keyboard shortcut (keys.js), unless the text already gives it
      tip.textContent = key && t && !t.includes(`(${key})`) ? `${t} (${key})` : t;
      if (!tip.textContent) return;
      tip.hidden = false;
      place();
    }, DELAY);
  };
  document.addEventListener("mouseover", enter);
  document.addEventListener("focusin", (e) => {
    const t = e.target;
    if (!(t instanceof Element)) return;
    const r = t.getBoundingClientRect();
    enter({ target: t, clientX: r.left, clientY: r.bottom - 20 });
  });
  document.addEventListener("focusout", restore);
  document.addEventListener("mousemove", (e) => { x = e.clientX; y = e.clientY; if (!tip.hidden) place(); });
  const hide = () => { clearTimeout(timer); tip.hidden = true; };
  for (const ev of ["mousedown", "keydown", "wheel", "blur"]) window.addEventListener(ev, hide, true);
  document.addEventListener("mouseleave", restore);
}
