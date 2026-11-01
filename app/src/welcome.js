// The welcome screen: shown once on a new install (and from Settings › Display), it asks what the
// boat is mostly used for (Use). That can be changed at any time.
// An install that already has saved views is an existing user: no welcome screen, they are not
// moved around.
import { PRIMARY } from "./role.js";

const KEY = "racenav.welcomed";
const SEEDED = "racenav.use.seeded";
// read before the views are saved for the first time
const existingUser = (() => { try { return localStorage.getItem("racenav.views") !== null; } catch { return true; } })();

const T = (k, c, r, w, h) => ({ k, c, r, w, h });
const TASKS = {
  cruise: {
    name: "Cruising", icon: "⛵", note: "chart, position, depth, wind", panels: [],
    screen: [T("HIGHWAY", 1, 1, 8, 4), T("WDIAL", 9, 1, 4, 4), T("COG", 1, 5, 4, 4), T("SOG", 5, 5, 4, 4), T("DPT", 9, 5, 4, 4)],
  },
  fish: {
    name: "Fishing", icon: "🎣", note: "shoot and haul, depth history, sea temperature, currents", panels: [],
    // the sounder (depth history) only here, on the Instruments screen: never in Navigation or Planning by default
    screen: [T("SOUNDER", 1, 1, 8, 4), T("WDIAL", 9, 1, 4, 4), T("DPT", 1, 5, 4, 4), T("COG", 5, 5, 4, 4), T("SOG", 9, 5, 4, 4)],
  },
  race: {
    name: "Racing", icon: "🏁", note: "wind instruments, polars, routing",
    panels: [{ id: "prace", x: 0.3, y: 0.74, dir: "row", size: "m", views: ["nav"], name: "Sailing", tiles: ["BSP", "HDG", "AWS", "AWA", "TWS", "TWA", "VMG"] }],
    screen: [T("HIGHWAY", 1, 1, 8, 4), T("WDIAL", 9, 1, 4, 4), T("COG", 1, 5, 6, 4), T("SOG", 7, 5, 6, 4)],
  },
};

export function setupWelcome({ use, inst }) {
  if (!PRIMARY) return;
  const done = () => { try { localStorage.setItem(KEY, "1"); } catch { /* ignore */ } };
  let seen = false;
  try { seen = localStorage.getItem(KEY) === "1"; } catch { /* ignore */ }
  if (existingUser && !seen) { done(); seen = true; }

  let task = "cruise";
  const el = document.createElement("div");
  el.id = "welcome";
  el.hidden = true;
  el.setAttribute("role", "dialog");
  el.setAttribute("aria-label", "Welcome to TweedPlot");
  el.innerHTML = `<div class="wl-box">
    <h1>Welcome to TweedPlot</h1>
    <p class="wl-lead">What is the boat mostly for? This only sets a starting point; everything stays available, and you can change it at any time in Settings › Display.</p>
    <div class="wl-tasks">${Object.entries(TASKS).map(([k, t]) => `<button data-task="${k}"><span class="wl-ic">${t.icon}</span><b>${t.name}</b><small>${t.note}</small></button>`).join("")}</div>
    <p class="wl-note">Navigation, Planning and Instruments are on the view button in the toolbar.</p>
    <div class="wl-go"><button class="wl-skip" data-act="skip">Not now</button><button class="wl-start" data-act="start">Start</button></div>
  </div>`;
  document.body.append(el);

  const paint = () => {
    for (const b of el.querySelectorAll("[data-task]")) b.classList.toggle("on", b.dataset.task === task);
  };
  el.addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    if (b.dataset.task) task = b.dataset.task;
    else if (b.dataset.act === "skip") { done(); el.hidden = true; return; }
    else if (b.dataset.act === "start") {
      use.setUse(task);
      done();
      el.hidden = true;
      return;
    }
    paint();
  });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !el.hidden) { done(); el.hidden = true; } });

  // a Use choice adds its instrument panels and takes away the other uses' ones, but only while they
  // are as the choice made them: a panel the user has changed (tiles added or taken out) is theirs now
  const addPanels = (u) => {
    const keep = new Set((TASKS[u]?.panels || []).map((p) => p.id));
    for (const k of Object.keys(TASKS)) {
      if (k === u) continue;
      for (const p of TASKS[k].panels) if (!keep.has(p.id)) inst.removePanel(p.id, p.tiles);
    }
    // the panels of earlier test versions
    inst.removePanel("pfish", null);
    inst.removePanel("pdepth", null);
    inst.removePanel("psounder", ["SOUNDER"]); // from a test build that put the sounder in Navigation
    // the Instruments screen is seeded once, by the first choice; later changes never touch it
    try {
      if (localStorage.getItem(SEEDED) !== "1") {
        inst.setScreen(TASKS[u]?.screen || []);
        localStorage.setItem(SEEDED, "1");
      }
    } catch { /* ignore */ }
    for (const p of TASKS[u]?.panels || []) inst.ensurePanel(p);
  };
  window.addEventListener("racenav-use", (e) => { if (e.detail.use) addPanels(e.detail.use); });

  const show = () => { task = use.use() || "cruise"; paint(); el.hidden = false; };
  document.getElementById("set-welcome")?.addEventListener("click", show);
  if (!seen) {
    // after the first chart is up: the start-up splash is above everything until then
    const wait = setInterval(() => {
      if (!document.getElementById("startup-splash")?.hidden) return;
      clearInterval(wait);
      show();
    }, 500);
    setTimeout(() => clearInterval(wait), 60000);
  }
}
