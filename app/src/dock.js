// The dock: a wide panel along the bottom of the chart for tables and graphs
// that don't fit a drawer (routing comparison now; polar plots, routing
// timelines and ensemble spreads later). Views register themselves:
//
//   dock.register("compare", { title: "Compare routings", render(body, tools) { … } })
//
// `render` is called when the view is shown and again on dock.refresh();
// `tools` is a header area for the view's own buttons (e.g. its settings cog).

const KEY = "racenav.dock";

export function setupDock(map) {
  const el = document.getElementById("dock");
  const tabs = el.querySelector(".dock-tabs");
  const tools = el.querySelector(".dock-tools");
  const body = el.querySelector(".dock-body");
  const views = new Map();
  let current = null;
  const changeListeners = [];
  let prefs = { height: 260, width: null }; // width: only once the person has dragged it (a view suggests one otherwise)
  let place = { side: "bottom" };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };

  const sideW = () => Math.min(window.innerWidth * 0.7, Math.max(280, prefs.width ?? place.width ?? 460));
  function layout() {
    const right = !!current && place.side === "right";
    document.documentElement.classList.toggle("dock-right", right);
    if (right) { el.style.height = ""; el.style.width = `${sideW()}px`; } else { el.style.width = ""; el.style.height = `${prefs.height}px`; }
    // beside the chart the dock takes width from the chart, not height
    document.documentElement.style.setProperty("--dock-h", current && !right ? `${prefs.height}px` : "0px");
    document.documentElement.style.setProperty("--dock-w", right ? `${sideW()}px` : "0px");
    window.dispatchEvent(new Event("racenav-layout")); // floating panels keep clear of the dock
  }
  function renderTabs() {
    tabs.innerHTML = "";
    for (const [name, v] of views) {
      const b = document.createElement("button");
      b.textContent = v.title;
      b.classList.toggle("on", name === current);
      b.onclick = () => show(name);
      tabs.append(b);
    }
  }
  function show(name) {
    current = name;
    place = (name && views.get(name).placement?.()) || { side: "bottom" };
    layout(); // the dock's place first, so the view draws to the size it will have
    el.hidden = !name;
    document.body.classList.toggle("dock-open", !!name);
    renderTabs();
    if (name) {
      tools.innerHTML = "";
      body.innerHTML = "";
      views.get(name).render(body, tools);
    }
    layout();
    for (const f of changeListeners) f(current);
  }
  el.querySelector(".dock-close").onclick = () => show(null);

  // drag the top edge to resize
  const grip = el.querySelector(".dock-grip");
  grip.addEventListener("pointerdown", (e) => {
    grip.setPointerCapture(e.pointerId);
    const side = place.side === "right";
    const x0 = e.clientX, y0 = e.clientY, h0 = prefs.height, w0 = sideW();
    const move = (ev) => {
      if (side) prefs.width = Math.max(280, Math.min(window.innerWidth * 0.7, w0 + (x0 - ev.clientX)));
      else prefs.height = Math.max(120, Math.min(window.innerHeight * 0.7, h0 + (y0 - ev.clientY)));
      layout();
    };
    const up = () => { grip.removeEventListener("pointermove", move); grip.removeEventListener("pointerup", up); save(); if (side) refresh(); };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
  });
  function refresh(name) { if (current && (!name || name === current)) { tools.innerHTML = ""; body.innerHTML = ""; views.get(current).render(body, tools); } }

  return {
    register(name, view) { views.set(name, view); if (current) renderTabs(); },
    show,
    toggle(name) { show(current === name ? null : name); },
    current: () => current,
    /** Called with the shown view's name (or null) whenever it changes. */
    onChange: (f) => changeListeners.push(f),
    /** Re-render the shown view (if it is `name`, or any when omitted). */
    refresh,
  };
}
