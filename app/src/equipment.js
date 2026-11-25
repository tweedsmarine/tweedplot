// Alarms from other equipment, in TweedPlot's alert list beside its own:
//   NMEA ALR and ALF (IEC 62923 bridge alert management), decoded in the back end (alerts.rs);
//   Signal K notifications above normal (alert, warn, alarm, emergency);
//   and a GPS that reports a datum other than WGS 84 (NMEA DTM), which offsets it from the charts.
// An alert stays while its equipment says it's active and goes when it says it has cleared; one
// the equipment stops reporting goes after three minutes (ALR and ALF are repeated while active).
// Acknowledging here quiets it here; acknowledge it at the equipment as well.
import { listen } from "@tauri-apps/api/event";

const LEVEL = { alarm: "alarm", warning: "warn", caution: "caution" };
const SK_LEVEL = { emergency: "alarm", alarm: "alarm", warn: "warn", alert: "caution" };
const TALKERS = { SD: "echo sounder", AI: "AIS", GP: "GPS", GN: "GNSS", II: "instruments", IN: "integrated navigation", HC: "compass", HE: "gyro",
  AG: "autopilot", AP: "autopilot", RA: "radar", VW: "speed log", WI: "weather station", CD: "DSC radio", EC: "ECDIS", ER: "engine room" };
const DATUMS = { W72: "WGS 72", S85: "SGS 85", P90: "PE 90", "999": "user defined" };
const STALE_MS = 3 * 60 * 1000;

/** The alarm's words for a Signal K notification path ("notifications.environment.depth.belowKeel"). Exported for tests. */
export function notificationText(path, message) {
  if (message) return message;
  const tail = path.replace(/^notifications\./, "").split(".").pop() || path;
  return tail.replace(/([a-z])([A-Z])/g, "$1 $2").replace(/^./, (c) => c.toUpperCase());
}

export function setupEquipmentAlerts({ getAlarms }) {
  const seen = new Map(); // id → last report (ms)
  const alarms = () => getAlarms();

  listen("equipment-alert", ({ payload }) => {
    const a = payload?.alert;
    if (!a) return;
    const id = `eq:${payload.sourceId}:${payload.talker}:${a.id}`;
    const al = alarms();
    if (!al) return;
    if (!a.active) { seen.delete(id); al.clear(id); return; }
    const wasNew = !seen.has(id);
    seen.set(id, Date.now());
    const from = TALKERS[payload.talker] ? `${TALKERS[payload.talker]} (${payload.talker})` : payload.talker;
    al.raise(id, {
      level: LEVEL[a.priority] ?? "alarm",
      text: a.text.toUpperCase(),
      detail: `From the ${from} on ${payload.source} (${a.id})${a.acked ? " · acknowledged at the equipment" : " · acknowledge it at the equipment too"}`,
    });
    // acknowledged at the equipment: quiet here too (still listed while it lasts)
    if (a.acked && wasNew) al.ack(id);
  }).catch(() => {});

  // reports that stopped: gone after three minutes
  setInterval(() => {
    const now = Date.now();
    for (const [id, at] of seen) if (now - at > STALE_MS) { seen.delete(id); alarms()?.clear(id); }
  }, 10000);

  let skShown = new Map(); // id → "state|message" as last raised
  /** Signal K notifications above normal ({ path: { state, message } }), with each vessel update:
   *  raised when new or changed (not on every update), cleared when they return to normal. */
  function signalK(notifications) {
    const al = alarms();
    if (!al || !notifications) return;
    const now = new Map();
    for (const [path, n] of Object.entries(notifications)) {
      const id = `sk:${path}`, key = `${n.state}|${n.message}`;
      now.set(id, key);
      if (skShown.get(id) === key) continue;
      al.raise(id, { level: SK_LEVEL[n.state] ?? "warn", text: notificationText(path, n.message).toUpperCase(),
        detail: `Signal K notification (${n.state}): ${path.replace(/^notifications\./, "")}` });
    }
    for (const id of skShown.keys()) if (!now.has(id)) al.clear(id);
    skShown = now;
  }

  let datumShown = false, lastDatum = null;
  /** The GPS's datum (NMEA DTM); a warning while it isn't WGS 84 (acted on when it changes). */
  function datum(code) {
    const al = alarms();
    if (!al || code == null || code === lastDatum) return;
    lastDatum = code;
    const off = code !== "W84";
    if (off) {
      al.raise("gps-datum", { level: "warn", text: "GPS DATUM IS NOT WGS 84",
        detail: `The GPS reports datum ${DATUMS[code] ?? code}. The charts are WGS 84, so the boat may be drawn some way from where it is: set the GPS to WGS 84.` });
    } else if (datumShown) al.clear("gps-datum");
    datumShown = off;
  }

  return { signalK, datum };
}
