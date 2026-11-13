import test from "node:test";
import assert from "node:assert/strict";
import { aisVisibleInView } from "./ais-visibility.js";

test("AIS visibility defaults off in Planning and preserves the previous setting elsewhere", () => {
  assert.equal(aisVisibleInView({}, "plan", true), false);
  assert.equal(aisVisibleInView({}, "nav", true), true);
  assert.equal(aisVisibleInView({}, "nav", false), false);
  assert.equal(aisVisibleInView({ plan: true, nav: false }, "plan", false), true);
  assert.equal(aisVisibleInView({ plan: true, nav: false }, "nav", true), false);
  assert.equal(aisVisibleInView({ plan: "true" }, "plan", true), false);
});
