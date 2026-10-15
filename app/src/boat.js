// Boat panel: polar, efficiency, manoeuvre costs (kept per PC).
// Future home of the polar view and polar tuning (with a dock view for the plot).
import { invoke } from "@tauri-apps/api/core";

const KEY = "racenav.boat";
const DEFAULTS = { polar: "", eff: 100, tack: 60, gybe: 60 };

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
    for (const p of list) sel.append(new Option(p.name, p.path));
    if (boat.polar && list.some((p) => p.path === boat.polar)) sel.value = boat.polar;
    boat.polar = sel.value;
    save();
  }
  $("boat-polar").onchange = (e) => { boat.polar = e.target.value; save(); };
  for (const [id, key] of [["boat-eff", "eff"], ["boat-tack", "tack"], ["boat-gybe", "gybe"]]) {
    $(id).value = boat[key];
    $(id).onchange = (e) => { const v = Number(e.target.value); if (Number.isFinite(v)) { boat[key] = v; save(); } };
  }
  loadPolars();
  return { get: () => ({ ...boat }) };
}
