// A small question box in the middle of the screen: a passphrase, or a few choices before
// a job (what to put in a library file). Enter answers, Esc cancels. Never used for alerts.
//
//   const v = await ask({ title, note, fields: [{ key, label, type: "password" | "text" | "checkbox", value }], ok: "Unlock", check: (v) => "error" | "" });
//   // v: { key: value } or null (cancelled)

export function ask({ title, note = "", fields = [], ok = "OK", cancel = "Cancel", check = () => "" }) {
  return new Promise((resolve) => {
    const back = document.createElement("div");
    back.className = "ask-back";
    const box = document.createElement("form");
    box.className = "ask";
    box.innerHTML = `<b class="ask-title"></b><div class="wx-note ask-note"></div><div class="ask-fields"></div><div class="ask-err" hidden></div>
      <div class="wx-row"><span class="grow"></span><button type="submit" class="on"></button><button type="button" class="ask-cancel"></button></div>`;
    box.querySelector(".ask-title").textContent = title;
    box.querySelector(".ask-note").textContent = note;
    box.querySelector(".ask-note").hidden = !note;
    box.querySelector('button[type="submit"]').textContent = ok;
    box.querySelector(".ask-cancel").textContent = cancel;
    const inputs = {};
    const wrap = box.querySelector(".ask-fields");
    for (const f of fields) {
      const row = document.createElement("label");
      row.className = f.type === "checkbox" ? "ask-check" : "ask-field";
      const inp = document.createElement("input");
      inp.type = f.type ?? "text";
      if (f.type === "checkbox") inp.checked = !!f.value; else inp.value = f.value ?? "";
      if (f.placeholder) inp.placeholder = f.placeholder;
      const span = document.createElement("span");
      span.textContent = f.label;
      if (f.type === "checkbox") row.append(inp, span); else row.append(span, inp);
      if (f.showIf) row.dataset.showIf = f.showIf;
      if (f.title) row.title = f.title;
      wrap.append(row);
      inputs[f.key] = inp;
    }
    const values = () => Object.fromEntries(Object.entries(inputs).map(([k, i]) => [k, i.type === "checkbox" ? i.checked : i.value]));
    // fields shown only when a tick box is ticked (the passphrase under "Protect it")
    const sync = () => { for (const r of wrap.querySelectorAll("[data-show-if]")) r.hidden = !inputs[r.dataset.showIf]?.checked; };
    for (const i of Object.values(inputs)) i.addEventListener("change", sync);
    sync();
    const done = (v) => { back.remove(); document.removeEventListener("keydown", esc, true); resolve(v); };
    const esc = (e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); done(null); } };
    document.addEventListener("keydown", esc, true);
    box.querySelector(".ask-cancel").onclick = () => done(null);
    box.onsubmit = (e) => {
      e.preventDefault();
      const v = values();
      const err = check(v);
      const el = box.querySelector(".ask-err");
      el.hidden = !err;
      el.textContent = err || "";
      if (!err) done(v);
    };
    back.append(box);
    document.body.append(back);
    setTimeout(() => (Object.values(inputs).find((i) => i.type !== "checkbox") ?? box.querySelector('button[type="submit"]')).focus(), 0);
  });
}

/** Ask for a protected library's passphrase. */
export function askPassphrase(name, retry = false) {
  return ask({
    title: `${name} is protected`,
    note: retry ? "That passphrase didn't open it. Try again (capitals count)." : "Type the passphrase it was protected with (ask whoever gave it to you).",
    fields: [{ key: "pass", label: "Passphrase", type: "password" }],
    ok: "Unlock",
    check: (v) => (v.pass ? "" : "Type the passphrase."),
  }).then((v) => v?.pass ?? null);
}
