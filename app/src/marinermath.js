// Sums for the mariner's symbols (mariner.js): range ring geometry and spacing, display
// scale, the S-52 scale bar, vector time marks. No map or page here, so they can be tested.

const R_NM = 3440.065; // mean Earth radius in nautical miles
const rad = (d) => (d * Math.PI) / 180;
const deg = (r) => (r * 180) / Math.PI;

/** Great-circle destination from c ([lon, lat]) on a bearing (°T) for nm. */
export function destination(c, brg, nm) {
  const d = nm / R_NM, t = rad(brg), p1 = rad(c[1]), l1 = rad(c[0]);
  const p2 = Math.asin(Math.sin(p1) * Math.cos(d) + Math.cos(p1) * Math.sin(d) * Math.cos(t));
  const l2 = l1 + Math.atan2(Math.sin(t) * Math.sin(d) * Math.cos(p1), Math.cos(d) - Math.sin(p1) * Math.sin(p2));
  // keep the longitude next to the centre's, so a ring across ±180° draws as one line
  let lon = deg(l2);
  while (lon - c[0] > 180) lon -= 360;
  while (lon - c[0] < -180) lon += 360;
  return [lon, deg(p2)];
}

/** A circle of radius nm round c, as a closed list of [lon, lat] (a true circle on the earth). */
export function circle(c, nm, steps = 96) {
  const out = [];
  for (let i = 0; i <= steps; i++) out.push(destination(c, (i / steps) * 360, nm));
  return out;
}

/** A range ring spacing typed by the mariner, in NM: "0.25", ".5", "1/4", "1 1/2", "¼", "½ NM".
 *  null when it isn't a spacing (0, negative, more than 100 NM, or not a number). */
export function parseSpacing(s) {
  let t = String(s ?? "").trim().toLowerCase().replace(/\s*(nm|nmi|m(iles?)?)$/, "").replace(",", ".");
  const vulgar = { "¼": 0.25, "½": 0.5, "¾": 0.75, "⅛": 0.125, "⅓": 1 / 3, "⅔": 2 / 3, "⅕": 0.2, "⅒": 0.1 };
  let whole = 0;
  const v = /^(\d+)?\s*([¼½¾⅛⅓⅔⅕⅒])$/.exec(t);
  if (v) return check((Number(v[1]) || 0) + vulgar[v[2]]);
  const mixed = /^(\d+)\s+(\d+)\s*\/\s*(\d+)$/.exec(t);
  if (mixed) { whole = Number(mixed[1]); t = `${mixed[2]}/${mixed[3]}`; }
  const f = /^(\d*\.?\d+)\s*\/\s*(\d*\.?\d+)$/.exec(t);
  if (f) return check(whole + Number(f[1]) / Number(f[2]));
  if (!/^(\d+\.?\d*|\.\d+)$/.test(t)) return null;
  return check(Number(t));
  function check(x) { return Number.isFinite(x) && x > 0 && x <= 100 ? x : null; }
}

/** A range for a ring's label: "0.25", "1.5", "10" (no trailing zeros). */
export function fmtRange(nm) {
  const r = Math.round(nm * 1000) / 1000;
  return String(r);
}

/** The spacing as the mariner would write it: common fractions of a mile as fractions. */
export function fmtSpacing(nm) {
  const f = { 0.125: "1/8", 0.25: "1/4", 0.5: "1/2", 0.75: "3/4" };
  return f[Math.round(nm * 1000) / 1000] ?? fmtRange(nm);
}

/** Metres per CSS pixel at a latitude and MapLibre zoom (512-px tiles). */
export function metresPerPixel(lat, zoom) {
  return (40075016.686 * Math.cos(rad(lat))) / (512 * 2 ** zoom);
}

/** The display scale's denominator (1 : n) on a screen with pxPerMm CSS pixels per millimetre. */
export function displayScale(lat, zoom, pxPerMm) {
  return metresPerPixel(lat, zoom) * pxPerMm * 1000;
}

/** S-52 scale bar (PresLib 8.5.3): SCALEB10, a one-mile bar in tenths, at display scales larger
 *  than 1:80,000; SCALEB11, a ten-mile latitude scale in miles, at smaller scales. Where a mile
 *  would be taller than `maxPx` (zoomed right in), a tenth of a mile in hundredths.
 *  { symbol, nm, segments, px } (px: the bar's length on the screen). */
export function scaleBar(lat, zoom, pxPerMm, maxPx) {
  const mpp = metresPerPixel(lat, zoom);
  const n = displayScale(lat, zoom, pxPerMm);
  const pxOf = (nm) => (nm * 1852) / mpp;
  if (n < 80000) {
    let nm = 1;
    while (pxOf(nm) > maxPx && nm > 0.002) nm /= 10;
    return { symbol: "SCALEB10", nm: Math.round(nm * 1000) / 1000, segments: 10, px: pxOf(nm) };
  }
  return { symbol: "SCALEB11", nm: 10, segments: 10, px: pxOf(10) };
}

/** Time marks along a vector of `minutes` at sog knots: [{ nm, big }] (S-52 VECMRK01 every
 *  minute, VECMRK02 every six). Too many to see (more than `most`): the six-minute ones only. */
export function vectorMarks(minutes, sog, most = 60) {
  if (!(sog > 0) || !(minutes > 0)) return [];
  const out = [];
  const every = minutes > most ? 6 : 1;
  for (let m = every; m <= minutes + 1e-9 && out.length < most; m += every) out.push({ nm: (sog * m) / 60, big: m % 6 === 0 });
  return out;
}

/** Past-track time marks: the first fix at or after each whole `everyMin` minutes.
 *  times in Unix seconds, alongside the points. [{ i, t, label }] (label on each `labelMin`). */
export function trackMarks(times, everyMin = 10, labelMin = 30) {
  const out = [];
  let next = null;
  for (let i = 0; i < times.length; i++) {
    const t = times[i];
    if (!Number.isFinite(t)) continue;
    if (next === null) next = Math.ceil(t / (everyMin * 60)) * everyMin * 60;
    if (t >= next) {
      const slot = Math.floor(t / (everyMin * 60)) * everyMin * 60;
      out.push({ i, t: slot, label: slot % (labelMin * 60) === 0 });
      next = slot + everyMin * 60;
    }
  }
  return out;
}
