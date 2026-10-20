// Tide stations on the chart (LINZ): daily ports and secondary ports, with a popup
// showing the day's tidal curve, highs and lows, and the height at the planning time.
// Data comes from the Rust side (tides crate), downloaded once from LINZ.
import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const KEY = "racenav.tides";
const BLUE = "#1e6fb5";
const DAY = 86400;

/** Wall-clock parts of Unix time `t` in a time zone. */
function parts(t, tz) {
  const f = new Intl.DateTimeFormat("en-NZ", { timeZone: tz, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric" });
  const o = {};
  for (const p of f.formatToParts(new Date(t * 1000))) o[p.type] = Number(p.value);
  return o;
}
/** Unix time of local midnight starting the day that holds `t`. */
function dayStart(t, tz) {
  t = Math.floor(t);
  const p = parts(t, tz);
  let s = t - (p.hour * 3600 + p.minute * 60 + p.second);
  // across a daylight change the day is 23 or 25 hours: check and correct once
  const q = parts(s, tz);
  if (q.hour !== 0) s -= (q.hour > 12 ? q.hour - 24 : q.hour) * 3600;
  return s;
}
const hhmm = (t, tz) => new Intl.DateTimeFormat("en-NZ", { timeZone: tz, hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date(t * 1000));
const dateLabel = (t, tz) => new Intl.DateTimeFormat("en-NZ", { timeZone: tz, weekday: "short", day: "numeric", month: "short", year: "numeric" }).format(new Date(t * 1000));
const zoneLabel = (t, tz) => (new Intl.DateTimeFormat("en-NZ", { timeZone: tz, timeZoneName: "short" }).formatToParts(new Date(t * 1000)).find((p) => p.type === "timeZoneName")?.value ?? "");

export function setupTides(map, { before, getTime, onTime }) {
  const $ = (id) => document.getElementById(id);
  const opts = { show: true };
  try { Object.assign(opts, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(opts)); } catch { /* ignore */ } };

  const empty = { type: "FeatureCollection", features: [] };
  let stations = empty;
  map.addSource("tides", { type: "geojson", data: empty });
  const icon = ["concat", "tide:", ["get", "kind"]];
  map.addLayer({ id: "tide-daily", type: "symbol", source: "tides", minzoom: 5, filter: ["==", ["get", "kind"], "daily"],
    layout: { "icon-image": icon, "icon-allow-overlap": true, "icon-ignore-placement": true } }, before);
  map.addLayer({ id: "tide-sec", type: "symbol", source: "tides", minzoom: 8, filter: ["!=", ["get", "kind"], "daily"],
    layout: { "icon-image": icon, "icon-allow-overlap": true, "icon-ignore-placement": true } }, "tide-daily");
  map.addLayer({ id: "tide-names", type: "symbol", source: "tides", minzoom: 10,
    layout: { "icon-image": ["concat", "txt:t:", ["get", "name"]], "icon-anchor": "left", "icon-offset": [11, 0], "icon-allow-overlap": false } }, before);
  const LAYERS = ["tide-daily", "tide-sec", "tide-names"];
  function apply() { for (const l of LAYERS) map.setLayoutProperty(l, "visibility", opts.show ? "visible" : "none"); }

  async function load() {
    try {
      const [fc, info] = await Promise.all([invoke("tides_stations"), invoke("tides_info")]);
      stations = fc || empty;
      map.getSource("tides").setData(stations);
      showInfo(info);
    } catch (e) { $("tide-status").textContent = `Tides: ${e}`; }
  }
  function showInfo(info) {
    $("tide-status").textContent = info?.stations
      ? `${info.stations} stations (${info.daily} with LINZ daily tables), ${info.years.join(", ") || "no years"}`
      : "No tide data yet: download the LINZ tables.";
  }

  $("tide-show").checked = opts.show;
  $("tide-show").onchange = () => { opts.show = $("tide-show").checked; save(); apply(); };
  apply();

  // ---------- download ----------------------------------------------------------
  const prog = $("tide-progress");
  listen("tides-progress", ({ payload: p }) => {
    prog.hidden = false;
    prog.querySelector(".bar").style.width = `${p.total ? Math.round((100 * p.done) / p.total) : 3}%`;
    prog.querySelector("span").textContent = p.total ? `${p.stage} · ${p.done}/${p.total}` : `${p.stage}…`;
  }).catch(() => {});
  $("tide-dl").onclick = async () => {
    const y = new Date().getFullYear();
    $("tide-dl").disabled = true; $("tide-cancel").disabled = false;
    try {
      const r = await invoke("tides_download", { years: [y, y + 1] });
      showInfo(r.info);
      await load();
    } catch (e) {
      $("tide-status").textContent = String(e) === "cancelled" ? "Download cancelled." : `Tide download failed: ${e}`;
    } finally {
      $("tide-dl").disabled = false; $("tide-cancel").disabled = true; prog.hidden = true;
    }
  };
  $("tide-cancel").onclick = () => invoke("tides_cancel").catch(() => {});

  // ---------- popup with the curve ----------------------------------------------------------
  let popup = null;
  let current = null; // { id, day (start of local day), tz }
  async function show(id, lngLat, day) {
    const t = getTime();
    const f = stations.features.find((x) => x.properties.id === id);
    const tz = f && f.geometry.coordinates[0] < -175 && f.geometry.coordinates[1] < -43 && f.geometry.coordinates[1] > -45 ? "Pacific/Chatham" : "Pacific/Auckland";
    const d0 = day ?? dayStart(t, tz);
    const d1 = dayStart(d0 + DAY + 3 * 3600, tz);
    let r;
    try { r = await invoke("tides_curve", { id, t0: Math.floor(d0), t1: Math.floor(d1) }); } catch (e) { r = { error: String(e) }; }
    current = { id, day: d0, tz, lngLat };
    const el = document.createElement("div");
    el.className = "tide-pop";
    el.innerHTML = `<h4></h4><div class="wx-note method"></div>
      <div class="nav"><button data-d="-1" title="Day before">◀</button><span class="day"></span><button data-d="1" title="Day after">▶</button><button data-d="0" title="The planning time">Now</button></div>
      <div class="svg"></div><table class="ext"></table><div class="wx-note src">LINZ tide predictions. Not official tide tables; not for navigation.</div>`;
    el.querySelector("h4").textContent = r.name ?? id;
    el.querySelector(".method").textContent = r.error ?? `${r.method}${r.approx ? " (LINZ: data approximate)" : ""}`;
    el.querySelector(".day").textContent = `${dateLabel(d0 + 3600, tz)} (${zoneLabel(d0 + 12 * 3600, tz)})`;
    for (const b of el.querySelectorAll("button")) {
      b.onclick = () => { const k = Number(b.dataset.d); show(id, lngLat, k === 0 ? undefined : dayStart(d0 + k * DAY + 3 * 3600, tz)); };
    }
    if (!r.error) {
      el.querySelector(".svg").innerHTML = curveSvg(r, d0, d1, t, tz);
      const tbl = el.querySelector(".ext");
      const ex = (r.extremes || []).filter((e) => e.t >= d0 && e.t < d1);
      for (const e of ex) {
        const tr = tbl.insertRow();
        tr.insertCell().textContent = e.high ? "High" : "Low";
        tr.insertCell().textContent = hhmm(e.t, tz);
        tr.insertCell().textContent = e.h == null ? "" : `${e.h.toFixed(1)} m`;
      }
      if (!ex.length) {
        el.querySelector(".svg").innerHTML = "";
        const y = new Date(d0 * 1000).getUTCFullYear();
        tbl.insertRow().insertCell().textContent = r.method?.startsWith("Reference port")
          ? "LINZ gives no times for this place."
          : r.years?.includes(y) ? "No tides for this day." : `No LINZ tables for ${y}: download them in the Chart panel.`;
      }
    }
    popup?.remove();
    popup = new maplibregl.Popup({ maxWidth: "360px", className: "tide-popup" }).setLngLat(lngLat).setDOMContent(el).addTo(map);
    popup.on("close", () => { if (current?.id === id) current = null; });
  }
  for (const l of ["tide-daily", "tide-sec"]) {
    map.on("click", l, (e) => {
      if (document.querySelector(".armed")) return;
      const f = e.features[0];
      show(f.properties.id, f.geometry.coordinates);
    });
    map.on("mouseenter", l, () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = "pointer"; });
    map.on("mouseleave", l, () => { if (!document.querySelector(".armed")) map.getCanvas().style.cursor = ""; });
  }
  // the planning time moved: follow it if the popup shows that day
  let pending = null;
  onTime?.(() => {
    if (!current || pending) return;
    pending = setTimeout(() => {
      pending = null;
      if (!current) return;
      const t = getTime();
      const d = dayStart(t, current.tz);
      show(current.id, current.lngLat, d);
    }, 250);
  });

  load();
  return { reload: load };
}

/** SVG of the day's curve: heights, highs and lows, mean levels, and the planning time. */
function curveSvg(r, d0, d1, now, tz) {
  const W = 320, H = 150, L = 30, R = 8, T = 12, B = 22;
  const pts = (r.curve || []).filter((p) => p[0] >= d0 && p[0] <= d1);
  const ex = (r.extremes || []).filter((e) => e.t >= d0 && e.t <= d1 && e.h != null);
  const hs = pts.map((p) => p[1]).concat(ex.map((e) => e.h));
  if (!hs.length) return "";
  let lo = Math.min(0, ...hs), hi = Math.max(...hs);
  if (r.levels?.mhws) hi = Math.max(hi, r.levels.mhws);
  hi = Math.ceil(hi + 0.2); lo = Math.floor(lo);
  const x = (t) => L + ((t - d0) / (d1 - d0)) * (W - L - R);
  const y = (h) => T + (1 - (h - lo) / (hi - lo)) * (H - T - B);
  let s = `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Tide curve">`;
  // grid: every 3 hours, every metre
  for (let k = 0; k <= 24; k += 3) {
    const t = d0 + k * 3600;
    if (t > d1) break;
    s += `<line x1="${x(t)}" x2="${x(t)}" y1="${T}" y2="${H - B}" class="g"/>`;
    s += `<text x="${x(t)}" y="${H - 6}" class="ax" text-anchor="middle">${hhmm(t, tz).slice(0, 2)}</text>`;
  }
  for (let h = lo; h <= hi; h += hi - lo > 4 ? 2 : 1) {
    s += `<line x1="${L}" x2="${W - R}" y1="${y(h)}" y2="${y(h)}" class="g"/><text x="${L - 4}" y="${y(h) + 3}" class="ax" text-anchor="end">${h}</text>`;
  }
  if (r.levels?.msl != null) s += `<line x1="${L}" x2="${W - R}" y1="${y(r.levels.msl)}" y2="${y(r.levels.msl)}" class="msl"/>`;
  if (pts.length > 1) {
    const d = pts.map((p, i) => `${i ? "L" : "M"}${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`).join("");
    s += `<path d="${d}L${x(pts.at(-1)[0]).toFixed(1)},${y(lo)}L${x(pts[0][0]).toFixed(1)},${y(lo)}Z" class="fill"/>`;
    s += `<path d="${d}" class="line"/>`;
  }
  for (const e of ex) {
    const above = e.high;
    s += `<circle cx="${x(e.t)}" cy="${y(e.h)}" r="3" class="pt"/>`;
    s += `<text x="${x(e.t)}" y="${y(e.h) + (above ? -6 : 13)}" class="lbl" text-anchor="middle">${hhmm(e.t, tz)} ${e.h.toFixed(1)}</text>`;
  }
  if (now >= d0 && now <= d1) {
    const p = pts.reduce((a, b) => (Math.abs(b[0] - now) < Math.abs(a[0] - now) ? b : a), pts[0]);
    s += `<line x1="${x(now)}" x2="${x(now)}" y1="${T}" y2="${H - B}" class="now"/>`;
    if (p && Math.abs(p[0] - now) <= 600) {
      const i = pts.indexOf(p);
      const rising = pts[Math.min(i + 1, pts.length - 1)][1] >= pts[Math.max(i - 1, 0)][1];
      s += `<circle cx="${x(now)}" cy="${y(p[1])}" r="4" class="nowpt"/>`;
      s += `<text x="${Math.min(x(now) + 6, W - 70)}" y="${T + 10}" class="nowlbl">${hhmm(now, tz)} ${p[1].toFixed(1)} m ${rising ? "↑" : "↓"}</text>`;
    }
  }
  return `${s}</svg>`;
}

/** Station marker: a "T" in a circle (filled: LINZ daily table; open: secondary port; grey: levels only). */
export function tideImage(id) {
  if (!id.startsWith("tide:")) return null;
  const kind = id.slice(5);
  const c = document.createElement("canvas");
  c.width = 34; c.height = 34;
  const g = c.getContext("2d");
  g.beginPath(); g.arc(17, 17, 13, 0, Math.PI * 2);
  const col = kind === "levels" ? "#8a97a0" : BLUE;
  g.fillStyle = kind === "daily" ? col : "#fff"; g.fill();
  g.lineWidth = 3; g.strokeStyle = kind === "daily" ? "#fff" : col; g.stroke();
  g.fillStyle = kind === "daily" ? "#fff" : col;
  g.font = `bold 18px "Segoe UI", system-ui, sans-serif`;
  g.textAlign = "center"; g.textBaseline = "middle";
  g.fillText("T", 17, 18);
  return g.getImageData(0, 0, 34, 34);
}
