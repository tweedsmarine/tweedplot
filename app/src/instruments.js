// Floating instrument panels: rows or columns of readings, dragged anywhere,
// always on top of the chart. Each reading can show a state ("stale", "warn",
// "alarm") that styles the tile, ready for user-set warnings and alarms later.

const KEY = "racenav.instruments";
const STALE_S = 10;

// ---------- what can be shown ---------------------------------------------------------
const f1 = (x) => x.toFixed(1);
const deg3 = (x) => String(Math.round(((x % 360) + 360) % 360) % 360).padStart(3, "0");
const sideAngle = (x) => `${Math.round(Math.abs(x))}`;

/**
 * Each item: label, unit, group, get(ctx) → value or null, fmt(value) → text,
 * optional state(value, ctx) → "" | "warn" | "alarm", side(value) → "port" | "stbd",
 * wide (needs a wide tile), live (from Signal K: greys out when the data stops).
 */
export const ITEMS = {
  SOG: { label: "SOG", unit: "kn", group: "Navigation", live: true, get: (c) => c.v.sog, fmt: f1 },
  COG: { label: "COG", unit: "°T", group: "Navigation", live: true, get: (c) => c.v.cog, fmt: deg3 },
  HDG: { label: "HDG", unit: "°T", group: "Navigation", live: true, get: (c) => c.v.heading, fmt: deg3 },
  HDM: { label: "HDG", unit: "°M", group: "Navigation", live: true, get: (c) => c.v.heading_mag, fmt: deg3 },
  BSP: { label: "BSP", unit: "kn", group: "Navigation", live: true, get: (c) => c.v.stw, fmt: f1 },
  DPT: {
    label: "Depth", unit: "m", group: "Navigation", live: true, get: (c) => c.v.depth ?? c.v.depth_surface, fmt: f1,
    // default depth warning: below safety depth + 2 m warns, below the safety depth alarms
    state: (d, c) => (d < c.safetyDepth ? "alarm" : d < c.safetyDepth + 2 ? "warn" : ""),
  },
  POS: { label: "Position", unit: "", group: "Navigation", live: true, wide: true, get: (c) => (c.v.lat != null && c.v.lon != null ? [c.v.lat, c.v.lon] : null), fmt: (p) => `${ddm(p[0], "N", "S")}<br>${ddm(p[1], "E", "W")}` },
  LOG: { label: "Log", unit: "NM", group: "Navigation", live: true, get: (c) => c.v.log, fmt: (x) => x.toFixed(0) },
  TRIP: { label: "Trip", unit: "NM", group: "Navigation", live: true, get: (c) => c.v.trip, fmt: f1 },
  UTC: { label: "UTC", unit: "", group: "Navigation", get: (c) => c.now, fmt: (t) => new Date(t).toISOString().slice(11, 19) },
  LOCAL: { label: "Local", unit: "", group: "Navigation", get: (c) => c.now, fmt: (t) => new Date(t).toLocaleTimeString("en-NZ", { hour12: false }) },

  AWS: { label: "AWS", unit: "kn", group: "Wind", live: true, get: (c) => c.v.aws, fmt: f1 },
  AWA: { label: "AWA", unit: "°", group: "Wind", live: true, get: (c) => c.v.awa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  TWS: { label: "TWS", unit: "kn", group: "Wind", live: true, get: (c) => c.v.tws, fmt: f1 },
  TWA: { label: "TWA", unit: "°", group: "Wind", live: true, get: (c) => c.v.twa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  TWD: { label: "TWD", unit: "°T", group: "Wind", live: true, get: (c) => c.v.twd, fmt: deg3 },
  VMG: { label: "VMG", unit: "kn", group: "Wind", live: true, get: (c) => (c.v.twa != null && (c.v.stw ?? c.v.sog) != null ? (c.v.stw ?? c.v.sog) * Math.cos((c.v.twa * Math.PI) / 180) : null), fmt: (x) => f1(Math.abs(x)) },

  NEXT: { label: "To mark", unit: "", group: "Route", wide: true, get: (c) => c.nav?.next?.name ?? null, fmt: (s) => esc(s) },
  BTW: { label: "BTW", unit: "°T", group: "Route", get: (c) => c.wp?.btw, fmt: deg3 },
  DTW: { label: "DTW", unit: "NM", group: "Route", get: (c) => c.wp?.dtw, fmt: (x) => (x < 10 ? x.toFixed(2) : x.toFixed(1)) },
  VMC: { label: "VMC", unit: "kn", group: "Route", live: true, get: (c) => c.wp?.vmc, fmt: f1 },
  TTG: { label: "TTG", unit: "", group: "Route", get: (c) => c.wp?.ttg, fmt: hm },
  ETA: { label: "ETA", unit: "", group: "Route", wide: true, get: (c) => (c.wp?.ttg != null ? c.now + c.wp.ttg * 1000 : null), fmt: (t) => new Date(t).toLocaleString("en-NZ", { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false }) },
  // + = boat to the right of the track (steer left to get back)
  XTE: { label: "XTE", unit: "", group: "Route", get: (c) => c.wp?.xte, fmt: (x) => `${Math.abs(x) < 10 ? Math.abs(x).toFixed(2) : Math.abs(x).toFixed(1)}<span class="u">NM ${x >= 0 ? "R" : "L"}</span>` },
  TTWA: { label: "Tgt TWA", unit: "°", group: "Route", get: (c) => c.target?.twa, fmt: sideAngle, side: (x) => (x < 0 ? "port" : "stbd") },
  TBSP: { label: "Tgt BSP", unit: "kn", group: "Route", get: (c) => c.target?.bsp, fmt: f1 },
  THDG: { label: "Tgt HDG", unit: "°T", group: "Route", get: (c) => c.target?.heading, fmt: deg3 },
  PERF: { label: "BSP / tgt", unit: "%", group: "Route", live: true, get: (c) => (c.target?.bsp > 0.2 && c.v.stw != null ? (100 * c.v.stw) / c.target.bsp : null), fmt: (x) => x.toFixed(0) },
};

const DEFAULT_LAYOUT = {
  v: 1,
  locked: false,
  panels: [
    { id: "p1", x: 0.3, y: 0.86, dir: "row", size: "m", tiles: ["SOG", "COG", "HDG", "DPT", "TWS", "TWA"] },
    { id: "p2", x: 0.86, y: 0.12, dir: "column", size: "s", tiles: ["NEXT", "BTW", "DTW", "TTG", "XTE"] },
  ],
};

export function setupInstruments() {
  const root = document.getElementById("inst");
  const menu = document.getElementById("tile-menu");
  let layout = loadLayout();
  let ctx = { v: {}, now: Date.now(), age: Infinity, safetyDepth: 3 };

  function save() { try { localStorage.setItem(KEY, JSON.stringify(layout)); } catch { /* ignore */ } }

  // ---------- build ----------------------------------------------------------
  function build() {
    root.innerHTML = "";
    root.classList.toggle("locked", layout.locked);
    for (const p of layout.panels) {
      const el = document.createElement("div");
      el.className = `ipanel ${p.dir} ${p.size}`;
      el.dataset.id = p.id;
      const grip = document.createElement("div");
      grip.className = "grip";
      grip.textContent = "⋮⋮";
      grip.title = "Drag to move this panel";
      el.append(grip);
      p.tiles.forEach((key, i) => {
        const it = ITEMS[key];
        const t = document.createElement("div");
        t.className = `tile ${p.size}${it?.wide ? " wide" : ""}`;
        t.dataset.i = i;
        t.innerHTML = `<div class="lbl"><span>${it ? it.label : key}</span><span class="sd"></span></div><div class="v">—</div>`;
        el.append(t);
      });
      root.append(el);
      place(el, p);
      dragByGrip(el, grip, p);
    }
    renderList();
    render();
  }

  function place(el, p) {
    const W = window.innerWidth, H = window.innerHeight;
    const w = el.offsetWidth, h = el.offsetHeight;
    // keep clear of the dock at the bottom when it is open
    const dockH = parseInt(getComputedStyle(document.documentElement).getPropertyValue("--dock-h")) || 0;
    const x = Math.max(0, Math.min(W - w, p.x * W));
    const y = Math.max(0, Math.min(H - dockH - h, p.y * H));
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
  }
  const replace = () => { for (const el of root.children) place(el, panel(el.dataset.id)); };
  window.addEventListener("resize", replace);
  window.addEventListener("racenav-layout", replace);
  const panel = (id) => layout.panels.find((p) => p.id === id);

  function dragByGrip(el, grip, p) {
    grip.addEventListener("pointerdown", (e) => {
      if (layout.locked) return;
      e.preventDefault();
      grip.setPointerCapture(e.pointerId);
      const r = el.getBoundingClientRect();
      const dx = e.clientX - r.left, dy = e.clientY - r.top;
      grip.style.cursor = "grabbing";
      const move = (ev) => {
        const W = window.innerWidth, H = window.innerHeight;
        let x = ev.clientX - dx, y = ev.clientY - dy;
        // snap to the screen edges
        if (x < 12) x = 0;
        if (y < 12) y = 0;
        if (W - (x + r.width) < 12) x = W - r.width;
        if (H - (y + r.height) < 12) y = H - r.height;
        p.x = x / W; p.y = y / H;
        place(el, p);
      };
      const up = () => {
        grip.removeEventListener("pointermove", move);
        grip.removeEventListener("pointerup", up);
        grip.style.cursor = "";
        save();
      };
      grip.addEventListener("pointermove", move);
      grip.addEventListener("pointerup", up);
    });
  }

  // ---------- values ----------------------------------------------------------
  function render() {
    for (const el of root.children) {
      const p = panel(el.dataset.id);
      if (!p) continue;
      for (const t of el.querySelectorAll(".tile")) {
        const it = ITEMS[p.tiles[Number(t.dataset.i)]];
        if (!it) continue;
        let val = null;
        try { val = it.get(ctx); } catch { val = null; }
        const has = val !== null && val !== undefined && !(typeof val === "number" && !Number.isFinite(val));
        const stale = it.live && ctx.age > STALE_S;
        const state = has && !stale && it.state ? it.state(val, ctx) : "";
        const html = has ? it.fmt(val) + (it.unit ? `<span class="u">${it.unit}</span>` : "") : "—";
        const v = t.querySelector(".v");
        if (v.innerHTML !== html) v.innerHTML = html;
        const sd = t.querySelector(".sd");
        const side = has && it.side ? it.side(val) : "";
        sd.className = `sd ${side}`;
        sd.textContent = side === "port" ? "PORT" : side === "stbd" ? "STBD" : "";
        t.classList.toggle("stale", stale);
        t.classList.toggle("warn", state === "warn");
        t.classList.toggle("alarm", state === "alarm");
      }
    }
  }
  setInterval(() => { ctx.now = Date.now(); ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000; render(); }, 1000);

  // ---------- tile menu ----------------------------------------------------------
  root.addEventListener("click", (e) => {
    const t = e.target.closest(".tile");
    if (!t) return;
    const p = panel(t.parentElement.dataset.id);
    openMenu(p, Number(t.dataset.i), t.getBoundingClientRect());
  });
  function openMenu(p, i, rect) {
    const cur = p.tiles[i];
    const groups = {};
    for (const [k, it] of Object.entries(ITEMS)) (groups[it.group] ??= []).push([k, it]);
    let html = "";
    for (const [g, items] of Object.entries(groups)) {
      html += `<div class="grp">${g}</div><div class="items">`;
      for (const [k, it] of items) html += `<button data-set="${k}" class="${k === cur ? "cur" : ""}">${it.label}${it.unit ? ` <small>${it.unit}</small>` : ""}</button>`;
      html += "</div>";
    }
    html += `<hr><button data-act="add">Add a reading after this one</button><button data-act="remove">Remove this reading</button>`;
    html += `<hr><div class="grp">This panel</div><div class="sizes">${["s", "m", "l"].map((s) => `<button data-size="${s}" class="${p.size === s ? "cur" : ""}">${{ s: "Small", m: "Medium", l: "Large" }[s]}</button>`).join("")}</div>`;
    html += `<button data-act="dir">${p.dir === "row" ? "Stack vertically" : "Lay out horizontally"}</button><button data-act="delpanel">Remove panel</button>`;
    menu.innerHTML = html;
    menu.hidden = false;
    // keep it on screen
    const mw = 300, mh = Math.min(menu.scrollHeight, window.innerHeight * 0.8);
    let x = rect.right + 6, y = rect.top;
    if (x + mw > window.innerWidth) x = rect.left - mw - 6;
    if (y + mh > window.innerHeight) y = window.innerHeight - mh - 8;
    menu.style.left = `${Math.max(4, x)}px`;
    menu.style.top = `${Math.max(4, y)}px`;
    menu.onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      if (b.dataset.set) p.tiles[i] = b.dataset.set;
      if (b.dataset.size) p.size = b.dataset.size;
      if (b.dataset.act === "add") p.tiles.splice(i + 1, 0, cur);
      if (b.dataset.act === "remove") { p.tiles.splice(i, 1); if (!p.tiles.length) layout.panels = layout.panels.filter((x) => x !== p); }
      if (b.dataset.act === "dir") p.dir = p.dir === "row" ? "column" : "row";
      if (b.dataset.act === "delpanel") layout.panels = layout.panels.filter((x) => x !== p);
      menu.hidden = true;
      save();
      build();
    };
  }
  document.addEventListener("pointerdown", (e) => { if (!menu.hidden && !menu.contains(e.target) && !e.target.closest(".tile")) menu.hidden = true; });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !menu.hidden) { menu.hidden = true; e.preventDefault(); } });

  // ---------- drawer controls ----------------------------------------------------------
  const lockBtn = document.getElementById("inst-lock");
  function renderList() {
    lockBtn.classList.toggle("on", layout.locked);
    lockBtn.textContent = layout.locked ? "Layout locked" : "Lock layout";
    const ul = document.getElementById("inst-panels");
    ul.innerHTML = "";
    if (!layout.panels.length) ul.innerHTML = `<li class="empty">No panels. Add one.</li>`;
    layout.panels.forEach((p, k) => {
      const li = document.createElement("li");
      li.innerHTML = `<div class="main"><b>Panel ${k + 1}</b><small></small></div><button data-act="dir">${p.dir === "row" ? "Vertical" : "Horizontal"}</button><button data-act="del">Remove</button>`;
      li.querySelector("small").textContent = p.tiles.map((t) => ITEMS[t]?.label ?? t).join(", ");
      li.querySelector('[data-act="dir"]').onclick = () => { p.dir = p.dir === "row" ? "column" : "row"; save(); build(); };
      li.querySelector('[data-act="del"]').onclick = () => { layout.panels.splice(k, 1); save(); build(); };
      ul.append(li);
    });
  }
  document.getElementById("inst-add").onclick = () => {
    layout.panels.push({ id: `p${Date.now().toString(36)}`, x: 0.45, y: 0.4, dir: "row", size: "m", tiles: ["SOG", "TWS"] });
    save(); build();
  };
  lockBtn.onclick = () => { layout.locked = !layout.locked; save(); build(); };
  document.getElementById("inst-reset").onclick = () => { layout = structuredClone(DEFAULT_LAYOUT); save(); build(); };

  build();

  return {
    /** New data: vessel state, route/waypoint and routing targets. */
    update(next) {
      ctx = { ...ctx, ...next, now: Date.now() };
      ctx.age = (Date.now() - (ctx.dataAt ?? 0)) / 1000;
      render();
    },
  };
}

function loadLayout() {
  try {
    const l = JSON.parse(localStorage.getItem(KEY) || "null");
    if (l && l.v === 1 && Array.isArray(l.panels)) return l;
  } catch { /* ignore */ }
  return structuredClone(DEFAULT_LAYOUT);
}

function hm(s) {
  if (s > 99 * 3600) return "&gt;99h";
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return h ? `${h}<span class="u">h</span>${String(m).padStart(2, "0")}` : `${m}<span class="u">min</span>`;
}

function ddm(deg, pos, neg) {
  const h = deg >= 0 ? pos : neg;
  const a = Math.abs(deg);
  let d = Math.floor(a);
  let m = (a - d) * 60;
  if (m >= 59.9995) { d += 1; m = 0; }
  return `${d}°${m.toFixed(3).padStart(6, "0")}'${h}`;
}

function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]); }

// ---------- route / waypoint maths ----------------------------------------------------------
const R_NM = 3440.065;
const rad = (d) => (d * Math.PI) / 180;
export function distNm(a, b) {
  const [l1, p1] = [rad(a[0]), rad(a[1])], [l2, p2] = [rad(b[0]), rad(b[1])];
  const h = Math.sin((p2 - p1) / 2) ** 2 + Math.cos(p1) * Math.cos(p2) * Math.sin((l2 - l1) / 2) ** 2;
  return 2 * Math.asin(Math.min(1, Math.sqrt(h))) * R_NM;
}
export function bearing(a, b) {
  const [l1, p1] = [rad(a[0]), rad(a[1])], [l2, p2] = [rad(b[0]), rad(b[1])];
  const y = Math.sin(l2 - l1) * Math.cos(p2);
  const x = Math.cos(p1) * Math.sin(p2) - Math.sin(p1) * Math.cos(p2) * Math.cos(l2 - l1);
  return ((Math.atan2(y, x) * 180) / Math.PI + 360) % 360;
}

/**
 * Waypoint numbers for the boat at `pos` ([lon, lat]) sailing `sog`/`cog`
 * towards `to`, on the leg that started at `from`.
 */
export function waypointData(pos, sog, cog, from, to) {
  const dtw = distNm(pos, to);
  const btw = bearing(pos, to);
  const vmc = sog != null && cog != null ? sog * Math.cos(rad(cog - btw)) : null;
  const ttg = vmc != null && vmc > 0.1 ? (dtw / vmc) * 3600 : null;
  let xte = null;
  if (from) {
    // cross-track distance from the great circle from → to (+ = boat right of the track)
    const d13 = distNm(from, pos) / R_NM;
    const t13 = rad(bearing(from, pos)), t12 = rad(bearing(from, to));
    xte = Math.asin(Math.sin(d13) * Math.sin(t13 - t12)) * R_NM;
  }
  return { dtw, btw, vmc, ttg, xte };
}

/** The routing's point at time `t` (unix s): heading, TWA, BSP, interpolated. */
export function targetAt(points, t) {
  if (!points?.length || t < points[0].t || t > points.at(-1).t) return null;
  let i = points.findIndex((p) => p.t > t);
  if (i <= 0) return points.at(-1);
  const a = points[i - 1];
  return a; // the leg being sailed: targets hold until the next routing point
}
