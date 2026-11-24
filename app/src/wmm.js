// World Magnetic Model 2025 (NOAA/NCEI and BGS, public domain; epoch 2025.0, valid to 2030.0).
// Gives magnetic variation (declination) anywhere on Earth, with no instruments or chart.
// Variation is positive east, so magnetic = true - variation.
// The model degrades near the magnetic poles; there a compass is unreliable anyway.

const EPOCH = 2025.0;
export const WMM_VALID_FROM = 2025.0;
export const WMM_VALID_TO = 2030.0;

// n m g gdot... as published: n, m, g (nT), h (nT), g dot (nT/yr), h dot (nT/yr)
const COF = `
 1 0 -29351.8 0.0 12.0 0.0
 1 1 -1410.8 4545.4 9.7 -21.5
 2 0 -2556.6 0.0 -11.6 0.0
 2 1 2951.1 -3133.6 -5.2 -27.7
 2 2 1649.3 -815.1 -8.0 -12.1
 3 0 1361.0 0.0 -1.3 0.0
 3 1 -2404.1 -56.6 -4.2 4.0
 3 2 1243.8 237.5 0.4 -0.3
 3 3 453.6 -549.5 -15.6 -4.1
 4 0 895.0 0.0 -1.6 0.0
 4 1 799.5 278.6 -2.4 -1.1
 4 2 55.7 -133.9 -6.0 4.1
 4 3 -281.1 212.0 5.6 1.6
 4 4 12.1 -375.6 -7.0 -4.4
 5 0 -233.2 0.0 0.6 0.0
 5 1 368.9 45.4 1.4 -0.5
 5 2 187.2 220.2 0.0 2.2
 5 3 -138.7 -122.9 0.6 0.4
 5 4 -142.0 43.0 2.2 1.7
 5 5 20.9 106.1 0.9 1.9
 6 0 64.4 0.0 -0.2 0.0
 6 1 63.8 -18.4 -0.4 0.3
 6 2 76.9 16.8 0.9 -1.6
 6 3 -115.7 48.8 1.2 -0.4
 6 4 -40.9 -59.8 -0.9 0.9
 6 5 14.9 10.9 0.3 0.7
 6 6 -60.7 72.7 0.9 0.9
 7 0 79.5 0.0 -0.0 0.0
 7 1 -77.0 -48.9 -0.1 0.6
 7 2 -8.8 -14.4 -0.1 0.5
 7 3 59.3 -1.0 0.5 -0.8
 7 4 15.8 23.4 -0.1 0.0
 7 5 2.5 -7.4 -0.8 -1.0
 7 6 -11.1 -25.1 -0.8 0.6
 7 7 14.2 -2.3 0.8 -0.2
 8 0 23.2 0.0 -0.1 0.0
 8 1 10.8 7.1 0.2 -0.2
 8 2 -17.5 -12.6 0.0 0.5
 8 3 2.0 11.4 0.5 -0.4
 8 4 -21.7 -9.7 -0.1 0.4
 8 5 16.9 12.7 0.3 -0.5
 8 6 15.0 0.7 0.2 -0.6
 8 7 -16.8 -5.2 -0.0 0.3
 8 8 0.9 3.9 0.2 0.2
 9 0 4.6 0.0 -0.0 0.0
 9 1 7.8 -24.8 -0.1 -0.3
 9 2 3.0 12.2 0.1 0.3
 9 3 -0.2 8.3 0.3 -0.3
 9 4 -2.5 -3.3 -0.3 0.3
 9 5 -13.1 -5.2 0.0 0.2
 9 6 2.4 7.2 0.3 -0.1
 9 7 8.6 -0.6 -0.1 -0.2
 9 8 -8.7 0.8 0.1 0.4
 9 9 -12.9 10.0 -0.1 0.1
 10 0 -1.3 0.0 0.1 0.0
 10 1 -6.4 3.3 0.0 0.0
 10 2 0.2 0.0 0.1 -0.0
 10 3 2.0 2.4 0.1 -0.2
 10 4 -1.0 5.3 -0.0 0.1
 10 5 -0.6 -9.1 -0.3 -0.1
 10 6 -0.9 0.4 0.0 0.1
 10 7 1.5 -4.2 -0.1 0.0
 10 8 0.9 -3.8 -0.1 -0.1
 10 9 -2.7 0.9 -0.0 0.2
 10 10 -3.9 -9.1 -0.0 -0.0
 11 0 2.9 0.0 0.0 0.0
 11 1 -1.5 0.0 -0.0 -0.0
 11 2 -2.5 2.9 0.0 0.1
 11 3 2.4 -0.6 0.0 -0.0
 11 4 -0.6 0.2 0.0 0.1
 11 5 -0.1 0.5 -0.1 -0.0
 11 6 -0.6 -0.3 0.0 -0.0
 11 7 -0.1 -1.2 -0.0 0.1
 11 8 1.1 -1.7 -0.1 -0.0
 11 9 -1.0 -2.9 -0.1 0.0
 11 10 -0.2 -1.8 -0.1 0.0
 11 11 2.6 -2.3 -0.1 0.0
 12 0 -2.0 0.0 0.0 0.0
 12 1 -0.2 -1.3 0.0 -0.0
 12 2 0.3 0.7 -0.0 0.0
 12 3 1.2 1.0 -0.0 -0.1
 12 4 -1.3 -1.4 -0.0 0.1
 12 5 0.6 -0.0 -0.0 -0.0
 12 6 0.6 0.6 0.1 -0.0
 12 7 0.5 -0.1 -0.0 -0.0
 12 8 -0.1 0.8 0.0 0.0
 12 9 -0.4 0.1 0.0 -0.0
 12 10 -0.2 -1.0 -0.1 -0.0
 12 11 -1.3 0.1 -0.0 0.0
 12 12 -0.7 0.2 -0.1 -0.1
`;

const N = 12;
const idx = (n, m) => (n * (n + 1)) / 2 + m;
const terms = COF.trim().split("\n").map((l) => l.trim().split(/\s+/).map(Number));
const g = new Float64Array(idx(N, N) + 1), h = new Float64Array(g.length);
const gd = new Float64Array(g.length), hd = new Float64Array(g.length);
for (const [n, m, gv, hv, gdv, hdv] of terms) {
  const i = idx(n, m);
  g[i] = gv; h[i] = hv; gd[i] = gdv; hd[i] = hdv;
}

// Schmidt semi-normalisation factors
const schmidt = new Float64Array(g.length);
schmidt[0] = 1;
for (let n = 1; n <= N; n++) {
  const i = idx(n, 0);
  schmidt[i] = (schmidt[idx(n - 1, 0)] * (2 * n - 1)) / n;
  for (let m = 1; m <= n; m++) schmidt[i + m] = schmidt[i + m - 1] * Math.sqrt(((n - m + 1) * (m === 1 ? 2 : 1)) / (n + m));
}

const A = 6378.137; // WGS84 semi-major axis, km
const F = 1 / 298.257223563;
const E2 = F * (2 - F);
const RE = 6371.2; // the model's reference radius, km
const rad = Math.PI / 180;

/** Field components (nT) in the local north, east, down frame at a geodetic position and decimal year. */
function field(latDeg, lonDeg, heightKm, year) {
  const lat = Math.max(-89.9999, Math.min(89.9999, latDeg)) * rad;
  const lon = lonDeg * rad;
  const dt = year - EPOCH;

  const sl = Math.sin(lat);
  const nu = A / Math.sqrt(1 - E2 * sl * sl);
  const p = (nu + heightKm) * Math.cos(lat);
  const z = (nu * (1 - E2) + heightKm) * sl;
  const r = Math.hypot(p, z);
  const lc = Math.asin(z / r); // geocentric latitude
  const sc = Math.sin(lc), cc = Math.cos(lc);

  const P = new Float64Array(g.length), dP = new Float64Array(g.length);
  P[0] = 1;
  for (let n = 1; n <= N; n++) {
    for (let m = 0; m <= n; m++) {
      const i = idx(n, m);
      if (n === m) {
        const j = idx(n - 1, m - 1);
        P[i] = cc * P[j];
        dP[i] = cc * dP[j] + sc * P[j];
      } else if (n === 1 && m === 0) {
        P[i] = sc * P[0];
        dP[i] = sc * dP[0] - cc * P[0];
      } else {
        const k = m > n - 2 ? 0 : ((n - 1) ** 2 - m * m) / ((2 * n - 1) * (2 * n - 3));
        const a = idx(n - 1, m), b = n - 2 >= m ? idx(n - 2, m) : -1;
        P[i] = sc * P[a] - (b >= 0 ? k * P[b] : 0);
        dP[i] = sc * dP[a] - cc * P[a] - (b >= 0 ? k * dP[b] : 0);
      }
    }
  }
  for (let i = 0; i < P.length; i++) { P[i] *= schmidt[i]; dP[i] *= schmidt[i]; }

  let bx = 0, by = 0, bz = 0;
  for (let n = 1; n <= N; n++) {
    const rr = (RE / r) ** (n + 2);
    for (let m = 0; m <= n; m++) {
      const i = idx(n, m);
      const gv = g[i] + dt * gd[i], hv = h[i] + dt * hd[i];
      const c = Math.cos(m * lon), s = Math.sin(m * lon);
      bz -= rr * (n + 1) * (gv * c + hv * s) * P[i];
      by += (rr * m * (gv * s - hv * c) * P[i]);
      bx += rr * (gv * c + hv * s) * dP[i];
    }
  }
  by /= cc;

  // rotate from geocentric to geodetic
  const psi = lc - lat;
  return { x: bx * Math.cos(psi) - bz * Math.sin(psi), y: by, z: bx * Math.sin(psi) + bz * Math.cos(psi) };
}

/** Decimal year of a Date (default now). */
export function decimalYear(date = new Date()) {
  const y = date.getUTCFullYear();
  const start = Date.UTC(y, 0, 1), end = Date.UTC(y + 1, 0, 1);
  return y + (date.getTime() - start) / (end - start);
}

const declination = (lat, lon, height, year) => {
  const { x, y } = field(lat, lon, height, year);
  return Math.atan2(y, x) / rad;
};

/**
 * Magnetic variation at a position. Returns { variation, dip, annualChange, year, inRange },
 * variation in degrees (east positive), dip in degrees, annualChange in degrees per year.
 * `inRange` is false outside the model's five-year life (the value is then an extrapolation).
 */
export function magneticVariation(lat, lon, date = new Date(), heightKm = 0) {
  if (!Number.isFinite(lat) || !Number.isFinite(lon)) return null;
  const year = date instanceof Date ? decimalYear(date) : Number(date);
  const f = field(lat, lon, heightKm, year);
  const diff = ((declination(lat, lon, heightKm, year + 0.5) - declination(lat, lon, heightKm, year - 0.5) + 540) % 360) - 180;
  return {
    variation: Math.atan2(f.y, f.x) / rad,
    dip: Math.atan2(f.z, Math.hypot(f.x, f.y)) / rad,
    annualChange: diff,
    year,
    inRange: year >= WMM_VALID_FROM && year < WMM_VALID_TO,
  };
}

/** A variation in degrees to a tenth, east or west, as everywhere in the app: "3.2°W", "19.0°E". */
export function fmtVariation(v) {
  return `${Math.abs(v).toFixed(1)}°${v < 0 ? "W" : "E"}`;
}
