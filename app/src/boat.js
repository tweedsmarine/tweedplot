// Routing › Boat and polar: polar, efficiency, manoeuvre costs (kept per PC).
// Future home of the polar view and polar tuning (with a dock view for the plot).
import { invoke } from "@tauri-apps/api/core";

const KEY = "racenav.boat";
const DEFAULTS = { polar: "", eff: 100, tack: 60, gybe: 60, airDraft: 0, mmsi: "",
  // Legacy profile fields are read once when migrating into Settings.
  length: 0, beam: 0, gpsFromBow: 0, gpsFromCentre: 0,
  name: "", imo: "", callSign: "", draft: 0, referenceLat: null, referenceLon: null };

export function setupBoat() {
  const $ = (id) => document.getElementById(id);
  let boat = { ...DEFAULTS };
  try { Object.assign(boat, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ polar: boat.polar, eff: boat.eff, tack: boat.tack, gybe: boat.gybe }));
    } catch { /* ignore */ }
  };

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
  for (const [id, key] of [["boat-eff", "eff"], ["boat-tack", "tack"], ["boat-gybe", "gybe"]]) {
    $(id).value = boat[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v)) { boat[key] = v; save(); window.dispatchEvent(new Event("racenav-boat")); } };
  }
  loadPolars();
  window.addEventListener("racenav-vessel-profile", ({ detail }) => {
    boat = { ...boat, ...detail };
    window.dispatchEvent(new Event("racenav-boat"));
  });
  return { get: () => ({ ...boat }) };
}
