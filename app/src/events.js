// Library > Log: the fishing log. One press saves an event (net shot, net hauled, line set
// and hauled, pots set and hauled, catch, a note) with the time, the boat's position and what
// the instruments said (depth, sea temperature, SOG, COG). A haul is paired with the shot it
// ends, so the tow's time and distance show. An interval timer reminds (a soft chime and a
// card, never an alarm) every so many minutes, and can start itself when gear goes in.
// Events follow the Whole library filters, belong to the trip they were made in, and are
// drawn on the chart (a tow as a dashed line from shot to haul).

import { placing } from "./modes.js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as activity from "./activity.js";
import { libList } from "./liblist.js";
import { ask } from "./ask.js";
import { ddm, exportLibraryFile } from "./library.js";
import { sharedTest, sharedActive, onShared } from "./libfilter.js";
import { distNm } from "./instruments.js";
import { infoBox } from "./infobox.js";

const $ = (id) => document.getElementById(id);
const KEY = "racenav.events";
// what each button saves; "start" is the kind a haul ends
const KINDS = {
  "net-shot": { label: "Net shot", gear: "net", icon: "sym-tri-down", colour: "#1f6fb2" },
  "net-haul": { label: "Net haul started", button: "Net haul", gear: "net", icon: "sym-tri-up", colour: "#d35400", start: "net-shot", dialog: true },
  "line-set": { label: "Line shot", gear: "line", icon: "sym-tri-down", colour: "#1f6fb2", window: true },
  "line-haul": { label: "Line haul started", button: "Line haul", gear: "line", icon: "sym-tri-up", colour: "#d35400", start: "line-set", dialog: true },
  "pot-set": { label: "Pots shot", gear: "pot", icon: "sym-tri-down", colour: "#1f6fb2" },
  "pot-haul": { label: "Pots haul started", button: "Pots haul", gear: "pot", icon: "sym-tri-up", colour: "#d35400", start: "pot-set", dialog: true },
  catch: { label: "Catch", gear: "", icon: "sym-fish", colour: "#16a085", ask: "What was caught (and how much)?" },
  // written by the line-shooting window (setter.js) or the haul box, not by a button here
  "line-end": { label: "Line shot finished", gear: "", icon: "sym-square", colour: "#1f6fb2", auto: true },
  "haul-end": { label: "Haul finished", gear: "", icon: "sym-square", colour: "#d35400", auto: true },
  float: { label: "Float", gear: "", icon: "sym-dot", colour: "#d35400", auto: true },
  "radio-buoy": { label: "Radio buoy", gear: "", icon: "sym-flag", colour: "#c2185b", auto: true },
  note: { label: "Note", gear: "", icon: "sym-excl", colour: "#5b6b75", ask: "The note" },
};
// the shoot and haul kinds of each gear
const SHOOT = { net: "net-shot", line: "line-set", pot: "pot-set" };
const HAUL = { net: "net-haul", line: "line-haul", pot: "pot-haul" };
const GEAR = { net: "Net", line: "Longline", pot: "Pots" };
const clock = (t) => new Date(t * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
const day = (t) => new Date(t * 1000).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
const toLocal = (t) => { const d = new Date(t * 1000); d.setMinutes(d.getMinutes() - d.getTimezoneOffset()); return d.toISOString().slice(0, 16); };
const dur = (s) => { const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60); return h ? `${h} h ${m} min` : `${m} min`; };

export function setupEvents(map, { ui, getBoat }) {
  let prefs = { gear: ["net", "line", "pot"], every: 60, auto: false, show: true, floats: false };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  let events = [];
  const byId = (id) => events.find((e) => e.id === id);

  // ---------- the buttons ----------
  function renderButtons() {
    const box = $("evt-buttons");
    box.innerHTML = "";
    for (const [kind, k] of Object.entries(KINDS)) {
      if (k.auto || (k.gear && !prefs.gear.includes(k.gear))) continue;
      const b = document.createElement("button");
      b.className = `evt-btn ${k.start ? "haul" : k.gear ? "shot" : "other"}`;
      b.textContent = k.button ?? k.label;
      b.title = k.window ? "Open the line-shooting window: the snood and float beeper (it logs the shot when started)"
        : k.dialog ? `Open the haul box: Start haul, Catch as it comes aboard, End haul`
        : `Save “${k.label}” now, with the boat's position, depth and sea temperature`;
      b.onclick = () => (k.window ? openSetter() : k.dialog ? openHaul(k.gear) : log(kind));
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

  async function log(kind, extra = {}) {
    const k = KINDS[kind];
    const at = Math.floor(Date.now() / 1000); // the moment pressed, even if a note is asked for
    const { pos, v = {} } = getBoat() ?? {};
    let notes = "";
    if (k.ask && extra.notes == null) {
      const a = await ask({ title: `${k.label} at ${clock(at)}`, fields: [{ key: "notes", label: k.ask, type: "text" }], ok: "Save" });
      if (!a) return;
      notes = a.notes;
    }
    if (extra.notes != null) { notes = extra.notes; }
    const e = { kind, name: k.label, at, lat: pos?.[1] ?? null, lon: pos?.[0] ?? null, depth: v.depth ?? null, temp: v.water_temp ?? null, sog: v.sog ?? null, cog: v.cog ?? null, notes, ...extra };
    if (extra.quiet) delete e.quiet;
    try {
      const saved = await invoke("event_save", { event: e });
      if (extra.quiet) { await refresh(); return saved; }
      const lines = [];
      if (!pos) lines.push("No position from Signal K: saved without one (Edit can add notes).");
      const shot = saved.pair && byId(saved.pair);
      if (shot) lines.push(`${shot.kind === "net-shot" ? "Tow" : "Soak"}: ${dur(saved.at - shot.at)} since it was shot at ${clock(shot.at)}.`);
      activity.note(`${k.label} ${clock(at)}`, { details: [...lines, ...(saved.depth != null ? [`Depth ${saved.depth.toFixed(1)} m`] : []), ...(saved.temp != null ? [`Sea ${saved.temp.toFixed(1)} °C`] : [])] });
      if (prefs.auto && k.gear && !k.start) startTimer();
      if (prefs.auto && k.start) stopTimer();
      await refresh();
      return saved;
    } catch (err) { activity.start(`Saving ${k.label}`).fail(String(err)); }
    refresh();
    return null;
  }

  // ---------- shooting and hauling (the toolbar's Shoot and Haul, and the Log's buttons) ----------
  const GEAR_LABEL = { net: "Net", line: "Line", pot: "Pots" };
  function openSetter() { invoke("setter_open").catch((e) => activity.start("Shooting the line").fail(String(e))); }
  function shoot(gear) {
    prefs.lastGear = gear; save(); renderTools();
    if (gear === "line") openSetter();
    else log(SHOOT[gear]);
  }

  // the haul box: Start haul at the first float (or the net / first pot), Catch as it comes
  // aboard (per hook, per basket: as the skipper likes), End haul at the last. A haul in
  // progress is read back from the log, so it survives closing the box or the app.
  const haulPrefs = () => (prefs.haul ??= { what: "", n: 1 });
  function haulState(gear) {
    const start = events.filter((e) => e.kind === HAUL[gear]).sort((a, b) => b.at - a.at)[0];
    if (!start || events.some((e) => e.kind === "haul-end" && e.pair === start.id)) return null;
    const catches = events.filter((e) => e.kind === "catch" && e.data?.haul === start.id);
    return { start, catches, n: catches.reduce((a, c) => a + (Number(c.data?.n) || 1), 0) };
  }
  let haulBox = null, haulGear = "line", haulTick = 0;
  function openHaul(gear) {
    haulGear = gear;
    prefs.lastGear = gear; save(); renderTools();
    const el = document.createElement("div");
    el.className = "haul-box";
    el.innerHTML = `<div class="haul-state"><b></b><small></small></div>
      <div class="haul-what"><label>Catch <input class="haul-text" type="text" placeholder="e.g. bigeye, or a basket"></label><label>Number <input class="haul-n" type="number" min="1" step="1"></label></div>
      <div class="haul-buttons"><button class="haul-start on">Start haul</button><button class="haul-catch">Catch</button><button class="haul-end warn">End haul</button></div>
      <ul class="haul-list"></ul>`;
    const text = el.querySelector(".haul-text"), num = el.querySelector(".haul-n");
    text.value = haulPrefs().what; num.value = haulPrefs().n;
    text.onchange = () => { haulPrefs().what = text.value.trim(); save(); };
    num.onchange = () => { haulPrefs().n = Math.max(1, Number(num.value) || 1); save(); };
    el.querySelector(".haul-start").onclick = async () => {
      if (haulState(gear)) return;
      await log(HAUL[gear]);
      renderHaul();
    };
    el.querySelector(".haul-catch").onclick = async () => {
      const st = haulState(gear);
      if (!st) return;
      const n = Math.max(1, Number(num.value) || 1), what = text.value.trim();
      await log("catch", { quiet: true, notes: [n > 1 ? `${n} ×` : "", what].filter(Boolean).join(" "), data: { haul: st.start.id, n, what } });
      renderHaul();
    };
    const endBtn = el.querySelector(".haul-end");
    endBtn.onclick = async () => {
      const st = haulState(gear);
      if (!st) return;
      if (!endBtn.dataset.armed) { endBtn.dataset.armed = "1"; endBtn.textContent = "End the haul?"; setTimeout(() => { delete endBtn.dataset.armed; endBtn.textContent = "End haul"; }, 3000); return; }
      delete endBtn.dataset.armed; endBtn.textContent = "End haul";
      const shot = st.start.pair && byId(st.start.pair);
      await log("haul-end", { name: `${GEAR_LABEL[gear]} haul finished`, pair: st.start.id,
        notes: `${st.n} caught in ${dur(Date.now() / 1000 - st.start.at)}${shot ? `, soak ${dur(st.start.at - shot.at)}` : ""}`,
        data: { gear, catches: st.n, haul: st.start.id, shot: shot?.id ?? null } });
      renderHaul();
    };
    haulBox = infoBox(map, "haul", { width: 340, height: 330 });
    haulBox.open(el, { title: `${GEAR_LABEL[gear]} haul`, onClose: () => { clearInterval(haulTick); haulBox = null; } });
    renderHaul();
    clearInterval(haulTick);
    haulTick = setInterval(renderHaul, 15000);
  }
  // the last haul of this gear that's finished, if it's newer than the last shot
  function lastHaulEnd() {
    const end = events.filter((e) => e.kind === "haul-end" && e.data?.gear === haulGear).sort((a, b) => b.at - a.at)[0];
    const shot = events.filter((e) => e.kind === SHOOT[haulGear]).sort((a, b) => b.at - a.at)[0];
    return end && (!shot || end.at > shot.at) ? end : null;
  }
  function renderHaul() {
    const el = document.querySelector(".haul-box");
    if (!haulBox || !el) return;
    const st = haulState(haulGear);
    const shot = st ? st.start.pair && byId(st.start.pair) : events.filter((e) => e.kind === SHOOT[haulGear]).sort((a, b) => b.at - a.at)[0];
    el.querySelector(".haul-state b").textContent = st ? `Hauling since ${clock(st.start.at)} · ${st.n} caught` : "Not hauling";
    el.querySelector(".haul-state small").textContent = st
      ? `${dur(Date.now() / 1000 - st.start.at)} so far${shot ? ` · shot at ${clock(shot.at)}, soak ${dur(st.start.at - shot.at)}` : ""}`
      : lastHaulEnd() ? `Last haul finished at ${clock(lastHaulEnd().at)}: ${lastHaulEnd().notes}. Start haul when the next comes aboard.`
      : shot ? `Last shot at ${clock(shot.at)} (${dur(Date.now() / 1000 - shot.at)} ago). Start haul when the first ${haulGear === "line" ? "float" : haulGear === "pot" ? "pot" : "of the net"} comes aboard.` : "Start haul when the gear starts coming aboard.";
    el.querySelector(".haul-start").disabled = !!st;
    el.querySelector(".haul-catch").disabled = !st;
    el.querySelector(".haul-end").disabled = !st;
    const ul = el.querySelector(".haul-list");
    ul.innerHTML = "";
    for (const c of (st?.catches ?? []).slice().sort((a, b) => b.at - a.at).slice(0, 8)) {
      const li = document.createElement("li");
      li.textContent = `${clock(c.at)}  ${c.notes || "catch"}`;
      ul.append(li);
    }
  }

  // ---------- the toolbar's Shoot and Haul: left click the last gear, right-click to choose ----------
  function renderTools() {
    const g = prefs.lastGear ?? "line";
    for (const [id, verb] of [["shoot-btn", "Shoot"], ["haul-btn", "Haul"]]) {
      const b = $(id);
      if (!b) continue;
      b.title = `${verb} the ${GEAR_LABEL[g].toLowerCase()} (right-click to choose net, line or pots)`;
      b.querySelector(".tb-gear").textContent = GEAR_LABEL[g][0];
    }
  }
  const gearPop = document.createElement("div");
  gearPop.className = "ctx gear-pop";
  gearPop.hidden = true;
  document.body.append(gearPop);
  function openGearPop(btn, verb) {
    gearPop.innerHTML = `<div class="grp">${verb}</div>${Object.entries(GEAR_LABEL).map(([g, l]) => `<button data-g="${g}" class="${g === prefs.lastGear ? "cur" : ""}">${verb} ${verb === "Shoot" ? (g === "pot" ? "pots" : `the ${l.toLowerCase()}`) : (g === "pot" ? "pots" : `the ${l.toLowerCase()}`)}</button>`).join("")}`;
    gearPop.onclick = (e) => { const b = e.target.closest("button[data-g]"); if (!b) return; gearPop.hidden = true; (verb === "Shoot" ? shoot : openHaul)(b.dataset.g); };
    const r = btn.getBoundingClientRect();
    gearPop.style.left = `${r.right + 8}px`;
    gearPop.style.top = `${Math.min(r.top, window.innerHeight - 160)}px`;
    gearPop.hidden = false;
  }
  document.addEventListener("pointerdown", (e) => { if (!gearPop.hidden && !gearPop.contains(e.target) && !e.target.closest("#shoot-btn, #haul-btn")) gearPop.hidden = true; });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !gearPop.hidden) { gearPop.hidden = true; e.preventDefault(); } });
  for (const [id, verb] of [["shoot-btn", "Shoot"], ["haul-btn", "Haul"]]) {
    const b = $(id);
    if (!b) continue;
    b.onclick = () => { const g = prefs.lastGear ?? "line"; if (verb === "Shoot") shoot(g); else openHaul(g); };
    b.addEventListener("contextmenu", (e) => { e.preventDefault(); openGearPop(b, verb); });
  }
  renderTools();

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
      { label: "Delete", danger: true, confirm: (es) => { const f = floatsOf(es.map((e) => e.id)).length; return `Delete ${es.length} event${es.length === 1 ? "" : "s"}${f ? ` and the ${f} float${f === 1 ? "" : "s"} of their sets` : ""}? (The Trash keeps them 30 days.)`; }, run: async (es) => {
        try { const n = await invoke("events_delete", { ids: withFloats(es.map((e) => e.id)) }); activity.note(`Deleted ${n} event${n === 1 ? "" : "s"}`); } catch (e) { activity.start("Deleting events").fail(String(e)); }
        refresh();
      } },
    ],
    empty: () => (events.some((e) => e.kind === "float") && !prefs.floats ? "No events listed (the floats are on the chart: see above)." : "Nothing logged yet. Press a button above when the gear goes in, comes up, or something's caught."),
    changed: () => draw(),
  });

  // a set's floats go with it: those logged while setting, by the shot's id
  const floatsOf = (ids) => { const set = new Set(ids); return events.filter((e) => e.kind === "float" && set.has(e.data?.set) && !set.has(e.id)); };
  const withFloats = (ids) => [...new Set([...ids, ...floatsOf(ids).map((e) => e.id)])];

  // floats on the chart that the list doesn't show (Floats unticked): say so, and offer to list or delete them
  const floatNote = $("evt-float-note");
  let floatDelArmed = null;
  function renderFloatNote() {
    const fl = prefs.floats ? [] : events.filter((e) => e.kind === "float" && e.lat != null && sharedTest(e, SHARED));
    floatNote.hidden = !fl.length || !prefs.show;
    if (floatNote.hidden) return;
    const orphans = fl.filter((e) => !e.data?.set || !byId(e.data.set)).length;
    floatNote.querySelector("span").textContent = `${fl.length} float${fl.length === 1 ? "" : "s"} from line sets on the chart, not listed${orphans ? ` (${orphans} from sets no longer in the log)` : ""}.`;
    const del = floatNote.querySelector('[data-act="del"]');
    del.onclick = async () => {
      if (!floatDelArmed) {
        del.textContent = `Delete ${fl.length}? Click again`;
        floatDelArmed = setTimeout(() => { floatDelArmed = null; del.textContent = "Delete them"; }, 4000);
        return;
      }
      clearTimeout(floatDelArmed); floatDelArmed = null; del.textContent = "Delete them";
      try { const n = await invoke("events_delete", { ids: fl.map((e) => e.id) }); activity.note(`Deleted ${n} float${n === 1 ? "" : "s"}`, { details: ["Library > Trash brings them back. Their marks (one a float, grouped by set) are in the Marks tab."] }); } catch (e) { activity.start("Deleting floats").fail(String(e)); }
      refresh();
    };
    floatNote.querySelector('[data-act="list"]').onclick = () => { prefs.floats = true; $("evt-floats").checked = true; save(); refresh(); };
  }

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
    const f = floatsOf([editing.id]).length;
    await invoke("events_delete", { ids: withFloats([editing.id]) }).catch(() => {});
    activity.note(`Deleted ${editing.name} ${clock(editing.at)}${f ? ` and its ${f} float${f === 1 ? "" : "s"}` : ""}`, { details: ["Library > Trash brings it back."] });
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
    const shown = [...list.filtered(), ...(prefs.floats ? [] : events.filter((e) => e.kind === "float" && sharedTest(e, SHARED)))].filter((e) => e.lat != null);
    map.getSource("evt").setData({ type: "FeatureCollection", features: shown.map((e) => ({ type: "Feature",
      properties: { id: e.id, icon: KINDS[e.kind]?.icon ?? "sym-dot", colour: KINDS[e.kind]?.colour ?? "#5b6b75", label: e.kind === "float" ? e.name : `${e.name} ${clock(e.at)}` },
      geometry: { type: "Point", coordinates: [e.lon, e.lat] } })) });
    const ids = new Set(shown.map((e) => e.id));
    const tows = shown.filter((e) => e.pair && ids.has(e.pair)).map((h) => { const s = byId(h.pair); return s?.lat != null ? { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: [[s.lon, s.lat], [h.lon, h.lat]] } } : null; }).filter(Boolean);
    map.getSource("evt-tows").setData({ type: "FeatureCollection", features: tows });
  }
  $("evt-floats").checked = prefs.floats;
  $("evt-floats").onchange = (e) => { prefs.floats = e.target.checked; save(); refresh(); };
  $("evt-show").checked = prefs.show;
  $("evt-show").onchange = (e) => { prefs.show = e.target.checked; save(); draw(); renderFloatNote(); };
  map.on("click", "evt-icons", (e) => {
    if (placing()) return;
    const ev = byId(e.features[0].properties.id);
    if (!ev) return;
    ui.openTab("library", "log");
    if (!list.highlight(ev.id)) { list.clearFilters(); list.highlight(ev.id); }
  });
  map.on("mouseenter", "evt-icons", () => { map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "evt-icons", () => { map.getCanvas().style.cursor = ""; });
  onShared(() => { draw(); renderFloatNote(); });

  async function refresh() {
    try { events = await invoke("events_list"); } catch { events = []; }
    // a set's floats (dozens a set) are listed only when asked for; they're always on the chart
    list.setItems(prefs.floats ? events : events.filter((e) => e.kind !== "float"));
    renderFloatNote();
    renderHaul();
    renderOpen();
  }
  listen("events-changed", () => refresh());
  listen("library-changed", () => refresh());
  renderButtons();
  renderTimer();
  refresh();
  return { refresh, events: () => events, sharedActive, shoot, openHaul };
}
