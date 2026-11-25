// Steering the autopilot along the route: once a second, while "Send to the autopilot" is on and a
// mark is navigated, the leg goes out as NMEA 0183 RMB, APB, XTE, BOD and BWC on the chosen
// serial or TCP input (the sentences are built in the back end, nmea.rs and marine-nmea).
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
  return {
    xteNm: xte,
    originId: nav.goto ? "Here" : nav.fromName ?? "Start",
    destId: nav.next.name ?? "Mark",
    destLat: nav.next.lat,
    destLon: nav.next.lon,
    rangeNm: wp.dtw,
    bearingTrue: wp.btw,
    bearingLegTrue: leg,
    vmcKn: Number.isFinite(wp.vmc) ? wp.vmc : null,
    steerTrue: steer,
    arrived: wp.dtw <= (arrivalNm ?? 0.1),
    passed,
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
    const usable = sources.filter((s) => s.transport?.type === "serial" || s.transport?.type === "tcp");
    if (src) {
      src.replaceChildren(new Option("Not set", ""), ...usable.map((s) => new Option(`${s.name} (${s.transport.type === "serial" ? s.transport.port : s.transport.address})`, s.id)));
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
      last = { ok: true, text: `Sending RMB, APB, XTE, BOD and BWC: to ${leg.destId}, ${leg.rangeNm.toFixed(2)} NM, XTE ${Math.abs(leg.xteNm).toFixed(2)} NM, steer ${side}.` };
    } catch (e) {
      last = { ok: false, text: `Not sent: ${e}` };
    }
    show();
  }
  setInterval(tick, 1000);
  loadSources();
  return { refresh: loadSources, sending: () => send.checked && !!prefs.source };
}
