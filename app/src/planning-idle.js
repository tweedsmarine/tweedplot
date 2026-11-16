// Planning and Environment return to Navigation after a time without input: a safety feature, so the
// screen left on a planning chart comes back to the vessel. It only applies while `active()` says a
// live position is arriving (at the desk there is nothing to return for), and `warnMs` before the
// switch `onWarn(secondsLeft)` is called each second so a notice can be shown; any input cancels it.
export function createPlanningIdle({ now, schedule, cancel, currentView, timeoutMinutes, onTimeout,
  active = () => true, warnMs = 0, onWarn = () => {}, onWarnEnd = () => {}, recheckMs = 15000 }) {
  let timer = null;
  let lastInput = now();
  let warnedAt = null;
  function stop() {
    if (timer !== null) cancel(timer);
    timer = null;
  }
  function endWarning() {
    if (warnedAt === null) return;
    warnedAt = null;
    onWarnEnd();
  }
  function check() {
    stop();
    const minutes = timeoutMinutes();
    if (!["plan", "env"].includes(currentView()) || !minutes) { endWarning(); return; }
    // no live position: stay, and look again (the position may start arriving)
    if (!active()) { endWarning(); timer = schedule(check, recheckMs); return; }
    const total = minutes * 60000, idle = now() - lastInput;
    const notice = Math.min(warnMs, total);
    if (idle < total - notice) { endWarning(); timer = schedule(check, total - notice - idle); return; }
    // the notice always runs its full length, even when the position has only just started arriving
    if (warnedAt === null) warnedAt = now();
    const left = warnedAt + notice - now();
    if (left <= 0) { endWarning(); onTimeout(); return; }
    onWarn(Math.ceil(left / 1000));
    timer = schedule(check, Math.min(1000, left));
  }
  function activity() {
    lastInput = now();
    check();
  }
  return { activity, check, stop };
}
