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
  let prefs = { height: 260 };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };

  function layout() {
    el.style.height = `${prefs.height}px`;
    document.documentElement.style.setProperty("--dock-h", current ? `${prefs.height}px` : "0px");
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
    el.hidden = !name;
    document.body.classList.toggle("dock-open", !!name);
    renderTabs();
    if (name) {
      tools.innerHTML = "";
      body.innerHTML = "";
      views.get(name).render(body, tools);
    }
    layout();
  }
  el.querySelector(".dock-close").onclick = () => show(null);

  // drag the top edge to resize
  const grip = el.querySelector(".dock-grip");
  grip.addEventListener("pointerdown", (e) => {
    grip.setPointerCapture(e.pointerId);
    const y0 = e.clientY, h0 = prefs.height;
    const move = (ev) => { prefs.height = Math.max(120, Math.min(window.innerHeight * 0.7, h0 + (y0 - ev.clientY))); layout(); };
    const up = () => { grip.removeEventListener("pointermove", move); grip.removeEventListener("pointerup", up); save(); };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
  });

  return {
    register(name, view) { views.set(name, view); if (current) renderTabs(); },
    show,
    toggle(name) { show(current === name ? null : name); },
    current: () => current,
    /** Re-render the shown view (if it is `name`, or any when omitted). */
    refresh(name) { if (current && (!name || name === current)) { tools.innerHTML = ""; body.innerHTML = ""; views.get(current).render(body, tools); } },
  };
}
