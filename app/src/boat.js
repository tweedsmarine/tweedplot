// Routing › Boat and polar: polar, efficiency, manoeuvre costs (kept per PC).
// The polar's plot and table editor is polaredit.js (in the dock).
import { invoke } from "@tauri-apps/api/core";

const KEY = "racenav.boat";
const DEFAULTS = { polar: "", eff: 100, tack: 60, gybe: 60, airDraft: 0, mmsi: "",
  // laylines: shown or not, and the angles used when there is no polar (total angle between the two tacks / gybes, and the cone's half-angle)
  layShow: true, layConeShow: false, layTack: 120, layGybe: 60, layCone: 20,
  // Legacy profile fields are read once when migrating into Settings.
  length: 0, beam: 0, gpsFromBow: 0, gpsFromCentre: 0,
  name: "", imo: "", callSign: "", draft: 0, referenceLat: null, referenceLon: null };

export function setupBoat() {
  const $ = (id) => document.getElementById(id);
  let boat = { ...DEFAULTS };
  try { Object.assign(boat, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ polar: boat.polar, eff: boat.eff, tack: boat.tack, gybe: boat.gybe,
        layShow: boat.layShow, layConeShow: boat.layConeShow, layTack: boat.layTack, layGybe: boat.layGybe, layCone: boat.layCone }));
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
  $("boat-polar").onchange = (e) => { boat.polar = e.target.value; save(); window.dispatchEvent(new Event("racenav-boat")); };
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
  for (const [id, key] of [["boat-eff", "eff"], ["boat-tack", "tack"], ["boat-gybe", "gybe"], ["boat-lay-tack", "layTack"], ["boat-lay-gybe", "layGybe"], ["boat-lay-cone", "layCone"]]) {
    $(id).value = boat[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v)) { boat[key] = v; save(); window.dispatchEvent(new Event("racenav-boat")); } };
  }
  const showBoxes = { "boat-lay-show": "layShow", "boat-lay-cone-show": "layConeShow" };
  for (const [id, key] of Object.entries(showBoxes)) {
    $(id).checked = !!boat[key];
    $(id).onchange = (e) => { boat[key] = e.target.checked; save(); window.dispatchEvent(new Event("racenav-boat")); };
  }
  loadPolars();
  // a polar saved in the polar editor: the list again, and the saved copy used if it replaced the one in use
  window.addEventListener("racenav-polars", async ({ detail }) => {
    if (detail?.use && detail.path) boat.polar = detail.path;
    await loadPolars();
    window.dispatchEvent(new Event("racenav-boat"));
  });
  window.addEventListener("racenav-vessel-profile", ({ detail }) => {
    boat = { ...boat, ...detail };
    window.dispatchEvent(new Event("racenav-boat"));
  });
  return {
    get: () => ({ ...boat }),
    /** Turn laylines or the tacking cone on or off (the right-click menu): keeps the Settings checkboxes in step. */
    toggle(key) {
      boat[key] = !boat[key];
      for (const [id, k] of Object.entries(showBoxes)) if (k === key) $(id).checked = boat[key];
      save();
      window.dispatchEvent(new Event("racenav-boat"));
    },
  };
}
