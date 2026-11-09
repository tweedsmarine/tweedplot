// Alarms: a banner at the top of the chart with a sound until acknowledged.
// Built in: shallow water, the safety contour ahead, air draft under charted bridges
// and overhead cables, off track (cross-track error), approaching the next mark,
// position lost, anchor watch, and the own man-overboard (MOB) button. All are set
// in the Alarms drawer, which also lists the active ones (one place for every alert,
// as IMO's bridge alert management asks).
// Other modules (AIS) raise their own through the returned `raise`/`clear`.
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { PRIMARY } from "./role.js";
import { distNm, bearing } from "./instruments.js";
import { ddm } from "./navmath.js";

const KEY = "racenav.alarms";
const DEFAULTS = {
  shallow: true, shallowM: null, // null = the safety depth
  anchorM: 50, anchor: null, // the watched anchor: {lon, lat, id (its library mark), radius (m), name}; anchorM is the default radius
  air: true, airMargin: 2,
  posLost: true,
  contour: true, contourMin: 6,
  xte: false, xteNm: 0.25,
  arrive: true, arriveMin: 2,
  watchdogMin: 5,
  // sounds (Alarms › Sound): alarms, warnings, the anchor alarm
  soundAlarm: "triple", soundWarn: "chime", soundAnchor: "siren",
  volume: 0.6,
};
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** Tones with Web Audio: no sound files needed. */
/** The sounds to choose from (Alarms › Sound): name → [label, notes]. Made here, so no sound files. */
export const SOUNDS = {
  triple: "Triple tone (high, low, high)",
  chime: "Chime (two rising notes)",
  siren: "Siren (rising and falling)",
  pulse: "Fast pulses",
  whoop: "Whoop (rising sweep)",
  bell: "Bell (ringing)",
  low: "Low two-tone (less shrill)",
  horn: "Horn (long low blasts)",
};

function makeSound(getVolume) {
  let ctx = null;
  /** One tone from f1 to f2 (Hz) at `at` s for `dur` s; `decay`: fades out like a bell. */
  const tone = (f1, f2, at, dur, type = "square", decay = false, level = 1) => {
    ctx ||= new (window.AudioContext || window.webkitAudioContext)();
    const o = ctx.createOscillator(), g = ctx.createGain();
    o.type = type;
    const t0 = ctx.currentTime + at;
    o.frequency.setValueAtTime(f1, t0);
    if (f2 !== f1) o.frequency.linearRampToValueAtTime(f2, t0 + dur);
    const v = getVolume() * 0.25 * level;
    g.gain.setValueAtTime(0, t0);
    g.gain.linearRampToValueAtTime(v, t0 + 0.01);
    if (decay) g.gain.exponentialRampToValueAtTime(0.0005, t0 + dur);
    else { g.gain.setValueAtTime(v, t0 + dur - 0.02); g.gain.linearRampToValueAtTime(0, t0 + dur); }
    o.connect(g).connect(ctx.destination);
    o.start(t0);
    o.stop(t0 + dur + 0.02);
  };
  const PLAY = {
    triple: () => { tone(880, 880, 0, 0.18); tone(660, 660, 0.22, 0.18); tone(880, 880, 0.44, 0.18); },
    chime: () => { tone(740, 740, 0, 0.12); tone(988, 988, 0.16, 0.2); },
    siren: () => { tone(600, 1200, 0, 0.45, "sawtooth", false, 0.7); tone(1200, 600, 0.45, 0.45, "sawtooth", false, 0.7); },
    pulse: () => { for (let i = 0; i < 6; i++) tone(1000, 1000, i * 0.12, 0.07); },
    whoop: () => { tone(400, 1100, 0, 0.6, "square", false, 0.8); },
    bell: () => { tone(1320, 1320, 0, 1.2, "sine", true, 1.4); tone(1980, 1980, 0, 0.8, "sine", true, 0.5); tone(1320, 1320, 0.6, 1.2, "sine", true, 1.4); },
    low: () => { tone(440, 440, 0, 0.25, "triangle", false, 1.4); tone(330, 330, 0.3, 0.25, "triangle", false, 1.4); },
    horn: () => { tone(220, 220, 0, 0.5, "sawtooth", false, 0.8); tone(220, 220, 0.65, 0.5, "sawtooth", false, 0.8); },
  };
  const play = (name) => { try { (PLAY[name] ?? PLAY.triple)(); } catch { /* no audio */ } };
  return { play };
}

export function setupAlarms(map, { getBoat, getBow = () => null, getSafetyDepth, getSafetyContour = () => 0, getAirDraft, getNav = () => null, onMobMarkDeleted = () => {}, toast = () => {} }) {
  let s = { ...DEFAULTS };
  try { Object.assign(s, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  let mob = null;
  try { mob = JSON.parse(localStorage.getItem(KEY + ".mob") || "null"); } catch { /* ignore */ }
  // The primary window does the alarm work and keeps the MOB and anchor; the other windows
  // show the alarms it publishes and send it their button presses (silence, MOB, anchor).
  const command = (act, id, extra) => emit("alarm-cmd", { act, id, extra }).catch(() => {});
  const save = () => {
    try {
      if (PRIMARY) {
        localStorage.setItem(KEY, JSON.stringify(s));
        localStorage.setItem(KEY + ".mob", JSON.stringify(mob));
      } else {
        const cur = JSON.parse(localStorage.getItem(KEY) || "{}");
        localStorage.setItem(KEY, JSON.stringify({ ...s, anchor: cur.anchor ?? null }));
      }
    } catch { /* ignore */ }
  };
  const snd = makeSound(() => s.volume);
  /** The sound for each kind of alarm (Alarms › Sound): red alarms, amber warnings, the anchor. */
  const sound = { play: snd.play, alarm: () => snd.play(s.soundAlarm), warn: () => snd.play(s.soundWarn) };
  /** An alarm's own sound: the anchor has its own; else by level. */
  const ring = (a) => (a.sound ? snd.play(s[a.sound] ?? a.sound) : a.level === "alarm" ? sound.alarm() : sound.warn());

  /** id → {level: "alarm"|"warn", text, detail, acked, since, onClick} */
  const active = new Map();
  /** what a monitor window shows: the primary's alarms, as plain data */
  let mirror = [];
  /** How long Dismiss keeps a lasting alarm quiet; it stays on the Active list meanwhile. */
  const HOLD_MS = 60000;
  /**
   * Raise (or refresh) an alarm. `stage` (default 0) is how bad the cause is: a higher stage than
   * before sounds again even if silenced or dismissed. `holdable` adds a Dismiss button.
   * `clearable` is for an event with no lasting cause (a source changed): its Clear button
   * removes it.
   */
  function raise(id, a) {
    if (!PRIMARY) return;
    const entry = a.holdable
      ? { ...a, dismiss: () => dismiss(id), dismissLabel: "Dismiss",
        dismissTitle: "Quiet for a minute: it stays on the Alarms list, and sounds again if the cause lasts or worsens" }
      : a.clearable
        ? { ...a, dismiss: () => clear(id), dismissLabel: "Clear", dismissTitle: "Remove this alarm: it has no lasting cause to clear it" }
        : a;
    const old = active.get(id);
    if (old) {
      const worse = (a.stage ?? 0) > (old.stage ?? 0);
      // a dismissed alarm whose minute is up and whose cause lasts: back in the banner, sounding
      const holdOver = old.heldUntil != null && Date.now() >= old.heldUntil;
      Object.assign(old, { ...entry, acked: old.acked && old.level === a.level && !worse && !holdOver });
      if (worse || holdOver) {
        old.heldUntil = null;
        ring(a);
      }
    } else {
      active.set(id, { ...entry, acked: false, since: Date.now() });
      ring(a);
    }
    render();
  }
  /** The cause has ended (or was changed by the user): the alarm goes, and may raise afresh. */
  function clear(id) {
    if (active.delete(id)) render();
  }
  /** Quiet a lasting alarm for HOLD_MS. Unlike a removal it stays on the Active list (and in the
   *  banner's silenced count), so a live collision risk never vanishes from the screen. */
  function dismiss(id) {
    const a = active.get(id);
    if (!a) return;
    a.acked = true;
    a.ackedAt = Date.now();
    a.heldUntil = Date.now() + HOLD_MS;
    render();
  }
  function ack(id) {
    if (!PRIMARY) { command("ack", id); return; }
    const a = active.get(id);
    if (a) { a.acked = true; a.ackedAt = Date.now(); render(); }
  }
  // Watchdog: a silenced high-priority alarm whose cause still lasts sounds again after the
  // watchdog time (Alarms › Sound), so it can't be forgotten.
  if (PRIMARY) setInterval(() => {
    const ms = Math.min(20, Math.max(1, Number(s.watchdogMin) || 5)) * 60000;
    let first = null;
    for (const a of active.values()) {
      if (a.watchdog && a.acked && Date.now() - (a.ackedAt ?? 0) >= ms) {
        a.acked = false;
        if (!first || (a.level === "alarm" && first.level !== "alarm")) first = a;
      }
    }
    if (first) { ring(first); render(); }
  }, 5000);
  /** 0, 1 or 2: how far a falling measure (minutes, depth) is below its limit: under half, under a quarter. */
  const stageBelow = (v, limit) => (v < limit / 4 ? 2 : v < limit / 2 ? 1 : 0);
  /** The same for a growing measure (off-track distance): over twice, over four times the limit. */
  const stageAbove = (v, limit) => (v > limit * 4 ? 2 : v > limit * 2 ? 1 : 0);
  const entries = () => (PRIMARY
    ? [...active.entries()]
    : mirror.map((a) => [a.id, { ...a,
      dismiss: a.canDismiss ? () => (a.id === "mob" ? confirmMob(() => command("clearmob")) : command("dismiss", a.id)) : null,
      onClick: a.focus ? () => map.easeTo({ center: a.focus, zoom: Math.max(map.getZoom(), 14) }) : null }]));
  function publish() {
    emit("alarm-state", [...active.entries()].map(([id, a]) => ({
      id, level: a.level, text: a.text, detail: a.detail, acked: a.acked, since: a.since, heldUntil: a.heldUntil ?? null,
      canDismiss: !!a.dismiss, dismissLabel: a.dismissLabel, dismissTitle: a.dismissTitle,
      focus: id === "mob" && mob ? [mob.lon, mob.lat] : id === "anchor" && s.anchor ? [s.anchor.lon, s.anchor.lat] : null,
    }))).catch(() => {});
  }

  // ---------- banner ----------------------------------------------------------------
  const banner = $("alarm-banner");
  function render() {
    banner.innerHTML = "";
    const byLevel = (a, b) => (a[1].level === b[1].level ? a[1].since - b[1].since : a[1].level === "alarm" ? -1 : 1);
    // Unacknowledged alarms (and the MOB) are stacked, never overlapping, so none is hidden.
    // Once silenced they move to the Alarms list; one steady row keeps them in view.
    const all = entries();
    const list = all.filter(([id, a]) => !a.acked || id === "mob").sort(byLevel);
    const silenced = all.filter(([id, a]) => a.acked && id !== "mob").sort(byLevel);
    banner.hidden = !list.length && !silenced.length;
    for (const [id, a] of list) {
      const row = document.createElement("div");
      row.className = `alarm ${a.level}${a.acked ? " acked" : ""}`;
      row.innerHTML = `<div class="txt"><b>${esc(a.text)}</b>${a.detail ? `<small>${esc(a.detail)}</small>` : ""}</div>` +
        (a.acked ? "" : `<button data-act="ack" title="Stop the sound; the alarm moves to the Alarms list and stays while its cause lasts">Silence</button>`) +
        (a.dismiss ? `<button data-act="dismiss" title="${esc(a.dismissTitle || "Dismiss")}">${esc(a.dismissLabel || "✕")}</button>` : "");
      row.querySelector(".txt").onclick = () => a.onClick?.();
      row.querySelector('[data-act="ack"]')?.addEventListener("click", () => ack(id));
      row.querySelector('[data-act="dismiss"]')?.addEventListener("click", () => a.dismiss());
      banner.append(row);
    }
    if (silenced.length) {
      const lvl = silenced.some(([, a]) => a.level === "alarm") ? "alarm" : "warn";
      const row = document.createElement("div");
      row.className = `alarm ${lvl} acked summary`;
      row.innerHTML = `<div class="txt"><b>${silenced.length} silenced ${silenced.length === 1 ? "alarm" : "alarms"} still active</b><small>Click to open the Alarms list</small></div>`;
      row.querySelector(".txt").onclick = () => document.querySelector('[data-pane="alarms"]')?.click();
      banner.append(row);
    }
    if (PRIMARY) publish();
    renderDrawer();
  }
  // the Alarms drawer's list of everything active, silenced ones included
  function renderDrawer() {
    const ul = $("al-active");
    if (!ul) return;
    const all = entries().sort((a, b) => (a[1].level === b[1].level ? a[1].since - b[1].since : a[1].level === "alarm" ? -1 : 1));
    $("al-count").textContent = all.length ? `${all.length}` : "";
    ul.innerHTML = all.length ? "" : '<li class="empty">No alarms or warnings.</li>';
    for (const [id, a] of all) {
      const li = document.createElement("li");
      li.className = `al-${a.level}${a.acked ? " acked" : ""}`;
      const t = new Date(a.since).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
      li.innerHTML = `<span class="al-lv">${a.level === "alarm" ? "Alarm" : "Warning"}</span><span class="grow"><b>${esc(a.text)}</b>${a.detail ? `<br><small>${esc(a.detail)}</small>` : ""}</span><span class="wx-note">${t}${a.heldUntil ? " · dismissed for a minute" : a.acked ? " · silenced" : ""}</span>` +
        (a.acked ? "" : '<button data-act="ack">Silence</button>') +
        (a.dismiss && id !== "mob" && !a.heldUntil ? `<button data-act="dismiss" title="${esc(a.dismissTitle || "Dismiss")}">${esc(a.dismissLabel || "Dismiss")}</button>` : "");
      li.querySelector('[data-act="ack"]')?.addEventListener("click", () => ack(id));
      li.querySelector('[data-act="dismiss"]')?.addEventListener("click", () => a.dismiss());
      li.querySelector(".grow").onclick = () => a.onClick?.();
      ul.append(li);
    }
  }
  // sound again every few seconds until silenced
  if (PRIMARY) setInterval(() => {
    const loud = [...active.values()].filter((a) => !a.acked);
    const top = loud.find((a) => a.level === "alarm");
    if (top) ring(top);
    else if (loud.length && Date.now() % 12000 < 3000) ring(loud[0]);
  }, 3000);
  if (!PRIMARY) listen("alarm-state", ({ payload }) => { mirror = payload || []; render(); });
  // ---------- chart: MOB (the anchor and its swing circle: anchor.js) --------------------------------------------
  map.addSource("alarm-geo", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
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
    if (mob) {
      f.push({ type: "Feature", properties: { k: "mob" }, geometry: { type: "Point", coordinates: [mob.lon, mob.lat] } });
      const b = getBoat();
      if (b.pos) f.push({ type: "Feature", properties: { k: "mobline" }, geometry: { type: "LineString", coordinates: [b.pos, [mob.lon, mob.lat]] } });
    }
    map.getSource("alarm-geo").setData({ type: "FeatureCollection", features: f });
  }

  // ---------- MOB ----------------------------------------------------------------
  const fmtAgo = (ms) => { const m = Math.floor(ms / 60000), sec = Math.floor((ms % 60000) / 1000); return `${m}:${String(sec).padStart(2, "0")}`; };
  /** Tell the rest of the app (the MOB instrument panel) that a MOB was set or cleared. */
  function mobChanged() { window.dispatchEvent(new CustomEvent("racenav-mob", { detail: mob })); }
  async function deleteMobMark(incident) {
    try {
      let id = incident.markId;
      if (!id) {
        const note = `Man overboard pressed ${new Date(incident.t).toISOString()}`;
        const marks = await invoke("marks_list");
        id = marks.find((mark) => mark.group === "MOB" && mark.notes === note)?.id;
      }
      if (id) {
        await invoke("mark_delete", { id, force: true });
        onMobMarkDeleted();
      }
    } catch (error) {
      toast(`Could not delete the MOB mark: ${error}`, 6000);
    }
  }
  /** Clearing a MOB asks twice: the first press turns the button into "Sure?". */
  function confirmMob(go) {
    const btn = banner.querySelector(".alarm.alarm [data-act='dismiss']");
    if (btn && btn.dataset.sure) go();
    else if (btn) { btn.dataset.sure = "1"; btn.textContent = "Sure?"; setTimeout(() => { if (btn.isConnected) { delete btn.dataset.sure; btn.textContent = "Clear MOB"; } }, 3000); }
  }
  function clearMob() {
    const incident = mob;
    if (!incident) return;
    mob = null;
    save();
    clear("mob");
    drawGeo();
    $("mob-btn").classList.remove("on");
    mobChanged();
    void deleteMobMark(incident);
  }
  function mobAlarm() {
    if (!mob) { clear("mob"); return; }
    const b = getBoat();
    const here = b.pos;
    const d = here ? distNm(here, [mob.lon, mob.lat]) : null;
    const brg = here ? bearing(here, [mob.lon, mob.lat]) : null;
    const dist = d == null ? "" : d < 0.2 ? `${Math.round(d * 1852)} m` : `${d.toFixed(2)} NM`;
    raise("mob", {
      level: "alarm", watchdog: true,
      text: `MAN OVERBOARD ${fmtAgo(Date.now() - mob.t)} ago${brg != null ? ` · ${String(Math.round(brg)).padStart(3, "0")}°T ${dist}` : ""}`,
      detail: `${ddm(mob.lat, "N", "S")} ${ddm(mob.lon, "E", "W")} · click to show on the chart`,
      onClick: () => map.easeTo({ center: [mob.lon, mob.lat], zoom: Math.max(map.getZoom(), 14) }),
      dismiss: () => confirmMob(clearMob),
      dismissLabel: "Clear MOB",
      dismissTitle: "End the man-overboard (asks again to be sure)",
    });
  }
  async function pressMob() {
    const b = getBoat();
    // the boat's position (live or where it was put); the chart centre if there is no boat
    const p = b.pos ?? [map.getCenter().lng, map.getCenter().lat];
    mob = { lon: p[0], lat: p[1], t: Date.now() };
    save();
    $("mob-btn").classList.add("on");
    mobAlarm();
    drawGeo();
    mobChanged();
    // keep a record in the library
    const when = new Date(mob.t);
    const incident = mob;
    try {
      const saved = await invoke("mark_save", { mark: { name: `MOB ${when.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`, lat: mob.lat, lon: mob.lon, kind: "danger", group: "MOB",
        notes: `Man overboard pressed ${when.toISOString()}` } });
      if (mob === incident) {
        incident.markId = saved.id;
        save();
      } else {
        await invoke("mark_delete", { id: saved.id, force: true });
        onMobMarkDeleted();
      }
    } catch (error) {
      toast(`Could not save or remove the MOB mark: ${error}`, 6000);
    }
  }
  $("mob-btn").onclick = () => (PRIMARY ? pressMob() : command("mob"));

  // Drop mark: a time-stamped mark at the boat's position, from any window. Unlike MOB it
  // raises no alarm and changes nothing on screen.
  $("drop-btn").onclick = async () => {
    const p = getBoat().pos ?? [map.getCenter().lng, map.getCenter().lat];
    const when = new Date();
    try {
      await invoke("mark_save", { mark: { name: `Drop ${when.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" })}`,
        lat: p[1], lon: p[0], kind: "waypoint", group: "Drops", notes: `Mark dropped ${when.toISOString()}` } });
      toast("Mark dropped", 2500);
      onMobMarkDeleted(); // refreshes the library and the marks on the chart
    } catch (error) {
      toast(`Could not drop the mark: ${error}`, 6000);
    }
  };
  if (mob) { $("mob-btn").classList.add("on"); }
  setTimeout(mobChanged, 0); // the instruments show a MOB still set from before

  // ---------- checks ----------------------------------------------------------------
  let lastPosAt = Date.now();
  let lastAirCheck = 0;
  let lastContourCheck = 0;
  let arrivedKey = null; // the mark the approach warning was given for
  function check() {
    const b = getBoat();
    const v = b.v || {};
    // shallow water (live depth only)
    const depth = v.depth_keel ?? v.depth;
    const limit = s.shallowM ?? getSafetyDepth();
    if (s.shallow && b.live && depth != null && limit > 0) {
      if (depth < limit) raise("shallow", { level: "alarm", watchdog: true, text: `SHALLOW ${depth.toFixed(1)} m`, detail: `Below the alarm depth of ${limit} m${v.depth_keel != null ? " (under the keel)" : " (below the transducer)"}` });
      else if (depth > limit + 0.5) clear("shallow");
    } else clear("shallow");
    // anchor watch
    // measured from the bow, as the anchor is placed and its swing circle drawn (anchor.js)
    const bowPos = b.pos ? (getBow() ?? b.pos) : null;
    if (s.anchor && bowPos) {
      const m = distNm(bowPos, [s.anchor.lon, s.anchor.lat]) * 1852;
      const r = anchorRadius();
      $("anc-state").textContent = `Watching ${s.anchor.name || "the anchor"}: the bow is ${Math.round(m)} m from it, alarm beyond ${r} m.`;
      if (m > r) raise("anchor", { level: "alarm", watchdog: true, sound: "soundAnchor", text: `ANCHOR DRAGGING: bow ${Math.round(m)} m from the anchor`, detail: `Outside the ${r} m swing circle of ${s.anchor.name || "the anchor"} · click to show`, onClick: () => { map.easeTo({ center: [s.anchor.lon, s.anchor.lat] }); window.dispatchEvent(new Event("racenav-anchor-open")); } });
      else if (m < r * 0.9) clear("anchor");
    } else clear("anchor");
    // position lost (live, Signal K connected, no fix for 30 s)
    if (b.live && v.lat != null && b.fresh) lastPosAt = Date.now();
    if (s.posLost && b.live && b.connected && Date.now() - lastPosAt > 30000) {
      raise("poslost", { level: "warn", watchdog: true, text: "POSITION LOST", detail: "No position data for 30 s: check the navigation source" });
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
          raise("air", { level: "alarm", watchdog: true, text: `AIR DRAFT: ${c.class} ${c.clearance_m.toFixed(1)} m clearance ${c.distance_m < 1852 ? Math.round(c.distance_m) + " m" : (c.distance_m / 1852).toFixed(1) + " NM"} ahead`,
            detail: `${c.name ? c.name + " · " : ""}your air draft ${air} m + ${s.airMargin} m margin; charted clearance is above HAT` });
        } else clear("air");
      }).catch(() => {});
    }
    if (!(s.air && air > 0 && b.live)) clear("air");
    // the safety contour ahead: shallower water (or land) on the course over ground, every 10 s
    const contour = getSafetyContour() || getSafetyDepth();
    if (s.contour && contour > 0 && b.live && b.pos && (v.sog ?? 0) > 0.5 && v.cog != null) {
      if (Date.now() - lastContourCheck > 10000) {
        lastContourCheck = Date.now();
        const distM = Math.max(200, (v.sog * 1852 * s.contourMin) / 60);
        invoke("chart_shallow_ahead", { lat: b.pos[1], lon: b.pos[0], cog: v.cog, distM, depth: contour }).then((m) => {
          if (m != null) {
            const mins = v.sog > 0 ? (m / 1852 / v.sog) * 60 : 0;
            raise("contour", { level: "alarm", stage: stageBelow(mins, s.contourMin), text: `SAFETY CONTOUR AHEAD: ${m < 1852 ? Math.round(m) + " m" : (m / 1852).toFixed(1) + " NM"} (${mins < 1 ? "under a minute" : Math.round(mins) + " min"})`,
              detail: `Water shallower than the ${contour} m safety contour, or land, on your course over ground (charted depths; not the high-resolution depths)` });
          } else clear("contour");
        }).catch(() => {});
      }
    } else clear("contour");
    // off track and approaching the next mark (Route › Navigate)
    const n = b.live ? getNav() : null;
    if (s.xte && n?.wp?.xte != null && n.nav.from && Math.abs(n.wp.xte) > s.xteNm) {
      raise("xte", { level: "alarm", stage: stageAbove(Math.abs(n.wp.xte), s.xteNm), text: `OFF TRACK ${Math.abs(n.wp.xte).toFixed(2)} NM ${n.wp.xte > 0 ? "right" : "left"} of the leg`, detail: `Cross-track error over ${s.xteNm} NM on the leg to ${n.nav.next.name}` });
    } else if (!(s.xte && n?.wp?.xte != null && Math.abs(n.wp.xte) > s.xteNm * 0.9)) clear("xte");
    if (s.arrive && n?.wp) {
      const key = `${n.nav.index}:${n.nav.next.name}`;
      const soon = (n.wp.ttg != null && n.wp.ttg / 60 <= s.arriveMin) || n.wp.dtw <= (n.arrivalNm ?? 0);
      if (soon) {
        arrivedKey = key;
        raise("arrive", { level: "warn", stage: n.wp.ttg != null ? stageBelow(n.wp.ttg / 60, s.arriveMin) : 0, text: `APPROACHING ${n.nav.next.name.toUpperCase()}`, detail: `${n.wp.dtw.toFixed(2)} NM, bearing ${String(Math.round(n.wp.btw)).padStart(3, "0")}°T${n.wp.ttg != null ? `, about ${Math.max(1, Math.round(n.wp.ttg / 60))} min` : ""}` });
      } else if (arrivedKey !== key) clear("arrive");
    } else clear("arrive");
    if (mob) mobAlarm();
    drawGeo();
  }
  if (PRIMARY) setInterval(check, 1000);

  // ---------- settings (the Alarms drawer) -------------------------------------------
  function syncUi() {
    $("al-shallow").checked = s.shallow;
    $("al-shallow-m").value = s.shallowM ?? "";
    $("al-shallow-m").placeholder = String(getSafetyDepth());
    $("al-air").checked = s.air;
    $("al-air-margin").value = s.airMargin;
    $("al-poslost").checked = s.posLost;
    $("al-contour").checked = s.contour;
    $("al-contour-min").value = String(s.contourMin);
    $("al-xte").checked = s.xte;
    $("al-xte-nm").value = s.xteNm;
    $("al-arrive").checked = s.arrive;
    $("al-arrive-min").value = s.arriveMin;
    $("al-watchdog").value = s.watchdogMin;
    $("al-volume").value = Math.round(s.volume * 100);
    $("anc-radius").value = s.anchorM;
    $("anc-drop").textContent = s.anchor ? "Weigh anchor" : "Drop anchor";
    if (!s.anchor) $("anc-state").textContent = "Off. Drop anchor (here, the Anchor button on the chart, or right-click the chart), then drag the anchor to where it lies and set its swing radius on its card.";
    $("anchor-btn")?.classList.toggle("on", !!s.anchor);
    window.dispatchEvent(new Event("racenav-anchor-watch"));
    const air = getAirDraft();
    $("al-air-note").textContent = air > 0 ? `Air draft ${air} m (Settings).` : "Set the air draft in Settings first.";
  }
  $("al-shallow").onchange = (e) => { s.shallow = e.target.checked; save(); };
  $("al-shallow-m").onchange = (e) => { const v = parseFloat(e.target.value); s.shallowM = Number.isFinite(v) && v > 0 ? v : null; save(); syncUi(); };
  $("al-air").onchange = (e) => { s.air = e.target.checked; save(); };
  $("al-air-margin").onchange = (e) => { const v = parseFloat(e.target.value); if (Number.isFinite(v) && v >= 0) s.airMargin = v; save(); };
  $("al-poslost").onchange = (e) => { s.posLost = e.target.checked; save(); };
  $("al-contour").onchange = (e) => { s.contour = e.target.checked; lastContourCheck = 0; save(); };
  $("al-contour-min").onchange = (e) => { s.contourMin = +e.target.value || 6; lastContourCheck = 0; save(); };
  $("al-xte").onchange = (e) => { s.xte = e.target.checked; save(); };
  $("al-xte-nm").onchange = (e) => { const v = parseFloat(e.target.value); if (Number.isFinite(v) && v > 0) s.xteNm = v; save(); syncUi(); };
  $("al-watchdog").onchange = (e) => { const v = Math.round(parseFloat(e.target.value)); if (Number.isFinite(v)) s.watchdogMin = Math.min(20, Math.max(1, v)); save(); syncUi(); };
  $("al-arrive").onchange = (e) => { s.arrive = e.target.checked; save(); };
  $("al-arrive-min").onchange = (e) => { const v = parseFloat(e.target.value); if (Number.isFinite(v) && v > 0) s.arriveMin = v; save(); syncUi(); };
  $("al-volume").oninput = (e) => { s.volume = +e.target.value / 100; save(); };
  $("al-test").onclick = () => sound.alarm();
  for (const [id, key] of [["al-snd-alarm", "soundAlarm"], ["al-snd-warn", "soundWarn"], ["al-snd-anchor", "soundAnchor"]]) {
    const sel = $(id);
    if (!sel) continue;
    sel.innerHTML = Object.entries(SOUNDS).map(([k, label]) => `<option value="${k}">${esc(label)}</option>`).join("");
    sel.value = s[key];
    sel.onchange = () => { s[key] = sel.value; save(); snd.play(sel.value); };
    $(`${id}-test`).onclick = () => snd.play(s[key]);
  }
  let ancTimer = 0; // the spinner sends a change on every click: save once it stops
  $("anc-radius").onchange = (e) => { clearTimeout(ancTimer); ancTimer = setTimeout(() => { const v = parseFloat(e.target.value); if (Number.isFinite(v) && v > 5) s.anchorM = v; save(); drawGeo(); }, 800); };
  // ---------- anchor watch: the anchor itself (a library mark, dragged into place, with its
  // card) is anchor.js; the watch on it lives here, in the primary window ----------------
  const anchorRadius = () => Math.max(5, Number(s.anchor?.radius) || s.anchorM);
  /** Watch an anchor ({lon, lat, id, radius, name}), or change the one watched; null weighs it. */
  function anchorSet(a) {
    if (!PRIMARY) { command("anchor-set", null, a); return; }
    s.anchor = a ? { lon: a.lon, lat: a.lat, id: a.id ?? null, radius: Number(a.radius) || s.anchorM, name: a.name ?? "" } : null;
    if (!s.anchor) clear("anchor");
    save(); syncUi(); drawGeo();
  }
  $("anc-drop").onclick = () => window.dispatchEvent(new Event(s.anchor ? "racenav-anchor-weigh" : "racenav-anchor-drop"));
  syncUi();
  renderDrawer();
  window.addEventListener("racenav-boat", syncUi);

  if (PRIMARY) {
    listen("alarm-cmd", ({ payload: { act, id, extra } }) => {
      if (act === "ack") ack(id);
      else if (act === "dismiss") active.get(id)?.dismiss?.();
      else if (act === "mob") pressMob();
      else if (act === "clearmob") clearMob();
      else if (act === "anchor-set") anchorSet(extra ?? null);
    });
    // a monitor window changed the alarm settings
    window.addEventListener("storage", (e) => {
      if (e.key !== KEY || !e.newValue) return;
      try { Object.assign(s, JSON.parse(e.newValue), { anchor: s.anchor }); syncUi(); drawGeo(); } catch { /* ignore */ }
    });
  } else {
    // the MOB and anchor live in storage, written by the primary
    window.addEventListener("storage", (e) => {
      try {
        if (e.key === KEY && e.newValue) Object.assign(s, JSON.parse(e.newValue));
        else if (e.key === KEY + ".mob") mob = JSON.parse(e.newValue || "null");
        else return;
      } catch { return; }
      $("mob-btn").classList.toggle("on", !!mob);
      syncUi(); drawGeo(); mobChanged();
    });
    setInterval(drawGeo, 1000);
  }

  const list = () => [...active.values()].map((a) => ({ level: a.level, text: a.text, detail: a.detail, acked: a.acked }));
  /** Silence every sounding alarm (they stay listed until cleared); how many were silenced. */
  function ackAll() {
    const ids = entries().filter(([, a]) => !a.acked).map(([id]) => id);
    for (const id of ids) ack(id);
    return ids.length;
  }
  return { raise, clear, ack, ackAll, sound, list, active: () => [...active.keys()], sync: syncUi,
    /** The anchor watch: get() the watched anchor or null, set(a) to watch one, weigh() to stop. */
    anchor: { get: () => (s.anchor ? { ...s.anchor, radius: anchorRadius() } : null), set: anchorSet, weigh: () => anchorSet(null), defaultRadius: () => s.anchorM } };
}
