// Detail level: Simple, Standard or Expert. Nothing is removed at a lower level, the less-used
// controls are only tucked away. An element marked data-lv="std" is hidden in Simple, data-lv="exp"
// in Simple and Standard. On a heading (h3, h4) the mark covers everything up to the next heading
// of the same or a higher level, so the markup does not need wrapping.
//
// The level is on <html data-detail>; the bar at the top of the screen slides in when the pointer
// goes there, and Settings › Display has the same choice.
import { own } from "./role.js";
import { VIEWS } from "./views.js";

const KEY = own("racenav.detail");
const LEVELS = ["simple", "standard", "expert"];
const $ = (id) => document.getElementById(id);

function spread() {
  for (const el of document.querySelectorAll("[data-lv]")) {
    const cls = `lv-${el.dataset.lv}`;
    el.classList.add(cls);
    const rank = { H3: 3, H4: 4 }[el.tagName];
    if (!rank) continue;
    for (let s = el.nextElementSibling; s; s = s.nextElementSibling) {
      const r = { H3: 3, H4: 4 }[s.tagName];
      if (r && r <= rank) break;
      s.classList.add(cls);
    }
  }
}

export function setupDetail(views) {
  spread();
  let level = "standard";
  try { const v = localStorage.getItem(KEY); if (LEVELS.includes(v)) level = v; } catch { /* storage unavailable */ }

  const vseg = $("view-seg");
  vseg.innerHTML = VIEWS.map((v) => `<button data-v="${v.id}" title="${v.name}">${v.name}</button>`).join("");
  vseg.onclick = (e) => { const b = e.target.closest("button[data-v]"); if (b) views.set(b.dataset.v); };
  const showView = () => { for (const b of vseg.children) b.classList.toggle("on", b.dataset.v === views.current()); };
  views.onChange(showView);
  showView();

  function apply() {
    document.documentElement.dataset.detail = level;
    for (const b of $("detail-seg").children) b.classList.toggle("on", b.dataset.l === level);
    const sel = $("set-detail");
    if (sel) sel.value = level;
    // the open drawer's rail button may have just gone: close it rather than leave an orphan
    const open = document.querySelector("#rail button.on[data-pane]");
    if (open && open.offsetParent === null) $("drawer-close")?.click();
    window.dispatchEvent(new CustomEvent("racenav-detail", { detail: { level } }));
    window.dispatchEvent(new Event("racenav-layout"));
  }
  const set = (l) => {
    if (!LEVELS.includes(l) || l === level) return;
    level = l;
    try { localStorage.setItem(KEY, level); } catch { /* ignore */ }
    apply();
  };
  $("detail-seg").onclick = (e) => { const b = e.target.closest("button[data-l]"); if (b) set(b.dataset.l); };
  $("set-detail")?.addEventListener("change", (e) => set(e.target.value));

  // Use: what the boat is mostly for. It decides which optional tools are on show.
  const UKEY = own("racenav.use");
  const USES = ["cruise", "fish", "race"];
  // unset (an install from before this setting) shows every tool until a choice is made
  let use = null;
  try { const v = localStorage.getItem(UKEY); if (USES.includes(v)) use = v; } catch { /* ignore */ }
  const paintUse = () => {
    if (use) document.documentElement.dataset.use = use;
    for (const b of $("use-seg").children) b.classList.toggle("on", b.dataset.u === use);
    const sel = $("set-use");
    if (sel) sel.value = use || "";
    window.dispatchEvent(new CustomEvent("racenav-use", { detail: { use } }));
    window.dispatchEvent(new Event("racenav-layout"));
  };
  const setUse = (u) => {
    if (!USES.includes(u) || u === use) return;
    use = u;
    try { localStorage.setItem(UKEY, use); } catch { /* ignore */ }
    paintUse();
  };
  $("use-seg").onclick = (e) => { const b = e.target.closest("button[data-u]"); if (b) setUse(b.dataset.u); };
  $("set-use")?.addEventListener("change", (e) => setUse(e.target.value));
  paintUse();

  // The bar opens on hover; the pin (or a click on the bar's own background, or the grip on touch)
  // keeps it open until it is unpinned.
  const zone = $("topbar-zone");
  const PIN = own("racenav.topbar.pinned");
  const pin = (on) => {
    zone.classList.toggle("pinned", on);
    $("topbar-pin").setAttribute("aria-pressed", String(on));
    $("topbar-pin").classList.toggle("on", on);
    try { localStorage.setItem(PIN, on ? "1" : "0"); } catch { /* ignore */ }
  };
  try { pin(localStorage.getItem(PIN) === "1"); } catch { /* ignore */ }
  const toggle = () => pin(!zone.classList.contains("pinned"));
  $("topbar-pin").onclick = toggle;
  $("topbar-grip").onclick = toggle;
  $("topbar").addEventListener("click", (e) => { if (e.target === $("topbar") || e.target.classList.contains("tb-lab")) toggle(); });
  apply();
  return { level: () => level, set, use: () => use, setUse };
}
