import test from "node:test";
import assert from "node:assert/strict";

globalThis.window = { addEventListener() {} };
const { surveyFlagsForView } = await import("./surveydepths.js");

test("survey depth and outline visibility are independent by view", () => {
  const defaults = { show: true, outlines: false };
  const views = {
    nav: { show: true, outlines: false },
    plan: { show: false, outlines: true },
  };
  assert.deepEqual(surveyFlagsForView(views, "nav", defaults), { show: true, outlines: false });
  assert.deepEqual(surveyFlagsForView(views, "plan", defaults), { show: false, outlines: true });
  assert.deepEqual(surveyFlagsForView(views, "env", defaults), defaults);
});

test("missing or invalid saved survey flags use the existing choices", () => {
  const defaults = { show: false, outlines: true };
  assert.deepEqual(surveyFlagsForView({ plan: { show: "yes" } }, "plan", defaults), defaults);
  assert.deepEqual(surveyFlagsForView(null, "nav", defaults), defaults);
});
