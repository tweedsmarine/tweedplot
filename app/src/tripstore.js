// The library's trips, shared by the Trips tab (trips.js) and the Trip filter in the Marks
// and Tracks lists. A trip holds what was made while it lasted: marks made between its
// start and end, tracks that ran in that time (an open trip runs to now).

let trips = [];
const listeners = [];

export const getTrips = () => trips;
export const tripById = (id) => trips.find((t) => String(t.id) === String(id));
export function setTrips(v) { trips = v ?? []; for (const f of listeners) f(trips); }
export const onTrips = (f) => listeners.push(f);

const span = (t) => [t.started ?? 0, t.ended ?? Date.now() / 1000];
export function markInTrip(m, t) {
  if (!t || m.date_unknown || !m.created) return false;
  const [a, b] = span(t);
  return m.created >= a && m.created <= b;
}
export function trackInTrip(tr, t) {
  const s = tr.stats?.start;
  if (!t || !s) return false;
  const [a, b] = span(t);
  return s <= b && (tr.stats?.end ?? s) >= a;
}

/** A Trip filter for a list (marks or tracks): one choice per trip, with how many each has. */
export function tripFilter(inTrip) {
  return {
    key: "trip", label: "Trip", type: "choice", title: "Only what was made during one trip",
    options: (items) => trips.map((t) => [t.id, `${t.name}${t.fishery ? ` · ${t.fishery}` : ""}`, items.reduce((n, x) => n + (inTrip(x, t) ? 1 : 0), 0)]),
    test: (x, v) => inTrip(x, tripById(v)),
  };
}

/** "2 d 4 h", "5 h 10 min". */
export function duration(s) {
  if (!(s > 0)) return "";
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  return d ? `${d} d ${h} h` : h ? `${h} h ${m} min` : `${m} min`;
}
