import test from "node:test";
import assert from "node:assert/strict";
import { preset, clean, fromFloating, PRESETS, OWN_SHIP, snapWidth, WIDTHS_MM } from "./sidepresets.js";

// a stand-in for the readings list: every key named in the presets, plus a few
const keys = new Set(OWN_SHIP);
for (const v of Object.values(PRESETS)) for (const f of Object.values(v)) for (const s of f()) { if (s.items) s.items.forEach((k) => keys.add(k)); if (s.item) keys.add(s.item); }
const ITEMS = Object.fromEntries([...keys, "SOUNDER", "HIGHWAY", "WDIAL", "MOBBRG"].map((k) => [k, { label: k, graphic: ["SOUNDER", "HIGHWAY", "WDIAL"].includes(k) }]));

test("every view has a preset for each Use, and an unknown Use is Cruising", () => {
  for (const v of ["nav", "plan"]) for (const u of ["cruise", "fish", "race"]) assert.ok(preset(v, u).length >= 2, `${v}.${u}`);
  assert.deepEqual(preset("nav", null), preset("nav", "cruise"));
});

test("presets are fresh copies (editing one doesn't change the next)", () => {
  const a = preset("nav", "cruise");
  a[0].items.push("X");
  assert.ok(!preset("nav", "cruise")[0].items.includes("X"));
});

test("clean drops readings and sections that no longer exist, and bounds heights", () => {
  const out = clean([
    { id: "a", kind: "readings", title: "A", cols: 2, items: ["SOG", "GONE"] },
    { id: "b", kind: "graphic", item: "GONE" },
    { id: "c", kind: "graphic", item: "WDIAL", h: 999 },
    null, "junk",
  ], ITEMS);
  assert.deepEqual(out.map((s) => s.id), ["a", "c"]);
  assert.deepEqual(out[0].items, ["SOG"]);
  assert.equal(out[1].h, 120);
});

test("the user's own floating panels move in; TweedPlot's own and hidden ones don't", () => {
  const moved = fromFloating([
    { id: "p1", tiles: ["SOG"] },
    { id: "pm", tiles: ["MOBBRG"], mob: true },
    { id: "pabc", name: "Fishing", tiles: ["DPT", "WTEMP", "SOUNDER"], views: ["nav"] },
    { id: "pdef", tiles: ["TWS", "TWD", "AWS", "AWA"] },
    { id: "pghi", tiles: ["SOG"], hidden: true },
    { id: "pjkl", tiles: ["SOG"], views: ["inst"] },
  ], ITEMS);
  assert.equal(moved.nav.length, 3);
  assert.equal(moved.plan.length, 1);
  assert.deepEqual(moved.nav[0], { id: "m-pabc", kind: "readings", title: "Fishing", cols: 1, items: ["DPT", "WTEMP"] });
  assert.equal(moved.nav[1].item, "SOUNDER");
  assert.equal(moved.nav[2].cols, 2);
});

test("width snaps to the nearest of narrow, normal and wide", () => {
  const px = 3.78;
  assert.equal(snapWidth(WIDTHS_MM.narrow * px + 5, px), "narrow");
  assert.equal(snapWidth(WIDTHS_MM.wide * px * 1.5, px, 1.5), "wide");
  assert.equal(snapWidth(10, px), "narrow");
});
