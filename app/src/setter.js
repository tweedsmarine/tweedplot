// The line-setting window: the snood and float beeper for setting a longline, in its own
// always-on-top window (setter.html). It replaces the hardware hook-and-buoy timers.
//
// The pattern, one basket after another: a float (long tone, 1 s, higher), k empty
// intervals, N snoods (short tones, 0.5 s), k empty intervals, the next float … The
// interval is set in seconds (4 to 20), or worked out from the snood spacing on the line
// and the line speed: the shooter's (typed in, or from its wheel and RPM), else SOG (a boat
// setting without a shooter). Tones are scheduled on the sound card's clock (Web Audio), a
// second or so ahead, so they keep exact time even if the computer is busy.
//
// Start logs "Line set" (with the radio buoys at each end, and the settings) in the Library
// log; each float can be logged with its position; Finish logs "Line set finished" with the
// counts. Weights and radio buoys may get tones of their own later (asked of skippers):
// the beat list below already has a type per beat, so a weight or buoy beat is one more type.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./clock24.js";
import { setWindowDark } from "./winchrome.js";

const $ = (id) => document.getElementById(id);
const KEY = "racenav.setter";
const KN = 1852 / 3600; // m/s in a knot
const DEF = { mode: "time", interval: 8, spacing: 50, shooter: null, wheel: null, rpm: null, snoods: 20, blanks: 2, hz1: 1000, hz2: 1500, wave: "square", vol: 0.8, out: "", logFloats: true, markFloats: false, target: null, presets: {} };
let cfg = { ...DEF };
try { cfg = { ...DEF, ...JSON.parse(localStorage.getItem(KEY) || "{}") }; } catch { /* storage unavailable */ }
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(cfg)); } catch { /* ignore */ } };

// ---------- colours: follow the main window's day / dusk / night ----------
function palette() {
  try {
    const p = JSON.parse(localStorage.getItem("racenav.palette") || "{}");
    document.documentElement.dataset.palette = p.mode || "day";
    document.documentElement.style.setProperty("--pal-dim", String((p.dim?.[p.mode] ?? 100) / 100));
    setWindowDark((p.mode || "day") !== "day"); // the title bar too
  } catch { /* day */ }
}
palette();
window.addEventListener("storage", (e) => { if (e.key === "racenav.palette") palette(); });

// ---------- the boat (for SOG and the float positions) ----------
let boat = null; // { lat, lon, sog, cog, depth, water_temp }
listen("vessel", ({ payload }) => {
  if (Number.isFinite(payload?.lat) && Number.isFinite(payload?.lon)) boat = payload;
  else if (payload) boat = { ...boat, sog: payload.sog ?? boat?.sog };
}).catch(() => {});

// ---------- sound ----------
let ac = null;
async function audio() {
  if (!ac) {
    ac = new AudioContext({ latencyHint: "interactive" });
    if (cfg.out && ac.setSinkId) await ac.setSinkId(cfg.out).catch(() => {});
  }
  if (ac.state === "suspended") await ac.resume();
  return ac;
}
const playing = new Set();
function tone(at, hz, secs) {
  const o = ac.createOscillator(), g = ac.createGain();
  o.type = cfg.wave === "sine" ? "sine" : "square";
  o.frequency.value = hz;
  // a square wave is louder for the same setting: bring it down to match
  const peak = cfg.vol * (o.type === "square" ? 0.35 : 0.9);
  g.gain.setValueAtTime(0, at);
  g.gain.linearRampToValueAtTime(peak, at + 0.01);
  g.gain.setValueAtTime(peak, at + secs - 0.015);
  g.gain.linearRampToValueAtTime(0, at + secs);
  o.connect(g).connect(ac.destination);
  o.start(at); o.stop(at + secs + 0.02);
  playing.add(o);
  o.onended = () => playing.delete(o);
}
const SOUND = {
  snood: (t) => tone(t, cfg.hz1, 0.5),
  float: (t) => tone(t, cfg.hz2, 1.0),
  end: (t) => { for (let i = 0; i < 3; i++) tone(t + i * 0.4, cfg.hz2, 0.25); },
};
async function fillOutputs() {
  const sel = $("st-out");
  try {
    const devs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audiooutput" && d.deviceId !== "default");
    devs.forEach((d, i) => sel.append(new Option(d.label || `Speaker ${i + 1}`, d.deviceId)));
  } catch { /* none listed */ }
  sel.value = cfg.out;
  sel.disabled = !("setSinkId" in AudioContext.prototype);
  sel.onchange = async () => { cfg.out = sel.value; save(); if (ac?.setSinkId) await ac.setSinkId(cfg.out).catch(() => {}); };
}

// ---------- the pattern ----------
// a set keeps the pattern it started with (changing it part way would jump the crew about);
// the interval can change at any time
const pat = () => run?.pat ?? { snoods: cfg.snoods, blanks: cfg.blanks };
const period = () => pat().snoods - 1 + 2 * pat().blanks; // beats from one float to the next
function beatType(off) {
  const { snoods, blanks } = pat();
  if (off === 0) return "float";
  if (off >= blanks && off < blanks + snoods) return "snood";
  return "blank";
}
function interval() {
  if (cfg.mode !== "distance") return { s: clamp(cfg.interval), why: "" };
  const shooter = Number(cfg.shooter) > 0 ? Number(cfg.shooter) : null;
  const speed = shooter ?? (boat?.sog > 0.3 ? boat.sog : null);
  if (!speed) return { s: clamp(cfg.interval), why: "no line speed yet: the set interval is used" };
  const s = cfg.spacing / (speed * KN);
  return { s: clamp(s), why: s < 4 || s > 20 ? `${s.toFixed(1)} s worked out, kept between 4 and 20 s` : "" };
}
function clamp(s) { return Math.min(20, Math.max(4, Number(s) || 8)); }
function renderPattern() {
  const p = [];
  for (let off = 0; off <= period(); off++) {
    const t = off === period() ? "float" : beatType(off);
    p.push(t === "float" ? "F" : t === "snood" ? "s" : "·");
  }
  $("st-pattern").textContent = `One basket: ${p.join(" ")}  (F float, s snood, · empty)`;
}

// ---------- the set ----------
let run = null; // { state, t0wall, beats, next: { off, at }, hooks, floats, baskets, last, ending, pausedFor, eventId }
let timerLoop = 0;

function nameRun(r) {
  r.startText = new Date().toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  r.group = `Set ${r.startText}`;
}
function newRun() {
  return { state: "running", pat: { snoods: Math.max(1, cfg.snoods | 0), blanks: Math.min(4, Math.max(1, cfg.blanks | 0)) }, startedWall: Date.now(), activeMs: 0, resumedWall: Date.now(), queue: [], nextOff: 0, nextAt: 0,
    hooks: 0, floats: 0, baskets: 0, last: false, ending: false, endAt: null };
}

async function start() {
  await audio();
  const b1 = $("st-b1").value.trim();
  cfg.target = Number($("st-target-in").value) || null; save();
  // the set in the log first: if the log can't be written, say so but still beep
  const pos = boat && Number.isFinite(boat.lat) ? boat : null;
  const iv = interval();
  try {
    const ev = await invoke("event_save", { event: {
      kind: "line-set", name: "Line set", at: Math.floor(Date.now() / 1000),
      lat: pos?.lat ?? null, lon: pos?.lon ?? null, depth: pos?.depth ?? null, temp: pos?.water_temp ?? null, sog: pos?.sog ?? null, cog: pos?.cog ?? null,
      notes: [b1 && `radio buoy ${b1} at the start`, $("st-notes").value.trim()].filter(Boolean).join(" · "),
      data: { buoy_start: b1 || null, buoy_end: $("st-b2").value.trim() || null, mode: cfg.mode, interval_s: iv.s, spacing_m: cfg.mode === "distance" ? cfg.spacing : null,
        shooter_kn: Number(cfg.shooter) || null, snoods_per_basket: cfg.snoods, blanks: cfg.blanks, target_hooks: cfg.target },
    } });
    run = newRun();
    run.eventId = ev.id;
    nameRun(run);
    if (b1) logBuoy(b1, "start");
  } catch (e) {
    run = newRun();
    nameRun(run);
    setNote(`Couldn't write the log: ${e}`);
  }
  // a short lead-in, then the first float
  run.nextAt = ac.currentTime + 3;
  setButtons();
  loop();
}

function schedule() {
  if (!run || run.state !== "running") return;
  // a second and a half ahead on the sound card's clock; further while the window is hidden,
  // in case the system slows its timers (the window asks it not to)
  const ahead = document.hidden ? 30 : 1.5;
  while (!run.ending && run.nextAt < ac.currentTime + ahead) {
    const off = run.nextOff % period();
    const type = beatType(off);
    run.queue.push({ type, at: run.nextAt, off, n: run.nextOff });
    if (type !== "blank") SOUND[type](run.nextAt);
    // the set ends at a float: after Last basket, or once the hooks to set are reached
    const hooksAfter = run.hooks + run.queue.filter((q) => q.type === "snood").length;
    if (type === "float" && run.nextOff > 0 && (run.last || (cfg.target && hooksAfter >= cfg.target))) {
      run.ending = true;
      run.endAt = run.nextAt + 1.5;
      SOUND.end(run.endAt);
      break;
    }
    run.nextOff += 1;
    run.nextAt += interval().s;
  }
}

function loop() {
  clearInterval(timerLoop);
  timerLoop = setInterval(tick, 100);
  tick();
}

function tick() {
  if (!run) return;
  schedule();
  const now = ac.currentTime;
  // beats now gone by: count them, and log the floats
  while (run.queue.length && run.queue[0].at <= now) {
    const b = run.queue.shift();
    if (b.type === "snood") run.hooks += 1;
    if (b.type === "float") {
      run.floats += 1;
      if (run.floats > 1) run.baskets += 1;
      if (cfg.logFloats) logFloat(run.floats);
      if (cfg.markFloats) markAt(`Float ${run.floats}`, "sym-float", "#d35400", `float ${run.floats}`);
    }
  }
  if (run.ending && run.endAt && now >= run.endAt + 1) { finish(); return; }
  render();
}

/** The next beat that sounds (a snood or a float), past any empty ones, even if it isn't
 *  scheduled yet: the countdown is to the next tone, not to the next interval. */
function nextSounded() {
  const q = run.queue.find((x) => x.type !== "blank");
  if (q) return q;
  if (run.ending) return null;
  const iv = interval().s;
  let n = run.queue.length ? run.queue.at(-1).n + 1 : run.nextOff;
  let at = run.queue.length ? run.queue.at(-1).at + iv : run.nextAt;
  if (run.queue.length && n !== run.nextOff) { n = run.nextOff; at = run.nextAt; }
  for (let i = 0; i <= period(); i++, n++, at += iv) {
    const off = n % period(), type = beatType(off);
    if (type !== "blank") return { type, at, off, n };
  }
  return null;
}

function render() {
  const box = document.querySelector(".st-next");
  if (!run) { $("st-count").textContent = "–"; $("st-what").textContent = "Press Start to begin the set"; box.className = "st-next"; return; }
  const paused = run.state === "paused";
  const sounded = run.state === "running" ? nextSounded() : null;
  box.className = `st-next${paused ? " paused" : sounded?.type === "float" ? " float" : ""}`;
  if (paused) { $("st-count").textContent = "II"; $("st-what").textContent = "Paused: Resume carries on with the pattern"; }
  else if (run.ending && !sounded) { $("st-count").textContent = "✓"; $("st-what").textContent = "Last float: finishing"; }
  else if (sounded) {
    const s = Math.max(0, sounded.at - ac.currentTime);
    // whole seconds, counting down to the tone (8 … 1, then the beep)
    $("st-count").textContent = String(Math.max(1, Math.ceil(s - 0.05)));
    // which snood of the basket this is: 1 to N
    const inBasket = sounded.off - pat().blanks + 1;
    $("st-what").textContent = sounded.type === "float" ? (run.floats === 0 ? "FIRST FLOAT" : "FLOAT NEXT") : `Snood ${inBasket} of ${pat().snoods}`;
  }
  $("st-hooks").textContent = run.hooks.toLocaleString();
  $("st-floats").textContent = run.floats.toLocaleString();
  $("st-baskets").textContent = run.baskets.toLocaleString();
  $("st-target").textContent = cfg.target ? `of ${cfg.target.toLocaleString()}` : "";
  const ms = run.activeMs + (run.state === "running" ? Date.now() - run.resumedWall : 0);
  $("st-time").textContent = `${Math.floor(ms / 3600000) ? `${Math.floor(ms / 3600000)}:` : ""}${String(Math.floor(ms / 60000) % 60).padStart(Math.floor(ms / 3600000) ? 2 : 1, "0")}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
  $("st-state").textContent = run.state === "running" ? (run.last || run.ending ? "Setting: last basket" : "Setting") : run.state === "paused" ? "Paused" : "Finished";
  live();
}

function live() {
  const iv = interval();
  const shooter = Number(cfg.shooter) > 0 ? Number(cfg.shooter) : null;
  const sog = boat?.sog;
  const parts = [`interval ${iv.s.toFixed(1)} s`];
  if (sog != null) parts.push(`SOG ${sog.toFixed(1)} kn`);
  if (shooter) parts.push(`shooter ${shooter.toFixed(1)} kn`);
  // sag ratio: the boat's speed over the line's (below 1 the line sags, and fishes deeper)
  if (shooter && sog > 0.3) parts.push(`sag ratio ${(sog / shooter).toFixed(2)}`);
  if (cfg.mode === "distance") parts.push(`${(iv.s * (shooter ?? sog ?? 0) * KN).toFixed(0)} m apart`);
  if (iv.why) parts.push(iv.why);
  $("st-live").textContent = parts.join(" · ");
}
function setNote(t) { $("st-live").textContent = t; }

function cancelQueued() {
  for (const o of playing) { try { o.stop(); } catch { /* gone */ } }
  playing.clear();
  // the beats not played yet go back into the pattern
  if (run?.queue.length) run.nextOff = run.queue[0].n;
  if (run) run.queue = [];
}

function pause() {
  if (!run) return;
  if (run.state === "running") {
    cancelQueued();
    run.state = "paused";
    run.activeMs += Date.now() - run.resumedWall;
    run.ending = false; run.endAt = null;
  } else if (run.state === "paused") {
    run.state = "running";
    run.resumedWall = Date.now();
    run.nextAt = ac.currentTime + 2;
  }
  setButtons(); render();
}

function shift(n) {
  if (!run || run.state !== "running") return;
  cancelQueued();
  run.nextOff = Math.max(0, run.nextOff + n);
  run.nextAt = ac.currentTime + (n > 0 ? 0.3 : interval().s);
  render();
}

async function finish() {
  if (!run || run.state === "finished") return;
  if (run.state === "running") run.activeMs += Date.now() - run.resumedWall;
  cancelQueued();
  clearInterval(timerLoop);
  run.state = "finished";
  const b2 = $("st-b2").value.trim();
  const pos = boat && Number.isFinite(boat.lat) ? boat : null;
  try {
    await invoke("event_save", { event: {
      kind: "line-end", name: "Line set finished", at: Math.floor(Date.now() / 1000),
      lat: pos?.lat ?? null, lon: pos?.lon ?? null, depth: pos?.depth ?? null, temp: pos?.water_temp ?? null, sog: pos?.sog ?? null, cog: pos?.cog ?? null,
      notes: `${run.hooks.toLocaleString()} hooks, ${run.floats} floats, ${run.baskets} baskets in ${$("st-time").textContent}${b2 ? ` · radio buoy ${b2} at the end` : ""}`,
      pair: run.eventId ?? "",
      data: { hooks: run.hooks, floats: run.floats, baskets: run.baskets, minutes: Math.round(run.activeMs / 60000), buoy_end: b2 || null, set: run.eventId ?? null },
    } });
    if (b2) logBuoy(b2, "end");
  } catch (e) { setNote(`Couldn't write the log: ${e}`); }
  setButtons(); render();
}

function logFloat(n) {
  const pos = boat && Number.isFinite(boat.lat) ? boat : null;
  if (!pos) return;
  invoke("event_save", { event: { kind: "float", name: `Float ${n}`, at: Math.floor(Date.now() / 1000), lat: pos.lat, lon: pos.lon, depth: pos.depth ?? null, temp: pos.water_temp ?? null, sog: pos.sog ?? null, cog: pos.cog ?? null, notes: "", data: { n, set: run?.eventId ?? null } } }).catch(() => {});
}
// a library mark at a float or radio buoy: grouped by set ("Set 28 Oct 05:30"), so the whole
// line can be shown, hidden or deleted together in Library > Marks > Groups
function markAt(name, kind, colour, what) {
  const pos = boat && Number.isFinite(boat.lat) ? boat : null;
  if (!pos || !run) return;
  const b1 = $("st-b1").value.trim(), b2 = $("st-b2").value.trim();
  invoke("mark_save", { mark: {
    name, lat: pos.lat, lon: pos.lon, kind, colour, group: run.group, created: Math.floor(Date.now() / 1000),
    depth: pos.depth ?? null, temp: pos.water_temp ?? null, sog: pos.sog ?? null, tags: "line-set",
    notes: [`${what} of the set started ${run.startText}`, (b1 || b2) && `radio buoys ${[b1, b2].filter(Boolean).join(" / ")}`].filter(Boolean).join(" · "),
  } }).catch(() => {});
}
function logBuoy(name, where) {
  if (cfg.markFloats) markAt(`Radio buoy ${name}`, "sym-float-flag", "#c2185b", `radio buoy ${name} (${where === "middle" ? "part way" : `at the ${where}`})`);
  const pos = boat && Number.isFinite(boat.lat) ? boat : null;
  invoke("event_save", { event: { kind: "radio-buoy", name: `Radio buoy ${name}`, at: Math.floor(Date.now() / 1000), lat: pos?.lat ?? null, lon: pos?.lon ?? null, depth: null, temp: null, sog: pos?.sog ?? null, cog: pos?.cog ?? null,
    notes: where === "start" ? "at the start of the line" : where === "end" ? "at the end of the line" : "", data: { buoy: name, where, set: run?.eventId ?? null } } }).catch(() => {});
}

function setButtons() {
  const on = run?.state === "running", paused = run?.state === "paused", done = !run || run.state === "finished";
  $("st-start").disabled = !done;
  $("st-start").textContent = run?.state === "finished" ? "Start another set" : "Start";
  $("st-pause").disabled = done;
  $("st-pause").textContent = paused ? "Resume" : "Pause";
  for (const id of ["st-back", "st-fwd", "st-last"]) $(id).disabled = !on;
  $("st-last").classList.toggle("on", !!run?.last);
  $("st-buoy").disabled = done;
  $("st-finish").disabled = done;
  document.title = on ? `Setting · ${run.hooks} hooks` : "TweedPlot line setting";
}

// ---------- the controls ----------
$("st-start").onclick = () => { if (run?.state === "finished") { run = null; } start(); };
$("st-pause").onclick = pause;
$("st-back").onclick = () => shift(-1);
$("st-fwd").onclick = () => shift(1);
$("st-last").onclick = () => { if (run) { run.last = !run.last; setButtons(); } };
$("st-finish").onclick = () => { const b = $("st-finish"); if (!b.dataset.armed) { b.dataset.armed = "1"; b.textContent = "Finish now?"; setTimeout(() => { delete b.dataset.armed; b.textContent = "Finish"; }, 3000); return; } delete b.dataset.armed; b.textContent = "Finish"; finish(); };
$("st-buoy").onclick = () => {
  const row = $("st-buoy");
  // a small box under the buttons (no browser prompts)
  const box = document.createElement("div");
  box.className = "wx-row";
  box.innerHTML = `<input type="text" placeholder="radio buoy name or number" class="grow"><button class="on">Log it</button><button>Cancel</button>`;
  row.closest(".st-buttons").after(box);
  const [inp, ok, no] = [box.querySelector("input"), ...box.querySelectorAll("button")];
  inp.focus();
  ok.onclick = () => { if (inp.value.trim()) logBuoy(inp.value.trim(), "middle"); box.remove(); };
  no.onclick = () => box.remove();
  inp.onkeydown = (e) => { if (e.key === "Enter") ok.click(); if (e.key === "Escape") no.click(); };
};
window.addEventListener("keydown", (e) => {
  if (e.target.closest("input, textarea, select")) return;
  if (e.key === " ") { e.preventDefault(); if (run && run.state !== "finished") pause(); }
});

function bindNumber(id, key, after = () => {}) {
  const el = $(id);
  el.value = cfg[key] ?? "";
  el.onchange = () => { cfg[key] = el.value === "" ? null : Number(el.value); save(); after(); };
}
bindNumber("st-interval", "interval");
bindNumber("st-spacing", "spacing");
bindNumber("st-shooter", "shooter");
bindNumber("st-snoods", "snoods", renderPattern);
bindNumber("st-blanks", "blanks", renderPattern);
bindNumber("st-hz1", "hz1");
bindNumber("st-hz2", "hz2");
const fromWheel = () => { if (cfg.wheel > 0 && cfg.rpm > 0) { cfg.shooter = Math.round(((cfg.wheel * cfg.rpm * 60) / 1852) * 10) / 10; $("st-shooter").value = cfg.shooter; save(); } };
bindNumber("st-wheel", "wheel", fromWheel);
bindNumber("st-rpm", "rpm", fromWheel);
$("st-mode").value = cfg.mode;
const showMode = () => { document.querySelector(".st-dist-rows").hidden = cfg.mode !== "distance"; };
$("st-mode").onchange = (e) => { cfg.mode = e.target.value; save(); showMode(); };
$("st-wave").value = cfg.wave;
$("st-wave").onchange = (e) => { cfg.wave = e.target.value; save(); };
$("st-vol").value = cfg.vol;
$("st-vol").oninput = (e) => { cfg.vol = Number(e.target.value); save(); };
$("st-logfloats").checked = cfg.logFloats;
$("st-logfloats").onchange = (e) => { cfg.logFloats = e.target.checked; save(); };
$("st-markfloats").checked = cfg.markFloats;
$("st-markfloats").onchange = (e) => { cfg.markFloats = e.target.checked; save(); };
$("st-target-in").value = cfg.target ?? "";
$("st-test1").onclick = async () => { await audio(); SOUND.snood(ac.currentTime + 0.05); };
$("st-test2").onclick = async () => { await audio(); SOUND.float(ac.currentTime + 0.05); };

// saved settings (a preset per gear: "Bigeye deep", "Swordfish")
const PRESET_KEYS = ["mode", "interval", "spacing", "shooter", "wheel", "rpm", "snoods", "blanks", "hz1", "hz2", "wave", "vol", "logFloats", "markFloats", "target"];
function renderPresets() {
  const sel = $("st-preset");
  sel.innerHTML = `<option value="">—</option>`;
  for (const n of Object.keys(cfg.presets).sort()) sel.append(new Option(n, n));
}
$("st-preset").onchange = (e) => {
  const p = cfg.presets[e.target.value];
  if (!p) return;
  Object.assign(cfg, p); save();
  location.reload(); // simplest way to show every field from the preset
};
$("st-save").onclick = () => {
  const row = $("st-save").closest(".wx-row");
  const box = document.createElement("div");
  box.className = "wx-row";
  box.innerHTML = `<input type="text" class="grow" placeholder="e.g. Bigeye deep"><button class="on">Save</button>`;
  row.after(box);
  const inp = box.querySelector("input");
  inp.value = $("st-preset").value;
  inp.focus();
  const ok = () => { const n = inp.value.trim(); if (!n) return; cfg.presets[n] = Object.fromEntries(PRESET_KEYS.map((k) => [k, cfg[k]])); save(); renderPresets(); $("st-preset").value = n; box.remove(); };
  box.querySelector("button").onclick = ok;
  inp.onkeydown = (e) => { if (e.key === "Enter") ok(); };
};
$("st-del").onclick = () => { const n = $("st-preset").value; if (n) { delete cfg.presets[n]; save(); renderPresets(); } };

setInterval(() => { $("st-clock").textContent = new Date().toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" }); if (!run || run.state !== "running") live(); }, 1000);
showMode();
renderPattern();
renderPresets();
fillOutputs();
setButtons();
render();
// closing the window while setting: ask first (the main window's log keeps what's done)
window.addEventListener("beforeunload", (e) => { if (run?.state === "running" || run?.state === "paused") { e.preventDefault(); e.returnValue = ""; } });
window.__setterRun = () => run; // for debugging in devtools
window.__setterBoat = (b) => { boat = b; }; // for testing without Signal K
