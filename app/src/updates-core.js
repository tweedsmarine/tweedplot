// Software updates: the decisions, kept apart from Tauri so they can be tested.
//
// Modes (Settings › Diagnostics › Software updates):
//   off      - never look (a manual Check now still works)
//   notify   - look at start-up and twice a day, and show a notice (the default)
//   download - as notify, and fetch the installer quietly first, so Install is quick
// An update is never installed by itself: installing closes TweedPlot, so the skipper
// chooses when (and is asked first).

export const MODES = ["off", "notify", "download"];
export const DEFAULT_MODE = "notify";
/** First automatic look after start-up: the charts and data load first. */
export const FIRST_CHECK_MS = 60 * 1000;
/** Then every 12 hours while TweedPlot is open. */
export const CHECK_EVERY_MS = 12 * 60 * 60 * 1000;

/** Stored preferences, made safe whatever was (or wasn't) stored. */
export function readPrefs(raw) {
  let p = {};
  try { p = raw ? JSON.parse(raw) : {}; } catch { p = {}; }
  if (!p || typeof p !== "object") p = {};
  return {
    mode: MODES.includes(p.mode) ? p.mode : DEFAULT_MODE,
    lastCheck: Number.isFinite(p.lastCheck) && p.lastCheck > 0 ? p.lastCheck : 0,
  };
}

/** Whether an automatic look is due now. */
export function checkDue(prefs, now) {
  if (prefs.mode === "off") return false;
  // a clock set back (a PC with a flat battery) mustn't stop the checks for months
  if (prefs.lastCheck > now) return true;
  return now - prefs.lastCheck >= CHECK_EVERY_MS;
}

/** The notice text for an update that has been found. */
export function noticeText(update, { downloaded = false, progress = null } = {}) {
  const v = `TweedPlot ${update.version}`;
  if (progress != null) return `${v}: downloading ${progress}%`;
  return downloaded ? `${v} is ready to install.` : `${v} is available (you have ${update.currentVersion}).`;
}

/** Download progress in whole per cent from the updater's events, or null when the size isn't known. */
export function progressPercent(done, total) {
  if (!Number.isFinite(total) || total <= 0) return null;
  return Math.max(0, Math.min(100, Math.floor((done / total) * 100)));
}
