// Chart › Encrypted charts (S-63): this system's user permit, installing cell permits
// (PERMIT.TXT), importing exchange sets, and the S-63 messages (SSE codes) from the
// last time the charts were opened.

import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);
const fmtDate = (d) => (d && d.length === 8 ? `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6)}` : d || "");

export function setupS63({ toast } = {}) {
  let status = null;
  let warned = false;

  const result = (html, kind = "") => { const el = $("s63-result"); el.className = `wx-note ${kind}`; el.innerHTML = html; };
  const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

  async function refresh() {
    try { status = await invoke("s63_status"); } catch { return; }
    if (!status) return;
    const up = $("s63-up");
    if (document.activeElement !== up) up.value = status.user_permit;
    up.classList.toggle("bad", !status.hw_ok);
    $("s63-up-note").textContent = status.test
      ? "IHO test permit (for the S-64 test data; TweedPlot has no manufacturer key of its own yet)"
      : status.hw_ok ? "Your user permit" : "SSE 17 - Invalid user permit";

    const rows = $("s63-table").querySelector("tbody");
    rows.innerHTML = "";
    let expired = 0, expiring = 0, bad = 0;
    for (const p of status.permits) {
      const tr = document.createElement("tr");
      tr.className = p.state;
      const st = p.state === "ok" ? `OK · ${p.service}` : p.text;
      tr.innerHTML = `<td>${esc(p.cell)}</td><td>${esc(fmtDate(p.expiry))}</td><td>${esc(st)}</td>`;
      tr.title = `From ${p.file}${p.date ? ` (${p.date})` : ""}${p.server ? `\nData server ${p.server}, edition ${p.edition}` : ""}`;
      rows.append(tr);
      if (p.state === "expired") expired++; else if (p.state === "expiring") expiring++; else if (p.state === "bad") bad++;
    }
    const n = status.permits.length;
    const extra = [expired && `${expired} expired`, expiring && `${expiring} expiring within 30 days`, bad && `${bad} unusable`].filter(Boolean).join(", ");
    $("s63-permit-sum").textContent = n ? `Cell permits: ${n}${extra ? ` (${extra})` : ""}` : "Cell permits: none";
    $("s63-permit-details").hidden = n === 0;

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

    // tell once per run when cells can't be shown or permits are running out
    if (!warned && (status.notes.length || expired || expiring)) {
      warned = true;
      const why = status.notes.length ? `${status.notes.length} ENC message${status.notes.length > 1 ? "s" : ""}` : extra;
      toast?.(`Encrypted charts: ${why}. See Chart › Encrypted charts`, 6000);
    }
  }

  $("s63-up-copy").onclick = async () => {
    try { await navigator.clipboard.writeText($("s63-up").value.trim()); result("User permit copied.", "good"); } catch { $("s63-up").select(); }
  };

  $("s63-up-set").onclick = async () => {
    try {
      const p = await invoke("s63_set_user_permit", { permit: $("s63-up").value });
      result(`Using user permit ${esc(p)}. Opening the charts again…`, "good");
    } catch (e) { result(esc(e), "bad"); }
    refresh();
  };

  $("s63-permits").onclick = () => $("s63-permit-file").click();
  $("s63-permit-file").onchange = async (ev) => {
    const f = ev.target.files?.[0];
    ev.target.value = "";
    if (!f) return;
    try {
      const text = await f.text();
      const done = await invoke("s63_install_permits", { text });
      const ok = done.filter((d) => d.text.startsWith("installed")).length;
      result(`${ok} of ${done.length} permits installed from ${esc(f.name)}.` +
        (done.length ? `<ul>${done.map((d) => `<li>${esc(d.cell)}: ${esc(d.text)}</li>`).join("")}</ul>` : ""), ok === done.length ? "good" : "warn");
    } catch (e) { result(`${esc(f.name)}: ${esc(e)}`, "bad"); }
    refresh();
  };

  $("s63-import").onclick = async () => {
    $("s63-import").disabled = true;
    result("Choose the exchange set's folder…");
    try {
      const r = await invoke("s63_import_exchange_set");
      if (r.cancelled) { result(""); return; }
      const s = r.serial;
      const what = s ? `${s.kind} exchange set from data server ${s.server}, ${s.week.replace(/^WK/, "week ")} (${fmtDate(s.date)})` : "Exchange set";
      result(`${esc(what)}: ${r.cells} cells (${r.encrypted} encrypted), ${r.files} files copied to ${esc(r.to)}.` +
        (r.encrypted && !r.permit_file ? " It has no PERMIT.TXT: install the permits from your supplier." : "") + " Opening the charts again…", "good");
    } catch (e) { result(esc(e), "bad"); } finally { $("s63-import").disabled = false; }
    refresh();
  };

  window.addEventListener("racenav-charts", refresh);
  refresh();
  return { refresh };
}
