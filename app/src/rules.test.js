import test from "node:test";
import assert from "node:assert/strict";
import { GEAR, applies, ruleFilter, flagsOf } from "./rules.js";

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

const RULES = [
  { name: "trawl ban", src: 1, prohib: 1, bt: 1, mt: 1 },
  { name: "46 m trawl", src: 1, prohib: 1, v46: 1, bt: 1, mt: 1 },
  { name: "set net ban", src: 1, prohib: 1, sn: 1 },
  { name: "marine reserve", src: 1, prohib: 1, allp: 1 },
  { name: "paua rule", src: 1 },
  { name: "mataitai", src: 2, prohib: 1 },
  { name: "voluntary", src: 3, bt: 1 },
];

test("a set netter under 46 m sees set net, all-method, customary and species rules", () => {
  const s = { cust: true, vol: false, mine: true, gear: ["sn"] };
  const shown = RULES.filter((r) => applies(r, s, 14)).map((r) => r.name);
  assert.deepEqual(shown, ["set net ban", "marine reserve", "paua rule", "mataitai"]);
});

test("a 60 m trawler sees the 46 m rules, and voluntary closures when asked", () => {
  const s = { cust: false, vol: true, mine: true, gear: ["bt"] };
  const shown = RULES.filter((r) => applies(r, s, 60)).map((r) => r.name);
  assert.deepEqual(shown, ["trawl ban", "46 m trawl", "marine reserve", "paua rule", "voluntary"]);
});

test("with no gear chosen every rule shows (by source)", () => {
  const s = { cust: true, vol: false, mine: true, gear: [] };
  assert.equal(RULES.filter((r) => applies(r, s, 0)).length, 6);
});

test("the map filter agrees with applies() for every gear, length and source choice", () => {
  const lengths = [0, 12, 46, 50];
  for (const g of [[], ...GEAR.map(([k]) => [k]), ["bt", "sn"]])
    for (const len of lengths)
      for (const cust of [true, false])
        for (const vol of [true, false])
          for (const mine of [true, false]) {
            const s = { cust, vol, mine, gear: g };
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
