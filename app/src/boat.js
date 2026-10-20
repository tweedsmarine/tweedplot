// Boat panel: polar, efficiency, manoeuvre costs (kept per PC).
// Future home of the polar view and polar tuning (with a dock view for the plot).
import { invoke } from "@tauri-apps/api/core";

const KEY = "racenav.boat";
const DEFAULTS = { polar: "", eff: 100, tack: 60, gybe: 60, airDraft: 0, mmsi: "",
  // size and GPS antenna position (m): from the bow, and off the centre line (+ starboard, - port)
  length: 0, beam: 0, gpsFromBow: 0, gpsFromCentre: 0 };

export function setupBoat() {
  const $ = (id) => document.getElementById(id);
  let boat = { ...DEFAULTS };
  try { Object.assign(boat, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(boat)); } catch { /* ignore */ } };

  async function loadPolars() {
    const list = (await invoke("polar_files").catch(() => [])) || [];
    const sel = $("boat-polar");
    sel.innerHTML = "";
    if (!list.length) sel.append(new Option("No polars found", ""));
    for (const p of list) sel.append(new Option(p.source === "shipped" ? `${p.name} (shipped)` : p.name, p.path));
    if (boat.polar && list.some((p) => p.path === boat.polar)) sel.value = boat.polar;
    boat.polar = sel.value;
    save();
  }
  $("boat-polar").onchange = (e) => { boat.polar = e.target.value; save(); };
  const note = (t, cls = "") => { const n = $("boat-polar-note"); n.textContent = t; n.className = `wx-note ${cls}`; };
  $("boat-polar-import").onclick = () => $("boat-polar-file").click();
  $("boat-polar-file").onchange = async (e) => {
    const f = e.target.files?.[0];
    e.target.value = "";
    if (!f) return;
    try {
      const path = await invoke("polar_import", { name: f.name, text: await f.text() });
      boat.polar = path;
      save();
      await loadPolars();
      note(`Imported to ${path}`, "good");
    } catch (err) { note(String(err), "bad"); }
  };
  $("boat-polar-folder").onclick = async () => {
    try { note(`Your polars: ${await invoke("polar_open_folder")}`); } catch (err) { note(`Could not open the folder: ${err}`, "bad"); }
  };
  for (const [id, key] of [["boat-eff", "eff"], ["boat-tack", "tack"], ["boat-gybe", "gybe"], ["boat-air", "airDraft"]]) {
    $(id).value = boat[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v)) { boat[key] = v; save(); window.dispatchEvent(new Event("racenav-boat")); } };
  }
  for (const [id, key] of [["boat-len", "length"], ["boat-beam", "beam"], ["boat-gps-bow", "gpsFromBow"]]) {
    $(id).value = boat[key] || "";
    $(id).onchange = (e) => { const v = Math.max(0, Number(e.target.value) || 0); boat[key] = v; e.target.value = v || ""; save(); window.dispatchEvent(new Event("racenav-boat")); };
  }
  $("boat-gps-side").value = Math.abs(boat.gpsFromCentre) || "";
  $("boat-gps-dir").value = boat.gpsFromCentre < 0 ? "-1" : "1";
  const side = () => {
    const v = Math.max(0, Number($("boat-gps-side").value) || 0);
    boat.gpsFromCentre = v * Number($("boat-gps-dir").value);
    save();
    window.dispatchEvent(new Event("racenav-boat"));
  };
  $("boat-gps-side").onchange = side;
  $("boat-gps-dir").onchange = side;
  $("boat-mmsi").value = boat.mmsi || "";
  $("boat-mmsi").onchange = (e) => {
    const v = e.target.value.replace(/\D/g, "");
    boat.mmsi = v.length === 9 ? v : "";
    e.target.value = boat.mmsi;
    save();
    window.dispatchEvent(new Event("racenav-boat"));
  };
  loadPolars();
  return { get: () => ({ ...boat }) };
}
