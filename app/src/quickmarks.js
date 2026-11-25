// Number-key marks: keys 1 to 0 each drop a mark at the boat in one press, no dialog, with the
// symbol, colour and name set for that key in Settings › Number-key marks. Every use (Cruising,
// Fishing, Racing) has them. A key with nothing set does nothing but say so.
// Saved per computer (localStorage); key 1 starts as a filled triangle, the others blank.

import { invoke } from "@tauri-apps/api/core";
import { MARK_SYMBOLS, SYMBOL_KINDS, symImage, symbolColour } from "./marksymbols.js";
import { iconSelect, imageUrl } from "./iconselect.js";

const KEY = "racenav.quickmarks";
export const QUICK_KEYS = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const GROUP = "Number-key marks";

/** Ten slots, key 1 first: { kind, colour, name } or null. */
export function defaultSlots() {
  return QUICK_KEYS.map((k) => (k === "1" ? { kind: "sym-tri-up-f", colour: "", name: "" } : null));
}

/** Slots read back from storage, anything unknown dropped. */
export function cleanSlots(v) {
  const d = defaultSlots();
  if (!Array.isArray(v)) return d;
  return QUICK_KEYS.map((_, i) => {
    const s = v[i];
    if (!s || !SYMBOL_KINDS[s.kind]) return null;
    return { kind: s.kind, colour: /^#[0-9a-fA-F]{6}$/.test(s.colour ?? "") ? s.colour : "", name: String(s.name ?? "").slice(0, 60) };
  });
}

/** The name a dropped mark gets: its own name, or the symbol's, then the time (so marks dropped in turn differ). */
export function markName(slot, when) {
  const t = when.toLocaleTimeString("en-NZ", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  return `${slot.name.trim() || SYMBOL_KINDS[slot.kind]} ${t}`;
}

/** The slot index for a keydown (top row or keypad 1–0), or -1. */
export function slotForKey(e) {
  const m = /^(?:Digit|Numpad)(\d)$/.exec(e.code ?? "");
  if (!m) return -1;
  return m[1] === "0" ? 9 : Number(m[1]) - 1;
}

export function setupQuickMarks({ getBoat, toast, refresh }) {
  let slots;
  try { slots = cleanSlots(JSON.parse(localStorage.getItem(KEY) || "null")); } catch { slots = defaultSlots(); }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(slots)); } catch { /* storage unavailable */ } };

  async function drop(i) {
    const key = QUICK_KEYS[i];
    const s = slots[i];
    if (!s) { toast(`Key ${key} has no mark set: Settings › Number-key marks`, 4000); return; }
    const p = getBoat();
    if (!p) { toast("No boat position: no mark dropped", 3000); return; }
    const when = new Date();
    try {
      await invoke("mark_save", { mark: { name: markName(s, when), lat: p[1], lon: p[0], kind: s.kind, colour: s.colour, group: GROUP,
        created: Math.floor(when.getTime() / 1000), notes: `Dropped with key ${key}, ${when.toISOString()}` } });
      toast(`${s.name.trim() || SYMBOL_KINDS[s.kind]} dropped (key ${key})`, 2500);
      refresh();
    } catch (err) {
      toast(`Could not drop the mark: ${err}`, 6000);
    }
  }

  // ---------- Settings › Number-key marks ------------------------------------------------
  const host = document.getElementById("quickmarks-settings");
  if (host) {
    const icon = (kind, colour) => (kind ? imageUrl(symImage(`sym:${kind}:${colour || ""}`)) : "");
    host.innerHTML = "";
    QUICK_KEYS.forEach((key, i) => {
      const row = document.createElement("div");
      row.className = "qm-row";
      row.innerHTML = `<kbd class="qm-key">${key}</kbd><select class="qm-kind" aria-label="Symbol for key ${key}"></select>
        <input class="qm-colour" type="color" aria-label="Colour for key ${key}" title="Colour (the symbol's own colour until changed)">
        <input class="qm-name" type="text" maxlength="60" aria-label="Name for key ${key}" placeholder="Name">`;
      const sel = row.querySelector(".qm-kind");
      sel.append(new Option("— none —", ""));
      const groups = new Map();
      for (const [k, label, grp] of MARK_SYMBOLS) {
        if (!groups.has(grp)) { const og = document.createElement("optgroup"); og.label = grp; groups.set(grp, og); sel.append(og); }
        groups.get(grp).append(new Option(label, k));
      }
      const col = row.querySelector(".qm-colour"), name = row.querySelector(".qm-name");
      const s = slots[i];
      sel.value = s?.kind ?? "";
      const picker = iconSelect(sel, (v) => icon(v, slots[i]?.colour));
      const show = () => {
        const cur = slots[i];
        col.disabled = name.disabled = !cur;
        col.value = cur ? cur.colour || symbolColour(cur.kind) : "#888888";
        name.value = cur?.name ?? "";
        name.placeholder = cur ? SYMBOL_KINDS[cur.kind] : "Name";
      };
      show();
      sel.onchange = () => {
        slots[i] = sel.value ? { kind: sel.value, colour: slots[i]?.colour ?? "", name: slots[i]?.name ?? "" } : null;
        save(); show(); picker?.refresh?.();
      };
      col.onchange = () => { if (!slots[i]) return; slots[i].colour = col.value; save(); picker?.refresh?.(true); };
      name.onchange = () => { if (!slots[i]) return; slots[i].name = name.value.trim(); save(); };
      host.append(row);
    });
  }

  return { drop };
}
