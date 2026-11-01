// Chart › Encrypted charts (S-63): this system's user permit, the Scheme Administrator's
// key, cell permits (PERMIT.TXT, kept per data server), importing exchange sets, the
// installed ENCs, and the S-63 messages (SSE codes).

import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);
const fmtDate = (d) => (d && d.length === 8 ? `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6)}` : d || "");
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** A destructive button: the first click asks (the button says so), the second does it. */
function twice(btn, ask, action) {
  if (btn.dataset.armed) { delete btn.dataset.armed; btn.textContent = btn.dataset.label; btn.classList.remove("warn"); action(); return; }
  btn.dataset.label = btn.textContent;
  btn.dataset.armed = "1";
  btn.textContent = ask;
  btn.classList.add("warn");
  setTimeout(() => { if (btn.dataset.armed) { delete btn.dataset.armed; btn.textContent = btn.dataset.label; btn.classList.remove("warn"); } }, 4000);
}

export function setupS63(map, { toast } = {}) {
  let status = null;
  let warned = false;

  /** S-63 10.8: a permanent warning while a cell with an expired permit (SSE 25) or
   *  that is not up to date (SSE 27) is on the chart. */
  function banner() {
    const el = $("s63-banner");
    const b = map.getBounds();
    const [w, s, e, n] = [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()];
    const shown = (status?.cells || []).filter((c) => c.warn && c.bbox && c.bbox[0] < e && c.bbox[2] > w && c.bbox[1] < n && c.bbox[3] > s);
    el.hidden = shown.length === 0;
    if (!shown.length) return;
    const lines = shown.slice(0, 3).map((c) => `<div>⚠ ${esc(c.warn)}</div>`);
    if (shown.length > 3) lines.push(`<div>… and ${shown.length - 3} more ENCs: see Chart › Encrypted charts</div>`);
    el.innerHTML = lines.join("");
  }
  map.on("moveend", banner);

  /** Show a report: [{ cell, level, text }] or a single line. */
  function report(lines, title) {
    const ul = $("s63-report");
    ul.innerHTML = "";
    const all = typeof lines === "string" ? [{ cell: "", level: "info", text: lines }] : lines;
    if (title) all.unshift({ cell: "", level: "info", text: title });
    for (const m of all) {
      const li = document.createElement("li");
      li.className = m.level;
      li.innerHTML = m.cell ? `<b>${esc(m.cell)}</b> ${esc(m.text)}` : esc(m.text);
      ul.append(li);
    }
  }
  const count = (lines, level) => lines.filter((m) => m.level === level).length;

  async function refresh() {
    try { status = await invoke("s63_status"); } catch { return; }
    if (!status) return;
    $("s63-inst").value = status.installation_code || "";
    $("s63-up").value = status.user_permit || "";
    $("s63-up").classList.toggle("bad", !status.hw_ok);
    const act = status.activation;
    $("s63-act-note").textContent = act?.ok ? `Activated ${fmtDate(act.issued)}${status.real_credentials ? "" : " (IHO test credentials)"}`
      : act ? `Activation not used: ${act.error}`
      : status.real_credentials ? "Not activated: send the installation code to Tweedsmarine for an activation code"
      : "Test build: the IHO test permit is used (for the S-64 test data)";
    $("s63-act-note").className = `grow wx-note ${act && !act.ok ? "bad" : !act && status.real_credentials ? "warn" : ""}`;
    $("s63-deact").hidden = !act;
    if (status.device_error) {
      $("s63-act-note").textContent = status.device_error;
      $("s63-act-note").className = "grow wx-note bad";
    }

    // who the licence is for: one vessel per licence, up to three computers on board
    const licRow = $("s63-lic-row"), lic = $("s63-lic");
    licRow.hidden = !act?.ok;
    if (act?.ok) {
      const own = (status.own_mmsi || "").trim();
      const differs = act.mmsi && own && own !== act.mmsi;
      lic.textContent = `Licensed to ${act.vessel || "(no vessel name)"}${act.mmsi ? `, MMSI ${act.mmsi}` : ""} · licence ${act.licence}, seat ${act.seat}`
        + (differs ? `. This boat's MMSI in Settings is ${own}: the licence is for one vessel, so if TweedPlot has moved to another boat, please ask Tweedsmarine for a licence for it.` : "");
      lic.className = `grow wx-note ${differs ? "warn" : ""}`;
    }
    const rel = status.released;
    $("s63-rel-box").hidden = !rel || !!act;
    if (rel && !act) {
      $("s63-rel").value = rel.code;
      $("s63-rel-note").textContent = `Seat ${rel.seat} of licence ${rel.licence}${rel.vessel ? ` (${rel.vessel})` : ""} given back ${fmtDate(rel.date)}. Send the release code to Tweedsmarine to use the seat on another computer.`;
    }

    const k = status.key, key = $("s63-key");
    key.textContent = !k ? "none installed (SSE 05)" : `${k.sa ? "IHO Scheme Administrator" : `${k.name}, not the IHO's (SSE 26)`}${k.not_after ? `, valid to ${fmtDate(k.not_after)}` : ""}`;
    key.className = !k ? "bad" : k.sa ? "" : "warn";
    key.title = k?.subject || "";

    // permits, by data server
    const box = $("s63-permit-list");
    box.innerHTML = "";
    const byDs = new Map();
    for (const p of status.permits) { if (!byDs.has(p.ds)) byDs.set(p.ds, []); byDs.get(p.ds).push(p); }
    let expired = 0, expiring = 0;
    for (const [ds, list] of byDs) {
      const head = document.createElement("div");
      head.className = "ds-head";
      head.innerHTML = `<span class="grow">Data server ${esc(ds)}: ${list.length} permit${list.length === 1 ? "" : "s"}</span><button title="Remove this data server's permits">Remove</button>`;
      const rm = head.querySelector("button");
      rm.onclick = () => twice(rm, "Sure? Remove", async () => {
        try { await invoke("s63_remove_permits", { ds }); report(`Permits from data server ${ds} removed: its ENCs can't be shown or updated until permits are installed again.`); } catch (e) { report([{ cell: "", level: "error", text: String(e) }]); }
        refresh();
      });
      const t = document.createElement("table");
      t.innerHTML = "<thead><tr><th>Cell</th><th>Expires</th><th>Status</th></tr></thead><tbody></tbody>";
      for (const p of list) {
        const tr = document.createElement("tr");
        tr.className = p.state;
        tr.innerHTML = `<td>${esc(p.cell)}</td><td>${esc(fmtDate(p.expiry))}</td><td>${esc(p.state === "ok" ? `OK · ${p.service}` : p.text)}</td>`;
        t.tBodies[0].append(tr);
        if (p.state === "expired") expired++; else if (p.state === "expiring") expiring++;
      }
      box.append(head, t);
    }
    const extra = [expired && `${expired} expired`, expiring && `${expiring} expiring within 30 days`].filter(Boolean).join(", ");
    $("s63-permit-sum").textContent = `Cell permits: ${status.permits.length}${extra ? ` (${extra})` : ""}`;
    $("s63-permit-details").hidden = status.permits.length === 0;

    // installed cells
    const rows = $("s63-cells").tBodies[0];
    rows.innerHTML = "";
    for (const c of status.cells) {
      const tr = document.createElement("tr");
      if (c.warn) { tr.className = "warn"; tr.title = c.warn; }
      tr.innerHTML = `<td>${esc(c.cell)}</td><td>${c.edition}</td><td>${c.update}</td><td>${esc(c.ds)}${c.warn ? " ⚠" : ""}</td>`;
      rows.append(tr);
    }
    const warnCells = status.cells.filter((c) => c.warn).length;
    $("s63-cell-sum").textContent = `Installed ENCs: ${status.cells.length}${warnCells ? ` (${warnCells} with warnings)` : ""}`;
    $("s63-cell-details").hidden = status.cells.length === 0;

    const list = $("s63-notes");
    list.innerHTML = "";
    for (const m of status.notes) {
      const li = document.createElement("li");
      li.textContent = `${m.cell}: ${m.text}`;
      li.title = m.file;
      list.append(li);
    }
    $("s63-note-sum").textContent = `Messages from opening the charts: ${status.notes.length}`;
    $("s63-note-details").hidden = status.notes.length === 0;
    banner();

    // say once per run when cells are shown with warnings or permits are running out
    if (!warned && (status.notes.length || expired || expiring)) {
      warned = true;
      const why = status.notes.length ? `${status.notes.length} ENC message${status.notes.length > 1 ? "s" : ""}` : extra;
      toast?.(`Encrypted charts: ${why}. See Chart › Encrypted charts`, 6000);
    }
  }

  $("s63-up-copy").onclick = async () => {
    try { await navigator.clipboard.writeText($("s63-up").value.trim()); report("User permit copied."); } catch { $("s63-up").select(); }
  };

  $("s63-inst-copy").onclick = async () => {
    try { await navigator.clipboard.writeText($("s63-inst").value.trim()); report("Installation code copied: send it to Tweedsmarine."); } catch { $("s63-inst").select(); }
  };

  $("s63-act-btn").onclick = async () => {
    const code = $("s63-act").value.trim();
    if (!code) { report([{ cell: "", level: "error", text: "Paste the activation code from Tweedsmarine first" }]); return; }
    try {
      const up = await invoke("s63_activate", { code });
      $("s63-act").value = "";
      report([{ cell: "", level: "ok", text: `Activated. User permit ${up}: give it to your chart supplier when ordering permits. Opening the charts again…` }]);
    } catch (e) { report([{ cell: "", level: "error", text: String(e) }]); }
    refresh();
  };

  $("s63-deact").onclick = () => twice($("s63-deact"), "Sure? Remove", async () => {
    try {
      const code = await invoke("s63_deactivate");
      report(code ? "Activation removed, and the charts decrypted with it deleted. Send the release code below to Tweedsmarine to free the seat."
        : "Activation removed.");
    } catch (e) { report([{ cell: "", level: "error", text: String(e) }]); }
    refresh();
  });

  $("s63-rel-copy").onclick = async () => {
    try { await navigator.clipboard.writeText($("s63-rel").value.trim()); report("Release code copied: send it to Tweedsmarine."); } catch { $("s63-rel").select(); }
  };

  $("s63-key-btn").onclick = () => $("s63-key-file").click();
  $("s63-key-file").onchange = async (ev) => {
    const f = ev.target.files?.[0];
    ev.target.value = "";
    if (!f) return;
    try {
      const bytes = Array.from(new Uint8Array(await f.arrayBuffer()));
      report(await invoke("s63_install_key", { name: f.name, bytes }));
    } catch (e) { report([{ cell: "", level: "error", text: `${f.name}: ${e}` }]); }
    refresh();
  };

  $("s63-permits").onclick = () => $("s63-permit-file").click();
  $("s63-permit-file").onchange = async (ev) => {
    const f = ev.target.files?.[0];
    ev.target.value = "";
    if (!f) return;
    try {
      const done = await invoke("s63_install_permits", { text: await f.text() });
      report(done, `${count(done, "ok") + count(done, "warn")} of ${done.length} permits installed from ${f.name}`);
    } catch (e) { report([{ cell: "", level: "error", text: `${f.name}: ${e}` }]); }
    refresh();
  };

  $("s63-import").onclick = async () => {
    $("s63-import").disabled = true;
    report("Choose the exchange set's folder…");
    try {
      const r = await invoke("s63_import_exchange_set");
      if (r.cancelled) { report([]); return; }
      const lines = r.report;
      const errs = count(lines, "error"), warns = count(lines, "warn");
      report(lines, `Imported: ${count(lines, "ok")} cells installed or updated${errs ? `, ${errs} errors` : ""}${warns ? `, ${warns} warnings` : ""}. Opening the charts again…`);
      if (errs || warns) toast?.(`ENC import: ${errs} errors, ${warns} warnings. See Chart › Encrypted charts`, 6000);
    } catch (e) { report([{ cell: "", level: "error", text: String(e) }]); } finally { $("s63-import").disabled = false; }
    refresh();
  };

  $("s63-remove-cells").onclick = () => twice($("s63-remove-cells"), "Sure? Remove all", async () => {
    try { await invoke("s63_remove_cells", { cell: "" }); report("All installed ENCs removed (the permits and key stay; import the exchange sets again to get them back)."); } catch (e) { report([{ cell: "", level: "error", text: String(e) }]); }
    refresh();
  });

  window.addEventListener("racenav-charts", refresh);
  refresh();
  return { refresh };
}
