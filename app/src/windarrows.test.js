import test from "node:test";
import assert from "node:assert/strict";
import { setupMapPalette } from "./mappalette.js";
import { apparent, setupWindArrows } from "./windarrows.js";

test("wind arrows point inwards in live and planning views and follow night palettes", () => {
  const root = {
    dataset: { palette: "day" },
    iho: false,
    hasAttribute(name) { return name === "data-iho" && this.iho; },
  };
  let onPaletteChange;
  globalThis.MutationObserver = class {
    constructor(callback) { onPaletteChange = callback; }
    observe() {}
  };
  globalThis.document = {
    documentElement: root,
    createElement() {
      return { getContext: () => ({
        beginPath() {}, moveTo() {}, lineTo() {}, closePath() {}, fill() {},
        getImageData: () => ({ width: 36, height: 110, data: new Uint8ClampedArray(36 * 110 * 4) }),
      }) };
    },
  };
  globalThis.localStorage = { getItem: () => null, setItem() {} };
  const layers = [];
  let data;
  const map = {
    addImage() {},
    addSource() {},
    addLayer(layer) { layers.push(layer); },
    getSource: () => ({ setData(value) { data = value; } }),
    getStyle: () => ({ layers }),
    getPaintProperty: (id, prop) => layers.find((layer) => layer.id === id).paint[prop],
    setPaintProperty(id, prop, value) { layers.find((layer) => layer.id === id).paint[prop] = value; },
  };
  const palette = setupMapPalette(map);
  const arrows = setupWindArrows(map, { before: "boat" });
  const layer = layers[0];
  assert.equal(layer.layout["icon-anchor"], "top");
  assert.deepEqual(layer.paint["icon-color"], ["match", ["get", "kind"], "true", "#2f8fe6", "apparent", "#f57c00", "#2f8fe6"]);

  arrows.live([1, 2], { heading: 350, twd: 0, tws: 15, awa: 30, aws: 12 });
  assert.deepEqual(data.features.map((f) => [f.properties.kind, f.properties.rot]), [["true", 180], ["apparent", 200]]);
  arrows.planning([3, 4], { twd: 270, tws: 20 }, 30, 7);
  const aw = apparent(270, 20, 30, 7);
  assert.deepEqual(data.features.map((f) => f.properties.rot), [90, (aw.awd + 180) % 360]);
  arrows.planning([3, 4], { twd: 270, tws: 20 }, null, null);
  assert.deepEqual(data.features.map((f) => f.properties.kind), ["true"]);

  root.dataset.palette = "iho-night";
  root.iho = true;
  onPaletteChange();
  palette.apply("Night");
  const nightColours = layer.paint["icon-color"];
  assert.match(nightColours[3], /^rgb\(/);
  assert.match(nightColours[5], /^rgb\(/);
  assert.notEqual(nightColours[3], nightColours[5]);
  assert.ok(Math.max(...nightColours[3].match(/\d+/g).map(Number)) < 80);
  assert.ok(Math.max(...nightColours[5].match(/\d+/g).map(Number)) < 80);
  assert.equal(layer.paint["icon-halo-color"], "rgb(0,0,0)");
  assert.equal(layer.paint["icon-opacity"], 0.9);

  root.iho = false;
  onPaletteChange();
  palette.apply(null);
  assert.equal(layer.paint["icon-opacity"], 0.6);
  assert.equal(layer.paint["icon-color"][3], "#2f8fe6");
  root.dataset.palette = "night-red";
  onPaletteChange();
  assert.equal(layer.paint["icon-opacity"], 0.6);
  root.dataset.palette = "night-green";
  onPaletteChange();
  assert.equal(layer.paint["icon-opacity"], 0.6);
  root.dataset.palette = "day";
  onPaletteChange();
  assert.equal(layer.paint["icon-opacity"], 0.9);
});
