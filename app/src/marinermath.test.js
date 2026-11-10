import test from "node:test";
import assert from "node:assert/strict";
import { destination, circle, parseSpacing, fmtRange, fmtSpacing, displayScale, scaleBar, vectorMarks, trackMarks } from "./marinermath.js";

test("a mile north is a minute of latitude", () => {
  const p = destination([173.28, -41.27], 0, 1);
  assert.ok(Math.abs(p[1] - (-41.27 + 1 / 60)) < 1e-4, JSON.stringify(p));
  assert.ok(Math.abs(p[0] - 173.28) < 1e-9);
});

test("a ring is closed and the same distance all round", () => {
  const c = [174.8, -36.8];
  const r = circle(c, 2, 72);
  assert.deepEqual(r[0], r.at(-1));
  for (const p of r) {
    const dlat = (p[1] - c[1]) * 60, dlon = (p[0] - c[0]) * 60 * Math.cos((c[1] * Math.PI) / 180);
    assert.ok(Math.abs(Math.hypot(dlat, dlon) - 2) < 0.01);
  }
});

test("a ring across 180° stays beside its centre", () => {
  for (const p of circle([179.99, -40], 5)) assert.ok(Math.abs(p[0] - 179.99) < 1);
});

test("spacings the mariner types", () => {
  for (const [s, v] of [["0.25", 0.25], [".5", 0.5], ["1/4", 0.25], ["1 1/2", 1.5], ["¼", 0.25], ["1½", 1.5], ["2 NM", 2], ["0,5", 0.5], ["3/8", 0.375], ["10", 10]]) {
    assert.equal(parseSpacing(s), v, s);
  }
  for (const s of ["", "0", "-1", "abc", "1/0", "200", "1/2/3"]) assert.equal(parseSpacing(s), null, s);
});

test("ranges and spacings are written plainly", () => {
  assert.equal(fmtRange(0.25), "0.25");
  assert.equal(fmtRange(1.5), "1.5");
  assert.equal(fmtRange(0.1 * 3), "0.3");
  assert.equal(fmtSpacing(0.25), "1/4");
  assert.equal(fmtSpacing(2), "2");
});

test("display scale and the S-52 scale bar", () => {
  // zoom 13 at 41°S on a 3.78 px/mm screen: about 1:27,000
  const n = displayScale(-41, 13, 3.78);
  assert.ok(n > 26000 && n < 28500, String(n));
  assert.equal(scaleBar(-41, 13, 3.78, 600).symbol, "SCALEB10");
  assert.equal(scaleBar(-41, 9, 3.78, 600).symbol, "SCALEB11");
  assert.equal(scaleBar(-41, 9, 3.78, 600).nm, 10);
  // just smaller than 1:80,000 (1:109,000 at zoom 11, 41°S) ten miles are 642 px: on a 600 px
  // chart the one-mile bar instead, and the ten-mile scale again once it fits
  assert.ok(scaleBar(-41, 11, 3.78, 300).px > 600);
  const fit = scaleBar(-41, 11, 3.78, 300, 600);
  assert.equal(fit.symbol, "SCALEB10");
  assert.equal(fit.nm, 1);
  assert.equal(scaleBar(-41, 10, 3.78, 300, 600).symbol, "SCALEB11");
  // zoomed right in: a tenth, then a hundredth of a mile
  assert.equal(scaleBar(-41, 16, 3.78, 600).nm, 0.1);
  assert.equal(scaleBar(-41, 18, 3.78, 600).nm, 0.01);
  // a mile at zoom 13: 1852 m / ~7.2 m per px
  const b = scaleBar(-41, 13, 3.78, 600);
  assert.ok(b.px > 240 && b.px < 270, String(b.px));
});

test("vector time marks: every minute, every sixth one big", () => {
  const m = vectorMarks(12, 6);
  assert.equal(m.length, 12);
  assert.equal(m[0].nm, 0.1);
  assert.deepEqual(m.filter((x) => x.big).map((x) => x.nm), [0.6, 1.2]);
  // a long vector: six-minute marks only
  assert.ok(vectorMarks(120, 6).every((x) => x.big));
  assert.deepEqual(vectorMarks(10, 0), []);
});

test("past track marks on whole ten minutes, labels on the half hours", () => {
  const t0 = Date.UTC(2026, 10, 10, 9, 57) / 1000;
  const times = Array.from({ length: 40 }, (_, i) => t0 + i * 60); // one a minute, 09:57–10:36
  const m = trackMarks(times, 10, 30);
  assert.deepEqual(m.map((x) => new Date(x.t * 1000).toISOString().slice(11, 16)), ["10:00", "10:10", "10:20", "10:30"]);
  assert.deepEqual(m.map((x) => x.label), [true, false, false, true]);
  assert.equal(m[0].i, 3);
});
