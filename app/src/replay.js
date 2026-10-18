// Replay a Signal K session log in place of the live data (Chart and Signal K ›
// Replay a log), with a floating bar: play/pause, speed, position, stop.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const $ = (id) => document.getElementById(id);

export function setupReplay({ onState } = {}) {
  let logs = [];
  let state = null; // last "replay" event
  let dragging = false;

  const fmtT = (ms) => {
    const d = new Date(ms);
    return d.toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", second: "2-digit" });
  };
  const fmtDur = (ms) => {
    const m = Math.round(ms / 60000);
    return m < 90 ? `${m} min` : `${(m / 60).toFixed(1)} h`;
  };
  const fmtMB = (b) => (b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1e3))} kB`);

  async function refresh() {
    try {
      const r = await invoke("replay_logs");
      logs = r?.logs || [];
      $("replay-log").title = `Logs in ${r?.dir ?? "—"}`;
    } catch { logs = []; }
    const sel = $("replay-log");
    sel.innerHTML = "";
    if (!logs.length) sel.innerHTML = `<option value="">No logs yet</option>`;
    for (const l of logs) {
      const o = document.createElement("option");
      o.value = l.path;
      o.textContent = `${fmtT(l.start)} · ${fmtDur(l.end - l.start)} · ${fmtMB(l.bytes)}`;
      sel.append(o);
    }
    $("replay-play").disabled = !logs.length;
  }

  function show(on) {
    $("replay-bar").hidden = !on;
    document.body.classList.toggle("replaying", on);
    onState?.(on);
  }

  $("replay-play").onclick = async () => {
    const path = $("replay-log").value;
    if (!path) return;
    try {
      await invoke("replay_start", { path, speed: +$("rp-speed").value });
      $("rp-play").textContent = "❚❚";
      show(true);
      $("replay-note").textContent = "Replaying: the bar at the top shows where you are. ✕ goes back to live data.";
    } catch (e) { $("replay-note").textContent = `Could not play it: ${e}`; }
  };
  $("rp-play").onclick = () => {
    const paused = !(state?.paused);
    invoke("replay_control", { paused }).catch(() => {});
    $("rp-play").textContent = paused ? "▶" : "❚❚";
    if (state) state.paused = paused;
  };
  $("rp-speed").onchange = () => invoke("replay_control", { speed: +$("rp-speed").value }).catch(() => {});
  $("rp-pos").oninput = () => {
    dragging = true;
    if (state) $("rp-time").textContent = fmtT(state.start + (state.end - state.start) * $("rp-pos").value / 1000);
  };
  $("rp-pos").onchange = () => {
    dragging = false;
    if (!state) return;
    const t = Math.round(state.start + (state.end - state.start) * $("rp-pos").value / 1000);
    invoke("replay_control", { seek: t }).catch(() => {});
  };
  $("rp-stop").onclick = () => invoke("replay_stop").catch(() => {});

  listen("replay", ({ payload }) => {
    state = payload;
    if ($("replay-bar").hidden) show(true);
    if (!dragging) {
      const f = payload.end > payload.start ? (payload.at - payload.start) / (payload.end - payload.start) : 0;
      $("rp-pos").value = String(Math.round(Math.max(0, Math.min(1, f)) * 1000));
      $("rp-time").textContent = fmtT(payload.at);
    }
    $("rp-time").title = `${payload.name}: ${fmtT(payload.start)} to ${fmtT(payload.end)}`;
  });
  listen("replay-end", ({ payload }) => {
    state = null;
    show(false);
    $("replay-note").textContent = payload?.error ? `The replay stopped: ${payload.error}` : "Replay finished: back to live data.";
    refresh();
  });

  refresh();
  return { refresh, active: () => !$("replay-bar").hidden };
}
