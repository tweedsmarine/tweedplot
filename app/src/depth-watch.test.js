import test from "node:test";
import assert from "node:assert/strict";
import { createDepthWatch, depthFresh, depthHistoryV, checkShallowWater } from "./depth-watch.js";
import { Recorder } from "./histcore.js";

test("depth ages independently for each measurement, including identical updates and failover", () => {
  let now = 100000;
  const watch = createDepthWatch(() => now);
  const v = { depth: 8, depth_keel: 3, depth_age_ms: { depth: 0, depth_keel: 0 } };
  const fresh = (data, alarm = false) => depthFresh(data, (field) => watch.age(field), alarm);
  watch.observe(v);
  now += 10000;
  watch.observe({ ...v, depth_age_ms: { depth: 0, depth_keel: 10000 } });
  assert.equal(fresh(v), true);
  assert.equal(fresh(v, true), false, "a fresh transducer does not refresh keel depth");
  watch.observe({ ...v, depth_age_ms: { depth: 0, depth_keel: 0 } });
  assert.equal(fresh(v, true), true, "an identical new value is a new observation");
  watch.observe({ ...v, depth_age_ms: { depth: 20000, depth_keel: 30000 } });
  assert.equal(fresh(v), false, "a fallback source retains its actual age");
  watch.observe({ depth: null, depth_age_ms: {} });
  assert.equal(fresh(v), false, "an unavailable group cannot reuse the previous source's ages");
});

test("replay age uses log time through pause and seeks", () => {
  let now = 1000;
  const watch = createDepthWatch(() => now);
  const v = { depth: 8, replay_time_ms: 1000, depth_received_at_ms: { depth: 1000 } };
  watch.progress(1000, 1000);
  watch.observe(v, true);
  now += 60000;
  assert.equal(watch.age("depth"), 0);
  watch.progress(11000, 1000);
  assert.equal(depthFresh(v, (f) => watch.age(f)), false);
  watch.observe({ ...v, replay_time_ms: 2000 }, true);
  assert.equal(watch.age("depth"), 1000);
  watch.observe({ replay_time_ms: 500, depth_received_at_ms: {} }, true);
  assert.equal(watch.age("depth"), null);
  watch.reset();
  watch.observe({ depth: 8, depth_age_ms: { depth: 0 } });
  assert.equal(watch.age("depth"), 0);
});

test("unavailable depth warns without clearing a shallow alarm until a fresh safe reading", () => {
  const active = new Map();
  const raise = (id, value) => active.set(id, value);
  const clear = (id) => active.delete(id);
  const check = (args) => checkShallowWater({ enabled: true, plottingLive: true, limit: 5, graceElapsed: true, ...args }, raise, clear);
  check({ depth: 3, fresh: true, underKeel: true });
  assert.ok(active.has("shallow"));
  check({ depth: 8, fresh: false });
  assert.ok(active.has("shallow"), "an old safe value must not clear an active alarm");
  assert.ok(active.has("depth-unavailable"));
  check({ depth: 8, fresh: true });
  assert.equal(active.size, 0);
  check({ depth: null, fresh: false, graceElapsed: false });
  assert.equal(active.size, 0, "startup grace");
  check({ depth: null, fresh: false });
  assert.ok(active.has("depth-unavailable"));
  check({ enabled: false });
  assert.equal(active.size, 0);
  check({ depth: null, fresh: false });
  check({ plottingLive: false });
  assert.equal(active.size, 0);
});

test("captured NMEA transducer depth clears the unavailable alarm and becomes unavailable when it stops", () => {
  let now = 100000;
  const watch = createDepthWatch(() => now);
  const active = new Map();
  const raise = (id, value) => active.set(id, value);
  const clear = (id) => active.delete(id);
  const check = (v) => checkShallowWater({
    enabled: true, plottingLive: true, limit: 5, depth: v.depth_keel ?? v.depth,
    fresh: depthFresh(v, (field) => watch.age(field), true), graceElapsed: watch.graceElapsed(), underKeel: v.depth_keel != null,
  }, raise, clear);
  now += 10000;
  check({});
  assert.ok(active.has("depth-unavailable"));
  const v = { depth: 29.8, depth_keel: null, depth_age_ms: { depth: 250 } };
  watch.observe(v);
  check(v);
  assert.equal(active.has("depth-unavailable"), false);
  assert.equal(active.has("shallow"), false);
  now += 9750;
  check(v);
  assert.ok(active.has("depth-unavailable"));
});

test("stale depth creates a history gap without interrupting fresh wind history", () => {
  const recorder = new Recorder();
  const v = { depth: 8, aws: 12 };
  recorder.sample(depthHistoryV(v, () => 0), 10000);
  recorder.sample(depthHistoryV(v, () => 10000), 15000);
  recorder.sample(depthHistoryV(v, () => 20000), 20000);
  recorder.sample(depthHistoryV(v, () => 0), 25000);
  recorder.sample({}, 30000);
  assert.deepEqual(recorder.raw.DPT.map((p) => p.t), [10000, 25000]);
  assert.deepEqual(recorder.raw.AWS.map((p) => p.t), [10000, 15000, 20000, 25000]);
});
