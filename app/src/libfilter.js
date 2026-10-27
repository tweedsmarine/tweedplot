// The Library's shared filters ("Whole library"): Dates, Months, Trip and In chart view,
// set once above the tabs and applied to every tab and to the chart. Each kind says what
// its time is (a mark when made, a track or trip the time it ran); routes and barriers
// have no date that means anything, so dates leave them be.
//
//   const g = shared.test(item, { time: (x) => [from, to] | t, inView: (x) => bool, inTrip: (x, trip) => bool });
//
// Filters that belong to one kind (Group, Type, From…) stay in that tab's list (liblist.js).

import { MONTHS, seasonOrder } from "./liblist.js";
import { getTrips, tripById, onTrips } from "./tripstore.js";

const KEY = "racenav.libfilter";
let state = {};
try { state = JSON.parse(localStorage.getItem(KEY) || "{}") || {}; } catch { /* storage unavailable */ }
const listeners = [];
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(state)); } catch { /* ignore */ } };
const changed = () => { save(); for (const f of listeners) f(state); };

export const sharedState = () => state;
export const onShared = (f) => listeners.push(f);
export const sharedActive = () => !!(state.months?.length || state.dates?.from || state.dates?.to || state.trip != null || state.view);
/** Set the shared filters (a saved filter brought back): replaces them all. */
export function setShared(v) { state = JSON.parse(JSON.stringify(v ?? {})); changed(); render(); }
/** The chart moved: with In chart view on, every tab filters again. */
export function viewMoved() { if (state.view) for (const f of listeners) f(state); }
/** Clear one or all. */
export function clearShared(key) { if (key) delete state[key]; else state = {}; changed(); render(); }

// local calendar days ("2026-11-03"): in New Zealand a UTC date is a day out half the time
const localDay = (t) => { const d = new Date(t * 1000); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`; };
function monthsOf(a, b) {
  const out = new Set(), d = new Date(a * 1000);
  d.setDate(1);
  for (let i = 0; i < 13 && d.getTime() / 1000 <= b; i++) { out.add(d.getMonth()); d.setMonth(d.getMonth() + 1); }
  return out;
}

/** Does an item pass the shared filters? `how` says what the kind's time is (null: not dated). */
export function sharedTest(x, how) {
  const span = how.time ? how.time(x) : null;
  const [a, b] = span == null ? [null, null] : Array.isArray(span) ? span : [span, span];
  const dated = how.time != null;
  if (dated && state.months?.length) {
    if (!a) return false;
    const ms = monthsOf(a, b ?? a);
    if (!state.months.some((m) => ms.has(m))) return false;
  }
  if (dated && (state.dates?.from || state.dates?.to)) {
    if (!a) return false;
    if (state.dates.to && localDay(a) > state.dates.to) return false;
    if (state.dates.from && localDay(b ?? a) < state.dates.from) return false;
  }
  if (dated && state.trip != null) {
    const t = tripById(state.trip);
    if (!t || !how.inTrip?.(x, t)) return false;
  }
  if (state.view && how.inView && !how.inView(x)) return false;
  return true;
}

// ---------- saved filters ("Tuna season"): one list for the whole library ----------
// A saved filter keeps the shared filters and each tab's own (Group, Type …), and brings
// them all back in one press. They're the same in every library.
const lists = new Map(); // key → { get, set, active }
export function registerList(key, l) { if (key !== "trash") lists.set(key, l); }
const SAVED = "racenav.saved";
const loadSaved = () => { try { return JSON.parse(localStorage.getItem(SAVED) || "[]"); } catch { return []; } };
const storeSaved = (v) => { try { localStorage.setItem(SAVED, JSON.stringify(v)); } catch { /* ignore */ } };
const clean = (o) => JSON.parse(JSON.stringify(o ?? {}));
const same = (a, b) => JSON.stringify(a ?? {}) === JSON.stringify(b ?? {});
function snapshot() { const tabs = {}; for (const [k, l] of lists) if (l.active()) tabs[k] = clean(l.get()); return { g: clean(state), tabs }; }
function matches(x) {
  if (!same(x.g, state)) return false;
  for (const [k, l] of lists) if (!same(x.tabs?.[k] ?? {}, l.active() ? l.get() : {})) return false;
  return true;
}
const anything = () => sharedActive() || [...lists.values()].some((l) => l.active());
const currentSaved = () => (anything() ? loadSaved().find(matches) : null);
// saved filters made in one tab before (racenav.saved.<tab>) join the one list
(function migrate() {
  const all = loadSaved();
  let changedAny = false;
  for (const key of ["marks", "tracks", "routes", "routings", "barriers", "trips"]) {
    let old = [];
    try { old = JSON.parse(localStorage.getItem(`${SAVED}.${key}`) || "[]"); } catch { /* none */ }
    for (const x of old) {
      if (all.some((y) => y.name === x.name)) continue;
      const tab = { ...(x.f ?? {}) }, g = { ...(x.g ?? {}) };
      for (const k of ["months", "dates", "trip", "view"]) if (k in tab) { g[k] = tab[k]; delete tab[k]; }
      all.push({ name: x.name, g, tabs: Object.keys(tab).length ? { [key]: tab } : {} });
      changedAny = true;
    }
    if (old.length) { try { localStorage.removeItem(`${SAVED}.${key}`); } catch { /* ignore */ } }
  }
  if (changedAny) storeSaved(all.sort((a, b) => a.name.localeCompare(b.name)));
})();
function useSaved(x) {
  state = clean(x.g);
  changed();
  for (const [k, l] of lists) l.set(x.tabs?.[k] ?? {});
  render();
}
function openSaved(chip) {
  closePop();
  pop = document.createElement("div");
  pop.className = "ll-pop ll-saved";
  pop.dataset.key = "saved";
  const list = loadSaved();
  for (const x of list) {
    const o = document.createElement("button");
    o.className = "ll-opt";
    o.classList.toggle("on", matches(x));
    o.title = "Use these filters, in every tab";
    o.innerHTML = `<span class="grow"></span><span class="ll-x" title="Forget this saved filter">✕</span>`;
    o.firstChild.textContent = x.name;
    o.onclick = (e) => {
      if (e.target.closest(".ll-x")) { storeSaved(loadSaved().filter((y) => y.name !== x.name)); openSaved(chip); render(); return; }
      closePop(); useSaved(x);
    };
    pop.append(o);
  }
  if (!list.length) pop.insertAdjacentHTML("beforeend", `<div class="wx-note">No saved filters yet. Set the filters you want (here and in the tabs), then save them here with a name, e.g. Tuna season.</div>`);
  if (anything()) {
    const lab = document.createElement("div");
    lab.className = "wx-note";
    lab.textContent = "Save the filters set now (these, and each tab's own):";
    const row = document.createElement("div");
    row.className = "wx-row";
    row.innerHTML = `<input type="text" class="grow" placeholder="Name, e.g. Tuna season" /><button class="on">Save</button>`;
    const inp = row.querySelector("input"), ok = row.querySelector("button");
    inp.value = currentSaved()?.name ?? "";
    ok.onclick = () => {
      const name = inp.value.trim();
      if (!name) { inp.focus(); return; }
      storeSaved([...loadSaved().filter((y) => y.name !== name), { name, ...snapshot() }].sort((a, b) => a.name.localeCompare(b.name)));
      closePop(); render();
    };
    inp.onkeydown = (e) => { if (e.key === "Enter") ok.click(); };
    pop.append(lab, row);
    setTimeout(() => inp.focus(), 0);
  }
  bar.append(pop);
  const r = chip.getBoundingClientRect(), rr = bar.getBoundingClientRect();
  pop.style.left = `${Math.max(0, Math.min(r.left - rr.left, rr.width - 240))}px`;
  pop.style.top = `${r.bottom - rr.top + 4}px`;
}
/** The tabs' own filters changed: the Saved button may now match (or no longer). */
export function tabChanged() { if (bar) renderSavedChip(); }
function renderSavedChip() {
  const b = bar?.querySelector(".ll-saved-chip");
  if (!b) return;
  const cur = currentSaved();
  b.classList.toggle("on", !!cur);
  b.firstChild.textContent = cur ? `★ ${cur.name}` : "★ Saved";
}

// ---------- the bar above the tabs ----------
let bar = null, pop = null;
function closePop() { pop?.remove(); pop = null; }

function text(key) {
  if (key === "months") return state.months?.length ? seasonOrder(state.months).map((m) => MONTHS[m]).join(", ") : "Months";
  if (key === "dates") return state.dates?.from || state.dates?.to ? `${state.dates.from || "…"} – ${state.dates.to || "…"}` : "Dates";
  if (key === "trip") { const t = state.trip != null && tripById(state.trip); return t ? `Trip: ${t.name}` : "Trip"; }
  return "In chart view";
}
const isOn = (key) => (key === "months" ? state.months?.length > 0 : key === "dates" ? !!(state.dates?.from || state.dates?.to) : key === "trip" ? state.trip != null : !!state.view);

export function render() {
  if (!bar) return;
  bar.innerHTML = `<span class="lf-head" title="These filters apply to every tab and to the chart. Routes and barriers aren't filtered by date">Whole library</span>`;
  const sv = document.createElement("button");
  sv.className = "ll-chip ll-saved-chip";
  sv.title = "Saved filters: use one (in every tab at once), or save the filters set now with a name";
  sv.innerHTML = `<span></span> ▾`;
  sv.onclick = () => { if (pop?.dataset.key === "saved") closePop(); else openSaved(sv); };
  bar.append(sv);
  for (const key of ["dates", "months", "trip", "view"]) {
    const b = document.createElement("button");
    b.className = "ll-chip";
    b.classList.toggle("on", isOn(key));
    b.title = { dates: "Only what was made (or ran) between two dates, in every tab", months: "Only what was made in these months, any year (e.g. Nov, Dec, Jan for the tuna season), in every tab", trip: "Only what was made during one trip, in every tab", view: "Only what is in the area shown on the chart, in every tab" }[key];
    b.innerHTML = `<span></span>${key === "view" ? "" : " ▾"}`;
    b.firstChild.textContent = text(key);
    b.onclick = () => {
      if (key === "view") { state.view = !state.view; if (!state.view) delete state.view; changed(); render(); return; }
      if (pop?.dataset.key === key) { closePop(); return; }
      open(key, b);
    };
    if (isOn(key) && key !== "view") {
      const x = document.createElement("span");
      x.className = "ll-x";
      x.textContent = "✕";
      x.title = "Clear this filter";
      x.onclick = (e) => { e.stopPropagation(); closePop(); clearShared(key); };
      b.append(x);
    }
    bar.append(b);
  }
  renderSavedChip();
}

function open(key, chip) {
  closePop();
  pop = document.createElement("div");
  pop.className = "ll-pop";
  pop.dataset.key = key;
  if (key === "months") {
    const grid = document.createElement("div");
    grid.className = "ll-months";
    const cur = new Set(state.months ?? []);
    MONTHS.forEach((label, val) => {
      const o = document.createElement("button");
      o.textContent = label;
      o.classList.toggle("on", cur.has(val));
      o.onclick = () => {
        if (cur.has(val)) cur.delete(val); else cur.add(val);
        o.classList.toggle("on", cur.has(val));
        state.months = [...cur].sort((a, b) => a - b);
        if (!state.months.length) delete state.months;
        changed();
        chip.firstChild.textContent = text("months");
        chip.classList.toggle("on", isOn("months"));
      };
      grid.append(o);
    });
    pop.append(grid);
    pop.insertAdjacentHTML("beforeend", `<div class="wx-note">Any year. Marks with an unknown date are left out.</div>`);
  } else if (key === "dates") {
    pop.innerHTML = `<div class="wx-row"><label>From</label><input type="date" class="ll-from"></div><div class="wx-row"><label>To</label><input type="date" class="ll-to"></div><div class="wx-note">Tracks and trips that ran in the dates are kept. Marks with an unknown date are left out.</div>`;
    const from = pop.querySelector(".ll-from"), to = pop.querySelector(".ll-to");
    from.value = state.dates?.from ?? ""; to.value = state.dates?.to ?? "";
    const set = () => { state.dates = { from: from.value, to: to.value }; if (!from.value && !to.value) delete state.dates; changed(); render(); };
    from.onchange = set; to.onchange = set;
  } else if (key === "trip") {
    const trips = getTrips();
    for (const t of trips) {
      const o = document.createElement("button");
      o.className = "ll-opt";
      o.classList.toggle("on", String(state.trip) === String(t.id));
      o.innerHTML = `<span class="grow"></span><small></small>`;
      o.firstChild.textContent = `${t.name}${t.fishery ? ` · ${t.fishery}` : ""}`;
      o.lastChild.textContent = t.started ? new Date(t.started * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }) : "";
      o.onclick = () => { state.trip = t.id; changed(); closePop(); render(); };
      pop.append(o);
    }
    if (!trips.length) pop.innerHTML = `<div class="wx-note">No trips yet: start one, or add a past trip, in the Trips tab.</div>`;
  }
  bar.append(pop);
  const r = chip.getBoundingClientRect(), rr = bar.getBoundingClientRect();
  pop.style.left = `${Math.max(0, Math.min(r.left - rr.left, rr.width - 240))}px`;
  pop.style.top = `${r.bottom - rr.top + 4}px`;
}

/** Put the bar in `el` (once). */
export function mountShared(el) {
  bar = el;
  bar.classList.add("lf-bar");
  document.addEventListener("pointerdown", (e) => { if (pop && !pop.contains(e.target) && !e.target.closest(".lf-bar .ll-chip")) closePop(); });
  onTrips(() => { if (state.trip != null && !tripById(state.trip)) clearShared("trip"); else render(); });
  render();
}
