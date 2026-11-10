import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { patchWindow, readWindows } from "./windowstate.js";

const PREF_KEY = "racenav.nmeaMonitor";
const MAX_FRAMES = 20000;
const MAX_RENDERED = 5000;
const VARIABLES = [
  ["latitude", "Latitude", "position", "lat", (v) => `${v.toFixed(5)}°`],
  ["longitude", "Longitude", "position", "lon", (v) => `${v.toFixed(5)}°`],
  ["sog", "SOG", "motion", "sog", (v) => `${v.toFixed(2)} kn`],
  ["cog", "COG", "motion", "cog", (v) => `${v.toFixed(1)}°T`],
  ["depth", "Depth", "depth", "depth", (v) => `${v.toFixed(1)} m`],
  ["heading", "Heading (HDT)", "heading", "heading", (v) => `${v.toFixed(1)}°T`],
  ["headingMag", "Magnetic heading", "heading", "headingMag", (v) => `${v.toFixed(1)}°M`],
  ["variation", "Magnetic variation", "heading", "variation", (v) => `${v.toFixed(1)}°`],
  ["stw", "Speed through water", "speedThroughWater", "stw", (v) => `${v.toFixed(2)} kn`],
  ["aws", "Apparent wind speed", "wind", "aws", (v) => `${v.toFixed(1)} kn`],
  ["awa", "Apparent wind angle", "wind", "awa", (v) => `${v.toFixed(0)}°`],
  ["tws", "True wind speed", "wind", "tws", (v) => `${v.toFixed(1)} kn`],
  ["twa", "True wind angle", "wind", "twa", (v) => `${v.toFixed(0)}°`],
  ["twd", "True wind direction", "wind", "twd", (v) => `${v.toFixed(0)}°T`],
  ["depthKeel", "Depth below keel", "depth", "depthKeel", (v) => `${v.toFixed(1)} m`],
  ["depthSurface", "Depth below surface", "depth", "depthSurface", (v) => `${v.toFixed(1)} m`],
  ["waterTemp", "Water temperature", "waterTemperature", "waterTemp", (v) => `${v.toFixed(1)} °C`],
  ["pressure", "Barometric pressure", "weather", "pressure", (v) => `${v.toFixed(1)} hPa`],
  ["airTemp", "Air temperature", "weather", "airTemp", (v) => `${v.toFixed(1)} °C`],
  ["humidity", "Relative humidity", "weather", "humidity", (v) => `${v.toFixed(0)} %`],
];
const DEFAULT_VARIABLES = ["latitude", "longitude", "sog", "cog", "depth", "heading"];

function readPrefs() {
  try {
    const saved = JSON.parse(localStorage.getItem(PREF_KEY) || "{}");
    return {
      timestamps: saved.timestamps !== false,
      variables: Array.isArray(saved.variables) ? saved.variables.slice(0, 6) : DEFAULT_VARIABLES,
      duration: saved.duration || "10",
      customMinutes: saved.customMinutes || "10",
    };
  } catch {
    return { timestamps: true, variables: DEFAULT_VARIABLES, duration: "10", customMinutes: "10" };
  }
}

function savePrefs(prefs) {
  try { localStorage.setItem(PREF_KEY, JSON.stringify(prefs)); } catch { /* storage unavailable */ }
  window.dispatchEvent(new CustomEvent("racenav-nmea-monitor-prefs", { detail: prefs }));
}

function isoTime(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value || "") : date.toISOString();
}

function frameText(frame, sourceName, timestamps) {
  const prefix = timestamps ? `${isoTime(frame.receivedAt)}  ` : "";
  const invalid = frame.checksumValid ? "" : `  [INVALID${frame.error ? `: ${frame.error}` : ""}]`;
  const gps = frame.gps ? `  [GPS ${frame.gps}${frame.ignored ? ", ignored" : ""}]` : frame.ignored ? "  [ignored]" : "";
  return `${prefix}${sourceName}  ${frame.raw}${invalid}${gps}`;
}

function captureHeader(capture) {
  return [
    `TweedPlot NMEA 0183 capture`,
    `Started: ${new Date(capture.startedAt).toISOString()}`,
    `Input filter: ${capture.inputName}`,
    `Timestamps: ${capture.timestamps ? "yes" : "no"}`,
    "",
    "",
  ].join("\r\n");
}

function captureLines(capture, frames) {
  return frames.map((item) => `${frameText(item.frame, item.sourceName, capture.timestamps)}\r\n`).join("");
}

function captureFooter(capture, endedAt) {
  return [
    capture.count ? "" : "No sentences were received during this capture.",
    `Ended: ${new Date(endedAt).toISOString()}`,
    `Duration: ${Math.round((endedAt - capture.startedAt) / 1000)} seconds, ${capture.count} sentence${capture.count === 1 ? "" : "s"}`,
    "",
  ].join("\r\n");
}

function sourceColour(sourceId, sources) {
  const index = sources.findIndex((source) => source.id === sourceId);
  let hash = 0;
  for (const char of sourceId) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  const hue = ((index >= 0 ? index : hash) * 137.508) % 360;
  return `hsl(${hue.toFixed(1)} 85% 68%)`;
}

async function openMonitorWindow(report) {
  try {
    const labels = await invoke("monitor_list");
    const existing = labels.find((label) => readWindows()[label]?.view === "nmea");
    if (existing) {
      await invoke("monitor_focus", { label: existing });
      return;
    }
    const label = await invoke("monitor_open", { view: "nmea" });
    patchWindow(label, { view: "nmea", open: true });
  } catch (error) {
    report(`Could not open the live NMEA monitor: ${error}`, true);
  }
}

export function setupNmeaMonitor(root, { standalone = false, openSettings = () => {} } = {}) {
  if (!root) return;
  root.classList.add("nmea-monitor");
  root.innerHTML = `
    <div class="nmea-monitor-head">
      <div><h3>${standalone ? "Live NMEA 0183 data" : "Live NMEA data"}</h3>
        <div class="nmea-monitor-note">Raw sentences from each enabled input. Red rows have a checksum or sentence error.</div></div>
      <span class="grow"></span>
      ${standalone ? '<button type="button" data-close>Close</button>' : '<button type="button" data-popout>Pop out</button>'}
    </div>
    <div class="nmea-monitor-controls">
      <label>Input <select data-filter><option value="">All inputs</option></select></label>
      <label class="nmea-timestamp"><input type="checkbox" data-timestamps /> Add timestamps</label>
      <button type="button" data-pause>Pause scrolling</button>
      <button type="button" data-copy-minute>Capture 1 minute and copy</button>
      <label>File capture
        <select data-duration><option value="10">10 min</option><option value="30">30 min</option><option value="60">60 min</option><option value="custom">Custom</option></select>
      </label>
      <input data-custom-minutes type="number" min="1" max="1440" step="1" aria-label="Custom capture duration in minutes" />
      <button type="button" data-record>Capture to file</button>
      <button type="button" data-cancel hidden>Cancel capture</button>
      <button type="button" data-edit>Configure inputs</button>
    </div>
    <div class="nmea-monitor-message" data-message role="status" aria-live="polite"></div>
    <section class="nmea-variable-panel" aria-label="Selected live values">
      <div class="nmea-variable-heading"><b>Selected values</b><span>Choose up to six values from the active input for each data group.</span></div>
      <div class="nmea-variable-grid" data-variables></div>
    </section>
    <div class="nmea-monitor-log" data-log role="log" aria-label="Received NMEA sentences" aria-live="off"></div>`;

  const $ = (selector) => root.querySelector(selector);
  const filter = $("[data-filter]");
  const timestamps = $("[data-timestamps]");
  const pauseButton = $("[data-pause]");
  const copyButton = $("[data-copy-minute]");
  const duration = $("[data-duration]");
  const customMinutes = $("[data-custom-minutes]");
  const recordButton = $("[data-record]");
  const cancelButton = $("[data-cancel]");
  const message = $("[data-message]");
  const variableGrid = $("[data-variables]");
  if (!standalone) $(".nmea-variable-panel").remove();
  const log = $("[data-log]");
  if (!standalone) {
    // the drawer's selected-source summary sits between the value selectors and the raw log
    for (const id of ["nmea-active-sources", "nmea-current-data"]) {
      const summary = document.getElementById(id);
      if (summary) log.before(summary);
    }
  }
  const prefs = readPrefs();
  const sources = [];
  const sourceNames = new Map();
  const observations = new Map();
  const groupSources = {};
  const frames = [];
  let paused = false;
  let pausedDropped = 0;
  let capture = null;
  let timer = 0;
  // Rows are only built while the monitor is on screen: hidden (a closed drawer, another view) it
  // just keeps the buffer, and draws it when shown again. A busy bus is many sentences a second.
  let onScreen = false, dirty = false;
  new IntersectionObserver((entries) => {
    onScreen = entries.some((e) => e.isIntersecting);
    if (onScreen && dirty && !paused) { dirty = false; renderLog(); }
  }).observe(root);

  function applyPrefs(next) {
    prefs.timestamps = next.timestamps !== false;
    prefs.variables = Array.isArray(next.variables) ? next.variables.slice(0, 6) : DEFAULT_VARIABLES;
    prefs.duration = next.duration || "10";
    prefs.customMinutes = next.customMinutes || "10";
    timestamps.checked = prefs.timestamps;
    duration.value = ["10", "30", "60", "custom"].includes(prefs.duration) ? prefs.duration : "10";
    customMinutes.value = prefs.customMinutes;
    customMinutes.hidden = duration.value !== "custom";
    renderVariables();
    if (!paused) renderLog();
  }

  function report(text, error = false) {
    message.textContent = text;
    message.classList.toggle("bad", error);
  }

  function setCaptureActive(active) {
    copyButton.disabled = active;
    recordButton.disabled = active;
    filter.disabled = active;
    timestamps.disabled = active;
    duration.disabled = active;
    customMinutes.disabled = active;
    cancelButton.hidden = !active;
  }

  function setSourceList(nextSources) {
    sources.splice(0, sources.length, ...nextSources);
    for (const source of sources) sourceNames.set(source.id, source.name);
    const selected = filter.value;
    filter.replaceChildren(new Option("All inputs", ""));
    for (const source of sources) {
      const option = new Option(source.name, source.id);
      option.style.color = sourceColour(source.id, sources);
      filter.add(option);
    }
    filter.value = sources.some((source) => source.id === selected) ? selected : "";
    renderLog();
    renderVariables();
  }

  function renderVariables() {
    variableGrid.replaceChildren();
    for (let index = 0; index < 6; index += 1) {
      const selected = prefs.variables[index] || "";
      const row = document.createElement("label");
      row.className = "nmea-variable";
      const select = document.createElement("select");
      select.setAttribute("aria-label", `Live value ${index + 1}`);
      select.add(new Option("Not shown", ""));
      for (const [id, label] of VARIABLES) select.add(new Option(label, id));
      select.value = VARIABLES.some(([id]) => id === selected) ? selected : "";
      const value = document.createElement("output");
      value.textContent = selected ? variableValue(selected) : "—";
      select.addEventListener("change", () => {
        prefs.variables[index] = select.value;
        savePrefs(prefs);
        renderVariables();
      });
      row.append(select, value);
      variableGrid.append(row);
    }
  }

  function variableValue(id) {
    const definition = VARIABLES.find(([key]) => key === id);
    if (!definition) return "—";
    const [, , group, field, format] = definition;
    const observation = observations.get(groupSources[group]);
    const value = observation?.[field];
    return typeof value === "number" && Number.isFinite(value) ? format(value) : "—";
  }

  function renderValues() {
    for (const row of variableGrid.querySelectorAll(".nmea-variable")) {
      const id = row.querySelector("select").value;
      row.querySelector("output").textContent = id ? variableValue(id) : "—";
    }
  }

  function renderLog() {
    if (!onScreen) { dirty = true; return; }
    const shown = frames.filter(({ frame }) => !filter.value || frame.sourceId === filter.value).slice(-MAX_RENDERED);
    const fragment = document.createDocumentFragment();
    for (const item of shown) fragment.append(makeRow(item));
    if (!shown.length) {
      const empty = document.createElement("div");
      empty.className = "nmea-monitor-empty";
      empty.textContent = "No sentences received from this input yet.";
      fragment.append(empty);
    }
    log.replaceChildren(fragment);
    log.scrollTop = log.scrollHeight;
  }

  function makeRow(item) {
    const { frame, sourceId, sourceName } = item;
    const row = document.createElement("div");
    row.className = `nmea-frame${frame.checksumValid ? "" : " invalid"}${frame.ignored ? " ignored" : ""}`;
    row.style.setProperty("--stream-colour", sourceColour(sourceId, sources));
    if (frame.error) row.title = frame.error;
    const time = document.createElement("span");
    time.className = "nmea-frame-time";
    time.textContent = prefs.timestamps ? isoTime(frame.receivedAt) : "";
    const source = document.createElement("span");
    source.className = "nmea-frame-source";
    source.textContent = sourceName;
    source.title = sourceName;
    // two GPS receivers on one input: which one sent it, and whether it was left out
    const gps = document.createElement("span");
    gps.className = "nmea-frame-gps";
    gps.textContent = frame.gps ? `GPS ${frame.gps}` : "";
    if (frame.gps || frame.ignored) {
      gps.title = frame.ignored
        ? `${frame.gps ? `GPS ${frame.gps}: ` : ""}not used (another receiver on this input is in use, or the fix is out of time order)`
        : `GPS ${frame.gps}: the receiver in use on this input`;
    }
    const raw = document.createElement("code");
    raw.textContent = frame.raw;
    row.append(time, source, raw);
    if (frame.gps) row.append(gps);
    if (frame.ignored) {
      const badge = document.createElement("b");
      badge.className = "nmea-frame-ignored";
      badge.textContent = "ignored";
      row.append(badge);
    }
    if (!frame.checksumValid) {
      const badge = document.createElement("b");
      badge.className = "nmea-frame-invalid";
      badge.textContent = frame.error || "INVALID";
      row.append(badge);
    }
    return row;
  }

  function appendFrame(item) {
    frames.push(item);
    if (frames.length > MAX_FRAMES) {
      pausedDropped += frames.length - MAX_FRAMES;
      frames.splice(0, frames.length - MAX_FRAMES);
    }
    if (paused) return;
    if (filter.value && filter.value !== item.sourceId) return;
    if (!onScreen) { dirty = true; return; }
    log.querySelector(".nmea-monitor-empty")?.remove();
    log.append(makeRow(item));
    while (log.childElementCount > MAX_RENDERED) log.firstElementChild.remove();
    log.scrollTop = log.scrollHeight;
  }

  function captureItemsFor(payload, receivedFrames) {
    if (!capture || capture.finishing) return;
    for (const frame of receivedFrames) {
      if (!capture.inputId || frame.sourceId === capture.inputId) {
        capture.frames.push({ frame, sourceName: payload.sourceName || sourceNames.get(payload.sourceId) || payload.sourceId });
        capture.count += 1;
      }
    }
    if (Date.now() >= capture.deadline) finishCapture();
  }

  /** A file capture's sentences so far go to the file (every few seconds), so a long capture
   *  never piles up in the page. Writes are chained so they land in order. */
  function flushCapture(c) {
    if (!c || c.kind !== "file" || !c.frames.length) return c?.writing ?? Promise.resolve();
    const text = captureLines(c, c.frames.splice(0));
    c.writing = (c.writing || Promise.resolve()).then(() => invoke("nmea_capture_append", { text }));
    return c.writing;
  }

  async function finishCapture() {
    if (!capture || capture.finishing) return;
    capture.finishing = true;
    const done = capture;
    capture = null;
    clearInterval(timer);
    timer = 0;
    setCaptureActive(false);
    const endedAt = Date.now();
    if (done.kind === "clipboard") {
      const text = captureHeader(done) + captureLines(done, done.frames) + captureFooter(done, endedAt);
      try {
        await navigator.clipboard.writeText(text);
        report(`Copied ${done.count} sentence${done.count === 1 ? "" : "s"} to the clipboard.`);
      } catch (error) {
        report(`Could not copy the NMEA capture to the clipboard (click in this window and try again): ${error}`, true);
      }
    } else {
      try {
        await flushCapture(done);
        await invoke("nmea_capture_append", { text: captureFooter(done, endedAt) });
        await invoke("nmea_capture_end", { discard: false });
        report(`Saved ${done.count} sentence${done.count === 1 ? "" : "s"} to ${done.path}.`);
      } catch (error) {
        report(`Could not save the NMEA capture: ${error}`, true);
        invoke("nmea_capture_end", { discard: false }).catch(() => {});
      }
    }
  }

  async function startCapture(kind, durationMs) {
    const inputId = filter.value;
    const c = {
      kind,
      startedAt: Date.now(),
      frames: [],
      count: 0,
      timestamps: prefs.timestamps,
      inputId,
      inputName: inputId ? sourceNames.get(inputId) || inputId : "All inputs",
    };
    setCaptureActive(true);
    if (kind === "file") {
      // where to, first: the sentences are written to it as they come
      try {
        c.path = await invoke("nmea_capture_begin", {
          header: captureHeader(c),
          suggest: `nmea-capture-${new Date(c.startedAt).toISOString().replaceAll(":", "-").slice(0, 19)}.txt`,
        });
      } catch (error) {
        setCaptureActive(false);
        report(`Could not start the NMEA capture: ${error}`, true);
        return;
      }
      if (!c.path) { setCaptureActive(false); report("File capture cancelled; no file was written."); return; }
      c.startedAt = Date.now(); // the time spent in the dialog doesn't count
    }
    c.deadline = c.startedAt + durationMs;
    capture = c;
    report(`${kind === "clipboard" ? "One-minute clipboard" : "File"} capture started (${Math.ceil(durationMs / 60000)} min).`);
    let tick = 0;
    timer = setInterval(() => {
      if (!capture) return;
      const remaining = Math.max(0, capture.deadline - Date.now());
      if (remaining === 0) { finishCapture(); return; }
      if (++tick % 3 === 0) flushCapture(capture).catch((error) => report(`Could not write the NMEA capture: ${error}`, true));
      const left = Math.ceil(remaining / 1000);
      report(`Capture in progress: ${left >= 120 ? `${Math.ceil(left / 60)} min` : `${left} s`} remaining, ${capture.count} sentence${capture.count === 1 ? "" : "s"}${capture.path ? ` to ${capture.path}` : ""}.`);
    }, 1000);
  }

  timestamps.checked = prefs.timestamps;
  duration.value = ["10", "30", "60", "custom"].includes(prefs.duration) ? prefs.duration : "10";
  customMinutes.value = prefs.customMinutes;
  customMinutes.hidden = duration.value !== "custom";
  const syncDuration = () => { customMinutes.hidden = duration.value !== "custom"; };
  duration.addEventListener("change", syncDuration);
  customMinutes.addEventListener("change", () => {
    prefs.customMinutes = customMinutes.value;
    savePrefs(prefs);
  });
  timestamps.addEventListener("change", () => {
    prefs.timestamps = timestamps.checked;
    savePrefs(prefs);
    if (!paused) renderLog();
  });
  filter.addEventListener("change", () => { if (!paused) renderLog(); });
  pauseButton.addEventListener("click", () => {
    paused = !paused;
    pauseButton.textContent = paused ? "Resume scrolling" : "Pause scrolling";
    pauseButton.classList.toggle("on", paused);
    if (!paused) {
      renderLog();
      const visible = Math.min(frames.length, MAX_RENDERED);
      const invalid = frames.reduce((count, item) => count + Number(!item.frame.checksumValid), 0);
      report(`${visible} recent sentences shown${frames.length > MAX_RENDERED ? `; ${frames.length - MAX_RENDERED} older rows are outside the visible scrollback` : ""}${pausedDropped ? `; older rows exceeded the ${MAX_FRAMES.toLocaleString()}-sentence buffer limit` : ""}.${invalid ? ` ${invalid} invalid sentence${invalid === 1 ? "" : "s"} in the current buffer.` : ""}`);
    } else report("Scrolling paused; incoming data continues to buffer.");
  });
  copyButton.addEventListener("click", () => startCapture("clipboard", 60_000));
  recordButton.addEventListener("click", () => {
    const minutes = duration.value === "custom" ? Number(customMinutes.value) : Number(duration.value);
    if (!Number.isInteger(minutes) || minutes < 1 || minutes > 1440) {
      report("Enter a custom capture duration from 1 to 1440 minutes.", true);
      customMinutes.focus();
      return;
    }
    prefs.duration = duration.value;
    prefs.customMinutes = customMinutes.value;
    savePrefs(prefs);
    startCapture("file", minutes * 60_000);
  });
  cancelButton.addEventListener("click", () => {
    if (!capture) return;
    const was = capture;
    clearInterval(timer);
    timer = 0;
    capture = null;
    setCaptureActive(false);
    if (was.kind === "file") {
      // let any write in flight finish, then delete the file
      (was.writing || Promise.resolve()).catch(() => {}).then(() => invoke("nmea_capture_end", { discard: true }))
        .then(() => report("Capture cancelled; the file was deleted."))
        .catch((error) => report(`Capture cancelled, but the file could not be deleted: ${error}`, true));
    } else report("Capture cancelled; no output was created.");
  });
  $("[data-edit]").addEventListener("click", () => {
    if (!standalone) { openSettings(); return; }
    emit("nmea-open-settings").catch((error) => report(`Could not open the input settings: ${error}`, true));
  });
  $("[data-popout]")?.addEventListener("click", () => openMonitorWindow(report));
  $("[data-close]")?.addEventListener("click", () => getCurrentWindow().close().catch((error) => report(`Could not close the monitor window: ${error}`, true)));

  invoke("nmea_sources_get").then((data) => setSourceList(data.sources || []))
    .catch((error) => report(`Could not load NMEA input names: ${error}`, true));
  listen("nmea-sources-changed", () => {
    invoke("nmea_sources_get").then((data) => setSourceList(data.sources || []))
      .catch((error) => report(`Could not refresh NMEA input names: ${error}`, true));
  }).catch((error) => report(`Could not subscribe to NMEA input changes: ${error}`, true));
  listen("nmea-observation", ({ payload }) => {
    if (!payload?.sourceId) return;
    const received = Array.isArray(payload.frames) && payload.frames.length
      ? payload.frames.map((frame) => ({
        sourceId: payload.sourceId,
        receivedAt: frame.receivedAt || new Date().toISOString(),
        raw: frame.raw || "",
        checksumValid: frame.checksumValid !== false,
        error: frame.error || null,
        gps: frame.gps || null,
        ignored: !!frame.ignored,
      }))
      : (payload.sentences || []).map((raw) => ({
        sourceId: payload.sourceId,
        receivedAt: new Date().toISOString(),
        raw,
        checksumValid: true,
        error: null,
      }));
    observations.set(payload.sourceId, payload.observation || {});
    const items = received.map((frame) => ({
      frame,
      sourceId: payload.sourceId,
      sourceName: payload.sourceName || sourceNames.get(payload.sourceId) || payload.sourceId,
    }));
    captureItemsFor(payload, received);
    for (const item of items) appendFrame(item);
    renderValues();
  }).catch((error) => report(`Could not subscribe to NMEA sentences: ${error}`, true));
  listen("nmea-mux-status", ({ payload }) => {
    if (!payload?.groupSources) return;
    Object.assign(groupSources, payload.groupSources);
    for (const group of Object.keys(groupSources)) {
      if (!payload.groupSources[group]) delete groupSources[group];
    }
    renderValues();
  }).catch((error) => report(`Could not subscribe to selected NMEA values: ${error}`, true));
  window.addEventListener("racenav-nmea-monitor-prefs", ({ detail }) => applyPrefs(detail));
  window.addEventListener("storage", (event) => {
    if (event.key !== PREF_KEY || !event.newValue) return;
    try { applyPrefs(JSON.parse(event.newValue)); } catch { report("Could not read shared NMEA monitor preferences.", true); }
  });

  renderVariables();
}
