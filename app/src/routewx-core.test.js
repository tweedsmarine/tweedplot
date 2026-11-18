import { test } from "node:test";
import assert from "node:assert/strict";
import { courseTrack, sampleTrack, coverage, gaps, createRouteWeatherRequest } from "./routewx-core.js";

test("course uses its fixed departure and rejects invalid speed and zero-length routes", () => {
  const course = { start: [173, -41], marks: [{ lon: 174, lat: -41 }] };
  const pts = courseTrack(course, 100000, 6);
  assert.equal(pts[0].t, 100000);
  assert.ok(pts[1].t > pts[0].t);
  assert.throws(() => courseTrack(course, 100000, 0), /invalid/);
  assert.throws(() => courseTrack({ start: [173, -41], marks: [{ lon: 173, lat: -41 }] }, 100000, 6), /no length/);
});

test("sampling includes both endpoints and rejects invalid timings", () => {
  const pts = [{ t: 1000, lon: 0, lat: 0 }, { t: 1600, lon: 1, lat: 0 }];
  const rows = sampleTrack(pts, (p, t) => ({ lon: (t - 1000) / 600, lat: 0 }));
  assert.deepEqual(rows.map((r) => r.t), [1000, 1600]);
  assert.ok(rows[1].nm > 0);
  assert.throws(() => sampleTrack([pts[1], pts[0]], () => null), /invalid/);
});

test("coverage distinguishes missing and partial fields", () => {
  assert.deepEqual(coverage([{ tws: 5 }, { tws: null, wave: 2 }], [{ key: "tws", label: "Wind" }, { key: "wave", label: "Waves" }, { key: "rain", label: "Rain" }]).map((x) => x.count), [1, 1, 0]);
});

test("stale and failed requests never publish an old result", async () => {
  const seen = [];
  const request = createRouteWeatherRequest((state) => seen.push(state));
  let finish;
  const old = request.run(() => new Promise((resolve) => { finish = resolve; }));
  request.invalidate();
  await request.run(async () => "new");
  finish("old");
  await old;
  assert.equal(request.state().data, "new");
  await request.run(async () => { throw new Error("offline"); });
  assert.equal(request.state().status, "error");
  assert.equal(request.state().data, null);
  assert.match(request.state().error, /offline/);
  assert.ok(seen.some((s) => s.status === "stale" && s.data === null));
});

test("gaps tell a forecast that ends early from holes inside it", () => {
  const f = [{ key: "tws", label: "Wind" }, { key: "wave", label: "Waves" }];
  const rows = [{ tws: 1, wave: null }, { tws: 2, wave: 1 }, { tws: 3, wave: null }, { tws: 4, wave: 2 }, { tws: null, wave: null }, { tws: null, wave: null }];
  const [w, v] = gaps(rows, f);
  assert.deepEqual([w.lead, w.inner, w.trail, w.earlier], [0, 0, 2, 0]);   // the route outlasts the forecast: no warning
  assert.deepEqual([v.lead, v.inner, v.trail, v.earlier], [1, 1, 2, 0]);
  // waves are a sea field: the hole where the wind has a value is land, not missing data
  const [, s] = gaps(rows, [f[0], { ...f[1], sea: true }]);
  assert.equal(s.inner, 0);
  // with neither wind nor waves there, it is still a hole (outside the GRIBs)
  const [, t] = gaps([{ tws: 1, wave: 1 }, { tws: null, wave: null }, { tws: 1, wave: 1 }], [f[0], { ...f[1], sea: true }]);
  assert.equal(t.inner, 1);
});
