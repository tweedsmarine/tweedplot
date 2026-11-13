export function createPositionWatch(now = () => performance.timeOrigin + performance.now()) {
  let startedAt = now();
  let lastFixAt = null;
  let lastObservation = null;
  let replayTime = null;
  let replayStart = null;
  let replayFix = null;
  const replayAge = () => replayTime == null || replayFix == null ? null : Math.max(0, replayTime - replayFix);
  return {
    reset() {
      startedAt = now();
      lastFixAt = null;
      lastObservation = null;
      replayTime = replayStart = replayFix = null;
    },
    replayProgress(at, start) {
      if (Number.isFinite(at) && Number.isFinite(start)) {
        replayTime = at;
        replayStart = start;
      }
    },
    observe(vessel, replay = false) {
      const { lat, lon, position_observation: observation, position_age_ms: age } = vessel;
      if (replay) {
        if (Number.isFinite(vessel.replay_time_ms)) replayTime = vessel.replay_time_ms;
        replayFix = Number.isFinite(lat) && lat >= -90 && lat <= 90 && Number.isFinite(lon) && lon >= -180 && lon <= 180 &&
          Number.isFinite(vessel.position_received_at_ms) ? vessel.position_received_at_ms : null;
        return;
      }
      if (!Number.isFinite(lat) || lat < -90 || lat > 90 || !Number.isFinite(lon) || lon < -180 || lon > 180 ||
          !Number.isSafeInteger(observation) || observation <= 0 || observation === lastObservation ||
          !Number.isFinite(age) || age < 0) return;
      lastObservation = observation;
      lastFixAt = now() - age;
    },
    age: () => replayTime == null ? lastFixAt == null ? null : Math.max(0, now() - lastFixAt) : replayAge(),
    lost: () => replayTime == null
      ? now() - (lastFixAt ?? startedAt) >= 30000
      : replayTime - (replayFix ?? replayStart ?? replayTime) >= 30000,
  };
}
