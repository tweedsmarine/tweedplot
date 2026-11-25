import test from "node:test";
import assert from "node:assert/strict";
import { readPrefs, checkDue, noticeText, progressPercent, CHECK_EVERY_MS, DEFAULT_MODE } from "./updates-core.js";

test("preferences: off until asked, and survive rubbish", () => {
  assert.equal(DEFAULT_MODE, "off");
  assert.deepEqual(readPrefs(null), { mode: "off", lastCheck: 0, asked: false });
  assert.deepEqual(readPrefs("not json"), { mode: "off", lastCheck: 0, asked: false });
  // a setting from before the question isn't an answer: asked again, off meanwhile
  assert.deepEqual(readPrefs('{"mode":"notify","lastCheck":5}'), { mode: "off", lastCheck: 5, asked: false });
  assert.deepEqual(readPrefs('{"mode":"notify","asked":true}'), { mode: "notify", lastCheck: 0, asked: true });
  assert.deepEqual(readPrefs('{"mode":"sometimes","lastCheck":-5,"asked":true}'), { mode: "off", lastCheck: 0, asked: true });
  assert.deepEqual(readPrefs('{"mode":"download","lastCheck":1000,"asked":true}'), { mode: "download", lastCheck: 1000, asked: true });
  assert.deepEqual(readPrefs("[1,2]").mode, "off");
});

test("automatic checks: never when off, otherwise twice a day", () => {
  const now = 10 * CHECK_EVERY_MS;
  assert.equal(checkDue({ mode: "off", lastCheck: 0 }, now), false);
  assert.equal(checkDue({ mode: "notify", lastCheck: 0, asked: true }, now), true);
  assert.equal(checkDue({ mode: "notify", lastCheck: 0, asked: false }, now), false);
  assert.equal(checkDue({ mode: "notify", lastCheck: now - 1000, asked: true }, now), false);
  assert.equal(checkDue({ mode: "download", lastCheck: now - CHECK_EVERY_MS, asked: true }, now), true);
  // the clock went back: look anyway
  assert.equal(checkDue({ mode: "notify", lastCheck: now + 5000, asked: true }, now), true);
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
