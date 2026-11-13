export function createPlanningIdle({ now, schedule, cancel, currentView, timeoutMinutes, onTimeout }) {
  let timer = null;
  let lastInput = now();
  function stop() {
    if (timer !== null) cancel(timer);
    timer = null;
  }
  function check() {
    stop();
    const minutes = timeoutMinutes();
    if (currentView() !== "plan" || !minutes) return;
    const remaining = minutes * 60000 - (now() - lastInput);
    if (remaining <= 0) onTimeout();
    else timer = schedule(check, remaining);
  }
  function activity() {
    lastInput = now();
    check();
  }
  return { activity, check, stop };
}
