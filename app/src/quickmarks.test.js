import { test } from "node:test";
import assert from "node:assert/strict";
import { defaultSlots, cleanSlots, markName, slotForKey } from "./quickmarks.js";

test("key 1 starts as a filled triangle, the rest blank", () => {
  const d = defaultSlots();
  assert.equal(d.length, 10);
  assert.equal(d[0].kind, "sym-tri-up-f");
  assert.ok(d.slice(1).every((s) => s === null));
});

test("stored slots are checked", () => {
  const s = cleanSlots([{ kind: "sym-fish", colour: "#ff0000", name: "Snapper" }, { kind: "nonsense" }, { kind: "sym-rock", colour: "red" }]);
  assert.deepEqual(s[0], { kind: "sym-fish", colour: "#ff0000", name: "Snapper" });
  assert.equal(s[1], null);
  assert.equal(s[2].colour, "");
  assert.equal(s[9], null);
  assert.equal(cleanSlots("junk")[0].kind, "sym-tri-up-f");
});

test("names and keys", () => {
  const when = new Date(2026, 10, 25, 19, 5, 9);
  assert.equal(markName({ kind: "sym-tri-up-f", name: "" }, when), "Filled triangle up 19:05:09");
  assert.equal(markName({ kind: "sym-fish", name: "Snapper " }, when), "Snapper 19:05:09");
  assert.equal(slotForKey({ code: "Digit1" }), 0);
  assert.equal(slotForKey({ code: "Digit0" }), 9);
  assert.equal(slotForKey({ code: "Numpad5" }), 4);
  assert.equal(slotForKey({ code: "KeyA" }), -1);
});
