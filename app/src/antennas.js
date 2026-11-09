// Which GPS antenna the position is coming from, and so where it is on the boat.
//
// Settings › Vessel › Position and GPS antenna has a default antenna position, and one per
// receiver when there is more than one: each NMEA input, or each receiver told apart on one input
// (two GPS mixed on a multiplexer, see marine_nmea::fixstreams). The position's input comes with
// every vessel update (groupSources.position); the receiver in use on that input comes with its
// status (gpsStreams). The bow (anchor alarm, anchor drop) and the hull outline use this antenna.
import { listen } from "@tauri-apps/api/event";

export function setupAntennas() {
  const inUse = new Map(); // input id → the key of the receiver in use, with two or more on it
  let positionSource = null;
  listen("nmea-status", ({ payload }) => {
    if (!payload?.id) return;
    const streams = payload.gpsStreams || [];
    inUse.set(payload.id, streams.length > 1 ? streams.find((s) => s.inUse)?.key ?? null : null);
  }).catch(() => {});
  listen("vessel", ({ payload }) => {
    if (payload?.replay) return;
    positionSource = payload?.groupSources?.position ?? null;
  }).catch(() => {});

  return {
    /** The antenna in use: { fromBow, fromCentre, label } (m; label empty for the default). */
    offsets(b) {
      const list = b.gpsAntennas || [];
      let a = null;
      if (positionSource) {
        const key = inUse.get(positionSource) || "";
        a = list.find((x) => x.sourceId === positionSource && (x.stream || "") === key)
          ?? list.find((x) => x.sourceId === positionSource && !x.stream);
      }
      return {
        fromBow: Number(a?.fromBowM ?? b.gpsFromBow) || 0,
        fromCentre: Number(a?.fromCenterlineM ?? b.gpsFromCentre) || 0,
        label: a?.label || "",
      };
    },
  };
}
