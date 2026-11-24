import test from "node:test";
import assert from "node:assert/strict";
import { magneticVariation, decimalYear, fmtVariation } from "./wmm.js";

// NOAA's published WMM2025 test values: year, height km, lat, lon, declination, inclination, D dot
const CASES = [
  [2025.0, 0, 80, 0, 1.28, 83.21, 0.52],
  [2025.0, 0, 0, 120, -0.16, -14.93, -0.03],
  [2025.0, 0, -80, 240, 68.78, -72.0, -0.12],
  [2025.0, 100, 80, 0, 0.85, 83.26, 0.52],
  [2025.0, 100, 0, 120, -0.15, -15.08, -0.03],
  [2025.0, 100, -80, 240, 68.21, -72.19, -0.11],
  [2027.5, 0, 80, 0, 2.59, 83.24, 0.53],
  [2027.5, 0, 0, 120, -0.24, -14.65, -0.03],
  [2027.5, 0, -80, 240, 68.49, -71.92, -0.12],
  [2027.5, 100, 80, 0, 2.16, 83.29, 0.52],
  [2027.5, 100, 0, 120, -0.23, -14.81, -0.03],
  [2027.5, 100, -80, 240, 67.93, -72.1, -0.11],
];

test("matches the NOAA WMM2025 test values", () => {
  for (const [year, h, lat, lon, d, i, dd] of CASES) {
    const r = magneticVariation(lat, lon, year, h);
    assert.ok(Math.abs(r.variation - d) < 0.02, `D ${lat},${lon} ${year}: ${r.variation} vs ${d}`);
    assert.ok(Math.abs(r.dip - i) < 0.02, `I ${lat},${lon} ${year}: ${r.dip} vs ${i}`);
    assert.ok(Math.abs(r.annualChange - dd) < 0.02, `Ddot ${lat},${lon} ${year}: ${r.annualChange} vs ${dd}`);
  }
});

test("Auckland variation is about 19°E in 2025", () => {
  const r = magneticVariation(-36.85, 174.77, new Date(Date.UTC(2025, 6, 1)));
  assert.ok(r.variation > 18 && r.variation < 20.5, String(r.variation));
  assert.equal(r.inRange, true);
});

test("flags dates outside the model's life", () => {
  assert.equal(magneticVariation(0, 0, new Date(Date.UTC(2032, 0, 1))).inRange, false);
  assert.equal(magneticVariation(NaN, 0), null);
});

test("decimal year and formatting", () => {
  assert.ok(Math.abs(decimalYear(new Date(Date.UTC(2025, 0, 1))) - 2025) < 1e-9);
  assert.equal(fmtVariation(-3.24), "3°14.4′W");
  assert.equal(fmtVariation(19.04), "19°02.4′E");
  assert.equal(fmtVariation(22.9999), "23°00.0′E");
});
