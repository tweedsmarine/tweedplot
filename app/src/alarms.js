// Alarms: a banner at the top of the chart with a sound until acknowledged.
// Built in: shallow water, anchor watch, air draft under charted bridges and
// overhead cables, position lost, and the own man-overboard (MOB) button.
// Other modules (AIS) raise their own through the returned `raise`/`clear`.
import { invoke } from "@tauri-apps/api/core";
import { distNm, bearing } from "./instruments.js";
import { ddm } from "./navmath.js";

const KEY = "racenav.alarms";
const DEFAULTS = {
  shallow: true, shallowM: null, // null = the safety depth
  anchorM: 50, anchor: null, // {lon, lat}
  air: true, airMargin: 2,
  posLost: true,
  volume: 0.6,
};
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** Tones with Web Audio: no sound files needed. */
function makeSound(getVolume) {
  let ctx = null;
  const beep = (f, at, dur) => {
    ctx ||= new (window.AudioContext || window.webkitAudioContext)();
    const o = ctx.createOscillator(), g = ctx.createGain();
    o.type = "square";
    o.frequency.value = f;
    const v = getVolume() * 0.25;
    g.gain.setValueAtTime(0, ctx.currentTime + at);
    g.gain.linearRampToValueAtTime(v, ctx.currentTime + at + 0.01);
    g.gain.setValueAtTime(v, ctx.currentTime + at + dur - 0.02);
    g.gain.linearRampToValueAtTime(0, ctx.currentTime + at + dur);
    o.connect(g).connect(ctx.destination);
    o.start(ctx.currentTime + at);
    o.stop(ctx.currentTime + at + dur + 0.02);
  };
  return {
    alarm() { try { beep(880, 0, 0.18); beep(660, 0.22, 0.18); beep(880, 0.44, 0.18); } catch { /* no audio */ } },
    warn() { try { beep(740, 0, 0.12); beep(988, 0.16, 0.2); } catch { /* no audio */ } },
  };
}

export function setupAlarms(map, { getBoat, getSafetyDepth, getAirDraft }) {
  let s = { ...DEFAULTS };
  try { Object.assign(s, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  let mob = null;
  try { mob = JSON.parse(localStorage.getItem(KEY + ".mob") || "null"); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(s)); localStorage.setItem(KEY + ".mob", JSON.stringify(mob)); } catch { /* ignore */ } };
  const sound = makeSound(() => s.volume);

  /** id → {level: "alarm"|"warn", text, detail, acked, since, onClick} */
  const active = new Map();
  function raise(id, a) {
    const old = active.get(id);
    if (old) {
      Object.assign(old, { ...a, acked: old.acked && old.level === a.level });
    } else {
      active.set(id, { ...a, acked: false, since: Date.now() });
      if (a.level === "alarm") sound.alarm(); else sound.warn();
    }
    render();
  }
  function clear(id) {
    if (active.delete(id)) render();
  }
  function ack(id) {
    const a = active.get(id);
    if (a) { a.acked = true; render(); }
  }

  // ---------- banner ----------------------------------------------------------------
  const banner = $("alarm-banner");
  function render() {
    banner.innerHTML = "";
    const list = [...active.entries()].sort((a, b) => (a[1].level === b[1].level ? a[1].since - b[1].since : a[1].level === "alarm" ? -1 : 1));
    banner.hidden = !list.length;
    for (const [id, a] of list) {
      const row = document.createElement("div");
      row.className = `alarm ${a.level}${a.acked ? " acked" : ""}`;
      row.innerHTML = `<div class="txt"><b>${esc(a.text)}</b>${a.detail ? `<small>${esc(a.detail)}</small>` : ""}</div>` +
        (a.acked ? "" : `<button data-act="ack" title="Silence this alarm (it stays here while the cause lasts)">Silence</button>`) +
        (a.dismiss ? `<button data-act="dismiss" title="${esc(a.dismissTitle || "Dismiss")}">${esc(a.dismissLabel || "✕")}</button>` : "");
      row.querySelector(".txt").onclick = () => a.onClick?.();
      row.querySelector('[data-act="ack"]')?.addEventListener("click", () => ack(id));
      row.querySelector('[data-act="dismiss"]')?.addEventListener("click", () => a.dismiss());
      banner.append(row);
    }
  }
  // sound again every few seconds until silenced
  setInterval(() => {
    const loud = [...active.values()].filter((a) => !a.acked);
    if (loud.some((a) => a.level === "alarm")) sound.alarm();
    else if (loud.length && Date.now() % 12000 < 3000) sound.warn();
  }, 3000);

  // ---------- chart: anchor circle and MOB --------------------------------------------
  map.addSource("alarm-geo", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
  map.addLayer({ id: "alarm-anchor", type: "line", source: "alarm-geo", filter: ["==", ["get", "k"], "anchor"],
    paint: { "line-color": "#c0392b", "line-width": 1.5, "line-dasharray": [3, 2] } });
  map.addLayer({ id: "alarm-mob-line", type: "line", source: "alarm-geo", filter: ["==", ["get", "k"], "mobline"],
    paint: { "line-color": "#d00000", "line-width": 2, "line-dasharray": [2, 1.5] } });
  map.addLayer({ id: "alarm-mob", type: "circle", source: "alarm-geo", filter: ["==", ["get", "k"], "mob"],
    paint: { "circle-radius": 9, "circle-color": "#ffcc00", "circle-stroke-color": "#d00000", "circle-stroke-width": 4 } });
  function circle(c, rM) {
    const pts = [];
    for (let i = 0; i <= 64; i++) {
      const a = (i / 64) * 2 * Math.PI;
      pts.push([c.lon + (rM * Math.sin(a)) / (111320 * Math.cos((c.lat * Math.PI) / 180)), c.lat + (rM * Math.cos(a)) / 111320]);
    }
    return pts;
  }
  function drawGeo() {
    const f = [];
    if (s.anchor) f.push({ type: "Feature", properties: { k: "anchor" }, geometry: { type: "LineString", coordinates: circle(s.anchor, s.anchorM) } });
    if (mob) {
      f.push({ type: "Feature", properties: { k: "mob" }, geometry: { type: "Point", coordinates: [mob.lon, mob.lat] } });
      const b = getBoat();
      if (b.pos) f.push({ type: "Feature", properties: { k: "mobline" }, geometry: { type: "LineString", coordinates: [b.pos, [mob.lon, mob.lat]] } });
    }
    map.getSource("alarm-geo").setData({ type: "FeatureCollection", features: f });
  }

  // ---------- MOB ----------------------------------------------------------------
  const fmtAgo = (ms) => { const m = Math.floor(ms / 60000), sec = Math.floor((ms % 60000) / 1000); return `${m}:${String(sec).padStart(2, "0")}`; };
  function mobAlarm() {
    if (!mob) { clear("mob"); return; }
    const b = getBoat();
    const here = b.pos;
    const d = here ? distNm(here, [mob.lon, mob.lat]) : null;
    const brg = here ? bearing(here, [mob.lon, mob.lat]) : null;
    const dist = d == null ? "" : d < 0.2 ? `${Math.round(d * 1852)} m` : `${d.toFixed(2)} NM`;
    raise("mob", {
      level: "alarm",
      text: `MAN OVERBOARD ${fmtAgo(Date.now() - mob.t)} ago${brg != null ? ` · ${String(Math.round(brg)).padStart(3, "0")}°T ${dist}` : ""}`,
      detail: `${ddm(mob.lat, "N", "S")} ${ddm(mob.lon, "E", "W")} · click to show on the chart`,
      onClick: () => map.easeTo({ center: [mob.lon, mob.lat], zoom: Math.max(map.getZoom(), 14) }),
      dismiss: () => {
        const btn = banner.querySelector(".alarm.alarm [data-act='dismiss']");
        if (btn && btn.dataset.sure) { mob = null; save(); clear("mob"); drawGeo(); $("mob-btn").classList.remove("on"); }
        else if (btn) { btn.dataset.sure = "1"; btn.textContent = "Sure?"; setTimeout(() => { if (btn.isConnected) { delete btn.dataset.sure; btn.textContent = "Clear MOB"; } }, 3000); }
      },
      dismissLabel: "Clear MOB",
      dismissTitle: "End the man-overboard (asks again to be sure)",
    });
  }
  $("mob-btn").onclick = async () => {
    const b = getBoat();
    // the boat's position (live or where it was put); the chart centre if there is no boat
    const p = b.pos ?? [map.getCenter().lng, map.getCenter().lat];
    mob = { lon: p[0], lat: p[1], t: Date.now() };
    save();
    $("mob-btn").classList.add("on");
    mobAlarm();
    drawGeo();
    // keep a record in the library
    const when = new Date(mob.t);
    invoke("mark_save", { mark: { name: `MOB ${when.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`, lat: mob.lat, lon: mob.lon, kind: "danger", group: "MOB",
      notes: `Man overboard pressed ${when.toISOString()}` } }).catch(() => {});
  };
  if (mob) { $("mob-btn").classList.add("on"); }

  // ---------- checks ----------------------------------------------------------------
  let lastPosAt = Date.now();
  let lastAirCheck = 0;
  function check() {
    const b = getBoat();
    const v = b.v || {};
    // shallow water (live depth only)
    const depth = v.depth_keel ?? v.depth;
    const limit = s.shallowM ?? getSafetyDepth();
    if (s.shallow && b.live && depth != null && limit > 0) {
      if (depth < limit) raise("shallow", { level: "alarm", text: `SHALLOW ${depth.toFixed(1)} m`, detail: `Below the alarm depth of ${limit} m${v.depth_keel != null ? " (under the keel)" : " (below the transducer)"}` });
      else if (depth > limit + 0.5) clear("shallow");
    } else clear("shallow");
    // anchor watch
    if (s.anchor && b.pos) {
      const m = distNm(b.pos, [s.anchor.lon, s.anchor.lat]) * 1852;
      $("anc-state").textContent = `Watching: ${Math.round(m)} m from the anchor, alarm beyond ${s.anchorM} m.`;
      if (m > s.anchorM) raise("anchor", { level: "alarm", text: `ANCHOR DRAGGING ${Math.round(m)} m`, detail: `Outside the ${s.anchorM} m watch circle`, onClick: () => map.easeTo({ center: [s.anchor.lon, s.anchor.lat] }) });
      else if (m < s.anchorM * 0.9) clear("anchor");
    } else clear("anchor");
    // position lost (live, Signal K connected, no fix for 30 s)
    if (b.live && v.lat != null && b.fresh) lastPosAt = Date.now();
    if (s.posLost && b.live && b.connected && Date.now() - lastPosAt > 30000) {
      raise("poslost", { level: "warn", text: "POSITION LOST", detail: "No position from Signal K for 30 s: check the GPS" });
    } else clear("poslost");
    // air draft: charted clearances ahead, every 10 s
    const air = getAirDraft();
    if (s.air && air > 0 && b.live && b.pos && Date.now() - lastAirCheck > 10000) {
      lastAirCheck = Date.now();
      const sog = v.sog ?? 0, cog = v.cog ?? v.heading;
      const range = Math.max(1852, (sog * 1852 * 10) / 60); // 1 NM, or 10 min ahead
      invoke("chart_clearances", { lat: b.pos[1], lon: b.pos[0], radiusM: range }).then((list) => {
        const low = (list || []).filter((c) => c.clearance_m < air + s.airMargin && (cog == null || Math.abs(((c.bearing - cog + 540) % 360) - 180) < 60 || c.distance_m < 150));
        if (low.length) {
          const c = low[0];
          raise("air", { level: "alarm", text: `AIR DRAFT: ${c.class} ${c.clearance_m.toFixed(1)} m clearance ${c.distance_m < 1852 ? Math.round(c.distance_m) + " m" : (c.distance_m / 1852).toFixed(1) + " NM"} ahead`,
            detail: `${c.name ? c.name + " · " : ""}your air draft ${air} m + ${s.airMargin} m margin; charted clearance is above HAT` });
        } else clear("air");
      }).catch(() => {});
    }
    if (!(s.air && air > 0 && b.live)) clear("air");
    if (mob) mobAlarm();
    drawGeo();
  }
  setInterval(check, 1000);

  // ---------- settings (Instruments › Alarms) -------------------------------------------
  function syncUi() {
    $("al-shallow").checked = s.shallow;
    $("al-shallow-m").value = s.shallowM ?? "";
    $("al-shallow-m").placeholder = String(getSafetyDepth());
    $("al-air").checked = s.air;
    $("al-air-margin").value = s.airMargin;
    $("al-poslost").checked = s.posLost;
    $("al-volume").value = Math.round(s.volume * 100);
    $("anc-radius").value = s.anchorM;
    $("anc-drop").textContent = s.anchor ? "Weigh anchor" : "Drop anchor here";
    if (!s.anchor) $("anc-state").textContent = "Off. Drop anchor at the boat's position, then set the swing radius.";
    const air = getAirDraft();
    $("al-air-note").textContent = air > 0 ? `Air draft ${air} m (Boat panel).` : "Set the air draft in the Boat panel first.";
  }
  $("al-shallow").onchange = (e) => { s.shallow = e.target.checked; save(); };
  $("al-shallow-m").onchange = (e) => { const v = parseFloat(e.target.value); s.shallowM = Number.isFinite(v) && v > 0 ? v : null; save(); syncUi(); };
  $("al-air").onchange = (e) => { s.air = e.target.checked; save(); };
  $("al-air-margin").onchange = (e) => { const v = parseFloat(e.target.value); if (Number.isFinite(v) && v >= 0) s.airMargin = v; save(); };
  $("al-poslost").onchange = (e) => { s.posLost = e.target.checked; save(); };
  $("al-volume").oninput = (e) => { s.volume = +e.target.value / 100; save(); };
  $("al-test").onclick = () => sound.alarm();
  $("anc-radius").onchange = (e) => { const v = parseFloat(e.target.value); if (Number.isFinite(v) && v > 5) s.anchorM = v; save(); drawGeo(); };
  $("anc-drop").onclick = () => {
    if (s.anchor) { s.anchor = null; clear("anchor"); }
    else {
      const b = getBoat();
      if (!b.pos) { $("anc-state").textContent = "No boat position yet."; return; }
      s.anchor = { lon: b.pos[0], lat: b.pos[1] };
    }
    save(); syncUi(); drawGeo();
  };
  syncUi();
  window.addEventListener("racenav-boat", syncUi);

  const list = () => [...active.values()].map((a) => ({ level: a.level, text: a.text, detail: a.detail, acked: a.acked }));
  return { raise, clear, ack, sound, list, active: () => [...active.keys()], sync: syncUi };
}
