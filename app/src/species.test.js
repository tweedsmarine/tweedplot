import test from "node:test";
import assert from "node:assert/strict";
import { SPECIES, speciesConfig } from "./wxpanel.js";

test("every species preset has a band inside its scale, the target in the band", () => {
  for (const sp of SPECIES) {
    const c = speciesConfig(sp);
    const f = c.filter;
    assert.ok(f.set, sp[0]);
    assert.ok(f.lo < f.blo && f.blo < f.tg && f.tg < f.bhi && f.bhi < f.hi, `${sp[0]}: ${JSON.stringify(f)}`);
    assert.ok(["sst", "subt"].includes(c.layer), sp[0]);
    assert.equal(c.layer === "subt", Number.isFinite(c.depth), sp[0]);
    assert.ok(c.overlays.isotherms && c.overlays.breaks, sp[0]);
  }
});
