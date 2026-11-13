import test from "node:test";
import assert from "node:assert/strict";

globalThis.location = { search: "" };
const { setupViews, OPERATIONAL_VIEWS } = await import("./views.js");

class Element {
  constructor(dataset = {}) {
    this.dataset = dataset;
    this.attributes = {};
    this.handlers = {};
    this.classList = { toggle() {} };
    this.style = {};
  }
  addEventListener(type, fn) { (this.handlers[type] ??= []).push(fn); }
  click() { this.onclick?.(); for (const fn of this.handlers.click ?? []) fn({ target: this }); }
  setAttribute(name, value) { this.attributes[name] = value; }
  querySelector() { return this.child; }
  getBoundingClientRect() { return { right: 60, top: 0 }; }
}

test("view controls and chart positions stay in sync, including the Planning timeout", (t) => {
  const originalNow = Date.now;
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  let now = 0;
  let timer = null;
  Date.now = () => now;
  globalThis.setTimeout = (fn) => { timer = fn; return fn; };
  globalThis.clearTimeout = (id) => { if (timer === id) timer = null; };
  t.after(() => {
    Date.now = originalNow;
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  });
  const buttons = ["nav", "plan", "inst"].map((id) => new Element({ viewSelect: id }));
  const viewButton = new Element();
  viewButton.child = new Element();
  const toolsButton = new Element();
  const timeoutSelect = new Element();
  const popup = new Element();
  popup.child = new Element();
  const root = new Element();
  const document = {
    documentElement: root,
    body: { append() {} },
    getElementById: (id) => ({ "view-btn": viewButton, "tools-btn": toolsButton, "planning-timeout": timeoutSelect })[id],
    querySelectorAll: () => buttons,
    createElement: () => popup,
    addEventListener() {},
  };
  const listeners = {};
  const window = {
    innerHeight: 800,
    addEventListener(type, fn) { (listeners[type] ??= []).push(fn); },
    dispatchEvent(event) { for (const fn of listeners[event.type] ?? []) fn(event); },
  };
  globalThis.document = document;
  globalThis.window = window;
  globalThis.localStorage = { getItem: () => null, setItem() {} };
  globalThis.requestAnimationFrame = () => {};
  globalThis.CustomEvent = class { constructor(type, options) { this.type = type; this.detail = options.detail; } };
  let zoom = 12;
  let center = { lng: 173, lat: -41 };
  const map = {
    resize() {}, stop() {}, getZoom: () => zoom, getCenter: () => center,
    jumpTo: (opts) => { zoom = opts.zoom; if (opts.center) center = { lng: opts.center[0], lat: opts.center[1] }; },
  };
  const views = setupViews(map);
  const selected = () => buttons.map((b) => b.attributes["aria-pressed"]);

  assert.deepEqual(OPERATIONAL_VIEWS.map((v) => v.id), ["nav", "plan", "inst"]);
  assert.deepEqual(selected(), ["true", "false", "false"]);
  buttons[1].click();
  assert.equal(views.current(), "plan");
  assert.deepEqual(selected(), ["false", "true", "false"]);
  assert.equal(zoom, 12, "first entry keeps the current chart zoom");
  assert.deepEqual(center, { lng: 173, lat: -41 }, "first entry keeps the current centre");
  zoom = 6;
  center.lng = 175;
  center.lat = -38;
  views.set("nav");
  assert.equal(zoom, 12, "Navigation restores its own zoom");
  assert.deepEqual(center, { lng: 173, lat: -41 }, "first return restores the saved Navigation centre");
  zoom = 14;
  center = { lng: 174, lat: -40 };
  views.set("plan");
  assert.equal(zoom, 6, "Planning restores its own zoom");
  assert.deepEqual(center, { lng: 175, lat: -38 }, "Planning restores its own centre");
  views.set("inst");
  assert.deepEqual(selected(), ["false", "false", "true"]);
  zoom = 4;
  views.set("nav");
  assert.equal(zoom, 14, "returning from Instruments restores Navigation zoom");
  assert.deepEqual(center, { lng: 174, lat: -40 }, "returning from Instruments restores Navigation centre");
  views.set("inst");
  views.set("plan");
  assert.equal(zoom, 6, "returning from Instruments restores Planning zoom");
  assert.deepEqual(center, { lng: 175, lat: -38 }, "returning from Instruments restores Planning centre");
  views.set("inst");
  viewButton.click();
  assert.match(popup.innerHTML, /data-v="nav"/);
  assert.match(popup.innerHTML, /data-v="plan"/);
  assert.match(popup.innerHTML, /data-v="inst"/);
  assert.doesNotMatch(popup.innerHTML, /data-v="nmea"/);
  views.set("nav");
  zoom = 15;
  center = { lng: 171, lat: -42 };
  views.set("plan");
  zoom = 5;
  center = { lng: 178, lat: -35 };
  assert.ok(timer, "Planning schedules the inactivity timeout");
  now += 5 * 60000;
  timer();
  assert.equal(views.current(), "nav");
  assert.equal(zoom, 15, "timeout restores Navigation zoom");
  assert.deepEqual(center, { lng: 171, lat: -42 }, "timeout restores Navigation centre");
  views.set("plan");
  assert.equal(zoom, 5, "Planning retains its zoom after timeout");
  assert.deepEqual(center, { lng: 178, lat: -35 }, "Planning retains its centre after timeout");
});
