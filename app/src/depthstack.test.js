import { test } from "node:test";
import assert from "node:assert/strict";
import { ramp, niceTicks, profileSvg, setArrow } from "./depthstack.js";
import { LAYERS } from "./weather.js";

test("colour ramp ends and middle", () => {
  assert.equal(ramp(["#000000", "#ffffff"], 0), "rgb(0,0,0)");
  assert.equal(ramp(["#000000", "#ffffff"], 1), "rgb(255,255,255)");
  assert.equal(ramp(["#000000", "#ffffff"], 0.5), "rgb(128,128,128)");
  assert.equal(ramp(["#000000", "#ffffff"], 7), "rgb(255,255,255)");
});

test("depth ticks", () => {
  assert.deepEqual(niceTicks(0, 450, 5), [0, 100, 200, 300, 400]);
  assert.deepEqual(niceTicks(12, 18, 4), [12, 14, 16, 18]);
});

test("water column picture", () => {
  const p = { depths: [0.5, 10, 25, 50, 100], temp: [18, 18, 17, 13, null], sal: [35, 35, 35, 35, null], kn: [0.8, 0.7, 0.4, 0.1, null], set: [90, 90, 80, 70, null],
    thermocline: [37.5, 1.6], shear: [17.5, 0.39] };
  const svg = profileSvg(p);
  assert.match(svg, /class="temp"/);
  assert.match(svg, /class="kn"/);
  assert.match(svg, /class="thermo"/);
  assert.match(svg, /class="shear"/);
  // no value at 100 m: the line stops at 50 m (four points, one move and three lines)
  const d = svg.match(/class="temp"/) && svg.match(/<path d="([^"]+)" class="temp"/)[1];
  assert.equal((d.match(/[ML]/g) || []).length, 4);
});

test("current set arrows in the water column", () => {
  assert.match(setArrow(10, 20, 90), /translate\(10\.0,20\.0\) rotate\(90\)/);
  const p = { depths: [0.5, 10, 25, 50, 100], temp: [18, 18, 17, 13, null], sal: [35, 35, 35, 35, null], kn: [0.8, 0.7, 0.4, 0.1, null], set: [90, 90, 80, 70, null] };
  const svg = profileSvg(p, { w: 560, h: 240 });
  const rot = [...svg.matchAll(/class="set" transform="[^"]*rotate\((\d+)\)/g)].map((m) => +m[1]);
  // levels at least 14 px apart: on a 0–50 m axis 202 px tall all four levels with a value get one
  assert.deepEqual(rot, [90, 90, 80, 70]);
  // no set: no arrows
  assert.doesNotMatch(profileSvg({ ...p, set: undefined }), /class="set"/);
});

test("layers below the surface", () => {
  for (const k of ["subt", "subs", "subcur"]) assert.ok(LAYERS[k].depth, k);
  for (const k of ["thermo_depth", "thermo_str", "shear_depth", "shear_str"]) assert.ok(LAYERS[k].stack && LAYERS[k].ocean, k);
});
