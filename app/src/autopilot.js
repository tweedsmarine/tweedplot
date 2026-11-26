// Steering the autopilot along the route: once a second, while "Send to the autopilot" is on and a
// mark is navigated, the leg goes out as NMEA 0183 RMB, APB, XTE, BOD, BWC and AAM on the chosen
// serial or TCP connection, or a UDP one with a "send to" address (the sentences are built in the back end, nmea.rs and marine-nmea).
//
// Safety: the switch is off each time TweedPlot starts (it is never remembered), and nothing is
// sent while nothing is navigated, so an autopilot in Track mode raises its own lost-data alarm
// rather than steering for an old mark. Where it goes (the source) and the talker are remembered.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { bearing } from "./instruments.js";

const KEY = "racenav.autopilot";
const $ = (id) => document.getElementById(id);
const norm = (d) => ((d % 360) + 360) % 360;
const off180 = (d) => ((d + 540) % 360) - 180;

/** Waypoint IDs (8 characters, as the sentences carry them) that tell the leg's two marks apart:
 * "Wellington approach" and "Wellington entrance" would both be "Wellingt", so they become
 * "Wel appr" and "Wel entr" (or numbered, "Welli18" and "Welli19", when that isn't enough). Exported for tests. */
export function distinctIds(a, b, n = null) {
  const clean = (x) => String(x ?? "").replace(/[^A-Za-z0-9 ._-]/g, "").trim();
  const sa = clean(a), sb = clean(b);
  if (!sa || !sb || sa === sb || sa.slice(0, 8).trim() !== sb.slice(0, 8).trim()) return [a, b];
  // where they differ, back to the start of that word or number
  let p = 0;
  while (sa[p] === sb[p]) p++;
  while (p > 0 && sa[p - 1] !== " ") p--;
  const f = (x) => {
    const tail = x.slice(p).trim();
    // a short difference (a number) keeps as much of the name as fits; a long one, its first letters
    return (tail.length <= 4 ? `${x.slice(0, 7 - tail.length).trim()} ${tail}` : `${x.slice(0, 3).trim()} ${tail}`).slice(0, 8).trim();
  };
  const [ia, ib] = p > 0 ? [f(sa), f(sb)] : [null, null];
  if (ia && ib && ia !== ib) return [ia, ib];
  const num = n ?? 2;
  return [`${sa.slice(0, 5).trim()}${num - 1}`, `${sb.slice(0, 5).trim()}${num}`];
}

/** The leg as the autopilot sentences describe it, from the navigated mark and the boat. Exported for tests. */
export function legFor({ nav, wp, boat, variation, arrivalNm }) {
  if (!nav?.next || !wp || !boat || wp.dtw == null || wp.btw == null) return null;
  const dest = [nav.next.lon, nav.next.lat];
  const leg = nav.from ? bearing(nav.from, dest) : wp.btw;
  const xte = wp.xte ?? 0;
  // the course to steer back onto the track, as the Highway works it out
  const correction = Math.min(30, (Math.atan2(Math.abs(xte), Math.max(wp.dtw, 0.1)) * 180) / Math.PI);
  const steer = norm(leg - Math.sign(xte) * correction);
  // past the line square to the leg through the mark
  const passed = Math.abs(off180(bearing(dest, boat) - leg)) < 90;
  const [originId, destId] = distinctIds(nav.goto ? "Here" : nav.fromName ?? "Start", nav.next.name ?? "Mark", nav.next.n);
  return {
    xteNm: xte,
    originId,
    destId,
    destLat: nav.next.lat,
    destLon: nav.next.lon,
    rangeNm: wp.dtw,
    bearingTrue: wp.btw,
    bearingLegTrue: leg,
    vmcKn: Number.isFinite(wp.vmc) ? wp.vmc : null,
    steerTrue: steer,
    arrived: wp.dtw <= (arrivalNm ?? 0.1),
    passed,
    arrivalRadiusNm: arrivalNm ?? 0.1,
    variation: Number.isFinite(variation) ? variation : null,
  };
}

export function setupAutopilot({ getLeg }) {
  let prefs = { source: "", talker: "GP" };
  try { prefs = { ...prefs, ...JSON.parse(localStorage.getItem(KEY) || "{}") }; } catch { /* defaults */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };

  const send = $("ap-send"), note = $("ap-note"), src = $("ap-source"), talker = $("ap-talker");
  if (!send) return null;
  send.checked = false; // never on at start-up

  let sources = [];
  async function loadSources() {
    try { sources = (await invoke("nmea_sources_get"))?.sources ?? []; } catch { sources = []; }
    // UDP sends only with a "send to" address (a UDP input on its own only listens)
    const usable = sources.filter((s) => s.transport?.type === "serial" || s.transport?.type === "tcp" || (s.transport?.type === "udp" && s.transport.sendTo));
    const where = (t) => (t.type === "serial" ? t.port : t.type === "tcp" ? t.address : `UDP to ${t.sendTo}`);
    if (src) {
      src.replaceChildren(new Option("Not set", ""), ...usable.map((s) => new Option(`${s.name} (${where(s.transport)})`, s.id)));
      src.value = usable.some((s) => s.id === prefs.source) ? prefs.source : "";
    }
    show();
  }
  src?.addEventListener("change", () => { prefs.source = src.value; save(); show(); });
  if (talker) {
    talker.value = prefs.talker;
    talker.addEventListener("change", () => { prefs.talker = talker.value; save(); });
  }
  send.addEventListener("change", () => { last = null; show(); tick(); });
  listen("nmea-sources-changed", () => loadSources()).catch(() => {});

  let last = null; // the last result: { ok, text }
  function show(text) {
    const name = sources.find((s) => s.id === prefs.source)?.name;
    send.disabled = !prefs.source;
    let t, warn = false;
    if (!prefs.source) t = "Choose the connection to the autopilot in Settings › Connections › Autopilot output.";
    else if (!send.checked) t = `Off. On sends the leg to the autopilot on ${name ?? "the chosen connection"} each second while navigating.`;
    else t = text ?? last?.text ?? "Starting…";
    if (send.checked && last && !last.ok) warn = true;
    note.textContent = t;
    note.classList.toggle("warn", warn);
  }

  async function tick() {
    if (!send.checked || !prefs.source) return;
    const l = getLeg();
    const leg = l ? legFor(l) : null;
    if (!leg) {
      last = { ok: false, text: "Not navigating: nothing is sent. An autopilot that was in Track mode will alarm for lost data." };
      show();
      return;
    }
    try {
      await invoke("autopilot_send", { sourceId: prefs.source, talker: prefs.talker, nav: leg });
      const side = leg.xteNm >= 0 ? "L" : "R";
      last = { ok: true, text: `Sending RMB, APB, XTE, BOD, BWC and AAM: to ${leg.destId}, ${leg.rangeNm.toFixed(2)} NM, XTE ${Math.abs(leg.xteNm).toFixed(2)} NM, steer ${side}.` };
    } catch (e) {
      last = { ok: false, text: `Not sent: ${e}` };
    }
    show();
  }
  setInterval(tick, 1000);
  loadSources();
  return { refresh: loadSources, sending: () => send.checked && !!prefs.source };
}
