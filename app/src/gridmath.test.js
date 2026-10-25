import test from "node:test";
import assert from "node:assert/strict";
import { columns, spanFromDrag, cleanSpan, moveReading, dropIndex, fits, clampTile, snapBelow, placeTile, firstFree } from "./gridmath.js";

test("columns follow width and size", () => {
  assert.equal(columns(330, "m"), 3);
  assert.equal(columns(10, "m"), 1);
  assert.equal(columns(492, "s"), 6);
});

test("span from a drag is snapped and limited", () => {
  assert.equal(spanFromDrag(110, 120, 110), 2);
  assert.equal(spanFromDrag(110, -500, 110), 1);
  assert.equal(spanFromDrag(110, 5000, 110), 6);
  assert.equal(spanFromDrag(110, 10, 0), 1);
});

test("saved spans are cleaned", () => {
  assert.deepEqual(cleanSpan(undefined), [1, 1]);
  assert.deepEqual(cleanSpan([2, 9]), [2, 6]);
  assert.deepEqual(cleanSpan(["x", 0]), [1, 1]);
});

test("moving a reading takes its span along", () => {
  const r = moveReading(["A", "B", "C"], [null, [2, 1], null], 1, 0);
  assert.deepEqual(r.tiles, ["B", "A", "C"]);
  assert.deepEqual(r.spans, [[2, 1], null, null]);
  const e = moveReading(["A", "B", "C"], undefined, 0, 3);
  assert.deepEqual(e.tiles, ["B", "C", "A"]);
});

test("drop position is before or after the target", () => {
  assert.equal(dropIndex(2, 0.2, 0.9), 2);
  assert.equal(dropIndex(2, 0.8, 0.1), 3);
  assert.equal(dropIndex(2, 0.8, 0.1, false), 2);
});

const T = (id, c, r, w = 2, h = 2) => ({ id, k: "SOG", c, r, w, h });

test("tiles must fit the grid and not overlap", () => {
  const a = T("a", 1, 1);
  assert.ok(fits([a], T("b", 3, 1)));
  assert.ok(!fits([a], T("b", 2, 2)));
  assert.ok(!fits([], T("b", 12, 1)));
  assert.ok(fits([a], a)); // a tile does not collide with itself
  assert.deepEqual(clampTile(T("x", 20, 20, 3, 3)), T("x", 10, 6, 3, 3));
});

test("a tile dropped under another locks to its column", () => {
  const above = T("a", 4, 1, 3, 2);
  const s = snapBelow([above], T("b", 5, 4, 2, 2), { c: 5, r: 4 });
  assert.deepEqual([s.c, s.r], [4, 3]);
  const far = snapBelow([above], T("b", 9, 4, 2, 2), { c: 9, r: 4 });
  assert.deepEqual([far.c, far.r], [9, 4]);
});

test("a tile dropped on another moves down its column to a free row", () => {
  const tiles = [T("a", 1, 1), T("b", 1, 3)];
  const p = placeTile(tiles, T("c", 1, 1), 1, 1);
  assert.deepEqual([p.c, p.r], [1, 5]);
  const full = [T("a", 1, 1, 12, 8)];
  assert.equal(placeTile(full, T("c", 1, 1), 1, 1), null);
});

test("first free place reads left to right, then down", () => {
  const f = firstFree([T("a", 1, 1, 12, 2)], 3, 2);
  assert.deepEqual([f.c, f.r, f.w, f.h], [1, 3, 3, 2]);
});
