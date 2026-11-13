/** Collision checks use alarm settings and target quality, not chart or list display filters. */
export function collisionAlarmEligible(target, { own, ownMmsi, enabled, lost, internetOnly, internetAlarms, ignoreMoored }) {
  return enabled && !!own?.live && target.lat != null && target.mmsi !== ownMmsi &&
    target.kind === "vessel" && !lost && (!internetOnly || internetAlarms) &&
    !(ignoreMoored && (target.status === 1 || target.status === 5));
}
