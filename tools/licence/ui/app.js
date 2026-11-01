// TweedPlot Licence Manager page. Everything goes through the Rust commands (lm_*).
"use strict";

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const $ = (id) => document.getElementById(id);
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const fmtDate = (d) => (d && d.length === 8 ? `${d.slice(6)}/${d.slice(4, 6)}/${d.slice(0, 4)}` : d || "");

let status = null;
let licences = [];
let customers = [];
let current = null;

function toast(text, ms = 2500) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), ms);
}

async function copy(text, what) {
  try { await navigator.clipboard.writeText(text); toast(`${what} copied`); } catch { toast("Couldn't copy: select it and press Ctrl+C"); }
}

function seatbar(used, of) {
  let h = "";
  for (let i = 0; i < Math.max(of, used); i++) h += `<i class="${i < used ? (i >= of ? "over" : "used") : ""}"></i>`;
  return `<span class="seatbar" title="${used} of ${of} seats in use">${h}</span>`;
}

// ---- status, banners --------------------------------------------------------------

async function loadStatus() {
  status = await invoke("lm_status");
  const mc = $("mode-chip");
  mc.textContent = status.test ? "TEST MODE · IHO test credentials" : status.has_credentials ? `M_ID ${status.m_id}` : "No credentials yet";
  mc.className = `chip ${status.test ? "test" : status.has_credentials ? "real" : "warn"}`;
  const bc = $("backup-chip");
  if (!status.last_backup) { bc.textContent = status.licences ? "Never backed up" : "No backup yet"; bc.className = `chip ${status.licences ? "warn" : ""}`; }
  else {
    const days = Math.floor((Date.now() - Date.parse(status.last_backup.replace(" ", "T") + "Z")) / 86400000);
    bc.textContent = `Backed up ${days <= 0 ? "today" : days === 1 ? "yesterday" : `${days} days ago`}${status.changes_since_backup ? ` · ${status.changes_since_backup} changes since` : ""}`;
    bc.className = `chip ${status.changes_since_backup && days > 7 ? "warn" : ""}`;
  }
  bc.onclick = () => openSettings("backup");
  bc.style.cursor = "pointer";

  const b = [];
  if (status.old_files.length) b.push(["bad", `The old command-line tool left secrets in the OEM folder in plain files (${status.old_files.join(", ")}).`, "Move them into the vault", async () => {
    try { const done = await invoke("lm_import_old"); toast(`Moved into the vault: ${done.join(", ") || "nothing new"}`, 4000); } catch (e) { toast(String(e), 5000); }
    refresh();
  }]);
  if (!status.test && !status.has_credentials) b.push(["", "Tweedsmarine's own credentials aren't in the vault yet. Until the IHO sends them, use test mode.", "Settings", () => openSettings("cred")]);
  if (!status.test && status.has_credentials && !status.pub_matches) b.push(["bad", "signing.pub in the OEM folder isn't the vault's key: TweedPlot would be built with the wrong one.", "Write it again", () => openSettings("cred")]);
  if (!status.test && status.has_credentials && !status.build_matches) b.push(["", "This Licence Manager was built before the credentials were set: rebuild it and TweedPlot on this PC so both use the vault's signing key."]);
  if (!status.protected) b.push(["", "The vault key is a plain file here (no Windows DPAPI on this system): use this only for development."]);
  if (status.licences && !status.last_backup) b.push(["", "There's no passphrase backup of the vault yet. Losing it means losing every customer's HW_ID.", "Back up now", () => openSettings("backup")]);
  const box = $("banners");
  box.innerHTML = "";
  for (const [cls, text, btn, fn] of b) {
    const d = document.createElement("div");
    d.className = `banner ${cls}`;
    d.innerHTML = `<span>${esc(text)}</span>`;
    if (btn) { const x = document.createElement("button"); x.textContent = btn; x.onclick = fn; d.append(x); }
    box.append(d);
  }
}

// ---- list -------------------------------------------------------------------------

function renderList() {
  const q = $("search").value.trim().toLowerCase();
  const all = $("show-revoked").checked;
  const shown = licences.filter((l) => (all || l.status !== "revoked")
    && (!q || [l.vessel, l.customer.name, l.customer.email, l.mmsi, l.call_sign, String(l.id), l.user_permit].some((v) => String(v).toLowerCase().includes(q))));
  $("list-count").textContent = `${shown.length} licence${shown.length === 1 ? "" : "s"}`;
  const ul = $("list");
  ul.innerHTML = "";
  for (const l of shown) {
    const li = document.createElement("li");
    li.className = `${l.status}${current?.id === l.id ? " on" : ""}`;
    li.innerHTML = `<span class="v">${esc(l.vessel)}</span>${seatbar(l.active, l.seats)}
      <span class="c">${esc(l.customer.name)}${l.mmsi ? ` · ${esc(l.mmsi)}` : ""}</span><span class="n">#${l.id}</span>`;
    li.onclick = () => show(l.id);
    ul.append(li);
  }
  if (!licences.length) ul.innerHTML = `<li class="c" style="cursor:default">No licences yet.</li>`;
}

async function refresh() {
  await loadStatus();
  [licences, customers] = await Promise.all([invoke("lm_licences"), invoke("lm_customers")]);
  renderList();
  if (current) show(current.id); else $("detail").querySelector(".empty") || clearDetail();
}

function clearDetail() {
  current = null;
  $("detail").innerHTML = `<div class="empty"><p><b>No licence chosen.</b></p></div>`;
}

// ---- detail -----------------------------------------------------------------------

async function show(id) {
  let r;
  try { r = await invoke("lm_licence", { id }); } catch { clearDetail(); renderList(); return; }
  const l = r.licence;
  current = l;
  renderList();
  const seats = l.seat_list.map((s) => `
    <tr class="${s.status === "active" ? "" : "old"}">
      <td><b>${s.seat}</b></td>
      <td>${esc(s.label) || "<span class='note'>—</span>"}</td>
      <td>${esc(fmtDate(s.issued))}</td>
      <td><span class="st ${s.status}">${esc(s.status)}</span>${s.ended ? `<div class="note">${esc(fmtDate(s.ended))}: ${esc(s.end_reason)}</div>` : ""}</td>
      <td><span class="mono trunc" title="${esc(s.installation_code)}">${esc(s.installation_code)}</span></td>
      <td><div class="actions">${s.status === "active" ? `
        <button class="small" data-copy="${s.id}" title="Copy this seat's activation code">Copy code</button>
        <button class="small" data-mail="${s.id}" title="Email the activation code again">Email</button>
        <button class="small danger" data-end="${s.id}" title="End this seat without a release code (computer lost or broken)">End…</button>` : ""}</div></td>
    </tr>`).join("");
  const log = r.log.map((e) => `<tr><td>${esc(e.at)}</td><td>${esc(e.action)}</td><td>${e.seat ?? ""}</td><td>${esc(e.detail)}</td></tr>`).join("");
  $("detail").innerHTML = `
    <div class="head">
      <div class="grow"><h1>${esc(l.vessel)}</h1>
        <div class="sub">Licence #${l.id} · ${esc(l.customer.name)} · since ${esc(fmtDate(l.created))}</div></div>
      <span class="badge ${l.status}">${esc(l.status)}</span>
    </div>
    <div class="card">
      <h3>Licence</h3>
      <dl class="kv">
        <dt>Vessel</dt><dd>${esc(l.vessel)}</dd>
        <dt>MMSI</dt><dd>${esc(l.mmsi) || "—"}</dd>
        <dt>Call sign</dt><dd>${esc(l.call_sign) || "—"}</dd>
        <dt>Seats</dt><dd>${seatbar(l.active, l.seats)} ${l.active} of ${l.seats} in use</dd>
        <dt>User permit</dt><dd><span class="mono">${esc(l.user_permit)}</span><button class="small" id="d-up">Copy</button></dd>
        <dt>Customer</dt><dd>${esc(l.customer.name)}${l.customer.email ? ` · <a href="#" id="d-mailto">${esc(l.customer.email)}</a>` : ""}${l.customer.phone ? ` · ${esc(l.customer.phone)}` : ""}</dd>
        ${l.notes ? `<dt>Notes</dt><dd>${esc(l.notes)}</dd>` : ""}
      </dl>
      <div class="buttons" style="margin-top:12px">
        ${l.status === "active" ? `<button id="d-revoke" class="danger">Revoke…</button>` : ""}
        <span class="grow"></span>
        <button id="d-edit">Edit…</button>
        ${l.status === "active" ? `<button id="d-seat" class="primary">Add a seat…</button>` : ""}
      </div>
    </div>
    <div class="card"><h3>Seats (computers)</h3>
      <table><thead><tr><th>Seat</th><th>Computer</th><th>Issued</th><th>Status</th><th>Installation code</th><th></th></tr></thead><tbody>${seats || `<tr><td colspan="6" class="note">No seats yet.</td></tr>`}</tbody></table>
    </div>
    <div class="card"><h3>History</h3>
      <table class="log"><thead><tr><th>When (UTC)</th><th>What</th><th>Seat</th><th>Detail</th></tr></thead><tbody>${log}</tbody></table>
    </div>`;
  $("d-up").onclick = () => copy(l.user_permit, "User permit");
  if ($("d-mailto")) $("d-mailto").onclick = (e) => { e.preventDefault(); invoke("lm_mail", { to: l.customer.email, subject: `TweedPlot licence for ${l.vessel}`, body: "" }); };
  $("d-edit").onclick = () => openEdit(l);
  if ($("d-seat")) $("d-seat").onclick = () => openIssue(l.id);
  if ($("d-revoke")) $("d-revoke").onclick = () => ask(`Revoke licence #${l.id} (${l.vessel})`,
    "No new seats can be issued on it and its active seats are marked revoked. TweedPlot works offline, so computers already activated keep working: tell the customer.",
    "Revoke", async (why) => { await invoke("lm_revoke", { id: l.id, why }); refresh(); });
  for (const b of document.querySelectorAll("[data-copy]")) b.onclick = () => copy(l.seat_list.find((s) => s.id == b.dataset.copy).activation_code, "Activation code");
  for (const b of document.querySelectorAll("[data-mail]")) b.onclick = () => {
    const s = l.seat_list.find((x) => x.id == b.dataset.mail);
    mail(l.customer.email, `TweedPlot activation for ${l.vessel} (seat ${s.seat})`, emailBody(l, s.seat, s.activation_code));
  };
  for (const b of document.querySelectorAll("[data-end]")) b.onclick = () => {
    const s = l.seat_list.find((x) => x.id == b.dataset.end);
    ask(`End seat ${s.seat} of ${l.vessel}`, "For a computer that is lost, broken or wiped and can't give a release code. The seat becomes free.", "End the seat",
      async (why) => { await invoke("lm_end_seat", { seatId: s.id, why }); refresh(); });
  };
}

function emailBody(l, seat, code) {
  const first = (l.customer.name || "").split(/\s+/)[0] || "there";
  return `Kia ora ${first},\r\n\r\nHere is the activation code for TweedPlot on ${l.vessel}, seat ${seat} of ${l.seats}:\r\n\r\n${code}\r\n\r\n`
    + "In TweedPlot, open Chart › Encrypted charts, paste the code into Activation code and press Activate. It only works on the computer whose installation code you sent.\r\n\r\n"
    + `Your user permit is ${l.user_permit}. Give it to your chart supplier when you order ENC permits.\r\n\r\nNgā mihi,\r\nChris\r\nTweedsmarine · tweedsmarine.nz\r\n`;
}

async function mail(to, subject, body) {
  try { await invoke("lm_mail", { to: to || "", subject, body }); } catch (e) { toast(String(e), 4000); }
}

// ---- ask for a reason -------------------------------------------------------------

function ask(title, text, verb, fn) {
  $("why-title").textContent = title;
  $("why-text").textContent = text;
  $("why-go").textContent = verb;
  $("why-input").value = "";
  $("why-err").textContent = "";
  const d = $("dlg-why");
  $("why-form").onsubmit = async (e) => {
    if (e.submitter?.value !== "go") return;
    e.preventDefault();
    try { await fn($("why-input").value); d.close(); } catch (err) { $("why-err").textContent = String(err); }
  };
  d.showModal();
}

// ---- issue ------------------------------------------------------------------------

function openIssue(licenceId) {
  $("iss-code").value = "";
  $("iss-code-note").textContent = "";
  $("iss-label").value = "";
  $("iss-reason").value = "";
  $("iss-over").hidden = true;
  $("iss-err").textContent = "";
  const sel = $("iss-licence");
  sel.innerHTML = "";
  for (const l of licences.filter((x) => x.status === "active")) sel.append(new Option(`#${l.id} ${l.vessel} — ${l.customer.name} (${l.active}/${l.seats} seats)`, l.id));
  const cs = $("iss-cust");
  cs.innerHTML = "";
  cs.append(new Option("A new customer", ""));
  for (const c of customers) cs.append(new Option(c.name + (c.email ? ` (${c.email})` : ""), c.id));
  for (const id of ["iss-cname", "iss-cemail", "iss-cphone", "iss-vessel", "iss-mmsi", "iss-call"]) $(id).value = "";
  $("iss-seats").value = 3;
  const existing = licenceId != null || licences.some((l) => l.status === "active");
  document.querySelector(`input[name=iss-which][value=${existing ? "existing" : "new"}]`).checked = true;
  if (licenceId != null) sel.value = licenceId;
  syncIssue();
  $("dlg-issue").showModal();
  $("iss-code").focus();
}

function syncIssue() {
  const isNew = document.querySelector("input[name=iss-which]:checked").value === "new";
  $("iss-licence").hidden = isNew;
  $("iss-new").hidden = !isNew;
  const newCust = !$("iss-cust").value;
  for (const el of document.querySelectorAll(".newcust")) el.hidden = !newCust;
}
for (const r of document.querySelectorAll("input[name=iss-which]")) r.onchange = syncIssue;
$("iss-cust").onchange = syncIssue;
$("iss-licence").onchange = () => { $("iss-over").hidden = true; };

let codeTimer;
$("iss-code").oninput = () => {
  clearTimeout(codeTimer);
  codeTimer = setTimeout(async () => {
    const n = $("iss-code-note");
    const code = $("iss-code").value.trim();
    if (!code) { n.textContent = ""; return; }
    try {
      const info = await invoke("lm_code_info", { code });
      n.className = info.in_use.length ? "note warn" : "note ok";
      n.textContent = info.in_use.length
        ? `A whole code. This computer already has ${info.in_use.map(([id, v, seat]) => `seat ${seat} on #${id} ${v}`).join(", ")}.`
        : "A whole code, for a computer not yet in the register.";
    } catch (e) { n.className = "note bad"; n.textContent = String(e); }
  }, 250);
};

$("issue-form").onsubmit = async (e) => {
  if (e.submitter?.value !== "go") return;
  e.preventDefault();
  const isNew = document.querySelector("input[name=iss-which]:checked").value === "new";
  const cust = $("iss-cust").value;
  const req = {
    installation_code: $("iss-code").value.trim(),
    licence_id: isNew ? null : Number($("iss-licence").value) || null,
    new_licence: isNew ? {
      customer_id: cust ? Number(cust) : null,
      customer_name: $("iss-cname").value, email: $("iss-cemail").value, phone: $("iss-cphone").value,
      vessel: $("iss-vessel").value, mmsi: $("iss-mmsi").value.trim(), call_sign: $("iss-call").value,
      seats: Number($("iss-seats").value) || 3,
    } : null,
    label: $("iss-label").value,
    over_limit_reason: $("iss-over").hidden ? null : $("iss-reason").value,
  };
  try {
    const r = await invoke("lm_issue", { req });
    $("dlg-issue").close();
    current = { id: r.licence_id };
    await refresh();
    showResult(r);
  } catch (err) {
    const t = String(err);
    $("iss-err").textContent = t;
    if (t.includes("seats on licence") && t.includes("in use")) { $("iss-over").hidden = false; $("iss-reason").focus(); }
  }
};

function showResult(r) {
  $("res-title").textContent = `Activation code: licence #${r.licence_id}, seat ${r.seat} (${r.how})`;
  $("res-warn").innerHTML = r.warnings.map((w) => `<div class="msg warn">${esc(w)}</div>`).join("");
  $("res-code").value = r.activation_code;
  $("res-up").value = r.user_permit;
  $("res-copy").onclick = () => copy(r.activation_code, "Activation code");
  $("res-copy-mail").onclick = () => copy(`${r.email_subject}\r\n\r\n${r.email_body}`, "Email text");
  $("res-mail").onclick = () => mail(r.email_to, r.email_subject, r.email_body);
  $("dlg-result").showModal();
}

// ---- release ----------------------------------------------------------------------

$("btn-release").onclick = () => { $("rel-code").value = ""; $("rel-msg").innerHTML = ""; $("dlg-release").showModal(); $("rel-code").focus(); };
$("release-form").onsubmit = async (e) => {
  if (e.submitter?.value !== "go") return;
  e.preventDefault();
  try {
    const m = await invoke("lm_release", { code: $("rel-code").value.trim() });
    $("rel-msg").innerHTML = `<div class="msg ok">${esc(m)}</div>`;
    refresh();
  } catch (err) { $("rel-msg").innerHTML = `<div class="msg bad">${esc(err)}</div>`; }
};

// ---- edit licence -----------------------------------------------------------------

function openEdit(l) {
  $("ed-vessel").value = l.vessel; $("ed-mmsi").value = l.mmsi; $("ed-call").value = l.call_sign;
  $("ed-seats").value = l.seats; $("ed-notes").value = l.notes;
  $("ed-cname").value = l.customer.name; $("ed-cemail").value = l.customer.email; $("ed-cphone").value = l.customer.phone;
  $("ed-err").textContent = "";
  $("edit-form").onsubmit = async (e) => {
    if (e.submitter?.value !== "go") return;
    e.preventDefault();
    try {
      await invoke("lm_update_licence", { id: l.id, vessel: $("ed-vessel").value, mmsi: $("ed-mmsi").value.trim(), callSign: $("ed-call").value, seats: Number($("ed-seats").value) || 1, notes: $("ed-notes").value });
      const c = l.customer;
      if (c.name !== $("ed-cname").value || c.email !== $("ed-cemail").value || c.phone !== $("ed-cphone").value) {
        await invoke("lm_update_customer", { customer: { ...c, name: $("ed-cname").value, email: $("ed-cemail").value, phone: $("ed-cphone").value } });
      }
      $("dlg-edit").close();
      refresh();
    } catch (err) { $("ed-err").textContent = String(err); }
  };
  $("dlg-edit").showModal();
}

// ---- settings ---------------------------------------------------------------------

function tab(name) {
  for (const b of document.querySelectorAll(".tabs button")) b.classList.toggle("on", b.dataset.tab === name);
  for (const p of document.querySelectorAll("[data-pane]")) p.hidden = p.dataset.pane !== name;
  if (name === "log") invoke("lm_log", { limit: 500 }).then((rows) => {
    $("log-all").innerHTML = rows.map((e) => `<tr><td>${esc(e.at)}</td><td>${esc(e.action)}</td><td>${e.licence_id ?? ""}</td><td>${e.seat ?? ""}</td><td>${esc(e.detail)}</td></tr>`).join("");
  });
}
for (const b of document.querySelectorAll(".tabs button")) b.onclick = () => tab(b.dataset.tab);

function fillSettings() {
  document.querySelector(`input[name=mode][value=${status.test ? "test" : "real"}]`).checked = true;
  $("cred-box").hidden = status.test;
  $("cred-mid").value = status.test ? "" : status.m_id;
  $("cred-mkey").value = "";
  $("cred-mkey").placeholder = status.has_credentials ? "••••• (in the vault)" : "";
  $("cred-state").textContent = status.has_credentials ? `Set: M_ID ${status.m_id}. The M_KEY can only be changed while there are no licences.` : "Not set yet: they come from the IHO with the OEM agreement.";
  $("cred-err").textContent = "";
  $("vault-info").innerHTML = `
    <dt>OEM folder</dt><dd class="mono">${esc(status.dir)}</dd>
    <dt>Vault key</dt><dd>${status.protected ? "Windows DPAPI, this Windows user" : "plain file (development only)"}</dd>
    <dt>signing.pub</dt><dd>${status.test ? "test key" : !status.has_credentials ? "not written yet" : status.pub_matches ? "matches the vault" : "<b>doesn't match the vault</b>"}</dd>
    <dt>This program's build</dt><dd>${!status.test && !status.has_credentials ? "—" : status.build_matches ? "has the vault's public key" : "built with another key: rebuild"}${status.build_real ? "" : " (test build)"}</dd>
    <dt>Licences</dt><dd>${status.licences}, ${status.active_seats} seats in use</dd>`;
}

function openSettings(which = "cred") {
  fillSettings();
  tab(which);
  $("bk-msg").innerHTML = "";
  $("dlg-settings").showModal();
}
$("btn-settings").onclick = () => openSettings();

for (const r of document.querySelectorAll("input[name=mode]")) r.onchange = async () => {
  const test = document.querySelector("input[name=mode]:checked").value === "test";
  try { status = await invoke("lm_set_mode", { test }); current = null; clearDetail(); await refresh(); fillSettings(); }
  catch (e) { toast(String(e), 5000); }
};

$("cred-go").onclick = async () => {
  try {
    await invoke("lm_set_credentials", { mId: $("cred-mid").value, mKey: $("cred-mkey").value });
    await refresh();
    fillSettings();
    toast("Credentials saved. Rebuild TweedPlot and the Licence Manager.", 5000);
  } catch (e) { $("cred-err").textContent = String(e); }
};

$("bk-go").onclick = async () => {
  const p = $("bk-pass").value;
  if (p !== $("bk-pass2").value) { $("bk-msg").innerHTML = `<div class="msg bad">The two passphrases differ</div>`; return; }
  try {
    const path = await invoke("lm_export_backup", { pass: p });
    if (path) { $("bk-msg").innerHTML = `<div class="msg ok">Saved and checked: ${esc(path)}</div>`; $("bk-pass").value = $("bk-pass2").value = ""; refresh(); }
  } catch (e) { $("bk-msg").innerHTML = `<div class="msg bad">${esc(e)}</div>`; }
};

$("rs-go").onclick = async () => {
  try {
    const path = await invoke("lm_restore_backup", { pass: $("rs-pass").value });
    if (path) { $("bk-msg").innerHTML = `<div class="msg ok">Restored from ${esc(path)}</div>`; $("rs-pass").value = ""; current = null; clearDetail(); refresh(); }
  } catch (e) { $("bk-msg").innerHTML = `<div class="msg bad">${esc(e)}</div>`; }
};

// ---- header -----------------------------------------------------------------------

$("btn-issue").onclick = () => openIssue(current?.id ?? null);
$("btn-register").onclick = async () => {
  try { const p = await invoke("lm_export_register"); if (p) toast(`Register saved: ${p}`, 4000); } catch (e) { toast(String(e), 5000); }
};
$("search").oninput = renderList;
$("show-revoked").onchange = renderList;

refresh().catch((e) => {
  document.body.insertAdjacentHTML("afterbegin", `<div class="banner bad" style="padding:10px 16px;background:#fde4e1">The vault couldn't be opened: ${esc(e)}</div>`);
});
