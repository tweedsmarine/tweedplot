// The user manual, inside the app. The text and pictures are bundled with the
// app (public/manual, imported from the manual's Claude Doc with
// tools/manual_import.py), so the manual can't be separated from the version
// it describes and works offline.
//
// A panel on the right of the window: tabs (reference, tutorials, glossary),
// contents, search across every tab, pictures full size on click, Wide.
// Open with the Manual button on the rail, F1, or the ? in a drawer's title bar
// (`openManual(tab, heading)`).

import { marked } from "marked";
import { invoke } from "@tauri-apps/api/core";

const KEY = "racenav.manual";
const BASE = "manual/";

/** Drawer → the reference section that explains it. */
export const PANE_HELP = {
  weather: ["reference", "Weather"],
  course: ["reference", "Marks, routes and the library"],
  routing: ["reference", "Routing"],
  fleet: ["reference", "AIS targets"],
  library: ["reference", "Marks, routes and the library"],
  chart: ["reference", "Charts"],
  inst: ["reference", "Instruments"],
};

export const slug = (s) => s.toLowerCase().replace(/<[^>]+>/g, "").replace(/&[a-z#0-9]+;/g, "").replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "");
const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export function setupManual() {
  let book = null; // { title, updated, tabs: [{ id, title, file, md, html, sections }] }
  let tab = null;
  let state = { tab: "reference", scroll: {}, wide: false, toc: true, width: 560 };
  try { Object.assign(state, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch { /* ignore */ }
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(state)); } catch { /* ignore */ } };

  const el = document.createElement("aside");
  el.id = "manual";
  el.hidden = true;
  el.innerHTML = `
    <div class="man-grip" title="Drag to resize"></div>
    <header>
      <div class="man-top">
        <strong>Manual</strong>
        <input type="search" class="man-search" placeholder="Search the manual" aria-label="Search the manual">
        <button class="man-toc-btn" title="Show or hide the contents">Contents</button>
        <button class="man-wide" title="Give the manual most of the window">Wide</button>
        <button class="man-close" title="Close (Esc)">✕</button>
      </div>
      <nav class="man-tabs"></nav>
    </header>
    <div class="man-main">
      <nav class="man-toc"></nav>
      <article class="man-body" tabindex="0"></article>
      <div class="man-results" hidden></div>
    </div>
    <div class="man-lightbox" hidden><img alt=""><p></p></div>`;
  document.body.append(el);
  const $ = (s) => el.querySelector(s);
  const body = $(".man-body"), toc = $(".man-toc"), results = $(".man-results"), search = $(".man-search");

  function layout() {
    el.classList.toggle("wide", !!state.wide);
    el.classList.toggle("no-toc", !state.toc);
    el.style.setProperty("--man-w", `${Math.max(380, Math.min(state.width, window.innerWidth - 120))}px`);
    $(".man-wide").classList.toggle("on", !!state.wide);
    $(".man-toc-btn").classList.toggle("on", !!state.toc);
  }

  async function load() {
    if (book) return book;
    const r = await fetch(BASE + "manual.json");
    book = await r.json();
    await Promise.all(book.tabs.map(async (t) => {
      t.md = await (await fetch(BASE + t.file)).text();
      t.html = null;
      // plain-text sections for search
      t.sections = [];
      let cur = { heading: t.title, level: 1, text: "" };
      for (const line of t.md.split("\n")) {
        const h = /^(#{1,3})\s+(.*)$/.exec(line);
        if (h) { t.sections.push(cur); cur = { heading: h[2].trim(), level: h[1].length, text: "" }; continue; }
        if (/^\s*\|?\s*:?-{3}/.test(line)) continue; // table rule
        cur.text += line.replace(/!\[[^\]]*\]\([^)]*\)/g, "").replace(/^\s*[-*] \[[ xX]\]/, "").replace(/[*_`#>|]/g, " ").replace(/\[([^\]]*)\]\([^)]*\)/g, "$1") + " ";
      }
      t.sections.push(cur);
    }));
    $(".man-tabs").innerHTML = book.tabs.map((t) => `<button data-tab="${t.id}">${esc(t.title)}</button>`).join("");
    return book;
  }

  const svgCache = new Map();
  async function inlineSvg(img) {
    const src = img.getAttribute("src");
    if (!svgCache.has(src)) svgCache.set(src, fetch(BASE + src).then((r) => r.text()).catch(() => ""));
    const svg = await svgCache.get(src);
    const fig = img.closest("figure");
    if (svg && fig) { fig.classList.add("diagram"); img.outerHTML = svg; }
  }

  function render(t) {
    if (!t.html) {
      const renderer = new marked.Renderer();
      renderer.heading = ({ tokens, depth }) => {
        const text = marked.Parser.parseInline(tokens);
        return `<h${depth} id="${slug(text)}">${text}</h${depth}>`;
      };
      renderer.image = ({ href, text }) => `<figure><img src="${esc(href)}" alt="${esc(text)}" loading="lazy"><figcaption>${esc(text)}</figcaption></figure>`;
      renderer.link = ({ href, tokens }) => `<a href="${esc(href)}"${/^https?:/.test(href) ? ' class="ext" title="Opens in your browser"' : ""}>${marked.Parser.parseInline(tokens)}</a>`;
      t.html = marked.parse(t.md, { renderer, gfm: true });
    }
    body.innerHTML = t.html + `<p class="man-foot">${esc(book.title)} · updated ${esc(book.updated)} · bundled with this version of TweedPlot</p>`;
    // a picture alone in a paragraph: lift the figure out of the <p>
    for (const p of body.querySelectorAll("p > figure:only-child")) p.replaceWith(p.firstChild);
    for (const img of body.querySelectorAll('img[src$=".svg"]')) inlineSvg(img);
    for (const img of body.querySelectorAll("img")) img.src = BASE + img.getAttribute("src");
    toc.innerHTML = [...body.querySelectorAll("h2, h3")]
      .map((h) => `<a href="#${h.id}" class="lv${h.tagName[1]}">${h.textContent}</a>`).join("");
  }

  async function show(id, heading, term) {
    await load();
    const t = book.tabs.find((x) => x.id === id) ?? book.tabs[0];
    if (tab) state.scroll[tab.id] = body.scrollTop;
    if (tab !== t) { tab = t; render(t); }
    state.tab = t.id;
    save();
    for (const b of el.querySelectorAll(".man-tabs button")) b.classList.toggle("on", b.dataset.tab === t.id);
    results.hidden = true;
    body.hidden = false;
    clearMarks();
    if (heading) {
      const h = body.querySelector(`#${CSS.escape(slug(heading))}`) || [...body.querySelectorAll("h1,h2,h3")].find((x) => x.textContent.trim() === heading);
      if (h) { body.scrollTop = h.offsetTop - 8; h.classList.add("flash"); setTimeout(() => h.classList.remove("flash"), 1500); }
    } else body.scrollTop = state.scroll[t.id] ?? 0;
    if (term) markTerm(term, heading);
  }

  // ---- search -------------------------------------------------------------------
  function clearMarks() {
    for (const m of body.querySelectorAll("mark.man-hit")) m.replaceWith(document.createTextNode(m.textContent));
    body.normalize();
  }
  function markTerm(term, heading) {
    const re = new RegExp(term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi");
    const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.parentElement.closest("svg, script") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT) });
    const nodes = [];
    while (walker.nextNode()) if (re.test(walker.currentNode.nodeValue)) nodes.push(walker.currentNode);
    for (const n of nodes) {
      const frag = document.createDocumentFragment();
      let last = 0;
      n.nodeValue.replace(re, (m, i) => {
        frag.append(n.nodeValue.slice(last, i));
        const mk = document.createElement("mark"); mk.className = "man-hit"; mk.textContent = m; frag.append(mk);
        last = i + m.length;
      });
      frag.append(n.nodeValue.slice(last));
      n.replaceWith(frag);
    }
    // the first hit at or after the section scrolled to
    const h = heading ? body.querySelector(`#${CSS.escape(slug(heading))}`) : null;
    const first = [...body.querySelectorAll("mark.man-hit")].find((m) => !h || m.compareDocumentPosition(h) & Node.DOCUMENT_POSITION_PRECEDING) ?? body.querySelector("mark.man-hit");
    if (first) body.scrollTop = Math.max(0, first.offsetTop - 80);
  }
  function runSearch(q) {
    q = q.trim();
    if (q.length < 2) { results.hidden = true; body.hidden = false; return; }
    const ql = q.toLowerCase();
    const hits = [];
    for (const t of book.tabs) for (const s of t.sections) {
      const text = s.text.replace(/\s+/g, " ");
      const i = text.toLowerCase().indexOf(ql);
      const inHead = s.heading.toLowerCase().includes(ql);
      if (i < 0 && !inHead) continue;
      const count = text.toLowerCase().split(ql).length - 1 + (inHead ? 3 : 0);
      const a = Math.max(0, i - 60);
      const snip = i < 0 ? text.slice(0, 140) : (a > 0 ? "…" : "") + text.slice(a, i + q.length + 80) + "…";
      hits.push({ t, s, count, snip });
    }
    hits.sort((x, y) => y.count - x.count);
    const hl = (s) => esc(s).replace(new RegExp(esc(q).replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi"), (m) => `<mark>${m}</mark>`);
    results.innerHTML = hits.length
      ? `<p class="man-count">${hits.length} section${hits.length > 1 ? "s" : ""} mention “${esc(q)}”</p>` +
        hits.slice(0, 60).map((h, k) => `<button data-k="${k}"><span class="where">${esc(h.t.title)} › ${hl(h.s.heading)}</span><span class="snip">${hl(h.snip)}</span></button>`).join("")
      : `<p class="man-count">Nothing in the manual mentions “${esc(q)}”.</p>`;
    results.onclick = (e) => {
      const b = e.target.closest("button[data-k]");
      if (!b) return;
      const h = hits[+b.dataset.k];
      show(h.t.id, h.s.level > 1 ? h.s.heading : null, q);
    };
    results.hidden = false;
    body.hidden = true;
  }
  let timer = 0;
  search.addEventListener("input", () => { clearTimeout(timer); timer = setTimeout(async () => { await load(); runSearch(search.value); }, 150); });
  search.addEventListener("keydown", (e) => { if (e.key === "Escape" && search.value) { e.stopPropagation(); e.preventDefault(); search.value = ""; runSearch(""); } });

  // ---- events -------------------------------------------------------------------
  $(".man-tabs").addEventListener("click", (e) => { const b = e.target.closest("button[data-tab]"); if (b) { search.value = ""; show(b.dataset.tab); } });
  $(".man-close").onclick = () => close();
  $(".man-wide").onclick = () => { state.wide = !state.wide; layout(); save(); };
  $(".man-toc-btn").onclick = () => { state.toc = !state.toc; layout(); save(); };
  toc.addEventListener("click", (e) => {
    const a = e.target.closest("a[href^='#']");
    if (!a) return;
    e.preventDefault();
    const h = body.querySelector(a.getAttribute("href"));
    if (h) { results.hidden = true; body.hidden = false; body.scrollTop = h.offsetTop - 8; }
  });
  body.addEventListener("click", (e) => {
    const a = e.target.closest("a[href]");
    if (a) {
      e.preventDefault();
      const href = a.getAttribute("href");
      if (/^https?:/.test(href)) invoke("open_url", { url: href }).catch(() => {});
      else if (href.startsWith("#")) { const h = body.querySelector(href); if (h) body.scrollTop = h.offsetTop - 8; }
      return;
    }
    const img = e.target.closest("figure:not(.diagram) img");
    const fig = e.target.closest("figure.diagram");
    if (img || fig) {
      const lb = $(".man-lightbox");
      lb.querySelector(".lb-svg")?.remove();
      const pic = lb.querySelector("img");
      pic.hidden = !img;
      if (img) pic.src = img.src;
      else { const box = document.createElement("div"); box.className = "lb-svg"; box.innerHTML = fig.querySelector("svg")?.outerHTML ?? ""; lb.prepend(box); }
      lb.querySelector("p").textContent = img ? img.alt : "";
      lb.hidden = false;
    }
  });
  $(".man-lightbox").onclick = () => { $(".man-lightbox").hidden = true; };
  body.addEventListener("scroll", () => { if (tab) state.scroll[tab.id] = body.scrollTop; });

  // resize from the left edge
  $(".man-grip").addEventListener("pointerdown", (e) => {
    e.preventDefault();
    const grip = e.currentTarget;
    grip.setPointerCapture(e.pointerId);
    const move = (ev) => { state.width = window.innerWidth - ev.clientX; state.wide = false; layout(); };
    const up = () => { grip.removeEventListener("pointermove", move); grip.removeEventListener("pointerup", up); save(); };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
  });

  window.addEventListener("keydown", (e) => {
    if (e.key === "F1") { e.preventDefault(); el.hidden ? open() : close(); return; }
    if (e.key === "Escape" && !el.hidden && !e.defaultPrevented) {
      if (!$(".man-lightbox").hidden) { $(".man-lightbox").hidden = true; e.preventDefault(); return; }
      if (el.contains(document.activeElement) || !document.body.classList.contains("drawer-open")) { close(); e.preventDefault(); }
    }
  });
  window.addEventListener("resize", layout);

  const listeners = [];
  function open(id, heading, term) {
    el.hidden = false;
    layout();
    document.body.classList.add("manual-open");
    for (const f of listeners) f(true);
    return show(id ?? state.tab, heading, term).catch((err) => { body.innerHTML = `<p>Could not load the manual: ${esc(String(err))}</p>`; });
  }
  function close() {
    el.hidden = true;
    document.body.classList.remove("manual-open");
    for (const f of listeners) f(false);
  }

  return {
    open,
    close,
    toggle: () => (el.hidden ? open() : close()),
    isOpen: () => !el.hidden,
    /** Open at the section that explains a drawer. */
    help: (pane) => { const [t, h] = PANE_HELP[pane] ?? ["reference", null]; return open(t, h); },
    onToggle: (f) => listeners.push(f),
  };
}
