import test from "node:test";
import assert from "node:assert/strict";
import { inside } from "./arpa.js";

test("radar target inside the collision limits", () => {
  const lim = { cpa: 0.5, tcpa: 12 };
  assert.ok(inside({ cpaNm: 0.2, tcpaMin: 8 }, lim));
  assert.ok(!inside({ cpaNm: 0.2, tcpaMin: 20 }, lim)); // too far off in time
  assert.ok(!inside({ cpaNm: 0.8, tcpaMin: 5 }, lim)); // passes clear
  assert.ok(!inside({ cpaNm: 0.1, tcpaMin: -3 }, lim)); // already past
  assert.ok(!inside({ cpaNm: 0.1, tcpaMin: 3, status: "lost" }, lim));
  assert.ok(!inside({ tcpaMin: 3 }, lim)); // no CPA from the radar
});

