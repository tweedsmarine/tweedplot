// Which window this is. The first window ("main") is the primary: it runs the jobs
// that must happen once however many windows are open (alarm checks and sound, MOB,
// anchor watch). Monitor windows ("monitor-N") run the same app, show whatever view
// they are set to, and hand those jobs to the primary.
import { getCurrentWindow } from "@tauri-apps/api/window";

export const LABEL = (() => { try { return getCurrentWindow().label; } catch { return "main"; } })();
export const PRIMARY = LABEL === "main";

/** Storage key for settings that belong to this window alone (view, colours). */
export const own = (key) => (PRIMARY ? key : `${key}@${LABEL}`);

export const startView = new URLSearchParams(location.search).get("view");
