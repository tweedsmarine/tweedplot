// Views: Navigation, Planning, Environment and Instruments are different arrangements of the
// same screen. Every option stays available in every view; a view only decides
// what is shown to begin with (the rail and drawers are "tools"), and the
// tools can be shown or hidden in any view. A view also has its own day/night
// colours (palette.js). More views (radar, sounder) are added to VIEWS.
//
// `racenav-view` fires on window with detail { id, chrome } when the view or its
// tools change; modules that need to re-lay themselves out listen for it.
// Each window has its own view (and tools/chart choices), so any window can show any view.
import { own, PRIMARY, LABEL, startView } from "./role.js";
import { patchWindow } from "./windowstate.js";
import { createPlanningIdle } from "./planning-idle.js";

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
    id: "env", name: "Environment", tools: false, map: true,
    icon: '<path d="M3 16c3-2 5-2 8 0s5 2 10-1M3 11c3-2 5-2 8 0s5 2 10-1"/><circle cx="16" cy="6" r="2"/>',
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
export const OPERATIONAL_VIEWS = VIEWS.filter((v) => v.id !== "nmea");

const $ = (id) => document.getElementById(id);

export function setupViews(map, { onPlanningTimeout = () => {}, livePosition = () => true } = {}) {
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
  const chartPositions = new Map();
  const chartViews = new Set(["nav", "plan", "env"]);

  function apply() {
    const v = def(s.current);
    const tools = toolsOn(v.id);
    const chart = mapOn(v.id);
    document.documentElement.classList.toggle("no-tools", !tools);
    document.documentElement.classList.toggle("no-map", !chart);
    document.documentElement.dataset.view = v.id;
    const vb = $("view-btn");
    vb.querySelector("svg").innerHTML = v.icon;
    vb.title = `View: ${v.name}. Click to choose a view or show or hide its chart (F2, F3, F4)`;
    vb.setAttribute("aria-label", `View: ${v.name}`);
    for (const button of document.querySelectorAll("[data-view-select]")) {
      const current = button.dataset.viewSelect === v.id;
      button.classList.toggle("on", current);
      button.setAttribute("aria-pressed", String(current));
    }
    const tb = $("tools-btn");
    tb.classList.toggle("on", tools);
    tb.title = tools ? `Hide the rail and drawers in the ${v.name} view` : `Show the rail and drawers in the ${v.name} view`;
    save();
    if (!PRIMARY) patchWindow(LABEL, { view: v.id });
    window.dispatchEvent(new CustomEvent("racenav-view", { detail: { id: v.id, tools } }));
    window.dispatchEvent(new Event("racenav-layout"));
    requestAnimationFrame(() => map.resize());
  }

  // a click lists the views by name (a new user can't guess what a cycling button will do next);
  // F2–F4 and the keys list switch directly
  $("view-btn").onclick = () => (pop.hidden ? openPop() : (pop.hidden = true));
  $("tools-btn").onclick = () => { s.tools[s.current] = !toolsOn(s.current); apply(); };

  // right-click (or hold) the view button: pick a view, and show or hide the chart in this one
  const pop = document.createElement("div");
  pop.id = "view-pop";
  pop.className = "ctx view-pop";
  pop.hidden = true;
  document.body.append(pop);
  function openPop() {
    const v = def(s.current);
    pop.innerHTML = `<div class="grp">View</div>${OPERATIONAL_VIEWS.map((x) => `<button data-v="${x.id}" class="${x.id === v.id ? "cur" : ""}">${x.name}</button>`).join("")}
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

  // Esc leaves the Live data and Instruments views for the view you came from (when nothing else
  // is open and the key isn't for a text box), so there is always a way back without F-keys
  let previous = "nav";
  window.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || e.defaultPrevented || !["nmea", "inst"].includes(s.current)) return;
    if (e.target.closest?.("input, textarea, select, [contenteditable]")) return;
    if (document.querySelector(".ctx:not([hidden]), dialog[open]")) return;
    api.set(previous === s.current ? "nav" : previous);
  });
  window.addEventListener("racenav-need-tools", () => { if (!toolsOn(s.current)) { s.tools[s.current] = true; apply(); } });
  const api = {
    current: () => s.current,
    set(id) {
      if (!def(id) || id === s.current) return;
      if (chartViews.has(s.current) || chartViews.has(id)) map.stop();
      if (chartViews.has(s.current)) {
        const { lng, lat } = map.getCenter();
        chartPositions.set(s.current, { center: [lng, lat], zoom: map.getZoom() });
      }
      if (!["nmea", "inst"].includes(s.current)) previous = s.current;
      s.current = id;
      if (chartViews.has(id) && chartPositions.has(id)) map.jumpTo(chartPositions.get(id));
      apply();
    },
    onChange(f) { window.addEventListener("racenav-view", (e) => f(e.detail.id, e.detail.tools)); },
  };
  const notice = document.createElement("div");
  notice.id = "idle-notice";
  notice.hidden = true;
  notice.setAttribute("role", "alert");
  notice.innerHTML = `<span></span><button type="button"></button>`;
  document.body.append(notice);
  notice.querySelector("button").onclick = () => idle.activity();
  const timeoutKey = own("racenav.planningTimeout");
  const timeoutSelect = $("planning-timeout");
  const validTimeout = (value) => Number.isInteger(value) && value >= 0 && value <= 10;
  let timeout = 5;
  try {
    const savedTimeout = localStorage.getItem(timeoutKey);
    if (savedTimeout !== null && validTimeout(Number(savedTimeout))) timeout = Number(savedTimeout);
  } catch (error) { console.error("Could not read Planning timeout:", error); }
  timeoutSelect.value = String(timeout);
  const idle = createPlanningIdle({
    now: () => Date.now(), schedule: (fn, delay) => setTimeout(fn, delay), cancel: clearTimeout,
    currentView: () => s.current, timeoutMinutes: () => timeout,
    onTimeout: () => { const previousView = s.current; api.set("nav"); onPlanningTimeout(previousView); },
    // only while a live position is arriving: at the desk there is nothing to return for
    active: () => { try { return !!livePosition(); } catch { return false; } },
    // a notice for the last 30 seconds; any input (or Stay) cancels it
    warnMs: 30000,
    onWarn: (seconds) => {
      const name = s.current === "env" ? "Environment" : "Planning";
      notice.querySelector("span").textContent = `No input: returning to Navigation in ${seconds} s`;
      notice.querySelector("button").textContent = `Stay in ${name}`;
      notice.hidden = false;
    },
    onWarnEnd: () => { notice.hidden = true; },
  });
  timeoutSelect.addEventListener("change", () => {
    const value = Number(timeoutSelect.value);
    if (!validTimeout(value)) { timeoutSelect.value = String(timeout); return; }
    timeout = value;
    try { localStorage.setItem(timeoutKey, String(timeout)); } catch (error) { console.error("Could not save Planning timeout:", error); }
    idle.activity();
  });
  for (const type of ["pointerdown", "pointermove", "wheel", "keydown", "touchstart"]) {
    window.addEventListener(type, (event) => {
      if (!event.isTrusted || !["plan", "env"].includes(s.current)) return;
      idle.check();
      if (["plan", "env"].includes(s.current)) idle.activity();
    }, { passive: true, capture: true });
  }
  document.addEventListener("visibilitychange", idle.check);
  window.addEventListener("focus", idle.check);
  let lastView = s.current;
  api.onChange((id) => {
    if (id !== lastView) { lastView = id; idle.activity(); }
  });
  for (const button of document.querySelectorAll("[data-view-select]")) {
    button.addEventListener("click", () => api.set(button.dataset.viewSelect));
  }
  apply();
  idle.activity();
  return api;
}
