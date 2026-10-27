// Library > Trips: start and end a trip with one large button (the recorder starts with it),
// add past trips from their dates, and see each trip's summary: time out, distance, marks
// and tracks made. A trip holds what was made while it lasted (tripstore.js), so Show
// filters the Marks and Tracks lists to it and draws its tracks.

import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
import { libList } from "./liblist.js";
import { setTrips, getTrips, duration } from "./tripstore.js";
import { exportLibraryFile } from "./library.js";

const $ = (id) => document.getElementById(id);
const now = () => Math.floor(Date.now() / 1000);
const day = (t) => new Date(t * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
const time = (t) => new Date(t * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
// datetime-local works in local time
const toLocal = (t) => { if (!t) return ""; const d = new Date(t * 1000); d.setMinutes(d.getMinutes() - d.getTimezoneOffset()); return d.toISOString().slice(0, 16); };
const fromLocal = (v) => (v ? Math.floor(new Date(v).getTime() / 1000) : null);

export function setupTrips(map, { ui, library, tracks }) {
  let trips = [];
  let editing = null;
  const collate = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" }).compare;

  function summary(t) {
    const end = t.ended ?? now();
    return [
      t.ended ? `${day(t.started)} – ${day(t.ended)}` : `since ${day(t.started)} ${time(t.started)}`,
      duration(end - t.started),
      t.distance_nm ? `${t.distance_nm.toFixed(0)} NM` : "",
      t.marks ? `${t.marks.toLocaleString()} mark${t.marks === 1 ? "" : "s"}` : "",
      t.tracks ? `${t.tracks.toLocaleString()} track${t.tracks === 1 ? "" : "s"}` : "",
    ].filter(Boolean).join(" · ");
  }

  const list = libList($("lib-trips-list"), {
    key: "trips",
    noun: ["trip", "trips"],
    search: (t, q) => `${t.name} ${t.number} ${t.fishery} ${t.crew} ${t.notes}`.toLowerCase().includes(q),
    sorts: [
      { key: "new", label: "Newest first", cmp: (a, b) => b.started - a.started },
      { key: "old", label: "Oldest first", cmp: (a, b) => a.started - b.started },
      { key: "name", label: "By name", cmp: (a, b) => collate(a.name, b.name) },
      { key: "long", label: "Longest first", cmp: (a, b) => b.distance_nm - a.distance_nm },
    ],
    filters: [
      { key: "fishery", label: "Fishery", type: "choice", options: (ts) => { const c = new Map(); for (const t of ts) if (t.fishery) c.set(t.fishery, (c.get(t.fishery) || 0) + 1); return [...c].sort((a, b) => b[1] - a[1]).map(([k, n]) => [k, k, n]); }, test: (t, v) => t.fishery === v },
    ],
    row: {
      render(t, el) {
        el.innerHTML = `<div class="main" title="Show this trip's marks and tracks"><b></b><small></small></div><button data-act="show">Show</button><button data-act="edit">Edit</button>`;
        el.querySelector("b").textContent = `${t.name}${t.fishery ? ` · ${t.fishery}` : ""}${t.ended ? "" : " (on now)"}`;
        el.querySelector("small").textContent = summary(t);
        el.querySelector('[data-act="show"]').onclick = () => show(t);
        el.querySelector('[data-act="edit"]').onclick = () => edit(t);
      },
      click: (t) => show(t),
    },
    actions: [
      { label: "Library file…", title: "Write the selected trips' marks and tracks as a library file of their own", run: async (ts) => {
        const marks = [], trks = [];
        for (const t of ts) { const c = await invoke("trip_contents", { id: t.id }); marks.push(...c.marks); trks.push(...c.tracks); }
        exportLibraryFile({ marks, tracks: trks }, ts.length === 1 ? ts[0].name : `${ts.length} trips`, ts.length === 1 ? ts[0].name : "trips");
      } },
      { label: "Delete", danger: true, title: "Delete the selected trips (their marks and tracks stay)",
        confirm: (ts) => `Delete ${ts.length === 1 ? `“${ts[0].name}”` : `${ts.length} trips`}? Their marks and tracks stay in the library.`,
        run: async (ts) => {
          try { const n = await invoke("trips_delete", { ids: ts.map((t) => t.id) }); activity.note(`Deleted ${n} trip${n === 1 ? "" : "s"}`, { details: ["Library > Trash brings them back."] }); } catch (e) { activity.start("Deleting trips").fail(String(e)); }
          refresh();
        } },
    ],
    empty: () => "No trips yet. Start one when you leave, or add a past trip from its dates.",
  });

  // ---------- the trip on now: one large button ----------
  let ticker = 0;
  function renderNow() {
    const box = $("trip-now");
    const on = trips.find((t) => !t.ended);
    clearInterval(ticker);
    if (!on) {
      box.innerHTML = `<button class="trip-big on" data-act="start" title="Start a trip now: what you make from now on goes into it, and the track recorder starts">Start a trip</button>
        <button data-act="past" class="trip-past" title="A trip already made: give its dates, and the marks and tracks made then fall into it">Add a past trip…</button>`;
      box.querySelector('[data-act="start"]').onclick = () => edit(null, true);
      box.querySelector('[data-act="past"]').onclick = () => edit(null, false);
      return;
    }
    box.innerHTML = `<div class="trip-on"><b></b><small></small></div>
      <button class="trip-big end" data-act="end" title="End the trip now (Edit can change when it ended)">End the trip</button>
      <div class="wx-row"><label class="grow" title="Stop the track recorder when the trip ends"><input type="checkbox" data-act="stoprec" checked /> Stop recording the track</label><button data-act="edit">Edit</button></div>`;
    const fill = () => {
      box.querySelector("b").textContent = `On trip: ${on.name}${on.fishery ? ` · ${on.fishery}` : ""}`;
      box.querySelector("small").textContent = summary(on);
    };
    fill();
    ticker = setInterval(fill, 60000);
    box.querySelector('[data-act="edit"]').onclick = () => edit(on);
    box.querySelector('[data-act="end"]').onclick = async () => {
      try {
        const t = await invoke("trip_end", { id: on.id, stopRecording: box.querySelector('[data-act="stoprec"]').checked });
        activity.note(`${t.name} ended`, { details: [summary(t)] });
      } catch (e) { activity.start("Ending the trip").fail(String(e)); }
      refresh();
    };
  }

  // ---------- the editor: a new trip (now or past), or changing one ----------
  const form = $("trip-edit");
  function edit(t, startNow = false) {
    editing = t;
    form.hidden = false;
    $("trip-edit-title").textContent = t ? `Edit ${t.name}` : startNow ? "Start a trip" : "A past trip";
    const n = (trips.length ? Math.max(...trips.map((x) => Number(x.number) || 0)) + 1 : 1);
    const last = trips[0];
    form.name.value = t?.name ?? "";
    form.number.value = t?.number ?? (startNow ? String(n) : "");
    form.fishery.value = t?.fishery ?? last?.fishery ?? "";
    form.crew.value = t?.crew ?? last?.crew ?? "";
    form.notes.value = t?.notes ?? "";
    form.started.value = toLocal(t?.started ?? (startNow ? now() : null));
    form.ended.value = toLocal(t?.ended);
    form.ended.closest(".wx-row").hidden = !t && startNow;
    form.querySelector(".trip-rec").hidden = !!t || !startNow;
    form.querySelector('[data-act="delete"]').hidden = !t;
    form.querySelector('button[type="submit"]').textContent = !t && startNow ? "Start the trip" : "Save";
    $("trip-now").hidden = true;
    $("trip-fisheries").innerHTML = [...new Set(trips.map((x) => x.fishery).filter(Boolean))].map((f) => `<option value="${f.replace(/"/g, "&quot;")}">`).join("");
    (startNow ? form.querySelector('button[type="submit"]') : form.name).focus();
  }
  function close() { form.hidden = true; editing = null; $("trip-now").hidden = false; }
  form.querySelector('[data-act="cancel"]').onclick = close;
  form.querySelector('[data-act="delete"]').onclick = async () => {
    if (!editing) return;
    try { await invoke("trips_delete", { ids: [editing.id] }); activity.note(`Deleted ${editing.name}`, { details: ["Its marks and tracks stay. Library > Trash brings the trip back."] }); } catch (e) { activity.start("Deleting the trip").fail(String(e)); }
    close(); refresh();
  };
  form.onsubmit = async (e) => {
    e.preventDefault();
    const started = fromLocal(form.started.value);
    const ended = fromLocal(form.ended.value);
    if (!editing && !started) { form.started.focus(); return; }
    const trip = { ...(editing ?? {}), id: editing?.id ?? 0, name: form.name.value, number: form.number.value, fishery: form.fishery.value, crew: form.crew.value, notes: form.notes.value, started, ended };
    const record = !editing && !ended && form.record.checked;
    try {
      const t = await invoke("trip_save", { trip, record });
      activity.note(editing ? `${t.name} saved` : t.ended ? `${t.name} added` : `${t.name} started`, { details: [summary(t), ...(record ? ["The track recorder is on."] : [])] });
      close();
    } catch (err) { activity.start(editing ? "Saving the trip" : "Starting a trip").fail(String(err)); }
    refresh();
  };

  // ---------- Show: the trip's marks and tracks, and the chart moved to them ----------
  function show(t) {
    const ms = library.showTrip(t);
    const ts = tracks.showTrip(t);
    let w = 180, s = 90, e = -180, n = -90;
    const add = (lon, lat) => { w = Math.min(w, lon); e = Math.max(e, lon); s = Math.min(s, lat); n = Math.max(n, lat); };
    for (const m of ms) add(m.lon, m.lat);
    for (const tr of ts) { const b = tr.stats?.bbox; if (b) { add(b[0], b[1]); add(b[2], b[3]); } }
    if (w <= e) map.fitBounds([[w, s], [e, n]], { padding: 60, maxZoom: 13, duration: 600 });
    activity.note(`Showing ${t.name}`, { details: [
      `${ms.length.toLocaleString()} marks and ${ts.length.toLocaleString()} tracks made during it.`,
      ...(ts.length > 200 ? ["Too many tracks to draw at once: pick some in the Tracks tab."] : []),
      "The Marks and Tracks lists are filtered to the trip, and the chart shows only its marks: Clear filters in Marks to see all again.",
    ] });
  }

  async function refresh() {
    try { trips = await invoke("trips_list"); } catch { trips = []; }
    setTrips(trips);
    list.setItems(trips);
    renderNow();
  }
  refresh();
  return { refresh, current: () => getTrips().find((t) => !t.ended) ?? null };
}
