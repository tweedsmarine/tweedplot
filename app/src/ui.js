// Icon rail and drawer: one panel open at a time, so the chart keeps the screen.

const KEY = "racenav.ui";

export function setupRail(map) {
  const rail = document.getElementById("rail");
  const drawer = document.getElementById("drawer");
  const title = document.getElementById("drawer-title");
  for (const pane of document.querySelectorAll(".pane[data-pane]")) {
    if (!drawer.contains(pane)) drawer.append(pane);
  }
  const panes = [...drawer.querySelectorAll(".pane")];
  const listeners = [];
  let current = null;

  function open(name) {
    current = name;
    for (const b of rail.querySelectorAll("button")) b.classList.toggle("on", b.dataset.pane === name);
    for (const p of panes) p.hidden = p.dataset.pane !== name;
    drawer.hidden = !name;
    document.body.classList.toggle("drawer-open", !!name);
    title.textContent = name ? panes.find((p) => p.dataset.pane === name)?.dataset.title ?? "" : "";
    try { localStorage.setItem(KEY, JSON.stringify({ pane: name })); } catch { /* ignore */ }
    for (const f of listeners) f(name);
    map.resize();
  }
  rail.addEventListener("click", (e) => {
    const b = e.target.closest("button[data-pane]");
    if (b) open(current === b.dataset.pane ? null : b.dataset.pane);
  });
  document.getElementById("drawer-close").onclick = () => open(null);
  window.addEventListener("keydown", (e) => {
    // Esc closes the drawer only when nothing else (menus, modes, selections) wants it
    if (e.key === "Escape" && !e.defaultPrevented && current && !document.querySelector(".ctx:not([hidden])") && !document.querySelector(".armed") && !document.body.classList.contains("ruler-armed")) {
      if (document.activeElement && drawer.contains(document.activeElement) && document.activeElement.tagName === "INPUT") return;
      open(null);
    }
  });

  let saved = null;
  try { saved = JSON.parse(localStorage.getItem(KEY) || "{}").pane; } catch { /* ignore */ }
  open(saved === undefined ? "weather" : saved === "route" ? "course" : saved === "boat" ? "routing" : saved);

  // Tabs inside a pane
  for (const tabs of drawer.querySelectorAll(".tabs")) {
    tabs.addEventListener("click", (e) => {
      const b = e.target.closest("button[data-tab]");
      if (!b) return;
      for (const x of tabs.querySelectorAll("button")) x.classList.toggle("on", x === b);
      for (const t of tabs.parentElement.querySelectorAll(".tab")) t.hidden = t.dataset.tab !== b.dataset.tab;
      for (const f of listeners) f(current, b.dataset.tab);
    });
  }

  return {
    open,
    current: () => current,
    onChange: (f) => listeners.push(f),
    /** Open a pane and one of its tabs. */
    openTab(pane, tab) {
      open(pane);
      drawer.querySelector(`.pane[data-pane="${pane}"] .tabs button[data-tab="${tab}"]`)?.click();
    },
  };
}
