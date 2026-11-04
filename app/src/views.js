// Views: Navigation, Planning and Instruments are different arrangements of the
// same screen. Every option stays available in every view; a view only decides
// what is shown to begin with (the rail, drawers and dock are "tools"), and the
// tools can be shown or hidden in any view. A view also has its own day/night
// colours (palette.js). More views (radar, sounder) are added to VIEWS.
//
// `racenav-view` fires on window with detail { id, chrome } when the view or its
// tools change; modules that need to re-lay themselves out listen for it.
// Each window has its own view (and tools/chart choices), so any window can show any view.
import { own, PRIMARY, LABEL, startView } from "./role.js";
import { patchWindow } from "./windowstate.js";

const KEY = own("racenav.views");

export const VIEWS = [
  {
    id: "nav", name: "Navigation", tools: true, map: true,
    icon: '<circle cx="12" cy="12" r="8.5"/><path d="M15.5 8.5l-2 5-5 2 2-5z" class="fill"/>',
  },
  {
    id: "plan", name: "Planning", tools: true, map: true,
    icon: '<circle cx="5" cy="18" r="2"/><circle cx="19" cy="6" r="2"/><path d="M7 17c5-1 3-8 10-10"/>',
  },
  {
    id: "inst", name: "Instruments", tools: false, map: false,
    icon: '<circle cx="12" cy="13" r="8"/><path d="M12 13l4-4"/><path d="M12 5v2M5 13h2M17 13h2"/>',
  },
  {
    id: "nmea", name: "Live data", tools: false, map: false,
    icon: '<path d="M3 12h3l2.2-6 4.2 12 2.2-6H21"/><path d="M4 5h2M18 19h2"/>',
  },
];

const $ = (id) => document.getElementById(id);

export function setupViews(map) {
  // The program always opens in Navigation; each view remembers for itself whether its panels
  // (the rail and drawers) are shown, and Navigation starts with them shown.
  let s = { current: "nav", tools: {}, map: {} };
  try { Object.assign(s, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  s.current = "nav";
  if (!PRIMARY && VIEWS.some((v) => v.id === startView)) s.current = startView;
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ } };
  const def = (id) => VIEWS.find((v) => v.id === id);
  const toolsOn = (id) => s.tools[id] ?? def(id).tools;
  const mapOn = (id) => s.map?.[id] ?? def(id).map;

  function apply() {
    const v = def(s.current);
    const tools = toolsOn(v.id);
    const chart = mapOn(v.id);
    document.documentElement.classList.toggle("no-tools", !tools);
    document.documentElement.classList.toggle("no-map", !chart);
    document.documentElement.dataset.view = v.id;
    const vb = $("view-btn");
    const next = VIEWS[(VIEWS.indexOf(v) + 1) % VIEWS.length];
    vb.querySelector("svg").innerHTML = v.icon;
    vb.title = `View: ${v.name} (click for ${next.name}; right-click or hold to choose, or to show or hide the chart)`;
    vb.setAttribute("aria-label", `View: ${v.name}`);
    const tb = $("tools-btn");
    tb.classList.toggle("on", tools);
    tb.title = tools ? `Hide the panels and dock in the ${v.name} view` : `Show the panels and dock in the ${v.name} view`;
    save();
    if (!PRIMARY) patchWindow(LABEL, { view: v.id });
    window.dispatchEvent(new CustomEvent("racenav-view", { detail: { id: v.id, tools } }));
    window.dispatchEvent(new Event("racenav-layout"));
    requestAnimationFrame(() => map.resize());
  }

  $("view-btn").onclick = () => api.set(VIEWS[(VIEWS.findIndex((v) => v.id === s.current) + 1) % VIEWS.length].id);
  $("tools-btn").onclick = () => { s.tools[s.current] = !toolsOn(s.current); apply(); };

  // right-click (or hold) the view button: pick a view, and show or hide the chart in this one
  const pop = document.createElement("div");
  pop.id = "view-pop";
  pop.className = "ctx view-pop";
  pop.hidden = true;
  document.body.append(pop);
  function openPop() {
    const v = def(s.current);
    pop.innerHTML = `<div class="grp">View</div>${VIEWS.map((x) => `<button data-v="${x.id}" class="${x.id === v.id ? "cur" : ""}">${x.name}</button>`).join("")}
      <hr /><label class="vp-chart"><input type="checkbox" ${mapOn(v.id) ? "checked" : ""}> Show the chart in the ${v.name} view</label>`;
    pop.querySelector("input").onchange = (e) => { s.map[s.current] = e.target.checked; apply(); pop.hidden = true; };
    const r = $("view-btn").getBoundingClientRect();
    pop.style.left = `${r.right + 8}px`;
    pop.style.top = `${Math.min(r.top, window.innerHeight - 200)}px`;
    pop.hidden = false;
  }
  pop.addEventListener("click", (e) => { const b = e.target.closest("button[data-v]"); if (b) { api.set(b.dataset.v); pop.hidden = true; } });
  const vb = $("view-btn");
  vb.addEventListener("contextmenu", (e) => { e.preventDefault(); openPop(); });
  let press = null;
  vb.addEventListener("pointerdown", (e) => { if (e.pointerType !== "mouse") press = setTimeout(() => { press = "done"; openPop(); }, 550); });
  vb.addEventListener("pointerup", () => { if (press && press !== "done") clearTimeout(press); });
  vb.addEventListener("click", (e) => { if (press === "done") { e.stopImmediatePropagation(); press = null; } }, true);
  document.addEventListener("pointerdown", (e) => { if (!pop.hidden && !pop.contains(e.target) && !e.target.closest("#view-btn")) pop.hidden = true; });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !pop.hidden) { pop.hidden = true; e.preventDefault(); } });

  window.addEventListener("racenav-need-tools", () => { if (!toolsOn(s.current)) { s.tools[s.current] = true; apply(); } });
  const api = {
    current: () => s.current,
    set(id) { if (def(id) && id !== s.current) { s.current = id; apply(); } },
    onChange(f) { window.addEventListener("racenav-view", (e) => f(e.detail.id, e.detail.tools)); },
  };
  apply();
  return api;
}
