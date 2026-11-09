// Points of interest for the mark library: round badges with a simple
// pictogram, drawn on a canvas on demand ("poi:<kind>"), so no image files.

/** kind → [label, ring colour] (the order is the menu order). */
export const POI_KINDS = {
  anchor: ["Anchor (where I anchored)", "#c45500"],
  anchorage: ["Good anchorage", "#1f4e79"],
  mooring: ["Mooring", "#1f4e79"],
  shelter: ["Shelter", "#1f4e79"],
  fishing: ["Fishing spot", "#00838f"],
  diving: ["Diving spot", "#00838f"],
  foul: ["Foul ground", "#a0157a"],
  wreck: ["Wreck", "#a0157a"],
  hazard: ["Hazard", "#c62828"],
  landing: ["Landing point", "#2e7d32"],
  camping: ["Camping spot", "#2e7d32"],
  hut: ["Hut / building", "#2e7d32"],
  viewpoint: ["Viewpoint", "#2e7d32"],
  water: ["Water filling", "#1565c0"],
  fuel: ["Fuel", "#5d4037"],
  provisions: ["Provisions / shop", "#5d4037"],
  info: ["Note", "#455a64"],
};

/** The image for a "poi:<kind>" id, or null. */
export function poiImage(id) {
  const m = /^poi:(\w+)$/.exec(id);
  if (!m || !POI_KINDS[m[1]]) return null;
  return drawPoi(m[1]);
}

function drawPoi(kind) {
  const S = 48, c = document.createElement("canvas");
  c.width = S; c.height = S;
  const g = c.getContext("2d");
  const col = POI_KINDS[kind][1];
  // badge
  g.beginPath(); g.arc(S / 2, S / 2, S / 2 - 3, 0, Math.PI * 2);
  g.fillStyle = "#ffffff"; g.fill();
  g.lineWidth = 3.5; g.strokeStyle = col; g.stroke();
  g.save();
  g.translate(S / 2, S / 2);
  g.strokeStyle = col; g.fillStyle = col; g.lineWidth = 2.6; g.lineCap = "round"; g.lineJoin = "round";
  const line = (pts) => { g.beginPath(); g.moveTo(...pts[0]); for (const p of pts.slice(1)) g.lineTo(...p); g.stroke(); };
  const poly = (pts) => { g.beginPath(); g.moveTo(...pts[0]); for (const p of pts.slice(1)) g.lineTo(...p); g.closePath(); g.fill(); };
  const circle = (x, y, r, fill) => { g.beginPath(); g.arc(x, y, r, 0, Math.PI * 2); fill ? g.fill() : g.stroke(); };
  switch (kind) {
    case "anchor": // the anchor dropped (anchor.js): the same pictogram in the mariner's orange
    case "anchorage":
      circle(0, -11, 3);
      line([[0, -8], [0, 12]]); line([[-6, -4], [6, -4]]);
      g.beginPath(); g.arc(0, 2, 11, 0.15 * Math.PI, 0.85 * Math.PI); g.stroke();
      break;
    case "mooring":
      circle(0, 3, 8, true); line([[0, -5], [0, -12]]); circle(0, -13, 2.5);
      break;
    case "shelter": // a cove: bay outline with a calm line inside
      g.beginPath(); g.arc(0, 2, 12, Math.PI * 0.05, Math.PI * 0.95, true); g.stroke();
      line([[-6, 6], [6, 6]]);
      break;
    case "fishing":
      g.beginPath(); g.ellipse(-2, 0, 10, 6, 0, 0, Math.PI * 2); g.fill();
      poly([[7, 0], [14, -6], [14, 6]]);
      g.fillStyle = "#fff"; circle(-7, -1, 1.6, true);
      break;
    case "diving": // dive flag: red with a white diagonal
      g.fillStyle = "#d32f2f"; g.fillRect(-12, -9, 24, 18);
      g.strokeStyle = "#fff"; g.lineWidth = 4; line([[-12, -9], [12, 9]]);
      break;
    case "foul": // chart "foul ground": a hash
      line([[-4, -11], [-6, 11]]); line([[5, -11], [3, 11]]); line([[-11, -4], [11, -4]]); line([[-12, 4], [10, 4]]);
      break;
    case "wreck": // chart wreck: hull with masts
      line([[-13, 4], [13, 4]]); line([[-6, 4], [-6, -8]]); line([[0, 4], [0, -11]]); line([[6, 4], [6, -8]]);
      break;
    case "hazard":
      g.lineWidth = 2.4; g.beginPath(); g.moveTo(0, -12); g.lineTo(12, 10); g.lineTo(-12, 10); g.closePath(); g.stroke();
      line([[0, -4], [0, 3]]); circle(0, 7, 1.4, true);
      break;
    case "landing": // arrow onto a beach
      line([[0, -12], [0, 4]]); line([[-6, -2], [0, 4], [6, -2]]); line([[-12, 10], [12, 10]]);
      break;
    case "camping":
      poly([[0, -12], [12, 10], [-12, 10]]);
      g.fillStyle = "#fff"; poly([[0, 0], [5, 10], [-5, 10]]);
      break;
    case "hut":
      poly([[0, -12], [12, -1], [-12, -1]]); g.fillRect(-8, -1, 16, 12);
      g.fillStyle = "#fff"; g.fillRect(-2.5, 4, 5, 7);
      break;
    case "viewpoint": // binoculars
      circle(-6, 3, 5.5); circle(6, 3, 5.5); line([[-1, 1], [1, 1]]); line([[-8, -3], [-6, -10]]); line([[8, -3], [6, -10]]);
      break;
    case "water": // drop
      g.beginPath(); g.moveTo(0, -13); g.bezierCurveTo(9, -2, 10, 3, 9, 6); g.arc(0, 5, 9, 0.05, Math.PI - 0.05); g.bezierCurveTo(-10, 3, -9, -2, 0, -13); g.fill();
      break;
    case "fuel": // pump
      g.fillRect(-10, -11, 13, 22); g.fillStyle = "#fff"; g.fillRect(-7, -8, 7, 6);
      line([[3, -6], [8, -2], [8, 8], [11, 8], [11, -4]]);
      break;
    case "provisions": // basket
      poly([[-12, -2], [12, -2], [8, 11], [-8, 11]]);
      g.beginPath(); g.arc(0, -2, 8, Math.PI, 0); g.stroke();
      break;
    case "info":
      circle(0, -9, 2.4, true); g.lineWidth = 4; line([[0, -3], [0, 11]]);
      break;
  }
  g.restore();
  return g.getImageData(0, 0, S, S);
}
