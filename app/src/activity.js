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
  movable(host);
  invoke("app_info").then((i) => { version = i?.version || ""; }).catch(() => {});
  return host;
}

// the cards can be dragged (by their title line) out of the way; they stay there, and
// double-clicking a title puts them back above the dock
const POS = "racenav.float.activity";
function movable(el) {
  let pos = null;
  try { pos = JSON.parse(localStorage.getItem(POS) || "null"); } catch { /* storage unavailable */ }
  const place = () => {
    el.classList.toggle("moved", !!pos);
    if (!pos) { Object.assign(el.style, { left: "", top: "", right: "", bottom: "" }); return; }
    const w = window.innerWidth, h = window.innerHeight, r = el.getBoundingClientRect();
    // kept over the chart: never over the data panel, the rail or a drawer (it hid the view buttons)
    const m = document.getElementById("map")?.getBoundingClientRect();
    const lo = m ? m.left : 0, hi = m ? Math.max(lo, m.right - r.width) : Math.max(0, w - r.width);
    const x = Math.min(Math.max(lo, pos.x * w), hi), y = Math.min(Math.max(0, pos.y * h), Math.max(0, h - Math.max(60, r.height)));
    Object.assign(el.style, { left: `${x}px`, top: `${y}px`, right: "auto", bottom: "auto" });
  };
  el.addEventListener("pointerdown", (e) => {
    const head = e.target.closest(".act-head");
    if (!head || e.target.closest("button") || e.button !== 0) return;
    e.preventDefault();
    const r = el.getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    head.setPointerCapture(e.pointerId);
    const move = (ev) => { pos = { x: (ev.clientX - dx) / window.innerWidth, y: (ev.clientY - dy) / window.innerHeight }; place(); };
    const up = () => {
      head.removeEventListener("pointermove", move); head.removeEventListener("pointerup", up);
      try { localStorage.setItem(POS, JSON.stringify(pos)); } catch { /* ignore */ }
    };
    head.addEventListener("pointermove", move);
    head.addEventListener("pointerup", up);
  });
  el.addEventListener("dblclick", (e) => {
    if (!e.target.closest(".act-head") || e.target.closest("button")) return;
    pos = null; try { localStorage.removeItem(POS); } catch { /* ignore */ } place();
  });
  window.addEventListener("resize", place);
  // the chart narrows when the data panel, a drawer or the dock opens: keep the cards over it
  const map = document.getElementById("map");
  if (map && typeof ResizeObserver === "function") new ResizeObserver(place).observe(map);
  place();
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
  el.innerHTML = `<div class="act-head" title="Drag to move the cards; double-click to put them back"><b class="act-title"></b><span class="grow"></span><button class="act-close" title="Close">✕</button></div>
    <div class="act-step"></div><div class="act-bar" hidden><div></div></div>
    <div class="act-result" hidden></div>
    <details class="act-details" hidden><summary>Details</summary><ul></ul></details>
    <div class="act-buttons"><button class="act-cancel" hidden>Cancel</button><button class="act-copy" hidden title="Copy what happened, with the version, to send to TweedsMarine">Copy for support</button></div>`;
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

let sysInfo = null;

function sysText() {
  if (!sysInfo) return "";
  const h = sysInfo.hardware || {};
  return [
    "System",
    `  TweedPlot ${sysInfo.version} (build ${BUILD}), ${sysInfo.arch}`,
    `  OS: ${h.os || sysInfo.os_family}`,
    `  Computer: ${h.model || "unknown"}`,
    `  CPU: ${h.cpu || "unknown"} (${sysInfo.cpu_threads} threads)`,
    `  RAM: ${h.ram ?? "?"} GB`,
    `  Graphics: ${h.gpu || "unknown"}`,
    `  WebView2: ${sysInfo.webview || "unknown"}`,
    "Data folders",
    ...(sysInfo.folders || []).map((f) => `  ${f.name}: ${f.path}`),
    "",
  ].join("\n");
}

/** Settings > Diagnostics > About this system. */
export async function setupSystemInfo() {
  const $ = (id) => document.getElementById(id);
  const dl = $("sys-info"), list = $("sys-folders");
  if (!dl) return;
  try { sysInfo = await invoke("system_info"); } catch (e) { dl.textContent = `Not available: ${e}`; return; }
  const h = sysInfo.hardware || {};
  const rows = [
    ["Version", `${sysInfo.version} (build ${BUILD})`],
    ["Operating system", `${h.os || sysInfo.os_family} (${sysInfo.arch})`],
    ["Computer", h.model || "unknown"],
    ["Processor", `${h.cpu || "unknown"}, ${sysInfo.cpu_threads} threads`],
    ["Memory", h.ram ? `${h.ram} GB` : "unknown"],
    ["Graphics", h.gpu || "unknown"],
    ["WebView2", sysInfo.webview || "unknown"],
  ];
  dl.replaceChildren(...rows.flatMap(([k, v]) => {
    const dt = document.createElement("dt"), dd = document.createElement("dd");
    dt.textContent = k; dd.textContent = v;
    return [dt, dd];
  }));
  list.replaceChildren(...(sysInfo.folders || []).map((f) => {
    const row = document.createElement("div");
    row.className = "sys-folder";
    const text = document.createElement("div");
    text.className = "grow";
    const name = document.createElement("b"); name.textContent = f.name;
    const path = document.createElement("div"); path.className = "mono wx-note"; path.textContent = f.path;
    const note = document.createElement("div"); note.className = "wx-note"; note.textContent = f.note;
    text.append(name, path, note);
    const btn = document.createElement("button");
    btn.textContent = "Open";
    btn.title = "Open this folder (it is created if it doesn't exist yet)";
    btn.onclick = () => invoke("open_folder", { path: f.path }).catch((e) => { note.textContent = `Could not open: ${e}`; });
    row.append(text, btn);
    return row;
  }));
  $("sys-copy").onclick = () => copy(`${supportText(recent.slice(-20))}\n\n${sysText()}`, $("sys-copy"));
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
  $("act-log-copy").onclick = () => copy(`${supportText(recent.slice(-20))}\n\n${sysText()}\nActivity log (${$("act-log-path").textContent}):\n${fileLines.slice(-60).join("\n")}`, $("act-log-copy"));
  window.addEventListener("racenav-activity", () => { if (!pre.closest(".tab")?.hidden) show(); });
  new MutationObserver(() => { if (!pre.closest(".tab")?.hidden) show(); }).observe(pre.closest(".tab") ?? document.body, { attributes: true, attributeFilter: ["hidden"] });
  show();
}

export { copy };
