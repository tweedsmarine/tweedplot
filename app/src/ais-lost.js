// When an AIS target counts as lost (IEC 62288 lost-target symbol): six of its nominal reporting
// intervals (ITU-R M.1371) without a position report, as shipborne radars and ECDIS do
// (IEC 62388: Class A at anchor 18 min, 0–14 kn 60 s, 14–23 kn 36 s, over 23 kn 12 s).
// Targets heard only over the internet arrive throttled by the service, so they keep the
// older, longer allowance.

const ANCHORED = new Set([1, 5]); // navigational status: at anchor, moored

/** The nominal reporting interval (s) for what the target last reported. */
export function reportInterval(t) {
  if (["sart", "mob", "epirb"].includes(t.kind)) return 60;
  if (t.kind === "aton") return 180;
  if (t.kind === "base" || t.kind === "aircraft") return 10;
  const sog = t.sog ?? 0;
  if (t.class === "B") return sog > 2 ? 30 : 180; // Class B "CS"
  if (t.class === "B+") return sog <= 2 ? 180 : sog <= 14 ? 30 : sog <= 23 ? 15 : 5; // Class B "SO"
  // Class A (or not known)
  if (ANCHORED.has(t.status) && sog <= 3) return 180;
  return sog <= 14 ? 10 : sog <= 23 ? 6 : 2;
}

/** Seconds without a position report after which the target is lost. */
export function lostAfter(t, { internetOnly = false } = {}) {
  if (internetOnly) {
    if (["sart", "mob", "epirb"].includes(t.kind)) return 6 * 60;
    return t.kind === "aton" || t.kind === "base" ? 20 * 60 : (t.sog ?? 0) > 2 ? 3 * 60 : 10 * 60;
  }
  return 6 * reportInterval(t);
}

/** Lost: never had a position, or not heard for longer than lostAfter. `now` in Unix seconds. */
export function targetLost(t, now, opts) {
  if (t.last_pos == null) return true;
  return now - t.last_pos > lostAfter(t, opts);
}
