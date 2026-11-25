import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { collisionAlarmEligible } from "./ais-collision.js";

test("collision eligibility is independent of chart and fleet display filters", () => {
  const target = { mmsi: 123456789, lat: -41, kind: "vessel", status: 0 };
  const settings = { own: { live: true }, ownMmsi: 987654321, enabled: true, lost: false, internetOnly: false, internetAlarms: false, ignoreMoored: true };
  for (const display of [
    { fleetOnly: true, inFleet: false, chartVisible: false, inDisplayRange: false },
    { fleetOnly: false, inFleet: true, chartVisible: true, inDisplayRange: true },
  ]) {
    assert.equal(collisionAlarmEligible(target, { ...settings, ...display }), true);
  }
});

test("collision eligibility preserves alarm, source, own-ship and quality exclusions", () => {
  const target = { mmsi: 123456789, lat: -41, kind: "vessel", status: 0 };
  const settings = { own: { live: true }, ownMmsi: 987654321, enabled: true, lost: false, internetOnly: false, internetAlarms: false, ignoreMoored: true };
  for (const change of [
    { enabled: false }, { own: { live: false } }, { own: null }, { lost: true },
    { ownMmsi: target.mmsi }, { internetOnly: true },
  ]) assert.equal(collisionAlarmEligible(target, { ...settings, ...change }), false);
  for (const change of [{ lat: null }, { kind: "aton" }, { status: 1 }, { status: 5 }]) {
    assert.equal(collisionAlarmEligible({ ...target, ...change }, settings), false);
  }
  assert.equal(collisionAlarmEligible(target, { ...settings, internetOnly: true, internetAlarms: true }), true);
  assert.equal(collisionAlarmEligible({ ...target, status: 1 }, { ...settings, ignoreMoored: false }), true);
});

test("the alarm checker raises and retains a hidden non-fleet collision target", () => {
  const source = readFileSync(new URL("./ais.js", import.meta.url), "utf8");
  const start = source.indexOf("  function checkAlarms() {");
  const end = source.indexOf("  // ---------- data", start);
  assert.ok(start >= 0 && end > start);
  const target = { mmsi: 123456789, lat: -41, lon: 173, kind: "vessel", status: 0, sog: 5, cog: 180, source: "nmea" };
  const own = { live: true, pos: [173, -41] };
  const state = { cpaOn: true, netAlarms: false, ignoreMoored: true, cpa: 0.5, tcpa: 12 };
  const active = new Map(), raised = new Set();
  const alarms = { raise: (id, value) => active.set(id, value), clear: (id) => active.delete(id) };
  let visible = true, lost = false;
  const create = new Function(
    "getOwn", "targets", "ownMmsi", "s", "raised", "alarms", "collisionAlarmEligible", "isLost", "netOnly",
    "cpaTcpa", "isDanger", "bearing", "shownInView", "shown", "isDistress",
    `${source.slice(start, end)}\nreturn checkAlarms;`,
  );
  const check = create(
    () => own, new Map([[target.mmsi, target]]), 987654321, state, raised, alarms, collisionAlarmEligible, () => lost, () => false,
    () => ({ cpa: 0.1, tcpa: 5, range: 1 }), () => true, () => 180, () => visible, () => false, (t) => ["sart", "mob", "epirb"].includes(t.kind),
  );
  check();
  assert.match(active.get("ais-123456789").text, /COLLISION RISK/);
  assert.match(active.get("ais-123456789").detail, /target hidden by display settings/);
  visible = false;
  check();
  assert.ok(active.has("ais-123456789"), "hiding AIS in the view must not clear the alarm");
  lost = true;
  check();
  assert.equal(active.size, 0, "a lost target retains the existing exclusion");
});
