// The Instruments view: one screen of individual readings on a fixed grid. Drag a tile to
// move it (a tile dropped under another locks to that tile's column), drag its corner to
// size it, click it to change the reading. The grid always fits the screen.
import { GRID, fits, clampTile, snapBelow, placeTile, firstFree } from "./gridmath.js";

const PAD = 6, GAP = 4; // must match #itiles in style.css

export function defaultTiles(items) {
  const keys = ["SOG", "COG", "HDG", "DPT", "TWS", "TWA", "AWS", "AWA", "BTW", "DTW", "XTE", "TTG"];
  return keys.filter((k) => items[k]).map((k, i) => ({
    id: `t${i + 1}`, k, c: (i % 4) * 3 + 1, r: Math.floor(i / 4) * 2 + 1, w: 3, h: 2,
  }));
}

export function setupTiles({ host, menu, items, getLayout, save, paint }) {
  const tiles = () => (getLayout().tiles ??= defaultTiles(items));
  const locked = () => !!getLayout().locked;
  let swallow = false;
  host.addEventListener("click", (e) => { if (swallow) { e.stopPropagation(); e.preventDefault(); } }, true);

  const place = (el, t) => {
    el.style.gridColumn = `${t.c} / span ${t.w}`;
    el.style.gridRow = `${t.r} / span ${t.h}`;
  };
  const byId = (id) => tiles().find((t) => t.id === id);

  function build() {
    host.innerHTML = "";
    if (!tiles().length) {
      host.innerHTML = `<div class="it-empty">No instruments here yet. Use “Add an instrument” in the Instruments panel.</div>`;
      return;
    }
    for (const t of tiles()) {
      const it = items[t.k];
      const el = document.createElement("div");
      el.className = `tile itile${it?.graphic ? " graphic" : ""}${it?.highway ? " highway-card" : ""}${it?.sounder ? " sounder-card" : ""}`;
      el.dataset.id = t.id;
      el.innerHTML = `<div class="lbl"><span>${it ? it.label : t.k}</span><span class="sd"></span></div><div class="v">—</div>`;
      place(el, t);
      if (!locked()) {
        const h = document.createElement("div");
        h.className = "trsz";
        h.title = "Drag to size this instrument";
        el.append(h);
      }
      host.append(el);
    }
    render();
  }

  function render() {
    for (const el of host.querySelectorAll(".itile")) {
      const t = byId(el.dataset.id);
      const it = t && items[t.k];
      if (it) paint(el, it, true);
    }
  }

  // ---------- geometry ----------------------------------------------------------
  function metrics() {
    const r = host.getBoundingClientRect();
    return { left: r.left + PAD, top: r.top + PAD, pw: (r.width - 2 * PAD + GAP) / GRID.cols, ph: (r.height - 2 * PAD + GAP) / GRID.rows };
  }
  const ghostFor = (t) => {
    let g = host.querySelector(".itile-ghost");
    if (!t) { g?.remove(); return; }
    if (!g) { g = document.createElement("div"); g.className = "itile-ghost"; host.append(g); }
    place(g, t);
  };

  // ---------- drag and resize -----------------------------------------------------
  host.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || locked()) return;
    const el = e.target.closest(".itile");
    if (!el) return;
    const t = byId(el.dataset.id);
    if (!t) return;
    if (e.target.closest(".trsz")) { e.preventDefault(); startResize(e, el, t); } else startDrag(e, el, t);
  });

  function startDrag(e, el, t) {
    const m = metrics();
    const r = el.getBoundingClientRect();
    const gx = e.clientX - r.left, gy = e.clientY - r.top;
    const x0 = e.clientX, y0 = e.clientY;
    let dragging = false, target = null;
    const move = (ev) => {
      if (!dragging) {
        if (Math.abs(ev.clientX - x0) + Math.abs(ev.clientY - y0) < 8) return;
        dragging = true;
        el.classList.add("dragging");
      }
      const want = {
        c: Math.round((ev.clientX - gx - m.left) / m.pw) + 1,
        r: Math.round((ev.clientY - gy - m.top) / m.ph) + 1,
      };
      const snapped = snapBelow(tiles(), t, want);
      target = placeTile(tiles(), t, snapped.c, snapped.r);
      ghostFor(target);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      ghostFor(null);
      el.classList.remove("dragging");
      if (!dragging) return;
      swallow = true;
      setTimeout(() => { swallow = false; }, 50);
      if (target && (target.c !== t.c || target.r !== t.r)) { Object.assign(t, { c: target.c, r: target.r }); save(); }
      build();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  }

  function startResize(e, el, t) {
    e.stopPropagation();
    const h = e.target;
    h.setPointerCapture(e.pointerId);
    const m = metrics();
    const r = el.getBoundingClientRect();
    const x0 = e.clientX, y0 = e.clientY;
    let cur = { ...t };
    const move = (ev) => {
      let w = Math.max(1, Math.round((r.width + (ev.clientX - x0) + GAP) / m.pw));
      const hh = Math.max(1, Math.round((r.height + (ev.clientY - y0) + GAP) / m.ph));
      let n = clampTile({ ...t, w, h: hh });
      // never over a neighbour: shrink until it fits
      while (!fits(tiles(), n) && (n.w > 1 || n.h > 1)) n = n.w >= n.h && n.w > 1 ? { ...n, w: n.w - 1 } : n.h > 1 ? { ...n, h: n.h - 1 } : { ...n, w: n.w - 1 };
      if (fits(tiles(), n)) { cur = n; place(el, cur); }
    };
    const up = () => {
      h.removeEventListener("pointermove", move);
      h.removeEventListener("pointerup", up);
      Object.assign(t, { w: cur.w, h: cur.h });
      swallow = true;
      setTimeout(() => { swallow = false; }, 50);
      save(); build();
    };
    h.addEventListener("pointermove", move);
    h.addEventListener("pointerup", up);
  }

  // ---------- choosing the reading --------------------------------------------------
  host.addEventListener("click", (e) => {
    const el = e.target.closest(".itile");
    if (!el || e.target.closest(".trsz")) return;
    const t = byId(el.dataset.id);
    if (t) openMenu(t, el.getBoundingClientRect());
  });

  function openMenu(t, rect) {
    const groups = {};
    for (const [k, it] of Object.entries(items)) if (!it.mobOnly) (groups[it.group] ??= []).push([k, it]);
    let html = "";
    for (const [g, list] of Object.entries(groups)) {
      html += `<div class="grp">${g}</div><div class="items">`;
      for (const [k, it] of list) html += `<button data-set="${k}" class="${k === t.k ? "cur" : ""}">${it.label}${it.unit ? ` <small>${it.unit}</small>` : ""}</button>`;
      html += "</div>";
    }
    html += `<hr><button data-act="remove">Remove this instrument</button>`;
    menu.innerHTML = html;
    menu.hidden = false;
    const mw = 300, mh = Math.min(menu.scrollHeight, window.innerHeight * 0.8);
    let x = rect.right + 6, y = rect.top;
    if (x + mw > window.innerWidth) x = rect.left - mw - 6;
    if (y + mh > window.innerHeight) y = window.innerHeight - mh - 8;
    menu.style.left = `${Math.max(4, x)}px`;
    menu.style.top = `${Math.max(4, y)}px`;
    menu.onclick = (ev) => {
      const b = ev.target.closest("button");
      if (!b) return;
      if (b.dataset.act === "remove") getLayout().tiles = tiles().filter((x) => x !== t);
      else if (b.dataset.set) {
        t.k = b.dataset.set;
        if (items[t.k]?.highway) {
          const p = placeTile(tiles(), { ...t, w: Math.max(t.w, 5), h: Math.max(t.h, 4) }, t.c, t.r);
          if (p) Object.assign(t, p);
        }
      }
      menu.hidden = true;
      save(); build();
    };
  }

  return {
    build, render,
    /** A new instrument in the first free place. */
    add(key = "SOG") {
      const hw = items[key]?.highway;
      const spot = firstFree(tiles(), hw ? 5 : 3, hw ? 4 : 2);
      if (!spot) return false;
      tiles().push({ id: `t${Date.now().toString(36)}`, k: key, c: spot.c, r: spot.r, w: spot.w, h: spot.h });
      save(); build();
      return true;
    },
    reset() { getLayout().tiles = defaultTiles(items); save(); build(); },
  };
}
