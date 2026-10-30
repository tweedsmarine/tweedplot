// Help text in the drawers: the grey explanatory notes become tooltips in Simple and Standard, and
// show in full in Expert. A card with notes gets an (i) in its header: hover for the text, click to
// show the notes in that card. A note outside a card is put in the tooltip of the control above it.
// Notes that the program fills in (they have an id) are status lines and are left alone.

export function setupHints() {
  const notes = [...document.querySelectorAll(".pane div.wx-note, .pane p.settings-note")]
    .filter((n) => !n.id && !n.querySelector("[id]") && n.textContent.trim());
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
      if (prev && !prev.title) prev.title = text;
    }
  }
  for (const [card, texts] of byCard) {
    const b = document.createElement("button");
    b.className = "dc-info";
    b.type = "button";
    b.textContent = "ⓘ";
    b.title = texts.join("\n\n");
    b.setAttribute("aria-label", "About these settings");
    b.onclick = () => card.classList.toggle("hints-on");
    card.querySelector("summary").insertBefore(b, card.querySelector(".dc-sum"));
  }
}
