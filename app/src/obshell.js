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
import "@oicl/openbridge-webcomponents/dist/icons/icon-connection-4-bars.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-media-live.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-media-pause.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-center-iec.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-center-off-iec.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-measure-google.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-radar-electronic-range-and-bearing-proposal.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-media-record.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-chart-seabed-proposal.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-table.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-ais-target-activated-iec.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-delete.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-camera.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-view-gallery-google.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-panel-right-open.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-panel-left-open.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-own-ship-iec.js";
import "@oicl/openbridge-webcomponents/dist/icons/icon-waypoint-edit-iec.js";
import { paletteApi } from "./palette.js";
import { setupOpenBridgeAlerts } from "./obalerts.js";

// the drawers, in the rail's order: [pane, label, icon]
const PANES = [
  ["weather", "Weather and tides", "obi-weather"],
  ["course", "Route", "obi-navigation-route"],
  ["routing", "Weather routing", "obi-monitoring-route"],
  ["fleet", "Targets and fleets", "obi-fleet-proposal"],
  ["library", "Library", "obi-media-library"],
  ["chart", "Chart", "obi-chart-layers"],
  ["inst", "Data panel and instruments", "obi-dashboard"],
  ["alarms", "Alarms", "obi-alarm-general"],
];
// the views, at the top of the navigation menu: [view, label, icon]
const VIEWS = [
  ["nav", "Navigation", "obi-own-ship-iec"],
  ["plan", "Planning", "obi-waypoint-edit-iec"],
  ["env", "Environment", "obi-chart-seabed-proposal"],
  ["inst", "Instruments", "obi-view-gallery-google"],
  ["nmea", "Live data", "obi-table"],
];
// the chart toolbar's buttons and their OpenBridge icons; two icons: the first when the button is on
const TOOL_ICONS = {
  "sk-state": ["obi-connection-4-bars"], "sk-live": ["obi-media-live", "obi-media-pause"],
  follow: ["obi-center-iec", "obi-center-off-iec"], "ruler-btn": ["obi-measure-google"],
  "ebl-btn": ["obi-radar-electronic-range-and-bearing-proposal"], "pb-rec": ["obi-media-record"], "pb-tb": ["obi-chart-seabed-proposal"],
  "readout-btn": ["obi-table"], "targets-btn": ["obi-ais-target-activated-iec"], "clear-track": ["obi-delete"], "shot-btn": ["obi-camera"],
  "side-btn": ["obi-panel-right-open"], "tools-btn": ["obi-panel-left-open"],
};
// On/off settings that take effect at once are OpenBridge toggle switches; check boxes stay for
// choosing items in a list (chart layers, sources, filters, names on targets)
const TOGGLES = [
  // alarms
  "al-shallow", "al-contour", "al-air", "al-xte", "al-arrive", "al-poslost", "ais-cpa-on", "ais-ignore-moored", "ais-net-alarms", "mar-alarm", "env-use-past",
  // weather, routes, routing
  "gf-auto", "stn-show", "rt-great-circle", "rt-current", "rt-hovertime", "set-avoid-dangers", "set-avoid-farms", "rt-barriers",
  // targets, library
  "ais-show", "of-only", "lib-show", "trk-time", "bar-hatch", "evt-auto",
  // chart: display settings, own-ship lines, online maps, own soundings
  "set-shallow-dangers", "set-shallow-pattern", "set-contour-labels", "set-sectors", "set-full-light-lines", "set-featnames", "set-soundings",
  "set-info", "set-text-important", "set-text-other", "set-datesym", "set-hdg-line", "set-cog-line", "set-beam-line", "set-setdrift", "set-own-rings",
  "inst-arrows", "wm-on", "wm-cached", "pb-live", "pb-check",
  // settings
  "boat-lay-show", "boat-lay-cone-show", "set-rec-all", "set-anchor-btn", "nmea-failover-enabled",
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
  setupOpenBridgeAlerts(bar.querySelector("#ob-actions"));
  bar.tall = true; // the taller bar: room for MOB, Drop and the anchor at a safe size
  document.body.prepend(bar);
  const clock = bar.querySelector("obc-clock");
  const tick = () => { clock.date = new Date().toISOString(); };
  tick();
  setInterval(tick, 1000);

  const nav = document.createElement("obc-navigation-menu");
  nav.id = "ob-nav";
  nav.softDismiss = true;
  nav.innerHTML = VIEWS.map(([v, label, icon]) =>
    `<obc-navigation-item slot="main" label="${label}" data-view-select="${v}"><${icon} slot="icon"></${icon}></obc-navigation-item>`).join("") +
    '<div slot="main" class="ob-sep" role="separator"></div>' +
    PANES.map((p) => item(...p)).join("") +
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
    for (const it of nav.querySelectorAll("obc-navigation-item")) {
      it.checked = it.dataset.viewSelect ? it.dataset.viewSelect === root.dataset.view : !!it.dataset.pane && it.dataset.pane === on;
    }
  }
  bar.addEventListener("menu-button-clicked", () => setNav(nav.hidden));
  nav.addEventListener("close", () => setNav(false));
  nav.addEventListener("click", (e) => {
    const it = e.target.closest("obc-navigation-item");
    if (!it) return;
    if (it.dataset.viewSelect) { /* views.js switches the view */ } else if (it.dataset.pane) document.querySelector(`#rail button[data-pane="${it.dataset.pane}"]`)?.click();
    else if (it.dataset.act === "manual") $("manual-btn")?.click();
    else if (it.dataset.act === "about") $("brand-btn")?.click();
    setNav(false);
  });
  // the drawers can be opened from elsewhere too (keys, links): keep the menu's tick right
  new MutationObserver(syncNav).observe($("rail"), { subtree: true, attributes: true, attributeFilter: ["class"] });

  // ---------- chart toolbar: OpenBridge icons (the buttons and what they do are unchanged) ----------
  for (const [id, names] of Object.entries(TOOL_ICONS)) {
    const b = $(id);
    if (!b) continue;
    b.querySelector("svg")?.remove();
    b.insertAdjacentHTML("afterbegin", names.map((n, i) => `<${n} class="ob-ic${names.length > 1 ? (i ? " ob-off" : " ob-on") : ""}"></${n}>`).join(""));
  }

  // ---------- toggle switches ----------
  for (const id of TOGGLES) {
    const c = $(id);
    if (c?.type === "checkbox") { c.classList.add("ob-toggle"); c.setAttribute("role", "switch"); }
  }

  // a label holding a check box or toggle: the text wraps beside the box, not under it
  for (const c of document.querySelectorAll('label > input[type="checkbox"]')) c.parentElement.classList.add("ob-check");

  // ---------- page name: the view ----------
  const page = () => { bar.pageName = VIEW_NAMES[root.dataset.view] ?? ""; syncNav(); };
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
