import test from "node:test";
import assert from "node:assert/strict";
import { readPrefs, checkDue, noticeText, progressPercent, CHECK_EVERY_MS, DEFAULT_MODE } from "./updates-core.js";

test("preferences default to notify, and survive rubbish", () => {
  assert.deepEqual(readPrefs(null), { mode: DEFAULT_MODE, lastCheck: 0 });
  assert.deepEqual(readPrefs("not json"), { mode: "notify", lastCheck: 0 });
  assert.deepEqual(readPrefs('{"mode":"sometimes","lastCheck":-5}'), { mode: "notify", lastCheck: 0 });
  assert.deepEqual(readPrefs('{"mode":"download","lastCheck":1000}'), { mode: "download", lastCheck: 1000 });
  assert.deepEqual(readPrefs("[1,2]").mode, "notify");
});

test("automatic checks: never when off, otherwise twice a day", () => {
  const now = 10 * CHECK_EVERY_MS;
  assert.equal(checkDue({ mode: "off", lastCheck: 0 }, now), false);
  assert.equal(checkDue({ mode: "notify", lastCheck: 0 }, now), true);
  assert.equal(checkDue({ mode: "notify", lastCheck: now - 1000 }, now), false);
  assert.equal(checkDue({ mode: "download", lastCheck: now - CHECK_EVERY_MS }, now), true);
  // the clock went back: look anyway
  assert.equal(checkDue({ mode: "notify", lastCheck: now + 5000 }, now), true);
});

test("notice text and progress", () => {
  const u = { version: "1.0.1", currentVersion: "1.0.0" };
  assert.equal(noticeText(u), "TweedPlot 1.0.1 is available (you have 1.0.0).");
  assert.equal(noticeText(u, { downloaded: true }), "TweedPlot 1.0.1 is ready to install.");
  assert.equal(noticeText(u, { progress: 42 }), "TweedPlot 1.0.1: downloading 42%");
  assert.equal(progressPercent(50, 200), 25);
  assert.equal(progressPercent(250, 200), 100);
  assert.equal(progressPercent(10, 0), null);
  assert.equal(progressPercent(10, undefined), null);
});
