import test from "node:test";
import assert from "node:assert/strict";
import { setDriftTracker } from "./setdrift.js";

test("set and drift: ground minus water vector, averaged", () => {
  const sd = setDriftTracker();
  // heading 000 at 5 kn through the water; over the ground 5 kn north plus 1 kn east = 5.10 kn at 011°
  const cog = (Math.atan2(1, 5) * 180) / Math.PI;
  for (let i = 0; i < 20; i++) sd.add(i * 1000, { heading: 0, stw: 5, cog, sog: Math.hypot(1, 5) });
  const v = sd.value(19000);
  assert.ok(Math.abs(v.drift - 1) < 1e-9);
  assert.ok(Math.abs(v.set - 90) < 1e-6);
});

test("set and drift needs enough samples and every input", () => {
  const sd = setDriftTracker();
  for (let i = 0; i < 4; i++) sd.add(i * 1000, { heading: 0, stw: 5, cog: 0, sog: 5 });
  assert.equal(sd.value(4000), null); // too few
  sd.add(5000, { heading: 0, stw: null, cog: 0, sog: 5 }); // no log: skipped
  assert.equal(sd.value(5000), null);
  for (let i = 6; i < 20; i++) sd.add(i * 1000, { heading: 350, stw: 4, cog: 350, sog: 4 });
  // no current: nothing left once the water vector is taken off
  assert.ok(sd.value(19000).drift < 1e-9);
  // samples older than the minute drop out
  assert.equal(sd.value(200000), null);
});
