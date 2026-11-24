import { test } from "node:test";
import assert from "node:assert/strict";
import { chosenReadings, balloonPlace, READINGS, available } from "./readout.js";

const keys = (list) => list.map((x) => x.key);

test("the readout shows the chosen readings the loaded files have", () => {
  const have = new Set(["WindU", "Mslp", "SeaTemp", "SubTemp"]);
  // nothing chosen: the everyday ones
  assert.deepEqual(keys(chosenReadings(have, null)), ["tws", "mslp", "sst", "subt"]);
  // chosen: only those, in the list's order, skipping what no file has
  assert.deepEqual(keys(chosenReadings(have, ["sst", "wave"])), ["sst"]);
  // Show all: the chosen first, then the rest the files have
  assert.deepEqual(keys(chosenReadings(have, ["sst"], true)), ["sst", "tws", "mslp", "subt"]);
  // chosen ones all missing: back to the defaults
  assert.deepEqual(keys(chosenReadings(have, ["chl"])), ["tws", "mslp", "sst", "subt"]);
});

test("files with none of the everyday readings still show something", () => {
  const have = new Set(["Chlorophyll", "Oxygen"]);
  assert.deepEqual(keys(chosenReadings(have, null)), ["chl", "o2"]);
  assert.equal(available(new Set()).length, 0);
});

test("every reading formats its own values and nothing else", () => {
  const row = { tws: 12.4, twd: 225, mslp: 1012.3, cur: { kn: 1.24, set: 90 }, subt: 12.42, depth: -266 };
  const get = (k) => READINGS.find((x) => x.key === k).value(row, 1.5);
  assert.equal(get("tws"), "225° SW 12 kn");
  assert.equal(get("mslp"), "1012 hPa · +1.5 /3 h");
  assert.equal(get("cur"), "1.2 kn toward 090° E");
  assert.equal(get("subt"), "12.4 °C at 266 m");
  assert.equal(get("wave"), false);
});

test("the balloon sits beside the cursor and flips at the edges", () => {
  assert.deepEqual(balloonPlace(100, 100, 200, 120, 1000, 800), [118, 118]);
  assert.deepEqual(balloonPlace(900, 100, 200, 120, 1000, 800), [682, 118]);
  assert.deepEqual(balloonPlace(100, 750, 200, 120, 1000, 800), [118, 612]);
});
