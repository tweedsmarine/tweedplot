import test from "node:test";
import assert from "node:assert/strict";
import { seaScale, seaTrackRange, filterRangeForGrid, filterColourStyle, fronts, filterable, forecastSpan, sliderStepForTime, weatherSelectionSummary, weatherVisibleInView, filterInUse, filterBounds, dataTimeStatus, applyPointTimePolicy, LAYERS, filterBand, bandDefault, bandOpacity, BAND_FADE } from "./weather.js";

test("automatic filter follows the complete grid at each time without losing extremes", () => {
  const earlier = filterRangeForGrid({ range: [0, 22], values: [0, 20] }, null);
  assert.deepEqual(earlier.filter, { lo: 0, hi: 22, tg: null });
  const later = filterRangeForGrid({ range: [0, 23.2], values: [0, 22] }, earlier.filter);
  assert.ok(later.filter.hi >= 23.2);
  assert.equal(later.filter.lo, 0);
  assert.strictEqual(filterRangeForGrid({ range: [0, 23.2] }, later.filter).filter, later.filter);
  const fallback = filterRangeForGrid({ values: [null, 3, 5, NaN] }, null);
  assert.ok(fallback.min <= 3 && fallback.max >= 5);
  assert.equal(filterRangeForGrid({ values: [null, NaN] }, null), null);
});

test("manual limits and target remain fixed when the forecast range changes", () => {
  const manual = { lo: 10, hi: 22, tg: 18, set: true };
  const next = filterRangeForGrid({ range: [0, 30] }, manual);
  assert.strictEqual(next.filter, manual);
  assert.ok(next.max >= 30);
  const log = filterRangeForGrid({ range: [-3.02, 0.78] }, null);
  assert.ok(log.filter.lo <= -3.02 && log.filter.hi >= 0.78);
  const constant = filterRangeForGrid({ range: [5, 5] }, null);
  assert.ok(constant.filter.lo <= 5 && constant.filter.hi > 5);
});

test("weather is off in Navigation until switched on there, whatever the other views show", () => {
  assert.equal(weatherVisibleInView("nav", false), false);
  assert.equal(weatherVisibleInView("nav", false, true), true);
  assert.equal(weatherVisibleInView("nav", true, true), true);
  for (const view of ["plan", "env"]) {
    assert.equal(weatherVisibleInView(view, false), true);
    assert.equal(weatherVisibleInView(view, true), false);
    assert.equal(weatherVisibleInView(view, true, true), false);
  }
});

test("weather layers keep their fixed scale until a filter is set; only the ends that were set hide values", () => {
  const auto = { lo: 4, hi: 29, tg: null };
  assert.equal(filterInUse(LAYERS.wind, auto), false);
  assert.equal(filterInUse(LAYERS.mslp, auto), false);
  assert.equal(filterInUse(LAYERS.sst, auto), true);
  assert.equal(filterInUse(LAYERS.agree_vec, auto), false);
  assert.equal(filterInUse(LAYERS.wind, null), false);
  const low = { lo: 16, hi: 29, tg: null, set: true, loSet: true, hiSet: false };
  assert.equal(filterInUse(LAYERS.wind, low), true);
  assert.deepEqual(filterBounds(low), { lo: true, hi: false });
  assert.deepEqual(filterBounds({ lo: 10, hi: 20, tg: 15, set: true, loSet: false, hiSet: false }), { lo: false, hi: false });
  // a filter saved before the ends were marked hides at both, as it did
  assert.deepEqual(filterBounds({ lo: 10, hi: 20, set: true }), { lo: true, hi: true });
  assert.deepEqual(filterBounds(auto), { lo: false, hi: false });
});

test("filter track fits the selected range and leaves room to widen at the edges", () => {
  const full = { min: 0, max: 30 };
  const f = { lo: 10, hi: 20, tg: 15 };
  const track = seaTrackRange(f, full);
  assert.ok(track.min < 10 && track.max > 20);
  assert.ok(track.max - track.min < full.max - full.min);
  const narrowed = seaTrackRange({ lo: 14, hi: 16, tg: 15 }, full);
  assert.ok(narrowed.max - narrowed.min < track.max - track.min);
  const widened = seaTrackRange({ lo: narrowed.min, hi: 16 }, full, narrowed, "lo");
  assert.ok(widened.min < narrowed.min);
  const widenedHigh = seaTrackRange({ lo: 14, hi: narrowed.max }, full, narrowed, "hi");
  assert.ok(widenedHigh.max > narrowed.max);
  assert.equal(seaTrackRange({ lo: 0, hi: 30 }, full).min, 0);
  assert.equal(seaTrackRange({ lo: 0, hi: 30 }, full).max, 30);
  const logTrack = seaTrackRange({ lo: -1, hi: 0 }, { min: -3, max: 2 });
  assert.ok(logTrack.min < -1 && logTrack.max > 0);
});

test("past analyses require opt-in and forecast fields never hold outside their own times", () => {
  const analysis = { model: "VIIRS", params: [{ param: "Chlorophyll", times: [100, 200] }] };
  assert.equal(dataTimeStatus(analysis, ["Chlorophyll"], 150).state, "valid");
  assert.equal(dataTimeStatus(analysis, ["Chlorophyll"], 201).state, "missing");
  assert.deepEqual(dataTimeStatus(analysis, ["Chlorophyll"], 201, true), { state: "past", time: 200 });
  assert.equal(dataTimeStatus(analysis, ["Chlorophyll"], 99, true).state, "missing");
  const forecast = { model: "HYCOM", params: [{ param: "SeaTemp", times: [100, 200] }] };
  assert.equal(dataTimeStatus(forecast, ["SeaTemp"], 201, true).state, "missing");
  assert.equal(dataTimeStatus(forecast, ["Salinity"], 150, true).state, "missing");
  assert.equal(applyPointTimePolicy({ chl: 2 }, analysis, 201, false).chl, null);
  assert.deepEqual(applyPointTimePolicy({ chl: 2 }, analysis, 201, true).past, { chl: 200 });
});

test("weather summary distinguishes hidden, unavailable and selected chart layers", () => {
  const state = {
    visible: true, sources: [], layerInfo: { key: "wind", layer: { label: "Wind" }, off: false },
    layerOptions: [{ key: "wind" }], overlayOptions: [{ label: "Isobars", on: true }, { label: "Wave direction", on: false }],
  };
  assert.equal(weatherSelectionSummary(state), "No GRIB loaded");
  state.sources = [{ path: "test" }];
  assert.equal(weatherSelectionSummary(state), "Selected: Wind, Isobars");
  state.layerOptions = [];
  assert.equal(weatherSelectionSummary(state), "Selected: Wind (no data), Isobars");
  state.layerInfo.off = true;
  assert.equal(weatherSelectionSummary(state), "Selected: No shading, Isobars");
  state.visible = false;
  assert.equal(weatherSelectionSummary(state), "Chart weather hidden");
  assert.equal(weatherSelectionSummary({ ...state, view: "nav" }), "Weather is off in Navigation");
});

test("GRIB coverage uses the outer span of available parameter times", () => {
  assert.equal(forecastSpan(null), null);
  assert.equal(forecastSpan({ params: [{ times: [] }] }), null);
  assert.deepEqual(forecastSpan({ params: [{ times: [200, 300] }, { times: [100, 400] }] }), [100, 400]);
  assert.deepEqual(forecastSpan({ params: [{ times: [200] }, { times: [null, 100, 400] }] }), [100, 400]);
});

test("the time slider preserves forecast times on different time grids", () => {
  assert.equal(sliderStepForTime(1700007200, 1700000000, 3600), 3600);
  assert.equal(sliderStepForTime(1700100000, 1700000000, 3600), "any");
  assert.equal(sliderStepForTime(1700107200, 1700000000, 3600), "any");
});

test("sea scale without a target runs evenly from low to high", () => {
  const s = seaScale({ lo: 10, hi: 20, tg: null });
  assert.equal(s[0][0], 10);
  assert.equal(s.at(-1)[0], 20);
  const steps = s.slice(1).map(([v], i) => v - s[i][0]);
  for (const d of steps) assert.ok(Math.abs(d - steps[0]) < 1e-9);
});

test("sea scale with a target puts the middle colour at the target", () => {
  const s = seaScale({ lo: 10, hi: 20, tg: 18 });
  const half = s.length / 2;
  assert.equal(s[0][0], 10);
  assert.ok(Math.abs(s[half][0] - 18) < 1e-9);
  for (let i = 1; i < s.length; i++) assert.ok(s[i][0] > s[i - 1][0], "rising");
});

test("filtered ocean and weather colours fit the limits and put the same middle colour at the target", () => {
  for (const layer of [LAYERS.sst, LAYERS.wind, LAYERS.ssh, LAYERS.chl]) {
    const filter = layer.log ? { lo: -2, hi: 1, tg: -0.5 } : { lo: 10, hi: 20, tg: 18 };
    const { expression, stops } = filterColourStyle(layer, filter, "smooth");
    assert.equal(stops[0][0], filter.lo);
    assert.equal(stops.at(-1)[0], filter.hi);
    assert.equal(stops.find(([value]) => value === filter.tg)[1], "#a4fc3b");
    assert.deepEqual(expression.slice(3), stops.flat(), "chart and bar use identical stops");
    const cleared = filterColourStyle(layer, { ...filter, tg: null }, "smooth").stops;
    assert.equal(cleared[0][0], filter.lo);
    assert.equal(cleared.at(-1)[0], filter.hi);
  }
  const narrow = seaScale({ lo: 1, hi: 1.0001, tg: 1.00008 });
  assert.equal(narrow.at(-1)[0], 1.0001);
  assert.equal(narrow[narrow.length / 2][0], 1.00008);
});

test("banded filtered colours use the same boundaries on the chart and colour bar", () => {
  for (const layer of [LAYERS.wind, LAYERS.ssh]) {
    const filter = { lo: 10, hi: 20, tg: 17.3 };
    const { expression, stops } = filterColourStyle(layer, filter, "2");
    assert.equal(expression[0], "step");
    assert.deepEqual(stops[0], [filter.lo, expression[2]]);
    assert.equal(stops.at(-1)[0], filter.hi);
    const boundaries = [];
    for (let i = 3; i < expression.length; i += 2) {
      boundaries.push(expression[i]);
      assert.deepEqual(stops.filter(([value]) => value === expression[i]).at(-1), [expression[i], expression[i + 1]]);
    }
    assert.ok(boundaries.includes(filter.tg), "target is a band boundary");
    if (layer.knots) assert.ok(boundaries.includes(12) && boundaries.includes(14), "wind band spacing remains in knots");
  }
});

test("fronts: only where the temperature changes fast enough", () => {
  // 5 x 5 grid at 0.1°: flat at 15 °C in the west, 18 °C in the east (a front down the middle)
  const ni = 5, nj = 5;
  const values = [];
  for (let j = 0; j < nj; j++) for (let i = 0; i < ni; i++) values.push(i < 2 ? 15 : 18);
  const gs = { ni, nj, lon0: 170, lat0: -40, dlon: 0.1, dlat: 0.1, values };
  const f = fronts(gs);
  assert.ok(f.length > 0);
  // every cell found is next to the step (columns 1 and 2), none in the flat parts
  for (const c of f) {
    const lon = c.geometry.coordinates[0][0][0] + 0.05;
    assert.ok(Math.abs(lon - 170.1) < 1e-6 || Math.abs(lon - 170.2) < 1e-6, String(lon));
    assert.ok(c.properties.g >= 0.02);
  }
  // a flat field has none, and missing values (land, cloud) are never taken as 0 °C
  assert.equal(fronts({ ...gs, values: values.map(() => 15) }).length, 0);
  assert.equal(fronts({ ...gs, values: values.map((v) => (v === 15 ? null : v)) }).length, 0);
});

test("the filter is for single-file layers, not the agreement or error displays", () => {
  assert.ok(filterable(LAYERS.sst));
  assert.ok(filterable(LAYERS.wind));
  assert.ok(!filterable(LAYERS.agree_vec));
  assert.ok(!filterable(undefined));
});

test("a highlight band: limits, defaults and opacity", () => {
  assert.equal(filterBand({ lo: 14, hi: 26 }), null);
  assert.equal(filterBand({ lo: 14, hi: 26, blo: 20, bhi: 20 }), null);
  assert.deepEqual(filterBand({ lo: 14, hi: 26, blo: 19, bhi: 23 }), [19, 23]);
  assert.deepEqual(bandDefault({ lo: 14, hi: 26 }), [17, 23]);
  // no band: the plain opacity
  assert.equal(bandOpacity({ lo: 14, hi: 26 }, 0.7), 0.7);
  assert.equal(bandOpacity(null, 0.7), 0.7);
  // outside the band at full strength now (BAND_FADE 1): its end colours set it apart
  assert.equal(BAND_FADE, 1);
  assert.equal(bandOpacity({ lo: 14, hi: 26, blo: 19, bhi: 23 }, 0.8), 0.8);
  // a band survives the automatic range: a set filter is kept when the data's range moves
  const kept = filterRangeForGrid({ range: [15, 27] }, { lo: 14, hi: 26, tg: null, blo: 19, bhi: 23, set: true });
  assert.deepEqual(filterBand(kept.filter), [19, 23]);
});

test("with a band the whole palette runs over the band, the ends taking the end colours", () => {
  const f = { lo: 14, hi: 26, tg: null, blo: 19, bhi: 23, set: true };
  const s = seaScale(f);
  assert.equal(s[0][0], 19);
  assert.equal(s.at(-1)[0], 23);
  // a target inside the band is the middle of the palette; one outside it is left out of the colours
  const withTarget = seaScale({ ...f, tg: 21.5 });
  assert.ok(withTarget.some(([v]) => Math.abs(v - 21.5) < 1e-9));
  assert.deepEqual(seaScale({ ...f, tg: 25 }), s);
  // stepped shading: below the band is the first colour, above it the last
  const { expression } = filterColourStyle({ auto: false }, f, "2");
  assert.equal(expression[0], "step");
  assert.ok(expression[3] >= 19 && expression.at(-2) <= 23);
});
