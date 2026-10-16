// GRIB download (Weather drawer > Download): model, parameters, area (drawn on
// the chart, the chart view, or round the course), forecast length and step.
// The Rust side finds the newest complete run on NOAA NOMADS, downloads it cut
// to the area, and the file is loaded as soon as it is written.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const KEY = "racenav.gribdl";
const DEFAULTS = { model: "gfs0p25", params: { gfs: ["wind", "gust", "mslp"], wave: ["waves", "period"] }, area: null, days: 7, every: 3 };

export function setupGribDownload(map, { weather, course }) {
  const $ = (id) => document.getElementById(id);
  let prefs = structuredClone(DEFAULTS);
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ } };
  let models = [];
  const status = (t, kind = "") => { const el = $("dl-status"); el.textContent = t; el.className = `wx-note ${kind}`; };
  const model = () => models.find((m) => m.id === prefs.model) ?? models[0];

  // ---------- the area on the chart ----------------------------------------------------------
  const empty = { type: "FeatureCollection", features: [] };
  map.addSource("dl-area", { type: "geojson", data: empty });
  map.addLayer({ id: "dl-area-fill", type: "fill", source: "dl-area", paint: { "fill-color": "#0b5cad", "fill-opacity": 0.06 } });
  map.addLayer({ id: "dl-area-line", type: "line", source: "dl-area", paint: { "line-color": "#0b5cad", "line-width": 2, "line-dasharray": [4, 2] } });
  function showArea() {
    const a = prefs.area;
    const open = $("dl-box").open && !$("drawer").hidden;
    map.getSource("dl-area").setData(a && open ? { type: "Feature", properties: {}, geometry: { type: "Polygon",
      coordinates: [[[a[0], a[1]], [a[2], a[1]], [a[2], a[3]], [a[0], a[3]], [a[0], a[1]]]] } } : empty);
    for (const [id, i] of [["dl-w", 0], ["dl-s", 1], ["dl-e", 2], ["dl-n", 3]]) $(id).value = a ? String(+a[i].toFixed(2)) : "";
  }
  function setArea(a) {
    if (!a) return;
    // tidy: whole tenths, south < north, and east after west (may pass 180 to cross the date line)
    let [w, s, e, n] = a.map((x) => Math.round(x * 10) / 10);
    if (s > n) [s, n] = [n, s];
    s = Math.max(-90, s); n = Math.min(90, n);
    if (e <= w) e += 360;
    prefs.area = [w, s, e, n];
    save();
    showArea();
    estimate();
  }
  $("dl-box").addEventListener("toggle", showArea);
  new MutationObserver(showArea).observe($("drawer"), { attributes: true, attributeFilter: ["hidden"] });

  $("dl-view").onclick = () => {
    const b = map.getBounds();
    setArea([b.getWest(), b.getSouth(), b.getEast(), b.getNorth()]);
  };
  $("dl-course").onclick = () => {
    const c = course.get();
    const pts = [c.start, ...c.marks.map((m) => [m.lon, m.lat])].filter(Boolean);
    if (!pts.length) { status("Set a course first (Course panel).", "warn"); return; }
    // keep longitudes continuous so a course across the date line gives a small box
    const lons = [];
    for (const p of pts) { let x = p[0]; if (lons.length) { while (x - lons.at(-1) > 180) x -= 360; while (x - lons.at(-1) < -180) x += 360; } lons.push(x); }
    const lats = pts.map((p) => p[1]);
    const span = Math.max(Math.max(...lons) - Math.min(...lons), Math.max(...lats) - Math.min(...lats));
    const m = Math.max(1, Math.min(10, span * 0.3));
    setArea([Math.min(...lons) - m, Math.min(...lats) - m, Math.max(...lons) + m, Math.max(...lats) + m]);
    fitArea();
  };
  for (const id of ["dl-w", "dl-s", "dl-e", "dl-n"]) {
    $(id).onchange = () => {
      const v = ["dl-w", "dl-s", "dl-e", "dl-n"].map((k) => parseFloat($(k).value));
      if (v.every(Number.isFinite)) setArea(v); else status("Type the four edges in degrees (south and west negative).", "warn");
    };
  }
  // draw: drag a rectangle
  let drawing = false, from = null;
  $("dl-draw").onclick = () => {
    drawing = !drawing;
    $("dl-draw").classList.toggle("armed", drawing);
    map.getCanvas().style.cursor = drawing ? "crosshair" : "";
    if (drawing) { map.dragPan.disable(); status("Drag a rectangle on the chart."); } else map.dragPan.enable();
  };
  map.on("mousedown", (e) => { if (drawing) { from = e.lngLat; e.preventDefault(); } });
  let to = null;
  map.on("mousemove", (e) => {
    if (!drawing || !from) return;
    to = e.lngLat;
    prefs.area = [Math.min(from.lng, to.lng), Math.min(from.lat, to.lat), Math.max(from.lng, to.lng), Math.max(from.lat, to.lat)];
    showArea();
  });
  // finish on mouse up anywhere (it may be released over a panel)
  window.addEventListener("mouseup", () => {
    if (!drawing || !from || !to) return;
    const a = [Math.min(from.lng, to.lng), Math.min(from.lat, to.lat), Math.max(from.lng, to.lng), Math.max(from.lat, to.lat)];
    to = null;
    from = null;
    drawing = false;
    $("dl-draw").classList.remove("armed");
    map.getCanvas().style.cursor = "";
    map.dragPan.enable();
    if (a[2] - a[0] < 0.2 || a[3] - a[1] < 0.2) { status("That area is too small: drag a larger rectangle.", "warn"); showArea(); return; }
    setArea(a);
    status("");
  });
  function fitArea() {
    const a = prefs.area;
    if (a) map.fitBounds([[a[0], a[1]], [a[2], a[3]]], { padding: { top: 60, bottom: 80, left: 440, right: 80 }, duration: 500 });
  }

  // ---------- model, parameters, time ----------------------------------------------------------
  function renderParams() {
    const m = model();
    const key = m.id === "gfs_wave0p25" ? "wave" : "gfs";
    const box = $("dl-params");
    box.innerHTML = "";
    for (const p of m.params) {
      const lab = document.createElement("label");
      lab.innerHTML = `<input type="checkbox"> <span></span>`;
      lab.querySelector("span").textContent = p.name;
      const cb = lab.querySelector("input");
      cb.checked = prefs.params[key].includes(p.id);
      cb.onchange = () => {
        const s = new Set(prefs.params[key]);
        if (cb.checked) s.add(p.id); else s.delete(p.id);
        prefs.params[key] = [...s];
        save(); estimate();
      };
      box.append(lab);
    }
    const days = $("dl-days");
    days.innerHTML = "";
    for (const d of [1, 2, 3, 5, 7, 10, 16]) if (d * 24 <= m.max_hours) days.append(new Option(`${d} day${d > 1 ? "s" : ""}`, String(d)));
    days.value = String(Math.min(prefs.days, 16));
    if (!days.value) days.value = "7";
    const ev = $("dl-every");
    ev.innerHTML = "";
    const fine = m.steps[0][1];
    for (const h of [1, 3, 6, 12]) if (h >= fine) ev.append(new Option(h === 1 ? "1 hour" : `${h} hours`, String(h)));
    ev.value = String(Math.max(prefs.every, fine));
    if (!ev.value) ev.value = String(fine);
  }
  function request() {
    const m = model();
    const key = m.id === "gfs_wave0p25" ? "wave" : "gfs";
    return { model: m.id, area: prefs.area, params: prefs.params[key], hours: Number($("dl-days").value) * 24, every: Number($("dl-every").value) };
  }
  async function estimate() {
    if (!models.length) return;
    if (!prefs.area) { $("dl-size").textContent = "Choose an area: draw it, use the chart view or the course."; return; }
    try {
      const r = await invoke("gribdl_estimate", { req: request() });
      const mb = r.bytes / 1e6;
      $("dl-size").textContent = `About ${mb < 1 ? `${Math.max(1, Math.round(r.bytes / 1000))} kB` : `${mb.toFixed(mb < 10 ? 1 : 0)} MB`} · ${r.steps} forecast times to +${r.last} h`;
    } catch (e) { $("dl-size").textContent = String(e); }
  }
  $("dl-model").onchange = (e) => { prefs.model = e.target.value; save(); renderParams(); estimate(); };
  $("dl-days").onchange = (e) => { prefs.days = Number(e.target.value); save(); estimate(); };
  $("dl-every").onchange = (e) => { prefs.every = Number(e.target.value); save(); estimate(); };

  // ---------- download ----------------------------------------------------------
  const prog = $("dl-progress");
  listen("gribdl-progress", ({ payload: p }) => {
    prog.hidden = false;
    const pc = p.total ? Math.round((100 * p.done) / p.total) : 2;
    prog.querySelector(".bar").style.width = `${pc}%`;
    prog.querySelector("span").textContent = p.total ? `${p.stage} · ${p.done}/${p.total} · ${(p.bytes / 1e6).toFixed(1)} MB` : `${p.stage}…`;
  }).catch(() => {});
  $("dl-go").onclick = async () => {
    const req = request();
    if (!req.area) { status("Choose an area first.", "warn"); return; }
    if (!req.params.length) { status("Tick at least one parameter.", "warn"); return; }
    $("dl-go").disabled = true;
    $("dl-cancel").disabled = false;
    status("Downloading…");
    const t0 = performance.now();
    try {
      const r = await invoke("gribdl_start", { req });
      status(`Downloaded ${r.name} in ${((performance.now() - t0) / 1000).toFixed(0)} s. Loading…`, "good");
      await weather.refreshFiles();
      await weather.load(r.path);
      status(`${r.name}: downloaded and loaded (${r.path.split(/[\\/]/).pop()}).`, "good");
    } catch (e) {
      status(String(e) === "cancelled" ? "Download cancelled." : `Download failed: ${e}`, String(e) === "cancelled" ? "warn" : "bad");
    } finally {
      $("dl-go").disabled = false;
      $("dl-cancel").disabled = true;
      prog.hidden = true;
    }
  };
  $("dl-cancel").onclick = () => invoke("gribdl_cancel").catch(() => {});

  (async () => {
    models = (await invoke("gribdl_models").catch(() => [])) || [];
    const sel = $("dl-model");
    for (const m of models) sel.append(new Option(m.name, m.id));
    if (!models.some((m) => m.id === prefs.model)) prefs.model = models[0]?.id ?? "gfs0p25";
    sel.value = prefs.model;
    if (models.length) renderParams();
    showArea();
    estimate();
  })();
}
