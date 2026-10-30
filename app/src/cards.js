// Drawer cards: each heading (h3) in a drawer, tab or form and what follows it becomes a card that
// opens and closes, one open at a time per group, with a one-line summary of its settings on the
// right. Only the markup's headings are used; every control keeps its id and place in its card.
// A heading's data-lv (detail level) moves to the card.

const val = (id) => document.getElementById(id);
const num = (id) => val(id)?.value ?? "";
const sel = (id) => val(id)?.selectedOptions?.[0]?.textContent?.trim() ?? "";
const line = (on, len) => (val(on)?.checked ? sel(len) || "on" : "off");

// summaries by heading text
const SUMMARY = {
  Safety: () => `depth ${num("set-safety-depth")} m · contour ${num("set-safety-contour")} m`,
  "Chart display": () => sel("set-dispcat"),
  "Own ship": () => `heading line ${line("set-hdg-line", "set-hdg-len")} · COG line ${line("set-cog-line", "set-cog-len")}`,
  "How much to show": () => sel("set-detail"),
};

// drawers where "n on" says something useful (lists of switches)
const COUNT_PANES = new Set(["chart", "alarms"]);

function onCount(card) {
  const boxes = [...card.querySelectorAll("input[type=checkbox]")];
  if (!boxes.length) return "";
  const on = boxes.filter((b) => b.checked).length;
  return on ? `${on} on` : "all off";
}

function wrap(container) {
  let card = null;
  const cards = [];
  for (const el of [...container.children]) {
    if (el.tagName === "H3") {
      card = document.createElement("details");
      card.className = "dcard";
      if (el.id) card.id = el.id;
      if (el.dataset.lv) card.dataset.lv = el.dataset.lv;
      const sum = document.createElement("summary");
      const title = document.createElement("span");
      title.className = "dc-title";
      title.append(...el.childNodes);
      const note = document.createElement("span");
      note.className = "dc-sum";
      sum.append(title, note);
      // a button in a heading (e.g. "Target list") works without opening or closing the card
      sum.addEventListener("click", (e) => { if (e.target.closest("button")) e.preventDefault(); });
      card.append(sum);
      card.dataset.title = (title.firstChild?.textContent ?? title.textContent).replace(/\s+/g, " ").trim();
      el.replaceWith(card);
      cards.push(card);
    } else if (card) {
      card.append(el);
    }
  }
  return cards;
}

export function setupCards() {
  const groups = [];
  for (const pane of document.querySelectorAll(".pane[data-pane]")) {
    const containers = [pane, ...pane.querySelectorAll(".tab, form, .ui-settings")];
    for (const c of containers) {
      if ([...c.children].filter((k) => k.tagName === "H3").length < 2) continue;
      const cards = wrap(c);
      cards[0].open = true;
      groups.push({ pane, cards });
      c.addEventListener("toggle", (e) => {
        const t = e.target;
        if (!t.classList?.contains("dcard") || !t.open) return;
        for (const o of cards) if (o !== t) o.open = false;
      }, true);
    }
  }
  const refresh = () => {
    for (const { pane, cards } of groups) {
      const counts = COUNT_PANES.has(pane.dataset.pane);
      for (const c of cards) {
        const f = SUMMARY[c.dataset.title];
        c.querySelector(".dc-sum").textContent = f ? f() : counts ? onCount(c) : "";
      }
    }
  };
  document.addEventListener("change", (e) => { if (e.target.closest?.(".dcard")) refresh(); });
  document.addEventListener("input", (e) => { if (e.target.closest?.(".dcard")) refresh(); });
  window.addEventListener("racenav-detail", refresh);
  // controls are filled in after start, and some change from elsewhere: refresh when a drawer opens
  for (const { pane } of groups) new MutationObserver(refresh).observe(pane, { attributes: true, attributeFilter: ["hidden"] });
  setTimeout(refresh, 1500);
  refresh();
}
