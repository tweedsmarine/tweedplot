import test from "node:test";
import assert from "node:assert/strict";
import { SPECIES, speciesConfig } from "./wxpanel.js";
import { speciesList, checkSpecies, speciesNote } from "./species.js";

test("every species preset has a band inside its scale, the target in the band", () => {
  for (const sp of SPECIES) {
    const c = speciesConfig(sp);
    const f = c.filter;
    assert.ok(f.set, sp[0]);
    assert.ok(f.lo < f.blo && f.blo < f.tg && f.tg < f.bhi && f.bhi < f.hi, `${sp[0]}: ${JSON.stringify(f)}`);
    assert.ok(["sst", "subt"].includes(c.layer), sp[0]);
    assert.equal(c.layer === "subt", Number.isFinite(c.depth), sp[0]);
    assert.ok(c.overlays.isotherms && c.overlays.breaks, sp[0]);
    assert.equal(checkSpecies(speciesList().find((r) => r.name === sp[0])), "", sp[0]);
  }
});

test("the user's figures replace a built-in species' and say what the default was", () => {
  const list = speciesList({ Yellowfin: [17.5, 24, 19] });
  const y = list.find((r) => r.name === "Yellowfin");
  assert.deepEqual([y.blo, y.bhi, y.tg, y.edited], [17.5, 24, 19, true]);
  assert.deepEqual([y.def.blo, y.def.bhi, y.def.tg], [18, 25, 20]);
  assert.match(speciesNote(y), /Your figures\. Default 18–25 °C, target 20 °C/);
  // the others keep following the defaults
  const s = list.find((r) => r.name === "Skipjack");
  assert.ok(!s.edited);
  assert.deepEqual(speciesConfig(y).filter, { lo: 13, hi: 28, tg: 19, blo: 17.5, bhi: 24, set: true });
});

test("the user's own species follow the built-in ones; bad or clashing ones are left out", () => {
  const own = [
    { name: "Kingfish", layer: "sst", depth: null, blo: 17, bhi: 22, tg: 19, note: "" },
    { name: "Swordfish", layer: "subt", depth: 400, blo: 8, bhi: 14, tg: 11, note: "" },
    { name: "Yellowfin", layer: "sst", depth: null, blo: 1, bhi: 30, tg: 10, note: "" },
    { name: "Broken", layer: "sst", depth: null, blo: 22, bhi: 18, tg: 20, note: "" },
  ];
  const list = speciesList({}, own);
  assert.equal(list.length, SPECIES.length + 2);
  const k = list.at(-2), sw = list.at(-1);
  assert.equal(k.name, "Kingfish");
  assert.ok(k.own);
  assert.deepEqual(speciesConfig(sw), { layer: "subt", depth: 400, overlays: speciesConfig(k).overlays, filter: { lo: 4, hi: 18, tg: 11, blo: 8, bhi: 14, set: true } });
  assert.equal(speciesConfig(k).depth, undefined);
});

test("species checks", () => {
  const ok = { name: "A", layer: "sst", blo: 18, bhi: 24, tg: 20 };
  assert.equal(checkSpecies(ok), "");
  assert.match(checkSpecies({ ...ok, name: " " }), /name/);
  assert.match(checkSpecies({ ...ok, tg: 25 }), /between/);
  assert.match(checkSpecies({ ...ok, tg: NaN }), /number/);
  assert.match(checkSpecies({ ...ok, bhi: 40 }), /36/);
  assert.match(checkSpecies({ ...ok, layer: "subt", depth: null }), /depth/);
  assert.equal(checkSpecies({ ...ok, layer: "subt", depth: 250 }), "");
  // damaged storage is ignored
  assert.equal(speciesList({ Yellowfin: "x" }, "y").length, SPECIES.length);
});
