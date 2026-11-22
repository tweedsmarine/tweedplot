// The anchor: a library mark (type Anchor, in the Anchorages group) with a watch on it.
//
// Drop it from the anchor button beside MOB and Drop, the chart's right-click menu (here, or at the
// boat) or Alarms › Anchor watch. It goes at the bow (the GPS position moved forward by the
// antenna's distance from the bow, along the heading). Then, during the pull test, drag the
// anchor to where it really lies: the readout gives its distance and bearing from the bow.
// Or type them on its card ("30 m dead ahead"). The swing circle follows; its radius is set on
// the card (Radius = scope gives the usual one, both from the bow).
//
// The card also keeps the bottom, the holding and notes. Weigh anchor ends the watch but keeps
// the mark, so the Anchorages group becomes a record of where you've anchored and how it held;
// click one (or right-click it) to anchor there again.
//
// The watch itself (the alarm) is in alarms.js, run by the primary window; this module draws
// and edits, in any window.
import { invoke } from "@tauri-apps/api/core";
import { createInfoBox } from "./infobox.js";
import { rhumb, rhumbDestination, fmtBrg, ddm, bowOf } from "./navmath.js";

export const ANCHOR_GROUP = "Anchorages";
const ORANGE = "#c45500";
const BOTTOMS = [["", "Not recorded"], ["sand", "Sand"], ["mud", "Mud"], ["clay", "Clay"], ["shingle", "Shingle / gravel"], ["weed", "Weed"],
  ["shell", "Shell"], ["rock", "Rock"], ["coral", "Coral"]];
const HOLDING = [["", "Not recorded"], ["1", "1 · poor (dragged)"], ["2", "2 · fair"], ["3", "3 · good"], ["4", "4 · very good"], ["5", "5 · excellent"]];
const empty = { type: "FeatureCollection", features: [] };
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** A circle of radius `m` metres round [lon, lat], as a ring. */
function ring(c, m) {
  const pts = [];
  for (let i = 0; i <= 72; i++) pts.push(rhumbDestination(c, (i / 72) * 360, m / 1852));
  return pts;
}

/** The anchor symbol, drawn once (paper-chart anchor, mariner's orange, white halo). */
function anchorImage() {
  const S = 44, c = document.createElement("canvas");
  c.width = c.height = S;
  const g = c.getContext("2d");
  g.translate(S / 2, S / 2);
  const draw = (col, w) => {
    g.strokeStyle = col; g.lineWidth = w; g.lineCap = "round"; g.lineJoin = "round";
    g.beginPath(); g.arc(0, -13, 3.5, 0, Math.PI * 2); g.stroke();
    g.beginPath(); g.moveTo(0, -9.5); g.lineTo(0, 15); g.stroke();
    g.beginPath(); g.moveTo(-8, -4); g.lineTo(8, -4); g.stroke();
    g.beginPath(); g.arc(0, 3, 13, 0.12 * Math.PI, 0.88 * Math.PI); g.stroke();
    g.beginPath(); g.moveTo(-12.5, 7); g.lineTo(-13.5, 2.5); g.moveTo(12.5, 7); g.lineTo(13.5, 2.5); g.stroke();
  };
  draw("rgba(255,255,255,0.95)", 7);
  draw(ORANGE, 3);
  return g.getImageData(0, 0, S, S);
}

export function setupAnchor(map, { library, alarms, getBoat, getVessel, getAntenna = null, toast = () => {}, before }) {
  map.addImage("anc:sym", anchorImage(), { pixelRatio: 2 });
  for (const id of ["anc-circle", "anc-line", "anc-pt"]) map.addSource(id, { type: "geojson", data: empty });
  map.addLayer({ id: "anc-fill", type: "fill", source: "anc-circle", paint: { "fill-color": ORANGE, "fill-opacity": 0.07 } }, before);
  map.addLayer({ id: "anc-ring", type: "line", source: "anc-circle", paint: { "line-color": ORANGE, "line-width": 2, "line-dasharray": [4, 2.5] } }, before);
  map.addLayer({ id: "anc-rode", type: "line", source: "anc-line", paint: { "line-color": ORANGE, "line-width": 1.4, "line-dasharray": [1, 2] } }, before);
  map.addLayer({ id: "anc-lbl", type: "symbol", source: "anc-line", filter: ["has", "lbl"],
    layout: { "icon-image": ["concat", "txt:r:", ["get", "lbl"]], "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-anchor": "bottom", "icon-offset": [0, -6] } }, before);
  map.addLayer({ id: "anc-sym", type: "symbol", source: "anc-pt",
    layout: { "icon-image": "anc:sym", "icon-allow-overlap": true, "icon-ignore-placement": true, "icon-rotation-alignment": "viewport" } }, before);

  let drag = null; // [lon, lat] while dragging the anchor
  let preview = null; // a radius being typed or spun on the card, drawn before it is saved
  const timers = {};

  /** The bow: the GPS position moved forward along the heading by the antenna's distance from the bow. */
  // (the same bow as the anchor alarm measures from: navmath.bowOf)
  function bow() {
    const b = getBoat();
    const o = getAntenna?.() ?? { fromBow: Number(getVessel().gpsFromBow) || 0, fromCentre: Number(getVessel().gpsFromCentre) || 0 };
    return bowOf(b.pos, b.v || {}, o.fromBow, o.fromCentre);
  }
  /** "34 m at 012°T from the bow". */
  function fromBow(p) {
    const b = bow();
    if (!b) return "";
    const r = rhumb(b.pos, p);
    return `${Math.round(r.nm * 1852)} m at ${fmtBrg(r.brg)}°T from the bow`;
  }

  // ---------- drawing ----------------------------------------------------------------
  function draw() {
    const a = alarms.anchor.get();
    library.exclude(a?.id ? [a.id] : []);
    if (!a) {
      for (const id of ["anc-circle", "anc-line", "anc-pt"]) map.getSource(id).setData(empty);
      return;
    }
    const p = drag ?? [a.lon, a.lat];
    map.getSource("anc-pt").setData({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: p } });
    const radius = preview ?? a.radius;
    map.getSource("anc-circle").setData({ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [ring(p, radius)] } });
    const b = bow();
    const feats = [];
    if (b) {
      feats.push({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: [b.pos, p] } });
      // while dragging, and close in, say where it is from the bow
      if (drag) feats.push({ type: "Feature", properties: { lbl: fromBow(p) }, geometry: { type: "Point", coordinates: p } });
    }
    feats.push({ type: "Feature", properties: { lbl: `${Math.round(radius)} m` }, geometry: { type: "Point", coordinates: rhumbDestination(p, 0, radius / 1852) } });
    map.getSource("anc-line").setData({ type: "FeatureCollection", features: feats });
  }
  window.addEventListener("racenav-anchor-watch", () => { draw(); refreshCard(); });
  setInterval(draw, 1000); // the boat swings: the line to the bow follows

  // ---------- dropping and weighing --------------------------------------------------
  async function drop(at = null) {
    const b = bow();
    const p = at ?? b?.pos;
    if (!p) { toast("No boat position yet: right-click the chart where the anchor is, then Drop anchor here.", 6000); return; }
    const v = getBoat().v || {};
    const when = new Date();
    const radius = alarms.anchor.defaultRadius();
    const depth = Number.isFinite(v.depth_surface) ? v.depth_surface : Number.isFinite(v.depth) ? v.depth : null;
    let mark = { name: `Anchor ${when.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })}`, lat: p[1], lon: p[0], kind: "anchor", group: ANCHOR_GROUP,
      notes: "", data: { radius_m: radius, dropped: Math.round(when.getTime() / 1000) } };
    if (depth != null) mark.depth = Math.round(depth * 10) / 10;
    try {
      mark = (await invoke("mark_save", { mark })) ?? mark;
    } catch (err) { toast(`Could not save the anchor mark: ${err}`, 6000); }
    alarms.anchor.set({ lon: mark.lon, lat: mark.lat, id: mark.id ?? null, radius, name: mark.name });
    await library.refresh();
    toast(at ? "Anchor dropped here. Drag it to where it lies, and set its swing radius on its card." : "Anchor dropped at the bow. During the pull test, drag it to where it lies (its card says how far ahead).", 7000);
    openCard(mark);
  }
  function weigh() {
    const a = alarms.anchor.get();
    if (!a) return;
    alarms.anchor.weigh();
    toast(`Anchor watch off. ${a.name || "The anchor"} stays in Library › Marks › ${ANCHOR_GROUP} with its notes.`, 5000);
    library.refresh();
  }
  /** Watch an anchor mark again (anchoring in the same place). */
  async function again(m) {
    const radius = Number(m.data?.radius_m) || alarms.anchor.defaultRadius();
    const data = { ...(m.data || {}), radius_m: radius, dropped: Math.round(Date.now() / 1000) };
    try { m = await invoke("mark_save", { mark: { ...m, data } }); } catch { /* keep going: the watch matters more */ }
    alarms.anchor.set({ lon: m.lon, lat: m.lat, id: m.id, radius, name: m.name });
    library.refresh();
    openCard(m);
  }
  window.addEventListener("racenav-anchor-drop", () => drop());
  window.addEventListener("racenav-anchor-weigh", () => weigh());
  window.addEventListener("racenav-anchor-open", () => { const a = alarms.anchor.get(); if (a) openCard(markOf(a)); });
  const markOf = (a) => (a?.id && library.byId(a.id)) || { id: a?.id, name: a?.name || "Anchor", lat: a?.lat, lon: a?.lon, kind: "anchor", group: ANCHOR_GROUP, notes: "", data: { radius_m: a?.radius } };
  const isWatched = (m) => { const a = alarms.anchor.get(); return !!(a && m && a.id && a.id === m.id); };

  // ---------- dragging the watched anchor ---------------------------------------------
  // No arming delay (unlike other marks): the anchor is set during the pull test, often in a hurry.
  // The pointer is followed on the window, not the map: the drag may pass over the anchor's
  // card or another panel, and must still end where the button is let go.
  function startDrag(e) {
    if (!alarms.anchor.get() || drag) return;
    if (e.originalEvent.button !== undefined && e.originalEvent.button !== 0) return;
    e.preventDefault();
    map.dragPan.disable();
    drag = [e.lngLat.lng, e.lngLat.lat];
    map.getCanvas().style.cursor = "grabbing";
    const at = (ev) => {
      const pt = ev.touches?.[0] ?? ev.changedTouches?.[0] ?? ev;
      const r = map.getCanvas().getBoundingClientRect();
      const ll = map.unproject([pt.clientX - r.left, pt.clientY - r.top]);
      return [ll.lng, ll.lat];
    };
    const move = (ev) => { drag = at(ev); draw(); };
    const up = (ev) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      const p = ev.type === "pointercancel" ? drag : at(ev);
      drag = null;
      map.dragPan.enable();
      map.getCanvas().style.cursor = "";
      moveTo(p);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  }
  map.on("mousedown", "anc-sym", startDrag);
  async function moveTo(p) {
    const a = alarms.anchor.get();
    if (!a) return;
    alarms.anchor.set({ ...a, lon: p[0], lat: p[1] });
    const m = a.id && library.byId(a.id);
    if (m) {
      try { await invoke("mark_save", { mark: { ...m, lon: p[0], lat: p[1] } }); await library.refresh(); } catch (err) { toast(`Could not save the anchor: ${err}`, 6000); }
    }
    toast(`Anchor ${fromBow(p) || `at ${ddm(p[1], "N", "S")} ${ddm(p[0], "E", "W")}`}.`, 4000);
    draw(); refreshCard();
  }
  map.on("mouseenter", "anc-sym", () => { if (!drag) map.getCanvas().style.cursor = "grab"; });
  map.on("mouseleave", "anc-sym", () => { if (!drag) map.getCanvas().style.cursor = ""; });
  let downAt = null;
  map.on("mousedown", "anc-sym", (e) => { downAt = [e.point.x, e.point.y]; });
  map.on("click", "anc-sym", (e) => {
    // a click, not the end of a drag: the card
    if (downAt && Math.hypot(e.point.x - downAt[0], e.point.y - downAt[1]) > 4) return;
    openCard(markOf(alarms.anchor.get()));
  });

  // ---------- the anchor's card --------------------------------------------------------
  const box = createInfoBox(map, "anchor", { title: "Anchor", width: 340, height: 470, showRing: false });
  let cardMark = null;
  function openCard(m) {
    if (!m) return;
    cardMark = m;
    box.open(cardHtml(m), { title: m.name || "Anchor", at: [m.lon, m.lat] });
    wire();
  }
  function refreshCard() {
    if (!box.isOpen || !cardMark) return;
    cardMark = (cardMark.id && library.byId(cardMark.id)) || cardMark;
    if (box.element.contains(document.activeElement)) return; // don't rebuild under the user's typing
    box.update(cardHtml(cardMark), { title: cardMark.name || "Anchor", at: [cardMark.lon, cardMark.lat] });
    wire();
  }
  function cardHtml(m) {
    const d = m.data || {};
    const watched = isWatched(m);
    const a = alarms.anchor.get();
    const radius = watched ? a.radius : Number(d.radius_m) || alarms.anchor.defaultRadius();
    const b = bow();
    const rel = b ? rhumb(b.pos, [m.lon, m.lat]) : null;
    const opt = (list, v) => list.map(([k, t]) => `<option value="${k}"${String(v ?? "") === k ? " selected" : ""}>${esc(t)}</option>`).join("");
    const dropped = d.dropped ? new Date(d.dropped * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "";
    return `<div class="anc-card">
      <p class="anc-state ${watched ? "on" : ""}">${watched ? `<b>Watching</b>: alarm if the boat goes more than ${Math.round(radius)} m from the anchor` : "Not watched: a record of where you anchored"}</p>
      ${watched && b ? `<fieldset><legend>Where the anchor lies</legend>
        <div class="anc-row"><input data-k="dist" type="number" min="0" step="1" value="${rel ? Math.round(rel.nm * 1852) : ""}"> m at
          <input data-k="brg" type="number" min="0" max="360" step="1" value="${rel ? Math.round(rel.brg) % 360 : ""}"> °T from the bow
          <button data-act="ahead" title="The bearing of the boat's heading">Dead ahead</button></div>
        <div class="anc-row"><button data-act="place">Move the anchor there</button><span class="wx-note">or drag it on the chart</span></div></fieldset>` : ""}
      <fieldset><legend>Swing circle</legend>
        <div class="anc-row"><label>Radius <input data-k="radius" type="number" min="5" step="1" value="${Math.round(radius)}"> m</label></div>
        <div class="anc-row"><label>Scope <input data-k="scope" type="number" min="0" step="1" value="${d.scope_m ?? ""}"> m</label>
          <button data-act="fromscope" title="The bow can't swing further from the anchor than the scope (chain or rode out), so the radius is the scope">Radius = scope</button></div>
        <p class="wx-note">Measured from the bow, as the alarm is.</p></fieldset>
      <fieldset><legend>The anchorage</legend>
        <div class="anc-row"><label>Name <input data-k="name" type="text" value="${esc(m.name)}"></label></div>
        <div class="anc-row"><label>Bottom <select data-k="bottom">${opt(BOTTOMS, d.bottom)}</select></label></div>
        <div class="anc-row"><label>Holding <select data-k="holding">${opt(HOLDING, d.holding)}</select></label></div>
        <label class="anc-notes">Notes<textarea data-k="notes" rows="3" placeholder="Shelter, swell, weed patches, how it set…">${esc(m.notes)}</textarea></label>
        <p class="wx-note">${[m.depth != null ? `Depth when dropped ${m.depth} m` : "", dropped ? `Dropped ${esc(dropped)}` : "", `${ddm(m.lat, "N", "S")} ${ddm(m.lon, "E", "W")}`].filter(Boolean).join(" · ")}</p></fieldset>
      <div class="anc-btns">
        ${watched ? `<button data-act="weigh" class="primary">Weigh anchor</button>` : `<button data-act="again" class="primary">Anchor here again</button>`}
        <button data-act="centre">Centre</button>
        ${m.id ? `<button data-act="library">In the Library</button>` : ""}
      </div></div>`;
  }
  function wire() {
    const el = box.body;
    const q = (k) => el.querySelector(`[data-k="${k}"]`);
    const act = (k, f) => el.querySelector(`[data-act="${k}"]`)?.addEventListener("click", f);
    const m = () => cardMark;
    /** Save the mark's fields and data (and the watch, if it is the watched one). */
    async function saveMark(patch = {}, dataPatch = {}) {
      const cur = m();
      const next = { ...cur, ...patch, data: { ...(cur.data || {}), ...dataPatch } };
      for (const [k, v] of Object.entries(next.data)) if (v === "" || v == null) delete next.data[k];
      cardMark = next;
      if (cur.id) {
        try { cardMark = await invoke("mark_save", { mark: next }); library.refresh(); } catch (err) { toast(`Could not save the anchor: ${err}`, 6000); }
      }
      if (isWatched(cur)) { const a = alarms.anchor.get(); alarms.anchor.set({ ...a, radius: Number(next.data.radius_m) || a.radius, name: next.name }); }
    }
    q("name")?.addEventListener("change", (e) => saveMark({ name: e.target.value.trim() || m().name }));
    q("notes")?.addEventListener("change", (e) => saveMark({ notes: e.target.value }));
    q("bottom")?.addEventListener("change", (e) => saveMark({}, { bottom: e.target.value }));
    q("holding")?.addEventListener("change", (e) => saveMark({}, { holding: e.target.value ? Number(e.target.value) : "" }));
    // number fields: the spinner sends a change on every click, so the circle follows at once but
    // the save waits until the clicking stops (saving refreshes the whole library)
    const later = (k, f) => (e) => { clearTimeout(timers[k]); timers[k] = setTimeout(() => f(e), 800); };
    q("radius")?.addEventListener("input", (e) => { const v = Number(e.target.value); if (v >= 5) { preview = v; draw(); } });
    q("radius")?.addEventListener("change", later("radius", (e) => { const v = Number(e.target.value); if (v >= 5) saveMark({}, { radius_m: Math.round(v) }).then(() => { preview = null; draw(); }); }));
    q("scope")?.addEventListener("change", later("scope", (e) => { const v = Number(e.target.value); saveMark({}, { scope_m: v > 0 ? Math.round(v) : "" }); }));
    act("fromscope", () => {
      const scope = Number(q("scope").value);
      if (!(scope > 0)) { toast("Enter the scope (chain or rode out) first.", 4000); return; }
      const r = Math.round(scope);
      q("radius").value = r;
      saveMark({}, { radius_m: r, scope_m: Math.round(scope) }).then(draw);
    });
    act("ahead", () => { const b = bow(); if (b?.heading != null) q("brg").value = Math.round(b.heading) % 360; else toast("No heading yet: enter the bearing.", 4000); });
    const place = () => {
      const b = bow();
      const dist = Number(q("dist").value), brg = Number(q("brg").value);
      if (!b || !(dist >= 0) || !Number.isFinite(brg)) return;
      moveTo(rhumbDestination(b.pos, brg, dist / 1852));
    };
    act("place", place);
    // typed or spun: the anchor moves once the clicking stops, not on every click
    q("dist")?.addEventListener("change", later("place", place));
    q("brg")?.addEventListener("change", later("place", place));
    act("weigh", () => { weigh(); refreshCard(); });
    act("again", () => again(m()));
    act("centre", () => map.easeTo({ center: [m().lon, m().lat], zoom: Math.max(map.getZoom(), 15) }));
    act("library", () => library.openMark(m().id));
  }
  library.onAnchorClick((m) => openCard(m));

  draw();
  return {
    drop,
    weigh,
    again,
    open: openCard,
    watching: () => !!alarms.anchor.get(),
    /** The anchor mark under a map point (the watched one, drawn here), for the right-click menu. */
    at(point) {
      const f = map.queryRenderedFeatures([[point.x - 8, point.y - 8], [point.x + 8, point.y + 8]], { layers: ["anc-sym"] })[0];
      return f ? markOf(alarms.anchor.get()) : null;
    },
  };
}
