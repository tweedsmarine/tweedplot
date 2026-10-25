// Layout maths for instrument panels, kept free of the DOM so it can be tested.
// A panel is a grid: its width decides the number of columns, each reading can
// span several columns and rows, and readings are reordered by dragging.

export const CELL = { s: 82, m: 110, l: 150 };
export const ROW = { s: 54, m: 70, l: 96 };
export const MAX_SPAN = 6;

/** Number of grid columns for a panel `w` px wide. */
export function columns(w, size = "m") {
  return Math.max(1, Math.round(w / (CELL[size] ?? CELL.m)));
}

/** A span (1..MAX_SPAN cells) after dragging a resize handle `delta` px from a `start` px extent. */
export function spanFromDrag(start, delta, pitch, max = MAX_SPAN) {
  if (!(pitch > 0)) return 1;
  return Math.max(1, Math.min(max, Math.round((start + delta) / pitch)));
}

/** Normalise a saved span to [columns, rows], each 1..MAX_SPAN. */
export function cleanSpan(s) {
  if (!Array.isArray(s)) return [1, 1];
  const c = (n) => Math.max(1, Math.min(MAX_SPAN, Math.round(Number(n)) || 1));
  return [c(s[0]), c(s[1])];
}

/**
 * Move reading `from` in `tiles` to index `to` (the position it should have afterwards),
 * taking its span with it. Returns new arrays; the inputs are not changed.
 */
export function moveReading(tiles, spans, from, to) {
  const t = tiles.slice();
  const s = tiles.map((_, i) => spans?.[i] ?? null);
  const [tt] = t.splice(from, 1);
  const [ss] = s.splice(from, 1);
  const at = Math.max(0, Math.min(t.length, to));
  t.splice(at, 0, tt);
  s.splice(at, 0, ss);
  return { tiles: t, spans: s };
}

/**
 * Where a dragged reading lands when dropped on reading `target` at pointer fraction
 * (`fx`, `fy`) across it: before it in the first half (reading order), after it in the second.
 */
export function dropIndex(target, fx, fy, wide = true) {
  const after = wide ? fx > 0.5 : fy > 0.5;
  return target + (after ? 1 : 0);
}

// ---------- the Instruments view: one screen of tiles on a fixed grid ----------
// A tile is { id, k, c, r, w, h }: its reading, its first column and row (from 1) and its
// size in cells. The grid always fits the screen, so a tile can never end up off it.

export const GRID = { cols: 12, rows: 8 };

const overlaps = (a, b) => a.c < b.c + b.w && b.c < a.c + a.w && a.r < b.r + b.h && b.r < a.r + a.h;
const inside = (t, g) => t.c >= 1 && t.r >= 1 && t.c + t.w - 1 <= g.cols && t.r + t.h - 1 <= g.rows;

/** Does `t` fit the grid without covering any tile except itself? */
export function fits(tiles, t, g = GRID) {
  return inside(t, g) && !tiles.some((o) => o.id !== t.id && overlaps(t, o));
}

/** Clamp a tile's size and position into the grid. */
export function clampTile(t, g = GRID) {
  const w = Math.max(1, Math.min(g.cols, Math.round(t.w) || 1));
  const h = Math.max(1, Math.min(g.rows, Math.round(t.h) || 1));
  return { ...t, w, h, c: Math.max(1, Math.min(g.cols - w + 1, Math.round(t.c) || 1)), r: Math.max(1, Math.min(g.rows - h + 1, Math.round(t.r) || 1)) };
}

/**
 * A tile dropped just under another locks to that tile's column (and sits against its
 * bottom edge): `want` is where the pointer put it; the result is the cell it snaps to.
 */
export function snapBelow(tiles, t, want, g = GRID) {
  let { c, r } = want;
  for (const o of tiles) {
    if (o.id === t.id) continue;
    const touchesBelow = Math.abs(r - (o.r + o.h)) <= 1 && want.c < o.c + o.w && want.c + t.w > o.c;
    if (touchesBelow && Math.abs(c - o.c) <= 1) { c = o.c; r = o.r + o.h; break; }
  }
  return clampTile({ ...t, c, r }, g);
}

/**
 * Put `t` at (c, r). If something is there, it keeps the column and moves down to the first
 * free row, then up, then anywhere. Returns the placed tile, or null if the grid is full.
 */
export function placeTile(tiles, t, c, r, g = GRID) {
  const want = clampTile({ ...t, c, r }, g);
  if (fits(tiles, want, g)) return want;
  const down = (from, to, step) => {
    for (let y = from; y !== to; y += step) { const n = { ...want, r: y }; if (fits(tiles, n, g)) return n; }
    return null;
  };
  const maxR = g.rows - want.h + 1;
  const found = down(want.r + 1, maxR + 1, 1) ?? down(want.r - 1, 0, -1);
  if (found) return found;
  return firstFree(tiles.filter((o) => o.id !== t.id), want.w, want.h, t.id, g);
}

/** The first free place for a w × h tile, reading order; shrinks it if need be. */
export function firstFree(tiles, w, h, id = "", g = GRID) {
  for (let ww = w, hh = h; ww >= 1 && hh >= 1; ww > 1 ? ww-- : hh--) {
    for (let r = 1; r + hh - 1 <= g.rows; r++) {
      for (let c = 1; c + ww - 1 <= g.cols; c++) {
        const n = { id, c, r, w: ww, h: hh };
        if (fits(tiles, n, g)) return n;
      }
    }
  }
  return null;
}

/** A size in cells for a tile `px` wide on a grid of `cols` cells across `total` px. */
export function cellsFromPx(px, total, cols) {
  return Math.max(1, Math.round((px / total) * cols));
}
