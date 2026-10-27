// Library > Log: the fishing log. One press saves an event (net shot, net hauled, line set
// and hauled, pots set and hauled, catch, a note) with the time, the boat's position and what
// the instruments said (depth, sea temperature, SOG, COG). A haul is paired with the shot it
// ends, so the tow's time and distance show. An interval timer reminds (a soft chime and a
// card, never an alarm) every so many minutes, and can start itself when gear goes in.
// Events follow the Whole library filters, belong to the trip they were made in, and are
// drawn on the chart (a tow as a dashed line from shot to haul).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as activity from "./activity.js";
import { libList } from "./liblist.js";
import { ask } from "./ask.js";
import { ddm, exportLibraryFile } from "./library.js";
import { sharedTest, sharedActive, onShared } from "./libfilter.js";
import { distNm } from "./instruments.js";

const $ = (id) => document.getElementById(id);
const KEY = "racenav.events";
// what each button saves; "start" is the kind a haul ends
const KINDS = {
  "net-shot": { label: "Net shot", gear: "net", icon: "sym-tri-down", colour: "#1f6fb2" },
  "net-haul": { label: "Net hauled", gear: "net", icon: "sym-tri-up", colour: "#d35400", start: "net-shot" },
  "line-set": { label: "Line set", gear: "line", icon: "sym-tri-down", colour: "#1f6fb2" },
  "line-haul": { label: "Line hauled", gear: "line", icon: "sym-tri-up", colour: "#d35400", start: "line-set" },
  "pot-set": { label: "Pots set", gear: "pot", icon: "sym-tri-down", colour: "#1f6fb2" },
  "pot-haul": { label: "Pots hauled", gear: "pot", icon: "sym-tri-up", colour: "#d35400", start: "pot-set" },
  catch: { label: "Catch", gear: "", icon: "sym-fish", colour: "#16a085", ask: "What was caught (and how much)?" },
  note: { label: "Note", gear: "", icon: "sym-excl", colour: "#5b6b75", ask: "The note" },
};
const GEAR = { net: "Net", line: "Longline", pot: "Pots" };
const clock = (t) => new Date(t * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
const day = (t) => new Date(t * 1000).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
const toLocal = (t) => { const d = new Date(t * 1000); d.setMinutes(d.getMinutes() - d.getTimezoneOffset()); return d.toISOString().slice(0, 16); };
const dur = (s) => { const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60); return h ? `${h} h ${m} min` : `${m} min`; };

export function setupEvents(map, { ui, getBoat }) {
  let prefs = { gear: ["net", "line", "pot"], every: 60, auto: false, show: true };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  let events = [];
  const byId = (id) => events.find((e) => e.id === id);

  // ---------- the buttons ----------
  function renderButtons() {
    const box = $("evt-buttons");
    box.innerHTML = "";
    for (const [kind, k] of Object.entries(KINDS)) {
      if (k.gear && !prefs.gear.includes(k.gear)) continue;
      const b = document.createElement("button");
      b.className = `evt-btn ${k.start ? "haul" : k.gear ? "shot" : "other"}`;
      b.textContent = k.label;
      b.title = `Save “${k.label}” now, with the boat's position, depth and sea temperature`;
      b.onclick = () => log(kind);
      box.append(b);
    }
    const g = $("evt-gear");
    g.innerHTML = "";
    for (const [key, label] of Object.entries(GEAR)) {
      const c = document.createElement("button");
      c.className = "ll-chip";
      c.classList.toggle("on", prefs.gear.includes(key));
      c.textContent = label;
      c.title = `Show the ${label.toLowerCase()} buttons`;
      c.onclick = () => { prefs.gear = prefs.gear.includes(key) ? prefs.gear.filter((x) => x !== key) : [...prefs.gear, key]; save(); renderButtons(); };
      g.append(c);
    }
  }

  async function log(kind) {
    const k = KINDS[kind];
    const at = Math.floor(Date.now() / 1000); // the moment pressed, even if a note is asked for
    const { pos, v = {} } = getBoat() ?? {};
    let notes = "";
    if (k.ask) {
      const a = await ask({ title: `${k.label} at ${clock(at)}`, fields: [{ key: "notes", label: k.ask, type: "text" }], ok: "Save" });
      if (!a) return;
      notes = a.notes;
    }
    const e = { kind, name: k.label, at, lat: pos?.[1] ?? null, lon: pos?.[0] ?? null, depth: v.depth ?? null, temp: v.water_temp ?? null, sog: v.sog ?? null, cog: v.cog ?? null, notes };
    try {
      const saved = await invoke("event_save", { event: e });
      const extra = [];
      if (!pos) extra.push("No position from Signal K: saved without one (Edit can add notes).");
      const shot = saved.pair && byId(saved.pair);
      if (shot) extra.push(`Tow: ${dur(saved.at - shot.at)} since ${k.label.replace("hauled", "").trim().toLowerCase()} ${shot.kind.endsWith("shot") ? "shot" : "set"} at ${clock(shot.at)}.`);
      activity.note(`${k.label} ${clock(at)}`, { details: [...extra, ...(saved.depth != null ? [`Depth ${saved.depth.toFixed(1)} m`] : []), ...(saved.temp != null ? [`Sea ${saved.temp.toFixed(1)} °C`] : [])] });
      if (prefs.auto && k.gear && !k.start) startTimer();
      if (prefs.auto && k.start) stopTimer();
    } catch (err) { activity.start(`Saving ${k.label}`).fail(String(err)); }
    refresh();
  }

  // ---------- gear in the water ----------
  let openTimer = 0;
  function renderOpen() {
    const box = $("evt-open");
    clearInterval(openTimer);
    const hauled = new Set(events.map((e) => e.pair).filter(Boolean));
    const open = events.filter((e) => KINDS[e.kind]?.gear && !KINDS[e.kind].start && !hauled.has(e.id) && Date.now() / 1000 - e.at < 3 * 86400);
    box.hidden = !open.length;
    if (!open.length) return;
    const fill = () => { box.textContent = open.map((e) => `${GEAR[KINDS[e.kind].gear]} in since ${clock(e.at)} (${dur(Date.now() / 1000 - e.at)})`).join(" · "); };
    fill();
    openTimer = setInterval(fill, 30000);
  }

  // ---------- the interval timer ----------
  let timer = null; // { due, every }
  let tick = 0;
  function chime() {
    try {
      const ac = new AudioContext(), o = ac.createOscillator(), g = ac.createGain();
      o.frequency.value = 660; g.gain.setValueAtTime(0.0001, ac.currentTime);
      g.gain.exponentialRampToValueAtTime(0.15, ac.currentTime + 0.05); g.gain.exponentialRampToValueAtTime(0.0001, ac.currentTime + 0.9);
      o.connect(g).connect(ac.destination); o.start(); o.stop(ac.currentTime + 1);
    } catch { /* no sound */ }
  }
  function startTimer() {
    const every = Math.max(1, Number($("evt-every").value) || prefs.every);
    prefs.every = every; save();
    timer = { every, due: Date.now() + every * 60000 };
    renderTimer();
  }
  function stopTimer() { timer = null; renderTimer(); }
  function renderTimer() {
    clearInterval(tick);
    $("evt-timer").textContent = timer ? "Stop" : "Start";
    $("evt-timer").classList.toggle("on", !!timer);
    const left = $("evt-timer-left");
    if (!timer) { left.textContent = ""; return; }
    const step = () => {
      const s = Math.max(0, Math.round((timer.due - Date.now()) / 1000));
      left.textContent = `next in ${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
      if (s === 0) {
        chime();
        activity.start(`Timer: ${timer.every} min`).done(`${clock(Date.now() / 1000)}: time to check the gear (or log something).`, { keep: true });
        timer.due += timer.every * 60000;
      }
    };
    step();
    tick = setInterval(step, 1000);
  }
  $("evt-every").value = String(prefs.every);
  $("evt-timer").onclick = () => (timer ? stopTimer() : startTimer());
  $("evt-auto").checked = prefs.auto;
  $("evt-auto").onchange = (e) => { prefs.auto = e.target.checked; save(); };

  // ---------- the list ----------
  const inView = (e) => { if (e.lat == null) return false; const b = map.getBounds(); return e.lat >= b.getSouth() && e.lat <= b.getNorth() && e.lon >= b.getWest() && e.lon <= b.getEast(); };
  const SHARED = { time: (e) => e.at, inView, inTrip: (e, t) => e.at >= t.started && e.at <= (t.ended ?? Date.now() / 1000) };
  function detail(e) {
    const shot = e.pair && byId(e.pair);
    return [
      e.lat != null ? `${ddm(e.lat, "N", "S")} ${ddm(e.lon, "E", "W")}` : "no position",
      e.depth != null ? `${e.depth.toFixed(0)} m` : "",
      e.temp != null ? `${e.temp.toFixed(1)} °C` : "",
      shot ? `tow ${dur(e.at - shot.at)}${shot.lat != null && e.lat != null ? `, ${distNm([shot.lon, shot.lat], [e.lon, e.lat]).toFixed(1)} NM` : ""}` : "",
      e.notes,
    ].filter(Boolean).join(" · ");
  }
  const list = libList($("lib-log-list"), {
    key: "log",
    noun: ["event", "events"],
    searchPlaceholder: "Find an event",
    search: (e, q) => `${e.name} ${e.notes}`.toLowerCase().includes(q),
    shared: SHARED,
    sorts: [
      { key: "new", label: "Newest first", cmp: (a, b) => b.at - a.at },
      { key: "old", label: "Oldest first", cmp: (a, b) => a.at - b.at },
    ],
    filters: [
      { key: "kind", label: "Event", type: "choice", options: (es) => { const c = new Map(); for (const e of es) c.set(e.kind, (c.get(e.kind) || 0) + 1); return [...c].map(([k, n]) => [k, KINDS[k]?.label ?? k, n]); }, test: (e, v) => e.kind === v },
    ],
    row: {
      render(e, el) {
        el.innerHTML = `<div class="main" title="Go to it on the chart"><b></b><small></small></div><button data-act="edit">Edit</button>`;
        el.querySelector("b").textContent = `${e.name} · ${day(e.at)}`;
        el.querySelector("small").textContent = detail(e);
        el.querySelector('[data-act="edit"]').onclick = () => edit(e);
      },
      click: (e) => { if (e.lat != null) map.easeTo({ center: [e.lon, e.lat], zoom: Math.max(map.getZoom(), 11) }); },
    },
    actions: [
      { label: "CSV", title: "Write the selected events as a CSV file (a spreadsheet, or an e-logbook)", run: async (es) => {
        try { const p = await invoke("events_csv", { name: "fishing log", ids: es.map((e) => e.id) }); activity.note(`${es.length} event${es.length === 1 ? "" : "s"} written as CSV`, { details: [`Written to ${p}`, "Times are UTC."] }); } catch (e) { activity.start("Writing CSV").fail(String(e)); }
      } },
      { label: "Library file…", title: "Write the selected events as a library file of their own", run: (es) => exportLibraryFile({ events: es.map((e) => e.id) }, `${es.length} log event${es.length === 1 ? "" : "s"}`, "fishing log") },
      { label: "Delete", danger: true, confirm: (es) => `Delete ${es.length} event${es.length === 1 ? "" : "s"}? (The Trash keeps them 30 days.)`, run: async (es) => {
        try { const n = await invoke("events_delete", { ids: es.map((e) => e.id) }); activity.note(`Deleted ${n} event${n === 1 ? "" : "s"}`); } catch (e) { activity.start("Deleting events").fail(String(e)); }
        refresh();
      } },
    ],
    empty: () => "Nothing logged yet. Press a button above when the gear goes in, comes up, or something's caught.",
    changed: () => draw(),
  });

  // ---------- editing one ----------
  const form = $("evt-edit");
  let editing = null;
  function edit(e) {
    editing = e;
    form.hidden = false;
    $("evt-edit-title").textContent = `${e.name} · ${day(e.at)}`;
    form.at.value = toLocal(e.at);
    form.notes.value = e.notes || "";
    form.notes.focus();
  }
  form.querySelector('[data-act="cancel"]').onclick = () => { form.hidden = true; editing = null; };
  form.querySelector('[data-act="delete"]').onclick = async () => {
    if (!editing) return;
    await invoke("events_delete", { ids: [editing.id] }).catch(() => {});
    activity.note(`Deleted ${editing.name} ${clock(editing.at)}`, { details: ["Library > Trash brings it back."] });
    form.hidden = true; editing = null; refresh();
  };
  form.onsubmit = async (ev) => {
    ev.preventDefault();
    if (!editing) return;
    const at = form.at.value ? Math.floor(new Date(form.at.value).getTime() / 1000) : editing.at;
    try { await invoke("event_save", { event: { ...editing, at, notes: form.notes.value } }); } catch (e) { activity.start("Saving the event").fail(String(e)); }
    form.hidden = true; editing = null; refresh();
  };

  // ---------- the chart ----------
  const empty = { type: "FeatureCollection", features: [] };
  map.addSource("evt", { type: "geojson", data: empty });
  map.addSource("evt-tows", { type: "geojson", data: empty });
  map.addLayer({ id: "evt-tows", type: "line", source: "evt-tows", paint: { "line-color": "#1f6fb2", "line-width": 1.6, "line-dasharray": [2, 2], "line-opacity": 0.8 } }, "lib-buoys");
  map.addLayer({ id: "evt-icons", type: "symbol", source: "evt",
    layout: { "icon-image": ["concat", "sym:", ["get", "icon"], ":", ["get", "colour"]], "icon-size": 0.6, "icon-allow-overlap": true, "icon-ignore-placement": true } }, "lib-buoys");
  map.addLayer({ id: "evt-labels", type: "symbol", source: "evt", minzoom: 10,
    layout: { "icon-image": ["concat", "lbl:", ["get", "label"]], "icon-anchor": "left", "icon-offset": [10, 0], "icon-size": 0.8, "icon-allow-overlap": false } }, "lib-buoys");
  function draw() {
    const vis = prefs.show ? "visible" : "none";
    for (const id of ["evt-tows", "evt-icons", "evt-labels"]) map.setLayoutProperty(id, "visibility", vis);
    // what the list shows (search, its filters and the Whole library ones)
    const shown = list.filtered().filter((e) => e.lat != null);
    map.getSource("evt").setData({ type: "FeatureCollection", features: shown.map((e) => ({ type: "Feature",
      properties: { id: e.id, icon: KINDS[e.kind]?.icon ?? "sym-dot", colour: KINDS[e.kind]?.colour ?? "#5b6b75", label: `${e.name} ${clock(e.at)}` },
      geometry: { type: "Point", coordinates: [e.lon, e.lat] } })) });
    const ids = new Set(shown.map((e) => e.id));
    const tows = shown.filter((e) => e.pair && ids.has(e.pair)).map((h) => { const s = byId(h.pair); return s?.lat != null ? { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: [[s.lon, s.lat], [h.lon, h.lat]] } } : null; }).filter(Boolean);
    map.getSource("evt-tows").setData({ type: "FeatureCollection", features: tows });
  }
  $("evt-show").checked = prefs.show;
  $("evt-show").onchange = (e) => { prefs.show = e.target.checked; save(); draw(); };
  map.on("click", "evt-icons", (e) => {
    if (document.querySelector(".armed")) return;
    const ev = byId(e.features[0].properties.id);
    if (!ev) return;
    ui.openTab("library", "log");
    if (!list.highlight(ev.id)) { list.clearFilters(); list.highlight(ev.id); }
  });
  map.on("mouseenter", "evt-icons", () => { map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "evt-icons", () => { map.getCanvas().style.cursor = ""; });
  onShared(() => draw());

  async function refresh() {
    try { events = await invoke("events_list"); } catch { events = []; }
    list.setItems(events);
    renderOpen();
  }
  listen("events-changed", () => refresh());
  listen("library-changed", () => refresh());
  renderButtons();
  renderTimer();
  refresh();
  return { refresh, events: () => events, sharedActive };
}
