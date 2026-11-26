import test from "node:test";
import assert from "node:assert/strict";

// the sums only: the module's view needs the page
const m = await import("./polaredit.js").catch(() => null);

const T = { tws: [6, 12, 20], twa: [45, 90, 150], v: [[3, 5, 6], [4, 6.5, 7.5], [3.8, 6, 8]] };

test("boat speed interpolates as the router does", { skip: !m }, () => {
  assert.equal(m.bsp(T, 90, 12), 6.5);
  assert.equal(m.bsp(T, -90, 12), 6.5);
  assert.ok(Math.abs(m.bsp(T, 67.5, 9) - (3.5 + 5.75) / 2) < 1e-9);
  assert.equal(m.bsp(T, 90, 3), 2); // below the first wind speed: down to 0 at 0 kn
  assert.equal(m.bsp(T, 90, 30), 7.5); // above the last: held
});

test("best VMG up and down", { skip: !m }, () => {
  const up = m.bestVmg(T, 12, true), down = m.bestVmg(T, 12, false);
  assert.ok(up.twa >= 25 && up.twa <= 90 && up.vmg > 0);
  assert.ok(down.twa >= 90 && down.vmg > 0);
});

test("a block from a spreadsheet, and numbers with commas", { skip: !m }, () => {
  assert.deepEqual(m.parseBlock("1\t2\r\n3\t4\r\n"), [["1", "2"], ["3", "4"]]);
  assert.deepEqual(m.parseBlock("TWA\\TWS;4;6\n40;1,8;2,7\n"), [["TWA\\TWS", "4", "6"], ["40", "1,8", "2,7"]]);
  assert.equal(m.num("4,5"), 4.5);
  assert.equal(m.num(""), null);
  assert.equal(m.num("x"), null);
});

test("problems and order", { skip: !m }, () => {
  assert.deepEqual(m.problems(T), {});
  const bad = { ...T, tws: [12, 6, 20], v: [[3, 5, -1], [4, 6.5, 7.5], [3.8, 6, 8]] };
  const p = m.problems(bad);
  assert.ok(p["h:1"] && p["0:2"]);
  const s = m.sorted({ ...T, tws: [12, 6, 20], v: [[5, 3, 6], [6.5, 4, 7.5], [6, 3.8, 8]] });
  assert.deepEqual(s.tws, [6, 12, 20]);
  assert.deepEqual(s.v[0], [3, 5, 6]);
});
