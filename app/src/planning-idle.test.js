import test from "node:test";
import assert from "node:assert/strict";
import { createPlanningIdle } from "./planning-idle.js";

test("Planning inactivity resets on input and returns to Navigation at the configured deadline", () => {
  let time = 0, view = "nav", minutes = 5, scheduled = null, calls = 0;
  const idle = createPlanningIdle({
    now: () => time,
    schedule: (fn, delay) => { scheduled = { fn, at: time + delay }; return scheduled; },
    cancel: () => { scheduled = null; },
    currentView: () => view,
    timeoutMinutes: () => minutes,
    onTimeout: () => { calls++; view = "nav"; },
  });

  idle.activity();
  assert.equal(scheduled, null);
  view = "plan";
  idle.activity();
  assert.equal(scheduled.at, 300000);
  time = 240000;
  idle.activity();
  assert.equal(scheduled.at, 540000);
  time = 539999;
  scheduled.fn();
  assert.equal(calls, 0);
  assert.equal(scheduled.at, 540000);
  time = 540000;
  scheduled.fn();
  assert.equal(calls, 1);
  assert.equal(view, "nav");

  view = "plan"; minutes = 0;
  idle.activity();
  assert.equal(scheduled, null);
  time += 3600000;
  idle.check();
  assert.equal(calls, 1);
  minutes = 1;
  idle.activity();
  time += 60001;
  idle.check();
  assert.equal(calls, 2);
});

test("Environment uses the same inactivity timeout and does not inherit Planning's elapsed time", () => {
  let time = 0, view = "plan", scheduled = null, calls = 0;
  const idle = createPlanningIdle({
    now: () => time,
    schedule: (fn, delay) => { scheduled = { fn, at: time + delay }; return scheduled; },
    cancel: () => { scheduled = null; },
    currentView: () => view,
    timeoutMinutes: () => 5,
    onTimeout: () => { calls++; view = "nav"; },
  });
  idle.activity();
  time = 240000;
  view = "env";
  idle.activity();
  assert.equal(scheduled.at, 540000);
  time = 540000;
  scheduled.fn();
  assert.equal(calls, 1);
  assert.equal(view, "nav");
  assert.equal(scheduled, null);
});

test("a notice runs for the last 30 seconds, input cancels it, and nothing happens without a live position", () => {
  let time = 0, view = "plan", scheduled = null, timeouts = 0, live = true, shown = null, ended = 0;
  const idle = createPlanningIdle({
    now: () => time,
    schedule: (fn, delay) => { scheduled = { fn, at: time + delay }; return scheduled; },
    cancel: () => { scheduled = null; },
    currentView: () => view,
    timeoutMinutes: () => 5,
    onTimeout: () => { timeouts++; view = "nav"; },
    active: () => live, warnMs: 30000, recheckMs: 15000,
    onWarn: (seconds) => { shown = seconds; },
    onWarnEnd: () => { shown = null; ended++; },
  });
  idle.activity();
  assert.equal(scheduled.at, 270000); // the notice starts 30 s before the five minutes are up
  time = 270000; scheduled.fn();
  assert.equal(shown, 30);
  assert.equal(timeouts, 0);
  time = 285000; scheduled.fn();
  assert.equal(shown, 15);
  idle.activity(); // any input, or Stay
  assert.equal(shown, null);
  assert.equal(ended, 1);
  assert.equal(scheduled.at, 285000 + 270000);
  time = 555000; scheduled.fn();
  time = 585000; scheduled.fn();
  assert.equal(timeouts, 1);
  assert.equal(shown, null);

  // no live position: no notice and no return, however long without input
  view = "plan"; live = false; time = 600000; idle.activity();
  time = 2000000; scheduled.fn();
  assert.equal(timeouts, 1);
  assert.equal(shown, null);
  assert.equal(scheduled.at, 2015000);
  // the position starts arriving: the full notice first, then the return
  live = true; time = 2015000; scheduled.fn();
  assert.equal(shown, 30);
  assert.equal(timeouts, 1);
  time = 2045000; scheduled.fn();
  assert.equal(timeouts, 2);
});
