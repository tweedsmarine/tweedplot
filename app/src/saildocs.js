// Saildocs GRIB requests: an email to query@saildocs.com such as
//   send GFS:48S,33S,165E,175W|0.25,0.25|0,3..120|WIND,PRMSL
// Saildocs replies with the GRIB attached (GRIB1); save it in the GRIB folder.
// Free; sizes grow with area x resolution x times x parameters, so keep them
// small over satellite links.

export const SAILDOCS = "query@saildocs.com";

/** What Saildocs offers per model (free models only). */
export const SD_MODELS = {
  GFS: { name: "GFS (NOAA)", res: [0.25, 0.5, 1, 2], max: 384, params: ["WIND", "PRMSL", "GUST", "RAIN", "WAVES"], def: ["WIND", "PRMSL"] },
  ICON: { name: "ICON (DWD)", res: [0.125, 0.25, 0.5, 1], max: 180, params: ["WIND", "PRMSL", "GUST", "RAIN"], def: ["WIND", "PRMSL"] },
  ECMWF: { name: "ECMWF (free part)", res: [0.4, 0.8, 1.2], max: 240, params: ["WIND", "PRMSL"], def: ["WIND", "PRMSL"] },
};
export const SD_PARAM_NAMES = { WIND: "Wind", PRMSL: "Pressure", GUST: "Gusts", RAIN: "Rain", WAVES: "Waves" };

const lat = (v) => `${+Math.abs(v).toFixed(2)}${v < 0 ? "S" : "N"}`;
const lon = (v) => { const w = ((v + 540) % 360) - 180; return `${+Math.abs(w).toFixed(2)}${w < 0 ? "W" : "E"}`; };

/**
 * The request line. `area` = [west, south, east, north] (east may be beyond 180);
 * edges are moved out to whole degrees.
 */
export function saildocsRequest({ model, area, res, hours, every, params }) {
  const m = SD_MODELS[model];
  const out = (v, dir) => (dir < 0 ? Math.floor(v) : Math.ceil(v));
  const [w, s, e, n] = area;
  const h = Math.min(hours, m.max);
  const last = h - (h % every);
  const times = last < every ? "0" : last === every ? `0,${every}` : `0,${every}..${last}`;
  return `send ${model}:${lat(out(n, 1))},${lat(out(s, -1))},${lon(out(w, -1))},${lon(out(e, 1))}|${res},${res}|${times}|${params.join(",")}`;
}

/** Rough size of the reply's GRIB, bytes (about 1.5 bytes a point plus headers). */
export function saildocsBytes({ area, res, hours, every, params, model }) {
  const [w, s, e, n] = area;
  const width = ((e - w) % 360 + 360) % 360 || 360;
  const pts = (Math.floor(width / res) + 2) * (Math.floor((n - s) / res) + 2);
  const fields = params.reduce((a, p) => a + (p === "WIND" ? 2 : 1), 0);
  const steps = Math.floor(Math.min(hours, SD_MODELS[model].max) / every) + 1;
  return Math.round((pts * 1.5 + 80) * fields * steps);
}
