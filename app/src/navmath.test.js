import test from "node:test";
import assert from "node:assert/strict";
import { rhumb, rhumbDestination, clearingZone, clearingInfo, bowOf } from "./navmath.js";

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

test("the bow is found from the antenna's distance aft and its offset off the centreline", () => {
  const p = [173.27, -41.26];
  const m = (a, b) => [((b[0] - a[0]) * 111120 * Math.cos((a[1] * Math.PI) / 180)), (b[1] - a[1]) * 111120];
  // heading north, antenna 10 m aft and 2 m to starboard: the bow is 10 m north and 2 m west
  const [e, n] = m(p, bowOf(p, { heading: 0 }, 10, 2).pos);
  assert.ok(Math.abs(n - 10) < 0.05 && Math.abs(e + 2) < 0.05, `${e}, ${n}`);
  // heading east, antenna 1.5 m to port: the bow is 1.5 m south of the antenna's line
  const [e2, n2] = m(p, bowOf(p, { heading: 90 }, 0, -1.5).pos);
  assert.ok(Math.abs(e2) < 0.05 && Math.abs(n2 + 1.5) < 0.05, `${e2}, ${n2}`);
  // no heading: the antenna itself
  assert.deepEqual(bowOf(p, {}, 10, 2).pos, p);
});
