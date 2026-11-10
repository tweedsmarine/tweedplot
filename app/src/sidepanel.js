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
import { ITEMS, readingState, paintGraphic, historyKey } from "./instruments.js";
import { preset, clean, fromFloating, OWN_SHIP, PANEL_VIEWS, WIDTHS_MM, snapWidth } from "./sidepresets.js";
import { CHANNELS as HIST, openHistoryCard } from "./history.js";
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

export function setupSidePanel({ inst, views, use, map }) {
  const side = $("side"), strip = $("side-strip"), root = document.documentElement;
  const body = side.querySelector(".sp-body"), ownEl = side.querySelector(".sp-own"), mobEl = side.querySelector(".sp-mob");
  const statusEl = side.querySelector(".sp-status"), actions = side.querySelector(".sp-actions"), alerts = side.querySelector(".sp-alerts");
  const menu = $("tile-menu");

  const prefs = { side: "right", text: 1, width: "normal", ...load(PKEY, {}) };
  if (![1, 1.25, 1.5].includes(Number(prefs.text))) prefs.text = 1;
  prefs.text = Number(prefs.text);
  if (!WIDTHS_MM[prefs.width]) prefs.width = "normal";
  let hidden = load(HKEY, {}) || {}; // { nav: true } = hidden in Navigation
  let data = load(KEY, null);
  if (!data || data.v !== 1 || typeof data.layouts !== "object") data = { v: 1, layouts: {} };
  let editing = false;

  // ---------- the old floating panels, once ----------
  if (!data.migrated) {
    data.migrated = true;
    const moved = fromFloating(inst.floatingPanels(), ITEMS);
    if (moved.nav.length || moved.plan.length) {
      data.movedPanels = moved; // added to each Use's layout when it is first made
    }
    store(KEY, data);
  }

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
    const t = $("sp-text");
    if (t) t.value = String(prefs.text);
    for (const g of side.querySelectorAll(".sp-g")) g.style.height = `${Math.round(Number(g.dataset.h) * pxmm * prefs.text)}px`;
    requestAnimationFrame(() => map.resize());
  }
  window.addEventListener("racenav-calibration", geometry);
  const setPref = (k, v) => { prefs[k] = v; store(PKEY, prefs); geometry(); };
  for (const id of ["sp-side", "set-side"]) $(id)?.addEventListener("change", (e) => setPref("side", e.target.value));
  $("sp-text")?.addEventListener("change", (e) => setPref("text", Number(e.target.value)));

  // ---------- shown, hidden (a line under the chart) or not at all (no chart) ----------
  const hasPanel = () => PANEL_VIEWS.includes(view());
  function visibility() {
    const on = hasPanel() && !hidden[view()];
    const line = hasPanel() && !!hidden[view()];
    side.hidden = !on;
    strip.hidden = !line;
    root.classList.toggle("side-on", on);
    root.classList.toggle("side-strip", line);
    $("side-btn")?.classList.toggle("on", on);
    placeChrome(on, line);
    requestAnimationFrame(() => map.resize());
  }
  function toggle() {
    if (!hasPanel()) return;
    hidden = { ...hidden, [view()]: !hidden[view()] };
    store(HKEY, hidden);
    visibility();
  }
  $("side-btn")?.addEventListener("click", toggle);
  side.querySelector(".sp-hide").onclick = toggle;
  strip.querySelector(".ss-show").onclick = toggle;

  // MOB, Drop and Anchor, and the alerts, go where the panel is: in it, on its line under the
  // chart, or back over the screen in the views without a chart
  const chrome = ["mob-btn", "drop-btn", "anchor-btn"].map($).filter(Boolean);
  const banner = $("alarm-banner");
  const home = document.body;
  function placeChrome(on, line) {
    const where = on ? actions : line ? strip.querySelector(".ss-actions") : null;
    for (const b of chrome) {
      if (where) { if (b.parentElement !== where) where.append(b); } else if (b.parentElement !== home) home.append(b);
    }
    if (on) { if (banner.parentElement !== alerts) alerts.append(banner); } else if (banner.parentElement !== home) home.append(banner);
  }

  // ---------- rows ----------
  // the own-ship block uses S-Mode's names (STW for boat speed through the water)
  const OWN_LABEL = { BSP: "STW", DPT: "Depth", POS: "Position" };
  function rowHtml(k, cls = "") {
    const it = ITEMS[k];
    if (!it) return "";
    const wide = it.wide ? " wide" : "";
    const tint = it.tint ? ` wt-${it.tint}` : "";
    const hist = historyKey(k) ? " has-hist" : "";
    const tip = `${it.label}${it.unit ? ` (${it.unit})` : ""}${hist ? " · right-click for its history" : ""}`;
    return `<div class="sp-row${wide}${tint}${hist}${cls}" data-k="${k}" title="${esc(tip)}"><span class="l">${esc((cls.includes("own") && OWN_LABEL[k]) || it.short || it.label)}</span><span class="sd"></span><span class="v">—</span></div>`;
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
    body.innerHTML = "";
    L.forEach((s, i) => {
      const sec = document.createElement("section");
      sec.className = `sp-sec${s.collapsed && !editing ? " collapsed" : ""}${s.kind === "readings" && s.cols === 2 ? " c2" : ""}`;
      sec.dataset.i = i;
      const tools = editing
        ? `<span class="sp-tools"><button data-act="up" title="Move up">▲</button><button data-act="down" title="Move down">▼</button>${s.kind === "readings" ? `<button data-act="cols" title="One or two columns">${s.cols === 2 ? "2 col" : "1 col"}</button>` : `<button data-act="taller" title="Taller">＋</button><button data-act="shorter" title="Shorter">−</button>`}<button data-act="del" title="Remove this section">✕</button></span>`
        : "";
      const title = editing ? `<input class="sp-title" value="${esc(s.title)}" placeholder="Section name" maxlength="30">` : `<span class="sp-title">${esc(s.title || " ")}</span>`;
      sec.innerHTML = `<h4 title="${editing ? "" : "Click to fold or open"}">${editing ? "" : `<span class="chev">${s.collapsed ? "▸" : "▾"}</span>`}${title}${tools}</h4>`;
      if (s.kind === "readings") {
        const g = document.createElement("div");
        g.className = "sp-rows";
        g.innerHTML = s.items.map((k) => rowHtml(k)).join("") + (editing ? `<button class="sp-addrow" data-act="add">+ Add a reading</button>` : "");
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
    strip.querySelector(".ss-rows").innerHTML = ["HDG", "COG", "SOG", "DPT", "POS", "TIME"].map((k) => rowHtml(k)).join("");
    geometry();
    paint();
  }
  const viewName = () => VIEWS.find((v) => v.id === view())?.name ?? "";
  const useName = () => ({ cruise: "Cruising", fish: "Fishing", race: "Racing" })[useNow()];

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
      `<b>${esc(viewName())}</b>`,
      replay ? `<span class="st-replay">REPLAY</span>` : live ? `<span class="st-live">LIVE</span>` : `<span class="st-paused">PAUSED</span>`,
      `<span title="Orientation: north up, or the chart turned">${Math.round(brg) === 0 || Math.round(brg) === 360 ? "N UP" : `ROT ${String(Math.round(brg)).padStart(3, "0")}°`}</span>`,
      `<span title="${follow ? "Relative motion: the chart moves under the boat (Follow on)" : "True motion: the boat moves across the chart (Follow off)"}">${follow ? "RM" : "TM"}</span>`,
      `<span title="Display scale on this screen (from the screen calibration)">${scale}</span>`,
    ];
    const html = parts.join(" · ");
    if (statusEl._html !== html) { statusEl.innerHTML = html; statusEl._html = html; }
  }
  function paint() {
    const ctx = inst.ctx();
    if (!side.hidden) {
      status();
      for (const el of side.querySelectorAll(".sp-row")) paintRow(el, ctx);
      for (const g of side.querySelectorAll(".sp-sec:not(.collapsed) .sp-g, .sp-mob .sp-g")) {
        const it = ITEMS[g.dataset.k];
        if (it && g.clientHeight) paintGraphic(g, it, ctx);
      }
      mob(ctx);
    }
    if (!strip.hidden) for (const el of strip.querySelectorAll(".sp-row")) paintRow(el, ctx);
  }
  setInterval(paint, 250);

  // a MOB: its own section under the own-ship block, whatever the view or layout
  let mobShown = false;
  function mob(ctx) {
    const on = !!ctx.mob;
    if (on !== mobShown) {
      mobShown = on;
      mobEl.hidden = !on;
      mobEl.innerHTML = on ? `<h4>MAN OVERBOARD</h4><div class="sp-rows">${["MOBBRG", "MOBRNG", "MOBAGO", "MOBPOS"].map((k) => rowHtml(k)).join("")}</div>` : "";
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
    if (!editing) {
      if (e.target.closest("h4")) { s.collapsed = !s.collapsed; save(); build(); }
      return;
    }
    if (b) {
      const a = b.dataset.act;
      if (a === "up" && i > 0) [L[i - 1], L[i]] = [L[i], L[i - 1]];
      else if (a === "down" && i < L.length - 1) [L[i + 1], L[i]] = [L[i], L[i + 1]];
      else if (a === "cols") s.cols = s.cols === 2 ? 1 : 2;
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
    let html = `<div class="grp">Add a section</div><button data-kind="readings">Readings</button><div class="grp">Graphics</div><div class="items">`;
    for (const [k, n] of graphics) if (ITEMS[k]) html += `<button data-g="${k}">${n}</button>`;
    html += `</div><div class="grp">History plot</div><div class="items">`;
    for (const [k, ch] of Object.entries(HIST)) if (ITEMS[`H_${k}`]) html += `<button data-g="H_${k}">${esc(ch.label)}</button>`;
    html += "</div>";
    showMenu(html, rect, (b) => {
      const L = layout();
      const id = `s${Date.now().toString(36)}`;
      if (b.dataset.kind) L.push({ id, kind: "readings", title: "", cols: 2, items: [] });
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

  // ---------- follow the view and the Use ----------
  views.onChange(() => { editing = false; visibility(); build(); });
  window.addEventListener("racenav-use", () => build());

  geometry();
  visibility();
  build();
  return { toggle, shown: () => !side.hidden, openHistory: (k) => openHistoryCard(k) };
}
