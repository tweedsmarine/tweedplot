import test from "node:test";
import assert from "node:assert/strict";

globalThis.location = { search: "" };
const { setupViews, OPERATIONAL_VIEWS } = await import("./views.js");
const { setupDock } = await import("./dock.js");
const { setupRouteWeather } = await import("./routewx.js");
globalThis.window = { addEventListener() {} };
const { chartViewSettings } = await import("./chartfeatures.js");

test("Environment starts on Base without overriding a saved chart presentation", () => {
  assert.deepEqual(chartViewSettings("env", null), { cat: "base" });
  assert.deepEqual(chartViewSettings("env", { cat: "standard", names: false, ignored: true }), { cat: "standard", names: false });
  assert.deepEqual(chartViewSettings("env", { names: false }), { cat: "base", names: false });
  assert.deepEqual(chartViewSettings("plan", null), {});
});

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

  const buttons = ["nav", "plan", "env", "inst"].map((id) => new Element({ viewSelect: id }));
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

  assert.deepEqual(OPERATIONAL_VIEWS.map((v) => v.id), ["nav", "plan", "env", "inst"]);
  assert.deepEqual(selected(), ["true", "false", "false", "false"]);
  buttons[1].click();
  assert.equal(views.current(), "plan");
  assert.deepEqual(selected(), ["false", "true", "false", "false"]);
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
  assert.deepEqual(selected(), ["false", "false", "false", "true"]);
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
  assert.match(popup.innerHTML, /data-v="env"/);
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
  views.set("env");
  zoom = 3;
  center = { lng: 176, lat: -34 };
  assert.ok(timer, "Environment schedules the inactivity timeout");
  now += 5 * 60000;
  timer();
  assert.equal(views.current(), "nav");
  assert.equal(zoom, 15, "Environment timeout restores Navigation zoom");
  views.set("env");
  assert.equal(zoom, 3, "Environment restores its own zoom");
  assert.deepEqual(center, { lng: 176, lat: -34 });
});

test("the dock can open and close while the rail and drawers remain hidden", () => {
  const classes = new Set(["no-tools"]);
  const root = { classList: { contains: (name) => classes.has(name) }, style: { setProperty(name, value) { this[name] = value; } } };
  const bodyClasses = new Set();
  const body = { classList: { toggle(name, on) { if (on) bodyClasses.add(name); else bodyClasses.delete(name); } } };
  const parts = Object.fromEntries([".dock-tabs", ".dock-tools", ".dock-body", ".dock-close", ".dock-grip"].map((name) => [name, new Element()]));
  parts[".dock-tabs"].append = () => {};
  const dockEl = new Element();
  dockEl.querySelector = (name) => parts[name];
  globalThis.document = { documentElement: root, body, getElementById: () => dockEl, createElement: () => new Element() };
  globalThis.localStorage = { getItem: () => null, setItem() {} };
  const events = [];
  globalThis.window = { dispatchEvent: (event) => events.push(event.type) };
  const dock = setupDock({});
  dock.register("routewx", { title: "Weather on route", render(container) { container.rendered = true; } });

  dock.show("routewx");
  assert.equal(dock.current(), "routewx");
  assert.equal(dockEl.hidden, false);
  assert.equal(parts[".dock-body"].rendered, true);
  assert.equal(root.style["--dock-h"], "260px");
  assert.ok(classes.has("no-tools"));
  assert.ok(!events.includes("racenav-need-tools"));
  dock.show(null);
  assert.equal(dockEl.hidden, true);
  assert.equal(root.style["--dock-h"], "0px");
  assert.equal(bodyClasses.has("dock-open"), false);
});

test("the route-weather button toggles the dock and follows changes from elsewhere", () => {
  const button = new Element();
  const classes = new Set();
  button.classList = { toggle(name, on) { if (on) classes.add(name); else classes.delete(name); } };
  globalThis.document = { getElementById: () => button };
  globalThis.localStorage = { getItem: () => null, setItem() {} };
  let current = null;
  const listeners = [];
  const dock = {
    current: () => current,
    onChange: (fn) => listeners.push(fn),
    register() {}, refresh() {},
    show(name) { current = name; for (const fn of listeners) fn(name); },
    toggle(name) { this.show(current === name ? null : name); },
  };
  const map = { addSource() {}, addLayer() {}, getSource: () => ({ setData() {} }) };
  const weather = { current: () => ({ path: null }), sources: () => [], onLoaded() {}, onChange() {} };
  const routeWx = setupRouteWeather(map, { dock, weather, routing: { onDraw() {} }, course: { onChange() {} } });

  assert.equal(button.attributes["aria-pressed"], "false");
  button.click();
  assert.equal(current, "routewx");
  assert.equal(button.attributes["aria-pressed"], "true");
  assert.ok(classes.has("on"));
  assert.match(button.title, /^Close/);
  button.click();
  assert.equal(current, null);
  assert.equal(button.attributes["aria-pressed"], "false");
  routeWx.open();
  assert.equal(button.attributes["aria-pressed"], "true");
  dock.show("compare");
  assert.equal(button.attributes["aria-pressed"], "false");
  assert.ok(!classes.has("on"));
  button.click();
  dock.show(null);
  assert.equal(button.attributes["aria-pressed"], "false");
});
