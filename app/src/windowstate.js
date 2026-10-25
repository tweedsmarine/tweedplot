// Remembers where each window was (position, size, maximised) and which monitor
// windows were open, so a work station with several screens comes back as it was left.
import { getCurrentWindow, availableMonitors, PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import { LABEL, PRIMARY } from "./role.js";

const KEY = "racenav.windows"; // label → {view?, x, y, w, h, max, open}
const QUIT = "racenav.quitting";

export function readWindows() {
  try { return JSON.parse(localStorage.getItem(KEY) || "{}"); } catch { return {}; }
}
export function patchWindow(label, o) {
  try {
    const all = readWindows();
    all[label] = { ...(all[label] || {}), ...o };
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch { /* ignore */ }
}

/** Is a window's top-left corner on a screen that is still connected? */
function onScreen(g, screens) {
  return screens.some((m) => g.x >= m.position.x - 50 && g.x < m.position.x + m.size.width - 100 &&
    g.y >= m.position.y - 50 && g.y < m.position.y + m.size.height - 100);
}

export async function setupWindowState() {
  let w;
  try { w = getCurrentWindow(); } catch { return; }
  if (PRIMARY) localStorage.removeItem(QUIT);
  else patchWindow(LABEL, { open: true });

  const g = readWindows()[LABEL];
  if (g && Number.isFinite(g.x)) {
    try {
      if (onScreen(g, await availableMonitors())) {
        if (g.w > 200 && g.h > 150) await w.setSize(new PhysicalSize(g.w, g.h));
        await w.setPosition(new PhysicalPosition(g.x, g.y));
        if (g.max) await w.maximize();
      }
    } catch { /* keep the default place */ }
  }

  let t = 0;
  const remember = () => {
    clearTimeout(t);
    t = setTimeout(async () => {
      try {
        if (await w.isMinimized?.()) return;
        const max = await w.isMaximized();
        if (max) { patchWindow(LABEL, { max: true }); return; }
        const p = await w.outerPosition(), s = await w.innerSize();
        patchWindow(LABEL, { x: p.x, y: p.y, w: s.width, h: s.height, max: false });
      } catch { /* ignore */ }
    }, 400);
  };
  w.onMoved(remember);
  w.onResized(remember);

  // closing the main window closes the app: the monitors that were open stay "open" for next time
  if (PRIMARY) w.onCloseRequested(() => { try { localStorage.setItem(QUIT, String(Date.now())); } catch { /* ignore */ } });
  else w.onCloseRequested(() => {
    if (Date.now() - Number(localStorage.getItem(QUIT) || 0) > 5000) patchWindow(LABEL, { open: false });
  });
}

/** Monitor windows that were open when the app last ran: [{label, view}]. */
export function monitorsToReopen() {
  return Object.entries(readWindows())
    .filter(([l, g]) => l.startsWith("monitor-") && g.open && g.view)
    .map(([label, g]) => ({ label, view: g.view }));
}
