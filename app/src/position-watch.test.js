import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createPositionWatch } from "./position-watch.js";

test("position age follows fixes, not unrelated vessel updates", () => {
  let now = 100000;
  const watch = createPositionWatch(() => now);
  const fix = { lat: -41, lon: 173, position_observation: 1, position_age_ms: 0 };
  watch.observe(fix);
  now += 29000;
  watch.observe({ ...fix, depth: 12 });
  assert.equal(watch.lost(), false);
  now += 1000;
  watch.observe({ ...fix, aws: 8 });
  assert.equal(watch.lost(), true);
  watch.observe({ ...fix, position_observation: 2 });
  assert.equal(watch.lost(), false, "an unchanged position can still be a new fix");
});

test("missing and invalid fixes do not reset the clock, and source changes reset the grace period", () => {
  let now = 0;
  const watch = createPositionWatch(() => now);
  now = 30000;
  watch.observe({ lat: 999, lon: 173, position_observation: 1, position_age_ms: 0 });
  assert.equal(watch.lost(), true);
  watch.reset();
  assert.equal(watch.lost(), false);
  now += 30000;
  assert.equal(watch.lost(), true);
  watch.observe({ lat: -41, lon: 173, position_observation: 2, position_age_ms: 9000 });
  assert.equal(watch.age(), 9000, "a fallback fix retains its original age");
  watch.observe({ lat: null, lon: null, position_observation: 0 });
  assert.equal(watch.age(), 9000);
});

test("a late window does not treat an old fix as fresh", () => {
  const watch = createPositionWatch(() => 100000);
  watch.observe({ lat: -41, lon: 173, position_observation: 1, position_age_ms: 31000 });
  assert.equal(watch.lost(), true);
  watch.reset();
  watch.observe({ lat: -41, lon: 173, position_observation: 1 });
  assert.equal(watch.age(), null, "an unknown fix age is not assumed fresh");
});

test("replay uses log time through pause, speed changes, seeking and return to live", () => {
  let now = 100000;
  const watch = createPositionWatch(() => now);
  const fix = { lat: -41, lon: 173, position_received_at_ms: 1000, replay_time_ms: 1000 };
  watch.replayProgress(1000, 1000);
  watch.observe(fix, true);
  now += 60000;
  assert.equal(watch.lost(), false, "pausing does not age a replay fix");
  watch.replayProgress(31000, 1000);
  assert.equal(watch.lost(), true, "accelerated log time ages the fix");
  watch.replayProgress(5000, 1000);
  watch.observe({ ...fix, replay_time_ms: 5000 }, true);
  assert.equal(watch.lost(), false, "seeking backwards restores freshness");
  watch.observe({ ...fix, replay_time_ms: 40000 }, true);
  assert.equal(watch.lost(), true, "seeking forwards ages the same fix");
  watch.observe({ lat: null, lon: null, replay_time_ms: 32000 }, true);
  assert.equal(watch.lost(), true, "a seek before any fix uses the log start");
  watch.reset();
  watch.observe({ lat: -41, lon: 173, position_observation: 2, position_age_ms: 0 });
  assert.equal(watch.lost(), false, "returning to live starts a new watch");
});

test("the position alarm raises on a silent or disconnected live feed and clears on recovery", () => {
  const source = readFileSync(new URL("./alarms.js", import.meta.url), "utf8");
  const start = source.indexOf("    // Position age is independent");
  const end = source.indexOf("    // air draft:", start);
  assert.ok(start >= 0 && end > start);
  const check = new Function("b", "s", "raise", "clear", source.slice(start, end));
  const active = new Set();
  const raise = (id) => active.add(id);
  const clear = (id) => active.delete(id);
  for (const connected of [true, false]) {
    check({ plottingLive: true, positionLost: true, connected, live: false }, { posLost: true }, raise, clear);
    assert.ok(active.has("poslost"));
  }
  check({ plottingLive: true, positionLost: false }, { posLost: true }, raise, clear);
  assert.equal(active.has("poslost"), false);
  check({ plottingLive: false, positionLost: true }, { posLost: true }, raise, clear);
  assert.equal(active.has("poslost"), false, "plotting pause suppresses this alarm");
  check({ plottingLive: true, positionLost: true }, { posLost: false }, raise, clear);
  assert.equal(active.has("poslost"), false, "disabled position alarm stays disabled");
});

test("the position readout uses position age rather than the age of other sensor data", () => {
  const source = readFileSync(new URL("./instruments.js", import.meta.url), "utf8");
  const start = source.indexOf("function readingStale(");
  const end = source.indexOf("\nfunction ", start + 1);
  assert.ok(start >= 0 && end > start);
  const items = { POS: {} };
  const readingStale = new Function("ITEMS", "STALE_S", "depthFresh", `${source.slice(start, end)}; return readingStale;`)(items, 10, () => false);
  assert.equal(readingStale(items.POS, { positionAt: Date.now() - 11000, age: 0 }), true);
  assert.equal(readingStale(items.POS, { positionAt: Date.now(), age: 100 }), false);
  assert.equal(readingStale(items.POS, { positionAt: null, age: 0 }), true);
});
