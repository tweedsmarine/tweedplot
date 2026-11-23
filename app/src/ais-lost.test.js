import test from "node:test";
import assert from "node:assert/strict";
import { reportInterval, lostAfter, targetLost } from "./ais-lost.js";

test("lost after six reporting intervals (IEC 62388 table)", () => {
  assert.equal(lostAfter({ kind: "vessel", class: "A", status: 1, sog: 0.1 }), 18 * 60);
  assert.equal(lostAfter({ kind: "vessel", class: "A", status: 0, sog: 8 }), 60);
  assert.equal(lostAfter({ kind: "vessel", class: "A", sog: 18 }), 36);
  assert.equal(lostAfter({ kind: "vessel", class: "A", sog: 30 }), 12);
  // anchored but dragging faster than 3 kn reports every 10 s
  assert.equal(reportInterval({ kind: "vessel", class: "A", status: 5, sog: 4 }), 10);
  assert.equal(lostAfter({ kind: "vessel", class: "B", sog: 1 }), 18 * 60);
  assert.equal(lostAfter({ kind: "vessel", class: "B", sog: 6 }), 180);
  assert.equal(lostAfter({ kind: "vessel", class: "B+", sog: 18 }), 90);
  assert.equal(lostAfter({ kind: "aton" }), 18 * 60);
  assert.equal(lostAfter({ kind: "sart" }), 6 * 60);
  assert.equal(lostAfter({ kind: "base" }), 60);
});

test("internet-only targets keep the long allowance", () => {
  assert.equal(lostAfter({ kind: "vessel", sog: 8 }, { internetOnly: true }), 180);
  assert.equal(lostAfter({ kind: "vessel", sog: 0 }, { internetOnly: true }), 600);
  assert.equal(targetLost({ kind: "vessel", sog: 8, last_pos: 1000 }, 1100, { internetOnly: true }), false);
  assert.equal(targetLost({ kind: "vessel", sog: 8, last_pos: 1000 }, 1100), true);
  assert.equal(targetLost({ kind: "vessel" }, 1100), true);
});
