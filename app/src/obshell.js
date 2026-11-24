// The OpenBridge frame round the chart: a top bar across the window (menu, view name, MOB, Drop and
// the anchor, clock, colours), the navigation menu that opens the drawers, and the brilliance menu
// (palette and brightness). OpenBridge web components (@oicl/openbridge-webcomponents, AGPL-3.0;
// see THIRD-PARTY-NOTICES.md). The drawers themselves are the same as before: the menu opens them
// by the rail's buttons, which stay in the page (hidden) so keys, links and the manual still work.

import "@oicl/openbridge-webcomponents/dist/components/top-bar/top-bar.js";
import "@oicl/openbridge-webcomponents/dist/components/clock/clock.js";
import "@oicl/openbridge-webcomponents/dist/components/navigation-menu/navigation-menu.js";
import "@oicl/openbridge-webcomponents/dist/components/navigation-item/navigation-item.js";
import "@oicl/openbridge-webcomponents/dist/components/brilliance-menu/brilliance-menu.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-weather.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-navigation-route.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-monitoring-route.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-fleet-proposal.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-media-library.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-chart-layers.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-dashboard.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-alarm-general.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-manual.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-settings-iec.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-info.js";
import { paletteApi } from "./palette.js";

// the drawers, in the rail's order: [pane, label, icon]
const PANES = [
  ["weather", "Weather and tides", "obi-weather"],
  ["course", "Route", "obi-navigation-route"],
  ["routing", "Weather routing", "obi-monitoring-route"],
  ["fleet", "Targets and fleets", "obi-fleet-proposal"],
  ["library", "Library", "obi-media-library"],
  ["chart", "Chart", "obi-chart-layers"],
  ["inst", "Instruments", "obi-dashboard"],
  ["alarms", "Alarms", "obi-alarm-general"],
];
const VIEW_NAMES = { nav: "Navigation", plan: "Planning", env: "Environment", inst: "Instruments", nmea: "Live data" };
// OpenBridge palettes and TweedPlot's colour modes (Night (red) and (green) are TweedPlot's own: Settings › Display)
const TO_OBC = { bright: "bright", day: "day", dusk: "dusk", "iho-night": "night", "night-red": "night", "night-green": "night" };
const FROM_OBC = { bright: "bright", day: "day", dusk: "dusk", night: "iho-night" };

const $ = (id) => document.getElementById(id);
const item = (pane, label, icon) =>
  `<obc-navigation-item slot="main" has-icon label="${label}" data-pane="${pane}"><${icon} slot="icon"></${icon}></obc-navigation-item>`;

export function setupOpenBridgeShell() {
  const root = document.documentElement;
  root.classList.add("ob-shell");

  const bar = document.createElement("obc-top-bar");
  bar.id = "ob-topbar";
  bar.appTitle = "TweedPlot";
  bar.showClock = true;
  bar.showDimmingButton = true;
  bar.showAppsButton = false;
  bar.showUserButton = false;
  bar.innerHTML = `<div slot="alerts" id="ob-actions"></div><obc-clock slot="clock" show-date></obc-clock>`;
  bar.tall = true; // the taller bar: room for MOB, Drop and the anchor at a safe size
  document.body.prepend(bar);
  const clock = bar.querySelector("obc-clock");
  const tick = () => { clock.date = new Date().toISOString(); };
  tick();
  setInterval(tick, 1000);

  const nav = document.createElement("obc-navigation-menu");
  nav.id = "ob-nav";
  nav.softDismiss = true;
  nav.innerHTML = PANES.map((p) => item(...p)).join("") +
    `<obc-navigation-item slot="footer" has-icon label="Manual" data-act="manual"><obi-manual slot="icon"></obi-manual></obc-navigation-item>
     <obc-navigation-item slot="footer" has-icon label="Settings" data-pane="settings"><obi-settings-iec slot="icon"></obi-settings-iec></obc-navigation-item>
     <obc-navigation-item slot="footer" has-icon label="About TweedPlot" data-act="about"><obi-info slot="icon"></obi-info></obc-navigation-item>`;
  nav.hidden = true;
  for (const it of nav.querySelectorAll("obc-navigation-item")) it.hasIcon = true;
  document.body.append(nav);

  const bm = document.createElement("obc-brilliance-menu");
  bm.id = "ob-brilliance";
  bm.softDismiss = true;
  bm.showBrightPalette = true;
  bm.hidden = true;
  document.body.append(bm);

  // ---------- navigation menu: opens the drawers ----------
  function setNav(open) {
    nav.hidden = !open;
    nav.open = open;
    bar.menuButtonActivated = open;
    if (open) syncNav();
  }
  function syncNav() {
    const on = document.querySelector("#rail button.on")?.dataset.pane;
    for (const it of nav.querySelectorAll("obc-navigation-item")) it.checked = !!it.dataset.pane && it.dataset.pane === on;
  }
  bar.addEventListener("menu-button-clicked", () => setNav(nav.hidden));
  nav.addEventListener("close", () => setNav(false));
  nav.addEventListener("click", (e) => {
    const it = e.target.closest("obc-navigation-item");
    if (!it) return;
    if (it.dataset.pane) document.querySelector(`#rail button[data-pane="${it.dataset.pane}"]`)?.click();
    else if (it.dataset.act === "manual") $("manual-btn")?.click();
    else if (it.dataset.act === "about") $("brand-btn")?.click();
    setNav(false);
  });
  // the drawers can be opened from elsewhere too (keys, links): keep the menu's tick right
  new MutationObserver(syncNav).observe($("rail"), { subtree: true, attributes: true, attributeFilter: ["class"] });

  // ---------- page name: the view ----------
  const page = () => { bar.pageName = VIEW_NAMES[root.dataset.view] ?? ""; };
  new MutationObserver(page).observe(root, { attributes: true, attributeFilter: ["data-view"] });
  page();

  // ---------- brilliance menu: palette and brightness ----------
  function syncBrilliance() {
    if (!paletteApi) return;
    bm.palette = TO_OBC[paletteApi.mode()] ?? "day";
    bm.brightness = paletteApi.dim();
  }
  function setBm(open) {
    bm.hidden = !open;
    bm.open = open;
    bar.dimmingButtonActivated = open;
    if (open) syncBrilliance();
    if (open && paletteApi && !listening) { listening = true; paletteApi.onChange(syncBrilliance); }
  }
  bar.addEventListener("dimming-button-clicked", () => setBm(bm.hidden));
  bm.addEventListener("close", () => setBm(false));
  bm.addEventListener("palette-changed", (e) => paletteApi?.set(FROM_OBC[e.detail.value] ?? "day"));
  bm.addEventListener("brightness-changed", (e) => paletteApi?.setDim(e.detail.value));
  let listening = false;
  window.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    if (!nav.hidden) setNav(false);
    if (!bm.hidden) setBm(false);
  });

  return { actions: $("ob-actions"), bar };
}
