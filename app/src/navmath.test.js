import test from "node:test";
import assert from "node:assert/strict";
import { rhumb, rhumbDestination, clearingZone, clearingInfo } from "./navmath.js";

test("rhumbDestination is the inverse of rhumb", () => {
  const a = [174.8, -36.8];
  const b = rhumbDestination(a, 70, 12);
  const r = rhumb(a, b);
  assert.ok(Math.abs(r.brg - 70) < 1e-6 && Math.abs(r.nm - 12) < 1e-6, JSON.stringify(r));
});

test("a clearing zone puts the third point 90° off on the danger side", () => {
  const mark = [174.8, -36.8];
  const end = rhumbDestination(mark, 45, 5);
  // danger to the right (clockwise) of the bearing: a point at 90°T from the mark
  const right = clearingZone(mark, end, rhumbDestination(mark, 100, 1));
  const ri = clearingInfo(right);
  // the mark bears 225° from the boat out at 045° from it; the danger is to the left looking at the mark
  assert.ok(Math.abs(ri.brg - 225) < 1e-6 && Math.abs(ri.nm - 5) < 1e-6);
  assert.equal(ri.side, -1);
  assert.ok(Math.abs(rhumb(mark, right[2]).brg - 135) < 1e-6);
  const left = clearingZone(mark, end, rhumbDestination(mark, 350, 1));
  assert.equal(clearingInfo(left).side, 1);
  assert.ok(Math.abs(rhumb(mark, left[2]).brg - 315) < 1e-6);
});
