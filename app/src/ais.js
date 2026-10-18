// AIS targets on the chart: from Signal K (a receiver on board) and online from
// aisstream.io shore stations, merged in Rust by MMSI. Symbols by kind and
// ship type, a 6-minute course vector, names, a popup, a target list with CPA
// and TCPA, and collision / distress alarms.
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ddm } from "./navmath.js";
import { distNm, bearing } from "./instruments.js";

const KEY = "racenav.ais";
const DEFAULTS = { show: true, names: true, key: "", online: false, cpaOn: true, cpa: 0.5, tcpa: 12, ignoreMoored: true, sort: "range" };
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const STATUS = ["Under way using engine", "At anchor", "Not under command", "Restricted manoeuvrability", "Constrained by draught", "Moored", "Aground",
  "Engaged in fishing", "Under way sailing", "", "", "Towing astern", "Pushing ahead or towing alongside", "", "AIS-SART active", ""];
/** Ship type → [category, colour]. */
function category(t) {
  if (t == null) return ["Unknown", "#8a96a0"];
  if (t === 30) return ["Fishing", "#e8792e"];
  if (t === 36 || t === 37) return [t === 36 ? "Sailing" : "Pleasure craft", "#9b59b6"];
  if (t === 31 || t === 32 || t === 52) return ["Tug / towing", "#1abc9c"];
  if (t >= 50 && t <= 59) return [{ 50: "Pilot", 51: "Search and rescue", 53: "Port tender", 54: "Anti-pollution", 55: "Law enforcement", 58: "Medical" }[t] ?? "Special craft", "#1abc9c"];
  if (t >= 40 && t <= 49) return ["High-speed craft", "#f1c40f"];
  if (t >= 60 && t <= 69) return ["Passenger", "#2e6fd8"];
  if (t >= 70 && t <= 79) return ["Cargo", "#27ae60"];
  if (t >= 80 && t <= 89) return ["Tanker", "#c0392b"];
  if (t === 33) return ["Dredging", "#1abc9c"];
  if (t === 35) return ["Military", "#5d6d7e"];
  return ["Other", "#8a96a0"];
}
const ATON = { 1: "Reference point", 2: "RACON", 3: "Fixed structure", 5: "Light", 9: "Beacon, cardinal N", 13: "Beacon, port", 14: "Beacon, starboard",
  20: "Cardinal buoy N", 21: "Cardinal buoy E", 22: "Cardinal buoy S", 23: "Cardinal buoy W", 24: "Port buoy", 25: "Starboard buoy", 28: "Isolated danger buoy", 29: "Safe water buoy", 30: "Special buoy", 31: "Light vessel / LANBY" };

/** Closest point of approach of target `t` to own boat `o` ({pos, cog, sog}): {cpa NM, tcpa min} or null. */
export function cpaTcpa(o, t) {
  if (!o?.pos || t.lat == null) return null;
  const lat0 = (o.pos[1] * Math.PI) / 180;
  const p = [(t.lon - o.pos[0]) * 60 * Math.cos(lat0), (t.lat - o.pos[1]) * 60]; // NM east, north
  const vel = (sog, cog) => (sog && cog != null ? [sog * Math.sin((cog * Math.PI) / 180), sog * Math.cos((cog * Math.PI) / 180)] : [0, 0]);
  const vo = vel(o.sog, o.cog), vt = vel(t.sog, t.cog);
  const v = [vt[0] - vo[0], vt[1] - vo[1]];
  const v2 = v[0] * v[0] + v[1] * v[1];
  const range = Math.hypot(p[0], p[1]);
  if (v2 < 1e-6) return { cpa: range, tcpa: null, range };
  const th = -(p[0] * v[0] + p[1] * v[1]) / v2; // hours
  const c = [p[0] + v[0] * th, p[1] + v[1] * th];
  return { cpa: Math.hypot(c[0], c[1]), tcpa: th * 60, range };
}

function addIcons(map) {
  const make = (draw, size = 32) => {
    const c = document.createElement("canvas");
    c.width = c.height = size;
    const g = c.getContext("2d");
    draw(g, size);
    return g.getImageData(0, 0, size, size);
  };
  const ship = (g, s, fill) => {
    g.beginPath(); g.moveTo(s / 2, 3); g.lineTo(s / 2 + 8, s - 5); g.lineTo(s / 2, s - 9); g.lineTo(s / 2 - 8, s - 5); g.closePath();
    if (fill) g.fill(); g.lineWidth = 2.5; g.stroke();
  };
  // SDF-free: white fill drawn later with icon-color needs sdf; use sdf images
  map.addImage("ais-a", make((g, s) => { g.fillStyle = g.strokeStyle = "#fff"; ship(g, s, true); }), { sdf: true });
  map.addImage("ais-b", make((g, s) => { g.fillStyle = g.strokeStyle = "#fff"; g.save(); g.translate(s / 2, s / 2); g.scale(0.8, 0.8); g.translate(-s / 2, -s / 2); ship(g, s, true); g.restore(); }), { sdf: true });
  map.addImage("ais-still", make((g, s) => { g.fillStyle = g.strokeStyle = "#fff"; g.beginPath(); g.arc(s / 2, s / 2, 6, 0, 2 * Math.PI); g.fill(); }), { sdf: true });
  map.addImage("ais-aton", make((g, s) => { g.strokeStyle = "#fff"; g.lineWidth = 3; g.beginPath(); g.moveTo(s / 2, 5); g.lineTo(s - 6, s / 2); g.lineTo(s / 2, s - 5); g.lineTo(6, s / 2); g.closePath(); g.stroke();
    g.beginPath(); g.moveTo(s / 2 - 4, s / 2); g.lineTo(s / 2 + 4, s / 2); g.moveTo(s / 2, s / 2 - 4); g.lineTo(s / 2, s / 2 + 4); g.stroke(); }), { sdf: true });
  map.addImage("ais-air", make((g, s) => { g.fillStyle = "#fff"; g.beginPath(); g.moveTo(s / 2, 3); g.lineTo(s / 2 + 2, 12); g.lineTo(s - 3, 18); g.lineTo(s - 3, 21); g.lineTo(s / 2 + 2, 17); g.lineTo(s / 2 + 2, 25); g.lineTo(s / 2 + 6, 28); g.lineTo(s / 2 - 6, 28); g.lineTo(s / 2 - 2, 25);
    g.lineTo(s / 2 - 2, 17); g.lineTo(3, 21); g.lineTo(3, 18); g.lineTo(s / 2 - 2, 12); g.closePath(); g.fill(); }), { sdf: true });
  map.addImage("ais-base", make((g, s) => { g.strokeStyle = "#fff"; g.lineWidth = 3; g.strokeRect(8, 8, s - 16, s - 16); }), { sdf: true });
  map.addImage("ais-sart", make((g, s) => { g.strokeStyle = "#fff"; g.lineWidth = 3; g.beginPath(); g.arc(s / 2, s / 2, s / 2 - 4, 0, 2 * Math.PI); g.stroke();
    g.beginPath(); g.moveTo(9, 9); g.lineTo(s - 9, s - 9); g.moveTo(s - 9, 9); g.lineTo(9, s - 9); g.stroke(); }), { sdf: true });
}

export function setupAis(map, { getOwn, alarms, ui }) {
  let s = { ...DEFAULTS };
  try { Object.assign(s, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ } };
  const targets = new Map(); // mmsi → target
  let online = { connected: false, detail: "off" };
  let selected = null;

  addIcons(map);
  map.addSource("ais", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "ais-vec", type: "line", source: "ais", filter: ["==", ["get", "k"], "vec"],
    paint: { "line-color": ["get", "col"], "line-width": 1.5 } });
  map.addLayer({ id: "ais-sym", type: "symbol", source: "ais", filter: ["==", ["get", "k"], "t"],
    layout: { "icon-image": ["get", "icon"], "icon-rotate": ["get", "rot"], "icon-rotation-alignment": "map", "icon-allow-overlap": true, "icon-ignore-placement": true,
      "icon-size": ["interpolate", ["linear"], ["zoom"], 6, 0.5, 12, 0.8] },
    paint: { "icon-color": ["get", "col"], "icon-halo-color": "#1d2a33", "icon-halo-width": 1.2, "icon-opacity": ["case", ["get", "lost"], 0.45, 1] } });
  map.addLayer({ id: "ais-names", type: "symbol", source: "ais", filter: ["all", ["==", ["get", "k"], "t"], ["has", "name"]], minzoom: 10,
    layout: { "icon-image": ["concat", "lbl:", ["get", "name"]], "icon-anchor": "top", "icon-offset": [0, 12], "icon-allow-overlap": false } });

  const isLost = (t) => {
    if (t.last_pos == null) return true;
    const age = Date.now() / 1000 - t.last_pos;
    const moving = (t.sog ?? 0) > 2;
    return t.kind === "aton" || t.kind === "base" ? age > 20 * 60 : age > (moving ? 3 : 10) * 60;
  };
  function style(t) {
    if (t.kind === "sart" || t.kind === "mob" || t.kind === "epirb") return ["ais-sart", "#d00000", 0];
    if (t.kind === "aton") return ["ais-aton", t.virtual_aton ? "#c2185b" : "#6a1b9a", 0];
    if (t.kind === "aircraft") return ["ais-air", "#e67e22", t.cog ?? 0];
    if (t.kind === "base") return ["ais-base", "#5d6d7e", 0];
    const col = category(t.ship_type)[1];
    const dir = t.heading ?? ((t.sog ?? 0) > 0.5 ? t.cog : null);
    if (dir == null) return ["ais-still", col, 0];
    return [t.class === "B" ? "ais-b" : "ais-a", col, dir];
  }
  function draw() {
    const vis = s.show ? "visible" : "none";
    for (const id of ["ais-vec", "ais-sym"]) map.setLayoutProperty(id, "visibility", vis);
    map.setLayoutProperty("ais-names", "visibility", s.show && s.names ? "visible" : "none");
    const f = [];
    for (const t of targets.values()) {
      if (t.lat == null) continue;
      const [icon, col, rot] = style(t);
      const lost = isLost(t);
      const props = { k: "t", mmsi: t.mmsi, icon, col: t.mmsi === selected ? "#ffd400" : col, rot, lost };
      if (t.name) props.name = t.name;
      f.push({ type: "Feature", properties: props, geometry: { type: "Point", coordinates: [t.lon, t.lat] } });
      // 6-minute course vector
      if (!lost && (t.sog ?? 0) > 0.5 && t.cog != null && t.kind !== "aton") {
        const d = (t.sog / 10) / 60; // degrees of latitude in 6 min
        const end = [t.lon + (d * Math.sin((t.cog * Math.PI) / 180)) / Math.cos((t.lat * Math.PI) / 180), t.lat + d * Math.cos((t.cog * Math.PI) / 180)];
        f.push({ type: "Feature", properties: { k: "vec", col }, geometry: { type: "LineString", coordinates: [[t.lon, t.lat], end] } });
      }
    }
    map.getSource("ais").setData({ type: "FeatureCollection", features: f });
  }

  // ---------- popup ----------------------------------------------------------------
  let popup = null;
  function describe(t) {
    const own = getOwn();
    const c = cpaTcpa(own, t);
    const rows = [];
    const add = (k, v) => { if (v != null && v !== "") rows.push(`<tr><th>${esc(k)}</th><td>${esc(v)}</td></tr>`); };
    const kindName = { vessel: t.ship_type != null ? category(t.ship_type)[0] : "Vessel", aton: `Aid to navigation${t.virtual_aton ? " (virtual: nothing physical there)" : ""}`, aircraft: "SAR aircraft",
      base: "AIS base station", sart: "AIS-SART (survival craft transmitter)", mob: "MOB device (person in the water)", epirb: "EPIRB" }[t.kind];
    add("Type", kindName + (t.class ? ` · class ${t.class}` : ""));
    add("MMSI", String(t.mmsi).padStart(9, "0"));
    add("Call sign", t.callsign);
    if (t.kind === "aton") add("Aid", ATON[t.aton_type] ?? (t.aton_type != null ? `type ${t.aton_type}` : null));
    if (t.off_position) add("Warning", "OFF POSITION");
    add("Status", t.status != null ? STATUS[t.status] : null);
    if (t.sog != null || t.cog != null) add("SOG / COG", `${t.sog != null ? t.sog.toFixed(1) + " kn" : "—"} / ${t.cog != null ? Math.round(t.cog) + "°T" : "—"}`);
    if (t.heading != null) add("Heading", `${Math.round(t.heading)}°T`);
    if (t.length) add("Size", `${t.length} × ${t.beam ?? "?"} m${t.draught ? `, draught ${t.draught} m` : ""}`);
    add("Destination", t.destination);
    if (t.altitude != null) add("Altitude", `${t.altitude} m`);
    add("Position", `${ddm(t.lat, "N", "S")} ${ddm(t.lon, "E", "W")}`);
    if (c) {
      add("Range / bearing", `${c.range.toFixed(2)} NM / ${String(Math.round(bearing(own.pos, [t.lon, t.lat]))).padStart(3, "0")}°T`);
      if (c.tcpa != null) add("CPA / TCPA", c.tcpa > 0 ? `${c.cpa.toFixed(2)} NM in ${Math.round(c.tcpa)} min` : "Moving apart");
    }
    if (t.last_pos) add("Position age", `${Math.round(Date.now() / 1000 - t.last_pos)} s`);
    add("Heard by", t.source.replace("signalk", "Signal K").replace("aisstream", "aisstream.io").replace("+", " and "));
    return `<div class="ais-pop"><b>${esc(t.name ?? `MMSI ${t.mmsi}`)}</b><table>${rows.join("")}</table></div>`;
  }
  function openPopup(mmsi) {
    const t = targets.get(mmsi);
    if (!t || t.lat == null) return;
    selected = mmsi;
    popup?.remove();
    popup = new maplibregl.Popup({ closeButton: true, maxWidth: "320px" }).setLngLat([t.lon, t.lat]).setHTML(describe(t)).addTo(map);
    popup.on("close", () => { if (selected === mmsi) { selected = null; draw(); } });
    draw();
  }
  map.on("click", "ais-sym", (e) => { const f = e.features?.[0]; if (f) openPopup(f.properties.mmsi); });
  map.on("mouseenter", "ais-sym", () => { map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "ais-sym", () => { map.getCanvas().style.cursor = ""; });

  // ---------- list ----------------------------------------------------------------
  function renderList() {
    const ul = $("ais-list");
    if ($("ais-list").closest(".pane")?.hidden) return;
    const own = getOwn();
    const rows = [...targets.values()].filter((t) => t.lat != null).map((t) => ({ t, c: cpaTcpa(own, t) }));
    const key = { range: (r) => r.c?.range ?? 1e9, cpa: (r) => (r.c?.tcpa > 0 ? r.c.cpa : 1e9), name: (r) => (r.t.name ?? "~").toLowerCase() }[s.sort];
    rows.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0));
    $("ais-count").textContent = rows.length ? `(${rows.length})` : "";
    ul.innerHTML = rows.length ? "" : `<li class="empty">No targets. Signal K passes on what a receiver on board hears; online AIS needs a key.</li>`;
    for (const { t, c } of rows.slice(0, 150)) {
      const li = document.createElement("li");
      const danger = c && c.tcpa > 0 && c.cpa < s.cpa && c.tcpa < s.tcpa;
      li.className = danger ? "danger" : "";
      li.innerHTML = `<div class="main"><b></b><small></small></div>`;
      li.querySelector("b").textContent = t.name ?? `MMSI ${t.mmsi}`;
      const [cat] = t.kind === "vessel" ? category(t.ship_type) : [{ aton: "AtoN", aircraft: "SAR aircraft", base: "Base station", sart: "AIS-SART", mob: "MOB", epirb: "EPIRB" }[t.kind]];
      li.querySelector("small").textContent = [cat, t.sog != null ? `${t.sog.toFixed(1)} kn` : null, c ? `${c.range.toFixed(1)} NM` : null,
        c && c.tcpa > 0 ? `CPA ${c.cpa.toFixed(2)} NM in ${Math.round(c.tcpa)} min` : null, isLost(t) ? "lost" : null].filter(Boolean).join(" · ");
      li.querySelector(".main").onclick = () => { map.easeTo({ center: [t.lon, t.lat], zoom: Math.max(map.getZoom(), 12) }); openPopup(t.mmsi); };
      ul.append(li);
    }
  }

  // ---------- alarms ----------------------------------------------------------------
  const raised = new Set();
  function checkAlarms() {
    const own = getOwn();
    const now = new Set();
    for (const t of targets.values()) {
      if (t.lat == null) continue;
      const id = `ais-${t.mmsi}`;
      if (["sart", "mob", "epirb"].includes(t.kind)) {
        const c = own?.pos ? cpaTcpa(own, t) : null;
        const what = { sart: "AIS-SART", mob: "MOB DEVICE", epirb: "AIS EPIRB" }[t.kind];
        alarms.raise(id, { level: "alarm", text: `${what} ${t.name ?? t.mmsi}${c ? ` · ${String(Math.round(bearing(own.pos, [t.lon, t.lat]))).padStart(3, "0")}°T ${c.range.toFixed(1)} NM` : ""}`,
          detail: `${ddm(t.lat, "N", "S")} ${ddm(t.lon, "E", "W")}${t.status === 14 && t.kind === "sart" && /test/i.test(t.name ?? "") ? " · test transmission" : ""} · click to show`,
          onClick: () => { map.easeTo({ center: [t.lon, t.lat], zoom: Math.max(map.getZoom(), 12) }); openPopup(t.mmsi); } });
        now.add(id);
        continue;
      }
      if (!s.cpaOn || !own?.live || t.kind !== "vessel" || isLost(t)) continue;
      if (s.ignoreMoored && (t.status === 1 || t.status === 5)) continue;
      const c = cpaTcpa(own, t);
      if (c && c.tcpa != null && c.tcpa > 0 && c.tcpa < s.tcpa && c.cpa < s.cpa) {
        alarms.raise(id, { level: "alarm", text: `COLLISION RISK ${t.name ?? t.mmsi}: CPA ${c.cpa.toFixed(2)} NM in ${Math.round(c.tcpa)} min`,
          detail: `${c.range.toFixed(2)} NM at ${String(Math.round(bearing(own.pos, [t.lon, t.lat]))).padStart(3, "0")}°T · ${t.sog?.toFixed(1) ?? "?"} kn ${t.cog != null ? Math.round(t.cog) + "°T" : ""} · click to show`,
          onClick: () => { map.easeTo({ center: [t.lon, t.lat] }); openPopup(t.mmsi); } });
        now.add(id);
      }
    }
    for (const id of raised) if (!now.has(id)) alarms.clear(id);
    raised.clear();
    for (const id of now) raised.add(id);
  }

  // ---------- data ----------------------------------------------------------------
  listen("ais", ({ payload }) => {
    if (payload.reset) targets.clear();
    for (const t of payload.targets || []) targets.set(t.mmsi, t);
    for (const m of payload.gone || []) targets.delete(m);
    draw();
    renderList();
    checkAlarms();
    statusUi();
  });
  listen("ais-status", ({ payload }) => { online = payload; statusUi(); });
  invoke("ais_targets").then((list) => { for (const t of list || []) targets.set(t.mmsi, t); draw(); }).catch(() => {});
  setInterval(() => { draw(); renderList(); checkAlarms(); if (popup && selected != null && targets.get(selected)) popup.setLngLat([targets.get(selected).lon, targets.get(selected).lat]).setHTML(describe(targets.get(selected))); }, 5000);

  // the online feed follows the chart: the view (wider when zoomed out), at most every 5 s
  function boxes() {
    const b = map.getBounds();
    const c = map.getCenter();
    let w = Math.min(6, Math.max(0.5, (b.getEast() - b.getWest()) * 1.2)), h = Math.min(6, Math.max(0.5, (b.getNorth() - b.getSouth()) * 1.2));
    const lat0 = Math.max(-89, c.lat - h / 2), lat1 = Math.min(89, c.lat + h / 2);
    let lon0 = c.lng - w / 2, lon1 = c.lng + w / 2;
    const wrap = (x) => ((x + 540) % 360) - 180;
    lon0 = wrap(lon0); lon1 = wrap(lon1);
    if (lon0 <= lon1) return [[[lat0, lon0], [lat1, lon1]]];
    return [[[lat0, lon0], [lat1, 180]], [[lat0, -180], [lat1, lon1]]]; // across the date line
  }
  let subTimer = null, lastSub = "";
  function subscribe(force) {
    const key = s.online ? s.key.trim() : "";
    const bx = key ? boxes() : [];
    const sig = key + JSON.stringify(bx.map((b) => b.map((p) => p.map((x) => Math.round(x * 4) / 4))));
    if (!force && sig === lastSub) return;
    lastSub = sig;
    invoke("ais_online", { key, boxes: bx }).catch(() => {});
  }
  map.on("moveend", () => { clearTimeout(subTimer); subTimer = setTimeout(() => subscribe(false), 5000); });

  // ---------- settings ----------------------------------------------------------------
  function statusUi() {
    const sk = [...targets.values()].filter((t) => t.source.includes("signalk")).length;
    const on = [...targets.values()].filter((t) => t.source.includes("aisstream")).length;
    $("ais-sk-dot").className = `dot ${sk ? "on" : "paused"}`;
    $("ais-sk-state").textContent = sk ? `Signal K: ${sk} target${sk === 1 ? "" : "s"}` : "Signal K: no AIS targets (no receiver, or nothing in range)";
    $("ais-on-dot").className = `dot ${!s.online ? "paused" : online.connected ? "on" : ""}`;
    $("ais-on-state").textContent = !s.online ? "Online (aisstream.io): off" : online.connected ? `Online (aisstream.io): connected, ${on} target${on === 1 ? "" : "s"}` : `Online (aisstream.io): ${online.detail || "connecting…"}`;
    $("ais-online").textContent = s.online ? "Go offline" : "Go online";
    $("ais-online").classList.toggle("on", s.online);
  }
  $("ais-show").checked = s.show;
  $("ais-names").checked = s.names;
  $("ais-key").value = s.key;
  $("ais-cpa-on").checked = s.cpaOn;
  $("ais-cpa").value = s.cpa;
  $("ais-tcpa").value = s.tcpa;
  $("ais-ignore-moored").checked = s.ignoreMoored;
  $("ais-sort").value = s.sort;
  $("ais-show").onchange = (e) => { s.show = e.target.checked; save(); draw(); };
  $("ais-names").onchange = (e) => { s.names = e.target.checked; save(); draw(); };
  $("ais-key").onchange = (e) => { s.key = e.target.value.trim(); save(); if (s.online) subscribe(true); };
  $("ais-online").onclick = () => {
    if (!s.online && !s.key.trim()) { $("ais-on-state").textContent = "Online (aisstream.io): paste your API key first"; $("ais-key").focus(); return; }
    s.online = !s.online; save(); subscribe(true); statusUi();
  };
  $("ais-key-link").onclick = (e) => { e.preventDefault(); invoke("open_url", { url: "https://aisstream.io/authenticate" }).catch(() => {}); };
  $("ais-cpa-on").onchange = (e) => { s.cpaOn = e.target.checked; save(); checkAlarms(); };
  $("ais-cpa").onchange = (e) => { const v = parseFloat(e.target.value); if (v > 0) s.cpa = v; save(); };
  $("ais-tcpa").onchange = (e) => { const v = parseFloat(e.target.value); if (v > 0) s.tcpa = v; save(); };
  $("ais-ignore-moored").onchange = (e) => { s.ignoreMoored = e.target.checked; save(); };
  $("ais-sort").onchange = (e) => { s.sort = e.target.value; save(); renderList(); };
  ui?.onChange?.(() => renderList());
  statusUi();
  if (s.online) setTimeout(() => subscribe(true), 1500);
  return { targets, openPopup };
}
