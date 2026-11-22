import test from "node:test";
import assert from "node:assert/strict";
import { fieldFromDirSpeed, sampleField, speedColour } from "./flow.js";

const grid = (values) => ({ lon0: 179, lat0: -41, dlon: 0.5, dlat: 0.5, ni: 4, nj: 2, values });

test("direction and speed become east and north components", () => {
  // flowing east at 2 kn, north at 1 kn
  const f = fieldFromDirSpeed(grid([90, 90, 0, 0, 90, 90, 0, null]), grid([2, 2, 1, 1, 2, 2, 1, 1]));
  assert.ok(Math.abs(f.u[0] - 2) < 1e-6 && Math.abs(f.v[0]) < 1e-6);
  assert.ok(Math.abs(f.u[2]) < 1e-6 && Math.abs(f.v[2] - 1) < 1e-6);
  assert.ok(Number.isNaN(f.u[7]));
  assert.equal(fieldFromDirSpeed(grid([0]), { ...grid([0]), ni: 3 }), null);
});

test("a field running past 180° is sampled from either side of the line", () => {
  const f = fieldFromDirSpeed(grid([90, 90, 90, 90, 90, 90, 90, 90]), grid([1, 1, 3, 3, 1, 1, 3, 3]));
  // 180.5° = -179.5°: the third column
  assert.ok(Math.abs(sampleField(f, -179.5, -41)[0] - 3) < 1e-6);
  assert.ok(Math.abs(sampleField(f, 180.5, -41)[0] - 3) < 1e-6);
  // halfway between 179.5 (1 kn) and 180 (3 kn)
  assert.ok(Math.abs(sampleField(f, 179.75, -40.75)[0] - 2) < 1e-6);
  assert.equal(sampleField(f, 170, -41), null);
  assert.equal(sampleField(f, 179.5, -39), null);
});

test("by land the nearest value is used, and nothing where there is none", () => {
  const f = fieldFromDirSpeed(grid([90, null, 90, 90, null, null, 90, 90]), grid([1, null, 1, 1, null, null, 1, 1]));
  assert.ok(Math.abs(sampleField(f, 179.1, -40.9)[0] - 1) < 1e-6);
  assert.equal(sampleField(f, 179.4, -40.6), null);
});

test("particle colours get stronger with speed", () => {
  const a = (c) => Number(c.match(/,([\d.]+)\)$/)[1]);
  assert.ok(a(speedColour(0)) < a(speedColour(1)));
  assert.equal(a(speedColour(5)), 1);
});
