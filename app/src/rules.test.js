import test from "node:test";
import assert from "node:assert/strict";
import { GEAR, applies, ruleFilter, flagsOf, closedFor } from "./rules.js";

/** Evaluate the parts of MapLibre's expression language the filters use. */
function evalExpr(e, props) {
  if (e === true || e === false) return e;
  if (!Array.isArray(e)) return e;
  const [op, ...a] = e;
  switch (op) {
    case "all": return a.every((x) => evalExpr(x, props));
    case "any": return a.some((x) => evalExpr(x, props));
    case "==": return evalExpr(a[0], props) === evalExpr(a[1], props);
    case "!=": return evalExpr(a[0], props) !== evalExpr(a[1], props);
    case "get": return props[a[0]];
    case "coalesce": { for (const x of a) { const v = evalExpr(x, props); if (v != null) return v; } return null; }
    default: throw new Error(op);
  }
}

// gear codes as MPI's: 1 prohibited, 2 restricted
const RULES = [
  { name: "trawl ban", src: 1, prohib: 1, bt: 1, mt: 1 },
  { name: "46 m trawl", src: 1, prohib: 1, v46: 1, bt: 1, mt: 1 },
  { name: "set net ban", src: 1, prohib: 1, sn: 1 },
  { name: "trawl mesh size", src: 1, bt: 2, mt: 2 },
  { name: "marine reserve", src: 1, prohib: 1, allp: 1 },
  { name: "paua rule", src: 1, spec: 1 },
  { name: "mataitai", src: 2, prohib: 1, allp: 1 },
  { name: "taiapure", src: 2 },
  { name: "voluntary", src: 3, prohib: 1, bt: 1 },
];
const names = (s, len) => RULES.filter((r) => applies(r, s, len)).map((r) => r.name);

test("by default only the areas closed to the boat's gear show", () => {
  assert.deepEqual(names({ cust: true, vol: false, mine: true, gear: ["sn"] }, 14), ["set net ban", "marine reserve", "mataitai"]);
  assert.deepEqual(names({ cust: false, vol: true, mine: true, gear: ["bt"] }, 14), ["trawl ban", "marine reserve", "voluntary"]);
  assert.deepEqual(names({ cust: false, vol: true, mine: true, gear: ["bt"] }, 60), ["trawl ban", "46 m trawl", "marine reserve", "voluntary"]);
});

test("with no gear chosen, the areas closed to any gear", () => {
  assert.deepEqual(names({ cust: true, vol: false, mine: false, gear: [] }, 0), ["trawl ban", "46 m trawl", "set net ban", "marine reserve", "mataitai"]);
});

test("restrictions too, when asked", () => {
  const s = { cust: true, vol: false, mine: true, gear: ["sn"], restr: true };
  assert.deepEqual(names(s, 14), ["set net ban", "marine reserve", "paua rule", "mataitai", "taiapure"]);
  assert.deepEqual(names({ ...s, gear: ["bt"] }, 14), ["trawl ban", "trawl mesh size", "marine reserve", "paua rule", "mataitai", "taiapure"]);
  assert.equal(names({ ...s, mine: false }, 0).length, 8);
});

test("a restriction on a method isn't a closure", () => {
  assert.equal(closedFor({ bt: 2 }, ["bt"]), false);
  assert.equal(closedFor({ bt: 1 }, ["bt"]), true);
  assert.equal(closedFor({ bt: 1, prohib: 1 }, ["sn"]), false);
});

test("the map filter agrees with applies() for every gear, length and source choice", () => {
  const lengths = [0, 12, 46, 50];
  for (const g of [[], ...GEAR.map(([k]) => [k]), ["bt", "sn"]])
    for (const len of lengths)
      for (const cust of [true, false])
        for (const vol of [true, false])
          for (const mine of [true, false])
          for (const restr of [true, false]) {
            const s = { cust, vol, mine, gear: g, restr };
            const f = ruleFilter(s, len);
            for (const r of RULES) {
              // tiles carry only the non-zero flags
              assert.equal(evalExpr(f, r), applies(r, s, len), `${r.name} ${JSON.stringify(s)} ${len}`);
            }
          }
});

test("rules_at hits turn into flag objects", () => {
  assert.deepEqual(flagsOf({ rid: 3, flags: [["src", 1], ["sn", 1]] }), { src: 1, sn: 1 });
});
