// Chart › Layers: show or hide the world depth layers, the world coastline and the charts'
// outlines (off until ticked).
// (High-resolution depths has its own checkbox above.) The choice is remembered. The map
// re-enables chart layers when the chart store opens, so apply() runs again after that.

const KEY = "tweedplot.layervis";
const NE = ["<=", ["get", "src"], 1]; // Natural Earth bands
const ETOPO = [">=", ["get", "src"], 2];

function load() {
  try { return JSON.parse(localStorage.getItem(KEY) || "{}"); } catch { return {}; }
}

export function setupLayerVisibility(map, { ready }) {
  const hidden = load();
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(hidden)); } catch { /* ignore */ } };
  const vis = (id, on) => { if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", on ? "visible" : "none"); };

  function apply() {
    if (!ready()) return;
    const ne = !hidden["lv-wdepth"], etopo = !hidden["lv-etopo"], coast = !hidden["lv-wcoast"];
    if (map.getLayer("wdepth")) {
      vis("wdepth", ne || etopo);
      map.setFilter("wdepth", ne && etopo ? null : ne ? NE : ETOPO);
    }
    vis("wdcnt", etopo);
    vis("chart-world-land", coast);
    vis("chart-wcoast", coast);
    const outlines = hidden["lv-outlines"] === false;
    vis("chart-outlines", outlines);
    const key = document.getElementById("cov-key");
    if (key) key.hidden = !outlines;
  }

  for (const id of ["lv-wdepth", "lv-etopo", "lv-wcoast", "lv-outlines"]) {
    const box = document.getElementById(id);
    if (!box) continue;
    // (the outlines are off until ticked: shown only when stored as not hidden)
    box.checked = id === "lv-outlines" ? hidden[id] === false : !hidden[id];
    box.onchange = () => { hidden[id] = !box.checked; save(); apply(); };
  }
  apply();
  return { apply };
}
