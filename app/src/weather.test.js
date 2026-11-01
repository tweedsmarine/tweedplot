import test from "node:test";
import assert from "node:assert/strict";
import { seaScale, fronts, filterable, LAYERS } from "./weather.js";

test("sea scale without a target runs evenly from low to high", () => {
  const s = seaScale({ lo: 10, hi: 20, tg: null });
  assert.equal(s[0][0], 10);
  assert.equal(s.at(-1)[0], 20);
  const steps = s.slice(1).map(([v], i) => v - s[i][0]);
  for (const d of steps) assert.ok(Math.abs(d - steps[0]) < 1e-9);
});

test("sea scale with a target puts the middle colour at the target", () => {
  const s = seaScale({ lo: 10, hi: 20, tg: 18 });
  const half = s.length / 2;
  assert.equal(s[0][0], 10);
  assert.ok(Math.abs(s[half][0] - 18) < 1e-9);
  for (let i = 1; i < s.length; i++) assert.ok(s[i][0] > s[i - 1][0], "rising");
});

test("fronts: only where the temperature changes fast enough", () => {
  // 5 x 5 grid at 0.1°: flat at 15 °C in the west, 18 °C in the east (a front down the middle)
  const ni = 5, nj = 5;
  const values = [];
  for (let j = 0; j < nj; j++) for (let i = 0; i < ni; i++) values.push(i < 2 ? 15 : 18);
  const gs = { ni, nj, lon0: 170, lat0: -40, dlon: 0.1, dlat: 0.1, values };
  const f = fronts(gs);
  assert.ok(f.length > 0);
  // every cell found is next to the step (columns 1 and 2), none in the flat parts
  for (const c of f) {
    const lon = c.geometry.coordinates[0][0][0] + 0.05;
    assert.ok(Math.abs(lon - 170.1) < 1e-6 || Math.abs(lon - 170.2) < 1e-6, String(lon));
    assert.ok(c.properties.g >= 0.02);
  }
  // a flat field has none, and missing values (land, cloud) are never taken as 0 °C
  assert.equal(fronts({ ...gs, values: values.map(() => 15) }).length, 0);
  assert.equal(fronts({ ...gs, values: values.map((v) => (v === 15 ? null : v)) }).length, 0);
});

test("the filter is for single-file layers, not the agreement or error displays", () => {
  assert.ok(filterable(LAYERS.sst));
  assert.ok(filterable(LAYERS.wind));
  assert.ok(!filterable(LAYERS.agree_vec));
  assert.ok(!filterable(undefined));
});
