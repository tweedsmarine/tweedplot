import test from "node:test";
import assert from "node:assert/strict";
import { parseCursor, recolour, nearEdge } from "./nightcursor.js";

const RING = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24'%3E%3Ccircle cx='12' cy='12' r='7' fill='none' stroke='%23b00000' stroke-width='2.5'/%3E%3C/svg%3E") 12 12, pointer`;
const ARROW = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24'%3E%3Cpath d='M3 2v17z' fill='%2300a000' stroke='%23000' stroke-width='1'/%3E%3C/svg%3E") 3 2, default`;

test("reads a cursor value", () => {
  const c = parseCursor(RING);
  assert.equal(c.hx, 12);
  assert.equal(c.hy, 12);
  assert.equal(c.size, 24);
  assert.ok(c.url.startsWith("data:image/svg+xml"));
  assert.equal(parseCursor("pointer"), null);
  assert.equal(parseCursor("text"), null);
});

test("recolours for the night filter", () => {
  const red = recolour(parseCursor(RING).url, "night-red", 0.6);
  assert.ok(red.includes("stroke='%23000000'"), red); // 176 / 0.6 > 255: black, the brightest after inverting
  const green = recolour(parseCursor(ARROW).url, "night-green", 1);
  assert.ok(green.includes("fill='%235f5f5f'"), green); // 255 - 160
  assert.ok(green.includes("stroke='%23fff'"));
});

test("edge zone", () => {
  assert.equal(nearEdge(500, 400, 1000, 800), false);
  assert.equal(nearEdge(980, 400, 1000, 800), true);
  assert.equal(nearEdge(500, 10, 1000, 800), true);
});
