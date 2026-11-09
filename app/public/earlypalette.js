// Before anything is drawn: this window's colours, so a night start doesn't flash a bright
// splash. palette.js takes over once the page is up (same key: racenav.palette, @label for monitors).
(function () {
  try {
    var t = window.__TAURI_INTERNALS__;
    var label = (t && t.metadata && t.metadata.currentWindow && t.metadata.currentWindow.label) || "main";
    var p = JSON.parse(localStorage.getItem("racenav.palette" + (label === "main" ? "" : "@" + label)) || "{}");
    if (!p.mode) return;
    var root = document.documentElement;
    root.dataset.palette = p.mode;
    var dims = { dusk: 75, "iho-night": 100, "night-red": 60, "night-green": 60 };
    var d = (p.dim && p.dim[p.mode] != null) ? p.dim[p.mode] : dims[p.mode];
    if (d != null) root.style.setProperty("--pal-dim", String(d / 100));
  } catch (e) { /* storage unavailable: day */ }
})();
