// Maritime boundaries (LINZ) and fishing rules (MPI) on the chart: Chart › Boundaries and
// fishing rules. The data comes in the chart tiles (layers "mbound", "rules", "rules_ln",
// "fareas"; see chart::marine); this module styles it, filters the rules to the ones for
// this boat (its gear and length), explains them on a click, and in the main window raises
// an alarm on entering, or heading into, an area closed to the boat's gear.

import { invoke } from "@tauri-apps/api/core";
import { infoBox } from "./infobox.js";
import { PRIMARY } from "./role.js";
import { GEAR, flagsOf, applies, ruleFilter, closedFor, closedExpr } from "./rules.js";

const KEY = "racenav.marine";
const $ = (id) => document.getElementById(id);
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** Boundary kinds (chart::marine::BOUNDARY_KINDS): label, colour, dash. */
const KINDS = {
  ts: ["Territorial sea limit (12 NM)", "#7b3fa0", [6, 3]],
  cz: ["Contiguous zone limit (24 NM)", "#7b3fa0", [2, 3]],
  eez: ["EEZ limit (200 NM)", "#5b2c83", [10, 4, 2, 4]],
  cs: ["Continental shelf limit", "#5b2c83", [2, 2]],
  base: ["Territorial sea baseline", "#9b7bb8", [1, 2]],
  agreed: ["Boundary agreed with Australia", "#5b2c83", [10, 4]],
  other: ["Maritime boundary", "#7b3fa0", [4, 4]],
};

const DEFAULTS = { bounds: true, rules: true, areas: false, cust: true, vol: false, mine: false, restr: false, gear: [], alarm: false, ahead: 10 };

/** Where a boat gets to in `min` minutes at `sog` kn on `cog` °T (flat-earth, short range). */
function ahead(pos, cog, sog, min) {
  const nm = (sog || 0) * min / 60;
  const a = ((cog || 0) * Math.PI) / 180;
  return [pos[0] + (nm * Math.sin(a)) / (60 * Math.cos((pos[1] * Math.PI) / 180)), pos[1] + (nm * Math.cos(a)) / 60];
}

export function setupMarine(map, { getBoat = () => ({}), getLength = () => 0, alarms = null } = {}) {
  let s = { ...DEFAULTS };
  try { Object.assign(s, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ } };

  // ---------- layers ----------------------------------------------------------------
  const src = { source: "chart" };
  // closed (to all, or the boat's methods) red, else by source: customary purple,
  // voluntary grey, restriction orange
  const colourFor = (gear) => ["case", closedExpr(gear), "#d62728", ["==", ["coalesce", ["get", "src"], 1], 2], "#8e44ad",
    ["==", ["coalesce", ["get", "src"], 1], 3], "#6c7a89", "#e67e22"];
  const ruleColour = colourFor([]);
  const below = map.getLayer("land-world") ? "land-world" : undefined; // under the land: MPI areas run inland
  const above = map.getLayer("soundg") ? "soundg" : undefined;
  map.addLayer({ id: "mar-rules-fill", type: "fill", ...src, "source-layer": "rules", layout: { visibility: "none" },
    paint: { "fill-color": ruleColour, "fill-opacity": ["case", ["==", ["coalesce", ["get", "seas"], 0], 1], 0.07, 0.13], "fill-antialias": false } }, below);
  map.addLayer({ id: "mar-rules-line", type: "line", ...src, "source-layer": "rules_ln", layout: { visibility: "none" },
    paint: { "line-color": ruleColour, "line-width": ["interpolate", ["linear"], ["zoom"], 5, 0.8, 11, 1.6], "line-opacity": 0.85 } }, below);
  map.addLayer({ id: "mar-rules-dash", type: "line", ...src, "source-layer": "rules_ln", layout: { visibility: "none" },
    paint: { "line-color": ruleColour, "line-width": ["interpolate", ["linear"], ["zoom"], 5, 0.8, 11, 1.4], "line-dasharray": [3, 2], "line-opacity": 0.85 } }, below);
  map.addLayer({ id: "mar-areas", type: "line", ...src, "source-layer": "fareas", layout: { visibility: "none" },
    paint: { "line-color": "#2e6f8e", "line-width": 1, "line-dasharray": [6, 3], "line-opacity": 0.7 } }, above);
  map.addLayer({ id: "mar-areas-lbl", type: "symbol", ...src, "source-layer": "fareas", minzoom: 6, layout: { visibility: "none",
    "symbol-placement": "line", "symbol-spacing": 400, "icon-image": ["concat", "txt:f:", ["get", "code"]], "icon-rotation-alignment": "viewport", "icon-allow-overlap": false } }, above);
  for (const [k, [, colour, dash]] of Object.entries(KINDS)) {
    map.addLayer({ id: `mar-bound-${k}`, type: "line", ...src, "source-layer": "mbound", filter: ["==", ["get", "kind"], k], layout: { visibility: "none" },
      paint: { "line-color": colour, "line-width": k === "base" ? 1 : 1.6, "line-dasharray": dash } }, above);
  }
  map.addLayer({ id: "mar-bound-lbl", type: "symbol", ...src, "source-layer": "mbound", minzoom: 5, layout: { visibility: "none",
    "symbol-placement": "line", "symbol-spacing": 600,
    "icon-image": ["concat", "txt:m:", ["match", ["get", "kind"], "ts", "12 NM", "cz", "24 NM", "eez", "EEZ 200 NM", "cs", "Continental shelf", "base", "Baseline", "agreed", "Agreed boundary", "Boundary"]],
    "icon-rotation-alignment": "map", "icon-allow-overlap": false } }, above);

  // nothing is drawn until the chart store is open: tiles asked for before then fail (503)
  let ready = false;
  window.addEventListener("racenav-charts", (e) => { if (e.detail?.ready !== false && e.detail?.tables > 0) { ready = true; apply(); } });
  const vis = (id, on) => map.getLayer(id) && map.setLayoutProperty(id, "visibility", on && ready ? "visible" : "none");
  function apply() {
    const len = getLength() || 0;
    for (const k of Object.keys(KINDS)) vis(`mar-bound-${k}`, s.bounds);
    vis("mar-bound-lbl", s.bounds);
    const f = ruleFilter(s, len);
    for (const id of ["mar-rules-fill", "mar-rules-line", "mar-rules-dash"]) vis(id, s.rules);
    const gear = s.mine ? s.gear : [];
    map.setFilter("mar-rules-fill", f);
    map.setFilter("mar-rules-line", ["all", closedExpr(gear), f]);
    map.setFilter("mar-rules-dash", ["all", ["!", closedExpr(gear)], f]);
    for (const id of ["mar-rules-fill", "mar-rules-line", "mar-rules-dash"]) map.setPaintProperty(id, id.endsWith("fill") ? "fill-color" : "line-color", colourFor(gear));
    vis("mar-areas", s.areas);
    vis("mar-areas-lbl", s.areas);
    // the drawer
    $("mar-bounds").checked = s.bounds;
    $("mar-rules").checked = s.rules;
    $("mar-fareas").checked = s.areas;
    $("mar-cust").checked = s.cust;
    $("mar-vol").checked = s.vol;
    $("mar-mine").checked = s.mine;
    $("mar-restr").checked = s.restr;
    $("mar-alarm").checked = s.alarm;
    $("mar-ahead").value = String(s.ahead);
    for (const cb of $("mar-gear").querySelectorAll("input")) cb.checked = s.gear.includes(cb.value);
    $("mar-gear").classList.toggle("dim", !s.mine);
    $("mar-len").textContent = len > 0
      ? `Vessel length ${len} m (Settings): ${len > 46 ? "the trawl rules for vessels over 46 m apply" : "the trawl rules for vessels over 46 m are left out"}.`
      : "Vessel length not set (Settings › Vessel): the trawl rules for vessels over 46 m are shown too.";
  }

  // gear checkboxes
  $("mar-gear").innerHTML = GEAR.map(([k, name]) => `<label><input type="checkbox" value="${k}" /> ${esc(name)}</label>`).join("");
  $("mar-gear").onchange = () => {
    s.gear = [...$("mar-gear").querySelectorAll("input:checked")].map((cb) => cb.value);
    if (s.gear.length) s.mine = true;
    save(); apply(); check();
  };
  for (const [id, key] of [["mar-bounds", "bounds"], ["mar-rules", "rules"], ["mar-fareas", "areas"], ["mar-cust", "cust"], ["mar-vol", "vol"], ["mar-mine", "mine"], ["mar-restr", "restr"], ["mar-alarm", "alarm"]]) {
    $(id).onchange = (e) => { s[key] = e.target.checked; save(); apply(); check(); };
  }
  $("mar-ahead").onchange = (e) => { s.ahead = +e.target.value || 10; save(); check(); };
  window.addEventListener("racenav-boat", apply);

  // ---------- details on a click ------------------------------------------------------
  const infoCache = new Map();
  const info = async (rid) => {
    if (!infoCache.has(rid)) infoCache.set(rid, await invoke("rule_info", { rid }).catch(() => null));
    return infoCache.get(rid);
  };
  window.addEventListener("racenav-charts", () => infoCache.clear());

  const gearNames = (f, code) => GEAR.filter(([k]) => f[k] === code).map(([, n]) => n);
  const field = (a, k) => { const v = a?.[k] ?? a?.[Object.keys(a || {}).find((x) => x.toLowerCase() === k.toLowerCase())]; return v == null || v === "" ? null : v; };
  function ruleHtml(hit, inf) {
    const f = flagsOf(hit), a = inf?.attrs || {};
    const shut = closedFor(f, s.mine ? s.gear : []);
    const kind = f.src === 2 ? "Customary area" : f.src === 3 ? "Voluntary closure" : shut ? "Closed" : "Restriction";
    const gear = f.allp ? "All commercial fishing prohibited" : [gearNames(f, 1).length ? `Prohibited: ${gearNames(f, 1).join(", ")}` : "", gearNames(f, 2).length ? `Restricted: ${gearNames(f, 2).join(", ")}` : ""].filter(Boolean).join(". ");
    const season = f.seas ? [field(a, "SeasonStart"), field(a, "SeasonEnd")].filter(Boolean).join(" to ") || "seasonal" : null;
    const rows = [
      ["What", field(a, "ShortDescription") || field(a, "Rule_")],
      ["Methods", gear || null],
      ["Species", field(a, "SpeciesCode")],
      ["Vessels", f.v46 ? "Over 46 m" : f.vlim ? "Some vessel sizes (see the regulation)" : null],
      ["Season", season],
      ["Conditions", field(a, "SpecialConditions")],
      ["Category", inf?.layers || null],
      ["Regulation", [field(a, "Instrument") || field(a, "LegalInstrument"), field(a, "SectionRule")].filter(Boolean).join(", ") || null],
      ["Responsible", field(a, "ResponsibleOrg")],
      ["Temporary", f.tmp ? "Yes (check it is still in force)" : null],
    ].filter(([, v]) => v);
    const url = [field(a, "RegulationURL"), field(a, "SourceURL"), field(a, "MetadataURL")].find((u) => typeof u === "string" && /^https?:/.test(u.trim()));
    const legal = field(a, "LegalDescription");
    return `<h4><span class="mar-tag ${shut ? "p" : f.src === 2 ? "c" : f.src === 3 ? "v" : "r"}">${kind}</span> ${esc(field(a, "Name") || field(a, "Location") || "Fishing rule")}</h4>
      <table>${rows.map(([k, v]) => `<tr><td>${k}</td><td>${esc(v)}</td></tr>`).join("")}</table>
      ${legal ? `<details><summary>Legal description</summary><p>${esc(legal)}</p></details>` : ""}
      ${url ? `<button class="mar-link" data-url="${esc(url)}">Open the regulation</button>` : ""}`;
  }

  map.on("click", async (e) => {
    if (!s.rules || document.querySelector(".armed")) return;
    const pad = 6;
    const box = [[e.point.x - pad, e.point.y - pad], [e.point.x + pad, e.point.y + pad]];
    // chart symbols and the app's own things keep their own info
    const other = map.queryRenderedFeatures(box).some((f) => /^(f-points|f-lights|f-line|rt-|lib-|fl-|dl-|boat|tide-|stn-|ais-|alarm-)/.test(f.layer.id));
    if (other || !map.queryRenderedFeatures(e.point, { layers: ["mar-rules-fill"] }).length) return;
    const [hits] = await invoke("rules_at", { points: [[e.lngLat.lng, e.lngLat.lat]] }).catch(() => [[]]);
    const len = getLength() || 0;
    const shown = (hits || []).filter((h) => applies(flagsOf(h), s, len)).slice(0, 8);
    if (!shown.length) return;
    const infos = await Promise.all(shown.map((h) => info(h.rid)));
    const el = document.createElement("div");
    el.className = "feat-pop mar-pop";
    el.innerHTML = shown.map((h, i) => ruleHtml(h, infos[i])).join("") +
      `<div class="wx-note">${s.mine && s.gear.length ? "The rules for your gear and vessel here. " : ""}Fishing rules: Ministry for Primary Industries (CC BY 4.0), a guide only: the regulations are the law. Not for navigation.</div>`;
    for (const b of el.querySelectorAll(".mar-link")) b.onclick = () => invoke("open_url", { url: b.dataset.url }).catch(() => {});
    infoBox(map, "chart", { width: 360, height: 360 }).open(el, { title: shown.length > 1 ? `${shown.length} fishing rules here` : "Fishing rule", at: [e.lngLat.lng, e.lngLat.lat] });
  });

  // ---------- closed-area alarm (main window) -----------------------------------------
  const ALARM_IN = "mar-closed-in", ALARM_AHEAD = "mar-closed-ahead";
  let busy = false;
  async function check() {
    if (!PRIMARY || !alarms || busy) return;
    const b = getBoat();
    if (!s.alarm || !s.rules || !b.pos || !b.live) { alarms.clear(ALARM_IN); alarms.clear(ALARM_AHEAD); return; }
    busy = true;
    try {
      const pts = [b.pos];
      const moving = (b.sog || 0) > 0.5 && b.cog != null;
      if (moving) pts.push(ahead(b.pos, b.cog, b.sog, s.ahead));
      const res = await invoke("rules_at", { points: pts });
      const len = getLength() || 0;
      const closed = (hits) => (hits || []).filter((h) => applies(flagsOf(h), { ...s, mine: s.gear.length > 0, restr: false }, len));
      const here = closed(res[0]);
      const there = moving ? closed(res[1]).filter((h) => !here.some((x) => x.rid === h.rid)) : [];
      const names = async (list) => (await Promise.all(list.slice(0, 3).map((h) => info(h.rid)))).map((i) => field(i?.attrs, "Name") || "closed area").join("; ");
      const forGear = s.gear.length ? ` to ${GEAR.filter(([k]) => s.gear.includes(k)).map(([, n]) => n.toLowerCase()).join(", ")}` : "";
      if (here.length) alarms.raise(ALARM_IN, { level: "warn", text: `Inside an area closed${forGear}`, detail: await names(here) });
      else alarms.clear(ALARM_IN);
      if (there.length) alarms.raise(ALARM_AHEAD, { level: "warn", text: `Closed area${forGear} ahead (within ${s.ahead} min)`, detail: await names(there) });
      else alarms.clear(ALARM_AHEAD);
    } catch { /* charts not ready */ } finally { busy = false; }
  }
  if (PRIMARY) setInterval(check, 15000);

  window.addEventListener("racenav-charts", apply);
  apply();
  return { apply, settings: () => ({ ...s }) };
}
