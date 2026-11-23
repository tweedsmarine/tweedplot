import test from "node:test";
import assert from "node:assert/strict";
import { routeAxis } from "./routewx.js";

test("distance axis: equal room per mile, and back to time", () => {
  // 10 NM in the first hour, then 2 NM in the next five (a slow stretch)
  const rows = [{ t: 0, nm: 0 }, { t: 3600, nm: 10 }, { t: 6 * 3600, nm: 12 }];
  const ax = routeAxis(rows, 0, 6 * 3600, "distance");
  assert.equal(ax.dist, true);
  assert.ok(Math.abs(ax.f(3600) - 10 / 12) < 1e-9);
  assert.ok(Math.abs(ax.f(1800) - 5 / 12) < 1e-9);
  assert.ok(Math.abs(ax.inv(10 / 12) - 3600) < 1e-6);
  assert.ok(Math.abs(ax.tAtNm(11) - 3600 * 3.5) < 1e-6);
  const time = routeAxis(rows, 0, 6 * 3600, "time");
  assert.equal(time.dist, false);
  assert.ok(Math.abs(time.f(3600) - 1 / 6) < 1e-9);
  // a route that doesn't move falls back to time
  assert.equal(routeAxis([{ t: 0, nm: 0 }, { t: 100, nm: 0 }], 0, 100, "distance").dist, false);
});
