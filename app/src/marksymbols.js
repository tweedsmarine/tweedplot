// Mark symbols a skipper picks for their own marks: shapes, seabed and dangers, and fish.
// Drawn on demand in the mark's own colour ("sym:<kind>:<#rrggbb>"), with a white halo so
// they read on any chart, like the symbols a plotter (TimeZero, Olex) offers. A mark of one
// of these kinds has kind "sym-…".

/** [kind, label, group], in menu order. */
export const MARK_SYMBOLS = [
  ["sym-circle", "Circle", "Shapes"], ["sym-circle-f", "Filled circle", "Shapes"],
  ["sym-square", "Square", "Shapes"], ["sym-square-f", "Filled square", "Shapes"],
  ["sym-diamond", "Diamond", "Shapes"], ["sym-diamond-f", "Filled diamond", "Shapes"],
  ["sym-tri-up", "Triangle up", "Shapes"], ["sym-tri-up-f", "Filled triangle up", "Shapes"],
  ["sym-tri-down", "Triangle down", "Shapes"], ["sym-tri-down-f", "Filled triangle down", "Shapes"],
  ["sym-cross", "Cross", "Shapes"], ["sym-x", "X", "Shapes"], ["sym-star", "Star", "Shapes"],
  ["sym-flag", "Flag", "Shapes"], ["sym-dot", "Dot", "Shapes"],
  ["sym-rock", "Rock", "Seabed and dangers"], ["sym-wreck", "Wreck", "Seabed and dangers"],
  ["sym-snag", "Obstruction / snag", "Seabed and dangers"],
  ["sym-fish", "Fish", "Fish"], ["sym-school", "School of fish", "Fish"], ["sym-tuna", "Tuna", "Fish"],
  ["sym-swordfish", "Swordfish", "Fish"], ["sym-shark", "Shark", "Fish"], ["sym-skate", "Skate / ray", "Fish"],
  ["sym-crayfish", "Crayfish", "Fish"], ["sym-albacore", "Albacore", "Fish"], ["sym-ling", "Ling", "Fish"],
  ["sym-hoki", "Hoki", "Fish"], ["sym-squid", "Squid", "Fish"],
  ["sym-trawl", "Trawl", "Fishing gear"], ["sym-longline", "Longline", "Fishing gear"], ["sym-hook", "Hook", "Fishing gear"],
  ["sym-float", "Float", "Fishing gear"], ["sym-float-flag", "Float with a flag", "Fishing gear"], ["sym-pot", "Pot / trap", "Fishing gear"],
];
export const SYMBOL_KINDS = Object.fromEntries(MARK_SYMBOLS.map(([k, l]) => [k, l]));
const DEFAULT = { Shapes: "#c0392b", "Seabed and dangers": "#1d2a33", Fish: "#00838f", "Fishing gear": "#d35400" };
const groupOf = Object.fromEntries(MARK_SYMBOLS.map(([k, , g]) => [k, g]));

/** The default colour a symbol is drawn in when its mark has none. */
export const symbolColour = (kind) => DEFAULT[groupOf[kind]] ?? "#1d2a33";

/** The image for "sym:<kind>:<#rrggbb or empty>", or null. */
export function symImage(id) {
  const m = /^sym:(sym-[\w-]+):(#[0-9a-fA-F]{6})?$/.exec(id);
  if (!m || !groupOf[m[1]]) return null;
  return drawSymbol(m[1], m[2] || symbolColour(m[1]));
}

function drawSymbol(kind, colour) {
  // drawn on a ±18 grid at twice the size (the chart shows it at half: about 36 px across)
  const S = 76, c = document.createElement("canvas");
  c.width = S; c.height = S;
  const g = c.getContext("2d");
  g.translate(S / 2, S / 2);
  g.scale(2, 2);
  g.lineCap = "round"; g.lineJoin = "round";
  // each symbol is drawn twice: a wide white halo, then in its colour
  const paths = [];
  const P = (draw, fill = false, width = 3) => paths.push({ draw, fill, width });
  const poly = (pts) => (h) => { h.beginPath(); h.moveTo(...pts[0]); for (const p of pts.slice(1)) h.lineTo(...p); h.closePath(); };
  const line = (pts) => (h) => { h.beginPath(); h.moveTo(...pts[0]); for (const p of pts.slice(1)) h.lineTo(...p); };
  const circ = (x, y, r) => (h) => { h.beginPath(); h.arc(x, y, r, 0, Math.PI * 2); };
  const filled = kind.endsWith("-f");
  const base = kind.replace(/-f$/, "");
  switch (base) {
    case "sym-circle": P(circ(0, 0, 10), filled); break;
    case "sym-square": P(poly([[-9, -9], [9, -9], [9, 9], [-9, 9]]), filled); break;
    case "sym-diamond": P(poly([[0, -12], [11, 0], [0, 12], [-11, 0]]), filled); break;
    case "sym-tri-up": P(poly([[0, -12], [11, 8], [-11, 8]]), filled); break;
    case "sym-tri-down": P(poly([[0, 12], [11, -8], [-11, -8]]), filled); break;
    case "sym-cross": P(line([[-11, 0], [11, 0]]), false, 3.5); P(line([[0, -11], [0, 11]]), false, 3.5); break;
    case "sym-x": P(line([[-9, -9], [9, 9]]), false, 3.5); P(line([[9, -9], [-9, 9]]), false, 3.5); break;
    case "sym-star": {
      const pts = [];
      for (let i = 0; i < 10; i++) { const r = i % 2 ? 5 : 12, a = -Math.PI / 2 + (i * Math.PI) / 5; pts.push([r * Math.cos(a), r * Math.sin(a)]); }
      P(poly(pts), true);
      break;
    }
    case "sym-flag": P(line([[-7, 12], [-7, -12]]), false, 3); P(poly([[-7, -12], [10, -6], [-7, 0]]), true); break;
    case "sym-dot": P(circ(0, 0, 5), true); break;
    case "sym-rock": // the chart's underwater rock: a cross with a dot in each quarter
      P(line([[-10, 0], [10, 0]]), false, 2.5); P(line([[0, -10], [0, 10]]), false, 2.5);
      for (const [x, y] of [[-6, -6], [6, -6], [-6, 6], [6, 6]]) P(circ(x, y, 1.8), true);
      break;
    case "sym-wreck": // the chart's wreck: a hull line with masts
      P(line([[-13, 3], [13, 3]]), false, 3); P(line([[-6, 3], [-6, -7]]), false, 2.5); P(line([[0, 3], [0, -10]]), false, 2.5); P(line([[6, 3], [6, -7]]), false, 2.5);
      break;
    case "sym-snag": // an obstruction: a dotted ring round a hash
      for (let i = 0; i < 12; i++) { const a = (i * Math.PI) / 6; P(circ(12 * Math.cos(a), 12 * Math.sin(a), 1.4), true); }
      P(line([[-3, -6], [-4, 6]]), false, 2); P(line([[3, -6], [2, 6]]), false, 2); P(line([[-6, -2], [6, -2]]), false, 2); P(line([[-6, 3], [6, 3]]), false, 2);
      break;
    case "sym-pot": // a trap with its float
      P(poly([[-10, -2], [10, -2], [10, 12], [-10, 12]]), false, 2.5);
      P(line([[-3, -2], [-3, 12]]), false, 1.5); P(line([[4, -2], [4, 12]]), false, 1.5); P(line([[-10, 5], [10, 5]]), false, 1.5);
      P(line([[0, -2], [0, -8]]), false, 1.8); P(circ(0, -10, 3), true);
      break;
    case "sym-fish":
      P((h) => { h.beginPath(); h.ellipse(-2, 0, 10, 6, 0, 0, Math.PI * 2); }, true);
      P(poly([[7, 0], [14, -7], [14, 7]]), true);
      break;
    case "sym-school":
      for (const [x, y] of [[-6, -6], [6, -1], [-4, 7]]) {
        P((h) => { h.beginPath(); h.ellipse(x - 1, y, 5, 3, 0, 0, Math.PI * 2); }, true);
        P(poly([[x + 3, y], [x + 7, y - 3.5], [x + 7, y + 3.5]]), true);
      }
      break;
    case "sym-tuna": // torpedo body, finlets, crescent tail
      P((h) => { h.beginPath(); h.moveTo(-15, 0); h.bezierCurveTo(-10, -7, 4, -7, 9, -1); h.lineTo(9, 1); h.bezierCurveTo(4, 7, -10, 7, -15, 0); h.closePath(); }, true);
      P((h) => { h.beginPath(); h.moveTo(8, 0); h.lineTo(16, -9); h.quadraticCurveTo(13, 0, 16, 9); h.closePath(); }, true);
      P(poly([[-3, -5], [1, -11], [3, -5]]), true); P(poly([[-1, 5], [2, 9], [3, 5]]), true);
      break;
    case "sym-swordfish": // long bill, tall dorsal fin, crescent tail
      P(line([[-16, -1], [-7, -1]]), false, 1.8);
      P((h) => { h.beginPath(); h.moveTo(-8, 0); h.bezierCurveTo(-4, -5, 6, -5, 10, -1); h.lineTo(10, 1); h.bezierCurveTo(6, 5, -4, 5, -8, 0); h.closePath(); }, true);
      P(poly([[-4, -3], [-1, -12], [3, -4]]), true);
      P((h) => { h.beginPath(); h.moveTo(9, 0); h.lineTo(16, -8); h.quadraticCurveTo(13, 0, 16, 8); h.closePath(); }, true);
      break;
    case "sym-shark": // body, tall dorsal, uneven tail
      P((h) => { h.beginPath(); h.moveTo(-15, 1); h.bezierCurveTo(-10, -6, 4, -6, 9, -1); h.lineTo(9, 2); h.bezierCurveTo(3, 6, -10, 6, -15, 1); h.closePath(); }, true);
      P(poly([[-4, -4], [0, -13], [3, -4]]), true);
      P(poly([[8, 0], [16, -11], [13, 1], [15, 6]]), true);
      P(poly([[-5, 4], [-1, 9], [1, 4]]), true);
      break;
    case "sym-skate": // a diamond with a long thin tail
      P(poly([[0, -12], [12, -1], [0, 5], [-12, -1]]), true);
      P(line([[0, 5], [0, 14]]), false, 1.8);
      break;
    case "sym-crayfish": // body, segmented tail, feelers
      P((h) => { h.beginPath(); h.ellipse(0, -4, 4.5, 7, 0, 0, Math.PI * 2); }, true);
      for (let i = 0; i < 3; i++) P((h) => { h.beginPath(); h.ellipse(0, 4 + i * 3.4, 4 - i * 0.6, 1.8, 0, 0, Math.PI * 2); }, true);
      P(poly([[0, 13], [-5, 16], [5, 16]]), true);
      P(line([[-2, -10], [-9, -16]]), false, 1.4); P(line([[2, -10], [9, -16]]), false, 1.4);
      P(line([[-4, -5], [-10, -4]]), false, 1.6); P(line([[4, -5], [10, -4]]), false, 1.6);
      break;
    case "sym-albacore": // a tuna with long pectoral fins
      P((h) => { h.beginPath(); h.moveTo(-15, 0); h.bezierCurveTo(-10, -7, 4, -7, 9, -1); h.lineTo(9, 1); h.bezierCurveTo(4, 7, -10, 7, -15, 0); h.closePath(); }, true);
      P((h) => { h.beginPath(); h.moveTo(8, 0); h.lineTo(16, -9); h.quadraticCurveTo(13, 0, 16, 9); h.closePath(); }, true);
      P(poly([[-7, 1], [6, 8], [-3, 3]]), true);
      P(poly([[-3, -5], [1, -10], [3, -5]]), true);
      break;
    case "sym-ling": // long eel-like body, one long fin along the back
      P((h) => { h.beginPath(); h.moveTo(-15, 1); h.bezierCurveTo(-13, -4, -6, -4, 0, -3); h.bezierCurveTo(8, -2, 13, -1, 16, 1); h.bezierCurveTo(12, 3, 6, 4, 0, 4); h.bezierCurveTo(-6, 5, -13, 5, -15, 1); h.closePath(); }, true);
      P((h) => { h.beginPath(); h.moveTo(-6, -3.5); h.bezierCurveTo(0, -7, 10, -5, 16, 1); h.lineTo(14, 0); h.bezierCurveTo(8, -3, 0, -3.5, -6, -3.5); h.closePath(); }, true);
      P(line([[-13, 3], [-15, 7]]), false, 1.2);
      break;
    case "sym-hoki": // big head and eye, body tapering to a fine tail
      P((h) => { h.beginPath(); h.moveTo(-14, 0); h.bezierCurveTo(-13, -6, -6, -6, -2, -4); h.lineTo(16, 0); h.lineTo(-2, 4); h.bezierCurveTo(-6, 6, -13, 6, -14, 0); h.closePath(); }, true);
      P(poly([[-3, -4], [0, -9], [2, -3.5]]), true);
      break;
    case "sym-squid": // mantle with fins, arms trailing
      P(poly([[0, -15], [6, -6], [3, 3], [-3, 3], [-6, -6]]), true);
      P(poly([[0, -15], [-6, -8], [6, -8]]), true);
      for (const x of [-3, -1, 1, 3]) P(line([[x, 3], [x * 1.6, 13]]), false, 1.5);
      P(line([[-2, 3], [-6, 15]]), false, 1.2); P(line([[2, 3], [6, 15]]), false, 1.2);
      break;
    case "sym-trawl": // a trawl net: warps from the boat to the doors, the net closing to the cod end
      P(line([[-15, -10], [-4, -6]]), false, 1.4); P(line([[-15, -10], [-4, 6]]), false, 1.4);
      P(poly([[-4, -8], [-2, -8], [-2, -4], [-4, -4]]), true); P(poly([[-4, 4], [-2, 4], [-2, 8], [-4, 8]]), true);
      P(poly([[-2, -6], [12, -2], [12, 2], [-2, 6]]), false, 2);
      P(line([[3, -4.5], [3, 4.5]]), false, 1.2); P(line([[8, -3], [8, 3]]), false, 1.2);
      P(circ(14, 0, 2.5), true);
      break;
    case "sym-longline": // the main line with floats, snoods hanging down to hooks
      P(line([[-15, -4], [15, -4]]), false, 2);
      P(circ(-15, -7, 2.5), true); P(circ(15, -7, 2.5), true);
      for (const x of [-8, 0, 8]) {
        P(line([[x, -4], [x, 6]]), false, 1.4);
        P((h) => { h.beginPath(); h.moveTo(x, 6); h.arc(x - 2, 8, 2, 0, Math.PI * 0.9); }, false, 1.4);
      }
      break;
    case "sym-hook": // a J hook with an eye and a barb
      P(circ(3, -12, 2.2), false, 2);
      P((h) => { h.beginPath(); h.moveTo(3, -9.5); h.lineTo(3, 5); h.arc(-3, 5, 6, 0, Math.PI, false); h.lineTo(-9, -1); }, false, 2.6);
      P(line([[-9, -1], [-6, 2]]), false, 2.2);
      break;
    case "sym-float": // a round fishing float with a rope loop
      P(circ(0, 2, 10), true);
      P(line([[0, -8], [0, -13]]), false, 2); P(circ(0, -14, 2), false, 1.6);
      break;
    case "sym-float-flag": // a float with a flag pole (a dan buoy)
      P(circ(-2, 8, 7), true);
      P(line([[-2, 1], [-2, -15]]), false, 2);
      P(poly([[-2, -15], [12, -11], [-2, -7]]), true);
      break;
    default: return null;
  }
  for (const pass of ["halo", "ink"]) {
    for (const p of paths) {
      p.draw(g);
      if (pass === "halo") { g.strokeStyle = "rgba(255,255,255,0.95)"; g.lineWidth = p.width + 4; g.stroke(); }
      else if (p.fill) { g.fillStyle = colour; g.fill(); }
      else { g.strokeStyle = colour; g.lineWidth = p.width; g.stroke(); }
    }
  }
  return g.getImageData(0, 0, S, S);
}

/**
 * Every mark type a library mark can have, in groups, for the editor, the selection's
 * Type… and the TimeZero symbols list: the <select>'s own options (course marks and
 * buoys) plus points of interest and the symbols here.
 */
export function addKindOptions(select, poiKinds) {
  if (select.querySelector("optgroup.sym")) return;
  if (!select.querySelector("optgroup.poi")) {
    const og = document.createElement("optgroup");
    og.className = "poi";
    og.label = "Point of interest";
    for (const [k, [label]] of Object.entries(poiKinds)) og.append(new Option(label, k));
    select.append(og);
  }
  const groups = new Map();
  for (const [k, label, grp] of MARK_SYMBOLS) {
    if (!groups.has(grp)) { const og = document.createElement("optgroup"); og.className = "sym"; og.label = grp; groups.set(grp, og); select.append(og); }
    groups.get(grp).append(new Option(label, k));
  }
}
