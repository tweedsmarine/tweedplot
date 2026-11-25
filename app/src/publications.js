// Nautical publications the chart notes refer to: LINZ's New Zealand Nautical Almanac
// (NZ 204) and its Annual New Zealand Notices to Mariners (one PDF each). Downloaded from
// linz.govt.nz only when the mariner says so (Chart › Get data › Nautical publications, or the
// offer when a link in a chart note finds it missing), kept, and opened from this computer. A new edition comes out each 1 July.
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./ask.js";

const $ = (id) => document.getElementById(id);
let status = null;
let toast = () => {};

/** The season an edition covers: "2026/27" from 1 July 2026. */
function currentEdition(d = new Date()) {
  const y = d.getMonth() >= 6 ? d.getFullYear() : d.getFullYear() - 1;
  return `${y}/${String((y + 1) % 100).padStart(2, "0")}`;
}

async function refresh() {
  try { status = await invoke("pubs_status"); } catch { status = null; }
  draw();
  return status;
}

function draw() {
  const note = $("pub-note");
  if (!note) return;
  const ed = status?.edition;
  const n = status?.notices?.length || 0;
  const stale = ed && ed !== currentEdition();
  note.textContent = ed
    ? `${ed} edition: ${n} Annual Notices${status.almanac ? " and the full Almanac" : ""}.${stale ? ` The ${currentEdition()} edition is out: Update.` : ""}`
    : "Not downloaded. The chart notes refer to them (\"Annual New Zealand Notices to Mariners No. 19\").";
  note.classList.toggle("warn", !!stale);
  $("pub-get").textContent = ed ? "Update" : "Download";
  $("pub-open-alm").hidden = !status?.almanac;
  const ul = $("pub-list");
  ul.replaceChildren(...(status?.notices || []).map((x) => {
    const li = document.createElement("li");
    const a = document.createElement("a");
    a.href = "#";
    a.textContent = `No. ${x.no}`;
    a.onclick = (e) => { e.preventDefault(); openPublication(x.no); };
    li.append(a, ` ${x.title}`);
    return li;
  }));
  $("pub-list-d").hidden = !n;
}

async function download(almanac) {
  const b = $("pub-get");
  if (b) b.disabled = true;
  toast(`Downloading the Annual Notices to Mariners${almanac ? " and the Almanac" : ""} from LINZ…`, 4000);
  try {
    status = await invoke("pubs_download", { almanac });
    toast(`${status.edition} Annual Notices to Mariners downloaded (${status.notices.length})${status.almanac ? ", with the Almanac" : ""}`, 4000);
    return true;
  } catch (e) {
    toast(`Could not download from LINZ: ${e}`, 7000);
    return false;
  } finally {
    if (b) b.disabled = false;
    draw();
  }
}

/** Open an Annual Notice ("19", "4A") or the Almanac ("almanac") from this computer. When it
 *  isn't here, say so and offer to download it: nothing goes to the internet unless asked
 *  (on a boat the connection may be slow, costly or not there at all). */
export async function openPublication(no) {
  try { if (await invoke("pub_open", { no })) return; } catch (e) { toast(`Could not open it: ${e}`, 5000); return; }
  const alm = no === "almanac";
  const what = alm ? "The New Zealand Nautical Almanac (NZ 204)" : `Annual Notice to Mariners No. ${no}`;
  const v = await ask({
    title: `${what} isn't on this computer`,
    note: `TweedPlot opens LINZ's publications from this computer, so they work without an internet connection. ` +
      `Download ${alm ? "the Almanac (about 9 MB) and this year's Annual Notices (about 3 MB)" : "this year's Annual Notices (about 3 MB)"} from LINZ now? ` +
      "You can also do it later: Chart › Get data › Nautical publications.",
    ok: "Download", cancel: "Not now",
  });
  if (!v) return;
  if (!(await download(alm))) return;
  try {
    if (!(await invoke("pub_open", { no }))) {
      toast(alm ? "LINZ's page had no Almanac PDF TweedPlot recognises; the Annual Notices were downloaded" : `There is no Annual Notice No. ${no} in this edition`, 7000);
    }
  } catch (e) { toast(`Could not open it: ${e}`, 5000); }
}

const NOTICE_RE = /Annual\s+(?:New\s+Zealand\s+)?Notices?\s+to\s+Mariners[,\s]*(?:(?:No\.?|Number)\s*)?(\d{1,2}[A-C]?)\b/gi;
const ALMANAC_RE = /(?:New\s+Zealand\s+)?Nautical\s+Almanac(?:\s*\(?NZ\s*204\)?)?/gi;

/** `text` as nodes, with references to an Annual Notice or the Almanac as links. */
export function linkPublications(text) {
  const hits = [];
  for (const m of text.matchAll(NOTICE_RE)) hits.push({ i: m.index, len: m[0].length, no: m[1].toUpperCase(), t: m[0] });
  for (const m of text.matchAll(ALMANAC_RE)) if (!hits.some((h) => m.index < h.i + h.len && h.i < m.index + m[0].length)) hits.push({ i: m.index, len: m[0].length, no: "almanac", t: m[0] });
  hits.sort((a, b) => a.i - b.i);
  const out = [];
  let at = 0;
  for (const h of hits) {
    if (h.i < at) continue;
    out.push(document.createTextNode(text.slice(at, h.i)));
    const a = document.createElement("a");
    a.href = "#";
    a.className = "pub-link";
    a.textContent = h.t;
    a.title = h.no === "almanac" ? "Open the New Zealand Nautical Almanac (LINZ)" : `Open Annual New Zealand Notice to Mariners No. ${h.no} (LINZ)`;
    a.onclick = (e) => { e.preventDefault(); openPublication(h.no); };
    out.push(a);
    at = h.i + h.len;
  }
  out.push(document.createTextNode(text.slice(at)));
  return out;
}

export function setupPublications(opts = {}) {
  toast = opts.toast || toast;
  if ($("pub-get")) $("pub-get").onclick = () => download(!!$("pub-alm")?.checked || !!status?.almanac);
  if ($("pub-open-alm")) $("pub-open-alm").onclick = () => openPublication("almanac");
  if ($("pub-site")) $("pub-site").onclick = (e) => { e.preventDefault(); invoke("open_url", { url: "https://www.linz.govt.nz/products-services/maritime-safety/notices-mariners/nz-annual-notices-mariners" }).catch(() => {}); };
  refresh();
}
