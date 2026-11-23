// The toolbar's Screenshot button (Ctrl+Shift+S): a picture of this window as the crew sees it,
// saved in one folder (screenshot.rs) for their own records or to send for support.
// The first time, it asks where: Desktop\TweedPlot-Screenshots to begin with, or a folder of
// their choosing. Settings › Display › Screenshots shows the folder and changes it.
import { invoke } from "@tauri-apps/api/core";
import * as activity from "./activity.js";
import { ask } from "./ask.js";

const $ = (id) => document.getElementById(id);
const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));

async function folder() {
  try { return (await invoke("screenshot_folder")) ?? { dir: null, default: "" }; } catch { return { dir: null, default: "" }; }
}

function showFolder(dir) {
  const el = $("shot-dir");
  if (el) el.textContent = dir || "Not chosen yet: the first screenshot asks";
  const open = $("shot-open");
  if (open) open.disabled = !dir;
}

/** First use: where should screenshots go? Returns the folder, or null if cancelled. */
async function firstUse(def) {
  const v = await ask({
    title: "Where should screenshots go?",
    note: `Screenshots are saved as pictures (PNG) in one folder, easy to find to look at or to attach to an email for support. Suggested: ${def}. You can change it later in Settings › Display.`,
    fields: [{ key: "where", label: "Folder", type: "select", options: [["default", "TweedPlot-Screenshots on the Desktop"], ["pick", "Choose another folder…"]] }],
    ok: "Save the screenshot",
  });
  if (!v) return null;
  if (v.where === "pick") return invoke("screenshot_pick_folder");
  return invoke("screenshot_use_default");
}

async function openFolder() {
  try { await invoke("screenshot_open_folder"); } catch (e) { activity.start("Opening the screenshots folder").fail(String(e)); }
}

let busy = false;
async function take() {
  if (busy) return;
  busy = true;
  try {
    let { dir, default: def } = await folder();
    if (!dir) {
      try { dir = await firstUse(def); } catch (e) { activity.start("Choosing the screenshots folder").fail(String(e)); return; }
      if (!dir) return;
      showFolder(dir);
    }
    // nothing of the button's own in the picture: the tooltip goes, and the question box (first
    // use) has gone by the time two frames are drawn
    const tip = $("tip");
    if (tip) tip.hidden = true;
    document.activeElement?.blur?.();
    await frame();
    const path = await invoke("screenshot_take");
    const j = activity.note("Screenshot saved", { details: [path] });
    if (j?.el) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "link-btn";
      b.textContent = "Open folder";
      b.onclick = openFolder;
      j.el.querySelector(".act-result")?.after(b) ?? j.el.append(b);
    }
  } catch (e) {
    activity.start("Taking a screenshot").fail(String(e));
  } finally {
    busy = false;
  }
}

async function change() {
  try {
    const d = await invoke("screenshot_pick_folder");
    if (d) showFolder(d);
  } catch (e) { activity.start("Choosing the screenshots folder").fail(String(e)); }
}

export function setupScreenshots() {
  const btn = $("shot-btn");
  if (btn) {
    btn.onclick = take;
    // right-click: straight to the folder (or choose one, before the first screenshot)
    btn.oncontextmenu = async (e) => {
      e.preventDefault();
      const { dir } = await folder();
      if (dir) openFolder(); else change();
    };
  }
  $("shot-change")?.addEventListener("click", change);
  $("shot-open")?.addEventListener("click", openFolder);
  folder().then((f) => showFolder(f.dir));
  return { take };
}
