const NM = 1852;

export function courseTrack(course, departure, speed) {
  const seq = [course.start ? { lon: course.start[0] ?? course.start.lon, lat: course.start[1] ?? course.start.lat } : null, ...course.marks].filter(Boolean);
  if (seq.length < 2) return null;
  if (!Number.isFinite(departure) || departure <= 0 || !Number.isFinite(speed) || speed < 0.5 || speed > 40 ||
      seq.some((p) => !Number.isFinite(p.lon) || !Number.isFinite(p.lat))) throw new Error("The route, departure or speed is invalid.");
  let t = departure;
  const pts = [{ t, lon: seq[0].lon, lat: seq[0].lat }];
  for (let i = 1; i < seq.length; i++) {
    t += distance(seq[i - 1], seq[i]) / NM / speed * 3600;
    pts.push({ t: Math.round(t), lon: seq[i].lon, lat: seq[i].lat });
  }
  if (pts.at(-1).t <= pts[0].t) throw new Error("The route has no length.");
  return pts;
}

export function sampleTrack(pts, positionAt, routing = false) {
  if (pts.length < 2 || !Number.isFinite(pts[0].t) || !Number.isFinite(pts.at(-1).t) || pts.at(-1).t <= pts[0].t ||
      pts.some((p, i) => !Number.isFinite(p.t) || !Number.isFinite(p.lon) || !Number.isFinite(p.lat) || (i > 0 && p.t < pts[i - 1].t))) {
    throw new Error("The route has invalid positions or times.");
  }
  const t0 = pts[0].t, t1 = pts.at(-1).t;
  const step = Math.max(600, Math.ceil((t1 - t0) / 240 / 300) * 300);
  const rows = [];
  let nm = 0, prev = null;
  for (let t = t0; ; t = Math.min(t + step, t1)) {
    const p = positionAt(pts, t);
    if (!p || !Number.isFinite(p.lon) || !Number.isFinite(p.lat)) throw new Error("Could not locate a point along the route.");
    if (prev) nm += distance(prev, p) / NM;
    rows.push({ t, lon: p.lon, lat: p.lat, nm, twa: routing ? p.twa : null });
    prev = p;
    if (t >= t1) break;
  }
  return rows;
}

export function coverage(rows, fields) {
  return fields.map(({ key, label }) => {
    const count = rows.filter((r) => r[key] != null).length;
    return { key, label, count, total: rows.length };
  });
}

/** Where each field's values are missing along the route: before its first value (lead), after its last (trail: the
 *  forecast has ended) or between values (inner: a real gap, such as land for waves). A route that outlasts the
 *  forecast is not a gap, so only lead, inner and ending-earlier-than-the-others are worth a warning. */
export function gaps(rows, fields) {
  const lastAny = Math.max(-1, ...fields.map(({ key }) => rows.findLastIndex((r) => r[key] != null)));
  return fields.map(({ key, label }) => {
    const idx = rows.map((r, i) => (r[key] != null ? i : -1)).filter((i) => i >= 0);
    const total = rows.length, count = idx.length;
    const lead = count ? idx[0] : total, last = count ? idx.at(-1) : -1;
    const trail = count ? total - 1 - last : 0;
    return { key, label, total, count, lead: count ? lead : 0, trail, inner: count ? last - idx[0] + 1 - count : 0, earlier: count > 0 && last < lastAny ? lastAny - last : 0 };
  });
}

export function createRouteWeatherRequest(notify) {
  let generation = 0;
  let state = { status: "empty", data: null, error: null };
  const publish = (next) => { state = next; notify(state); };
  return {
    state: () => state,
    invalidate(status = "stale", error = null) {
      generation++;
      publish({ status, data: null, error });
    },
    async run(work) {
      const current = ++generation;
      publish({ status: "loading", data: null, error: null });
      try {
        const data = await work();
        if (current === generation) publish({ status: "ready", data, error: null });
      } catch (error) {
        if (current === generation) publish({ status: "error", data: null, error: String(error) });
      }
    },
  };
}

function distance(a, b) {
  const R = 6371008.8, r = Math.PI / 180;
  const dLat = (b.lat - a.lat) * r, dLon = (b.lon - a.lon) * r;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a.lat * r) * Math.cos(b.lat * r) * Math.sin(dLon / 2) ** 2;
  return 2 * R * Math.asin(Math.min(1, Math.sqrt(h)));
}
