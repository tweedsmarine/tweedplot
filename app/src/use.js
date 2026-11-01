// Use: what the boat is mostly for (Cruising, Fishing or Racing), chosen on the welcome screen or in
// Settings › Display. It decides which optional tools are on show: Fishing shows the shoot and haul
// buttons and own soundings in the toolbar. It is on <html data-use>, and "racenav-use" is sent
// when it changes.
import { own } from "./role.js";

const $ = (id) => document.getElementById(id);
const UKEY = own("racenav.use");
const USES = ["cruise", "fish", "race"];

export function setupUse() {
  // unset (an install from before this setting) shows every tool until a choice is made
  let use = null;
  try { const v = localStorage.getItem(UKEY); if (USES.includes(v)) use = v; } catch { /* ignore */ }
  const paint = () => {
    if (use) document.documentElement.dataset.use = use;
    const sel = $("set-use");
    if (sel) sel.value = use || "";
    window.dispatchEvent(new CustomEvent("racenav-use", { detail: { use } }));
    window.dispatchEvent(new Event("racenav-layout"));
  };
  const setUse = (u) => {
    if (!USES.includes(u) || u === use) return;
    use = u;
    try { localStorage.setItem(UKEY, use); } catch { /* ignore */ }
    paint();
  };
  $("set-use")?.addEventListener("change", (e) => setUse(e.target.value));
  paint();
  return { use: () => use, setUse };
}
