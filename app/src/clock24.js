// Clock times everywhere in 24-hour form (14:05, not 2:05 pm), whatever the computer's
// locale says: Date's toLocaleString / toLocaleTimeString and Intl.DateTimeFormat are given
// hourCycle "h23" (and hour12 is dropped, as it would override it). Imported first by main.js.
const h23 = (o) => {
  const x = { ...(o ?? {}) };
  delete x.hour12;
  x.hourCycle = "h23";
  return x;
};
const P = Date.prototype;
const ls = P.toLocaleString, lt = P.toLocaleTimeString;
P.toLocaleString = function (loc, o) { return ls.call(this, loc, h23(o)); };
P.toLocaleTimeString = function (loc, o) { return lt.call(this, loc, h23(o)); };
const DTF = Intl.DateTimeFormat;
function DateTimeFormat24(loc, o) { return new DTF(loc, o && ("hour" in o || "timeStyle" in o) ? h23(o) : o); }
DateTimeFormat24.prototype = DTF.prototype;
DateTimeFormat24.supportedLocalesOf = DTF.supportedLocalesOf;
Intl.DateTimeFormat = DateTimeFormat24;
