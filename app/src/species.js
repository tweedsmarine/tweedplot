// Species presets: the sea temperature band each species is usually found in, highlighted on the
// sea temperature layer with the target marked, isotherms and temperature breaks drawn. A starting
// point, not a rule: fish follow food, fronts and currents as much as temperature, and local
// knowledge beats any band. Sources are given in the manual (Weather and tides › Species presets).
//
// The built-in figures below can be changed by the user (Species… in the presets row): a change
// to a built-in species is kept as its own figures over the default, so a species not edited keeps
// following the defaults as they are updated; species the user adds are kept in full.
//   [name, layer, depth (m, for a layer at depth), band low, band high, target, note]
export const SPECIES = [
  ["Albacore", "sst", null, 15, 20, 17, "Juvenile albacore, the fish NZ trollers catch, in about 15–21 °C surface water, often along fronts; poor NZ seasons follow cold summers"],
  ["Southern bluefin", "sst", null, 14, 18, 16, "Large fish tagged in NZ waters in winter were in 13.6–18.3 °C surface water"],
  ["Yellowfin", "sst", null, 18, 25, 20, "Arrive in NZ as offshore water reaches about 18–20 °C (NZ anglers' and commercial fishers' reports); preferred about 20–25 °C"],
  ["Bigeye (at 250 m)", "subt", 250, 10, 14, 12, "By day at 200–400 m in 8–14 °C water; longline catches best at 12–14 °C around 250 m. Needs a file with temperature at depth (Copernicus or HYCOM)"],
  ["Skipjack", "sst", null, 18, 26, 20, "In NZ they need water of at least about 18–19 °C; the main fisheries elsewhere are in 19–23 °C"],
  ["Striped marlin", "sst", null, 18, 24, 21, "Marlin tagged off northern NZ spent about three quarters of their time in 20–24 °C water; caught down to about 18.5 °C"],
  ["Mahi-mahi", "sst", null, 21, 28, 23, "NZ catches come in about 21–23 °C water, when the north is warm; overseas, catch rates are highest at 24–29 °C"],
];

const OVERLAYS = { arrows: false, waveArrows: false, currArrows: true, isobars: false, isotherms: true, breaks: true };

/** One preset as a row: { name, layer, depth, blo, bhi, tg, note }. */
const fromTuple = ([name, layer, depth, blo, bhi, tg, note]) => ({ name, layer, depth, blo, bhi, tg, note });

/** The species list the user sees: the built-in ones, with the user's figures where changed
 *  (edited: true, def: the built-in row), then the user's own (own: true).
 *  edits: { name: [low, high, target] }; own: [{ name, layer, depth, blo, bhi, tg, note }]. */
export function speciesList(edits = {}, own = []) {
  const out = SPECIES.map((t) => {
    const def = fromTuple(t);
    const e = edits?.[def.name];
    if (!Array.isArray(e) || e.length !== 3 || !e.every(Number.isFinite)) return { ...def };
    const [blo, bhi, tg] = e;
    return { ...def, blo, bhi, tg, edited: true, def };
  });
  for (const o of Array.isArray(own) ? own : []) {
    if (!o || typeof o.name !== "string" || checkSpecies(o)) continue;
    if (out.some((x) => x.name === o.name)) continue;
    out.push({ ...o, depth: o.layer === "subt" ? o.depth : null, own: true });
  }
  return out;
}

/** "" when a row is usable, else what is wrong with it. */
export function checkSpecies(r) {
  const name = String(r.name ?? "").trim();
  if (!name) return "Every species needs a name.";
  const n = (v) => typeof v === "number" && Number.isFinite(v);
  if (![r.blo, r.bhi, r.tg].every(n)) return `${name}: type a number in each of From, Target and To.`;
  if (r.blo < -2 || r.bhi > 36) return `${name}: temperatures from −2 to 36 °C.`;
  if (!(r.blo < r.tg && r.tg < r.bhi)) return `${name}: the target must lie between From and To.`;
  if (r.layer === "subt" && !(n(r.depth) && r.depth >= 1 && r.depth <= 1000)) return `${name}: a depth from 1 to 1000 m.`;
  return "";
}

/** The weather settings a species preset applies (layer, overlays, filter with the band and target). */
export function speciesConfig(sp) {
  const r = Array.isArray(sp) ? fromTuple(sp) : sp;
  return {
    layer: r.layer,
    ...(r.layer === "subt" && r.depth != null ? { depth: r.depth } : {}),
    overlays: OVERLAYS,
    filter: { lo: Math.floor(r.blo) - 4, hi: Math.ceil(r.bhi) + 4, tg: r.tg, blo: r.blo, bhi: r.bhi, set: true },
  };
}

/** The hover note for a preset: its note, and for a changed built-in one what the default was. */
export function speciesNote(r) {
  if (r.edited) return `Your figures. Default ${r.def.blo}–${r.def.bhi} °C, target ${r.def.tg} °C: ${r.def.note}`;
  return r.note || (r.own ? "Your own species" : "");
}

const num = (s) => {
  const t = String(s ?? "").trim().replace(",", ".").replace("−", "-");
  return t === "" ? NaN : Number(t);
};

/** The species editor: every species in a table (band from, target, band to; the layer and depth
 *  for your own), Add species, Reset for a changed built-in one, Delete for your own, Reset all.
 *  Resolves { edits, own } on Save, null on Cancel. */
export function editSpecies(edits = {}, own = []) {
  return new Promise((resolve) => {
    let rows = speciesList(edits, own).map((r) => ({ ...r }));
    const back = document.createElement("div");
    back.className = "ask-back";
    const box = document.createElement("form");
    box.className = "ask sp-edit";
    box.innerHTML = `<b class="ask-title">Species presets</b>
      <div class="wx-note ask-note">Sea temperature in °C: the band a species is usually found in (from, to) and the target inside it. Built-in species you change keep your figures; Reset puts back the default.</div>
      <div class="sp-scroll"><table class="sp-table"><thead><tr><th>Species</th><th>Layer</th><th>From</th><th>Target</th><th>To</th><th></th></tr></thead><tbody></tbody></table></div>
      <div class="ask-err" hidden></div>
      <div class="wx-row"><button type="button" class="sp-add">Add species</button><button type="button" class="sp-reset-all">Reset all</button><span class="grow"></span><button type="submit" class="on">Save</button><button type="button" class="ask-cancel">Cancel</button></div>`;
    const tbody = box.querySelector("tbody");
    const err = box.querySelector(".ask-err");
    const showErr = (m) => { err.hidden = !m; err.textContent = m || ""; };

    const cell = (r, key, w = "4.5em") => {
      const i = document.createElement("input");
      i.type = "text";
      i.inputMode = "decimal";
      i.value = Number.isFinite(r[key]) ? String(r[key]) : "";
      i.style.width = w;
      i.setAttribute("aria-label", `${r.name || "New species"}: ${{ blo: "band from", tg: "target", bhi: "band to", depth: "depth" }[key]}`);
      i.oninput = () => { r[key] = num(i.value); showErr(""); };
      return i;
    };
    function render() {
      tbody.innerHTML = "";
      for (const r of rows) {
        const tr = document.createElement("tr");
        if (r.edited) tr.className = "sp-edited";
        const name = document.createElement("td");
        if (r.own) {
          const i = document.createElement("input");
          i.type = "text";
          i.value = r.name;
          i.placeholder = "Name";
          i.style.width = "9em";
          i.oninput = () => { r.name = i.value; showErr(""); };
          name.append(i);
        } else {
          name.textContent = r.name;
          name.title = speciesNote(r);
        }
        const layer = document.createElement("td");
        if (r.own) {
          const s = document.createElement("select");
          s.append(new Option("Surface", "sst"), new Option("At depth", "subt"));
          s.value = r.layer;
          const d = cell(r, "depth", "3.5em");
          d.hidden = r.layer !== "subt";
          const m = document.createElement("span");
          m.textContent = " m";
          m.hidden = d.hidden;
          s.onchange = () => { r.layer = s.value; d.hidden = m.hidden = r.layer !== "subt"; if (r.layer === "subt" && !Number.isFinite(r.depth)) { r.depth = 100; d.value = "100"; } };
          layer.append(s, d, m);
        } else {
          layer.textContent = r.layer === "subt" ? `${r.depth} m` : "Surface";
        }
        const td = (el) => { const c = document.createElement("td"); c.append(el); return c; };
        const act = document.createElement("td");
        if (r.edited) {
          const b = document.createElement("button");
          b.type = "button";
          b.textContent = "Reset";
          b.title = `Back to the default, ${r.def.blo}–${r.def.bhi} °C, target ${r.def.tg} °C`;
          b.onclick = () => { Object.assign(r, { blo: r.def.blo, bhi: r.def.bhi, tg: r.def.tg, edited: false }); render(); };
          act.append(b);
        } else if (r.own) {
          const b = document.createElement("button");
          b.type = "button";
          b.textContent = "Delete";
          b.onclick = () => { rows = rows.filter((x) => x !== r); render(); };
          act.append(b);
        }
        tr.append(name, layer, td(cell(r, "blo")), td(cell(r, "tg")), td(cell(r, "bhi")), act);
        tbody.append(tr);
      }
    }
    render();

    box.querySelector(".sp-add").onclick = () => {
      rows.push({ name: "", layer: "sst", depth: null, blo: NaN, bhi: NaN, tg: NaN, note: "", own: true });
      render();
      tbody.lastElementChild?.querySelector("input")?.focus();
    };
    box.querySelector(".sp-reset-all").onclick = () => {
      rows = [...SPECIES.map(fromTuple), ...rows.filter((r) => r.own)];
      render();
    };

    const done = (v) => { back.remove(); document.removeEventListener("keydown", esc, true); resolve(v); };
    const esc = (e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); done(null); } };
    document.addEventListener("keydown", esc, true);
    box.querySelector(".ask-cancel").onclick = () => done(null);
    box.onsubmit = (e) => {
      e.preventDefault();
      const out = { edits: {}, own: [] };
      const names = new Set();
      for (const r of rows) {
        r.name = String(r.name ?? "").trim();
        const m = checkSpecies(r);
        if (m) { showErr(m); return; }
        if (names.has(r.name.toLowerCase())) { showErr(`There are two species called ${r.name}.`); return; }
        names.add(r.name.toLowerCase());
        if (r.own) {
          out.own.push({ name: r.name, layer: r.layer, depth: r.layer === "subt" ? r.depth : null, blo: r.blo, bhi: r.bhi, tg: r.tg, note: r.note ?? "" });
        } else {
          const def = fromTuple(SPECIES.find((t) => t[0] === r.name));
          if (r.blo !== def.blo || r.bhi !== def.bhi || r.tg !== def.tg) out.edits[r.name] = [r.blo, r.bhi, r.tg];
        }
      }
      done(out);
    };
    back.append(box);
    document.body.append(back);
  });
}

