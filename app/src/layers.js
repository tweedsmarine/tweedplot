// Chart › Layers: one list of everything that can be drawn on the chart, so a glance says what is
// on and off. Layers whose own controls live elsewhere (tides and stations in Weather, marks and
// barriers in the Library, AIS in Targets, laylines in Settings) appear here as mirrors of those
// controls (data-mirror="<id>"): ticking one ticks the other, and the owning module is none the
// wiser. Each row's ⚙ goes to the layer's settings (data-go="pane|card or section|element"); a
// "Get…" shows where the layer's data hasn't been downloaded yet (data-dot = the Get data dot).

const $ = (id) => document.getElementById(id);

export function setupLayers({ ui }) {
  const box = document.querySelector('.pane[data-pane="chart"]');
  if (!box) return { sync() {} };
  const mirrors = [...box.querySelectorAll("[data-mirror]")];

  /** Copy the original's state into the mirror (options too, for a select). */
  function pull(m) {
    const o = $(m.dataset.mirror);
    if (!o) { m.closest(".ly-row, .ly-head")?.setAttribute("hidden", ""); return; }
    if (m.type === "checkbox") m.checked = o.checked;
    else if (m.tagName === "SELECT") {
      if (m.options.length !== o.options.length || [...m.options].some((opt, i) => opt.value !== o.options[i].value || opt.textContent !== o.options[i].textContent)) {
        m.innerHTML = o.innerHTML;
      }
      m.value = o.value;
    }
    m.disabled = o.disabled;
  }
  for (const m of mirrors) {
    m.addEventListener("change", () => {
      const o = $(m.dataset.mirror);
      if (!o) return;
      if (m.type === "checkbox") o.checked = m.checked; else o.value = m.value;
      o.dispatchEvent(new Event("change", { bubbles: true }));
      o.dispatchEvent(new Event("input", { bubbles: true }));
      setTimeout(sync, 50);
    });
  }
  // a change on an original (in its own drawer) shows here at once
  document.addEventListener("change", (e) => {
    const id = e.target?.id;
    if (id) for (const m of mirrors) if (m.dataset.mirror === id) pull(m);
  }, true);

  /** Hidden in its own drawer (no wave data in the GRIB, say): hidden here too. */
  function hiddenThere(el) {
    for (let n = el; n && !n.classList?.contains("pane"); n = n.parentElement) {
      if (n.hidden || n.style?.display === "none") return true;
    }
    return false;
  }

  function sync() {
    for (const m of mirrors) pull(m);
    for (const r of box.querySelectorAll("[data-row-of]")) {
      const o = $(r.dataset.rowOf);
      r.hidden = !o || hiddenThere(o);
    }
    // the weather group only means something with a GRIB loaded
    const wx = $("ly-wx");
    if (wx) wx.classList.toggle("ly-empty", ![...wx.querySelectorAll(".ly-row")].some((r) => !r.hidden && r.querySelector("input")));
    for (const g of box.querySelectorAll(".ly-get")) {
      const dot = $(g.dataset.dot);
      g.hidden = !dot || dot.classList.contains("ok");
    }
  }

  /** Go to a layer's settings: "pane|card or section title|element id". */
  function go(where) {
    const [pane, section = "", elId = ""] = where.split("|");
    const tabs = { barriers: "barriers", Laylines: "vessel", Fishing: "vessel", Performance: "vessel" };
    if (pane === "library" && section) ui.openTab("library", tabs[section] ?? section);
    else if (pane === "settings") ui.openTab("settings", tabs[section] ?? "vessel");
    else ui.openTab(pane, null);
    setTimeout(() => {
      const root = document.querySelector(`.pane[data-pane="${pane}"]`);
      if (!root) return;
      let target = null;
      if (section && !tabs[section]?.startsWith("barriers")) {
        // a drawer card (h3) or a folding section (summary)
        const card = [...root.querySelectorAll("details.dcard")].find((d) => d.dataset.title === section && d.offsetParent !== null)
          ?? [...root.querySelectorAll("details > summary")].find((s) => s.textContent.trim() === section)?.parentElement;
        if (card) { card.open = true; target = card; }
      }
      const el = elId ? $(elId) : null;
      if (el) {
        for (let n = el.parentElement; n && n !== root; n = n.parentElement) if (n.tagName === "DETAILS") n.open = true;
        target = el.closest(".data-item, .wx-row, .ly-row") ?? el;
      }
      if (target) {
        target.scrollIntoView({ block: "center", behavior: "smooth" });
        target.classList.add("ly-flash");
        setTimeout(() => target.classList.remove("ly-flash"), 1600);
      }
    }, 80);
  }
  box.addEventListener("click", (e) => {
    const b = e.target.closest("[data-go]");
    if (b) { e.preventDefault(); go(b.dataset.go); return; }
    const c = e.target.closest("[data-click]");
    if (c) { $(c.dataset.click)?.click(); setTimeout(sync, 50); }
  });

  // modules change some originals without an event (a right-click turns laylines on): catch up
  // while the Layers card is open, and each time the drawer opens
  setInterval(() => { if (box.offsetParent !== null) sync(); }, 1500);
  sync();
  return { sync, go };
}
