// Rhumb-line bearing and distance (straight lines on the Mercator chart) and
// the formats used for them by the ruler and the cursor readouts.

const R_NM = 3440.065;
const rad = (d) => (d * Math.PI) / 180;

/** Great-circle distance and initial true bearing from a to b ([lon, lat]). */
export function greatCircle(a, b) {
  const [l1, p1] = [rad(a[0]), rad(a[1])], [l2, p2] = [rad(b[0]), rad(b[1])];
  const y = Math.sin(l2 - l1) * Math.cos(p2);
  const x = Math.cos(p1) * Math.sin(p2) - Math.sin(p1) * Math.cos(p2) * Math.cos(l2 - l1);
  const h = Math.sin((p2 - p1) / 2) ** 2 + Math.cos(p1) * Math.cos(p2) * Math.sin((l2 - l1) / 2) ** 2;
  return {
    brg: ((Math.atan2(y, x) * 180) / Math.PI + 360) % 360,
    nm: 2 * Math.asin(Math.min(1, Math.sqrt(h))) * R_NM,
  };
}

/** Sample a great-circle arc at one-degree intervals, keeping longitude continuous across the dateline. */
export function greatCirclePath(a, b) {
  const toVector = ([lon, lat]) => {
    const l = rad(lon), p = rad(lat), c = Math.cos(p);
    return [c * Math.cos(l), c * Math.sin(l), Math.sin(p)];
  };
  const start = toVector(a), end = toVector(b);
  const dot = Math.max(-1, Math.min(1, start.reduce((sum, value, i) => sum + value * end[i], 0)));
  const angle = Math.acos(dot);
  const segments = Math.max(1, Math.ceil(angle / rad(1)));
  const sinAngle = Math.sin(angle);
  let orthogonal = null;
  if (Math.abs(sinAngle) < 1e-8 && angle > 1e-8) {
    const basis = Math.abs(start[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
    const cross = [
      start[1] * basis[2] - start[2] * basis[1],
      start[2] * basis[0] - start[0] * basis[2],
      start[0] * basis[1] - start[1] * basis[0],
    ];
    const length = Math.hypot(...cross);
    orthogonal = cross.map((value) => value / length);
  }
  const coords = [a];
  for (let i = 1; i < segments; i++) {
    const t = i / segments;
    let vector;
    if (orthogonal) {
      vector = start.map((value, k) => value * Math.cos(angle * t) + orthogonal[k] * Math.sin(angle * t));
    } else if (angle < 1e-8) {
      vector = start.map((value, k) => value * (1 - t) + end[k] * t);
    } else {
      const from = Math.sin((1 - t) * angle) / sinAngle;
      const to = Math.sin(t * angle) / sinAngle;
      vector = start.map((value, k) => value * from + end[k] * to);
    }
    const lon = (Math.atan2(vector[1], vector[0]) * 180) / Math.PI;
    const lat = (Math.atan2(vector[2], Math.hypot(vector[0], vector[1])) * 180) / Math.PI;
    const previousLon = coords.at(-1)[0];
    coords.push([lon + 360 * Math.round((previousLon - lon) / 360), lat]);
  }
  const previousLon = coords.at(-1)[0];
  coords.push([b[0] + 360 * Math.round((previousLon - b[0]) / 360), b[1]]);
  return coords;
}

/** GeoJSON great-circle geometry split at the antimeridian. */
export function greatCircleGeometry(a, b) {
  const path = greatCirclePath(a, b);
  const wrap = (lon) => ((((lon + 180) % 360) + 360) % 360) - 180;
  let startLon = wrap(path[0][0]);
  if (startLon === -180 && path[1]?.[0] < path[0][0]) startLon = 180;
  const lines = [[[startLon, path[0][1]]]];
  for (let i = 1; i < path.length; i++) {
    const [previousLon, previousLat] = path[i - 1];
    const [longitude, latitude] = path[i];
    const increasing = longitude > previousLon && Math.floor((longitude + 180) / 360) > Math.floor((previousLon + 180) / 360);
    const decreasing = longitude < previousLon && Math.ceil((longitude - 180) / 360) < Math.ceil((previousLon - 180) / 360);
    if (increasing || decreasing) {
      const boundary = increasing
        ? 180 + 360 * Math.floor((previousLon + 180) / 360)
        : -180 + 360 * Math.ceil((previousLon - 180) / 360);
      const fraction = (boundary - previousLon) / (longitude - previousLon);
      const crossingLat = previousLat + fraction * (latitude - previousLat);
      lines.at(-1).push([increasing ? 180 : -180, crossingLat]);
      lines.push([[increasing ? -180 : 180, crossingLat]]);
      if (fraction >= 1 - 1e-12) continue;
    }
    lines.at(-1).push([wrap(longitude), latitude]);
  }
  return lines.length === 1
    ? { type: "LineString", coordinates: lines[0] }
    : { type: "MultiLineString", coordinates: lines };
}

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
  if (pos === "E" && Number.isFinite(deg)) deg = ((((deg + 180) % 360) + 360) % 360) - 180; // map past ±180
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}
