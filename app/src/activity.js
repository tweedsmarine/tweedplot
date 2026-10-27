// Activity cards: what long jobs (imports, deletes, exports, downloads) are doing and how
// they went, in one place above the dock, seen with the drawers closed. Alerts are not
// shown here: they stay in the Alarms drawer and banner, with their colours and sounds
// (Bridge Alert Management): a card never looks or sounds like an alert.
//
//   const job = activity.start("Importing sarda.tzd", { cancel: () => … });
//   job.progress("Tracks", 1200, 14076);
//   job.done("38,082 marks matched, 11,777 tracks", { details: [...], keep: true });
//   job.fail("Could not read the file", { details: [...] });
//   activity.note("Deleted 1,240 marks.");
//
// Every job and note goes into the activity log (Settings > Diagnostics, and
// <data folder>\logs\activity.log). Copy for support puts the version, the job and its
// whole report on the clipboard.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/* global __BUILD__ */
const BUILD = typeof __BUILD__ === "string" ? __BUILD__ : "dev";
const recent = []; // the last jobs and notes, newest last
let host = null;
let version = "";
let n = 0;

function container() {
  if (host) return host;
  host = document.createElement("div");
  host.id = "activity";
  host.setAttribute("role", "status");
  host.setAttribute("aria-live", "polite");
  document.body.append(host);
  invoke("app_info").then((i) => { version = i?.version || ""; }).catch(() => {});
  return host;
}

const fmt = (v) => (typeof v === "number" ? v.toLocaleString() : v);

function log(entry) {
  recent.push(entry);
  if (recent.length > 200) recent.shift();
  const line = `${entry.ok === false ? "FAILED " : ""}${entry.title}${entry.result ? `: ${entry.result}` : ""}${entry.details?.length ? ` | ${entry.details.join(" | ")}` : ""}`;
  invoke("activity_log", { line }).catch(() => {});
  window.dispatchEvent(new CustomEvent("racenav-activity"));
}

/** The text Copy for support puts on the clipboard. */
export function supportText(entries = recent.slice(-1)) {
  const head = [`TweedPlot ${version} (build ${BUILD})`, `${navigator.userAgent}`, new Date().toString(), ""];
  const body = entries.flatMap((e) => [
    `${new Date(e.at).toLocaleString()}  ${e.ok === false ? "FAILED: " : ""}${e.title}`,
    ...(e.params ? [`  ${e.params}`] : []),
    ...(e.result ? [`  ${e.result}`] : []),
    ...(e.details || []).map((d) => `  - ${d}`),
    "",
  ]);
  const before = recent.slice(-12, -entries.length || undefined).map((e) => `${new Date(e.at).toLocaleTimeString()}  ${e.ok === false ? "FAILED " : ""}${e.title}${e.result ? `: ${e.result}` : ""}`);
  return [...head, ...body, ...(before.length ? ["Earlier:", ...before] : [])].join("\n");
}

async function copy(text, btn) {
  try {
    await navigator.clipboard.writeText(text);
    if (btn) { const t = btn.textContent; btn.textContent = "Copied"; setTimeout(() => { btn.textContent = t; }, 1500); }
  } catch { /* clipboard unavailable */ }
}

function card(title) {
  const el = document.createElement("div");
  el.className = "act-card";
  el.innerHTML = `<div class="act-head"><b class="act-title"></b><span class="grow"></span><button class="act-close" title="Close">✕</button></div>
    <div class="act-step"></div><div class="act-bar" hidden><div></div></div>
    <div class="act-result" hidden></div>
    <details class="act-details" hidden><summary>Details</summary><ul></ul></details>
    <div class="act-buttons"><button class="act-cancel" hidden>Cancel</button><button class="act-copy" hidden title="Copy what happened, with the version, to send to Tweedsmarine">Copy for support</button></div>`;
  el.querySelector(".act-title").textContent = title;
  el.querySelector(".act-close").onclick = () => el.remove();
  container().append(el);
  return el;
}

/** A job on a card: progress while it runs, then its result. */
export function start(title, { cancel = null, params = "" } = {}) {
  const id = `job${Date.now().toString(36)}${(n++).toString(36)}`;
  const el = card(title);
  el.classList.add("running");
  const step = el.querySelector(".act-step"), bar = el.querySelector(".act-bar"), fill = bar.firstElementChild;
  const cancelBtn = el.querySelector(".act-cancel");
  if (cancel) {
    cancelBtn.hidden = false;
    cancelBtn.onclick = () => { cancelBtn.disabled = true; cancelBtn.textContent = "Stopping…"; cancel(id); };
  }
  step.textContent = "Starting…";
  const at = Date.now();
  let finished = false;
  const unlisten = listen("job-progress", (e) => {
    if (e.payload?.job === id) job.progress(e.payload.step, e.payload.done, e.payload.total);
  });
  const finish = (ok, result, { details = [], keep = false } = {}) => {
    if (finished) return;
    // big numbers from the app's reports read better grouped: 8577906 → 8,577,906
    const group = (t) => String(t).replace(/\b\d{5,}\b/g, (d) => Number(d).toLocaleString());
    result = group(result);
    details = details.map(group);
    finished = true;
    unlisten.then((f) => f()).catch(() => {});
    el.classList.remove("running");
    el.classList.add(ok ? "ok" : "failed");
    step.hidden = true;
    bar.hidden = true;
    cancelBtn.hidden = true;
    const res = el.querySelector(".act-result");
    res.hidden = !result;
    res.textContent = result;
    const entry = { at, title, params, ok, result, details };
    if (details.length) {
      const d = el.querySelector(".act-details");
      d.hidden = false;
      const ul = d.querySelector("ul");
      for (const t of details) { const li = document.createElement("li"); li.textContent = t; ul.append(li); }
      entry.details = details;
    }
    const cp = el.querySelector(".act-copy");
    cp.hidden = false;
    cp.onclick = () => copy(supportText([entry]), cp);
    log(entry);
    // a plain success goes after a while; anything to read (failed, left out) stays until closed
    if (ok && !keep) setTimeout(() => { if (!el.matches(":hover")) el.remove(); else el.addEventListener("mouseleave", () => el.remove(), { once: true }); }, 8000);
  };
  const job = {
    id,
    /** `text` replaces "step: done of total" (e.g. sizes in MB) while the bar still follows done/total. */
    progress(s, done = 0, total = 0, text = "") {
      if (finished) return;
      step.textContent = text || (total ? `${s}: ${fmt(done)} of ${fmt(total)}` : `${s}…`);
      bar.hidden = !total;
      if (total) fill.style.width = `${Math.min(100, (done / total) * 100)}%`;
    },
    done: (result, o) => finish(true, result, o),
    fail: (result, o) => finish(false, result, o),
  };
  return job;
}

/** A one-off result (no progress): a small card that goes after a few seconds. */
export function note(text, { details = [] } = {}) {
  const j = start(text);
  j.done("", { details });
  return j;
}

/** The activity log: recent entries (this session), and the file's last lines. */
export async function readLog(lines = 200) {
  try { return await invoke("activity_log_read", { n: lines }); } catch { return { path: "", lines: [] }; }
}

/** Settings > Diagnostics > Activity log. */
export function setupActivityLog() {
  const $ = (id) => document.getElementById(id);
  const pre = $("act-log");
  if (!pre) return;
  let fileLines = [];
  async function show() {
    const r = await readLog(300);
    fileLines = r.lines || [];
    $("act-log-path").textContent = r.path || "";
    pre.textContent = fileLines.length ? [...fileLines].reverse().join("\n") : "Nothing yet.";
  }
  $("act-log-refresh").onclick = show;
  $("act-log-copy").onclick = () => copy(`${supportText(recent.slice(-20))}\n\nActivity log (${$("act-log-path").textContent}):\n${fileLines.slice(-60).join("\n")}`, $("act-log-copy"));
  window.addEventListener("racenav-activity", () => { if (!pre.closest(".tab")?.hidden) show(); });
  new MutationObserver(() => { if (!pre.closest(".tab")?.hidden) show(); }).observe(pre.closest(".tab") ?? document.body, { attributes: true, attributeFilter: ["hidden"] });
  show();
}

export { copy };
