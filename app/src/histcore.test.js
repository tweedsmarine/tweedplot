import test from "node:test";
import assert from "node:assert/strict";
import { Recorder, tendencyWords, unwrap, MIN_MS } from "./histcore.js";

const T0 = Date.UTC(2026, 10, 10, 0, 0, 0);

function feed(r, fromMs, toMs, f) {
  for (let t = fromMs; t < toMs; t += 1000) r.sample(f(t), t);
}

test("minute bins keep the mean, the lowest and the highest (gusts)", () => {
  const r = new Recorder();
  feed(r, T0, T0 + 2 * MIN_MS + 1000, (t) => ({ tws: (t - T0) % MIN_MS === 30_000 ? 25 : 10 }));
  const b = r.bins.TWS;
  assert.equal(b.length, 2);
  assert.ok(Math.abs(b[0].m - (59 * 10 + 25) / 60) < 1e-9);
  assert.equal(b[0].lo, 10);
  assert.equal(b[0].hi, 25);
});

test("a direction is averaged as a vector, through north", () => {
  const r = new Recorder();
  feed(r, T0, T0 + MIN_MS + 1000, (t) => ({ twd: ((t - T0) / 1000) % 2 ? 350 : 10 }));
  const m = r.bins.TWD[0].m;
  assert.ok(m < 0.5 || m > 359.5, String(m));
  assert.equal(r.bins.TWD[0].lo, undefined);
});

test("a gap in the data is not filled in, and the bin closes when data stops", () => {
  const r = new Recorder();
  feed(r, T0, T0 + MIN_MS, () => ({ tws: 12 }));
  feed(r, T0 + MIN_MS, T0 + 5 * MIN_MS, () => ({}));
  feed(r, T0 + 5 * MIN_MS, T0 + 6 * MIN_MS + 1000, () => ({ tws: 14 }));
  assert.deepEqual(r.bins.TWS.map((b) => (b.t - T0) / MIN_MS), [0, 5]);
});

test("old bins are dropped after the hours kept; five-second points after an hour", () => {
  const r = new Recorder({ keepHours: 6 });
  for (let t = T0; t < T0 + 8 * 3600e3; t += 5000) r.sample({ pressure: 1010 }, t);
  const b = r.bins.PRES;
  assert.ok(b[0].t >= T0 + 2 * 3600e3 - MIN_MS);
  assert.ok(r.raw.PRES.length <= 721);
});

test("the stored form round-trips", () => {
  const r = new Recorder();
  feed(r, T0, T0 + 3 * MIN_MS + 1000, (t) => ({ tws: 10 + (t - T0) / MIN_MS, twd: 200, pressure: 1012.3 }));
  const j = JSON.parse(JSON.stringify(r.toJSON()));
  const q = new Recorder();
  assert.ok(q.load(j, T0 + 4 * MIN_MS));
  assert.equal(q.bins.TWS.length, r.bins.TWS.length);
  assert.ok(Math.abs(q.bins.TWS[1].m - r.bins.TWS[1].m) < 0.051);
  assert.equal(q.bins.TWD[0].m, 200);
  assert.equal(q.bins.PRES[0].m, 1012.3);
});

test("three-hour pressure change and the words for it", () => {
  const r = new Recorder();
  // falling 4 hPa in 3 h, sampled every 10 s
  for (let t = T0; t <= T0 + 3 * 3600e3 + MIN_MS; t += 10_000) r.sample({ pressure: 1015 - (4 * (t - T0)) / (3 * 3600e3) }, t);
  const d = r.change("PRES", 3, T0 + 3 * 3600e3 + MIN_MS);
  assert.ok(Math.abs(d + 4) < 0.1, String(d));
  assert.equal(tendencyWords(d), "Falling quickly");
  assert.equal(tendencyWords(0.05), "Steady");
  assert.equal(tendencyWords(1.2), "Rising slowly");
  assert.equal(tendencyWords(-7), "Falling very rapidly");
  // not enough history: no tendency
  const s = new Recorder();
  feed(s, T0, T0 + 10 * MIN_MS, () => ({ pressure: 1000 }));
  assert.equal(s.change("PRES", 3, T0 + 10 * MIN_MS), null);
});

test("series: minute bins, then five-second points, then the bin being filled", () => {
  const r = new Recorder();
  feed(r, T0, T0 + 90 * MIN_MS + 2500, () => ({ sog: 6 }));
  const now = T0 + 90 * MIN_MS + 2500;
  const s = r.series("SOG", now - 3 * 3600e3, now);
  for (let i = 1; i < s.length; i++) assert.ok(s[i].t > s[i - 1].t, `ordered at ${i}`);
  assert.ok(s.at(-1).t >= now - 5000);
  const short = r.series("SOG", now - 15 * MIN_MS, now);
  assert.ok(short.length > 150, String(short.length));
});

test("unwrap makes a veer through north continuous", () => {
  assert.deepEqual(unwrap([350, 355, 5, 15, null, 20]), [350, 355, 365, 375, null, 380]);
});
