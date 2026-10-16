// Rhumb-line bearing and distance (straight lines on the Mercator chart) and
// the formats used for them by the ruler and the cursor readouts.

const R_NM = 3440.065;
const rad = (d) => (d * Math.PI) / 180;

/** Rhumb line from a to b ([lon, lat]): { brg (°T), nm }. */
export function rhumb(a, b) {
  const p1 = rad(a[1]), p2 = rad(b[1]);
  let dl = rad(b[0] - a[0]);
  if (Math.abs(dl) > Math.PI) dl = dl > 0 ? dl - 2 * Math.PI : dl + 2 * Math.PI; // the short way round
  const dpsi = Math.log(Math.tan(Math.PI / 4 + p2 / 2) / Math.tan(Math.PI / 4 + p1 / 2));
  const dp = p2 - p1;
  const q = Math.abs(dpsi) > 1e-12 ? dp / dpsi : Math.cos(p1);
  const nm = Math.sqrt(dp * dp + q * q * dl * dl) * R_NM;
  const brg = ((Math.atan2(dl, dpsi) * 180) / Math.PI + 360) % 360;
  return { brg, nm };
}

/** "045" */
export const fmtBrg = (b) => String(Math.round(((b % 360) + 360) % 360) % 360).padStart(3, "0");

/** "0.42 NM (780 m)", "3.27 NM", "128.4 NM" */
export function fmtNm(nm) {
  if (nm < 0.2) return `${nm.toFixed(2)} NM (${Math.round(nm * 1852)} m)`;
  return nm < 10 ? `${nm.toFixed(2)} NM` : `${nm.toFixed(1)} NM`;
}

/** Degrees and decimal minutes: 41°15.974'S */
export function ddm(deg, pos, neg) {
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}
