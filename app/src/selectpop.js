// Drop-down lists drawn by the page. The list of a native <select> is drawn by the system, outside
// the page's colour filter, so in night mode it is white with a grey highlight. Every <select>
// (including ones made later) opens this list instead; the <select> keeps the value and fires
// its usual change event, so code that reads or listens to it is unchanged.

export function setupSelectPop() {
  const list = document.createElement("div");
  list.className = "isel-list sel-pop";
  list.setAttribute("role", "listbox");
  list.hidden = true;
  document.body.append(list);

  let owner = null;
  let rows = []; // the choosable rows, in order
  let hl = -1;

  const close = () => { list.hidden = true; owner = null; rows = []; hl = -1; };
  const highlight = (i, scroll = true) => {
    rows[hl]?.classList.remove("hl");
    hl = i;
    rows[hl]?.classList.add("hl");
    if (scroll) rows[hl]?.scrollIntoView({ block: "nearest" });
  };
  const choose = (i) => {
    const s = owner, o = rows[i]?.opt;
    close();
    if (s && o && s.value !== o.value) { s.value = o.value; s.dispatchEvent(new Event("input", { bubbles: true })); s.dispatchEvent(new Event("change", { bubbles: true })); }
    s?.focus();
  };

  function open(s) {
    owner = s;
    list.innerHTML = "";
    rows = [];
    const add = (o, indent) => {
      const r = document.createElement("div");
      r.className = "isel-opt" + (o.value === s.value ? " on" : "") + (o.disabled ? " off" : "");
      r.setAttribute("role", "option");
      r.textContent = o.textContent;
      if (indent) r.style.paddingLeft = "20px";
      list.append(r);
      if (!o.disabled) { r.opt = o; r.idx = rows.length; rows.push(r); }
    };
    for (const el of s.children) {
      if (el.tagName === "OPTGROUP") {
        const h = document.createElement("div");
        h.className = "isel-group";
        h.textContent = el.label;
        list.append(h);
        for (const o of el.children) add(o, true);
      } else if (el.tagName === "OPTION") add(el, false);
    }
    const b = s.getBoundingClientRect();
    list.hidden = false;
    list.style.minWidth = `${b.width}px`;
    list.style.maxHeight = "";
    const h = Math.min(list.scrollHeight, window.innerHeight * 0.6);
    const below = window.innerHeight - b.bottom - 8;
    const down = below >= h || below > b.top;
    list.style.left = `${Math.max(4, Math.min(b.left, window.innerWidth - list.offsetWidth - 8))}px`;
    list.style.top = `${down ? b.bottom + 2 : Math.max(8, b.top - h - 2)}px`;
    list.style.maxHeight = `${Math.max(160, down ? below : b.top - 10)}px`;
    highlight(Math.max(0, rows.findIndex((r) => r.opt.value === s.value)));
  }

  const native = (s) => s instanceof HTMLSelectElement && !s.multiple && s.size <= 1 && !s.disabled;

  document.addEventListener("mousedown", (e) => {
    const t = e.target instanceof Element ? e.target : null;
    if (list.contains(t)) return;
    const s = t?.closest("select");
    if (!list.hidden) { const same = s === owner; close(); if (same) { e.preventDefault(); return; } }
    if (e.button !== 0 || !native(s)) return;
    e.preventDefault(); // no system list
    s.focus();
    open(s);
  }, true);

  list.addEventListener("mouseover", (e) => { const r = e.target.closest(".isel-opt"); if (r?.opt) highlight(r.idx, false); });
  list.addEventListener("click", (e) => { const r = e.target.closest(".isel-opt"); if (r?.opt) choose(r.idx); });

  document.addEventListener("keydown", (e) => {
    if (!list.hidden) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); highlight(Math.max(0, Math.min(rows.length - 1, hl + (e.key === "ArrowDown" ? 1 : -1)))); }
      else if (e.key === "Home" || e.key === "End") { e.preventDefault(); highlight(e.key === "Home" ? 0 : rows.length - 1); }
      else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); choose(hl); }
      else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
      else if (e.key === "Tab") close();
      return;
    }
    const s = e.target;
    if (native(s) && (e.key === " " || e.key === "F4" || (e.altKey && e.key === "ArrowDown"))) { e.preventDefault(); open(s); }
  }, true);

  window.addEventListener("resize", close);
  window.addEventListener("blur", close);
  window.addEventListener("wheel", (e) => { if (!list.hidden && !list.contains(e.target)) close(); }, { capture: true, passive: true });
}
