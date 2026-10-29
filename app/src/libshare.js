// Library menu > Export…, and the choosing half of Merge from another library…: what to
// write from the library (marks, routes, tracks, trips, logs, routings, barriers and the own
// soundings), all into one library file, and, from a library file, what to bring in (the
// file is looked into first, so the dialog shows what it holds).

import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
import { ask } from "./ask.js";

const CATS = [
  { key: "marks", label: "Marks", title: "Marks and waypoints" },
  { key: "routes", label: "Routes", title: "Routes, with the marks they use (those come too, even without Marks ticked)" },
  { key: "tracks", label: "Tracks", title: "Recorded and imported tracks" },
  { key: "trips", label: "Trips", title: "The trips list (a trip's marks, tracks and logs are found by its dates)" },
  { key: "events", label: "Logs", title: "The fishing log: shots, sets, hauls, catches and notes" },
  { key: "routings", label: "Routings", title: "Weather routings and their results" },
  { key: "barriers", label: "Barriers", title: "Lines and areas the routing keeps out of" },
];
const N = (k) => k.toLocaleString();
const noun = (k, one, many = `${one}s`) => `${N(k)} ${k === 1 ? one : many}`;
const SOUNDINGS_TITLE = "Own soundings: your depth soundings, as the seabed built from them (not the tracks they came from)";

/** Write what the user chooses as one library file (and the own soundings, if wanted, in it or
 *  as an XYZ file beside it). `current` is the library in use ({ name, path }). */
export async function exportDialog(current) {
  let have;
  try { have = await invoke("library_file_contents", { path: current.path }); } catch (e) { activity.start("Export").fail(String(e)); return; }
  const snd = await invoke("pbathy_status").then((s) => ({ cells: s.info.cells, left: s.info.rejected })).catch(() => ({ cells: 0, left: 0 }));
  if (!CATS.some((c) => have[c.key]) && !snd.cells) { activity.note("Nothing to export", { details: ["The library and the own soundings are empty."] }); return; }
  const o = await ask({
    title: "Export from the library",
    note: "Tick what to write. It all goes into one library file, which can be looked into and chosen from again when it's brought into another library (Library > Merge from another library).",
    fields: [
      ...CATS.map((c) => ({ key: c.key, label: `${c.label} (${N(have[c.key])})`, type: "checkbox", value: have[c.key] > 0, disabled: !have[c.key], title: c.title })),
      { key: "soundings", label: `Own soundings (${N(snd.cells)} cells)`, type: "checkbox", value: snd.cells > 0, disabled: !snd.cells, title: SOUNDINGS_TITLE },
      { key: "format", label: "Own soundings as", type: "select", showIf: "soundings", options: [["file", "In the library file (nothing lost)"], ["xyz", "A separate XYZ text file (for other software)"]], title: "XYZ is a line of longitude, latitude and height for each cell: other software reads it, but it keeps no counts or what was left out" },
      { key: "notes", label: "Include notes", type: "checkbox", value: true, title: "Untick to leave out the notes (catch details and so on)" },
      { key: "protect", label: "Protect the library file with a passphrase", type: "checkbox", value: false, title: "Without the passphrase the file can't be opened or read. An XYZ file can't be protected." },
      { key: "pass", label: "Passphrase (at least 6 characters)", type: "password", showIf: "protect" },
      { key: "pass2", label: "The passphrase again", type: "password", showIf: "protect" },
    ],
    ok: "Choose where…",
    check: (v) => (!CATS.some((c) => v[c.key]) && !v.soundings ? "Tick something to export."
      : !v.protect ? "" : v.pass.length < 6 ? "The passphrase must be at least 6 characters." : v.pass !== v.pass2 ? "The two passphrases aren't the same." : ""),
  });
  if (!o) return;
  const lib = CATS.some((c) => o[c.key]);
  const xyz = o.soundings && o.format === "xyz";
  const inFile = o.soundings && !xyz;
  const makeFile = lib || inFile;
  const protect = o.protect && makeFile;

  let file = null;
  let xyzPath = null;
  if (makeFile) {
    file = await invoke("library_pick_db", { save: true, protected: protect, title: `Export from the library${protect ? " (protected)" : ""}`, suggest: `Library export.${protect ? "tplib" : "gpkg"}` }).catch(() => null);
    if (!file) return;
    if (xyz) xyzPath = file.replace(/\.[^.\\/]+$/, "") + " soundings.xyz";
  } else {
    xyzPath = await invoke("pbathy_pick_xyz_save", { suggest: "Own soundings.xyz" }).catch(() => null);
    if (!xyzPath) return;
  }

  const cancel = (id) => invoke("job_cancel", { job: id });
  const details = [];
  const parts = [];
  if (makeFile) {
    const job = activity.start("Exporting from the library", { params: file, cancel });
    try {
      const ids = async (flag, cmd) => (flag ? (await invoke(cmd)).map((x) => x.id) : []);
      const [marks, routes, tracks, events] = await Promise.all([ids(o.marks, "marks_list"), ids(o.routes, "routes_list"), ids(o.tracks, "tracks_list"), ids(o.events, "events_list")]);
      const n = await invoke("library_export", {
        path: file, marks, routes, tracks, events, notes: o.notes, passphrase: protect ? o.pass : null,
        trips: o.trips, routings: o.routings, barriers: o.barriers, soundings: inFile, job: job.id,
      });
      parts.push(
        n.marks && noun(n.marks, "mark"), n.routes && noun(n.routes, "route"), n.tracks && noun(n.tracks, "track"), n.trips && noun(n.trips, "trip"),
        n.events && noun(n.events, "log entry", "log entries"), n.routings && noun(n.routings, "routing"), n.barriers && noun(n.barriers, "barrier"),
        n.soundings && `${N(n.soundings)} cells of own soundings`,
      );
      details.push(`Written to ${file}`);
      if (!o.notes) details.push("Notes left out.");
      if (protect) details.push("Protected: it needs the passphrase to open. Give the passphrase separately (not in the same email).");
      job.done(`${parts.filter(Boolean).join(", ") || "Nothing"} written.`, { details: [...details, "Bring it into another library with Library > Merge from another library: it shows what's in the file to choose from."] });
    } catch (e) { job.fail(String(e)); return; }
  }
  if (xyz || (xyzPath && !makeFile)) {
    const job = activity.start("Writing own soundings as XYZ", { params: xyzPath, cancel });
    try {
      const lines = await invoke("pbathy_export_xyz", { path: xyzPath, job: job.id });
      job.done(`${N(lines)} soundings written.`, { details: [`Written to ${xyzPath}`, "One line per cell: longitude, latitude, height in metres (negative down). Not the counts, or the soundings the checks left out."] });
    } catch (e) { job.fail(String(e)); }
  }
}

/** Look into a library file (already unlocked) and ask what to bring in. Resolves to
 *  { scope, any, soundings } (scope: the categories of `library_merge`; any: whether one is
 *  ticked; soundings: whether to bring in the own soundings), or null if cancelled. */
export async function chooseFromLibraryFile(path) {
  const name = path.split(/[\\/]/).pop();
  let c;
  try { c = await invoke("library_file_contents", { path }); } catch (e) { activity.start(`Reading ${name}`).fail(`Not a TweedPlot library, or it can't be read: ${e}`); return null; }
  const cells = c.soundings?.cells ?? 0;
  const left = c.soundings?.left_out ?? 0;
  if (!CATS.some((x) => c[x.key]) && !cells) { activity.note(`${name} is empty`, { details: ["There is nothing in it to bring in."] }); return null; }
  const o = await ask({
    title: `Bring in from ${name}`,
    note: "This is what the file holds. Tick what to bring in; what's already in this library is matched and left alone.",
    fields: [
      ...CATS.map((x) => ({ key: x.key, label: `${x.label} (${N(c[x.key])})`, type: "checkbox", value: c[x.key] > 0, disabled: !c[x.key], title: x.title })),
      { key: "soundings", label: `Own soundings (${N(cells)} cells${left ? `, and ${N(left)} left out` : ""})`, type: "checkbox", value: cells > 0, disabled: !cells, title: `${SOUNDINGS_TITLE}. They are added to the own soundings you have: bringing the same file in again is stopped.` },
    ],
    ok: "Bring in",
    check: (v) => (CATS.some((x) => v[x.key]) || v.soundings ? "" : "Tick something to bring in."),
  });
  if (!o) return null;
  const scope = Object.fromEntries(CATS.map((x) => [x.key, !!o[x.key]]));
  return { scope, any: Object.values(scope).some(Boolean), soundings: !!o.soundings };
}

/** Bring the own soundings of a library file into the own soundings. */
export async function importSoundings(path) {
  const name = path.split(/[\\/]/).pop();
  const job = activity.start(`Bringing in own soundings from ${name}`, { params: path, cancel: (id) => invoke("job_cancel", { job: id }) });
  try {
    const r = await invoke("pbathy_import_from_library", { path, job: job.id });
    if (r.already) { job.done("Nothing new: the own soundings in this file were brought in before."); return; }
    job.done(`${N(r.added)} cells of own soundings added${r.left_out ? `, ${N(r.left_out)} left out` : ""}.`, { details: ["Chart > Chart data > Own soundings shows them."] });
  } catch (e) { job.fail(String(e)); }
}
