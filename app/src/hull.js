// A ship's outline at its real size on the chart, placed by its GPS antenna: for our own
// boat (Settings) and for AIS targets (their reported dimensions), at harbour scales.

/** Metres per screen pixel at a latitude and zoom (MapLibre's 512-pixel tiles). */
export function metresPerPixel(lat, zoom) {
  return (78271.517 * Math.cos((lat * Math.PI) / 180)) / 2 ** zoom;
}

/** Whether a ship `length` m long is worth drawing to scale: at least `minPx` on screen. */
export function bigEnough(length, lat, zoom, minPx = 18) {
  return length > 0 && length / metresPerPixel(lat, zoom) >= minPx;
}

/**
 * The outline as a closed ring of [lon, lat]: a pointed bow, parallel sides and a square
 * stern. `pos` is where the GPS antenna is; `fromBow` m aft of the bow and `fromCentre` m
 * to starboard of the centre line (negative: to port). Heading in degrees true.
 */
export function hullRing(pos, heading, { length, beam, fromBow = null, fromCentre = 0 }) {
  const L = length, B = beam || length / 3;
  const fb = fromBow ?? L / 2, fc = fromCentre ?? 0;
  const s = 0.2 * L; // where the sides start, aft of the bow
  // ship frame: x to starboard, y forward, from the bow tip
  const pts = [[0, 0], [B * 0.28, -0.06 * L], [B * 0.44, -0.13 * L], [B / 2, -s], [B / 2, -L], [-B / 2, -L], [-B / 2, -s], [-B * 0.44, -0.13 * L], [-B * 0.28, -0.06 * L], [0, 0]];
  const th = ((heading ?? 0) * Math.PI) / 180, c = Math.cos(th), sn = Math.sin(th);
  const mLat = 111320, mLon = 111320 * Math.cos((pos[1] * Math.PI) / 180);
  return pts.map(([x, y]) => {
    const dx = x - fc, dy = y + fb; // from the antenna
    const e = dx * c + dy * sn, n = -dx * sn + dy * c;
    return [pos[0] + e / mLon, pos[1] + n / mLat];
  });
}
