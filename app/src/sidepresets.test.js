import test from "node:test";
import assert from "node:assert/strict";
import { preset, clean, fromFloating, PRESETS, OWN_SHIP, snapWidth, WIDTHS_MM, upgrade } from "./sidepresets.js";

// a stand-in for the readings list: every key named in the presets, plus a few
const keys = new Set(OWN_SHIP);
for (const v of Object.values(PRESETS)) for (const f of Object.values(v)) for (const s of f()) { if (s.items) s.items.forEach((k) => keys.add(k)); if (s.item) keys.add(s.item); }
const ITEMS = Object.fromEntries([...keys, "SOUNDER", "HIGHWAY", "WDIAL", "MOBBRG"].map((k) => [k, { label: k, graphic: ["SOUNDER", "HIGHWAY", "WDIAL"].includes(k) }]));

test("every view has a preset for each Use, and an unknown Use is Cruising", () => {
  for (const v of ["nav", "plan"]) for (const u of ["cruise", "fish", "race"]) assert.ok(preset(v, u).length >= 1, `${v}.${u}`);
  assert.deepEqual(preset("nav", null), preset("nav", "cruise"));
});

test("planning migration consolidates GRIB readings but preserves custom content", () => {
  const data = { presetsVersion: 2, layouts: { "plan.fish": [
    { id: "sea", kind: "readings", items: ["GSST", "GCUR"] },
    { id: "mine", kind: "readings", items: ["SOG", "GTP"], collapsed: true },
    { id: "grib", kind: "readings", items: ["GTWS", "GTIME"] },
    { id: "cursor", kind: "readings", items: ["CPOS"] },
  ] } };
  assert.ok(upgrade(data));
  assert.deepEqual(data.layouts["plan.fish"].map((s) => s.id), ["weatherTime", "mine", "cursor"]);
  assert.deepEqual(data.layouts["plan.fish"][1].items, ["SOG"]);
  assert.equal(data.layouts["plan.fish"][1].collapsed, true);
  assert.equal(upgrade(data), false);
});

test("Planning starts with chart time, while detailed weather sections remain optional", () => {
  for (const use of ["cruise", "fish", "race"]) {
    assert.ok(!preset("nav", use).some((s) => ["weather", "gribPoint", "gribFiles"].includes(s.kind)));
    assert.equal(preset("plan", use)[0].kind, "weatherTime");
    assert.ok(!preset("plan", use).some((s) => ["weather", "gribPoint"].includes(s.kind)));
  }
  const original = [
    { id: "mine", kind: "readings", title: "Mine", items: ["SOG"], collapsed: true },
    { id: "route", kind: "readings", items: ["BTW"] },
  ];
  const data = { presetsVersion: 3, layouts: { "plan.cruise": structuredClone(original), "nav.cruise": structuredClone(original) } };
  upgrade(data);
  assert.deepEqual(data.layouts["plan.cruise"].slice(1), original);
  assert.deepEqual(data.layouts["nav.cruise"], original);
  const arranged = [data.layouts["plan.cruise"][1], { kind: "gribPoint" }, data.layouts["plan.cruise"][0], { kind: "weather" }, { kind: "gribPoint" }];
  assert.deepEqual(clean(arranged, ITEMS).map((s) => s.kind), ["readings", "gribPoint", "weatherTime", "weather"]);
});

test("forecast time is added once beside existing weather without changing custom layouts", () => {
  const data = { presetsVersion: 4, layouts: {
    "plan.fish": [{ id: "mine", kind: "readings", items: ["SOG"] }, { id: "weather", kind: "weather" }, { id: "gribPoint", kind: "gribPoint" }],
    "plan.race": [{ id: "mine", kind: "readings", items: ["SOG"] }],
    "nav.fish": [{ id: "weather", kind: "weather" }],
  } };
  assert.equal(upgrade(data), true);
  assert.deepEqual(data.layouts["plan.fish"].map((s) => s.kind), ["readings", "weatherTime"]);
  assert.deepEqual(data.layouts["plan.race"].map((s) => s.kind), ["weatherTime", "readings"]);
  assert.deepEqual(data.layouts["nav.fish"].map((s) => s.kind), ["weatherTime", "weather"]);
  assert.equal(upgrade(data), false);
});

test("GRIB file controls move to the drawer without replacing custom sections", () => {
  const data = { presetsVersion: 6, layouts: {
    "plan.fish": [{ id: "mine", kind: "readings", items: ["SOG"], collapsed: true }, { id: "weather", kind: "weather" }, { id: "gribFiles", kind: "gribFiles" }, { id: "gribPoint", kind: "gribPoint" }],
    "nav.fish": [{ id: "weather", kind: "weather" }, { id: "mine", kind: "readings", items: ["SOG"] }],
    "plan.race": [{ id: "mine", kind: "readings", items: ["SOG"] }],
    "plan.cruise": [{ id: "gribFiles", kind: "gribFiles" }, { id: "weather", kind: "weather" }],
  } };
  assert.equal(upgrade(data), true);
  assert.deepEqual(data.layouts["plan.fish"].map((s) => s.kind), ["weatherTime", "readings"]);
  assert.equal(data.layouts["plan.fish"][1].collapsed, true);
  assert.deepEqual(data.layouts["nav.fish"].map((s) => s.kind), ["weather", "readings"]);
  assert.deepEqual(data.layouts["plan.race"].map((s) => s.kind), ["weatherTime", "readings"]);
  assert.deepEqual(data.layouts["plan.cruise"].map((s) => s.kind), ["weatherTime"]);
  assert.equal(upgrade(data), false);
});

test("Planning migration keeps custom sections and leaves other views unchanged", () => {
  const custom = { id: "mine", kind: "readings", items: ["SOG"], collapsed: true };
  const time = { id: "weatherTime", kind: "weatherTime" };
  const detailed = [{ id: "weather", kind: "weather" }, { id: "gribPoint", kind: "gribPoint" }];
  const data = { presetsVersion: 7, layouts: {
    "plan.fish": [custom, detailed[0], time, detailed[1]],
    "plan.cruise": [custom],
    "nav.fish": structuredClone(detailed),
    "env.fish": structuredClone(detailed),
  } };
  assert.equal(upgrade(data), true);
  assert.deepEqual(data.layouts["plan.fish"], [custom, time]);
  assert.deepEqual(data.layouts["plan.cruise"], [{ id: "weatherTime", kind: "weatherTime", title: "Chart time" }, custom]);
  assert.deepEqual(data.layouts["nav.fish"], detailed);
  assert.deepEqual(data.layouts["env.fish"], detailed);
  assert.equal(upgrade(data), false);
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

test("Navigation · Fishing starts without the sounder", () => {
  assert.ok(!preset("nav", "fish").some((s) => s.item === "SOUNDER"));
});

test("upgrade: untouched layouts take the new preset; the user's own keep their sections", () => {
  const data = { v: 1, layouts: {
    "nav.cruise": [{ id: "route", kind: "readings", items: ["NEXT"], collapsed: true }, { id: "wind", kind: "readings", items: [] }],
    "nav.fish": [{ id: "fish", kind: "readings", items: [] }, { id: "snd", kind: "graphic", item: "SOUNDER" }, { id: "sx1", kind: "readings", items: ["SOG"] }],
  } };
  assert.ok(upgrade(data));
  assert.deepEqual(data.layouts["nav.cruise"].map((s) => s.id), preset("nav", "cruise").map((s) => s.id));
  assert.equal(data.layouts["nav.cruise"][0].collapsed, true);
  assert.deepEqual(data.layouts["nav.fish"].map((s) => s.id), ["fish", "sx1"]);
  assert.equal(upgrade(data), false);
});

test("clean keeps marks and targets sections, with a bounded count", () => {
  const out = clean([{ id: "m", kind: "marks", n: 40 }, { id: "t", kind: "targets" }], ITEMS);
  assert.deepEqual(out.map((s) => [s.kind, s.n]), [["marks", 12], ["targets", 5]]);
});
