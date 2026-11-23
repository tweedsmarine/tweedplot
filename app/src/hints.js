// Help text in the drawers: the grey explanatory notes are tucked away. A card with notes gets an
// (i) in its header: hover for the text, click to open the card with its notes showing (click again
// to hide them). A note outside a card is put in the tooltip of the control above it.
// Notes that the program fills in (they have an id) are status lines and are left alone.

// A note inside a block the program shows only sometimes (hidden now, and not a drawer, tab or
// card) is a message for that moment, not help: "the route being navigated is not the one below".
function inShownLater(n) {
  for (let e = n.parentElement; e && !e.classList.contains("pane"); e = e.parentElement) {
    if (e.hidden && !e.classList.contains("tab")) return true;
  }
  return false;
}

export function setupHints() {
  const notes = [...document.querySelectorAll(".pane div.wx-note, .pane p.settings-note")]
    .filter((n) => !n.id && !n.querySelector("[id]") && n.textContent.trim() && !inShownLater(n));
  const byCard = new Map();
  for (const n of notes) {
    n.classList.add("hint");
    const text = n.textContent.replace(/\s+/g, " ").trim();
    const card = n.closest(".dcard");
    if (card) {
      if (!byCard.has(card)) byCard.set(card, []);
      byCard.get(card).push(text);
    } else {
      const prev = n.previousElementSibling;
      if (prev && !prev.title && !prev.dataset.tip) prev.title = text;
    }
  }
  for (const [card, texts] of byCard) {
    const b = document.createElement("button");
    b.className = "dc-info";
    b.type = "button";
    b.textContent = "ⓘ";
    b.title = texts.join("\n\n");
    b.setAttribute("aria-label", "About these settings");
    // (the click doesn't open or close the card itself: cards.js stops that for buttons in a heading)
    b.onclick = () => { const on = card.classList.toggle("hints-on"); if (on) card.open = true; };
    card.querySelector("summary").insertBefore(b, card.querySelector(".dc-sum"));
  }
}
