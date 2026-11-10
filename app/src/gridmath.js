// Layout maths for the Instruments view's screen of tiles (insttiles.js), kept free of the DOM
// so it can be tested.

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
