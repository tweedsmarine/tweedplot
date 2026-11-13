const FIELDS = ["depth", "depth_keel", "depth_surface"];
export const DEPTH_STALE_MS = 10000;

export function createDepthWatch(now = () => performance.timeOrigin + performance.now()) {
  let started = now(), seen = false, liveTimes = {}, replayTimes = {}, replayAt = null, replayStart = null;
  return {
    reset() {
      started = now();
      seen = false;
      liveTimes = {};
      replayTimes = {};
      replayAt = replayStart = null;
    },
    progress(at, start) {
      if (Number.isFinite(at) && Number.isFinite(start)) {
        replayAt = at;
        replayStart = start;
      }
    },
    observe(v, replay = false) {
      if (replay) {
        replayAt = v.replay_time_ms;
        replayTimes = v.depth_received_at_ms ?? {};
      } else {
        const received = now();
        liveTimes = {};
        for (const field of FIELDS) {
          const age = v.depth_age_ms?.[field];
          if (Number.isFinite(v[field]) && Number.isFinite(age) && age >= 0) {
            liveTimes[field] = received - age;
            seen = true;
          }
        }
      }
    },
    age(field) {
      const time = replayAt == null ? liveTimes[field] : replayTimes[field];
      return Number.isFinite(time) ? Math.max(0, (replayAt ?? now()) - time) : null;
    },
    graceElapsed() {
      return replayAt == null ? seen || now() - started >= DEPTH_STALE_MS
        : replayStart != null && replayAt - replayStart >= DEPTH_STALE_MS;
    },
  };
}

export function depthField(v, forAlarm = false) {
  return forAlarm ? (v.depth_keel != null ? "depth_keel" : "depth")
    : (v.depth != null ? "depth" : "depth_surface");
}

export function depthFresh(v, age, forAlarm = false) {
  const field = depthField(v, forAlarm);
  const value = v[field];
  const elapsed = age(field);
  return Number.isFinite(value) && Number.isFinite(elapsed) && elapsed < DEPTH_STALE_MS;
}

export function depthHistoryV(v, age) {
  return depthFresh(v, age) ? v : { ...v, depth: null, depth_surface: null };
}

export function checkShallowWater({ enabled, plottingLive, depth, fresh, graceElapsed, limit, underKeel }, raise, clear) {
  if (!enabled || !plottingLive || !(limit > 0)) {
    clear("shallow");
    clear("depth-unavailable");
  } else if (!fresh) {
    if (graceElapsed) raise("depth-unavailable", {
      level: "warn", watchdog: true, text: "DEPTH DATA UNAVAILABLE",
      detail: "Shallow-water alarm cannot be checked: check the depth source",
    });
  } else {
    clear("depth-unavailable");
    if (depth < limit) raise("shallow", {
      level: "alarm", watchdog: true, text: `SHALLOW ${depth.toFixed(1)} m`,
      detail: `Below the alarm depth of ${limit} m${underKeel ? " (under the keel)" : " (below the transducer)"}`,
    });
    else if (depth > limit + 0.5) clear("shallow");
  }
}
