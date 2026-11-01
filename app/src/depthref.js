// What the depths of a track are measured below (the surface, the transducer, the keel, or
// already reduced to chart datum), asked when a file doesn't say: after an import, before own
// soundings are built from tracks, and from the Tracks list. Own soundings use it to reduce
// each sounding to chart datum (with the tide; see pbathy.rs).
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./ask.js";
import * as activity from "./activity.js";

export const DEPTH_REF = {
  transducer: "below the transducer",
  surface: "below the surface",
  keel: "below the keel",
  datum: "reduced to chart datum",
  "": "reference not known",
};
const KEY = "racenav.depthref.last";

/** Ask what the depths are measured below. Resolves to { kind, offset } (offset null: the
 *  transducer depth or draft in Settings › Vessel), or null if cancelled. */
export async function askDepthRef({ title, note = "", current = null }) {
  let last = {};
  try { last = JSON.parse(localStorage.getItem(KEY) || "{}"); } catch { /* none */ }
  const s = await invoke("settings_get").catch(() => null);
  const tdep = s?.vessel?.transducer_depth_m, draft = s?.vessel?.draft_m;
  const fromSettings = [tdep != null ? `transducer ${tdep} m` : "transducer not set", draft != null ? `draft ${draft} m` : "draft not set"].join(", ");
  const kind0 = current?.kind || last.kind || "transducer";
  const off0 = current ? current.offset : last.offset;
  const v = await ask({
    title,
    note: `${note}${note ? " " : ""}Own soundings use this, with the tide, to reduce the depths to chart datum. You can change it later in Library › Tracks (Depth reference…).`,
    fields: [
      { key: "kind", label: "The depths are measured", type: "select", value: kind0, options: [
        ["transducer", "Below the transducer (most sounders)"],
        ["surface", "Below the surface (waterline)"],
        ["keel", "Below the keel"],
        ["datum", "Already reduced to chart datum (as TimeZero's)"],
      ] },
      { key: "offset", label: `Transducer depth, or the draft for below the keel (m). Empty: Settings › Vessel (${fromSettings})`, type: "text", value: off0 != null ? String(off0) : "", placeholder: "from Settings › Vessel" },
    ],
    ok: "Set",
    check: (x) => {
      const t = x.offset.trim();
      if (t && !(Number.isFinite(Number(t)) && Number(t) >= 0 && Number(t) < 30)) return "The offset is a number of metres (0 to 30), or empty.";
      return "";
    },
  });
  if (!v) return null;
  const offset = v.offset.trim() === "" ? null : Number(v.offset);
  try { localStorage.setItem(KEY, JSON.stringify({ kind: v.kind, offset })); } catch { /* ignore */ }
  return { kind: v.kind, offset };
}

/** Set the depth reference of tracks `ids`. Resolves to the number changed. */
export async function setDepthRef(ids, r) {
  return invoke("tracks_set_depth_ref", { ids, kind: r.kind, offset: r.offset });
}

/** After an import: ask about the new tracks whose depths don't say what they're below. */
export async function askAfterImport() {
  const ids = await invoke("tracks_depth_unknown", { newestImport: true }).catch(() => []);
  if (!ids.length) return;
  const n = ids.length;
  const r = await askDepthRef({
    title: `Depths in the imported track${n === 1 ? "" : "s"}`,
    note: `${n.toLocaleString()} imported track${n === 1 ? " has" : "s have"} depths, but the file doesn't say what they are measured from.`,
  });
  if (!r) { activity.note("Depth reference not set", { details: ["Own soundings will ask again before using these tracks."] }); return; }
  try {
    await setDepthRef(ids, r);
    activity.note(`Depths of ${n.toLocaleString()} track${n === 1 ? "" : "s"}: ${DEPTH_REF[r.kind]}`);
  } catch (e) { activity.start("Setting the depth reference").fail(String(e)); }
}
