// Which fishing rules apply to a boat: its gear and length (pure functions, used by
// marine.js and its tests). Rule flags are those of chart::marine (src 1 regulation,
// 2 customary, 3 voluntary; prohib; allp; v46; gear: bt, mt, ...).

/** Fishing methods, as MPI's gear flags (short names in the tiles). */
export const GEAR = [
  ["bt", "Bottom trawl"], ["mt", "Midwater trawl"], ["ds", "Danish seine"], ["ps", "Purse seine"],
  ["bs", "Beach seine / drag net"], ["sn", "Set net"], ["ll", "Longline"], ["pt", "Potting"],
  ["dr", "Dredge"], ["tr", "Troll"], ["hl", "Handline"], ["hd", "Hand gathering / diving"],
];
const ALL_GEAR = GEAR.map((g) => g[0]);


/** A rule's flags as an object, from a tile feature's properties or a rules_at hit. */
export function flagsOf(x) {
  if (Array.isArray(x?.flags)) return Object.fromEntries(x.flags);
  return x || {};
}

/** Whether a rule (its flags) is shown with these settings for a boat `length` m long. */
export function applies(f, s, length) {
  const src = f.src || 1;
  if (src === 2 && !s.cust) return false;
  if (src === 3 && !s.vol) return false;
  if (!s.mine || !s.gear.length) return true;
  // trawl rules for vessels over 46 m don't bind a boat known to be shorter
  if (f.v46 && length > 0 && length <= 46) return false;
  if (f.allp || src === 2) return true;
  if (s.gear.some((g) => f[g])) return true;
  // no method named: a species or general rule, shown to be safe
  return !ALL_GEAR.some((g) => f[g]);
}

/** The same test as a MapLibre filter on the tile properties. */
export function ruleFilter(s, length) {
  const c = (k) => ["coalesce", ["get", k], 0];
  const src = ["any", ["==", c("src"), 1], ...(s.cust ? [["==", c("src"), 2]] : []), ...(s.vol ? [["==", c("src"), 3]] : [])];
  if (!s.mine || !s.gear.length) return src;
  return ["all", src,
    length > 0 && length <= 46 ? ["!=", c("v46"), 1] : true,
    ["any", ["==", c("allp"), 1], ["==", c("src"), 2], ...s.gear.map((g) => ["==", c(g), 1]), ["all", ...ALL_GEAR.map((g) => ["==", c(g), 0])]]];
}
