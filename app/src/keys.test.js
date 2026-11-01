import test from "node:test";
import assert from "node:assert/strict";
import { KEYS, matches, parseKey } from "./keys.js";

const ev = (o) => ({ key: "", code: "", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...o });

test("parses modifiers", () => {
  assert.deepEqual(parseKey("Ctrl+M"), { ctrl: true, shift: false, alt: false, key: "M" });
  assert.deepEqual(parseKey("Shift+]"), { ctrl: false, shift: true, alt: false, key: "]" });
});
test("matches letters either case, and modifiers exactly", () => {
  assert.equal(matches("W", ev({ key: "w" })), true);
  assert.equal(matches("W", ev({ key: "w", ctrlKey: true })), false);
  assert.equal(matches("A", ev({ key: "A", shiftKey: true })), false);
  assert.equal(matches("Shift+A", ev({ key: "A", shiftKey: true })), true);
  assert.equal(matches("Ctrl+M", ev({ key: "m", ctrlKey: true })), true);
  assert.equal(matches("Ctrl+M", ev({ key: "m", metaKey: true })), true);
});
test("matches brackets by position, ? with or without Shift", () => {
  assert.equal(matches("]", ev({ key: "]", code: "BracketRight" })), true);
  assert.equal(matches("Shift+[", ev({ key: "{", code: "BracketLeft", shiftKey: true })), true);
  assert.equal(matches("[", ev({ key: "{", code: "BracketLeft", shiftKey: true })), false);
  assert.equal(matches("?", ev({ key: "?", shiftKey: true })), true);
  assert.equal(matches("Space", ev({ key: " ", code: "Space" })), true);
});
test("has no key used twice", () => {
  const seen = new Set();
  for (const k of KEYS.filter((x) => !x.own)) {
    assert.equal(seen.has(k.keys), false, k.keys);
    seen.add(k.keys);
  }
});

