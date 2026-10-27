// One list for every kind of library object (marks, routes, tracks, routings, barriers):
// the same search, filter chips, sort, count, selection and actions in every tab, and a
// list that draws only the rows on screen, so 80,000 marks scroll as smoothly as eight.
//
//   const list = libList(el, {
//     key: "marks",                         // filters and sort are remembered under this
//     noun: ["mark", "marks"],
//     search: (item, q) => …,               // q is lower case
//     filters: [{ key, label, type: "choice" | "multi" | "toggle" | "dates", options: (items) => [[value, label, count]], test: (item, value) => bool }],
//     sorts: [{ key, label, cmp }],
//     row: { render: (item, el) => …, click: (item) => … },
//     actions: [{ label, title, run: (items, value) => …, input?: { type: "text" | "select" | "colour", label, options }, confirm?: (items) => "question" }],
//     empty: () => "text",
//     changed: (filtered) => …,             // after every change of items, search or filters
//   });
//   list.setItems(items); list.filtered(); list.selected();

import { iconSelect } from "./iconselect.js";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
export { MONTHS };

const fmtN = (n) => n.toLocaleString();
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** Month and date tests for an item's time (Unix seconds; 0 or null: unknown, never matched). */
/** Months in season order: round the year from after the longest gap (Nov, Dec, Jan, not Jan, Nov, Dec). */
export function seasonOrder(months) {
  const m = [...months].sort((a, b) => a - b);
  if (m.length < 2) return m;
  let best = 0, start = 0;
  for (let i = 0; i < m.length; i++) {
    const gap = (m[(i + 1) % m.length] - m[i] + 12) % 12 || 12;
    if (gap > best) { best = gap; start = (i + 1) % m.length; }
  }
  return [...m.slice(start), ...m.slice(0, start)];
}

export function monthTest(t, months) {
  if (!t || !months?.length) return !months?.length;
  return months.includes(new Date(t * 1000).getMonth());
}
export function dateTest(t, range) {
  if (!range || (!range.from && !range.to)) return true;
  if (!t) return false;
  const d = new Date(t * 1000).toISOString().slice(0, 10);
  return (!range.from || d >= range.from) && (!range.to || d <= range.to);
}

export function libList(root, cfg) {
  const ROW = cfg.rowHeight ?? 50;
  const [one, many] = cfg.noun ?? ["item", "items"];
  const KEY = `racenav.list.${cfg.key}`;
  let state = { q: "", sort: cfg.sorts?.[0]?.key ?? "", f: {} };
  try { Object.assign(state, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* storage unavailable */ }
  state.q = ""; // a search isn't kept from one session to the next
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify({ sort: state.sort, f: state.f })); } catch { /* ignore */ } };

  let items = [], shown = [];
  const sel = new Set();

  root.classList.add("ll");
  root.innerHTML = `
    <div class="ll-top"><input class="ll-search grow" type="search" placeholder="Find" /><select class="ll-sort" title="Order"></select></div>
    <div class="ll-chips"></div>
    <div class="ll-count"><span class="ll-n grow"></span><button class="ll-all" title="Tick every one listed">Select all</button></div>
    <div class="ll-bar" hidden><span class="ll-sel"></span><span class="ll-acts"></span><button class="ll-none" title="Untick them all">✕</button></div>
    <div class="ll-ask" hidden></div>
    <div class="ll-scroll"><div class="ll-space"></div></div>`;
  const $ = (s) => root.querySelector(s);
  const search = $(".ll-search"), sortSel = $(".ll-sort"), chips = $(".ll-chips"), scroll = $(".ll-scroll"), space = $(".ll-space");
  search.placeholder = cfg.searchPlaceholder ?? `Find a ${one}`;
  for (const s of cfg.sorts ?? []) sortSel.append(new Option(s.label, s.key));
  sortSel.hidden = !(cfg.sorts?.length > 1);
  sortSel.value = state.sort;

  // ---------- filtering ----------
  function active(f) {
    const v = state.f[f.key];
    return f.type === "multi" ? v?.length > 0 : f.type === "dates" ? !!(v?.from || v?.to) : f.type === "toggle" ? !!v : v != null && v !== "";
  }
  function apply() {
    const q = state.q.trim().toLowerCase();
    const fs = (cfg.filters ?? []).filter(active);
    shown = items.filter((x) => (!q || cfg.search(x, q)) && fs.every((f) => f.test(x, state.f[f.key])));
    const s = cfg.sorts?.find((x) => x.key === state.sort);
    if (s) shown.sort(s.cmp);
    // the selection follows the list: what's no longer listed is let go
    const ids = new Set(shown.map((x) => x.id));
    for (const id of [...sel]) if (!ids.has(id)) sel.delete(id);
    renderChips();
    renderCount();
    renderRows(true);
    cfg.changed?.(shown);
  }

  // ---------- chips ----------
  let pop = null;
  function closePop() { pop?.remove(); pop = null; }
  document.addEventListener("pointerdown", (e) => { if (pop && !pop.contains(e.target) && !e.target.closest(".ll-chip")) closePop(); });
  function chipText(f) {
    const v = state.f[f.key];
    if (!active(f)) return f.label;
    if (f.type === "toggle") return f.label;
    if (f.type === "multi") return f.months ? seasonOrder(v).map((m) => MONTHS[m]).join(", ") : `${f.label}: ${v.length}`;
    if (f.type === "dates") return `${v.from || "…"} – ${v.to || "…"}`;
    const o = f.options(items).find((x) => String(x[0]) === String(v));
    return `${f.label}: ${o ? o[1] : v}`;
  }
  function renderChips() {
    chips.innerHTML = "";
    for (const f of cfg.filters ?? []) {
      const b = document.createElement("button");
      b.className = "ll-chip";
      b.classList.toggle("on", active(f));
      b.title = f.title ?? "";
      b.innerHTML = `<span></span>${f.type === "toggle" ? "" : " ▾"}`;
      b.firstChild.textContent = chipText(f);
      b.onclick = () => {
        if (f.type === "toggle") { state.f[f.key] = !state.f[f.key]; save(); apply(); return; }
        if (pop?.dataset.key === f.key) { closePop(); return; }
        openPop(f, b);
      };
      if (active(f) && f.type !== "toggle") {
        const x = document.createElement("span");
        x.className = "ll-x";
        x.textContent = "✕";
        x.title = "Clear this filter";
        x.onclick = (e) => { e.stopPropagation(); delete state.f[f.key]; save(); closePop(); apply(); };
        b.append(x);
      }
      chips.append(b);
    }
    if ((cfg.filters ?? []).some(active)) {
      const c = document.createElement("button");
      c.className = "ll-chip ll-clear";
      c.textContent = "Clear filters";
      c.onclick = () => { state.f = {}; save(); closePop(); apply(); };
      chips.append(c);
    }
  }
  function openPop(f, chip) {
    closePop();
    pop = document.createElement("div");
    pop.className = "ll-pop";
    pop.dataset.key = f.key;
    const v = state.f[f.key];
    if (f.type === "choice") {
      const opts = f.options(items);
      for (const [val, label, count] of opts) {
        const o = document.createElement("button");
        o.className = "ll-opt";
        o.classList.toggle("on", String(v) === String(val));
        o.innerHTML = `<span class="grow"></span><small></small>`;
        o.firstChild.textContent = label;
        o.lastChild.textContent = count != null ? fmtN(count) : "";
        o.onclick = () => { state.f[f.key] = val; save(); closePop(); apply(); };
        pop.append(o);
      }
      if (!opts.length) pop.innerHTML = `<div class="wx-note">Nothing to choose from.</div>`;
    } else if (f.type === "multi") {
      const grid = document.createElement("div");
      grid.className = "ll-months";
      const cur = new Set(v ?? []);
      (f.months ? MONTHS.map((m, i) => [i, m]) : f.options(items)).forEach(([val, label]) => {
        const o = document.createElement("button");
        o.textContent = label;
        o.classList.toggle("on", cur.has(val));
        o.onclick = () => {
          if (cur.has(val)) cur.delete(val); else cur.add(val);
          o.classList.toggle("on", cur.has(val));
          state.f[f.key] = [...cur].sort((a, b) => a - b);
          save(); apply();
          chip.firstChild.textContent = chipText(f);
        };
        grid.append(o);
      });
      pop.append(grid);
      if (f.note) { const n = document.createElement("div"); n.className = "wx-note"; n.textContent = f.note; pop.append(n); }
    } else if (f.type === "dates") {
      pop.innerHTML = `<div class="wx-row"><label>From</label><input type="date" class="ll-from"></div><div class="wx-row"><label>To</label><input type="date" class="ll-to"></div>`;
      if (f.note) { const n = document.createElement("div"); n.className = "wx-note"; n.textContent = f.note; pop.append(n); }
      const from = pop.querySelector(".ll-from"), to = pop.querySelector(".ll-to");
      from.value = v?.from ?? ""; to.value = v?.to ?? "";
      const set = () => { state.f[f.key] = { from: from.value, to: to.value }; save(); apply(); };
      from.onchange = set; to.onchange = set;
    }
    root.append(pop);
    const r = chip.getBoundingClientRect(), rr = root.getBoundingClientRect();
    pop.style.left = `${Math.max(0, Math.min(r.left - rr.left, rr.width - 240))}px`;
    pop.style.top = `${r.bottom - rr.top + 4}px`;
  }

  // ---------- count, selection, actions ----------
  function renderCount() {
    $(".ll-n").textContent = items.length === shown.length ? `${fmtN(items.length)} ${items.length === 1 ? one : many}` : `${fmtN(shown.length)} of ${fmtN(items.length)} ${many}`;
    $(".ll-all").hidden = !shown.length || !cfg.actions?.length;
    $(".ll-all").textContent = shown.length === items.length ? "Select all" : `Select these ${fmtN(shown.length)}`;
    const bar = $(".ll-bar");
    bar.hidden = !sel.size;
    $(".ll-count").hidden = !!sel.size;
    if (!sel.size) { $(".ll-ask").hidden = true; return; }
    $(".ll-sel").textContent = `${fmtN(sel.size)} selected`;
    const acts = $(".ll-acts");
    acts.innerHTML = "";
    for (const a of cfg.actions ?? []) {
      const b = document.createElement("button");
      b.textContent = a.label;
      b.title = a.title ?? "";
      if (a.danger) b.classList.add("warn");
      b.onclick = () => ask(a);
      acts.append(b);
    }
  }
  /** An action's question (confirm) or input (group name, type, colour), in place, then run it. */
  function ask(a) {
    const picked = shown.filter((x) => sel.has(x.id));
    const box = $(".ll-ask");
    const go = async (value) => {
      box.hidden = true;
      await a.run(picked, value);
    };
    if (!a.confirm && !a.input) { go(); return; }
    box.hidden = false;
    box.innerHTML = "";
    const q = document.createElement("span");
    q.className = "grow";
    q.textContent = a.confirm ? a.confirm(picked) : `${a.input.label} for ${fmtN(picked.length)} ${picked.length === 1 ? one : many}:`;
    box.append(q);
    let inp = null;
    if (a.input) {
      if (a.input.type === "select") {
        inp = a.input.select ? a.input.select() : document.createElement("select");
        if (!a.input.select) for (const [v, t] of a.input.options()) inp.append(new Option(t, v));
      } else {
        inp = document.createElement("input");
        inp.type = a.input.type === "colour" ? "color" : "text";
        if (a.input.list) {
          const id = `ll-dl-${cfg.key}`;
          let dl = document.getElementById(id);
          if (!dl) { dl = document.createElement("datalist"); dl.id = id; document.body.append(dl); }
          dl.innerHTML = a.input.list().map((v) => `<option value="${esc(v)}">`).join("");
          inp.setAttribute("list", id);
        }
        if (a.input.value) inp.value = a.input.value(picked);
      }
      box.append(inp);
      // a list of symbols shows each one's picture (iconselect.js)
      if (a.input.icons) iconSelect(inp, a.input.icons).refresh?.();
      else setTimeout(() => inp.focus(), 0);
      inp.onkeydown = (e) => { if (e.key === "Enter") ok.click(); if (e.key === "Escape") no.click(); };
    }
    const ok = document.createElement("button");
    ok.textContent = a.okLabel ?? (a.confirm ? a.label : "Apply");
    ok.className = a.danger ? "on warn" : "on";
    ok.onclick = () => go(inp?.value);
    const no = document.createElement("button");
    no.textContent = a.confirm ? "Keep" : "Cancel";
    no.onclick = () => { box.hidden = true; };
    box.append(ok, no);
  }
  $(".ll-all").onclick = () => { for (const x of shown) sel.add(x.id); renderCount(); renderRows(); };
  $(".ll-none").onclick = () => { sel.clear(); renderCount(); renderRows(); };

  // ---------- rows: only those on screen ----------
  let drawn = new Map();
  function renderRows(reset = false) {
    if (reset) { for (const el of drawn.values()) el.remove(); drawn.clear(); }
    space.style.height = `${shown.length * ROW}px`;
    let empty = scroll.querySelector(".ll-empty");
    if (!shown.length) {
      if (!empty) { empty = document.createElement("div"); empty.className = "ll-empty wx-note"; scroll.append(empty); }
      empty.textContent = items.length ? `No ${many} match the search and filters.` : cfg.empty?.() ?? `No ${many}.`;
    } else empty?.remove();
    const top = scroll.scrollTop, h = scroll.clientHeight || 400;
    const a = Math.max(0, Math.floor(top / ROW) - 5), b = Math.min(shown.length, Math.ceil((top + h) / ROW) + 5);
    const want = new Set();
    for (let i = a; i < b; i++) {
      const x = shown[i];
      want.add(x.id);
      let el = drawn.get(x.id);
      if (!el) {
        el = document.createElement("div");
        el.className = "ll-row";
        el.dataset.id = x.id;
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.className = "ll-tick";
        cb.title = "Select";
        cb.hidden = !cfg.actions?.length;
        cb.onclick = (e) => {
          e.stopPropagation();
          if (cb.checked) sel.add(x.id); else sel.delete(x.id);
          renderCount();
        };
        const body = document.createElement("div");
        body.className = "ll-body";
        el.append(cb, body);
        cfg.row.render(x, body, el);
        if (cfg.row.click) body.querySelector(".main")?.addEventListener("click", () => cfg.row.click(x));
        space.append(el);
        drawn.set(x.id, el);
      }
      el.style.top = `${i * ROW}px`;
      el.querySelector(".ll-tick").checked = sel.has(x.id);
      el.classList.toggle("picked", sel.has(x.id));
      cfg.row.update?.(x, el);
    }
    for (const [id, el] of drawn) if (!want.has(id)) { el.remove(); drawn.delete(id); }
  }
  let raf = 0;
  scroll.addEventListener("scroll", () => { if (!raf) raf = requestAnimationFrame(() => { raf = 0; renderRows(); }); });
  new ResizeObserver(() => renderRows()).observe(scroll);

  search.oninput = () => { state.q = search.value; apply(); };
  sortSel.onchange = () => { state.sort = sortSel.value; save(); apply(); };

  return {
    setItems(v) { items = v ?? []; apply(); },
    refresh: apply,
    /** Redraw the rows on screen (e.g. after something they show changed). */
    redraw: () => renderRows(true),
    filtered: () => shown,
    selected: () => shown.filter((x) => sel.has(x.id)),
    clearSelection() { sel.clear(); renderCount(); renderRows(); },
    /** Scroll a row into view. */
    reveal(id) {
      const i = shown.findIndex((x) => x.id === id);
      if (i >= 0) scroll.scrollTop = Math.max(0, i * ROW - scroll.clientHeight / 2);
    },
    get filters() { return state.f; },
    get sort() { return state.sort; },
    setFilter(key, value) { if (value == null) delete state.f[key]; else state.f[key] = value; save(); apply(); },
    root,
  };
}
