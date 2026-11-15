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
