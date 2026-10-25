// The list of monitor windows in the Instruments drawer: open any number showing any view,
// bring one forward, close it. Windows that were open when the app last closed reopen
// at startup, in the same place and colours (windowstate.js).
import { invoke } from "@tauri-apps/api/core";
import { VIEWS } from "./views.js";
import { PRIMARY } from "./role.js";
import { readWindows, patchWindow, monitorsToReopen } from "./windowstate.js";

const $ = (id) => document.getElementById(id);

export function setupMonitors(toast = () => {}) {
  const sel = $("mon-new-view");
  for (const v of VIEWS) sel.add(new Option(v.name, v.id));
  const nameOf = (id) => VIEWS.find((v) => v.id === id)?.name ?? id;

  async function open(view, label) {
    try { return await invoke("monitor_open", { view, label }); } catch (e) { toast(`Monitor window: ${e}`, 4000); return null; }
  }

  let shown = "";
  async function refresh() {
    let labels = [];
    try { labels = await invoke("monitor_list"); } catch { return; }
    const g = readWindows();
    const rows = labels.map((l) => [l, g[l]?.view ?? "nav"]);
    const sig = JSON.stringify(rows);
    if (sig === shown) return;
    shown = sig;
    const ul = $("mon-list");
    ul.innerHTML = "";
    if (!rows.length) ul.innerHTML = `<li class="empty">No monitor windows are open.</li>`;
    for (const [label, view] of rows) {
      const li = document.createElement("li");
      li.innerHTML = `<div class="main"><b></b><small></small></div><button data-act="show">Show</button><button data-act="close">Close</button>`;
      li.querySelector("b").textContent = `Monitor ${label.replace("monitor-", "")}`;
      li.querySelector("small").textContent = nameOf(view);
      li.querySelector('[data-act="show"]').onclick = () => invoke("monitor_focus", { label }).catch(() => {});
      li.querySelector('[data-act="close"]').onclick = async () => {
        patchWindow(label, { open: false });
        await invoke("monitor_close", { label }).catch(() => {});
        setTimeout(refresh, 300);
      };
      ul.append(li);
    }
  }

  $("mon-new").onclick = async () => {
    const label = await open(sel.value);
    if (label) patchWindow(label, { view: sel.value, open: true });
    setTimeout(refresh, 500);
  };

  if (PRIMARY) {
    (async () => {
      const already = await invoke("monitor_list").catch(() => []);
      for (const m of monitorsToReopen()) if (!already.includes(m.label)) await open(m.view, m.label);
      refresh();
    })();
  } else refresh();
  setInterval(refresh, 3000);
}
