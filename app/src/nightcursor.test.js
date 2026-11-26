import test from "node:test";
import assert from "node:assert/strict";
import { parseCursor, recolour, nearEdge, dimmed, nightCopies } from "./nightcursor.js";

const RING = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24'%3E%3Ccircle cx='12' cy='12' r='7' fill='none' stroke='%23b00000' stroke-width='2.5'/%3E%3C/svg%3E") 12 12, pointer`;
const ARROW = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24'%3E%3Cpath d='M3 2v17z' fill='%2300a000' stroke='%23000' stroke-width='1'/%3E%3C/svg%3E") 3 2, default`;

test("reads a cursor value", () => {
  const c = parseCursor(RING);
  assert.equal(c.hx, 12);
  assert.equal(c.hy, 12);
  assert.equal(c.size, 24);
  assert.ok(c.url.startsWith("data:image/svg+xml"));
  assert.equal(parseCursor("pointer"), null);
  assert.equal(parseCursor("text"), null);
});

test("recolours for the night filter", () => {
  const red = recolour(parseCursor(RING).url, "night-red");
  assert.ok(red.includes("stroke='%234f4f4f'"), red); // 255 - 176: the night red after inverting (then dimmed with the page)
  const green = recolour(parseCursor(ARROW).url, "night-green");
  assert.ok(green.includes("fill='%235f5f5f'"), green); // 255 - 160
  assert.ok(green.includes("stroke='%23fff'"));
});

test("edge zone", () => {
  assert.equal(nearEdge(500, 400, 1000, 800), false);
  assert.equal(nearEdge(980, 400, 1000, 800), true);
  assert.equal(nearEdge(500, 10, 1000, 800), true);
});

import { namedFor } from "./nightcursor.js";
test("a system cursor by name gets the night one, from the element or the one it sits in", () => {
  const arrow = 'url("data:image/svg+xml,%3Cpath d=\'M3 2v17l\'") 3 2, default';
  const hand = 'url("data:image/svg+xml,hand") 9 2, pointer';
  assert.equal(namedFor(["pointer"]), "hand");
  assert.equal(namedFor([arrow, "pointer"]), "hand"); // the title inside a clickable heading
  assert.equal(namedFor([arrow, arrow, arrow]), null);
  assert.equal(namedFor([hand, "pointer"]), null); // already a night cursor
  assert.equal(namedFor(["default", "pointer"]), "arrow");
  assert.equal(namedFor(["ns-resize"]), "ns");
  assert.equal(namedFor(["text"]), null); // the text caret stays
});

test("cursors dimmed with Brightness", () => {
  const u = "data:image/svg+xml,%3Cpath fill='%23b00000' stroke='%23555'/%3E";
  assert.equal(dimmed(u, 0.5), "data:image/svg+xml,%3Cpath fill='%23580000' stroke='%232b2b2b'/%3E");
  assert.equal(dimmed(u, 1), "data:image/svg+xml,%3Cpath fill='%23b00000' stroke='%23555555'/%3E");
});

test("night copies of the cursor rules", () => {
  const rule = (sel, cur, imp = "") => ({ selectorText: sel, style: { getPropertyValue: () => cur, getPropertyPriority: () => imp } });
  const out = nightCopies([rule(".sp-sec h4", "pointer"), rule("button:disabled", "default"), rule("input", "text"), rule(".x::before", "pointer"),
    { cssRules: [rule(".m", "grab", "important")] }, rule("html[data-palette] .y", "pointer")]);
  assert.deepEqual(out, [
    ".sp-sec h4 { cursor: var(--ncur-hand, pointer); }",
    "button:disabled { cursor: var(--ncur-arrow, default); }",
    ".m { cursor: var(--ncur-grab, grab) !important; }",
  ]);
});
