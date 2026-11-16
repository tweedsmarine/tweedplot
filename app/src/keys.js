// Keyboard shortcuts, all in one table: the handler, the "?" sheet, the key shown in each
// button's tooltip and the manual's list (its {keys} table, manual.js) all come from KEYS, so
// they can't drift apart.
//
// Single keys only work when you aren't typing (a text box, number box or list has the
// keyboard); keys with Ctrl work everywhere. The arrow keys and + / − move the chart
// whenever the keyboard isn't in a drawer, the manual, a dialog or a floating window, so the
// chart no longer has to be clicked first.
//
// Some keys are handled elsewhere and are only listed here (`own: true`): F1 (manual.js),
// F11 (winchrome.js), Esc (each panel closes itself).

const $ = (id) => document.getElementById(id);
const click = (id) => () => { const b = $(id); if (b && !b.disabled) b.click(); };

/**
 * group, keys (as shown), what it does, and run(ctx). `btn`: the button whose tooltip shows
 * the key. `map`: only while the chart is shown. `own`: handled by another module.
 */
export const KEYS = [
  { group: "Views", keys: "F2", label: "Navigation view", run: (c) => c.views.set("nav") },
  { group: "Views", keys: "F3", label: "Planning view", run: (c) => c.views.set("plan") },
  { group: "Views", keys: "F4", label: "Instruments view", run: (c) => c.views.set("inst") },
  { group: "Views", keys: "F6", label: "Environment view", run: (c) => c.views.set("env") },
  { group: "Views", keys: "P", label: "Show or hide the rail and drawers", btn: "tools-btn", run: click("tools-btn") },
  { group: "Views", keys: "O", label: "Show the data panel beside the chart, or hide it to a line under the chart", btn: "side-btn", run: click("side-btn") },
  { group: "Views", keys: "Esc", label: "In the Instruments or Live data view: back to the view you came from", own: true },
  { group: "Views", keys: "F11", label: "Full screen", own: true },

  { group: "Drawers (the same key again closes it)", keys: "W", label: "Weather and tides", pane: "weather" },
  { group: "Drawers (the same key again closes it)", keys: "R", label: "Route", pane: "course" },
  { group: "Drawers (the same key again closes it)", keys: "G", label: "Weather routing", pane: "routing" },
  { group: "Drawers (the same key again closes it)", keys: "L", label: "Library", pane: "library" },
  { group: "Drawers (the same key again closes it)", keys: "C", label: "Chart", pane: "chart" },
  { group: "Drawers (the same key again closes it)", keys: "I", label: "Instruments (the data panel, the Instruments view, history)", pane: "inst" },
  { group: "Drawers (the same key again closes it)", keys: "A", label: "Alarms", pane: "alarms" },
  { group: "Drawers (the same key again closes it)", keys: "S", label: "Settings", pane: "settings" },
  { group: "Drawers (the same key again closes it)", keys: "Esc", label: "Close the drawer (or the menu, box or mode that is open)", own: true },

  { group: "Chart", keys: "← → ↑ ↓", label: "Move the chart (turns Follow off)", own: true },
  { group: "Chart", keys: "Shift+← Shift+→", label: "Turn the chart", own: true },
  { group: "Chart", keys: "+ −", label: "Zoom in and out", own: true },
  { group: "Chart", keys: "U", label: "North up", btn: "north-btn", map: true, run: click("north-btn") },
  { group: "Chart", keys: "B", label: "Centre on the boat (and follow it)", map: true, run: (c) => c.centreBoat() },
  { group: "Chart", keys: "F", label: "Follow the boat on or off", btn: "follow", map: true, run: click("follow") },
  { group: "Chart", keys: "M", label: "Measure (ruler); Esc stops", btn: "ruler-btn", map: true, run: click("ruler-btn") },
  { group: "Chart", keys: "V", label: "EBL/VRM from the boat (click fixes it, Esc cancels)", btn: "ebl-btn", map: true, run: click("ebl-btn") },
  { group: "Chart", keys: "T", label: "Target list", btn: "targets-btn", run: click("targets-btn") },
  { group: "Chart", keys: "N", label: "Colours: day, dusk, night (IHO), night red, night green", btn: "palette-btn", run: click("palette-btn") },
  { group: "Chart", keys: "Space", label: "Live or paused", btn: "sk-live", run: click("sk-live") },

  { group: "Weather time", keys: "[", label: "Back one step", run: (c) => c.weatherStep(-1) },
  { group: "Weather time", keys: "]", label: "Forward one step", run: (c) => c.weatherStep(1) },
  { group: "Weather time", keys: "Shift+[", label: "Back 6 hours", run: (c) => c.weatherHours(-6) },
  { group: "Weather time", keys: "Shift+]", label: "Forward 6 hours", run: (c) => c.weatherHours(6) },
  { group: "Weather time", keys: "Home", label: "Now", run: click("side-wx-now") },

  { group: "Marks and library", keys: "D", label: "Record own soundings on or off", btn: "pb-rec", map: true, run: click("pb-rec") },
  { group: "Marks and library", keys: "K", label: "Drop anchor, or weigh it when anchored (press twice)", btn: "anchor-btn", map: true, run: click("anchor-btn") },
  { group: "Marks and library", keys: "E", label: "Drop a mark at the boat (event mark, no alarm)", btn: "drop-btn", run: click("drop-btn") },
  { group: "Marks and library", keys: "Ctrl+F", label: "Find in the library", run: (c) => c.findInLibrary() },

  { group: "Safety", keys: "Ctrl+M", label: "Man overboard", btn: "mob-btn", run: click("mob-btn") },
  { group: "Safety", keys: "Shift+A", label: "Silence the sounding alarms (they stay on the Alarms list while their cause lasts)", run: (c) => c.silence() },

  { group: "Help", keys: "F1", label: "User manual", own: true },
  { group: "Help", keys: "?", label: "This list of keys", run: (c) => c.sheet() },
];

/** Parse "Ctrl+Shift+[" into what a keydown must match. */
export function parseKey(spec) {
  const parts = spec.split("+");
  const last = parts.pop() || "+";
  return { ctrl: parts.includes("Ctrl"), shift: parts.includes("Shift"), alt: parts.includes("Alt"), key: last };
}

/** Whether a keydown event is this key (letters by character, brackets by position, so any layout works). */
export function matches(spec, e) {
  const k = parseKey(spec);
  const ctrl = e.ctrlKey || e.metaKey;
  if (ctrl !== k.ctrl || e.altKey !== k.alt) return false;
  if (k.key === "?") return e.key === "?";
  if (e.shiftKey !== k.shift) return false;
  if (k.key === "[") return e.code === "BracketLeft";
  if (k.key === "]") return e.code === "BracketRight";
  if (k.key === "Space") return e.code === "Space";
  if (k.key.length === 1) return e.key.toLowerCase() === k.key.toLowerCase();
  return e.key === k.key;
}

/** Whether the focused element uses the keys itself (typing, choosing, sliding). */
export function typingIn(el) {
  if (!el || el === document.body) return false;
  if (el.isContentEditable) return true;
  if (el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  if (el.tagName === "INPUT") return !["checkbox", "button", "submit", "reset", "color", "file"].includes(el.type);
  return !!el.closest('[role="listbox"], [role="menu"], [role="slider"], [role="grid"], [role="tree"]');
}

/** Places whose own scrolling or keys come first for the arrow keys. */
const PANELS = "#drawer, #manual, #dock, .float-win, .ask-back, #welcome, [role=\"dialog\"], .ctx, .sel-pop";

export function setupKeys(map, ctx) {
  const c = { ...ctx, sheet: openSheet };
  const busy = () => !!document.querySelector(".ask-back:not([hidden]), #welcome:not([hidden]), .sel-pop:not([hidden]), .ctx:not([hidden]), #keys-sheet:not([hidden])");

  // the key in each button's tooltip
  for (const k of KEYS) if (k.btn && $(k.btn)) $(k.btn).dataset.key = k.keys;
  for (const k of KEYS) if (k.pane) { const b = document.querySelector(`#rail button[data-pane="${k.pane}"]`); if (b) b.dataset.key = k.keys; }

  window.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.repeat && !/^Arrow|^[+=-]$/.test(e.key)) return;
    const ae = document.activeElement;
    const typing = typingIn(ae);
    const noMap = document.documentElement.classList.contains("no-map");

    // the chart's own keys, without clicking it first
    if (!typing && !noMap && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const inCanvas = map.getCanvasContainer().contains(ae);
      const arrow = /^Arrow/.test(e.key);
      if (arrow && !e.shiftKey && inCanvas) c.followOff?.(); // MapLibre moves it
      if (!inCanvas && !(ae && ae !== document.body && ae.closest(PANELS)) && !busy()) {
        const pan = { ArrowLeft: [-100, 0], ArrowRight: [100, 0], ArrowUp: [0, -100], ArrowDown: [0, 100] }[e.key];
        if (pan && !e.shiftKey) { e.preventDefault(); c.followOff?.(); map.panBy(pan, { duration: 150 }); return; }
        if (arrow && e.shiftKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
          e.preventDefault();
          map.easeTo({ bearing: map.getBearing() + (e.key === "ArrowLeft" ? 15 : -15), duration: 200 });
          return;
        }
        if (e.key === "+" || e.key === "=") { e.preventDefault(); map.zoomIn({ duration: 200 }); return; }
        if (e.key === "-" || e.key === "_") { e.preventDefault(); map.zoomOut({ duration: 200 }); return; }
      }
    }

    for (const k of KEYS) {
      if (k.own || !matches(k.keys, e)) continue;
      const withCtrl = parseKey(k.keys).ctrl || /^F\d+$/.test(k.keys);
      if (!withCtrl && (typing || busy())) return;
      if (k.map && noMap) return;
      // Space on a focused button or tick box presses that, as usual
      if (k.keys === "Space" && ae && (ae.tagName === "BUTTON" || ae.tagName === "INPUT" || ae.tagName === "SUMMARY")) return;
      // Home in a drawer or window scrolls it to the top, as usual
      if (k.keys === "Home" && ae && ae !== document.body && ae.closest(PANELS)) return;
      e.preventDefault();
      if (k.pane) c.ui.open(c.ui.current() === k.pane ? null : k.pane);
      else k.run(c);
      return;
    }
  });

  // ---------- the "?" sheet --------------------------------------------------------------
  const sheet = document.createElement("div");
  sheet.id = "keys-sheet";
  sheet.className = "ask-back keys-back";
  sheet.hidden = true;
  sheet.setAttribute("role", "dialog");
  sheet.setAttribute("aria-label", "Keyboard shortcuts");
  const groups = [...new Set(KEYS.map((k) => k.group))];
  const esc = (s) => s.replace(/[&<>]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[ch]);
  sheet.innerHTML = `<div class="ask keys-box"><div class="wx-row"><b class="ask-title grow">Keyboard shortcuts</b><button class="keys-close" title="Close (Esc)">✕</button></div>
    <p class="wx-note">Single keys work when you aren't typing in a box. The arrow keys and + / − move the chart unless the keyboard is in a drawer or window.</p>
    <div class="keys-cols">${groups.map((g) => `<section><h4>${esc(g)}</h4><table>${KEYS.filter((k) => k.group === g)
      .map((k) => `<tr><td>${k.keys.split(" ").map((x) => `<kbd>${esc(x)}</kbd>`).join(" ")}</td><td>${esc(k.label)}</td></tr>`).join("")}</table></section>`).join("")}</div></div>`;
  document.body.append(sheet);
  const close = () => { sheet.hidden = true; };
  sheet.querySelector(".keys-close").onclick = close;
  sheet.addEventListener("click", (e) => { if (e.target === sheet) close(); });
  window.addEventListener("keydown", (e) => {
    if (sheet.hidden) return;
    if (e.key === "Escape" || e.key === "?") { e.preventDefault(); e.stopImmediatePropagation(); close(); }
  }, true);
  function openSheet() { sheet.hidden = false; sheet.querySelector(".keys-close").focus(); }
  return { open: openSheet };
}
