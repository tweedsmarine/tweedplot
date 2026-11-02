// The disclaimer: shown at the first start, and again whenever its text changes (VERSION), over
// everything else until accepted. About › Disclaimer shows it again at any time.
// The wording is to be checked by TweedsMarine's lawyer before a public release.
const KEY = "racenav.disclaimer";
/** Raise when the text changes in substance: everyone is asked to accept it again. */
const VERSION = "1";

const TEXT = `
  <h1>Before you use TweedPlot</h1>
  <p class="dc-strong">TweedPlot is an aid to navigation. It is not a type-approved ECDIS, and it does not replace official charts and publications. Not for navigation: keep the official charts, a proper lookout, and every other means of navigation available to you.</p>
  <ul>
    <li>Charts, depths, tides, weather, AIS and other data come from third parties (LINZ, the IHO, NOAA, ECMWF and others). They can be incomplete, out of date or wrong, and they may be drawn differently from the official chart. Forecasts and routes are estimates.</li>
    <li>TweedPlot is provided <b>"as is", without warranty of any kind</b>, express or implied, including fitness for a particular purpose. To the extent the law allows, TweedsMarine and the contributors are not liable for any loss or damage arising from its use (GNU GPL v3, sections 15 and 16).</li>
    <li>The chart symbols, colours and portrayal rules come from the IHO S-101 Portrayal Catalogue, provided "as is" by the International Hydrographic Organization (IHO), NOAA and the contributors, who disclaim all warranties and all liability for its use. They did not make TweedPlot and do not endorse it. Full notice: Manual › Reference › Data sources and credits.</li>
    <li>You are responsible for the safe navigation of your vessel.</li>
  </ul>`;

export function setupDisclaimer() {
  const el = document.createElement("div");
  el.id = "disclaimer";
  el.hidden = true;
  el.setAttribute("role", "alertdialog");
  el.setAttribute("aria-label", "Disclaimer");
  el.innerHTML = `<div class="dc-box">${TEXT}<div class="dc-go"><button class="dc-ok">I understand and accept</button></div></div>`;
  document.body.append(el);
  const ok = el.querySelector(".dc-ok");
  let accepted = false;
  try { accepted = localStorage.getItem(KEY) === VERSION; } catch { /* storage unavailable: ask */ }
  ok.onclick = () => {
    try { localStorage.setItem(KEY, VERSION); } catch { /* ignore */ }
    el.hidden = true;
  };
  // seen again from About: the same text, the same button
  document.getElementById("about-disclaimer")?.addEventListener("click", (e) => { e.preventDefault(); el.hidden = false; ok.focus(); });
  // every window asks until it has been accepted in one (they share the setting)
  if (!accepted) {
    el.hidden = false;
    ok.focus();
  }
  // nothing behind it reacts to keys while it is up
  window.addEventListener("keydown", (e) => { if (!el.hidden && e.key !== "Tab" && e.key !== "Enter" && e.key !== " ") e.stopImmediatePropagation(); }, true);
}
