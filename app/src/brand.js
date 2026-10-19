// TweedPlot branding: the logo button at the top of the rail opens the About box.
import { invoke } from "@tauri-apps/api/core";

export const BRAND = { name: "TweedPlot", tagline: "Professional Marine Chart Plotter", site: "https://tweedsmarine.nz", company: "TweedsMarine" };

export function setupBrand() {
  const $ = (id) => document.getElementById(id);
  const box = $("about");
  $("brand-btn").onclick = () => {
    box.hidden = !box.hidden;
    if (!box.hidden) invoke("app_info").then((i) => { $("about-version").textContent = `Version ${i.version}`; }).catch(() => {});
  };
  $("about-close").onclick = () => { box.hidden = true; };
  $("about-site").onclick = (e) => { e.preventDefault(); invoke("open_url", { url: BRAND.site }).catch(() => { window.open(BRAND.site); }); };
}
