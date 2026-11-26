// The polar: a plot of the polar in use and a table to edit it like a spreadsheet, in the dock
// (Settings › Vessel › Performance › View and edit…).
//
// The table is the polar file's: wind angles (TWA) down the side, wind speeds (TWS) across the top,
// boat speeds in knots. Type in a cell, move with the arrow keys, Enter or Tab; drag or Shift+click
// to select a block, then scale it by a percentage, copy it, or paste a block from a spreadsheet
// (paste a whole table, headings and all, into the top-left cell). Changed cells are marked and the
// plot shows the saved curve dashed beside the edited one. Saving writes into your polar folder;
// a shipped polar is never changed: your copy with its name is used instead.
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./ask.js";
import { ramp, onTheme } from "./theme.js";

// ---------- the sums (the same as the polar crate's) ----------

function bracket(v, x) {
  if (x <= v[0]) return [0, 0, 0];
  const last = v.length - 1;
  if (x >= v[last]) return [last, last, 0];
  let i = 0;
  while (v[i + 1] <= x) i++;
  return [i, i + 1, (x - v[i]) / (v[i + 1] - v[i])];
}
const lerp = (a, b, f) => a + (b - a) * f;

/** Boat speed from a table { tws, twa, v } at a TWA and TWS, as the router works it out. */
export function bsp(t, twa, tws) {
  const a = Math.abs(((((twa + 180) % 360) + 360) % 360) - 180);
  if (tws <= 0 && t.tws[0] > 0) return 0;
  tws = Math.max(0, tws);
  const [i0, i1, fa] = bracket(t.twa, a);
  const row = (i) => {
    const r = t.v[i];
    if (tws < t.tws[0] && t.tws[0] > 0) return (r[0] * tws) / t.tws[0];
    const [j0, j1, fs] = bracket(t.tws, tws);
    return lerp(r[j0], r[j1], fs);
  };
  return lerp(row(i0), row(i1), fa);
}

/** Best VMG upwind (25° to 90°) or downwind (90° to 180°) at a TWS: { twa, bsp, vmg } or null. */
export function bestVmg(t, tws, up) {
  let best = null;
  for (let a = up ? 25 : 90; a <= (up ? 90 : 180) + 1e-9; a += 0.5) {
    const b = bsp(t, a, tws);
    const v = (up ? 1 : -1) * b * Math.cos((a * Math.PI) / 180);
    if (!best || v > best.vmg) best = { twa: a, bsp: b, vmg: v };
  }
  return best && best.vmg > 0 ? best : null;
}

/** A block pasted from a spreadsheet or text: rows of cells (tab, ';' or ',' between them). */
export function parseBlock(text) {
  const lines = text.replace(/\r/g, "").split("\n").filter((l, i, all) => l.trim() !== "" || i < all.length - 1).filter((l) => l.trim() !== "");
  const sep = lines.some((l) => l.includes("\t")) ? "\t" : lines.some((l) => l.includes(";")) ? ";" : lines.some((l) => /\d,\d+\s*,/.test(l) || (l.match(/,/g) || []).length > 1) ? "," : /\s+/;
  return lines.map((l) => l.split(sep).map((c) => c.trim()));
}

/** A number from a cell ("4.5", "4,5"), or null. */
export const num = (s) => {
  const v = Number(String(s).trim().replace(",", "."));
  return String(s).trim() !== "" && Number.isFinite(v) ? v : null;
};

/** What is wrong with a table, for each cell: { "h:j" | "a:i" | "i:j": message }. Empty when it can be saved. */
export function problems(t) {
  const p = {};
  t.tws.forEach((w, j) => {
    if (!Number.isFinite(w) || w < 0) p[`h:${j}`] = "Wind speed: a number, 0 or more";
    else if (j > 0 && w <= t.tws[j - 1]) p[`h:${j}`] = "Wind speeds must go up from left to right";
  });
  t.twa.forEach((a, i) => {
    if (!Number.isFinite(a) || a < 0 || a > 180) p[`a:${i}`] = "Wind angle: 0 to 180";
    else if (i > 0 && a <= t.twa[i - 1]) p[`a:${i}`] = "Wind angles must go up from top to bottom";
  });
  t.v.forEach((r, i) => r.forEach((v, j) => { if (!Number.isFinite(v) || v < 0) p[`${i}:${j}`] = "Boat speed: a number, 0 or more"; }));
  return p;
}

/** Put the table in order: rows by TWA, columns by TWS (after pasting or editing a heading). */
export function sorted(t) {
  const ci = t.tws.map((_, j) => j).sort((a, b) => t.tws[a] - t.tws[b]);
  const ri = t.twa.map((_, i) => i).sort((a, b) => t.twa[a] - t.twa[b]);
  return { ...t, tws: ci.map((j) => t.tws[j]), twa: ri.map((i) => t.twa[i]), v: ri.map((i) => ci.map((j) => t.v[i][j])) };
}

const copy = (t) => ({ ...t, tws: [...t.tws], twa: [...t.twa], v: t.v.map((r) => [...r]) });
const fmt = (v) => (Number.isFinite(v) ? String(Math.round(v * 1000) / 1000) : "");
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

// a colour for each wind speed, light to strong (followed into the night palettes)
const TWS_COLOURS = ["#5e4fa2", "#3288bd", "#1b9e9e", "#4daf4a", "#a6b51a", "#e6ab02", "#f46d43", "#d53e4f", "#9e0142"];
const colourFor = (j, n) => ramp(TWS_COLOURS[n <= 1 ? 0 : Math.round((j * (TWS_COLOURS.length - 1)) / (n - 1))]);

// ---------- the dock view ----------

export function setupPolarEdit({ dock, getBoatPolar, toast = () => {} }) {
  let t = null; // the table being edited: { name, path, yours, tws, twa, v }
  let saved = null; // as last read or saved
  let undo = [];
  let redo = [];
  let sel = null; // { r0, c0, r1, c1 } in grid cells: row -1 = the TWS headings, column -1 = the TWA column
  let focusCol = null; // the TWS column being looked at (its curve drawn bold)
  let els = null;

  const dirty = () => saved && JSON.stringify([t.tws, t.twa, t.v]) !== JSON.stringify([saved.tws, saved.twa, saved.v]);
  const changed = (i, j) => saved && (saved.twa[i] !== t.twa[i] || saved.tws[j] !== t.tws[j] || saved.v[i]?.[j] !== t.v[i][j]);

  async function load(path) {
    if (!path) { t = saved = null; undo = []; redo = []; render(); return; }
    try {
      const p = await invoke("polar_table", { path });
      t = { name: p.name, path: p.path, yours: p.yours, tws: p.tws, twa: p.twa, v: p.speeds };
      saved = copy(t);
      undo = []; redo = []; sel = null; focusCol = null;
    } catch (e) { toast(String(e), 6000); t = saved = null; }
    render();
  }

  function change(fn) {
    undo.push(copy(t));
    if (undo.length > 200) undo.shift();
    redo = [];
    fn();
    draw();
  }

  // ---- rendering ----
  async function render() {
    if (!els) return;
    const { body } = els;
    body.innerHTML = "";
    // the view's own bar inside it: the dock's header row is the views' tabs
    const root = document.createElement("div");
    root.className = "pe-root";
    const tools = document.createElement("div");
    tools.className = "pe-bar";
    root.append(tools);
    body.append(root);
    const list = (await invoke("polar_files").catch(() => [])) || [];
    const pick = document.createElement("select");
    pick.title = "The polar to show (the one in use is chosen in Settings › Vessel › Performance)";
    if (!list.length) pick.append(new Option("No polars found", ""));
    for (const p of list) pick.append(new Option(`${p.name}${p.source === "shipped" ? " (shipped)" : ""}${p.path === getBoatPolar() ? " · in use" : ""}`, p.path));
    if (t) pick.value = t.path;
    pick.onchange = async () => {
      if (t && dirty() && !(await ask({ title: "Leave your changes?", note: `${t.name} has changes that aren't saved.`, ok: "Leave them", cancel: "Keep editing" }))) { pick.value = t.path; return; }
      load(pick.value);
    };
    const btn = (label, title, fn) => { const b = document.createElement("button"); b.textContent = label; b.title = title; b.onclick = fn; tools.append(b); return b; };
    tools.append(pick);
    els.state = document.createElement("span");
    els.state.className = "wx-note pe-state";
    tools.append(els.state);
    els.undo = btn("Undo", "Undo the last change (Ctrl+Z)", doUndo);
    els.redo = btn("Redo", "Redo (Ctrl+Y)", doRedo);
    btn("Scale…", "Multiply the selected boat speeds (or all of them) by a percentage", scale);
    btn("+ Angle", "Add a wind angle row below the selected one, halfway to the next", addRow);
    btn("+ Speed", "Add a wind speed column after the selected one", addCol);
    btn("− Angle", "Delete the selected wind angle rows", delRows);
    btn("− Speed", "Delete the selected wind speed columns", delCols);
    els.revert = btn("Revert", "Back to the polar as saved", () => { if (t && saved) change(() => { t = copy(saved); }); });
    els.save = btn("Save", "Save into your polar folder (a shipped polar is saved as your own copy with its name)", () => save(false));
    btn("Save as…", "Save under a new name in your polar folder", () => save(true));

    const wrap = document.createElement("div");
    wrap.className = "pe-wrap";
    root.append(wrap);
    if (!t) { wrap.innerHTML = `<p class="wx-note">${list.length ? "Choose a polar." : "No polars found. Import one in Settings › Vessel › Performance."}</p>`; return; }
    wrap.innerHTML = `<div class="pe-plot"><svg viewBox="0 0 172 300" preserveAspectRatio="xMidYMid meet"></svg><div class="pe-tip" hidden></div></div>
      <div class="pe-grid-wrap"><table class="pe-grid" role="grid" data-shape=""></table><div class="pe-msg wx-note"></div></div>`;
    els.svg = wrap.querySelector("svg");
    els.tip = wrap.querySelector(".pe-tip");
    els.grid = wrap.querySelector("table");
    els.msg = wrap.querySelector(".pe-msg");
    wireGrid();
    draw();
  }

  function draw() {
    if (!els?.grid || !t) return;
    drawGrid();
    drawPlot();
    const pr = problems(t);
    const n = Object.keys(pr).length;
    const d = dirty();
    els.state.textContent = n ? `${n} cell${n > 1 ? "s" : ""} to fix` : d ? "changed, not saved" : t.yours ? "saved" : "shipped (Save makes your copy)";
    els.state.classList.toggle("bad", n > 0);
    els.save.disabled = n > 0 || (!d && t.yours);
    els.undo.disabled = !undo.length;
    els.redo.disabled = !redo.length;
    els.revert.disabled = !d;
    els.msg.textContent = n ? [...new Set(Object.values(pr))].join(" · ") : `${t.twa.length} angles × ${t.tws.length} wind speeds · boat speeds in knots · ${t.path}`;
  }

  // the grid: rows -1 (headings) .. twa.length-1, plus VMG rows; columns -1 (TWA) .. tws.length-1.
  // Built when its shape changes, otherwise its cells are updated in place (the cell being typed in
  // and the click that is moving to another cell are left alone).
  function cellState(r, c, pr) {
    const inS = sel && r >= Math.min(sel.r0, sel.r1) && r <= Math.max(sel.r0, sel.r1) && c >= Math.min(sel.c0, sel.c1) && c <= Math.max(sel.c0, sel.c1);
    const key = r < 0 ? `h:${c}` : c < 0 ? `a:${r}` : `${r}:${c}`;
    const v = r < 0 ? t.tws[c] : c < 0 ? t.twa[r] : t.v[r][c];
    const chg = saved && (r < 0 ? saved.tws[c] !== v : c < 0 ? saved.twa[r] !== v : changed(r, c));
    const cls = [r < 0 ? "pe-h" : c < 0 ? "pe-a" : "", chg ? "chg" : "", inS ? "sel" : "", pr[key] ? "bad" : "", c === focusCol && c >= 0 && r >= 0 ? "col" : ""].filter(Boolean).join(" ");
    return { v: fmt(v), cls, title: pr[key] || "" };
  }
  function drawGrid() {
    const g = els.grid;
    const pr = problems(t);
    const shape = `${t.twa.length}x${t.tws.length}`;
    if (g.dataset.shape !== shape) {
      const cell = (r, c) => { const st = cellState(r, c, pr); return `<td class="${st.cls}"><input type="text" data-r="${r}" data-c="${c}" value="${esc(st.v)}" inputmode="decimal" spellcheck="false" title="${esc(st.title)}" /></td>`; };
      const head = `<tr><th class="pe-corner" title="Click to select the whole table; paste a whole table here, headings and all">TWA \\ TWS</th>${t.tws.map((_, j) => cell(-1, j)).join("")}</tr>`;
      const rows = t.twa.map((_, i) => `<tr>${cell(i, -1)}${t.tws.map((_, j) => cell(i, j)).join("")}</tr>`).join("");
      const foot = ["Beat TWA", "Beat VMG", "Run TWA", "Run VMG"].map((l) => `<tr class="pe-vmg"><th>${l}</th>${t.tws.map(() => "<td></td>").join("")}</tr>`).join("");
      g.innerHTML = `<thead><tr class="pe-sw"><th></th>${t.tws.map(() => "<td><i></i></td>").join("")}</tr>${head}</thead><tbody>${rows}</tbody><tfoot>${foot}</tfoot>`;
      g.dataset.shape = shape;
    } else {
      for (const inp of g.querySelectorAll("input")) {
        const st = cellState(+inp.dataset.r, +inp.dataset.c, pr);
        if (inp !== document.activeElement && inp.value !== st.v) inp.value = st.v;
        if (inp.parentElement.className !== st.cls) inp.parentElement.className = st.cls;
        if (inp.title !== st.title) inp.title = st.title;
      }
    }
    // the focused cell shows the table's value once it has been committed (a paste, undo, scale)
    const ae = document.activeElement;
    if (ae?.closest?.(".pe-grid") === g && !ae.dataset.typing) { const st = cellState(+ae.dataset.r, +ae.dataset.c, pr); if (ae.value !== st.v) { ae.value = st.v; ae.select(); } }
    g.querySelectorAll(".pe-sw i").forEach((el, j) => { el.style.background = colourFor(j, t.tws.length); });
    const ok = Object.keys(pr).length === 0;
    const vm = t.tws.map((w) => (ok ? [bestVmg(t, w, true), bestVmg(t, w, false)] : [null, null]));
    g.querySelectorAll("tfoot tr").forEach((tr, k) => tr.querySelectorAll("td").forEach((td, j) => {
      const b = vm[j][k < 2 ? 0 : 1];
      td.textContent = !b ? "—" : k % 2 === 0 ? `${b.twa.toFixed(1)}°` : b.vmg.toFixed(2);
    }));
  }

  const input = (r, c) => els.grid.querySelector(`input[data-r="${r}"][data-c="${c}"]`);
  function goTo(r, c, extend = false) {
    r = Math.max(-1, Math.min(t.twa.length - 1, r));
    c = Math.max(-1, Math.min(t.tws.length - 1, c));
    sel = extend && sel ? { ...sel, r1: r, c1: c } : { r0: r, c0: c, r1: r, c1: c };
    focusCol = c >= 0 ? c : focusCol;
    drawGrid();
    drawPlot();
    const inp = input(r, c);
    if (inp) { inp.focus(); inp.select(); inp.scrollIntoView({ block: "nearest", inline: "nearest" }); }
  }

  // a typed value: committed when the cell is left (Enter, Tab, an arrow, a click elsewhere)
  function commit(inp) {
    delete inp.dataset.typing;
    const r = +inp.dataset.r, c = +inp.dataset.c;
    const v = num(inp.value);
    const old = r < 0 ? t.tws[c] : c < 0 ? t.twa[r] : t.v[r][c];
    if (v === null) { inp.value = fmt(old); return; }
    if (v === old) return;
    change(() => {
      if (r < 0) t.tws[c] = v; else if (c < 0) t.twa[r] = v; else t.v[r][c] = v;
      // a heading moved past its neighbours: the table is put back in order
      if (r < 0 || c < 0) { const s = sorted(t); if (Object.keys(problems(s)).length < Object.keys(problems(t)).length) t = s; }
    });
  }

  function wireGrid() {
    const g = els.grid;
    let dragging = false;
    g.addEventListener("focusin", (e) => { if (e.target.tagName === "INPUT") e.target.select(); });
    g.addEventListener("input", (e) => { if (e.target.tagName === "INPUT") e.target.dataset.typing = "1"; });
    g.addEventListener("change", (e) => { if (e.target.tagName === "INPUT") commit(e.target); });
    g.addEventListener("mousedown", (e) => {
      const inp = e.target.closest?.("input");
      if (!inp) return;
      const r = +inp.dataset.r, c = +inp.dataset.c;
      if (e.shiftKey && sel) { e.preventDefault(); sel = { ...sel, r1: r, c1: c }; drawGrid(); return; }
      sel = { r0: r, c0: c, r1: r, c1: c };
      if (c >= 0) focusCol = c;
      dragging = true;
      // drawn again once the click is done (a redraw now would lose the click's focus)
      setTimeout(() => { drawGrid(); drawPlot(); }, 0);
    });
    g.addEventListener("mouseover", (e) => {
      const inp = e.target.closest?.("input");
      if (!dragging || !inp || !sel) return;
      const r = +inp.dataset.r, c = +inp.dataset.c;
      if (r === sel.r1 && c === sel.c1) return;
      sel = { ...sel, r1: r, c1: c };
      drawGrid();
    });
    window.addEventListener("mouseup", () => { dragging = false; });
    g.addEventListener("keydown", (e) => {
      const inp = e.target.closest?.("input");
      if (!inp) return;
      const r = +inp.dataset.r, c = +inp.dataset.c;
      const mod = e.ctrlKey || e.metaKey;
      if (mod && (e.key === "z" || e.key === "Z")) { e.preventDefault(); commit(inp); e.shiftKey ? doRedo() : doUndo(); return; }
      if (mod && (e.key === "y" || e.key === "Y")) { e.preventDefault(); doRedo(); return; }
      const whole = inp.selectionStart === 0 && inp.selectionEnd === inp.value.length;
      const move = {
        ArrowUp: [-1, 0], ArrowDown: [1, 0],
        ArrowLeft: whole || inp.selectionStart === 0 ? [0, -1] : null,
        ArrowRight: whole || inp.selectionEnd === inp.value.length ? [0, 1] : null,
        Enter: [e.shiftKey ? -1 : 1, 0],
        Tab: [0, e.shiftKey ? -1 : 1],
      }[e.key];
      if (!move) {
        if (e.key === "Escape") { const old = r < 0 ? t.tws[c] : c < 0 ? t.twa[r] : t.v[r][c]; inp.value = fmt(old); delete inp.dataset.typing; inp.select(); e.stopPropagation(); }
        return;
      }
      e.preventDefault();
      commit(inp);
      const extend = e.shiftKey && /^Arrow/.test(e.key);
      goTo((extend ? sel?.r1 ?? r : r) + move[0], (extend ? sel?.c1 ?? c : c) + move[1], extend);
    });
    g.addEventListener("copy", (e) => {
      if (!sel || (sel.r0 === sel.r1 && sel.c0 === sel.c1)) return; // one cell: its text, as usual
      e.preventDefault();
      const [ra, rb] = [Math.min(sel.r0, sel.r1), Math.max(sel.r0, sel.r1)];
      const [ca, cb] = [Math.min(sel.c0, sel.c1), Math.max(sel.c0, sel.c1)];
      const val = (r, c) => (r < 0 && c < 0 ? "TWA\\TWS" : r < 0 ? fmt(t.tws[c]) : c < 0 ? fmt(t.twa[r]) : fmt(t.v[r][c]));
      const lines = [];
      for (let r = ra; r <= rb; r++) { const l = []; for (let c = ca; c <= cb; c++) l.push(val(r, c)); lines.push(l.join("\t")); }
      e.clipboardData.setData("text/plain", lines.join("\r\n"));
    });
    g.addEventListener("click", (e) => { if (e.target.closest?.(".pe-corner")) { sel = { r0: -1, c0: -1, r1: t.twa.length - 1, c1: t.tws.length - 1 }; drawGrid(); } });
    els.grid.closest(".pe-wrap").addEventListener("paste", (e) => {
      const text = e.clipboardData?.getData("text/plain") ?? "";
      const block = parseBlock(text);
      const inp = e.target.closest?.("input");
      const corner = e.target.closest?.(".pe-corner") || (inp && +inp.dataset.r === -1 && +inp.dataset.c === -1);
      // a whole table: headings across the top and down the side
      if ((corner || !inp) && block.length > 1 && block[0].length > 1) {
        e.preventDefault();
        const tws = block[0].slice(1).map(num), rows = block.slice(1);
        const twa = rows.map((r) => num(r[0])), v = rows.map((r) => tws.map((_, j) => num(r[j + 1] ?? "")));
        if (tws.some((x) => x === null) || twa.some((x) => x === null) || v.flat().some((x) => x === null)) { toast("That isn't a polar table: numbers with the wind speeds across the top and angles down the side", 6000); return; }
        change(() => { t = sorted({ ...t, tws, twa, v }); });
        toast(`Pasted ${twa.length} angles × ${tws.length} wind speeds`, 3000);
        return;
      }
      if (!inp || (block.length <= 1 && (block[0]?.length ?? 0) <= 1)) return; // one value: typed into the cell as usual
      e.preventDefault();
      const r0 = +inp.dataset.r, c0 = +inp.dataset.c;
      let n = 0;
      change(() => {
        block.forEach((row, di) => row.forEach((s, dj) => {
          const r = r0 + di, c = c0 + dj, v = num(s);
          if (v === null || r > t.twa.length - 1 || c > t.tws.length - 1) return;
          if (r < 0 && c < 0) return;
          if (r < 0) t.tws[c] = v; else if (c < 0) t.twa[r] = v; else t.v[r][c] = v;
          n++;
        }));
      });
      sel = { r0, c0, r1: Math.min(t.twa.length - 1, r0 + block.length - 1), c1: Math.min(t.tws.length - 1, c0 + Math.max(...block.map((b) => b.length)) - 1) };
      drawGrid();
      toast(`Pasted ${n} value${n === 1 ? "" : "s"}`, 2500);
    });
  }

  // ---- the plot ----
  function drawPlot() {
    const svg = els?.svg;
    if (!svg || !t) return;
    const max = Math.max(1, ...t.v.flat().filter(Number.isFinite), ...(saved ? saved.v.flat() : []));
    const step = max > 12 ? 4 : max > 6 ? 2 : 1;
    const top = Math.ceil(max / step) * step;
    const R = 136, cx = 18, cy = 150;
    const xy = (a, s) => [cx + (R * s * Math.sin((a * Math.PI) / 180)) / top, cy - (R * s * Math.cos((a * Math.PI) / 180)) / top];
    const P = ([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`;
    let s = "";
    for (let k = step; k <= top; k += step) s += `<path class="pe-ring" d="M${P(xy(0, k))}A${(R * k) / top},${(R * k) / top} 0 0 1 ${P(xy(180, k))}"/><text class="pe-lab" x="${cx + 3}" y="${(cy - (R * k) / top + 10).toFixed(1)}">${k}</text>`;
    for (let a = 30; a < 180; a += 30) s += `<path class="pe-ring" d="M${cx},${cy}L${P(xy(a, top))}"/><text class="pe-lab" x="${P(xy(a, top * 1.07)).split(",")[0]}" y="${P(xy(a, top * 1.07)).split(",")[1]}" text-anchor="middle" dominant-baseline="middle">${a}°</text>`;
    s += `<path class="pe-ring" d="M${cx},${cy - R}L${cx},${cy + R}"/>`;
    const n = t.tws.length;
    const okT = Object.keys(problems(t)).length === 0;
    const curve = (tab, j, cls, colour) => {
      const w = tab.tws[j];
      const lo = tab.twa[0], hi = tab.twa[tab.twa.length - 1];
      const pts = [];
      for (let a = lo; a <= hi + 1e-9; a += 1) pts.push(xy(a, bsp(tab, a, w)));
      return `<path class="${cls}" stroke="${colour}" d="M${pts.map(P).join("L")}"/>`;
    };
    for (let j = 0; j < n; j++) {
      const colour = colourFor(j, n);
      const sj = saved ? saved.tws.indexOf(t.tws[j]) : -1;
      const edited = sj >= 0 && t.twa.length && (t.twa.join() !== saved.twa.join() || t.v.some((r, i) => r[j] !== saved.v[i][sj]));
      if (edited) s += curve(saved, sj, "pe-was", colour);
      if (!okT) continue;
      s += curve(t, j, `pe-c${focusCol === j ? " on" : ""}`, colour);
      for (const up of [true, false]) {
        const b = bestVmg(t, t.tws[j], up);
        if (b) { const [x, y] = xy(b.twa, b.bsp); s += `<circle class="pe-vmgpt" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="2.6" fill="${colour}"/>`; }
      }
    }
    // the cell being edited
    if (okT && sel && sel.r1 >= 0 && sel.c1 >= 0 && t.v[sel.r1]) {
      const [x, y] = xy(t.twa[sel.r1], t.v[sel.r1][sel.c1]);
      s += `<circle class="pe-cur" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="4.5"/>`;
    }
    s += `<text class="pe-lab" x="${cx + 3}" y="${cy + R + 12}">kn</text>`;
    svg.innerHTML = s;
    svg.onmousemove = (e) => {
      const pt = svg.createSVGPoint(); pt.x = e.clientX; pt.y = e.clientY;
      const p = pt.matrixTransform(svg.getScreenCTM().inverse());
      const a = (Math.atan2(p.x - cx, cy - p.y) * 180) / Math.PI;
      const r = (Math.hypot(p.x - cx, p.y - cy) * top) / R;
      if (a < 0 || a > 180 || !okT) { els.tip.hidden = true; return; }
      // the nearest curve to the pointer
      let best = null;
      t.tws.forEach((w, j) => { const b = bsp(t, a, w); const d = Math.abs(b - r); if (!best || d < best.d) best = { j, w, b, d }; });
      els.tip.hidden = false;
      els.tip.textContent = `TWA ${a.toFixed(0)}° · TWS ${best.w} kn · ${best.b.toFixed(2)} kn`;
      const box = svg.getBoundingClientRect();
      els.tip.style.left = `${e.clientX - box.left + 10}px`; els.tip.style.top = `${e.clientY - box.top + 10}px`;
      if (focusCol !== best.j) { focusCol = best.j; drawPlot(); }
    };
    svg.onmouseleave = () => { els.tip.hidden = true; };
  }

  // ---- the tools ----
  function doUndo() { if (!undo.length) return; redo.push(copy(t)); t = undo.pop(); draw(); }
  function doRedo() { if (!redo.length) return; undo.push(copy(t)); t = redo.pop(); draw(); }
  const range = () => sel ? { ra: Math.min(sel.r0, sel.r1), rb: Math.max(sel.r0, sel.r1), ca: Math.min(sel.c0, sel.c1), cb: Math.max(sel.c0, sel.c1) } : null;
  async function scale() {
    if (!t) return;
    const g = range();
    const some = g && (g.ra !== g.rb || g.ca !== g.cb);
    const v = await ask({ title: some ? "Scale the selected boat speeds" : "Scale every boat speed", note: "100 % leaves them as they are; 95 % takes 5 % off.", fields: [{ key: "pc", label: "Percentage", value: "100" }], ok: "Scale",
      check: (x) => (num(x.pc) && num(x.pc) > 0 && num(x.pc) <= 300 ? "" : "A percentage from 1 to 300") });
    if (!v) return;
    const f = num(v.pc) / 100;
    change(() => {
      t.v.forEach((r, i) => r.forEach((_, j) => {
        if (!some || (i >= g.ra && i <= g.rb && j >= g.ca && j <= g.cb)) t.v[i][j] = Math.round(t.v[i][j] * f * 1000) / 1000;
      }));
    });
  }
  function addRow() {
    if (!t) return;
    const i = Math.max(0, Math.min(t.twa.length - 1, range()?.rb ?? t.twa.length - 1));
    const a = i < t.twa.length - 1 ? (t.twa[i] + t.twa[i + 1]) / 2 : Math.min(180, t.twa[i] + 10);
    if (t.twa.includes(a) || a > 180) { toast("No room for another angle there", 3000); return; }
    change(() => { t.twa.splice(i + 1, 0, Math.round(a * 10) / 10); t.v.splice(i + 1, 0, t.tws.map((w) => Math.round(bsp(t, a, w) * 100) / 100)); });
    goTo(i + 1, -1);
  }
  function addCol() {
    if (!t) return;
    const j = Math.max(0, Math.min(t.tws.length - 1, range()?.cb ?? t.tws.length - 1));
    const w = j < t.tws.length - 1 ? (t.tws[j] + t.tws[j + 1]) / 2 : t.tws[j] + 5;
    change(() => { t.tws.splice(j + 1, 0, w); t.v.forEach((r, i) => r.splice(j + 1, 0, Math.round(bsp({ ...t, tws: t.tws.filter((_, k) => k !== j + 1), v: t.v.map((x) => x.filter((_, k) => k !== j + 1)) }, t.twa[i], w) * 100) / 100)); });
    goTo(-1, j + 1);
  }
  function delRows() {
    const g = range();
    if (!t || !g || g.ra < 0) { toast("Select the wind angle rows to delete first", 3000); return; }
    if (g.rb - g.ra + 1 >= t.twa.length) { toast("A polar needs at least one wind angle", 3000); return; }
    change(() => { t.twa.splice(g.ra, g.rb - g.ra + 1); t.v.splice(g.ra, g.rb - g.ra + 1); });
    sel = null; draw();
  }
  function delCols() {
    const g = range();
    if (!t || !g || g.ca < 0) { toast("Select the wind speed columns to delete first", 3000); return; }
    if (g.cb - g.ca + 1 >= t.tws.length) { toast("A polar needs at least one wind speed", 3000); return; }
    change(() => { t.tws.splice(g.ca, g.cb - g.ca + 1); t.v.forEach((r) => r.splice(g.ca, g.cb - g.ca + 1)); });
    sel = null; focusCol = null; draw();
  }
  async function save(asNew) {
    if (!t || Object.keys(problems(t)).length) return;
    let name = t.name;
    if (asNew) {
      const v = await ask({ title: "Save the polar as", note: "In your polar folder, as a .csv that qtVlm, OpenCPN and TweedPlot read.", fields: [{ key: "name", label: "Name", value: `${t.name} edited` }], ok: "Save",
        check: (x) => (x.name.trim() ? "" : "Give it a name") });
      if (!v) return;
      name = v.name.trim();
    }
    const args = { name, tws: t.tws, twa: t.twa, speeds: t.v };
    let path;
    try { path = await invoke("polar_save", { ...args, overwrite: !asNew && t.yours }); } catch (e) {
      if (String(e) !== "exists") { toast(String(e), 6000); return; }
      const ok = await ask({ title: `Replace your polar "${name}"?`, note: "There is already a polar of yours with that name.", ok: "Replace", cancel: "Cancel" });
      if (!ok) return;
      try { path = await invoke("polar_save", { ...args, overwrite: true }); } catch (e2) { toast(String(e2), 6000); return; }
    }
    const wasInUse = t.path === getBoatPolar();
    t = { ...t, name: path.split(/[\\/]/).pop().replace(/\.[^.]+$/, ""), path, yours: true };
    saved = copy(t);
    toast(`Saved ${path}`, 4000);
    // the polar list and routing follow (the one in use stays in use, now your copy)
    window.dispatchEvent(new CustomEvent("racenav-polars", { detail: { path, use: wasInUse } }));
    render();
  }

  dock.register("polar", {
    title: "Polar",
    placement: () => ({ side: "bottom", minHeight: 480 }),
    render(body, tools) {
      els = { body, tools };
      if (!t || (t.path !== getBoatPolar() && !dirty())) load(getBoatPolar()); else render();
    },
  });
  dock.onChange((cur) => { if (cur !== "polar") els = null; });
  onTheme(() => { if (els) draw(); });
  let fit = null;
  window.addEventListener("racenav-layout", () => { clearTimeout(fit); fit = setTimeout(() => { if (els) drawPlot(); }, 150); });

  return {
    open() { dock.show("polar"); },
    dirty: () => !!t && dirty(),
  };
}
