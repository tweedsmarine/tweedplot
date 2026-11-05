import test from "node:test";
import assert from "node:assert/strict";
import { hullSymbolFeatures, targetSymbolFeatures } from "./ais-symbols.js";

const symbol = (overrides = {}) => targetSymbolFeatures({
  position: [174.8, -41.3], heading: 35, icon: "ais-s-a", color: "#123456", outline: "#ffffff", zoom: 10, pxPerMm: 3.78, mmsi: 123456789, sortKey: 20,
  ...overrides,
});

test("target polygons are closed, ordered vector features with target identity", () => {
  const features = symbol();
  assert.equal(features.length, 2);
  assert.deepEqual(features.map((f) => f.properties.z), [20, 21]);
  assert.deepEqual(features.map((f) => f.properties.col), ["#ffffff", "#123456"]);
  for (const f of features) {
    const ring = f.geometry.coordinates[0];
    assert.deepEqual(ring[0], ring.at(-1));
    assert.ok(ring.every(([lon, lat]) => Number.isFinite(lon) && Number.isFinite(lat)));
    let area = 0;
    for (let i = 0; i < ring.length - 1; i++) area += ring[i][0] * ring[i + 1][1] - ring[i + 1][0] * ring[i][1];
    assert.ok(area > 0, "outer polygon rings must be counterclockwise");
    assert.equal(f.properties.mmsi, 123456789);
  }
});

test("colored target body and border match the previous raster dimensions", () => {
  const [outline, body] = symbol();
  const symbolBounds = (feature) => {
    const ring = feature.geometry.coordinates[0].slice(0, -1);
    const cosLat = Math.cos((-41.3 * Math.PI) / 180);
    const mPerPixel = (78271.517 * cosLat) / 2 ** 10;
    const angle = 35 * Math.PI / 180;
    const local = ring.map(([lon, lat]) => {
      const east = (lon - 174.8) * 111320 * cosLat / mPerPixel;
      const north = (lat + 41.3) * 111320 / mPerPixel;
      const screenX = east;
      const screenY = -north;
      return [screenX * Math.cos(angle) + screenY * Math.sin(angle), -screenX * Math.sin(angle) + screenY * Math.cos(angle)];
    });
    const xs = local.map(([x]) => x), ys = local.map(([, y]) => y);
    return [Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)];
  };
  const [bodyWidth, bodyLength] = symbolBounds(body);
  const [outlineWidth, outlineLength] = symbolBounds(outline);
  const classAOutlineWidth = 2 * (8 + (2.2 * 8) / Math.hypot(8, 11));
  assert.ok(Math.abs(bodyWidth - 16 * 5.5 * 3.78 / classAOutlineWidth * 0.9667) < 0.1);
  assert.ok(Math.abs(bodyLength - 24 * 5.5 * 3.78 / classAOutlineWidth * 0.9667) < 0.1);
  assert.ok(outlineWidth > bodyWidth + 2);
  assert.ok(outlineLength > bodyLength + 3.5);
});

test("target size follows calibrated CSS pixels per millimetre", () => {
  const [outline] = symbol({ zoom: 10, pxPerMm: 5 });
  const ring = outline.geometry.coordinates[0];
  const cosLat = Math.cos((-41.3 * Math.PI) / 180);
  const mPerPixel = (78271.517 * cosLat) / 2 ** 10;
  const angle = 35 * Math.PI / 180;
  const local = ring.slice(0, -1).map(([lon, lat]) => {
    const east = (lon - 174.8) * 111320 * cosLat / mPerPixel;
    const north = (lat + 41.3) * 111320 / mPerPixel;
    const screenX = east;
    const screenY = -north;
    return [screenX * Math.cos(angle) + screenY * Math.sin(angle), -screenX * Math.sin(angle) + screenY * Math.cos(angle)];
  });
  const xs = local.map(([x]) => x);
  const widthPx = Math.max(...xs) - Math.min(...xs);
  assert.ok(Math.abs(widthPx / 5 - 5.5) < 0.1);
});

test("fishing and fleet marks are vector polygons above the target fill", () => {
  const features = symbol({ icon: "ais-f-bp", fleetColor: "#e67e22" });
  assert.deepEqual(features.map((f) => f.properties.z), [20, 21, 22, 1000024]);
  assert.equal(features.at(-1).properties.col, "#e67e22");
  assert.ok(features.at(-1).geometry.coordinates[0].length > 3);
});

test("special AIS target shapes include their vector details", () => {
  assert.equal(symbol({ icon: "ais-aton" }).length, 4);
  assert.equal(symbol({ icon: "ais-sart" }).length, 4);
  assert.equal(symbol({ icon: "ais-air" }).length, 2);
  assert.equal(symbol({ icon: "ais-base" }).length, 2);
});

test("stationary fishing targets retain their vector crossbar", () => {
  assert.equal(symbol({ icon: "ais-sleep-fish" }).length, 3);
  assert.equal(symbol({ icon: "ais-still-fish" }).length, 3);
});

test("true-scale hulls use the same ordered outline-and-fill polygons", () => {
  const ring = [[174.8, -41.3], [174.8002, -41.3], [174.8002, -41.3001], [174.8, -41.3001], [174.8, -41.3]];
  const features = hullSymbolFeatures({ ring, color: "#123456", outline: "#ffffff", zoom: 12, length: 20, mmsi: 987654321, sortKey: 10 });
  assert.deepEqual(features.map((f) => f.properties.z), [10, 11]);
  assert.deepEqual(features.map((f) => f.properties.col), ["#ffffff", "#123456"]);
  assert.equal(features[1].properties.opacity, 0.45);
  assert.deepEqual(features[0].geometry.coordinates[0][0], features[0].geometry.coordinates[0].at(-1));
});
