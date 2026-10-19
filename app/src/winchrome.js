// The window itself: a dark title bar with the dusk and night colours (Windows draws
// it light otherwise, a bright strip across the top at night), and full screen
// with F11 or a button. Outside Tauri (the browser preview) these quietly do nothing.
import { getCurrentWindow } from "@tauri-apps/api/window";

const win = () => { try { return getCurrentWindow(); } catch { return null; } };

/** Dark title bar for dusk and night; day follows Windows' own setting. */
export async function setWindowDark(dark) {
  try { await win()?.setTheme(dark ? "dark" : null); } catch { /* not in Tauri */ }
}

export async function toggleFullscreen() {
  try { const w = win(); if (w) await w.setFullscreen(!(await w.isFullscreen())); } catch { /* not in Tauri */ }
}

/** F11 toggles full screen in this window. */
export function installFullscreenKey() {
  window.addEventListener("keydown", (e) => {
    if (e.key === "F11") { e.preventDefault(); toggleFullscreen(); }
  });
}
