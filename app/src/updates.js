// Software updates: looks for a newer TweedPlot (the signed latest.json of the newest
// GitHub release, through Tauri's updater plugin), shows a notice, and installs only when
// the skipper says so. The decisions are in updates-core.js.
//
// Where: the notice at the top of the screen (#update-notice), Settings › Diagnostics ›
// Software updates (#upd-mode, #upd-check, #upd-status) and Check for updates in the About box.
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { ask } from "./ask.js";
import { readPrefs, checkDue, noticeText, progressPercent, FIRST_CHECK_MS, CHECK_EVERY_MS } from "./updates-core.js";

const KEY = "racenav.updates";
const RELEASES = "https://github.com/tweedsmarine/tweedplot/releases";

export function setupUpdates() {
  const $ = (id) => document.getElementById(id);
  const inTauri = typeof window !== "undefined" && !!window.__TAURI_INTERNALS__;
  let prefs = load();
  let found = null;        // the Update object from the last look
  let downloaded = false;  // its installer is fetched (mode download)
  let busy = false;        // a look, download or install is under way
  let dismissed = "";      // the version Later was chosen for (this session)

  function load() {
    try { return readPrefs(localStorage.getItem(KEY)); } catch { return readPrefs(null); }
  }
  function save() {
    try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* private storage off: defaults next time */ }
  }
  function log(line) { invoke("activity_log", { line: `Updates: ${line}` }).catch(() => {}); }
  function status(text) {
    for (const id of ["upd-status", "about-update-status"]) { const el = $(id); if (el) el.textContent = text; }
  }

  // ---- the notice ----
  const notice = $("update-notice");
  function showNotice(progress = null) {
    if (!notice || !found) return;
    $("update-text").textContent = noticeText(found, { downloaded, progress });
    $("update-install").disabled = busy;
    $("update-later").disabled = busy && progress != null;
    notice.hidden = false;
  }
  function hideNotice() { if (notice) notice.hidden = true; }

  // ---- looking ----
  /** manual: from Check now / Check for updates, so every outcome is reported. */
  async function look(manual) {
    if (!inTauri) { if (manual) status("Updates are checked in the installed app only."); return; }
    if (busy) return;
    busy = true;
    if (manual) status("Checking…");
    try {
      const u = await check({ timeout: 20000 });
      prefs.lastCheck = Date.now();
      save();
      if (!u) {
        found?.close?.().catch?.(() => {});
        found = null;
        downloaded = false;
        hideNotice();
        status(`Up to date (checked ${new Date(prefs.lastCheck).toLocaleString()}).`);
        return;
      }
      if (found?.version === u.version && downloaded) {
        u.close?.().catch?.(() => {}); // keep the one already downloaded
      } else {
        found?.close?.().catch?.(() => {}); // each Update holds a resource in the Rust side
        found = u;
        downloaded = false;
      }
      status(`TweedPlot ${found.version} is available.`);
      log(`${found.version} available (installed ${found.currentVersion})`);
      // the look is over: the notice drawn below must not show Install greyed out as busy
      busy = false;
      if (prefs.mode === "download" && !downloaded) await fetchInstaller(false);
      if (manual || dismissed !== found.version) showNotice();
    } catch (e) {
      // at sea there is often no internet: automatic looks say nothing
      const msg = String(e?.message ?? e);
      if (manual) status(`Couldn't check for updates: ${msg}`);
      log(`check failed: ${msg}`);
    } finally {
      busy = false;
    }
  }

  async function fetchInstaller(report) {
    if (!found || downloaded || busy) return;
    busy = true;
    let total = 0, done = 0;
    try {
      await found.download((ev) => {
        if (ev.event === "Started") total = ev.data?.contentLength ?? 0;
        else if (ev.event === "Progress") {
          done += ev.data?.chunkLength ?? 0;
          if (report) showNotice(progressPercent(done, total) ?? 0);
        }
      });
      downloaded = true;
      log(`${found.version} downloaded`);
    } catch (e) {
      const msg = String(e?.message ?? e);
      log(`download failed: ${msg}`);
      if (report) status(`Couldn't download the update: ${msg}`);
    } finally {
      busy = false;
    }
  }

  // ---- installing ----
  async function install() {
    if (!found || busy) return;
    const ok = await ask({
      title: `Install TweedPlot ${found.version}?`,
      note: "TweedPlot closes, installs the update and opens again. Your charts, library and settings are kept.",
      ok: "Install now",
      cancel: "Not now",
    });
    if (!ok) return;
    if (!downloaded) {
      await fetchInstaller(true);
      if (!downloaded) { showNotice(); return; }
    }
    busy = true;
    showNotice();
    $("update-text").textContent = `Installing TweedPlot ${found.version}…`;
    log(`installing ${found.version}`);
    try {
      await found.install(); // on Windows the app closes here and the installer takes over
    } catch (e) {
      busy = false;
      const msg = String(e?.message ?? e);
      log(`install failed: ${msg}`);
      status(`Couldn't install the update: ${msg}`);
      showNotice();
    }
  }

  // ---- wiring ----
  $("update-install")?.addEventListener("click", install);
  $("update-later")?.addEventListener("click", () => { dismissed = found?.version ?? ""; hideNotice(); });
  $("update-notes")?.addEventListener("click", () => {
    const url = found ? `${RELEASES}/tag/v${found.version}` : RELEASES;
    invoke("open_url", { url }).catch(() => window.open(url));
  });
  const mode = $("upd-mode");
  if (mode) {
    mode.value = prefs.mode;
    mode.addEventListener("change", () => {
      prefs.mode = mode.value;
      save();
      if (prefs.mode === "download" && found && !downloaded) fetchInstaller(false);
    });
  }
  for (const id of ["upd-check", "about-update"]) $(id)?.addEventListener("click", () => look(true));
  if (prefs.lastCheck) status(`Last checked ${new Date(prefs.lastCheck).toLocaleString()}.`);

  // automatic looks: a minute after start-up, then whenever one is due (checked hourly,
  // so a PC that slept through the 12 hours looks soon after it wakes)
  if (inTauri) {
    const auto = () => { if (checkDue(prefs, Date.now())) look(false); };
    setTimeout(() => { if (prefs.mode !== "off") look(false); }, FIRST_CHECK_MS);
    setInterval(auto, Math.min(CHECK_EVERY_MS, 60 * 60 * 1000));
  }

  return { check: () => look(true) };
}
