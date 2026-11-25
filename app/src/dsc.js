// DSC calls from the VHF (NMEA 0183 DSC and DSE, decoded in the back end: marine-nmea dsc.rs):
// a distress alert, relay or acknowledgement raises an alarm and puts the caller on the chart
// (its position as sent, refined by a DSE when one follows); urgency and safety calls raise a
// warning or caution. Clicking the alarm shows the position. The caller is named from AIS when
// a target with that MMSI is known.
import { listen } from "@tauri-apps/api/event";

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

export function setupDsc(map, { getAlarms, aisName = () => null, toast = () => {} }) {
  const calls = new Map(); // mmsi → { call, at }
  function layers() {
    if (map.getSource("dsc-calls")) return;
    map.addSource("dsc-calls", { type: "geojson", data: EMPTY });
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
      .map(({ call, label }) => ({ type: "Feature", properties: { p: call.priority, name: label }, geometry: { type: "Point", coordinates: [call.lon, call.lat] } })) });
  }
  function show(call) {
    if (call.lat == null || call.lon == null) { toast("This DSC call gave no position", 4000); return; }
    map.flyTo({ center: [call.lon, call.lat], zoom: Math.max(map.getZoom(), 11) });
  }

  listen("dsc", ({ payload }) => {
    const call = payload?.call;
    if (!call?.mmsi) return;
    const name = aisName(call.mmsi);
    const { text, detail } = describe(call, name);
    const id = `dsc-${call.priority}-${call.mmsi}`;
    calls.set(call.mmsi, { call, label: `DSC ${call.priority} ${name || call.mmsi}`, at: Date.now() });
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
      onClear: () => { calls.delete(call.mmsi); try { draw(); } catch { /* ignore */ } },
      onClick: () => show(call),
    });
  }).catch(() => {});

  return { calls: () => [...calls.values()].map((c) => c.call) };
}
