// The data panel: the boat's numbers in a panel beside the chart that never covers it, as ECDIS
// and professional ECS do (IMO MSC.191(79) 5.1.3: the chart area kept apart from the data and
// dialogue areas; MSC.1/Circ.1609 S-Mode: heading, COG, SOG, STW, position and time together,
// always in the same place).
//
//  - a status line: view, live/paused/replay, orientation, motion, display scale;
//  - MOB, Drop and Anchor;
//  - the own-ship block (always there): HDG, COG, SOG, STW, depth, position, local and UTC time;
//  - a MOB section while a MOB is set;
//  - sections that change with the view (Navigation, Planning) and the Use (Cruising, Fishing,
//    Racing): starting layouts in sidepresets.js, each editable (✎);
//  - the alerts, at the bottom.
//
// Hidden (O, or ⟩), it leaves one line of own-ship data under the chart. Its sizes follow the
// screen calibration (Settings › Display), like the S-52 symbols, so text is the same size in
// millimetres on every screen. The chart's container is narrowed by the panel (style.css,
// --side-w), so the chart is never under it.
import { ITEMS, readingState, paintGraphic, historyKey, distNm } from "./instruments.js";
import { preset, clean, fromFloating, upgrade, weatherSection as makeWeatherSection, WEATHER_SECTIONS, OWN_SHIP, PANEL_VIEWS, WIDTHS_MM, snapWidth } from "./sidepresets.js";
import { CHANNELS as HIST, openHistoryCard, recorder } from "./history.js";
import { unwrap } from "./histcore.js";
import { s52PxPerMm } from "./chartfeatures.js";
import { displayScale } from "./marinermath.js";
import { VIEWS } from "./views.js";
import { own } from "./role.js";

const KEY = "racenav.side"; // layouts, shared by every window
const PKEY = "racenav.side.prefs"; // side, text size, width (every window)
const HKEY = own("racenav.side.hidden"); // hidden or shown, each window for itself
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function load(key, d) { try { const v = JSON.parse(localStorage.getItem(key) || "null"); return v ?? d; } catch { return d; } }
function store(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch { /* ignore */ } }

export function setupSidePanel({ inst, views, use, map, getAis = () => null, centreOn = () => {}, centreBoat = () => {} }) {
  const side = $("side"), strip = $("side-strip"), root = document.documentElement;
  const body = side.querySelector(".sp-body"), ownEl = side.querySelector(".sp-own"), mobEl = side.querySelector(".sp-mob");
  const pinnedTime = side.querySelector(".sp-pinned-time"), pinnedPoint = side.querySelector(".sp-pinned-point");
  const routeWeather = side.querySelector(".sp-routewx");
  const weatherSection = $("side-weather"), weatherTime = $("side-wx-clock");
  const weatherElements = { weather: weatherSection, weatherTime };
  const weatherParking = $("side-weather-parking");
  const envTabs = side.querySelector(".sp-env-tabs"), envData = side.querySelector(".sp-env-data");
  const filesHome = $("wx-files-home"), filesSection = $("wx-files-section");
  const planCard = document.querySelector('.pane[data-pane="course"] .dcard[data-title="Plan"]');
  const planHome = document.createComment("Route planning controls");
  planCard.before(planHome);
  // in the Route drawer while the planning controls are in the panel: where they went
  const planAway = document.createElement("div");
  planAway.className = "wx-note plan-away";
  planAway.hidden = true;
  planAway.innerHTML = `Planning this route (name, start, marks, how each is reached, departure) is in the data panel beside the chart while the Planning view is on. <button type="button">Show it</button>`;
  planHome.after(planAway);
  /** Open the Plan card in the panel, scroll to it and flash it. */
  function revealPlan() {
    if (!side.contains(planCard)) return false;
    planCard.open = true;
    planCard.scrollIntoView({ block: "start", behavior: "smooth" });
    planCard.classList.remove("flash");
    void planCard.offsetWidth;
    planCard.classList.add("flash");
    return true;
  }
  planAway.querySelector("button").onclick = revealPlan;
  const statusEl = side.querySelector(".sp-status"), actions = side.querySelector(".sp-actions"), alerts = side.querySelector(".sp-alerts");
  const cursorSlot = side.querySelector(".sp-cursor");
  const menu = $("tile-menu");

  const prefs = { side: "right", text: 1, width: "normal", ...load(PKEY, {}) };
  // the panel text is one fixed size (13 px at the calibrated scale), the same as the rest of the app;
  // the old Text size choice is ignored
  prefs.text = 1;
  if (!WIDTHS_MM[prefs.width]) prefs.width = "normal";
  let hidden = load(HKEY, {}) || {}; // { nav: true } = hidden in Navigation
  let data = load(KEY, null);
  if (!data || data.v !== 1 || typeof data.layouts !== "object") data = { v: 1, layouts: {} };
  let editing = false;
  let envTab = "explore";

  // ---------- the old floating panels, once ----------
  if (!data.migrated) {
    data.migrated = true;
    const moved = fromFloating(inst.floatingPanels(), ITEMS);
    if (moved.nav.length || moved.plan.length) {
      data.movedPanels = moved; // added to each Use's layout when it is first made
    }
    store(KEY, data);
  }
  // layouts saved by an earlier version: up to the current starting layouts (sidepresets.js)
  if (upgrade(data)) store(KEY, data);

  const view = () => root.dataset.view || "nav";
  const useNow = () => use.use() || "cruise";
  const key = () => `${view()}.${useNow()}`;
  function layout() {
    const k = key();
    if (!data.layouts[k]) {
      const v = view();
      data.layouts[k] = [...preset(v, useNow()), ...(data.movedPanels?.[v] ?? []).map((s) => ({ ...s, items: s.items ? [...s.items] : undefined }))];
      store(KEY, data);
    }
    data.layouts[k] = clean(data.layouts[k], ITEMS);
    return data.layouts[k];
  }
  const save = () => store(KEY, data);
  window.addEventListener("storage", (e) => {
    if (e.key === KEY && e.newValue) { try { data = JSON.parse(e.newValue); build(); } catch { /* ignore */ } }
    if (e.key === PKEY && e.newValue) { try { Object.assign(prefs, JSON.parse(e.newValue)); geometry(); } catch { /* ignore */ } }
  });

  // ---------- sizes from the screen calibration ----------
  function geometry() {
    const pxmm = s52PxPerMm();
    root.style.setProperty("--pxmm", String(pxmm));
    root.style.setProperty("--sp-k", String(prefs.text));
    const w = Math.round(WIDTHS_MM[prefs.width] * pxmm * prefs.text);
    root.style.setProperty("--side-w", `${w}px`);
    root.classList.toggle("side-left", prefs.side === "left");
    for (const id of ["sp-side", "set-side"]) { const el = $(id); if (el) el.value = prefs.side; }
    if ($("sp-width")) $("sp-width").value = prefs.width;
    for (const g of side.querySelectorAll(".sp-g")) g.style.height = `${Math.round(Number(g.dataset.h) * pxmm * prefs.text)}px`;
    requestAnimationFrame(() => map.resize());
  }
  window.addEventListener("racenav-calibration", geometry);
  const setPref = (k, v) => { prefs[k] = v; store(PKEY, prefs); geometry(); };
  for (const id of ["sp-side", "set-side"]) $(id)?.addEventListener("change", (e) => setPref("side", e.target.value));
  $("sp-width")?.addEventListener("change", (e) => {
    if (WIDTHS_MM[e.target.value]) setPref("width", e.target.value);
  });

  // ---------- shown, hidden (a line under the chart) or not at all (no chart) ----------
  // On a narrow screen (a tablet, a small window) a panel beside the chart would leave too little
  // chart: the line under the chart stays, and the panel slides over the chart's edge when asked
  // for (O, ⟨ Panel), until it is put away again. That is the one time it covers the chart.
  const NARROW_PX = 1000;
  const narrow = () => window.innerWidth < NARROW_PX;
  let sheet = false;
  const hasPanel = () => PANEL_VIEWS.includes(view());
  function visibility() {
    const n = narrow();
    const on = hasPanel() && (n ? sheet : !hidden[view()]);
    const focus = on && ["plan", "env"].includes(view());
    const line = hasPanel() && (n || !!hidden[view()] || focus);
    side.hidden = !on;
    routeWeather.hidden = !["plan", "env"].includes(view());
    side.querySelector(".sp-edit").hidden = view() === "env";
    strip.hidden = !line || (n && on && !focus);
    root.classList.toggle("side-narrow", n);
    root.classList.toggle("side-on", on);
    root.classList.toggle("side-strip", line);
    root.classList.toggle("weather-focus", focus);
    $("side-btn")?.classList.toggle("on", on);
    const sb = $("side-btn");
    if (sb) sb.title = `${root.classList.contains("side-left") ? "Data panel (on the left)" : "Right panel"}: ${on ? "hide" : "show"} the data panel (O)`;
    placeChrome(on, !strip.hidden, focus);
    requestAnimationFrame(() => map.resize());
  }
  function toggle() {
    if (!hasPanel()) return;
    if (narrow()) sheet = !sheet;
    else {
      hidden = { ...hidden, [view()]: !hidden[view()] };
      store(HKEY, hidden);
    }
    visibility();
    build();
  }
  let wasNarrow = narrow();
  window.addEventListener("resize", () => { if (narrow() !== wasNarrow) { wasNarrow = narrow(); sheet = false; visibility(); build(); } });
  $("side-btn")?.addEventListener("click", toggle);
  side.querySelector(".sp-hide").onclick = toggle;
  strip.querySelector(".ss-show").onclick = toggle;

  // MOB, Drop and Anchor, and the alerts, go where the panel is: in it, on its line under the
  // chart, or back over the screen in the views without a chart
  const chrome = ["mob-btn", "drop-btn", "anchor-btn"].map($).filter(Boolean);
  const banner = $("alarm-banner");
  const cursorEl = $("cursor");
  const home = document.body;
  const put = (el, where) => { if (el && el.parentElement !== where) where.append(el); };
  function placeChrome(on, line, focus) {
    // at the top of the panel in every view that has it (Planning and Environment too); on the line under
    // the chart when the panel is hidden; back over the screen in the views without a chart
    // with the OpenBridge frame: always in the top bar
    const where = document.getElementById("ob-actions") ?? (on ? actions : line ? strip.querySelector(".ss-actions") : home);
    for (const b of chrome) put(b, where);
    // the alerts: in the panel, or on the line under the chart (never over the chart)
    put(banner, on && !focus ? alerts : line ? strip.querySelector(".ss-alerts") : home);
    // the cursor's position, depth and bearing and range from the boat: off the chart too
    put(cursorEl, on ? cursorSlot : line ? strip.querySelector(".ss-cursor") : home);
  }

  // ---------- rows ----------
  // the own-ship block uses S-Mode's names (STW for boat speed through the water)
  const OWN_LABEL = { BSP: "STW", DPT: "Depth", POS: "Position" };
  // what a click on a reading does outside edit mode
  const ACTIONS = {
    POS: { tip: "centre the chart on the boat", run: () => centreBoat() },
    NEXT: { tip: "show the next mark", run: (c) => c.nav?.next && centreOn([c.nav.next.lon, c.nav.next.lat]) },
    BTW: { tip: "show the next mark", run: (c) => c.nav?.next && centreOn([c.nav.next.lon, c.nav.next.lat]) },
    DTW: { tip: "show the next mark", run: (c) => c.nav?.next && centreOn([c.nav.next.lon, c.nav.next.lat]) },
    TTG: { tip: "show the next mark", run: (c) => c.nav?.next && centreOn([c.nav.next.lon, c.nav.next.lat]) },
    ETA: { tip: "show the next mark", run: (c) => c.nav?.next && centreOn([c.nav.next.lon, c.nav.next.lat]) },
    XTE: { tip: "show the leg being sailed", run: (c) => c.nav?.next && c.nav.from && centreOn([(c.nav.from[0] + c.nav.next.lon) / 2, (c.nav.from[1] + c.nav.next.lat) / 2]) },
    MOBBRG: { tip: "show the MOB position", run: (c) => c.mob && centreOn([c.mob.lon, c.mob.lat]) },
    MOBRNG: { tip: "show the MOB position", run: (c) => c.mob && centreOn([c.mob.lon, c.mob.lat]) },
    MOBPOS: { tip: "show the MOB position", run: (c) => c.mob && centreOn([c.mob.lon, c.mob.lat]) },
    CPOS: { tip: "show the cursor's place", run: (c) => c.cursor && centreOn(c.cursor) },
  };
  let trend = false; // set while a trend section's rows are made
  function rowHtml(k, cls = "") {
    const it = ITEMS[k];
    if (!it) return "";
    const wide = it.wide ? " wide" : "";
    const tint = it.tint ? ` wt-${it.tint}` : "";
    const hist = historyKey(k) ? " has-hist" : "";
    const tip = `${it.label}${it.unit ? ` (${it.unit})` : ""}${hist ? " · right-click for its history" : ""}`;
    const act = ACTIONS[k] ? " act" : "";
    const tipAct = ACTIONS[k] ? ` · click: ${ACTIONS[k].tip}` : "";
    const spark = trend && historyKey(k) ? `<svg class="sp-spark" viewBox="0 0 100 10" preserveAspectRatio="none" data-h="${historyKey(k)}"><polyline/></svg>` : "";
    return `<div class="sp-row${wide}${tint}${hist}${act}${spark ? " trend" : ""}${cls}" data-k="${k}" title="${esc(tip + tipAct)}"><span class="l">${esc((cls.includes("own") && OWN_LABEL[k]) || it.short || it.label)}</span><span class="sd"></span><span class="v">—</span>${spark}</div>`;
  }
  function paintRow(el, ctx) {
    const it = ITEMS[el.dataset.k];
    if (!it) return;
    const r = readingState(it, ctx);
    let html = r.html + (r.unit ? `<span class="u">${r.unit}</span>` : "");
    if (el._line ??= !!el.closest("#side-strip")) html = html.replace(/<br\s*\/?>/g, " ");
    const v = el.querySelector(".v");
    if (el._html !== html) { v.innerHTML = html; el._html = html; }
    const sd = r.side === "port" ? "P" : r.side === "stbd" ? "S" : "";
    const s = el.querySelector(".sd");
    if (s.textContent !== sd) { s.textContent = sd; s.className = `sd ${r.side}`; }
    el.classList.toggle("stale", r.stale);
    el.classList.toggle("warn", r.state === "warn");
    el.classList.toggle("alarm", r.state === "alarm");
  }

  // ---------- build ----------
  function build() {
    side.classList.toggle("editing", editing);
    $("sp-edit-btn")?.classList.toggle("on", editing);
    ownEl.innerHTML = OWN_SHIP.map((k) => rowHtml(k, " own")).join("");
    const L = layout();
    planHome.before(planCard);
    weatherParking.append(weatherSection, weatherTime);
    pinnedTime.innerHTML = "";
    pinnedPoint.innerHTML = "";
    body.innerHTML = "";
    const environment = view() === "env";
    (environment ? envData : filesHome).append(filesSection);
    envTabs.hidden = !environment;
    setEnvironmentTab(envTab);
    // the planning controls go in the panel in Planning, if the panel is shown; otherwise they stay in the Route drawer
    if (view() === "plan" && !side.hidden) {
      planCard.open = true;
      body.append(planCard);
    }
    planAway.hidden = !side.contains(planCard);
    if (L.some((s) => s.kind === "weatherTime")) {
      const clock = weatherTime.querySelector(".sp-wx-clock");
      $("side-wx-gribs").append($("wxp-run"));
      clock.insertBefore(routeWeather, $("side-wx-time-status"));
    } else {
      $("wxp-run-fallback").append($("wxp-run"));
      body.append(routeWeather);
    }
    L.forEach((s, i) => {
      const sec = document.createElement("section");
      sec.className = `sp-sec${s.collapsed && !editing ? " collapsed" : ""}${s.kind === "readings" && s.cols === 2 ? " c2" : ""}`;
      sec.dataset.i = i;
      if (Object.hasOwn(WEATHER_SECTIONS, s.kind)) {
        if (editing) sec.innerHTML = `<h4><span class="sp-title">${esc(WEATHER_SECTIONS[s.kind])}</span><span class="sp-tools"><button data-act="up" title="Move up">▲</button><button data-act="down" title="Move down">▼</button><button data-act="del" title="Remove this section">✕</button></span></h4>`;
        sec.append(weatherElements[s.kind]);
        if (s.kind === "weatherTime" && (view() === "plan" || root.classList.contains("weather-focus")) && !editing) pinnedTime.append(sec);
        else body.append(sec);
        return;
      }
      const tools = editing
        ? `<span class="sp-tools"><button data-act="up" title="Move up">▲</button><button data-act="down" title="Move down">▼</button>${s.kind === "readings"
          ? `<button data-act="cols" title="One or two columns">${s.cols === 2 ? "2 col" : "1 col"}</button><button data-act="trend" class="${s.trend ? "on" : ""}" title="Trend lines: the last 3 hours under each reading that has a history">∿</button>`
          : s.kind === "graphic" ? `<button data-act="taller" title="Taller">＋</button><button data-act="shorter" title="Shorter">−</button>`
          : `<button data-act="more" title="Show more">＋</button><button data-act="fewer" title="Show fewer">−</button>`}<button data-act="del" title="Remove this section">✕</button></span>`
        : "";
      const title = editing ? `<input class="sp-title" value="${esc(s.title)}" placeholder="Section name" maxlength="30">` : `<span class="sp-title">${esc(s.title || " ")}</span>`;
      sec.innerHTML = `<h4 title="${editing ? "" : "Click to fold or open"}">${editing ? "" : `<span class="chev">${s.collapsed ? "▸" : "▾"}</span>`}${title}${tools}</h4>`;
      if (s.kind === "readings") {
        const g = document.createElement("div");
        g.className = "sp-rows";
        trend = !!s.trend;
        g.innerHTML = s.items.map((k) => rowHtml(k)).join("") + (editing ? `<button class="sp-addrow" data-act="add">+ Add a reading</button>` : "");
        trend = false;
        sec.append(g);
      } else if (s.kind === "marks" || s.kind === "targets") {
        const g = document.createElement("div");
        g.className = `sp-list sp-${s.kind}`;
        g.dataset.kind = s.kind;
        g.dataset.n = s.n;
        sec.append(g);
      } else {
        const g = document.createElement("div");
        g.className = "sp-g";
        g.dataset.k = s.item;
        g.dataset.h = s.h;
        sec.append(g);
      }
      body.append(sec);
    });
    if (editing) {
      const add = document.createElement("div");
      add.className = "sp-addsec";
      add.innerHTML = `<button data-act="addsec">+ Add a section</button><button data-act="reset" title="Back to the starting layout for ${viewName()} · ${useName()}">Reset</button><button data-act="done" class="on">Done</button>`;
      body.append(add);
    }
    const focused = root.classList.contains("weather-focus");
    const stripKeys = focused ? OWN_SHIP : ["HDG", "COG", "SOG", "DPT", "POS", "TIME"];
    strip.querySelector(".ss-rows").innerHTML = stripKeys.map((k) => rowHtml(k, focused ? " own" : "")).join("");
    geometry();
    listsAt = 0; sparksAt = 0;
    paint();
  }
  const viewName = () => VIEWS.find((v) => v.id === view())?.name ?? "";
  const useName = () => ({ cruise: "Cruising", fish: "Fishing", race: "Racing" })[useNow()];

  function setEnvironmentTab(tab) {
    envTab = tab;
    const environment = view() === "env";
    envData.hidden = !environment || tab !== "data";
    body.hidden = environment && tab === "data";
    pinnedTime.hidden = environment && tab === "data";
    pinnedPoint.hidden = environment && tab === "data";
    for (const button of envTabs.querySelectorAll("[data-env-tab]")) button.setAttribute("aria-pressed", String(button.dataset.envTab === tab));
    window.dispatchEvent(new Event("racenav-layout"));
  }
  envTabs.addEventListener("click", (e) => {
    const button = e.target.closest("[data-env-tab]");
    if (button) setEnvironmentTab(button.dataset.envTab);
  });

  // ---------- paint (four times a second) ----------
  function status() {
    const live = $("sk-live")?.classList.contains("on");
    const replay = $("replay-bar") && !$("replay-bar").hidden;
    const brg = ((map.getBearing() % 360) + 360) % 360;
    const follow = $("follow")?.classList.contains("on");
    const c = map.getCenter();
    let scale = "";
    try { const n = displayScale(c.lat, map.getZoom(), s52PxPerMm()); scale = `1:${Math.round(n).toLocaleString("en-NZ")}`; } catch { /* ignore */ }
    const parts = [
      ...(inst.ctx().nav?.goto ? [`<span class="st-goto" title="Navigating to a Go to point (right-click the chart › Cancel go to returns to the route)">GO TO</span>`] : []),
      replay ? `<span class="st-replay">REPLAY</span>${live ? "" : ` · <span class="st-paused">PLOTTING PAUSED</span>`}` : live ? `<span class="st-live">LIVE</span>` : `<span class="st-paused">PAUSED</span>`,
      ...($("ais-show") && !$("ais-show").checked ? [`<span class="st-paused" title="AIS targets are hidden on the chart in this view; reception and alarms remain active">AIS HIDDEN</span>`] : []),
      `<span title="Orientation: north up, or the chart turned">${Math.round(brg) === 0 || Math.round(brg) === 360 ? "N UP" : `ROT ${String(Math.round(brg)).padStart(3, "0")}°`}</span>`,
      `<span title="${follow ? "Relative motion: the chart moves under the boat (Follow on)" : "True motion: the boat moves across the chart (Follow off)"}">${follow ? "RM" : "TM"}</span>`,
      `<span title="Display scale on this screen (from the screen calibration)">${scale}</span>`,
    ];
    const html = parts.join(" · ");
    if (statusEl._html !== html) { statusEl.innerHTML = html; statusEl._html = html; }
  }
  // The one-line strip under the chart: when its readings are wider than the room (Environment's wide
  // panel, a narrow window) the type shrinks a little, down to 60 %, rather than cutting the position off.
  let fitAt = 0, fitK = 1;
  function fitStrip() {
    const rows = strip.querySelector(".ss-rows");
    if (!rows || !rows.clientWidth) return;
    const natural = rows.scrollWidth / fitK;
    const k = Math.max(0.6, Math.min(1, (rows.clientWidth / natural) * 0.98));
    if (Math.abs(k - fitK) > 0.015) { fitK = k; strip.style.setProperty("--ss-k", k.toFixed(3)); }
  }
  new ResizeObserver(() => fitStrip()).observe(strip);
  function paint() {
    if (!strip.hidden && Date.now() - fitAt > 1000) { fitAt = Date.now(); fitStrip(); }
    const ctx = inst.ctx();
    if (!side.hidden) {
      status();
      for (const el of side.querySelectorAll(".sp-row")) paintRow(el, ctx);
      for (const g of side.querySelectorAll(".sp-sec:not(.collapsed) .sp-g, .sp-mob .sp-g")) {
        const it = ITEMS[g.dataset.k];
        if (it && g.clientHeight) paintGraphic(g, it, ctx);
      }
      mob(ctx);
      if (Date.now() - listsAt > 1000) { listsAt = Date.now(); for (const g of side.querySelectorAll(".sp-sec:not(.collapsed) .sp-list")) paintList(g, ctx); }
      if (Date.now() - sparksAt > 30_000) { sparksAt = Date.now(); paintSparks(); }
    }
    if (!strip.hidden) for (const el of strip.querySelectorAll(".sp-row")) paintRow(el, ctx);
  }
  let listsAt = 0, sparksAt = 0;
  setInterval(paint, 250);

  // ---------- lists: the marks ahead, and the targets that matter ----------
  const hhmm = (t) => new Date(t).toLocaleTimeString("en-NZ", { hour: "2-digit", minute: "2-digit", hour12: false });
  const nm = (x) => (x == null ? "—" : x < 10 ? x.toFixed(2) : x.toFixed(1));
  function paintList(g, ctx) {
    const n = Number(g.dataset.n) || 5;
    let html = "";
    if (g.dataset.kind === "marks") {
      const nav = ctx.nav;
      const pos = ctx.v?.lat != null && ctx.v?.lon != null ? [ctx.v.lon, ctx.v.lat] : null;
      if (!nav?.next) html = `<div class="sp-empty">No route being sailed (Route › Navigate)</div>`;
      else {
        const marks = [nav.next, ...(nav.ahead ?? [])].slice(0, n);
        const sog = ctx.v?.sog > 0.3 ? ctx.v.sog : null;
        let d = ctx.wp?.dtw ?? (pos ? distNm(pos, [nav.next.lon, nav.next.lat]) : null);
        marks.forEach((m, i) => {
          if (i > 0 && d != null) d += distNm([marks[i - 1].lon, marks[i - 1].lat], [m.lon, m.lat]);
          const eta = d != null && sog ? hhmm(Date.now() + (d / sog) * 3600e3) : "—";
          html += `<div class="sp-li${i === 0 ? " next" : ""}" data-lon="${m.lon}" data-lat="${m.lat}" title="Click to show ${esc(m.name)} on the chart. ETA at the present speed over ground"><span class="no">${m.n ?? ""}</span><span class="nm">${esc(m.name)}</span><span class="d">${nm(d)}<small>NM</small></span><span class="t">${eta}</span></div>`;
        });
        const more = (nav.ahead?.length ?? 0) + 1 - marks.length;
        if (more > 0) html += `<div class="sp-empty">and ${more} more</div>`;
      }
    } else {
      const ais = getAis();
      const list = ais?.closest?.(n) ?? [];
      if (!list.length) html = `<div class="sp-empty">No AIS targets</div>`;
      for (const t of list) {
        // TCPA in minutes up to 10 h, then hours, and past 99 h just ">99 h" (a slow closing target far off)
        const tc = t.tcpa < 600 ? `${Math.round(t.tcpa)}<small>min</small>` : t.tcpa < 99 * 60 ? `${Math.round(t.tcpa / 60)}<small>h</small>` : `>99<small>h</small>`;
        const cpa = t.cpa != null ? `${nm(t.cpa)}<small>NM</small> ${tc}` : `<small>not closing</small>`;
        html += `<div class="sp-li tgt${t.danger ? " danger" : ""}${t.lost ? " lost" : ""}" data-id="${esc(t.id)}" title="Click to show ${esc(t.name)} and open its card. CPA and TCPA, then range"><span class="sw" style="background:${esc(t.col)}"></span><span class="nm">${esc(t.name)}</span><span class="d">${cpa}</span><span class="t">${nm(t.range)}<small>NM</small></span></div>`;
      }
    }
    if (g._html !== html) { g.innerHTML = html; g._html = html; }
    // an empty list folds into its heading ("Marks ahead  none"), not a line of its own
    const none = g.dataset.kind === "marks" ? (!ctx.nav?.next ? "no route (Route › Navigate)" : "") : (!(getAis()?.closest?.(n) ?? []).length ? "none" : "");
    const h = g.closest(".sp-sec")?.querySelector("h4");
    if (h && (h.dataset.none ?? "") !== none) { if (none) h.dataset.none = none; else delete h.dataset.none; }
    g.hidden = !!none;
  }

  // ---------- trend lines (the last three hours from the history recorder) ----------
  function paintSparks() {
    const now = Date.now();
    for (const sv of side.querySelectorAll(".sp-sec:not(.collapsed) .sp-spark")) {
      const k = sv.dataset.h;
      const pts = recorder.series(k, now - 3 * 3600e3, now);
      const pl = sv.querySelector("polyline");
      // no history yet: no empty line taking up room
      sv.style.display = pts.length < 2 ? "none" : "";
      if (pts.length < 2) { pl.setAttribute("points", ""); continue; }
      let ys = pts.map((p) => p.m);
      if (HIST[k]?.dir) ys = unwrap(ys);
      let lo = Math.min(...ys), hi = Math.max(...ys);
      const minSpan = { PRES: 2, TWS: 4, AWS: 4, TWD: 20, AWA: 20 }[k] ?? 1;
      if (hi - lo < minSpan) { const c = (hi + lo) / 2; lo = c - minSpan / 2; hi = c + minSpan / 2; }
      const t0 = now - 3 * 3600e3;
      pl.setAttribute("points", pts.map((p, i) => `${(((p.t - t0) / (3 * 3600e3)) * 100).toFixed(1)},${(9.5 - ((ys[i] - lo) / (hi - lo)) * 9).toFixed(2)}`).join(" "));
    }
  }

  // a MOB: its own section under the own-ship block, whatever the view or layout
  let mobShown = false;
  function mob(ctx) {
    const on = !!ctx.mob;
    if (on !== mobShown) {
      mobShown = on;
      mobEl.hidden = !on;
      // the highway steers back to the MOB (course to steer, distance to it); under it the time
      // since and where it happened (for the log and the distress call)
      mobEl.innerHTML = on ? `<h4>MAN OVERBOARD</h4><div class="sp-g" data-k="HIGHWAY" data-h="62"></div><div class="sp-rows">${["MOBAGO", "MOBPOS"].map((k) => rowHtml(k)).join("")}</div>` : "";
      geometry();
    }
  }
  window.addEventListener("racenav-mob-panel", () => {
    // a new MOB: the panel comes up if it was hidden
    if (hasPanel() && hidden[view()]) toggle();
  });

  // ---------- folding and editing ----------
  body.addEventListener("click", (e) => {
    const sec = e.target.closest(".sp-sec");
    const L = layout();
    const b = e.target.closest("button");
    if (b?.dataset.act === "addsec") return addSection(b.getBoundingClientRect());
    if (b?.dataset.act === "reset") { data.layouts[key()] = preset(view(), useNow()); save(); build(); return; }
    if (b?.dataset.act === "done") { setEditing(false); return; }
    if (!sec) return;
    const i = Number(sec.dataset.i), s = L[i];
    if (Object.hasOwn(WEATHER_SECTIONS, s.kind) && e.target.closest("h4")?.parentElement !== sec) return;
    if (!editing) {
      if (e.target.closest("h4")) { s.collapsed = !s.collapsed; save(); build(); return; }
      const li = e.target.closest(".sp-li");
      if (li?.dataset.id) getAis()?.select?.(li.dataset.id);
      else if (li?.dataset.lon) centreOn([Number(li.dataset.lon), Number(li.dataset.lat)]);
      const row = e.target.closest(".sp-row");
      if (row && ACTIONS[row.dataset.k]) ACTIONS[row.dataset.k].run(inst.ctx());
      return;
    }
    if (b) {
      const a = b.dataset.act;
      if (a === "up" && i > 0) [L[i - 1], L[i]] = [L[i], L[i - 1]];
      else if (a === "down" && i < L.length - 1) [L[i + 1], L[i]] = [L[i], L[i + 1]];
      else if (a === "cols") s.cols = s.cols === 2 ? 1 : 2;
      else if (a === "trend") s.trend = !s.trend;
      else if (a === "more") s.n = Math.min(12, (s.n || 5) + 1);
      else if (a === "fewer") s.n = Math.max(1, (s.n || 5) - 1);
      else if (a === "taller") s.h = Math.min(120, s.h + 5);
      else if (a === "shorter") s.h = Math.max(20, s.h - 5);
      else if (a === "del") L.splice(i, 1);
      else if (a === "add") return readingMenu(b.getBoundingClientRect(), (k) => { s.items.push(k); save(); build(); });
      else return;
      save(); build();
      return;
    }
    const row = e.target.closest(".sp-row");
    if (row) {
      const j = [...row.parentElement.querySelectorAll(".sp-row")].indexOf(row);
      rowMenu(row.getBoundingClientRect(), s, j);
    }
  });
  ownEl.addEventListener("click", (e) => { const row = e.target.closest(".sp-row"); if (row && ACTIONS[row.dataset.k]) ACTIONS[row.dataset.k].run(inst.ctx()); });
  mobEl.addEventListener("click", (e) => { const row = e.target.closest(".sp-row"); if (row && ACTIONS[row.dataset.k]) ACTIONS[row.dataset.k].run(inst.ctx()); });
  body.addEventListener("change", (e) => {
    const t = e.target.closest("input.sp-title");
    if (!t) return;
    const s = layout()[Number(t.closest(".sp-sec").dataset.i)];
    s.title = t.value.trim();
    save();
  });
  function setEditing(on) { editing = on; build(); }
  side.querySelector(".sp-edit").onclick = () => setEditing(!editing);
  $("sp-edit-btn")?.addEventListener("click", () => {
    if (!hasPanel()) views.set("nav");
    if (hidden[view()]) toggle();
    setEditing(!editing);
  });
  $("sp-reset-btn")?.addEventListener("click", () => {
    if (!hasPanel()) return;
    data.layouts[key()] = preset(view(), useNow());
    save(); build();
  });

  // menus: the shared context-menu box (as the Instruments screen's)
  function showMenu(html, rect, onClick) {
    menu.innerHTML = html;
    menu.hidden = false;
    const mw = 300, mh = Math.min(menu.scrollHeight, window.innerHeight * 0.8);
    let x = prefs.side === "left" ? rect.right + 6 : rect.left - mw - 6;
    if (x < 4 || x + mw > window.innerWidth) x = Math.max(4, Math.min(window.innerWidth - mw - 4, rect.left));
    let y = rect.top;
    if (y + mh > window.innerHeight) y = window.innerHeight - mh - 8;
    menu.style.left = `${x}px`;
    menu.style.top = `${Math.max(4, y)}px`;
    menu.onclick = (ev) => {
      const b = ev.target.closest("button");
      if (!b) return;
      menu.hidden = true;
      onClick(b);
    };
  }
  function readingMenu(rect, pick, cur = null) {
    const groups = {};
    for (const [k, it] of Object.entries(ITEMS)) if (!it.graphic && !it.mobOnly) (groups[it.group] ??= []).push([k, it]);
    let html = "";
    for (const [g, list] of Object.entries(groups)) {
      html += `<div class="grp">${esc(g)}</div><div class="items">`;
      for (const [k, it] of list) html += `<button data-set="${k}" class="${k === cur ? "cur" : ""}">${esc(it.label)}${it.unit ? ` <small>${esc(it.unit)}</small>` : ""}</button>`;
      html += "</div>";
    }
    showMenu(html, rect, (b) => { if (b.dataset.set) pick(b.dataset.set); });
  }
  function rowMenu(rect, s, j) {
    const k = s.items[j];
    const html = `<div class="grp">${esc(ITEMS[k]?.label ?? k)}</div><button data-a="change">Change the reading…</button><button data-a="up">Move up</button><button data-a="down">Move down</button><button data-a="del">Remove</button>`;
    showMenu(html, rect, (b) => {
      const a = b.dataset.a;
      if (a === "change") return readingMenu(rect, (n) => { s.items[j] = n; save(); build(); }, k);
      if (a === "up" && j > 0) [s.items[j - 1], s.items[j]] = [s.items[j], s.items[j - 1]];
      if (a === "down" && j < s.items.length - 1) [s.items[j + 1], s.items[j]] = [s.items[j], s.items[j + 1]];
      if (a === "del") s.items.splice(j, 1);
      save(); build();
    });
  }
  function addSection(rect) {
    const graphics = [["WDIAL", "Wind dial", 48], ["COMPASS", "Compass", 48], ["HIGHWAY", "Highway", 70], ["SOUNDER", "Sounder history", 40]];
    const current = layout();
    const weatherChoices = Object.entries(WEATHER_SECTIONS).filter(([kind]) => !current.some((s) => s.kind === kind)).map(([kind, label]) => `<button data-kind="${kind}">${label}</button>`).join("");
    let html = `<div class="grp">Add a section</div><button data-kind="readings">Readings</button><button data-kind="marks">Marks ahead (route)</button><button data-kind="targets">AIS targets (closest by CPA)</button>${weatherChoices}<div class="grp">Graphics</div><div class="items">`;
    for (const [k, n] of graphics) if (ITEMS[k]) html += `<button data-g="${k}">${n}</button>`;
    html += `</div><div class="grp">History plot</div><div class="items">`;
    for (const [k, ch] of Object.entries(HIST)) if (ITEMS[`H_${k}`]) html += `<button data-g="H_${k}">${esc(ch.label)}</button>`;
    html += "</div>";
    showMenu(html, rect, (b) => {
      const L = layout();
      const id = `s${Date.now().toString(36)}`;
      if (b.dataset.kind === "readings") L.push({ id, kind: "readings", title: "", cols: 2, items: [] });
      else if (b.dataset.kind === "marks") L.push({ id, kind: "marks", title: "Marks ahead", n: 5 });
      else if (b.dataset.kind === "targets") L.push({ id, kind: "targets", title: "Targets", n: 4 });
      else if (Object.hasOwn(WEATHER_SECTIONS, b.dataset.kind) && !L.some((s) => s.kind === b.dataset.kind)) L.push(makeWeatherSection(b.dataset.kind));
      else if (b.dataset.g) {
        const g = graphics.find((x) => x[0] === b.dataset.g);
        L.push({ id, kind: "graphic", title: g ? g[1] : ITEMS[b.dataset.g].label, item: b.dataset.g, h: g ? g[2] : 35 });
      }
      save(); build();
    });
  }
  document.addEventListener("pointerdown", (e) => { if (!menu.hidden && !menu.contains(e.target) && !e.target.closest("#side")) menu.hidden = true; });

  // ---------- width: drag the inner edge, snapping to narrow, normal or wide ----------
  const grip = side.querySelector(".sp-grip");
  grip.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const r = side.getBoundingClientRect();
    const move = (ev) => {
      const w = prefs.side === "left" ? ev.clientX - r.left : r.right - ev.clientX;
      root.style.setProperty("--side-w", `${Math.max(150, Math.min(window.innerWidth * 0.45, w))}px`);
    };
    const up = (ev) => {
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", up);
      const w = prefs.side === "left" ? ev.clientX - r.left : r.right - ev.clientX;
      setPref("width", snapWidth(w, s52PxPerMm(), prefs.text));
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
  });

  // ---------- chart display: Base, Standard, All (from chartfeatures.js) ----------
  // The display mode is part of the IMO display base (always shown); Standard is the S-Mode one
  // press back to the standard display. A standard category the mariner has changed is "changed"
  // (IMO ECDIS PS 10.5: permanently indicated). Base alone isn't enough to navigate by (S-52).
  // a list beside the view icons; "Standard *" when the standard categories have been changed
  const disp = side.querySelector(".sp-disp"), dispNote = side.querySelector(".sp-disp-note"), ssDisp = strip.querySelector(".ss-disp");
  disp.addEventListener("change", () => window.dispatchEvent(new CustomEvent("racenav-set-dispcat", { detail: { cat: disp.value } })));
  // a list takes two clicks: off the standard display, one button takes it back in one (IMO S-Mode)
  const stdBtn = side.querySelector(".sp-std");
  stdBtn.onclick = () => window.dispatchEvent(new CustomEvent("racenav-set-dispcat", { detail: { cat: "standard" } }));
  window.addEventListener("racenav-dispcat", (e) => {
    const d = e.detail || {};
    disp.hidden = !d.enabled;
    disp.value = d.cat ?? "standard";
    const std = disp.querySelector('option[value="standard"]');
    if (std) std.textContent = d.cat === "standard" && !d.standard ? "Standard *" : "Standard";
    const note = [d.cat === "standard" && !d.standard ? "Standard changed" : d.cat === "base" ? "Base only: not enough to navigate by" : "", d.dated].filter(Boolean).join(" · ");
    dispNote.textContent = note;
    dispNote.classList.toggle("warn", d.cat === "base" || !!d.dated);
    disp.classList.toggle("warn", d.cat === "base" || !!d.dated);
    stdBtn.hidden = !d.enabled || (d.cat === "standard" && !!d.standard);
    ssDisp.hidden = !d.enabled;
    ssDisp.textContent = { base: "BASE", standard: d.standard ? "STD" : "STD*", all: "ALL" }[d.cat] ?? "";
    ssDisp.classList.toggle("warn", d.cat === "base" || !!d.dated);
  });
  // the view icons
  for (const b of side.querySelectorAll(".sp-views [data-view-select]")) {
    const v = VIEWS.find((x) => x.id === b.dataset.viewSelect);
    if (v) b.querySelector("svg").innerHTML = v.icon;
  }

  // ---------- follow the view and the Use ----------
  let lastView = view();
  views.onChange((id) => {
    if (id !== lastView && id === "env" && narrow()) sheet = true;
    lastView = id;
    editing = false; visibility(); build();
  });
  window.addEventListener("racenav-use", () => build());

  geometry();
  visibility();
  build();
  return { toggle, shown: () => !side.hidden, openHistory: (k) => openHistoryCard(k),
    /** The route's planning controls are in the panel: open them there (true), or false when they are in the Route drawer. */
    revealPlan,
    openEnvironmentData() {
      if (view() !== "env") views.set("env");
      if (side.hidden) toggle();
      setEnvironmentTab("data");
    },
    openWeather() {
      if (view() !== "env") views.set("env");
      setEnvironmentTab("explore");
      let changed = false;
      if (!layout().some((s) => s.kind === "weather")) {
        layout().unshift(makeWeatherSection("weather"));
        changed = true;
      }
      if (!layout().some((s) => s.kind === "weatherTime")) {
        layout().unshift(makeWeatherSection("weatherTime"));
        changed = true;
      }
      if (changed) save();
      if (side.hidden) toggle(); else build();
      weatherSection.open = true;
      weatherSection.scrollIntoView({ block: "start" });
    } };
}
