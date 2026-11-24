// Alerts in the OpenBridge top bar (IEC 62923 bridge alert management, as OpenBridge draws it):
// the most important unacknowledged alert as a message beside the alert button, the alert button
// (count, highest level, flashing while any is unacknowledged), and the alert menu under it with
// every active alert, its acknowledge and dismiss actions, and a way to the Alarms drawer.
// alarms.js keeps deciding what is raised, sounded, acknowledged and cleared; it sends the list
// here ("racenav-alerts") each time it changes.

import "@oicl/openbridge-webcomponents/dist/components/alert-button/alert-button.js";
import "@oicl/openbridge-webcomponents/dist/components/alert-menu/alert-menu.js";
import "@oicl/openbridge-webcomponents/dist/components/alert-menu-item/alert-menu-item.js";
import "@oicl/openbridge-webcomponents/dist/components/alert-icon/alert-icon.js";
import "@oicl/openbridge-webcomponents/dist/components/topbar-message-item/topbar-message-item.js";

const TYPE = { alarm: "alarm", warn: "warning", caution: "caution" };
const RANK = { alarm: 0, warn: 1, caution: 2 };
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const hhmm = (t) => new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const icon = (a, slot) =>
  `<obc-alert-icon slot="${slot}" alerttype="${TYPE[a.level] ?? "warning"}" ${a.acked ? "acknowledged" : ""} active></obc-alert-icon>`;

/** `slot`: the top bar's alerts area, where the message and the button go (before MOB, Drop and the anchor). */
export function setupOpenBridgeAlerts(slot) {
  const msg = document.createElement("obc-topbar-message-item");
  msg.id = "ob-alert-msg";
  msg.hidden = true;
  const btn = document.createElement("obc-alert-button");
  btn.id = "ob-alert-btn";
  btn.showSilenceButton = false;
  btn.counter = true;
  btn.flatMaxBreakpointPx = 0; // the full button (count, level), never the flat one
  slot.prepend(msg, btn);

  const menu = document.createElement("obc-alert-menu");
  menu.id = "ob-alert-menu";
  menu.softDismiss = true;
  menu.showAlertListButton = true;
  menu.showSilenceButton = false;
  menu.hidden = true;
  document.body.append(menu);

  let state = { alerts: [], ackAll: () => 0 };
  const sorted = () => [...state.alerts].sort((a, b) =>
    a.acked !== b.acked ? (a.acked ? 1 : -1) : RANK[a.level] !== RANK[b.level] ? RANK[a.level] - RANK[b.level] : b.since - a.since);

  function setMenu(open) {
    menu.hidden = !open;
    menu.open = open;
    if (open) renderMenu();
  }
  btn.addEventListener("click-alert", () => setMenu(menu.hidden));
  menu.addEventListener("close", () => setMenu(false));
  menu.addEventListener("ack-all-visible-click", () => state.ackAll());
  menu.addEventListener("go-to-alert-list-click", () => {
    setMenu(false);
    const b = document.querySelector('#rail button[data-pane="alarms"]');
    if (b && !b.classList.contains("on")) b.click();
  });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape" && !menu.hidden) setMenu(false); });

  function renderMenu() {
    const list = sorted();
    menu.canAckAll = list.some((a) => !a.acked);
    menu.innerHTML = list.map((a, i) => `<obc-alert-menu-item data-i="${i}"
        status="${a.acked ? "acknowledged" : "unacknowledged"}" time="${hhmm(a.since)}" size="${a.detail ? "double-line" : "single-line"}"
        primaryactionstate="${a.acked ? "none" : "enabled"}" secondaryactionstate="${a.dismiss ? "enabled" : "none"}"
        secondaryactionlabel="${esc(a.dismissLabel)}">
        ${icon(a, "alert-icon")}<span slot="title">${esc(a.text)}</span>${a.detail ? `<span slot="description">${esc(a.detail)}</span>` : ""}
      </obc-alert-menu-item>`).join("");
    menu.querySelectorAll("obc-alert-menu-item").forEach((el) => {
      const a = list[+el.dataset.i];
      el.addEventListener("ack-click", () => a.ack());
      el.addEventListener("ack-secondary-click", () => a.dismiss?.());
      el.addEventListener("item-click", () => a.click?.());
    });
  }

  // the Alarms drawer's list of active alarms: OpenBridge alert icons and buttons, laid out for the
  // drawer's width (the alert menu's items need more room than the drawer has)
  const drawerList = document.getElementById("al-active");
  function renderDrawer(list) {
    if (!drawerList) return;
    drawerList.classList.add("ob-alert-list");
    drawerList.innerHTML = list.length ? list.map((a, i) => `<li data-i="${i}" class="ob-al ob-al-${TYPE[a.level] ?? "warning"}${a.acked ? " acked" : ""}">
        ${icon(a, "x").replace(' slot="x"', "")}
        <div class="ob-al-text"><b>${esc(a.text)}</b>${a.detail ? `<small>${esc(a.detail)}</small>` : ""}
          <span class="ob-al-time">${hhmm(a.since)}${a.held ? " · dismissed for a minute" : a.acked ? " · acknowledged" : ""}</span></div>
        <div class="ob-al-acts">${a.acked ? "" : '<button type="button" data-act="ack">ACK</button>'}${a.dismiss ? `<button type="button" data-act="dismiss">${esc(a.dismissLabel)}</button>` : ""}</div>
      </li>`).join("") : '<li class="empty">No alarms or warnings.</li>';
    drawerList.querySelectorAll("li.ob-al").forEach((el) => {
      const a = list[+el.dataset.i];
      el.querySelector('[data-act="ack"]')?.addEventListener("click", () => a.ack());
      el.querySelector('[data-act="dismiss"]')?.addEventListener("click", () => a.dismiss?.());
      el.querySelector(".ob-al-text").addEventListener("click", () => a.click?.());
    });
  }

  function render() {
    const list = sorted();
    renderDrawer(list);
    const unacked = list.filter((a) => !a.acked);
    const top = list[0];
    btn.nAlerts = list.length;
    btn.alertType = top ? TYPE[top.level] : undefined;
    btn.blinking = unacked.length > 0;
    btn.counts = {
      countAlarm: list.filter((a) => a.level === "alarm").length,
      countWarning: list.filter((a) => a.level === "warn").length,
      countCaution: list.filter((a) => a.level === "caution").length,
    };
    // the message: the most important alert still to be acknowledged (the MOB stays shown until cleared)
    const shown = unacked[0] ?? list.find((a) => a.mob);
    msg.hidden = !shown;
    if (shown) {
      msg.type = shown.acked ? "simple" : "with-button";
      msg.showTitle = true;
      msg.showDescription = !!shown.detail;
      msg.showTimestamp = true;
      msg.actionLabel = "ACK";
      msg.innerHTML = `${icon(shown, "primary-icon")}<span slot="title">${esc(shown.text)}</span>` +
        `${shown.detail ? `<span slot="description">${esc(shown.detail)}</span>` : ""}<span slot="time">${hhmm(shown.since)}</span>` +
        (shown.acked ? "" : '<span slot="action-text">ACK</span>');
      msgTarget = shown;
    }
    if (!menu.hidden) renderMenu();
  }
  let msgTarget = null;
  msg.addEventListener("action-click", () => msgTarget?.ack());
  msg.addEventListener("message-click", () => (msgTarget?.click ? msgTarget.click() : setMenu(true)));

  window.addEventListener("racenav-alerts", (e) => { state = e.detail; render(); });
  render();
}
