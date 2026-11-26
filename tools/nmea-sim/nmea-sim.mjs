#!/usr/bin/env node
// A simulated passage as NMEA 0183, for demonstrations, screenshots and testing without a boat.
//
//   node nmea-sim.mjs make [route.json | route.gpx] [--out file.nmea] [--speed 7.5] [--no-ais]
//   node nmea-sim.mjs distress [--out distress-test.nmea]
//   node nmea-sim.mjs equipment [--out equipment-test.nmea]
//   node nmea-sim.mjs listen [--port 10111]
//   node nmea-sim.mjs play file.nmea [--udp 127.0.0.1:10110 | --tcp 10110] [--from 02:30] [--rate 1] [--loop] [--keep-time]
//
// make  writes one second of sentences after another: position, course and speed (RMC, GGA, VTG),
//       heading (HDT), speed through the water (VHW), depth (DBT, DPT), apparent and true wind
//       (MWV, MWD), water temperature (MTW), barometer, air temperature and humidity (MDA), and a
//       few invented AIS targets (!AIVDM). The route is a list of waypoints, each with the
//       depth, wind and tidal stream to expect there (see nelson-wellington.json); a GPX route
//       exported from TweedPlot works too (default depth, wind and no stream).
// play  sends the file to TweedPlot a second at a time, as a UDP broadcast or as a TCP server.
//       The times in the file are replaced by the computer's own (--keep-time leaves them), so
//       the passage always happens now. In TweedPlot: Settings › Connections › NMEA data
//       sources › Add source › UDP listener, 127.0.0.1:10110 (or TCP client, 127.0.0.1:10110).
//
// Nothing here is a real vessel: the AIS names, call signs and MMSIs are made up.

import fs from "node:fs";
import path from "node:path";
import dgram from "node:dgram";
import net from "node:net";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const R = Math.PI / 180;
const KN = 1 / 3600; // NM a second, per knot

// ---------- small things ----------
/** A repeatable random number generator (mulberry32), so the same route makes the same file. */
function rng(seed) {
  let a = seed >>> 0;
  return () => { a |= 0; a = (a + 0x6d2b79f5) | 0; let t = Math.imul(a ^ (a >>> 15), 1 | a); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
const norm360 = (d) => ((d % 360) + 360) % 360;
const norm180 = (d) => norm360(d + 180) - 180;
const pad = (n, w) => String(n).padStart(w, "0");
export function checksum(body) { let c = 0; for (let i = 0; i < body.length; i++) c ^= body.charCodeAt(i); return c.toString(16).toUpperCase().padStart(2, "0"); }
export const sentence = (body) => `$${body}*${checksum(body)}`;
const aivdm = (body) => `!${body}*${checksum(body)}`;

/** Distance (NM) and bearing (°T) between two [lon, lat], on a local flat chart (fine for legs of a few miles). */
function leg(a, b) {
  const k = Math.cos(((a[1] + b[1]) / 2) * R);
  const dx = (b[0] - a[0]) * 60 * k, dy = (b[1] - a[1]) * 60;
  return { d: Math.hypot(dx, dy), brg: norm360(Math.atan2(dx, dy) / R) };
}
/** Move [lon, lat] by east and north components in NM. */
function move(p, e, n) { return [p[0] + e / 60 / Math.cos(p[1] * R), p[1] + n / 60]; }

const ddm = (v, w) => { const a = Math.abs(v), d = Math.floor(a); return `${pad(d, w)}${(((a - d) * 60).toFixed(4)).padStart(7, "0")}`; };
const latF = (lat) => `${ddm(lat, 2)},${lat < 0 ? "S" : "N"}`;
const lonF = (lon) => `${ddm(lon, 3)},${lon < 0 ? "W" : "E"}`;
const hms = (t) => `${pad(t.getUTCHours(), 2)}${pad(t.getUTCMinutes(), 2)}${pad(t.getUTCSeconds(), 2)}.00`;
const dmy = (t) => `${pad(t.getUTCDate(), 2)}${pad(t.getUTCMonth() + 1, 2)}${pad(t.getUTCFullYear() % 100, 2)}`;

// ---------- AIS ----------
class Bits {
  constructor() { this.b = []; }
  u(v, n) { v = Math.round(v); for (let i = n - 1; i >= 0; i--) this.b.push(Math.floor(v / 2 ** i) % 2); return this; }
  s(v, n) { v = Math.round(v); return this.u(v < 0 ? v + 2 ** n : v, n); }
  t(str, chars) {
    str = String(str).toUpperCase().padEnd(chars, "@").slice(0, chars);
    for (const ch of str) { const c = ch.charCodeAt(0); this.u(c >= 64 ? c - 64 : c, 6); }
    return this;
  }
  /** The armoured payload and the number of fill bits. */
  payload() {
    const fill = (6 - (this.b.length % 6)) % 6;
    const b = [...this.b, ...Array(fill).fill(0)];
    let out = "";
    for (let i = 0; i < b.length; i += 6) { let v = 0; for (let j = 0; j < 6; j++) v = v * 2 + b[i + j]; out += String.fromCharCode(v < 40 ? v + 48 : v + 56); }
    return { out, fill };
  }
}
let seq = 0;
/** One AIS message as one or more !AIVDM sentences. */
export function vdm(bits, channel = "A") {
  const { out, fill } = bits.payload();
  const parts = [];
  for (let i = 0; i < out.length; i += 60) parts.push(out.slice(i, i + 60));
  if (parts.length === 1) return [aivdm(`AIVDM,1,1,,${channel},${out},${fill}`)];
  seq = (seq % 9) + 1;
  return parts.map((p, i) => aivdm(`AIVDM,${parts.length},${i + 1},${seq},${channel},${p},${i === parts.length - 1 ? fill : 0}`));
}
const aisPos = (b, lon, lat) => b.s(lon * 600000, 28).s(lat * 600000, 27);
const hdgOrNone = (h) => (h == null ? 511 : Math.round(norm360(h)) % 360);
/** Class A position report (message 1). */
function msg1(t, sec) {
  const b = new Bits().u(1, 6).u(0, 2).u(t.mmsi, 30).u(t.status ?? 0, 4).s(t.sog < 0.2 ? 0 : -128, 8).u(Math.min(1022, t.sog * 10), 10).u(1, 1);
  aisPos(b, t.pos[0], t.pos[1]).u(norm360(t.cog) * 10, 12).u(hdgOrNone(t.hdg), 9).u(sec % 60, 6).u(0, 2).u(0, 3).u(0, 1).u(0, 19);
  return vdm(b);
}
/** Class A static and voyage data (message 5). */
function msg5(t) {
  const [bow, stern, port, stbd] = t.dims;
  const b = new Bits().u(5, 6).u(0, 2).u(t.mmsi, 30).u(0, 2).u(t.imo ?? 0, 30).t(t.callsign ?? "", 7).t(t.name, 20).u(t.type, 8)
    .u(bow, 9).u(stern, 9).u(port, 6).u(stbd, 6).u(1, 4).u(0, 4).u(0, 5).u(24, 5).u(60, 6).u((t.draught ?? 0) * 10, 8).t(t.dest ?? "", 20).u(0, 1).u(0, 1);
  return vdm(b);
}
/** Class B position report (message 18). */
function msg18(t, sec) {
  const b = new Bits().u(18, 6).u(0, 2).u(t.mmsi, 30).u(0, 8).u(Math.min(1022, t.sog * 10), 10).u(1, 1);
  aisPos(b, t.pos[0], t.pos[1]).u(norm360(t.cog) * 10, 12).u(hdgOrNone(t.hdg), 9).u(sec % 60, 6).u(0, 2).u(1, 1).u(0, 1).u(1, 1).u(1, 1).u(1, 1).u(0, 1).u(0, 1).u(0, 1).u(0, 19).u(0, 1);
  return vdm(b, "B");
}
/** Class B static data (message 24, parts A and B). */
function msg24(t) {
  const [bow, stern, port, stbd] = t.dims;
  const a = new Bits().u(24, 6).u(0, 2).u(t.mmsi, 30).u(0, 2).t(t.name, 20);
  const b = new Bits().u(24, 6).u(0, 2).u(t.mmsi, 30).u(1, 2).u(t.type, 8).t("DMO", 3).u(0, 4).u(0, 20).t(t.callsign ?? "", 7).u(bow, 9).u(stern, 9).u(port, 6).u(stbd, 6).u(0, 6);
  return [...vdm(a, "B"), ...vdm(b, "B")];
}

// Invented targets. Each runs to and fro along its line (lon, lat) at its speed; kn 0 = stopped.
// MMSIs 512 000 9xx are not allocated to anyone known to us; the names are made up.
const TARGETS = [
  { mmsi: 512000901, name: "ARAPAWA DEMO", callsign: "ZMD901", cls: "A", type: 60, status: 0, kn: 17.5, dims: [150, 30, 12, 12], draught: 5.5, dest: "PICTON", imo: 9000011,
    line: [[174.835, -41.37], [174.78, -41.405], [174.66, -41.395], [174.55, -41.31], [174.345, -41.215]] },
  { mmsi: 512000902, name: "TASMAN TRADER DEMO", callsign: "ZMD902", cls: "A", type: 70, status: 0, kn: 12.5, dims: [100, 28, 10, 10], draught: 7.2, dest: "LYTTELTON", imo: 9000023,
    line: [[174.30, -40.60], [174.52, -41.20], [174.62, -41.55]] },
  { mmsi: 512000903, name: "KAITIAKI DEMO", callsign: "ZMD903", cls: "A", type: 30, status: 7, kn: 3.2, dims: [14, 10, 4, 4], draught: 3.8, dest: "NELSON", imo: 0,
    line: [[173.30, -41.10], [173.48, -41.02]] },
  { mmsi: 512000904, name: "WINDSONG DEMO", callsign: "ZMD904", cls: "B", type: 36, kn: 5.6, dims: [6, 6, 2, 2],
    line: [[173.94, -40.885], [174.06, -40.845]] },
  { mmsi: 512000905, name: "OCEAN HARVEST DEMO", callsign: "ZMD905", cls: "A", type: 70, status: 1, kn: 0, dims: [120, 40, 14, 14], draught: 8.0, dest: "NELSON", imo: 9000035, hdg: 320,
    line: [[173.20, -41.205]] },
  { mmsi: 512000906, name: "TERAWHITI DEMO", callsign: "ZMD906", cls: "B", type: 37, kn: 21, dims: [5, 5, 2, 2],
    line: [[174.83, -41.36], [174.70, -41.40], [174.60, -41.33]] },
];
/** Where a target is, t seconds in: to and fro along its line. */
function targetAt(tg, t) {
  const pts = tg.line;
  if (pts.length === 1 || !tg.kn) return { pos: pts[0], cog: tg.hdg ?? 0, sog: 0, hdg: tg.hdg ?? null };
  const legs = pts.slice(1).map((p, i) => leg(pts[i], p));
  const total = legs.reduce((a, l) => a + l.d, 0);
  let s = (tg.kn * KN * t + (tg.mmsi % 7) * 0.9) % (2 * total);
  const back = s > total;
  if (back) s = 2 * total - s;
  for (let i = 0; i < legs.length; i++) {
    if (s <= legs[i].d || i === legs.length - 1) {
      const f = Math.min(1, s / legs[i].d);
      const pos = [pts[i][0] + (pts[i + 1][0] - pts[i][0]) * f, pts[i][1] + (pts[i + 1][1] - pts[i][1]) * f];
      const cog = norm360(legs[i].brg + (back ? 180 : 0));
      return { pos, cog, sog: tg.kn, hdg: cog };
    }
    s -= legs[i].d;
  }
}

// ---------- the route ----------
const DEFAULTS = { depth: 40, twd: 320, tws: 14, set: 0, drift: 0 };
function readRoute(file) {
  const text = fs.readFileSync(file, "utf8");
  let wps;
  if (/\.gpx$/i.test(file)) {
    wps = [...text.matchAll(/<rtept\s+([^>]*)>([\s\S]*?)<\/rtept>/g)].map((m) => ({
      lat: Number(/lat="([^"]+)"/.exec(m[1])?.[1]), lon: Number(/lon="([^"]+)"/.exec(m[1])?.[1]), name: /<name>([^<]*)<\/name>/.exec(m[2])?.[1] ?? "" }));
  } else wps = JSON.parse(text);
  wps = wps.filter((w) => Number.isFinite(w.lat) && Number.isFinite(w.lon)).map((w) => ({ ...DEFAULTS, ...w }));
  if (wps.length < 2) throw new Error(`${file}: a route needs at least two waypoints`);
  return wps;
}

// ---------- make ----------
function make(args) {
  const routeFile = args._[0] ?? path.join(HERE, "nelson-wellington.json");
  const wps = readRoute(routeFile);
  const out = args.out ?? path.join(process.cwd(), `${path.basename(routeFile).replace(/\.[^.]+$/, "")}.nmea`);
  const cruise = Number(args.speed ?? 7.5);
  const withAis = !args["no-ais"];
  const rand = rng(20261111);
  const start = new Date(args.start ?? "2026-11-14T18:00:00Z"); // 07:00 NZDT: a day passage
  const VAR = 22; // magnetic variation, °E, round Cook Strait

  // where along the route each waypoint is, to interpolate depth, wind and stream between them
  const legs = wps.slice(1).map((w, i) => leg([wps[i].lon, wps[i].lat], [w.lon, w.lat]));
  const along = [0];
  for (const l of legs) along.push(along.at(-1) + l.d);
  const total = along.at(-1);
  const at = (s, key) => {
    let i = 0;
    while (i < legs.length - 1 && s > along[i + 1]) i++;
    const f = Math.max(0, Math.min(1, (s - along[i]) / (legs[i].d || 1)));
    const a = wps[i][key], b = wps[i + 1][key];
    if (key === "twd" || key === "set") return norm360(a + norm180(b - a) * f);
    return a + (b - a) * f;
  };
  // slow wanderings: a few sines with random phases
  const wander = (periods) => { const ph = periods.map(() => rand() * 2 * Math.PI); return (t) => periods.reduce((a, p, i) => a + Math.sin((2 * Math.PI * t) / p + ph[i]), 0) / periods.length; };
  const wSpeed = wander([47, 211, 900]), wHead = wander([9, 23, 61]), wDepth = wander([40, 170, 610, 1900]), wGust = wander([35, 140, 620]), wDir = wander([90, 400, 1500]), wTemp = wander([1800, 5400]);

  let pos = [wps[0].lon, wps[0].lat];
  let i = 0; // the leg being sailed
  let hdg = legs[0].brg;
  let done = 0; // distance run along finished legs
  const reached = [0]; // seconds in at each waypoint
  const fd = fs.openSync(out, "w");
  let buf = [], lines = 0, t = 0;
  const flush = () => { if (buf.length) { fs.writeSync(fd, buf.join("\r\n") + "\r\n"); lines += buf.length; buf = []; } };

  for (; t < 48 * 3600; t++) {
    const a = [wps[i].lon, wps[i].lat], b = [wps[i + 1].lon, wps[i + 1].lat];
    const toB = leg(pos, b);
    // along and across the leg
    const fromA = leg(a, pos);
    const rel = (fromA.brg - legs[i].brg) * R;
    const alongLeg = fromA.d * Math.cos(rel), xte = fromA.d * Math.sin(rel); // + = right of track
    const s = done + Math.max(0, Math.min(legs[i].d, alongLeg));
    // arrived: past the waypoint's beam, or close to it
    if (alongLeg >= legs[i].d - 0.02 || toB.d < 0.03) {
      done += legs[i].d;
      i++;
      reached[i] = t;
      if (i >= legs.length) break;
      continue;
    }
    // the stream, and the speed through the water (slower in the narrows, a little uneven)
    const set = at(s, "set"), drift = at(s, "drift");
    const ce = drift * Math.sin(set * R), cn = drift * Math.cos(set * R);
    const stw = Math.max(2, cruise * (1 + 0.035 * wSpeed(t)));
    // steer to make good the track, closing a cross-track error over about a quarter of a mile
    const want = norm360(legs[i].brg - Math.atan2(xte, 0.25) / R * 0.6);
    const across = -ce * Math.cos(want * R) + cn * Math.sin(want * R); // stream to the left of the wanted course
    const crab = Math.asin(Math.max(-0.6, Math.min(0.6, across / stw))) / R;
    const steer = norm360(want + crab);
    const turn = Math.max(-2, Math.min(2, norm180(steer - hdg))); // 2° a second at most
    hdg = norm360(hdg + turn);
    const heading = norm360(hdg + 1.5 * wHead(t)); // yaw
    const ve = stw * Math.sin(hdg * R) + ce, vn = stw * Math.cos(hdg * R) + cn;
    const sog = Math.hypot(ve, vn), cog = norm360(Math.atan2(ve, vn) / R);
    pos = move(pos, ve * KN, vn * KN);

    // the wind: true from the route, apparent from the boat's motion over the ground
    const tws = Math.max(1, at(s, "tws") * (1 + 0.18 * wGust(t)));
    const twd = norm360(at(s, "twd") + 9 * wDir(t));
    const we = -tws * Math.sin(twd * R) - ve, wn = -tws * Math.cos(twd * R) - vn; // air over the deck
    const aws = Math.hypot(we, wn);
    const awa = norm360(Math.atan2(-we, -wn) / R - heading);
    const twa = norm360(twd - heading);
    const depth = Math.max(4, at(s, "depth") * (1 + 0.09 * wDepth(t)));
    const TD = 1.5; // the transducer below the waterline
    const frac = s / total;
    const water = 16.4 - 1.8 * frac + 0.25 * wTemp(t);
    const baro = 1016.2 - 5.5 * frac - (t / 3600) * 0.05;
    const air = 15.5 + 3.5 * Math.sin(Math.PI * Math.min(1, t / (12 * 3600))) + 0.3 * wTemp(t + 900);
    const hum = 72 + 8 * frac;

    const now = new Date(start.getTime() + t * 1000);
    buf.push(
      sentence(`GPRMC,${hms(now)},A,${latF(pos[1])},${lonF(pos[0])},${sog.toFixed(1)},${cog.toFixed(1)},${dmy(now)},${VAR.toFixed(1)},E,D`),
      sentence(`GPGGA,${hms(now)},${latF(pos[1])},${lonF(pos[0])},2,${10 + Math.round(1.5 * wHead(t / 30))},0.9,3.0,M,12.0,M,4.0,0120`),
      sentence(`GPVTG,${cog.toFixed(1)},T,${norm360(cog - VAR).toFixed(1)},M,${sog.toFixed(1)},N,${(sog * 1.852).toFixed(1)},K,D`),
      sentence(`HEHDT,${heading.toFixed(1)},T`),
      sentence(`VWVHW,${heading.toFixed(1)},T,${norm360(heading - VAR).toFixed(1)},M,${stw.toFixed(1)},N,${(stw * 1.852).toFixed(1)},K`),
      sentence(`SDDBT,${((depth - TD) * 3.28084).toFixed(1)},f,${(depth - TD).toFixed(1)},M,${((depth - TD) * 0.546807).toFixed(1)},F`),
      sentence(`SDDPT,${(depth - TD).toFixed(1)},${TD.toFixed(1)}`),
      sentence(`WIMWV,${awa.toFixed(0)},R,${aws.toFixed(1)},N,A`),
      sentence(`WIMWV,${twa.toFixed(0)},T,${tws.toFixed(1)},N,A`),
      sentence(`WIMWD,${twd.toFixed(0)},T,${norm360(twd - VAR).toFixed(0)},M,${tws.toFixed(1)},N,${(tws * 0.514444).toFixed(1)},M`),
    );
    if (t % 5 === 0) {
      buf.push(sentence(`YXMTW,${water.toFixed(1)},C`),
        sentence(`WIMDA,${(baro / 33.8639).toFixed(2)},I,${(baro / 1000).toFixed(4)},B,${air.toFixed(1)},C,${water.toFixed(1)},C,${hum.toFixed(0)},,,C,${twd.toFixed(0)},T,${norm360(twd - VAR).toFixed(0)},M,${tws.toFixed(1)},N,${(tws * 0.514444).toFixed(1)},M`));
    }
    if (withAis) {
      TARGETS.forEach((tg, n) => {
        const st = { ...tg, ...targetAt(tg, t) };
        if (tg.cls === "A") {
          const every = tg.kn ? 6 : 180; // under way every few seconds, at anchor every three minutes
          if ((t + n * 2) % every === 0) buf.push(...msg1(st, t));
          if ((t + n * 11) % 360 === 20) buf.push(...msg5(st));
        } else {
          if ((t + n * 3) % 30 === 0) buf.push(...msg18(st, t));
          if ((t + n * 13) % 360 === 25) buf.push(...msg24(st));
        }
      });
    }
    if (buf.length > 4000) flush();
  }
  flush();
  fs.closeSync(fd);
  const h = Math.floor(t / 3600), m = Math.round((t % 3600) / 60);
  console.log(`${out}\n${wps[0].name} to ${wps.at(-1).name}: ${total.toFixed(1)} NM in ${h} h ${pad(m, 2)} min at about ${cruise} kn through the water; ${lines} sentences, ${(fs.statSync(out).size / 1e6).toFixed(1)} MB.`);
  let s = 0;
  console.log("Waypoint                        NM run   time in (for play --from)");
  wps.forEach((w, k) => { if (k) s += legs[k - 1].d; const tt = reached[k] ?? t; console.log(`${w.name.padEnd(30)} ${s.toFixed(1).padStart(6)}   ${pad(Math.floor(tt / 3600), 2)}:${pad(Math.floor((tt % 3600) / 60), 2)}`); });
}

// ---------- distress ----------
// A short scenario to test distress alerting: own boat motoring slowly in Tasman Bay while AIS-SART,
// MOB and EPIRB-AIS transmitters come up (active and test) and the VHF passes on DSC distress alerts,
// a relay, an acknowledgement and urgency and safety calls. One event a minute or so, each placed
// round the boat so that they don't land on top of each other. All MMSIs are invented.
//   at (s)  what                                     where from the boat
const DISTRESS = [
  { at: 30, ais: { mmsi: 970099101, status: 14, text: "SART ACTIVE", drift: [0.6, 100] }, off: [1.2, 40], what: "AIS-SART, active" },
  { at: 90, ais: { mmsi: 972099102, status: 14, text: "MOB ACTIVE", drift: [0.8, 120] }, off: [0.35, 200], what: "AIS MOB device, active (astern)" },
  { at: 150, ais: { mmsi: 974099103, status: 14, text: "EPIRB ACTIVE", drift: [0.4, 80] }, off: [2.5, 320], what: "EPIRB-AIS, active" },
  { at: 210, ais: { mmsi: 970099104, status: 15, text: "SART TEST", drift: [0, 0], until: 300 }, off: [1.5, 140], what: "AIS-SART, test (stops after 90 s)" },
  { at: 270, dsc: "alert", mmsi: 512009971, nature: "01", dse: true, off: [3.0, 10], what: "DSC distress alert, flooding, with DSE position" },
  { at: 330, dsc: "relay", mmsi: 512009972, from: "005120111", nature: "05", off: [4.0, 260], what: "DSC distress relay (coast station), sinking" },
  { at: 390, dsc: "ack", mmsi: 512009971, from: "005120111", nature: "01", off: [3.0, 10], ref: 270, what: "DSC distress acknowledgement of the 04:30 alert" },
  { at: 450, dsc: "urgency", mmsi: 512009973, what: "DSC all-ships urgency call" },
  { at: 510, dsc: "safety", mmsi: "005120111", what: "DSC all-ships safety call (coast station)" },
  { at: 570, dsc: "alert", mmsi: 512009974, nature: "10", nopos: true, what: "DSC distress alert, man overboard, no position" },
  { at: 630, ais: { mmsi: 972099105, status: 15, text: "MOB TEST", drift: [0, 0], until: 700 }, off: [0.8, 290], what: "AIS MOB device, test (stops after 70 s)" },
  { at: 690, dsc: "alert", mmsi: 512009971, nature: "01", dse: true, off: [3.0, 10], ref: 270, what: "DSC distress alert repeated (the 04:30 ship)" },
];
/** DSC position field (quadrant, ddmm, dddmm) and the DSE's extra minutes digits. */
function dscPos([lon, lat]) {
  const q = lat < 0 ? (lon < 0 ? 3 : 2) : lon < 0 ? 1 : 0;
  const part = (v) => { const a = Math.abs(v), d = Math.floor(a), m = (a - d) * 60, mi = Math.floor(m); return [d, mi, Math.floor((m - mi) * 10000)]; };
  const [la, lam, laf] = part(lat), [lo, lom, lof] = part(lon);
  return [`${q}${pad(la, 2)}${pad(lam, 2)}${pad(lo, 3)}${pad(lom, 2)}`, `${pad(laf, 4)}${pad(lof, 4)}`];
}
const addr = (m) => `${pad(m, 9)}0`;
/** Safety-related broadcast (AIS message 14). */
function msg14(mmsi, text) { return vdm(new Bits().u(14, 6).u(0, 2).u(mmsi, 30).u(0, 2).t(text, text.length)); }

function distress(args) {
  const out = args.out ?? path.join(process.cwd(), "distress-test.nmea");
  const start = new Date(args.start ?? "2026-11-14T18:00:00Z");
  const len = 780; // 13 minutes
  const VAR = 22, SOG = 5, COG = 45;
  let pos = [173.24, -41.16]; // Tasman Bay, north of Nelson
  const track = [];
  for (let t = 0; t <= len; t++) { track.push(pos); pos = move(pos, SOG * KN * Math.sin(COG * R), SOG * KN * Math.cos(COG * R)); }
  const place = (ev) => { const [d, b] = ev.off; return move(track[ev.ref ?? ev.at], d * Math.sin(b * R), d * Math.cos(b * R)); };
  const lines = [];
  for (let t = 0; t < len; t++) {
    const now = new Date(start.getTime() + t * 1000), p = track[t];
    lines.push(
      sentence(`GPRMC,${hms(now)},A,${latF(p[1])},${lonF(p[0])},${SOG.toFixed(1)},${COG.toFixed(1)},${dmy(now)},${VAR.toFixed(1)},E,D`),
      sentence(`GPGGA,${hms(now)},${latF(p[1])},${lonF(p[0])},2,10,0.9,3.0,M,12.0,M,4.0,0120`),
      sentence(`GPVTG,${COG.toFixed(1)},T,${norm360(COG - VAR).toFixed(1)},M,${SOG.toFixed(1)},N,${(SOG * 1.852).toFixed(1)},K,D`),
      sentence(`HEHDT,${COG.toFixed(1)},T`),
    );
    for (const ev of DISTRESS) {
      if (ev.ais) {
        const a = ev.ais, end = a.until ?? len;
        if (t < ev.at || t >= end) continue;
        const since = t - ev.at;
        // an AIS-SART sends a burst of position reports once a minute (here every 20 s, to see it move sooner),
        // and "… ACTIVE" / "… TEST" every four minutes
        if (since % 20 === 0) {
          const [kn, set] = a.drift;
          const d = kn * KN * since;
          const at = move(place(ev), d * Math.sin(set * R), d * Math.cos(set * R));
          lines.push(...msg1({ mmsi: a.mmsi, status: a.status, sog: kn, cog: set, pos: at, hdg: null }, t));
        }
        if (since % 240 === 1) lines.push(...msg14(a.mmsi, a.text));
        continue;
      }
      if (t !== ev.at) continue;
      const hhmm = `${pad(now.getUTCHours(), 2)}${pad(now.getUTCMinutes(), 2)}`;
      const [pf, extra] = ev.off && !ev.nopos ? dscPos(place(ev)) : ["9999999999", null];
      const time = ev.nopos ? "8888" : hhmm;
      let f;
      if (ev.dsc === "alert") f = `12,${addr(ev.mmsi)},,${ev.nature},00,${pf},${time},,,S,${ev.dse ? "E" : ""}`;
      else if (ev.dsc === "relay") f = `16,${addr(ev.from)},12,12,00,${pf},${time},${addr(ev.mmsi)},${ev.nature},S,`;
      else if (ev.dsc === "ack") f = `16,${addr(ev.from)},12,20,00,${pf},${time},${addr(ev.mmsi)},${ev.nature},B,`;
      else if (ev.dsc === "urgency") f = `16,${addr(ev.mmsi)},10,00,09,,,,,S,`;
      else f = `16,${addr(ev.mmsi)},08,00,09,,,,,S,`;
      lines.push(sentence(`CDDSC,${f}`));
      if (ev.dse && extra) lines.push(sentence(`CDDSE,1,1,A,${addr(ev.mmsi)},00,${extra}`));
    }
  }
  fs.writeFileSync(out, lines.join("\r\n") + "\r\n");
  console.log(`${out}\n${len / 60} min, ${lines.length} sentences. Own boat at ${SOG} kn ${COG}°T from 41°09.6'S 173°14.4'E.\n\n time   event`);
  for (const ev of DISTRESS) console.log(`${pad(Math.floor(ev.at / 60), 2)}:${pad(ev.at % 60, 2)}   ${ev.what}${ev.ais ? ` (${ev.ais.mmsi})` : ` (${ev.mmsi})`}`);
}

// ---------- equipment ----------
// A short scenario to test what TweedPlot does with other equipment: radar (ARPA) targets from
// TTM and TLL, one on a collision course and one passing clear, and alarms from a sounder and the
// radar as ALR and ALF (one that can be acknowledged from TweedPlot, one that can't). With
// `listen` in another window, what TweedPlot sends back (autopilot sentences, ACK and ACN) shows.
//   at (s)  what
const EQUIPMENT = [
  { at: 20, what: "Radar targets R1 (collision course, CPA under 0.05 NM, inside 12 min from about 03:00) and R2 (passing clear) acquired" },
  { at: 60, what: "Sounder ALR 012 SHALLOW WATER, active, until 05:00" },
  { at: 120, what: "Sounder ALF warning 3015 (category B: can be acknowledged from TweedPlot), until 07:00" },
  { at: 180, what: "Radar ALF alarm 3100 (category A: only at the radar), until 06:00" },
];
function equipment(args) {
  const out = args.out ?? path.join(process.cwd(), "equipment-test.nmea");
  const start = new Date(args.start ?? "2026-11-14T18:00:00Z");
  const len = 600;
  const VAR = 22, SOG = 5, COG = 45;
  const own0 = [173.24, -41.16];
  const ownAt = (t) => move(own0, SOG * KN * t * Math.sin(COG * R), SOG * KN * t * Math.cos(COG * R));
  // R1 meets own ship near t = 900 s (15 min) from the starboard bow; R2 passes 1.5 NM off
  const meet = ownAt(900);
  const r1 = { n: 1, name: "SHIP1", sog: 12, cog: 300, at0: move(move(meet, 0.05, 0), -12 * KN * 900 * Math.sin(300 * R), -12 * KN * 900 * Math.cos(300 * R)) };
  const r2 = { n: 2, name: "SHIP2", sog: 9, cog: 200, at0: move(ownAt(0), 1.5, 3.5) };
  const tgtAt = (g, t) => move(g.at0, g.sog * KN * t * Math.sin(g.cog * R), g.sog * KN * t * Math.cos(g.cog * R));
  const lines = [];
  for (let t = 0; t < len; t++) {
    const now = new Date(start.getTime() + t * 1000), p = ownAt(t);
    const time = `${hms(now)}`;
    lines.push(
      sentence(`GPRMC,${time},A,${latF(p[1])},${lonF(p[0])},${SOG.toFixed(1)},${COG.toFixed(1)},${dmy(now)},${VAR.toFixed(1)},E,D`),
      sentence(`GPGGA,${time},${latF(p[1])},${lonF(p[0])},2,10,0.9,3.0,M,12.0,M,4.0,0120`),
      sentence(`HEHDT,${COG.toFixed(1)},T`),
    );
    // radar targets every 3 s, with the radar's own CPA and TCPA
    if (t >= 20 && t % 3 === 2) {
      const vo = [SOG * Math.sin(COG * R), SOG * Math.cos(COG * R)];
      for (const g of [r1, r2]) {
        const q = tgtAt(g, t), { d, brg } = leg(p, q);
        const rel = [q[0] - p[0], q[1] - p[1]];
        const k = Math.cos(p[1] * R);
        const pr = [rel[0] * 60 * k, rel[1] * 60];
        const vt = [g.sog * Math.sin(g.cog * R), g.sog * Math.cos(g.cog * R)];
        const vr = [vt[0] - vo[0], vt[1] - vo[1]];
        const v2 = vr[0] ** 2 + vr[1] ** 2;
        const th = v2 > 1e-9 ? -(pr[0] * vr[0] + pr[1] * vr[1]) / v2 : 0; // hours
        const cpa = Math.hypot(pr[0] + vr[0] * th, pr[1] + vr[1] * th);
        const status = t < 26 ? "Q" : "T";
        lines.push(sentence(`RATTM,${pad(g.n, 2)},${d.toFixed(2)},${brg.toFixed(1)},T,${g.sog.toFixed(1)},${g.cog.toFixed(1)},T,${cpa.toFixed(2)},${(th * 60).toFixed(1)},N,${g.name},${status},,${time},A`));
        lines.push(sentence(`RATLL,${pad(g.n, 2)},${latF(q[1])},${lonF(q[0])},${g.name},${time},${status},`));
      }
    }
    // alarms, repeated every 30 s while they last (as equipment does), then once more when cleared
    const every = (from, to) => t >= from && (t - from) % 30 === 0 && t <= to;
    if (every(60, 300)) lines.push(sentence(`SDALR,${time},012,${t < 300 ? "A" : "V"},V,SHALLOW WATER`));
    if (every(120, 420)) lines.push(sentence(`SDALF,1,1,${Math.floor((t - 120) / 30) % 10},${time},B,W,${t < 420 ? "V" : "N"},SDR,3015,1,0,0,Depth below shallow limit`));
    if (every(180, 360)) lines.push(sentence(`RAALF,1,1,${Math.floor((t - 180) / 30) % 10},${time},A,A,${t < 360 ? "V" : "N"},,3100,1,0,0,Dangerous target`));
  }
  fs.writeFileSync(out, lines.join("\r\n") + "\r\n");
  console.log(`${out}\n${len / 60} min, ${lines.length} sentences. Own boat at ${SOG} kn ${COG}°T.\n\n time   event`);
  for (const ev of EQUIPMENT) console.log(`${pad(Math.floor(ev.at / 60), 2)}:${pad(ev.at % 60, 2)}   ${ev.what}`);
}

// ---------- listen ----------
// Prints what arrives on a UDP port: what TweedPlot sends on a UDP connection with a "send to"
// address (autopilot sentences, ACK, ACN).
function listen(args) {
  const port = Number(args.port ?? args.udp ?? 10111);
  const sock = dgram.createSocket({ type: "udp4", reuseAddr: true });
  sock.on("message", (m, from) => {
    for (const l of m.toString().split(/\r?\n/)) if (l.trim()) console.log(`${new Date().toLocaleTimeString("en-GB", { hour12: false })}  ${from.address}  ${l}`);
  });
  sock.bind(port, () => console.log(`Listening on UDP ${port} (Ctrl+C to stop)`));
}

// ---------- play ----------
function play(args) {
  const file = args._[0];
  if (!file) throw new Error("play needs the .nmea file");
  const rate = Math.max(0.1, Number(args.rate ?? 1));
  const all = fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean);
  // one group a second: each starts at its RMC
  const groups = [];
  for (const ln of all) { if (/^\$..RMC,/.test(ln) || !groups.length) groups.push([]); groups.at(-1).push(ln); }
  let k = 0;
  if (args.from) { const [h, m = 0] = String(args.from).split(":").map(Number); k = Math.min(groups.length - 1, h * 3600 + m * 60); }
  const restamp = (ln, now) => {
    // a DSC call's time (field 7, hhmm UTC) follows the clock too, unless it is 8888 (not known)
    const dsc = /^\$(..DSC,)([^*]*)\*/.exec(ln);
    if (dsc) {
      const f = dsc[2].split(",");
      if (/^\d{4}$/.test(f[6] ?? "") && f[6] !== "8888") f[6] = `${pad(now.getUTCHours(), 2)}${pad(now.getUTCMinutes(), 2)}`;
      return sentence(dsc[1] + f.join(","));
    }
    const m = /^\$(..(RMC|GGA),)([^*]*)\*/.exec(ln);
    if (!m) return ln;
    const f = m[3].split(",");
    f[0] = hms(now);
    if (m[2] === "RMC") f[8] = dmy(now);
    return sentence(m[1] + f.join(","));
  };
  let send, where;
  if (args.tcp) {
    const clients = new Set();
    const port = Number(args.tcp === true ? 10110 : args.tcp);
    net.createServer((sock) => { clients.add(sock); sock.on("close", () => clients.delete(sock)); sock.on("error", () => clients.delete(sock)); }).listen(port, "127.0.0.1");
    send = (text) => { for (const c of clients) c.write(text); };
    where = `TCP server on 127.0.0.1:${port} (in TweedPlot: TCP client, 127.0.0.1:${port})`;
  } else {
    const [host, port] = String(args.udp === true || !args.udp ? "127.0.0.1:10110" : args.udp).split(":");
    const sock = dgram.createSocket("udp4");
    send = (text) => sock.send(text, Number(port), host);
    where = `UDP to ${host}:${port} (in TweedPlot: UDP listener, ${host}:${port})`;
  }
  console.log(`${path.basename(file)}: ${groups.length} seconds (${(groups.length / 3600).toFixed(1)} h), from ${pad(Math.floor(k / 3600), 2)}:${pad(Math.floor((k % 3600) / 60), 2)}, ${rate}× speed\n${where}\nCtrl+C stops.`);
  const tick = () => {
    if (k >= groups.length) { if (args.loop) k = 0; else { console.log("\nEnd of the passage."); process.exit(0); } }
    const now = new Date();
    const g = args["keep-time"] ? groups[k] : groups[k].map((ln) => restamp(ln, now));
    send(g.join("\r\n") + "\r\n");
    if (k % 60 === 0) {
      const f = g[0].split(",");
      process.stdout.write(`\r${pad(Math.floor(k / 3600), 2)}:${pad(Math.floor((k % 3600) / 60), 2)}  ${f[3]}${f[4]} ${f[5]}${f[6]}  ${f[7]} kn  ${f[8]}°T   `);
    }
    k++;
  };
  setInterval(tick, 1000 / rate);
}

// ---------- command line ----------
function parse(argv) {
  const a = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const x = argv[i];
    if (x.startsWith("--")) { const k = x.slice(2); const v = argv[i + 1]; if (v === undefined || v.startsWith("--")) a[k] = true; else { a[k] = v; i++; } } else a._.push(x);
  }
  return a;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [cmd, ...rest] = process.argv.slice(2);
  try {
    if (cmd === "make") make(parse(rest));
    else if (cmd === "play") play(parse(rest));
    else if (cmd === "distress") distress(parse(rest));
    else if (cmd === "equipment") equipment(parse(rest));
    else if (cmd === "listen") listen(parse(rest));
    else console.log("node nmea-sim.mjs make [route.json | route.gpx] [--out file.nmea] [--speed 7.5] [--start 2026-11-14T18:00:00Z] [--no-ais]\nnode nmea-sim.mjs distress [--out distress-test.nmea]\nnode nmea-sim.mjs equipment [--out equipment-test.nmea]\nnode nmea-sim.mjs listen [--port 10111]\nnode nmea-sim.mjs play file.nmea [--udp 127.0.0.1:10110 | --tcp 10110] [--from hh:mm] [--rate 1] [--loop] [--keep-time]");
  } catch (e) { console.error(String(e.message || e)); process.exit(1); }
}
