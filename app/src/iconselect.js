// A drop-down with a picture in front of each choice (native <select> options can't
// show images). The <select> stays in the form, hidden, and keeps the value, so
// forms and code that read or set it work as before; call refresh() after setting
// its value or options from code.

/** ImageData (or a canvas) → a data URL, trimmed of its clear margin. */
export function imageUrl(im) {
  if (!im) return "";
  const c = document.createElement("canvas");
  c.width = im.width; c.height = im.height;
  const g = c.getContext("2d");
  if (im instanceof ImageData) g.putImageData(im, 0, 0); else g.drawImage(im, 0, 0);
  const d = g.getImageData(0, 0, c.width, c.height).data;
  let x0 = c.width, y0 = c.height, x1 = -1, y1 = -1;
  for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) {
    if (d[(y * c.width + x) * 4 + 3] > 8) { x0 = Math.min(x0, x); x1 = Math.max(x1, x); y0 = Math.min(y0, y); y1 = Math.max(y1, y); }
  }
  if (x1 < 0) return "";
  const t = document.createElement("canvas");
  t.width = x1 - x0 + 3; t.height = y1 - y0 + 3;
  t.getContext("2d").drawImage(c, x0 - 1, y0 - 1, t.width, t.height, 0, 0, t.width, t.height);
  return t.toDataURL();
}

export function iconSelect(select, iconFor) {
  const urls = new Map();
  const url = (v) => { if (!urls.has(v)) { let u = ""; try { u = iconFor(v) || ""; } catch { /* none */ } urls.set(v, u); } return urls.get(v); };
  const pic = (v) => { const u = url(v); return u ? `<img src="${u}" alt="">` : '<span class="isel-none"></span>'; };
  const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "isel-btn";
  btn.setAttribute("aria-haspopup", "listbox");
  select.after(btn);
  select.hidden = true;
  const list = document.createElement("div");
  list.className = "isel-list";
  list.setAttribute("role", "listbox");
  list.hidden = true;
  document.body.append(list);

  function refresh() {
    const o = select.selectedOptions[0];
    btn.innerHTML = o ? `${pic(o.value)}<span class="isel-label">${esc(o.textContent)}</span><span class="isel-caret">▾</span>` : "—";
    btn.title = select.title || "";
  }
  function build() {
    list.innerHTML = "";
    for (const el of select.children) {
      const opts = el.tagName === "OPTGROUP" ? [...el.children] : [el];
      if (el.tagName === "OPTGROUP") { const h = document.createElement("div"); h.className = "isel-group"; h.textContent = el.label; list.append(h); }
      for (const o of opts) {
        const r = document.createElement("div");
        r.className = "isel-opt" + (o.value === select.value ? " on" : "");
        r.setAttribute("role", "option");
        r.dataset.v = o.value;
        r.innerHTML = `${pic(o.value)}<span>${esc(o.textContent)}</span>`;
        list.append(r);
      }
    }
  }
  function open() {
    build();
    const b = btn.getBoundingClientRect();
    list.hidden = false;
    list.style.minWidth = `${b.width}px`;
    const h = Math.min(list.scrollHeight, window.innerHeight * 0.6);
    const below = window.innerHeight - b.bottom - 8;
    list.style.left = `${Math.min(b.left, window.innerWidth - list.offsetWidth - 8)}px`;
    list.style.top = `${below >= h || below > b.top ? b.bottom + 2 : Math.max(8, b.top - h - 2)}px`;
    list.style.maxHeight = `${Math.max(160, below >= h || below > b.top ? below : b.top - 10)}px`;
    list.querySelector(".isel-opt.on")?.scrollIntoView({ block: "nearest" });
  }
  const close = () => { list.hidden = true; };
  function choose(v) {
    if (select.value !== v) { select.value = v; select.dispatchEvent(new Event("change", { bubbles: true })); }
    refresh(); close(); btn.focus();
  }
  btn.onclick = () => (list.hidden ? open() : close());
  list.addEventListener("click", (e) => { const r = e.target.closest(".isel-opt"); if (r) choose(r.dataset.v); });
  document.addEventListener("pointerdown", (e) => { if (!list.hidden && !list.contains(e.target) && !btn.contains(e.target)) close(); });
  btn.addEventListener("keydown", (e) => {
    const vals = [...select.options].map((o) => o.value);
    const i = vals.indexOf(select.value);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const v = vals[Math.max(0, Math.min(vals.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      select.value = v; select.dispatchEvent(new Event("change", { bubbles: true })); refresh();
      if (!list.hidden) build();
    } else if (e.key === "Escape" && !list.hidden) { e.preventDefault(); e.stopPropagation(); close(); }
    else if ((e.key === "Enter" || e.key === " ") && list.hidden) { e.preventDefault(); open(); }
    else if (e.key === "Enter" && !list.hidden) { e.preventDefault(); close(); }
  });
  select.addEventListener("change", refresh);
  refresh();
  return { refresh };
}
