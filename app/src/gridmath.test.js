import test from "node:test";
import assert from "node:assert/strict";
import { fits, clampTile, snapBelow, placeTile, firstFree } from "./gridmath.js";

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
