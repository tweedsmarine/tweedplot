// DSC calls from the VHF (NMEA 0183 DSC and DSE, decoded in the back end: marine-nmea dsc.rs):
// a distress alert, relay or acknowledgement raises an alarm and puts the caller on the chart
// (its position as sent, refined by a DSE when one follows); urgency and safety calls raise a
// warning or caution. Clicking the alarm, or the call's mark on the chart, shows the position and
// an info box like an AIS target's: what was called, by whom, where, when, and the calls heard
// from that MMSI. The caller is named from AIS when a target with that MMSI is known.
import { listen } from "@tauri-apps/api/event";
import { createInfoBox } from "./infobox.js";
import { distNm, bearing } from "./instruments.js";

const EMPTY = { type: "FeatureCollection", features: [] };
const LEVEL = { distress: "alarm", urgency: "warn", safety: "caution" };

const ddm = (v, pos, neg) => {
  const a = Math.abs(v), d = Math.floor(a), m = (a - d) * 60;
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${v >= 0 ? pos : neg}`;
};

/** The alarm's words for a call (exported for tests). */
export function describe(call, name) {
  const who = name ? `${name} (${call.mmsi})` : `MMSI ${call.mmsi}`;
  const kind = call.priority === "distress"
    ? call.kind === "relay" ? "DSC DISTRESS RELAY" : call.kind === "acknowledgement" ? "DSC DISTRESS ACKNOWLEDGED" : "DSC DISTRESS"
    : call.priority === "urgency" ? "DSC URGENCY" : "DSC SAFETY";
  const where = call.lat != null && call.lon != null ? `${ddm(call.lat, "N", "S")} ${ddm(call.lon, "E", "W")}` : "position not given";
  const parts = [call.nature, where, call.timeUtc ? `${call.timeUtc} UTC` : null, call.fromMmsi ? `relayed by ${call.fromMmsi}` : null].filter(Boolean);
  return { text: `${kind}: ${who}`, detail: parts.join(" · ") };
}

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const KIND = (c) => c.priority === "distress"
  ? { alert: "Distress alert", relay: "Distress relay", acknowledgement: "Distress acknowledgement" }[c.kind] ?? "Distress"
  : c.priority === "urgency" ? "Urgency call (all ships)" : "Safety call (all ships)";
const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const ago = (ms) => { const s = Math.round((Date.now() - ms) / 1000); return s < 60 ? `${s} s ago` : s < 3600 ? `${Math.round(s / 60)} min ago` : `${(s / 3600).toFixed(1)} h ago`; };

/** The info box's rows for a caller's latest call and the calls heard from it (exported for tests). */
export function describeBox(entry, { name = null, own = null } = {}) {
  const { call, at, source, refined, history = [] } = entry;
  const rows = [];
  const add = (k, v) => { if (v != null && v !== "") rows.push(`<tr><th>${esc(k)}</th><td>${esc(v)}</td></tr>`); };
  add("Call", KIND(call));
  if (call.nature) add("Nature of distress", call.nature);
  add(call.kind === "relay" || call.kind === "acknowledgement" ? "Vessel in distress" : "Caller", `${name ? `${name} · ` : ""}MMSI ${call.mmsi}`);
  if (call.fromMmsi) add(call.kind === "acknowledgement" ? "Acknowledged by" : "Relayed by", `MMSI ${call.fromMmsi}${/^00/.test(call.fromMmsi) ? " (coast station)" : ""}`);
  if (call.lat != null && call.lon != null) {
    add("Position", `${ddm(call.lat, "N", "S")} ${ddm(call.lon, "E", "W")}${refined ? " (enhanced, from the DSE)" : " (to the nearest minute)"}`);
    if (own) add("Range / bearing", `${distNm(own, [call.lon, call.lat]).toFixed(2)} NM / ${String(Math.round(bearing(own, [call.lon, call.lat]))).padStart(3, "0")}°T`);
  } else add("Position", "not given");
  add("Time of position", call.timeUtc ? `${call.timeUtc} UTC (as sent)` : call.lat != null ? "not given" : null);
  add("Received", `${clock(at)} (${ago(at)})`);
  add("Heard on", source);
  const list = history.length > 1
    ? `<p class="wx-note">Calls heard from this MMSI:</p><ul class="dsc-hist">${history.slice().reverse().map((h) => `<li>${esc(clock(h.at))} · ${esc(KIND(h.call))}${h.call.nature ? ` · ${esc(h.call.nature)}` : ""}</li>`).join("")}</ul>`
    : "";
  const note = call.priority === "distress" ? `<p class="wx-note">Listen on VHF channel 16 before responding.</p>` : "";
  return `<div class="ais-pop"><table>${rows.join("")}</table>${note}${list}<div class="ais-pop-btns"><button data-act="dsc-clear">Clear this call</button></div></div>`;
}

export function setupDsc(map, { getAlarms, aisName = () => null, toast = () => {}, getOwn = () => null }) {
  const calls = new Map(); // mmsi → { call, at, source, refined, history, label }
  const box = createInfoBox(map, "dsc", { width: 330, height: 360, showRing: false });
  let boxed = null; // the MMSI whose box is open
  function layers() {
    if (map.getSource("dsc-calls")) return;
    map.addSource("dsc-calls", { type: "geojson", data: EMPTY });
    hook();
    map.addLayer({ id: "dsc-ring", type: "circle", source: "dsc-calls",
      paint: { "circle-radius": 14, "circle-color": "rgba(0,0,0,0)", "circle-stroke-width": 3,
        "circle-stroke-color": ["match", ["get", "p"], "distress", "#d0021b", "urgency", "#f57c00", "#c9a400"] } });
    map.addLayer({ id: "dsc-dot", type: "circle", source: "dsc-calls",
      paint: { "circle-radius": 5, "circle-color": ["match", ["get", "p"], "distress", "#d0021b", "urgency", "#f57c00", "#c9a400"],
        "circle-stroke-width": 1.5, "circle-stroke-color": "#ffffff" } });
    map.addLayer({ id: "dsc-label", type: "symbol", source: "dsc-calls",
      layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 18], "icon-allow-overlap": true } });
  }
  function draw() {
    if (!map.getSource("dsc-calls")) layers();
    map.getSource("dsc-calls").setData({ type: "FeatureCollection", features: [...calls.values()]
      .filter(({ call }) => call.lat != null && call.lon != null)
      .map(({ call, label }) => ({ type: "Feature", properties: { p: call.priority, name: label, mmsi: call.mmsi }, geometry: { type: "Point", coordinates: [call.lon, call.lat] } })) });
  }
  const alarmId = (call) => `dsc-${call.priority}-${call.mmsi}`;
  function forget(mmsi) {
    const e = calls.get(mmsi);
    if (!e) return;
    calls.delete(mmsi);
    for (const p of new Set(e.history.map((h) => h.call.priority))) getAlarms()?.clear(`dsc-${p}-${mmsi}`);
    if (boxed === mmsi) box.close?.();
    try { draw(); } catch { /* ignore */ }
  }
  function content(mmsi) {
    const e = calls.get(mmsi);
    return describeBox(e, { name: aisName(mmsi), own: getOwn() });
  }
  function wire(mmsi) {
    box.body?.querySelector('[data-act="dsc-clear"]')?.addEventListener("click", () => forget(mmsi));
  }
  const title = (e) => `DSC ${e.call.priority}: ${aisName(e.call.mmsi) || `MMSI ${e.call.mmsi}`}`;
  function openBox(mmsi) {
    const e = calls.get(mmsi);
    if (!e) return;
    boxed = mmsi;
    box.open(content(mmsi), { title: title(e), at: e.call.lat != null ? [e.call.lon, e.call.lat] : null, onClose: () => { if (boxed === mmsi) boxed = null; } });
    wire(mmsi);
  }
  function refreshBox() {
    const e = boxed != null ? calls.get(boxed) : null;
    if (!box.isOpen || !e) return;
    box.update(content(boxed), { title: title(e), at: e.call.lat != null ? [e.call.lon, e.call.lat] : null });
    wire(boxed);
  }
  setInterval(refreshBox, 5000); // the range, bearing and "ago" move on
  function show(call) {
    openBox(call.mmsi);
    if (call.lat == null || call.lon == null) { toast("This DSC call gave no position", 4000); return; }
    map.flyTo({ center: [call.lon, call.lat], zoom: Math.max(map.getZoom(), 11) });
  }
  function hook() {
    for (const l of ["dsc-ring", "dsc-dot", "dsc-label"]) {
      map.on("click", l, (ev) => { const f = ev.features?.[0]; if (f) openBox(String(f.properties.mmsi)); });
      map.on("mouseenter", l, () => { map.getCanvas().style.cursor = "pointer"; });
      map.on("mouseleave", l, () => { map.getCanvas().style.cursor = ""; });
    }
  }

  listen("dsc", ({ payload }) => {
    const call = payload?.call;
    if (!call?.mmsi) return;
    const name = aisName(call.mmsi);
    const { text, detail } = describe(call, name);
    const id = alarmId(call);
    const prev = calls.get(call.mmsi);
    // a DSE's enhanced position comes as the same call again: it refines the last one, not a new call
    const refines = prev?.call.expansion && !call.expansion && prev.call.kind === call.kind && prev.call.priority === call.priority && prev.call.timeUtc === call.timeUtc;
    const history = (prev?.history ?? []).slice(-19);
    if (refines) history[history.length - 1] = { call, at: prev.at };
    else history.push({ call, at: Date.now() });
    calls.set(call.mmsi, { call, label: `DSC ${call.priority} ${name || call.mmsi}`, at: refines ? prev.at : Date.now(), source: payload.source,
      refined: refines, history });
    try { draw(); } catch { /* the map isn't ready: drawn with the next call */ }
    const alarms = getAlarms();
    alarms?.raise(id, {
      level: LEVEL[call.priority] ?? "warn",
      text,
      detail: `${detail}${call.lat != null ? " · click to show on the chart" : ""}${payload.source ? ` · from ${payload.source}` : ""}`,
      // a call has no lasting cause to end it: it stays until cleared, and a distress alert's
      // watchdog sounds it again if it is silenced and left
      clearable: true,
      watchdog: call.priority === "distress",
      onClear: () => forget(call.mmsi),
      onClick: () => show(call),
    });
    if (boxed === call.mmsi) refreshBox();
  }).catch(() => {});

  return { open: openBox, calls: () => [...calls.values()].map((c) => c.call) };
}
