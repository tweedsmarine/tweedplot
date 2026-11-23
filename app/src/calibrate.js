// Calibrating an image (Chart › Layers › Images › ⊕, or "To calibrate"): a mode that borrows
// the screen. The chart area splits in two: the picture itself on the left (its own zoom and
// pan, a second small map showing its pixels flat), the chart on the right; the drawer lists
// the control points. Nothing is laid over the chart but the image being placed.
//
// Add points: click a place on the picture, then the same place on the chart (or the other
// way round). A chart click snaps to a library mark; a position can also be typed. Each point
// shows how far the fit puts it from where it was clicked; one far out can be left out. From
// three points (two for shift, scale and turn) the image is shown on the chart through the
// fit as it is worked on. Nudge moves and turns the whole image (drag, or Shift+drag to turn)
// to line it up with the own track. Save uses it; Leave keeps the points as a draft;
// Discard drops them. Going back to Navigation (F2 or the inactivity timer) leaves, keeping
// the draft. Alarms and MOB work throughout.

import * as maplibregl from "maplibre-gl";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./ask.js";
import { parseCoord } from "./library.js";

const $ = (id) => document.getElementById(id);
const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const MODELS = [["auto", "Automatic"], ["shift", "Shift only"], ["similarity", "Shift, scale and turn"], ["affine", "Affine (3+ points)"], ["poly2", "Second order (6+ points)"]];

/** 41°16.234′S */
function fmtDM(v, hemis) {
  const h = v < 0 ? hemis[1] : hemis[0];
  const a = Math.abs(v), d = Math.floor(a), m = (a - d) * 60;
  return `${d}°${m.toFixed(3)}′${h}`;
}
const fmtM = (m) => (m == null ? "—" : m >= 1000 ? `${(m / 1000).toFixed(1)} km` : m >= 10 ? `${Math.round(m)} m` : `${m.toFixed(1)} m`);

export function setupCalibrate(map, { images, ui, tileUrl, toast }) {
  let s = null; // the session: { e, info, calib, active, report, overlay, opacity, nudging, ver, sideWasOn, layerWasVisible, fitTimer }
  let pxMap = null;
  const imgMarkers = [];
  const chartMarkers = [];

  // ---------- the picture's flat world: pixels ↔ the small map's positions ----------
  const worldPx = () => 256 * 2 ** s.info.pixelZoom;
  const pxToLngLat = (c, r) => new maplibregl.MercatorCoordinate(c / worldPx(), r / worldPx()).toLngLat();
  function lngLatToPx(ll) {
    const m = maplibregl.MercatorCoordinate.fromLngLat(ll);
    return [m.x * worldPx(), m.y * worldPx()];
  }

  // ---------- open and leave ----------
  async function open(e) {
    if (s) await leave(true);
    let info;
    try { info = await invoke("images_calib_info", { id: e.id }); } catch (err) { toast(String(err), 6000); return; }
    const start = info.draft ?? info.calibration ?? { points: [], model: "auto", nudge: { eastM: 0, northM: 0, turnDeg: 0 } };
    s = {
      e, info,
      calib: { points: (start.points ?? []).map((p) => ({ ...p })), model: start.model ?? "auto", nudge: { eastM: 0, northM: 0, turnDeg: 0, ...(start.nudge ?? {}) } },
      active: -1, report: null, overlay: true, opacity: 0.6, nudging: false, ver: 0,
      sideWasOn: document.documentElement.classList.contains("side-on"),
      dirty: !!info.draft,
    };
    // the data panel steps aside (to its line under the chart, with the alerts)
    if (s.sideWasOn) $("side-btn")?.click();
    document.documentElement.classList.add("calibrating");
    map.getContainer().classList.add("drawing");
    // the image's own layer is hidden while its draft is shown (the list puts it back on leaving)
    const own = `img-${e.id}`;
    if (map.getLayer(own)) map.setLayoutProperty(own, "visibility", "none");
    ui.open("calibrate");
    buildPane();
    buildImageMap();
    requestAnimationFrame(() => { map.resize(); pxMap?.resize(); });
    drawMarkers();
    await refit(true);
    // a georeferenced image, or a draft with a fit: show its area on the chart
    const b = s.report?.bounds ?? (info.plain ? null : e.bounds);
    if (b && b.every(Number.isFinite) && b[2] > b[0]) map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: 40, maxZoom: 16, duration: 0 });
  }

  /** Leave calibration. keep: the points stay as a draft (false: the draft is dropped). */
  async function leave(keep) {
    if (!s) return;
    const id = s.e.id;
    clearTimeout(s.fitTimer);
    if (keep && s.dirty) {
      try { await invoke("images_calib_fit", { id, calib: s.calib }); } catch { /* the draft stays as it was */ }
    }
    if (!keep) { try { await invoke("images_calib_discard", { id }); } catch { /* nothing to drop */ } }
    for (const m of [...imgMarkers, ...chartMarkers]) m.remove();
    imgMarkers.length = chartMarkers.length = 0;
    if (map.getLayer("calib-draft")) map.removeLayer("calib-draft");
    if (map.getSource("calib-draft")) map.removeSource("calib-draft");
    pxMap?.remove();
    pxMap = null;
    map.getContainer().classList.remove("drawing", "calib-nudging");
    map.dragPan.enable();
    document.documentElement.classList.remove("calibrating");
    const sideWasOn = s.sideWasOn;
    s = null;
    if (sideWasOn && !document.documentElement.classList.contains("side-on")) $("side-btn")?.click();
    if (ui.current() === "calibrate") ui.open("chart");
    await images.refresh();
    requestAnimationFrame(() => map.resize());
  }

  // ---------- the drawer ----------
  function buildPane() {
    const p = $("calib-pane");
    p.innerHTML = `
      <div class="calib-name"></div>
      <div class="wx-note calib-help"></div>
      <div class="calib-points"></div>
      <div class="wx-row"><button type="button" class="calib-add" title="A new point: then click the place on the picture and on the chart">+ Point</button><span class="grow"></span><span class="calib-rms wx-note"></span></div>
      <div class="wx-row"><label class="grow" for="calib-model">Fit</label><select id="calib-model"></select></div>
      <div class="wx-row"><label class="grow"><input type="checkbox" class="calib-ov" /> Show on the chart</label><input type="range" class="calib-op" min="20" max="100" step="5" title="How strongly the image shows over the chart" aria-label="Opacity of the image being calibrated" /></div>
      <div class="wx-row"><button type="button" class="calib-nudge" title="Move and turn the whole image to line it up with your track or the chart: drag the chart, Shift+drag to turn, or use the arrows">Nudge</button><span class="grow calib-nudge-sum wx-note"></span><button type="button" class="link-btn calib-nudge-zero" title="Take the nudge off">Reset</button></div>
      <div class="calib-nudge-box" hidden>
        <div class="wx-row"><span class="grow">Move</span><select class="calib-step" title="How far one press moves it"><option value="1">1 m</option><option value="5" selected>5 m</option><option value="20">20 m</option><option value="100">100 m</option></select>
          <button type="button" data-n="w" title="West">◀</button><button type="button" data-n="n" title="North">▲</button><button type="button" data-n="s" title="South">▼</button><button type="button" data-n="e" title="East">▶</button></div>
        <div class="wx-row"><span class="grow">Turn</span><select class="calib-tstep" title="How far one press turns it"><option value="0.1">0.1°</option><option value="0.5" selected>0.5°</option><option value="2">2°</option></select>
          <button type="button" data-t="-1" title="Anticlockwise">⟲</button><button type="button" data-t="1" title="Clockwise">⟳</button></div>
      </div>
      <div class="wx-row calib-actions"><button type="button" class="on calib-save" title="Use this calibration">Save</button><button type="button" class="calib-leave" title="Back to the chart, keeping the points to carry on later">Leave</button><button type="button" class="calib-discard" title="Drop the changes made since the last save">Discard…</button></div>
      <div class="wx-row"><button type="button" class="link-btn calib-clear" title="Take the saved calibration off: back to the image's own position (or to To calibrate)">Remove the calibration…</button></div>
      <div class="wx-note">Pick points spread over the image, on things that don't move: rocks, points of land, jetty ends, buildings. Images and their calibrations are not surveyed: check against your depth and eyes.</div>`;
    p.querySelector(".calib-name").textContent = s.e.name;
    const sel = p.querySelector("#calib-model");
    for (const [v, t] of MODELS) sel.append(new Option(t, v));
    sel.value = s.calib.model;
    sel.onchange = () => { s.calib.model = sel.value; changed(); };
    p.querySelector(".calib-add").onclick = () => { s.calib.points.push({ px: null, ll: null, off: false, from: "" }); s.active = s.calib.points.length - 1; drawAll(); };
    const ov = p.querySelector(".calib-ov"), op = p.querySelector(".calib-op");
    ov.checked = s.overlay;
    op.value = String(Math.round(s.opacity * 100));
    ov.onchange = () => { s.overlay = ov.checked; overlay(); };
    op.oninput = () => { s.opacity = Number(op.value) / 100; if (map.getLayer("calib-draft")) map.setPaintProperty("calib-draft", "raster-opacity", s.opacity); };
    p.querySelector(".calib-nudge").onclick = () => setNudging(!s.nudging);
    p.querySelector(".calib-nudge-zero").onclick = () => { s.calib.nudge = { eastM: 0, northM: 0, turnDeg: 0 }; changed(); };
    for (const b of p.querySelectorAll("[data-n]")) {
      b.onclick = () => {
        const d = Number(p.querySelector(".calib-step").value);
        const n = s.calib.nudge;
        ({ n: () => (n.northM += d), s: () => (n.northM -= d), e: () => (n.eastM += d), w: () => (n.eastM -= d) })[b.dataset.n]();
        changed();
      };
    }
    for (const b of p.querySelectorAll("[data-t]")) {
      b.onclick = () => { s.calib.nudge.turnDeg = +(s.calib.nudge.turnDeg + Number(b.dataset.t) * Number(p.querySelector(".calib-tstep").value)).toFixed(3); changed(); };
    }
    p.querySelector(".calib-save").onclick = save;
    p.querySelector(".calib-leave").onclick = () => leave(true);
    p.querySelector(".calib-discard").onclick = async () => {
      if (s.dirty) {
        const ok = await ask({ title: "Discard changes", note: "Drop the points and nudge changed since the calibration was last saved?", ok: "Discard" });
        if (!ok) return;
      }
      await leave(false);
    };
    p.querySelector(".calib-clear").onclick = async () => {
      const ok = await ask({ title: "Remove the calibration", note: s.info.plain ? "Take the calibration off? The picture goes back under To calibrate, its points dropped." : "Take the calibration off? The image goes back to where its own georeferencing puts it.", ok: "Remove" });
      if (!ok) return;
      try { await invoke("images_calib_clear", { id: s.e.id }); } catch (err) { toast(String(err), 6000); return; }
      await leave(false);
    };
  }

  function drawPane() {
    const p = $("calib-pane");
    if (!s || !p.querySelector(".calib-points")) return;
    const r = s.report;
    const pts = s.calib.points;
    const box = p.querySelector(".calib-points");
    box.innerHTML = pts.length ? `<table class="calib-table"><thead><tr><th>#</th><th title="Clicked on the picture">Picture</th><th title="Its position">Chart</th><th title="How far the fit puts it from that position">Error</th><th title="Used in the fit">Use</th><th></th></tr></thead><tbody></tbody></table>` : "";
    const tb = box.querySelector("tbody");
    const rms = r?.rmsM ?? null;
    pts.forEach((pt, i) => {
      const tr = document.createElement("tr");
      tr.className = (i === s.active ? "active " : "") + (pt.off ? "off" : "");
      const res = r?.residuals?.[i];
      const far = res != null && rms != null && pts.filter((x) => !x.off && x.px && x.ll).length >= 4 && res > Math.max(3 * rms, 5);
      tr.innerHTML = `<td>${i + 1}</td><td>${pt.px ? "✓" : "<span class=\"need\">click</span>"}</td>
        <td class="ll" title="${pt.ll ? esc(`${fmtDM(pt.ll[1], "NS")} ${fmtDM(pt.ll[0], "EW")}${pt.from ? ` (${pt.from})` : ""}`) : ""}">${pt.ll ? esc(pt.from && pt.from !== "chart" ? pt.from.replace(/^mark: /, "") : "✓") : "<span class=\"need\">click</span>"}</td>
        <td class="${far ? "far" : ""}" title="${far ? "Far from where the others put it: check it, or leave it out" : ""}">${pt.px && pt.ll ? fmtM(res) : ""}</td>
        <td><input type="checkbox" ${pt.off ? "" : "checked"} aria-label="Use point ${i + 1}" /></td>
        <td><button type="button" class="ly-gear" data-a="type" title="Type its position">✎</button><button type="button" class="ly-gear" data-a="del" title="Delete the point">✕</button></td>`;
      tr.onclick = (ev) => { if (ev.target.closest("button, input")) return; s.active = i; drawAll(); flyTo(i); };
      tr.querySelector("input").onchange = (ev) => { pt.off = !ev.target.checked; changed(); };
      tr.querySelector('[data-a="del"]').onclick = () => { pts.splice(i, 1); if (s.active >= pts.length) s.active = pts.length - 1; changed(); };
      tr.querySelector('[data-a="type"]').onclick = () => typePosition(i);
      tb.append(tr);
    });
    // what to do next
    const help = p.querySelector(".calib-help");
    const cur = pts[s.active];
    help.classList.remove("warn");
    if (s.nudging) help.textContent = "Nudging: drag the chart to move the image, Shift+drag to turn it, or use the arrows. Nudge again to go back to points.";
    else if (cur && !cur.px && !cur.ll) help.textContent = `Point ${s.active + 1}: click the place on the picture and the same place on the chart.`;
    else if (cur && !cur.px) help.textContent = `Point ${s.active + 1}: now click the same place on the picture.`;
    else if (cur && !cur.ll) help.textContent = `Point ${s.active + 1}: now click the same place on the chart (a mark snaps), or ✎ to type its position.`;
    else if (r?.note) { help.textContent = r.note; help.classList.add("warn"); }
    else help.textContent = "Click a place on the picture, then the same place on the chart, for another point. Drag a point to move it.";
    const rmsEl = p.querySelector(".calib-rms");
    rmsEl.textContent = r?.rmsM != null ? `${MODELS.find((m) => m[0] === r.model)?.[1] ?? r.model}: ±${fmtM(r.rmsM)}` : r?.model === "nudge" ? "Nudged" : "";
    rmsEl.title = "Root mean square of the errors of the points used: how well they agree";
    const n = s.calib.nudge;
    const moved = n.eastM || n.northM || n.turnDeg;
    p.querySelector(".calib-nudge-sum").textContent = moved ? `${n.eastM >= 0 ? "E" : "W"} ${fmtM(Math.abs(n.eastM))}, ${n.northM >= 0 ? "N" : "S"} ${fmtM(Math.abs(n.northM))}, ${n.turnDeg >= 0 ? "+" : ""}${n.turnDeg.toFixed(1)}°` : "";
    p.querySelector(".calib-nudge-zero").hidden = !moved;
    p.querySelector(".calib-nudge").classList.toggle("on", s.nudging);
    p.querySelector(".calib-nudge-box").hidden = !s.nudging;
    p.querySelector(".calib-save").disabled = !r || (!r.bounds && !(r.model === "nudge"));
    p.querySelector(".calib-clear").hidden = !s.e.calibration;
  }

  async function typePosition(i) {
    const pt = s.calib.points[i];
    const v = await ask({
      title: `Point ${i + 1}: its position`,
      note: "As on a chart or from a GPS: 41 16.234 S, 41°16.234′S, -41.2706 …",
      fields: [
        { key: "lat", label: "Latitude", value: pt.ll ? fmtDM(pt.ll[1], "NS") : "", placeholder: "41 16.234 S" },
        { key: "lon", label: "Longitude", value: pt.ll ? fmtDM(pt.ll[0], "EW") : "", placeholder: "173 17.040 E" },
      ],
      ok: "Use it",
      check: (x) => {
        const lat = parseCoord(x.lat, "NS"), lon = parseCoord(x.lon, "EW");
        return lat == null || Math.abs(lat) > 85 ? "The latitude isn't right." : lon == null || Math.abs(lon) > 180 ? "The longitude isn't right." : "";
      },
    });
    if (!v) return;
    pt.ll = [parseCoord(v.lon, "EW"), parseCoord(v.lat, "NS")];
    pt.from = "typed";
    s.active = i;
    changed();
  }

  // ---------- the picture's pane ----------
  function buildImageMap() {
    const { width: w, height: h, pixelZoom: z } = s.info;
    const tl = pxToLngLat(0, 0), br = pxToLngLat(w, h);
    const url = tileUrl.replace("{z}/{x}/{y}.pbf", `imgp/${s.e.id}/{z}/{x}/{y}.png`);
    pxMap = new maplibregl.Map({
      container: "calib-img",
      style: {
        version: 8,
        sources: { px: { type: "raster", tiles: [url], tileSize: 256, minzoom: 0, maxzoom: z, bounds: [tl.lng, br.lat, br.lng, tl.lat] } },
        layers: [
          { id: "bg", type: "background", paint: { "background-color": "#1b2228" } },
          { id: "px", type: "raster", source: "px", paint: { "raster-resampling": "nearest", "raster-fade-duration": 0 } },
        ],
      },
      bounds: [[tl.lng, br.lat], [br.lng, tl.lat]],
      fitBoundsOptions: { padding: 20 },
      maxZoom: z + 4,
      renderWorldCopies: false,
      dragRotate: false,
      pitchWithRotate: false,
      touchPitch: false,
      attributionControl: false,
    });
    pxMap.touchZoomRotate.disableRotation();
    pxMap.getCanvas().style.cursor = "crosshair";
    pxMap.on("click", (ev) => {
      if (!s || s.nudging) return;
      const [c, r] = lngLatToPx(ev.lngLat);
      if (c < 0 || r < 0 || c > w || r > h) return;
      setEnd("px", [c, r], "");
    });
  }

  // ---------- clicks on the chart ----------
  map.on("click", (ev) => {
    if (!s || s.nudging) return;
    // a library mark under the click: its exact position
    const pad = 8;
    const box = [[ev.point.x - pad, ev.point.y - pad], [ev.point.x + pad, ev.point.y + pad]];
    const f = map.queryRenderedFeatures(box).find((x) => /^lib-/.test(x.layer.id) && x.geometry?.type === "Point");
    if (f) setEnd("ll", f.geometry.coordinates.slice(0, 2), `mark: ${f.properties?.name || "mark"}`);
    else setEnd("ll", [ev.lngLat.lng, ev.lngLat.lat], "chart");
  });

  /** Put one end of a point: on the point being made, else on a new one. */
  function setEnd(key, v, from) {
    const pts = s.calib.points;
    let i = s.active;
    const cur = pts[i];
    // the point being made (one end or none) takes it; clicking the same side again moves that end
    const making = cur && (!cur.px || !cur.ll);
    if (!making) {
      pts.push({ px: null, ll: null, off: false, from: "" });
      i = pts.length - 1;
    }
    pts[i][key] = v;
    if (key === "ll") pts[i].from = from;
    s.active = i;
    changed();
  }

  // ---------- markers ----------
  function markerEl(i, kind) {
    const d = document.createElement("div");
    d.className = `calib-pt ${kind}`;
    d.textContent = String(i + 1);
    return d;
  }
  function drawMarkers() {
    for (const m of [...imgMarkers, ...chartMarkers]) m.remove();
    imgMarkers.length = chartMarkers.length = 0;
    if (!s) return;
    s.calib.points.forEach((pt, i) => {
      const cls = `${i === s.active ? "active" : ""} ${pt.off ? "off" : ""} ${pt.px && pt.ll ? "" : "half"}`;
      if (pt.px && pxMap) {
        const el = markerEl(i, `img ${cls}`);
        const m = new maplibregl.Marker({ element: el, draggable: !s.nudging }).setLngLat(pxToLngLat(pt.px[0], pt.px[1])).addTo(pxMap);
        m.on("dragend", () => { pt.px = lngLatToPx(m.getLngLat()); s.active = i; changed(); });
        el.addEventListener("click", (ev) => { ev.stopPropagation(); s.active = i; drawAll(); });
        imgMarkers.push(m);
      }
      if (pt.ll) {
        const el = markerEl(i, `chart ${cls}`);
        const m = new maplibregl.Marker({ element: el, draggable: !s.nudging }).setLngLat(pt.ll).addTo(map);
        m.on("dragend", () => { const ll = m.getLngLat(); pt.ll = [ll.lng, ll.lat]; pt.from = "chart"; s.active = i; changed(); });
        el.addEventListener("click", (ev) => { ev.stopPropagation(); s.active = i; drawAll(); });
        chartMarkers.push(m);
      }
    });
  }

  function flyTo(i) {
    const pt = s.calib.points[i];
    if (pt?.px && pxMap) pxMap.easeTo({ center: pxToLngLat(pt.px[0], pt.px[1]) });
    if (pt?.ll) map.easeTo({ center: pt.ll });
  }

  function drawAll() { drawPane(); drawMarkers(); }

  // ---------- fitting and the draft on the chart ----------
  function changed() {
    s.dirty = true;
    drawAll();
    clearTimeout(s.fitTimer);
    s.fitTimer = setTimeout(() => refit(false), 150);
  }

  async function refit(quiet) {
    if (!s) return;
    const id = s.e.id;
    try {
      const r = await invoke("images_calib_fit", { id, calib: s.calib });
      if (!s || s.e.id !== id) return;
      s.report = r;
      s.ver += 1;
    } catch (err) {
      if (!quiet) toast(String(err), 5000);
    }
    drawPane();
    overlay();
  }

  function overlay() {
    if (!s) return;
    const r = s.report;
    const ok = s.overlay && r && r.bounds && r.bounds.every(Number.isFinite) && r.bounds[2] > r.bounds[0];
    if (!ok) {
      if (map.getLayer("calib-draft")) map.removeLayer("calib-draft");
      if (map.getSource("calib-draft")) map.removeSource("calib-draft");
      return;
    }
    const url = tileUrl.replace("{z}/{x}/{y}.pbf", `imgd/${s.e.id}/{z}/{x}/{y}.png`) + `&v=${s.ver}`;
    const b = r.bounds;
    // a new area each fit: the source is made again (setTiles keeps the old bounds)
    if (map.getLayer("calib-draft")) map.removeLayer("calib-draft");
    if (map.getSource("calib-draft")) map.removeSource("calib-draft");
    const z = Math.max(0, Math.min(22, Math.ceil(Math.log2((2 * Math.PI * 6378137 * Math.cos(((b[1] + b[3]) / 2) * Math.PI / 180)) / (256 * Math.max(0.05, r.resM ?? 10))))));
    map.addSource("calib-draft", { type: "raster", tiles: [url], tileSize: 256, maxzoom: Math.min(22, z + 1),
      bounds: [Math.max(-180, b[0]), Math.max(-85, b[1]), Math.min(180, b[2]), Math.min(85, b[3])] });
    map.addLayer({ id: "calib-draft", type: "raster", source: "calib-draft", paint: { "raster-opacity": s.opacity, "raster-fade-duration": 0 } },
      map.getLayer("img-over-anchor") ? "img-over-anchor" : undefined);
  }

  async function save() {
    if (!s) return;
    clearTimeout(s.fitTimer);
    try {
      await invoke("images_calib_fit", { id: s.e.id, calib: s.calib });
      const e = await invoke("images_calib_save", { id: s.e.id });
      toast(`${e.name}: calibrated${e.calibRmsM != null ? `, points agree to ±${fmtM(e.calibRmsM)}` : ""}`, 4000);
      s.dirty = false;
      await leave(true);
    } catch (err) { toast(String(err), 6000); }
  }

  // ---------- nudge: drag the chart to move the image, Shift+drag to turn it ----------
  function setNudging(on) {
    s.nudging = on;
    map.getContainer().classList.toggle("calib-nudging", on);
    if (on) map.dragPan.disable(); else map.dragPan.enable();
    drawAll();
  }
  let drag = null;
  map.on("mousedown", (ev) => {
    if (!s?.nudging || ev.originalEvent.button !== 0) return;
    const b = s.report?.bounds;
    const centre = b ? [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2] : [ev.lngLat.lng, ev.lngLat.lat];
    drag = { start: ev.lngLat, turn: ev.originalEvent.shiftKey, n0: { ...s.calib.nudge }, centre };
  });
  map.on("mousemove", (ev) => {
    if (!drag || !s) return;
    const k = Math.cos(drag.start.lat * Math.PI / 180) * 111_320;
    const n = s.calib.nudge;
    if (drag.turn) {
      const ang = (ll) => Math.atan2((ll.lng - drag.centre[0]) * Math.cos(drag.centre[1] * Math.PI / 180), ll.lat - drag.centre[1]) * 180 / Math.PI;
      n.turnDeg = +(drag.n0.turnDeg + ang(ev.lngLat) - ang(drag.start)).toFixed(3);
    } else {
      n.eastM = +(drag.n0.eastM + (ev.lngLat.lng - drag.start.lng) * k).toFixed(2);
      n.northM = +(drag.n0.northM + (ev.lngLat.lat - drag.start.lat) * 111_320).toFixed(2);
    }
    drawPane();
    clearTimeout(s.fitTimer);
    s.fitTimer = setTimeout(() => refit(true), 120);
  });
  window.addEventListener("mouseup", () => { if (drag && s) { drag = null; changed(); } else drag = null; });

  // ---------- leaving from outside ----------
  // Esc leaves (keeping the points), before anything else takes the key; a question box open takes it first
  window.addEventListener("keydown", (ev) => {
    if (!s || ev.key !== "Escape") return;
    if (document.querySelector(".ask-back:not([hidden])")) return;
    ev.preventDefault();
    ev.stopPropagation();
    leave(true);
  }, true);
  // back to Navigation (F2 or the inactivity timer): leave, keeping the points
  window.addEventListener("racenav-view", (ev) => { if (s && ev.detail?.id === "nav") leave(true); });
  // the drawer closed or another opened: leave too
  ui.onChange((pane) => { if (s && pane !== "calibrate") leave(true); });
  // started with the calibration drawer remembered from last time: show the chart drawer instead
  if (ui.current() === "calibrate") ui.open("chart");

  images.onCalibrate(open);
  return { open, active: () => !!s };
}
