import test from "node:test";
import assert from "node:assert/strict";
import { Evented } from "maplibre-gl";

class Element {
  constructor(name) {
    this.name = name;
    this.children = [];
    this.listeners = {};
    this.attributes = {};
    this.style = {
      set cssText(value) { this.display = value.includes("display:none") ? "none" : ""; },
    };
    this.captured = new Set();
  }
  append(child) { this.children.push(child); }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener(type, fn) { this.listeners[type] = fn; }
  setPointerCapture(id) { this.captured.add(id); }
  hasPointerCapture(id) { return this.captured.has(id); }
  releasePointerCapture(id) { this.captured.delete(id); }
  emit(type, properties = {}) {
    const event = { preventDefault() {}, ...properties };
    this.listeners[type]?.(event);
  }
}

globalThis.window = { innerWidth: 1000, innerHeight: 800, addEventListener() {} };
globalThis.document = { createElementNS: (_, name) => new Element(name), body: new Element("body") };
const storage = new Map();
globalThis.localStorage = {
  getItem: (key) => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
  removeItem: (key) => storage.delete(key),
};
const { setupVarDiagram } = await import("./varrose.js");

test("diagram can be moved, removed, and placed again", (t) => {
  t.mock.method(globalThis, "setInterval", () => 0);
  const map = Object.assign(new Evented(), {
    getContainer: () => ({ getBoundingClientRect: () => ({ left: 100, top: 50 }) }),
    getBearing: () => 0,
    unproject: ([x, y]) => ({ lng: x, lat: y }),
  });
  const diagram = setupVarDiagram(map, { getVariation: () => 10, getAnnualChange: () => 0 });
  const svg = document.body.children.at(-1);
  assert.equal(svg.style.display, "none");

  diagram.place({ x: 300, y: 350 });
  assert.equal(svg.style.display, "");
  const before = JSON.parse(storage.get("racenav.vardiagram"));
  svg.emit("pointerdown", { button: 0, pointerId: 1, clientX: 400, clientY: 400 });
  svg.emit("pointermove", { pointerId: 2, clientX: 900, clientY: 700 });
  svg.emit("pointermove", { pointerId: 1, clientX: 450, clientY: 425 });
  svg.emit("pointercancel", { pointerId: 1 });
  const moved = JSON.parse(storage.get("racenav.vardiagram"));
  assert.deepEqual(moved, { x: before.x + 50, y: before.y + 25 });

  let clicked;
  map.on("contextmenu", (e) => { clicked = { hit: diagram.hit(), point: e.point, lngLat: e.lngLat }; });
  svg.emit("contextmenu", { clientX: 450, clientY: 425 });
  assert.deepEqual(clicked, { hit: true, point: { x: 350, y: 375 }, lngLat: { lng: 350, lat: 375 } });
  assert.equal(diagram.hit(), false);
  svg.emit("pointerdown", { button: 0, pointerId: 3, clientX: 450, clientY: 425 });
  diagram.remove();
  assert.equal(diagram.shown(), false);
  assert.equal(svg.style.display, "none");
  assert.equal(storage.has("racenav.vardiagram"), false);
  assert.equal(svg.hasPointerCapture(3), false);
  svg.emit("pointermove", { pointerId: 3, clientX: 500, clientY: 500 });
  svg.emit("pointerup", { pointerId: 3 });
  diagram.place({ x: 300, y: 350 });
  assert.equal(svg.style.display, "");
  assert.equal(diagram.shown(), true);
});
