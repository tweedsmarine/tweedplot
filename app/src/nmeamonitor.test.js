import test from "node:test";
import assert from "node:assert/strict";

globalThis.location = { search: "" };
globalThis.window = { __TAURI_INTERNALS__: {} };
const storage = new Map();
globalThis.localStorage = {
  getItem: (key) => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
};
const { openMonitorWindow } = await import("./nmeamonitor.js");

test("the NMEA pop-out opens, focuses an existing window and reports failures", async () => {
  const calls = [];
  let mode = "new";
  window.__TAURI_INTERNALS__.invoke = async (command, args) => {
    calls.push([command, args]);
    if (mode === "error") throw new Error("window failed");
    if (command === "monitor_list") return mode === "new" ? [] : ["monitor-1"];
    if (command === "monitor_open") return "monitor-1";
  };
  const errors = [];
  await openMonitorWindow((...args) => errors.push(args));
  assert.deepEqual(calls.map(([command]) => command), ["monitor_list", "monitor_open"]);
  assert.deepEqual(calls[1][1], { view: "nmea" });
  assert.deepEqual(JSON.parse(storage.get("racenav.windows"))["monitor-1"], { view: "nmea", open: true });

  calls.length = 0;
  mode = "existing";
  await openMonitorWindow((...args) => errors.push(args));
  assert.deepEqual(calls, [["monitor_list", {}], ["monitor_focus", { label: "monitor-1" }]]);

  mode = "error";
  await openMonitorWindow((...args) => errors.push(args));
  assert.match(errors[0][0], /Could not open the live NMEA monitor:.*window failed/);
  assert.equal(errors[0][1], true);
});
