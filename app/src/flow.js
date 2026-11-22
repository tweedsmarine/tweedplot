// Animated flow: particles drifting with a vector field (ocean currents first; wind could use it
// later), drawn on a canvas over the chart, as on earth.nullschool or Windy.
//
// The field is a regular lat/lon grid of east and north components (knots). It is turned into a
// screen-space velocity grid whenever the chart stops moving, so each frame only looks up a small
// array: no projection per particle. While the chart moves the particles are cleared, and they
// start again from new random places when it stops.

/** A field on a regular grid: values east (u) and north (v), knots; missing as null or NaN. */
export function fieldFromDirSpeed(dir, spd) {
  // dir: the way the flow goes, degrees true; spd: knots (grib_grid "currentdir" and "current")
  if (!dir || !spd || dir.ni !== spd.ni || dir.nj !== spd.nj) return null;
  const n = dir.ni * dir.nj, u = new Float32Array(n), v = new Float32Array(n);
  for (let k = 0; k < n; k++) {
    const d = dir.values[k], s = spd.values[k];
    if (d == null || s == null) { u[k] = NaN; v[k] = NaN; continue; }
    const r = (d * Math.PI) / 180;
    u[k] = s * Math.sin(r);
    v[k] = s * Math.cos(r);
  }
  return { lon0: dir.lon0, lat0: dir.lat0, dlon: dir.dlon, dlat: dir.dlat, ni: dir.ni, nj: dir.nj, u, v };
}

/** Bilinear value of the field at a point: [u, v] knots, or null off the grid or by land. */
export function sampleField(f, lon, lat) {
  let x = (lon - f.lon0) / f.dlon;
  // the grid may run past 180°: try the point a turn either way
  if (x < 0 || x > f.ni - 1) {
    const turns = [360, -360].map((t) => (lon + t - f.lon0) / f.dlon).find((xx) => xx >= 0 && xx <= f.ni - 1);
    if (turns == null) return null;
    x = turns;
  }
  const y = (lat - f.lat0) / f.dlat;
  if (y < 0 || y > f.nj - 1) return null;
  const i0 = Math.floor(x), j0 = Math.floor(y), i1 = Math.min(i0 + 1, f.ni - 1), j1 = Math.min(j0 + 1, f.nj - 1);
  const tx = x - i0, ty = y - j0;
  const at = (a, i, j) => a[j * f.ni + i];
  const out = [];
  for (const a of [f.u, f.v]) {
    const c = [at(a, i0, j0), at(a, i1, j0), at(a, i0, j1), at(a, i1, j1)];
    if (c.some((z) => !Number.isFinite(z))) {
      // by land: the nearest corner that has a value, or nothing
      const near = c[(tx < 0.5 ? 0 : 1) + (ty < 0.5 ? 0 : 2)];
      if (!Number.isFinite(near)) return null;
      out.push(near);
      continue;
    }
    const top = c[0] + (c[1] - c[0]) * tx, bot = c[2] + (c[3] - c[2]) * tx;
    out.push(top + (bot - top) * ty);
  }
  return out;
}

/**
 * Particles over `map`. Options: `pxPerKnot` screen speed of one knot (pixels a second; weak
 * flow is helped along by `speedPower` < 1 so half a knot still visibly moves),
 * `colour` (a function of knots, or a fixed CSS colour), `density` particles per 10 000 px².
 * Returns { setField(field|null), setOptions(o), destroy() }.
 */
export function createFlow(map, opts = {}) {
  // tuned on the boat PC (22 Nov): faster, longer trails, thicker, with a dark edge so they show on
  // pale shading as well as dark water
  const o = { pxPerKnot: 55, speedPower: 0.7, density: 5, life: [180, 400], fade: 0.015, width: 2.2, casing: "rgba(10,20,28,0.8)", maxKn: 4, colour: (kn) => speedColour(kn), ...opts };
  const canvas = document.createElement("canvas");
  canvas.className = "flow-canvas";
  Object.assign(canvas.style, { position: "absolute", left: "0", top: "0", pointerEvents: "none" });
  // over the chart's own canvas, under the markers and pop-ups MapLibre adds after it
  map.getCanvasContainer().insertBefore(canvas, map.getCanvas().nextSibling);
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  let field = null, grid = null, parts = [], raf = 0, moving = false, last = 0, dpr = 1, frames = 0;
  const CELL = 6; // screen velocity grid spacing, px

  function size() {
    const c = map.getCanvas();
    dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth, h = c.clientHeight;
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    return { w, h };
  }

  /** Screen-space velocity (px a second) every CELL px, from the field, for the chart as it is now. */
  function build(reseed = true) {
    const { w, h } = size();
    if (!field || !w || !h) { grid = null; return; }
    const nx = Math.ceil(w / CELL) + 1, ny = Math.ceil(h / CELL) + 1;
    const vx = new Float32Array(nx * ny).fill(NaN), vy = new Float32Array(nx * ny).fill(NaN), kn = new Float32Array(nx * ny).fill(NaN);
    const b = (-map.getBearing() * Math.PI) / 180, cb = Math.cos(b), sb = Math.sin(b);
    let any = false;
    for (let j = 0; j < ny; j++) {
      for (let i = 0; i < nx; i++) {
        const ll = map.unproject([i * CELL, j * CELL]);
        const s = sampleField(field, ll.lng, ll.lat);
        if (!s) continue;
        // east is +x, north is -y on a north-up screen; turned with the chart's bearing
        const ex = s[0], ny_ = -s[1];
        const k = j * nx + i;
        const knots = Math.hypot(s[0], s[1]);
        const scale = knots > 0 ? (o.pxPerKnot * knots ** o.speedPower) / knots : 0;
        vx[k] = (ex * cb - ny_ * sb) * scale;
        vy[k] = (ex * sb + ny_ * cb) * scale;
        kn[k] = Math.hypot(s[0], s[1]);
        any = true;
      }
    }
    grid = any ? { nx, ny, vx, vy, kn, w, h } : null;
    if (reseed || !grid) seed();
  }
  const lookup = (x, y) => {
    const i = Math.round(x / CELL), j = Math.round(y / CELL);
    if (i < 0 || j < 0 || i >= grid.nx || j >= grid.ny) return -1;
    const k = j * grid.nx + i;
    return Number.isFinite(grid.vx[k]) ? k : -1;
  };
  function spawn(p) {
    for (let tries = 0; tries < 12; tries++) {
      const x = Math.random() * grid.w, y = Math.random() * grid.h;
      if (lookup(x, y) >= 0) { p.x = x; p.y = y; p.age = 0; p.life = o.life[0] + Math.random() * (o.life[1] - o.life[0]); return true; }
    }
    p.age = p.life = 1e9; // nowhere with a value (mostly land): rests
    return false;
  }
  function seed() {
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (!grid) { parts = []; return; }
    const n = Math.min(6000, Math.round((grid.w * grid.h) / 10000 * o.density));
    parts = Array.from({ length: n }, () => { const p = {}; spawn(p); p.age = Math.random() * p.life; return p; });
  }

  function frame(now) {
    raf = 0;
    if (!grid || moving || document.hidden) return;
    const dt = Math.min(0.1, last ? (now - last) / 1000 : 1 / 30);
    last = now;
    // trails: fade what is there. Canvas alpha (0–255) stops fading once a·fade rounds to nothing
    // (below about 0.5/fade), so every 20 frames whatever is that faint is wiped, rather than left
    // as ghost trails: a trail lasts about ln(floor/255)/ln(1 − fade) frames (≈ 2 s at 0.015).
    if (++frames % 20 === 0) sweep();
    ctx.save();
    ctx.globalCompositeOperation = "destination-out";
    ctx.fillStyle = `rgba(0,0,0,${o.fade})`;
    ctx.fillRect(0, 0, grid.w, grid.h);
    ctx.restore();
    ctx.lineWidth = o.width;
    ctx.lineCap = "round";
    // draw in a few colour buckets (by speed) so the stroke style changes rarely
    const buckets = new Map();
    for (const p of parts) {
      if (p.age++ > p.life) { spawn(p); continue; }
      const k = lookup(p.x, p.y);
      if (k < 0) { spawn(p); continue; }
      const nx = p.x + grid.vx[k] * dt, ny = p.y + grid.vy[k] * dt;
      const c = typeof o.colour === "function" ? o.colour(Math.min(grid.kn[k], o.maxKn)) : o.colour;
      let seg = buckets.get(c);
      if (!seg) buckets.set(c, (seg = []));
      seg.push(p.x, p.y, nx, ny);
      p.x = nx; p.y = ny;
    }
    const path = (seg) => { ctx.beginPath(); for (let i = 0; i < seg.length; i += 4) { ctx.moveTo(seg[i], seg[i + 1]); ctx.lineTo(seg[i + 2], seg[i + 3]); } };
    // a dark edge first, under every line, then the light lines
    if (o.casing) {
      // square-ended, so it doesn't cover the end of the line drawn on the last frame
      ctx.lineWidth = o.width + 2.4;
      ctx.lineCap = "butt";
      ctx.strokeStyle = o.casing;
      for (const seg of buckets.values()) { path(seg); ctx.stroke(); }
      ctx.lineWidth = o.width;
      ctx.lineCap = "round";
    }
    for (const [c, seg] of buckets) {
      ctx.strokeStyle = c;
      path(seg);
      ctx.stroke();
    }
    raf = requestAnimationFrame(frame);
  }
  function sweep() {
    const img = ctx.getImageData(0, 0, canvas.width, canvas.height), d = img.data;
    let changed = false;
    const floor = Math.ceil(0.5 / o.fade) + 2;
    for (let i = 3; i < d.length; i += 4) if (d[i] && d[i] < floor) { d[i] = 0; changed = true; }
    if (changed) ctx.putImageData(img, 0, 0);
  }
  const run = () => { if (!raf && grid && !moving) { last = 0; raf = requestAnimationFrame(frame); } };
  const stop = () => { if (raf) cancelAnimationFrame(raf); raf = 0; };

  const onMoveStart = () => { moving = true; stop(); ctx.clearRect(0, 0, canvas.width, canvas.height); };
  const onMoveEnd = () => { moving = false; build(); run(); };
  const onResize = () => { build(); run(); };
  const onVis = () => { if (!document.hidden) run(); };
  map.on("movestart", onMoveStart);
  map.on("moveend", onMoveEnd);
  map.on("resize", onResize);
  document.addEventListener("visibilitychange", onVis);

  return {
    /** A new field (a new time step or area), or null to stop and clear. */
    setField(f) {
      field = f;
      if (!f) { stop(); grid = null; parts = []; ctx.clearRect(0, 0, canvas.width, canvas.height); return; }
      if (moving) return;
      // keep the particles where they are if there are some (a time step), else seed
      const had = parts.length > 0 && grid;
      build(!had);
      if (had && grid) for (const p of parts) if (lookup(p.x, p.y) < 0) spawn(p);
      run();
    },
    setOptions(x) { Object.assign(o, x); if (grid) { build(); run(); } },
    destroy() {
      stop();
      map.off("movestart", onMoveStart); map.off("moveend", onMoveEnd); map.off("resize", onResize);
      document.removeEventListener("visibilitychange", onVis);
      canvas.remove();
    },
  };
}

/** Particle colour by speed: pale and see-through when slack, bright white in a strong current. */
export function speedColour(kn) {
  const a = Math.min(1, 0.6 + kn * 0.4);
  const b = Math.round(Math.min(1, kn / 2) * 10) / 10; // ten steps
  const g = Math.round(235 + 20 * b);
  return `rgba(${g},${g},255,${(Math.round(a * 10) / 10).toFixed(1)})`;
}
