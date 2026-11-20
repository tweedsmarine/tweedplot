// Chart › Layers › Images: KAP raster charts and GeoTIFFs from the computer (copied into
// TweedPlot's images folder, or catalogued where they are: charts already installed for another
// program, or on an external drive, are read there and never copied or deleted), and
// Sentinel-2 satellite images downloaded from Earth Search on AWS (imagery_cmd.rs). Rust draws
// each image as tiles on the chart protocol (/img/<id>/<z>/<x>/<y>.png); here they become
// raster layers, under the chart (over the depth shading, under the contours, land and
// soundings) or over it. Own soundings always stay on top of the images.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask } from "./ask.js";

const $ = (id) => document.getElementById(id);
// invisible marker layers that keep the two places images go, whatever else moves
const UNDER = "img-under-anchor";
const OVER = "img-over-anchor";

export function setupImages(map, { tileUrl, toast }) {
  let list = [];
  let scenes = [];
  // list folded away; choosing images for bulk changes (ids)
  let folded = false;
  try { folded = localStorage.getItem("tweedplot.images.folded") === "1"; } catch { /* storage unavailable */ }
  let selecting = false;
  const picked = new Set();
  let area = null;
  const layerId = (id) => `img-${id}`;
  const tiles = (id) => tileUrl.replace("{z}/{x}/{y}.pbf", `img/${id}/{z}/{x}/{y}.png`);

  function anchors() {
    if (!map.getLayer(UNDER)) map.addLayer({ id: UNDER, type: "background", layout: { visibility: "none" } }, map.getLayer("pbathy") ? "pbathy" : undefined);
    if (!map.getLayer(OVER)) map.addLayer({ id: OVER, type: "background", layout: { visibility: "none" } }, map.getLayer("soundg") ? "soundg" : undefined);
  }

  // A raster chart shows only over the zooms its scale suits: from 8 times smaller than its
  // own pixels (fading in over half a zoom level) to 4 times larger (fading out over one), as
  // chart plotters switch charts with the display scale. Satellite images and GeoTIFFs show at
  // every zoom.
  const zoomRange = (e) => (e.kind === "kap" && e.nativeZoom ? { from: Math.max(0, e.nativeZoom - 3), to: Math.min(23, e.nativeZoom + 2) } : null);
  function opacityOf(e, op = e.opacity) {
    const r = zoomRange(e);
    if (!r) return op;
    return ["interpolate", ["linear"], ["zoom"], Math.max(0, r.from - 0.5), 0, r.from, op, r.to, op, r.to + 1, 0];
  }
  function showWhole(e) {
    const b = lonLatBounds(e.bounds);
    map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: 40, maxZoom: e.nativeZoom || 16 });
    // a large chart fitted to the screen can be zoomed out past where it shows: zoom in to it
    const r = zoomRange(e);
    if (r) map.once("moveend", () => { if (map.getZoom() < r.from) map.easeTo({ zoom: r.from + 0.2 }); });
  }

  const lonLatBounds = (b) => [Math.max(-180, b[0]), Math.max(-85, b[1]), Math.min(180, b[2]), Math.min(85, b[3])];

  function syncMap() {
    anchors();
    const ids = new Set(list.map((e) => layerId(e.id)));
    for (const l of map.getStyle().layers) {
      if (/^img-[0-9a-f]+$/.test(l.id) && !ids.has(l.id)) { map.removeLayer(l.id); if (map.getSource(l.id)) map.removeSource(l.id); }
    }
    // later images in the list draw on top
    for (const e of list) {
      const id = layerId(e.id);
      if (!map.getSource(id)) {
        map.addSource(id, { type: "raster", tiles: [tiles(e.id)], tileSize: 256, bounds: lonLatBounds(e.bounds), maxzoom: Math.min(22, (e.nativeZoom || 14) + 1) });
      }
      if (map.getLayer(id)) map.removeLayer(id);
      const r = zoomRange(e);
      map.addLayer({ id, type: "raster", source: id,
        ...(r ? { minzoom: Math.max(0, r.from - 0.5), maxzoom: Math.min(24, r.to + 1) } : {}),
        layout: { visibility: e.visible && !e.missing ? "visible" : "none" },
        paint: { "raster-opacity": opacityOf(e), "raster-resampling": "linear", "raster-fade-duration": 0 } },
        e.placement === "over" ? OVER : UNDER);
    }
    // own soundings above any image drawn over the chart; back in their own place otherwise
    if (map.getLayer("pbathy")) {
      const over = list.some((e) => e.visible && e.placement === "over");
      if (over) map.moveLayer("pbathy", OVER);
      else if (map.getLayer("osm")) map.moveLayer("pbathy", "osm");
    }
    credit();
    // own ship gets a halo over images (mariner.js)
    const shown = list.some((e) => e.visible && !e.missing);
    window.tweedplotImagesShown = shown; // for modules set up after this one
    window.dispatchEvent(new CustomEvent("raster-images", { detail: { shown } }));
  }

  function credit() {
    const credits = mergeYears([...new Set(list.filter((e) => e.visible && !e.missing && e.credit).map((e) => e.credit))]);
    const kaps = list.some((e) => e.visible && !e.missing && e.kind === "kap");
    const text = [credits.length ? `Imagery: ${credits.join("; ")}.` : "", kaps ? "Raster charts: their publishers." : ""].filter(Boolean).join(" ");
    window.dispatchEvent(new CustomEvent("credit-extra", { detail: { key: "images", text } }));
  }

  /** "… data 2025", "… data 2026" → "… data 2025–2026" (one phrase per source, years run together). */
  function mergeYears(credits) {
    const years = new Map(); // text before the year → years
    const out = [];
    for (const c of credits) {
      const m = /^(.*\S)\s+(\d{4})$/.exec(c);
      if (!m) { out.push(c); continue; }
      if (!years.has(m[1])) { years.set(m[1], new Set()); out.push(m[1]); }
      years.get(m[1]).add(Number(m[2]));
    }
    return out.map((c) => {
      const ys = years.get(c);
      if (!ys) return c;
      const s = [...ys].sort((a, b) => a - b);
      const runs = [];
      for (const y of s) {
        const last = runs[runs.length - 1];
        if (last && y === last[1] + 1) last[1] = y; else runs.push([y, y]);
      }
      return `${c} ${runs.map(([a, b]) => (a === b ? `${a}` : `${a}–${b}`)).join(", ")}`;
    });
  }

  const fmtRes = (m) => (m >= 100 ? `${Math.round(m)} m` : m >= 1 ? `${m.toFixed(m >= 10 ? 0 : 1)} m` : `${Math.round(m * 100)} cm`);
  /** How old a satellite image is: what matters for ice, banks and channels that move. */
  function age(date) {
    const t = Date.parse(date);
    if (!Number.isFinite(t)) return "";
    const d = Math.floor((Date.now() - t) / 86_400_000);
    if (d < 1) return "today";
    if (d < 2) return "1 day old";
    if (d < 21) return `${d} days old`;
    if (d < 75) return `${Math.round(d / 7)} weeks old`;
    if (d < 548) return `${Math.round(d / 30.4)} months old`;
    return `${(d / 365.25).toFixed(d < 3650 ? 1 : 0).replace(/\.0$/, "")} years old`;
  }
  /** The line under an image's name: what the name doesn't already say, most useful first. */
  function meta(e) {
    if (e.missing) return "File not found: is its drive connected?";
    const parts = [];
    const inName = (s) => s && e.name.toLowerCase().includes(String(s).toLowerCase());
    if (e.kind === "kap") {
      if (e.scale && !inName(e.scale.toLocaleString("en-NZ")) && !inName(e.scale)) parts.push(`1:${e.scale.toLocaleString("en-NZ")}`);
      if (e.fitRmsPx != null) parts.push(e.fitRmsPx < 0.05 ? "fit exact" : `fit ${e.fitRmsPx.toFixed(1)} px`);
      parts.push(`${fmtRes(e.resM)} pixels`);
      if (e.date && !inName(e.date)) parts.push(e.date);
    } else {
      const sat = /sentinel|landsat/i.test(e.source);
      parts.push(`${fmtRes(e.resM)} pixels`);
      if (sat && e.date) parts.push(age(e.date));
      else if (e.date && !inName(e.date)) parts.push(e.date);
      if (!inName(e.source) && !(sat && /sentinel|landsat/i.test(e.name))) parts.push(e.source);
      if (e.fitRmsPx != null) parts.push(e.fitRmsPx < 0.05 ? "fit exact" : `fit ${e.fitRmsPx.toFixed(1)} px`);
    }
    if (e.linked) parts.push("in place");
    return parts.filter(Boolean).join(" · ");
  }
  /** Where an image's file is, for tooltips. */
  const whereText = (e) => (e.linked ? `Used where it is: ${e.file}` : "TweedPlot's copy, in its images folder");

  async function update(e, patch) {
    try {
      const n = await invoke("images_update", { id: e.id, patch });
      Object.assign(e, n);
    } catch (err) { toast(String(err), 5000); }
  }

  function head() {
    const shown = list.filter((e) => e.visible && !e.missing).length;
    const away = list.filter((e) => e.missing).length;
    $("img-count").textContent = list.length ? `${list.length} image${list.length === 1 ? "" : "s"}, ${shown} shown${away ? `, ${away} not found` : ""}` : "";
    $("img-fold").textContent = folded ? "▸" : "▾";
    $("img-fold").setAttribute("aria-expanded", String(!folded));
    $("img-fold").hidden = !list.length;
    $("img-body").hidden = folded && list.length > 0;
    $("img-select").hidden = !list.length;
    $("img-select").classList.toggle("on", selecting);
    $("img-bulk").hidden = !selecting;
    for (const id of [...picked]) if (!list.some((e) => e.id === id)) picked.delete(id);
    const n = picked.size;
    $("img-nsel").textContent = `${n} of ${list.length} selected`;
    $("img-all").checked = n > 0 && n === list.length;
    $("img-all").indeterminate = n > 0 && n < list.length;
    for (const id of ["img-b-show", "img-b-hide", "img-b-place", "img-b-op", "img-b-del"]) $(id).disabled = n === 0;
  }

  function render() {
    const box = $("img-list");
    if (!box) return;
    head();
    box.replaceChildren();
    if (!list.length) {
      const n = document.createElement("div");
      n.className = "wx-note";
      n.textContent = "No images yet: import a KAP chart or GeoTIFF, or get a satellite image of the area on the screen.";
      box.append(n);
    }
    // top of the list = drawn on top
    [...list].reverse().forEach((e) => {
      const row = document.createElement("div");
      row.className = "img-row" + (selecting && picked.has(e.id) ? " picked" : "") + (e.missing ? " missing" : "");
      const top = document.createElement("div");
      top.className = "ly-row";
      if (selecting) {
        const pick = document.createElement("input");
        pick.type = "checkbox"; pick.className = "img-pick";
        pick.checked = picked.has(e.id);
        pick.title = "Select for the changes above";
        pick.setAttribute("aria-label", `Select ${e.name}`);
        pick.onchange = () => { if (pick.checked) picked.add(e.id); else picked.delete(e.id); row.classList.toggle("picked", pick.checked); head(); };
        top.append(pick);
      }
      const lab = document.createElement("label");
      const zr = zoomRange(e);
      lab.title = `${e.name}\n${meta(e)}\n${e.width.toLocaleString("en-NZ")} × ${e.height.toLocaleString("en-NZ")} pixels`
        + (zr ? `\nShown from zoom ${zr.from} to ${zr.to + 1}, as its scale suits` : "")
        + `\n${whereText(e)}`;
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.checked = e.visible;
      cb.onchange = async () => { await update(e, { visible: cb.checked }); syncMap(); head(); };
      const name = document.createElement("span");
      name.className = "img-name";
      name.textContent = e.name;
      lab.append(cb, name);
      const go = document.createElement("button");
      go.type = "button"; go.className = "ly-gear"; go.textContent = "⌖"; go.title = "Show the whole image";
      go.onclick = () => showWhole(e);
      const del = document.createElement("button");
      del.type = "button"; del.className = "ly-gear"; del.textContent = "✕";
      del.title = e.linked ? "Remove the image from the list (its file is left where it is)" : "Remove the image (deletes TweedPlot's copy)";
      del.onclick = async () => {
        const note = e.linked
          ? `Remove “${e.name}” from TweedPlot's list? Its file is left where it is, untouched.`
          : `Remove “${e.name}” from TweedPlot? Its copy in the images folder is deleted (the file you imported it from is not).`;
        const ok = await ask({ title: "Remove image", note, ok: "Remove" });
        if (!ok) return;
        try { await invoke("images_remove", { id: e.id }); } catch (err) { toast(String(err), 5000); return; }
        await refresh();
      };
      if (selecting) {
        const i = list.indexOf(e);
        const up = document.createElement("button");
        up.type = "button"; up.className = "ly-gear"; up.textContent = "▲"; up.title = "Draw above the image over it in the list";
        up.disabled = i === list.length - 1;
        up.onclick = () => move(i, i + 1);
        const down = document.createElement("button");
        down.type = "button"; down.className = "ly-gear"; down.textContent = "▼"; down.title = "Draw below the image under it in the list";
        down.disabled = i === 0;
        down.onclick = () => move(i, i - 1);
        top.append(lab, up, down, go, del);
      } else {
        top.append(lab, go, del);
      }
      const sub = document.createElement("div");
      sub.className = "img-sub";
      const info = document.createElement("span");
      info.className = "wx-note img-meta" + (e.fitRmsPx != null && e.fitRmsPx > 3 ? " warn" : "");
      info.textContent = meta(e);
      info.title = `${e.source}${e.date ? ` · ${e.date}` : ""}\n${meta(e)}`;
      if (e.fitRmsPx != null && e.fitRmsPx > 3) info.title = "The chart's reference points don't agree well: it may be misplaced by several pixels. Check it against your GPS.";
      if (e.missing) { info.className = "wx-note img-meta warn"; info.title = `${e.file}\nTweedPlot uses this file where it is, and can't find it now. It shows again when the drive or folder is back.`; }
      const op = document.createElement("input");
      op.type = "range"; op.min = "10"; op.max = "100"; op.step = "5"; op.value = String(Math.round(e.opacity * 100));
      op.title = "Opacity"; op.setAttribute("aria-label", `Opacity of ${e.name}`);
      op.oninput = () => { if (map.getLayer(layerId(e.id))) map.setPaintProperty(layerId(e.id), "raster-opacity", opacityOf(e, Number(op.value) / 100)); };
      op.onchange = () => update(e, { opacity: Number(op.value) / 100 });
      const pl = document.createElement("select");
      pl.title = "Under the chart: over the depth shading, under contours, land and soundings. Over the chart: over everything but soundings, own soundings, marks and the boat";
      pl.append(new Option("Under chart", "under"), new Option("Over chart", "over"));
      pl.value = e.placement;
      pl.onchange = async () => { await update(e, { placement: pl.value }); syncMap(); };
      sub.append(info, op, pl);
      row.append(top, sub);
      box.append(row);
    });
  }

  async function refresh() {
    try { list = await invoke("images_list"); } catch { list = []; }
    render();
    syncMap();
  }

  // ---- folding and bulk changes ----
  $("img-fold").onclick = () => {
    folded = !folded;
    try { localStorage.setItem("tweedplot.images.folded", folded ? "1" : "0"); } catch { /* ignore */ }
    if (folded) { selecting = false; picked.clear(); }
    render();
  };
  $("img-select").onclick = () => {
    selecting = !selecting;
    picked.clear();
    if (selecting && folded) { folded = false; try { localStorage.setItem("tweedplot.images.folded", "0"); } catch { /* ignore */ } }
    render();
  };
  $("img-done").onclick = () => { selecting = false; picked.clear(); render(); };
  // list order is drawing order: the end of the list is on top
  async function move(from, to) {
    if (to < 0 || to >= list.length) return;
    const ids = list.map((e) => e.id);
    [ids[from], ids[to]] = [ids[to], ids[from]];
    try { await invoke("images_reorder", { ids }); } catch (err) { toast(String(err), 5000); }
    await refresh();
  }
  $("img-b-scale").onclick = async () => {
    try { list = await invoke("images_order_by_scale"); } catch (err) { toast(String(err), 5000); return; }
    render();
    syncMap();
  };
  $("img-all").onchange = () => {
    picked.clear();
    if ($("img-all").checked) for (const e of list) picked.add(e.id);
    render();
  };
  async function bulk(patch) {
    const errors = [];
    for (const e of list.filter((x) => picked.has(x.id))) {
      try { Object.assign(e, await invoke("images_update", { id: e.id, patch })); } catch (err) { errors.push(`${e.name}: ${err}`); }
    }
    if (errors.length) toast(errors.join("; "), 6000);
    render();
    syncMap();
  }
  $("img-b-show").onclick = () => bulk({ visible: true });
  $("img-b-hide").onclick = () => bulk({ visible: false });
  $("img-b-place").onchange = async () => {
    const v = $("img-b-place").value;
    $("img-b-place").value = "";
    if (v) await bulk({ placement: v });
  };
  $("img-b-op").oninput = () => {
    for (const e of list) if (picked.has(e.id) && map.getLayer(layerId(e.id))) map.setPaintProperty(layerId(e.id), "raster-opacity", opacityOf(e, Number($("img-b-op").value) / 100));
  };
  $("img-b-op").onchange = () => bulk({ opacity: Number($("img-b-op").value) / 100 });
  $("img-b-del").onclick = async () => {
    const sel = list.filter((e) => picked.has(e.id));
    if (!sel.length) return;
    const names = sel.slice(0, 5).map((e) => e.name).join(", ") + (sel.length > 5 ? ` and ${sel.length - 5} more` : "");
    const linked = sel.filter((e) => e.linked).length;
    const what = linked === sel.length
      ? "Their files are left where they are, untouched."
      : linked
        ? "TweedPlot's copies in the images folder are deleted; images used in place keep their files, untouched (as do the files copies were made from)."
        : "TweedPlot's copies in the images folder are deleted (the files you imported them from are not).";
    const ok = await ask({ title: `Remove ${sel.length} image${sel.length === 1 ? "" : "s"}`, note: `Remove ${names} from TweedPlot? ${what}`, ok: "Remove" });
    if (!ok) return;
    const errors = [];
    for (const e of sel) {
      try { await invoke("images_remove", { id: e.id }); picked.delete(e.id); } catch (err) { errors.push(`${e.name}: ${err}`); }
    }
    if (errors.length) toast(errors.join("; "), 6000);
    await refresh();
  };

  // ---- importing: copied in, or catalogued where they are ----
  let importing = false;
  let lastHow = "copy", lastWhat = "files";
  try {
    const v = JSON.parse(localStorage.getItem("tweedplot.images.import") || "{}");
    if (v.how === "copy" || v.how === "inPlace") lastHow = v.how;
    if (v.what === "files" || v.what === "folder") lastWhat = v.what;
  } catch { /* storage unavailable */ }
  $("img-import").onclick = async () => {
    if (importing) return;
    const v = await ask({
      title: "Add KAP charts or GeoTIFFs",
      note: "Copy in: TweedPlot keeps its own copy in its images folder, so the images stay when a USB stick or drive is taken away. "
        + "Use where they are: nothing is copied. Good for charts already installed for another program (OpenCPN and others) or kept on a large drive. "
        + "They show while that folder can be reached, and removing them from TweedPlot leaves the files alone.",
      fields: [
        { key: "how", label: "How", type: "select", value: lastHow, options: [["copy", "Copy into TweedPlot's images folder"], ["inPlace", "Use them where they are (catalogue only)"]] },
        { key: "what", label: "Pick", type: "select", value: lastWhat, options: [["files", "Files…"], ["folder", "A folder, with the folders inside it…"]] },
      ],
      ok: "Choose…",
    });
    if (!v) return;
    lastHow = v.how; lastWhat = v.what;
    try { localStorage.setItem("tweedplot.images.import", JSON.stringify({ how: lastHow, what: lastWhat })); } catch { /* ignore */ }
    importing = true;
    const btn = $("img-import");
    btn.disabled = true;
    let r;
    try { r = await invoke("images_import", { how: v.how, folder: v.what === "folder" }); } catch (err) { toast(String(err), 7000); }
    btn.disabled = false;
    importing = false;
    if (!r) return;
    const msgs = [];
    const n = r.added?.length ?? 0;
    const verb = v.how === "inPlace" ? "Catalogued" : "Added";
    if (n) msgs.push(n === 1 ? `${verb} ${r.added[0].name}` : `${verb} ${n} images`);
    if (r.already?.length) msgs.push(`${r.already.length} already in the list`);
    if (r.notGeo) msgs.push(`${r.notGeo} TIFF${r.notGeo === 1 ? " isn't" : "s aren't"} georeferenced (skipped)`);
    if (r.truncated) msgs.push("only the first 5,000 image files were looked at: pick a smaller folder for the rest");
    if (r.errors?.length) {
      const shown = r.errors.slice(0, 4).join("; ");
      msgs.push(`could not add ${r.errors.length}: ${shown}${r.errors.length > 4 ? "; …" : ""}`);
    }
    if (n) {
      folded = false;
      await refresh();
      // one image: go to it; many: go to the area they cover
      const bs = r.added.map((e) => lonLatBounds(e.bounds));
      const b = [Math.min(...bs.map((x) => x[0])), Math.min(...bs.map((x) => x[1])), Math.max(...bs.map((x) => x[2])), Math.max(...bs.map((x) => x[3]))];
      if (n === 1) showWhole(r.added[0]);
      else if (b[2] - b[0] < 90) map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: 40, maxZoom: 15 });
    }
    if (msgs.length) toast(msgs.join(". ") + ".", r.errors?.length || r.truncated ? 10000 : 4000);
  };
  // a folder being looked through: say how many files (cataloguing a big chart folder takes a while)
  listen("images-import-count", ({ payload: count }) => {
    toast(`Reading ${count.toLocaleString("en-NZ")} image file${count === 1 ? "" : "s"}…`, 4000);
  });
  // a linked drive plugged back in shows again when the window is next used
  window.addEventListener("focus", async () => {
    if (!list.some((e) => e.linked)) return;
    let now;
    try { now = await invoke("images_list"); } catch { return; }
    const was = new Map(list.map((e) => [e.id, !!e.missing]));
    if (now.length !== list.length || now.some((e) => was.get(e.id) !== !!e.missing)) { list = now; render(); syncMap(); }
  });

  // ---- Sentinel-2 ----
  const iso = (d) => d.toISOString().slice(0, 10);
  const today = new Date();
  $("img-s2-to").value = iso(today);
  $("img-s2-from").value = iso(new Date(today.getTime() - 365 * 86400e3));

  function areaNow() {
    const b = map.getBounds();
    const a = [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()];
    const lat = ((a[1] + a[3]) / 2) * Math.PI / 180;
    const w = (a[2] - a[0]) * 111.32 * Math.cos(lat), h = (a[3] - a[1]) * 111.32;
    return { a, w, h };
  }
  function showArea() {
    const n = $("img-s2-area");
    if (!n || $("img-s2").hidden) return;
    const { w, h } = areaNow();
    const big = w > 150 || h > 150;
    n.textContent = `Area: the chart on the screen, ${w.toFixed(w < 10 ? 1 : 0)} × ${h.toFixed(h < 10 ? 1 : 0)} km.${big ? " Too large: zoom in to the part you need." : ""}`;
    n.className = `wx-note${big ? " warn" : ""}`;
    $("img-s2-search").disabled = big;
  }
  map.on("moveend", showArea);
  $("img-s2-toggle").onclick = () => { $("img-s2").hidden = !$("img-s2").hidden; showArea(); };

  const fmtDate = (s) => { const d = new Date(s); return isNaN(d) ? s : d.toLocaleDateString("en-NZ", { day: "numeric", month: "short", year: "numeric" }); };
  function renderScenes() {
    const box = $("img-s2-results");
    box.replaceChildren();
    if (!scenes.length) return;
    for (const s of scenes.slice(0, 30)) {
      const row = document.createElement("div");
      row.className = "img-scene";
      const img = document.createElement("img");
      img.alt = ""; img.width = 56; img.height = 56; img.loading = "lazy";
      if (s.thumbnail) invoke("s2_thumbnail", { url: s.thumbnail }).then((u) => { img.src = u; }).catch(() => { img.classList.add("none"); });
      const t = document.createElement("div");
      t.className = "grow";
      const cloud = s.cloud == null ? "cloud ?" : `${s.cloud < 1 ? "<1" : Math.round(s.cloud)}% cloud`;
      t.innerHTML = "<b></b><div class=\"wx-note\"></div>";
      t.querySelector("b").textContent = `${fmtDate(s.datetime)} · ${cloud}`;
      t.querySelector(".wx-note").textContent = `${s.platform ? s.platform.replace("sentinel-", "Sentinel-").toUpperCase().replace("SENTINEL", "Sentinel") : "Sentinel-2"} · tile ${s.tile || "?"} · covers ${Math.round(s.cover * 100)}% of the area`;
      const get = document.createElement("button");
      get.type = "button"; get.textContent = "Get";
      get.title = "Download this scene for the area (cloud cover is for the scene's whole 110 km tile: look at the preview)";
      get.onclick = () => download(s);
      row.append(img, t, get);
      box.append(row);
    }
  }

  $("img-s2-search").onclick = async () => {
    const { a } = areaNow();
    area = a;
    const btn = $("img-s2-search");
    btn.disabled = true;
    const note = $("img-s2-status");
    note.textContent = "Searching Earth Search…"; note.className = "wx-note";
    try {
      scenes = await invoke("s2_search", { bbox: a, from: $("img-s2-from").value, to: $("img-s2-to").value, maxCloud: Number($("img-s2-cloud").value) });
      note.textContent = scenes.length ? `${scenes.length} scene${scenes.length === 1 ? "" : "s"}: most of the area and least cloud first.` : "No scenes: try a longer date range or more cloud.";
    } catch (err) {
      scenes = [];
      note.textContent = String(err); note.className = "wx-note bad";
    }
    btn.disabled = false;
    renderScenes();
    showArea();
  };

  let busy = false;
  async function download(s) {
    if (busy) return;
    busy = true;
    const prog = $("img-s2-prog"), bar = $("img-s2-bar"), note = $("img-s2-status");
    prog.hidden = false; bar.value = 0; bar.max = 1;
    note.textContent = `Downloading ${fmtDate(s.datetime)}…`; note.className = "wx-note";
    for (const b of $("img-s2-results").querySelectorAll("button")) b.disabled = true;
    try {
      const e = await invoke("s2_download", { scene: s, bbox: area, maxPx: Number($("img-s2-size").value) });
      note.textContent = `Added ${e.name} (${fmtRes(e.resM)} pixels).`; note.className = "wx-note good";
      await refresh();
    } catch (err) {
      note.textContent = String(err); note.className = "wx-note bad";
    }
    prog.hidden = true;
    for (const b of $("img-s2-results").querySelectorAll("button")) b.disabled = false;
    busy = false;
  }
  $("img-s2-stop").onclick = () => invoke("s2_stop").catch(() => {});
  listen("s2-progress", ({ payload: p }) => {
    const bar = $("img-s2-bar");
    if (bar && p.total) { bar.max = p.total; bar.value = p.done; }
  });

  refresh();
  return { refresh };
}
