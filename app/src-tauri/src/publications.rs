//! Nautical publications the chart notes refer to: LINZ's New Zealand Nautical Almanac
//! (NZ 204) and its Annual New Zealand Notices to Mariners, one PDF per notice. Not
//! shipped with TweedPlot: downloaded from linz.govt.nz on the mariner's request, kept in
//! `<data>/publications`, and opened in the PDF viewer. A new edition comes out each
//! 1 July; the file names carry it (hydro_202627-almanac_ANTM-19_pdf.pdf). Crown
//! copyright: "Sourced from Land Information New Zealand, Crown Copyright Reserved."

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SITE: &str = "https://www.linz.govt.nz";
const NOTICES_PAGE: &str = "https://www.linz.govt.nz/products-services/maritime-safety/notices-mariners/nz-annual-notices-mariners";
const ALMANAC_PAGE: &str = "https://www.linz.govt.nz/products-services/maritime-safety/new-zealand-nautical-almanac/nz-nautical-almanac-nz-204-full-version";
const UA: &str = concat!("TweedPlot/", env!("CARGO_PKG_VERSION"), " (marine chart plotter; +https://tweedsmarine.nz)");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Notice {
    /// "19", "4A"
    pub no: String,
    pub title: String,
    pub url: String,
    /// the file kept, in the publications folder
    pub file: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    /// "2026/27"
    pub edition: String,
    /// when downloaded (unix seconds)
    pub fetched: i64,
    pub notices: Vec<Notice>,
    /// the full almanac's file, when downloaded
    pub almanac: Option<String>,
}

pub fn dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    let dev = crate::dev_root();
    if dev.is_dir() {
        return dev.join("publications");
    }
    app.path().app_data_dir().map(|p| p.join("publications")).unwrap_or_else(|_| PathBuf::from("publications"))
}

fn read_index(dir: &Path) -> Option<Index> {
    serde_json::from_slice(&std::fs::read(dir.join("index.json")).ok()?).ok()
}

/// "2026/27" from a LINZ almanac file name (hydro_202627-…).
fn edition_of(url: &str) -> Option<String> {
    let i = url.find("hydro_")? + 6;
    let d = url.get(i..i + 6)?;
    d.bytes().all(|c| c.is_ascii_digit()).then(|| format!("{}/{}", &d[..4], &d[4..]))
}

/// Every link in `html` (href, text without tags) whose href contains `needle` and ends in .pdf.
fn pdf_links(html: &str, needle: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("href=\"") {
        rest = &rest[i + 6..];
        let Some(j) = rest.find('"') else { break };
        let href = rest[..j].replace("&amp;", "&");
        rest = &rest[j..];
        if !href.contains(needle) || !href.to_ascii_lowercase().ends_with(".pdf") {
            continue;
        }
        // the link's text: up to </a>, tags removed
        let text = rest.find('>').and_then(|k| {
            let after = &rest[k + 1..];
            after.find("</a>").map(|e| &after[..e])
        });
        let mut t = String::new();
        let mut tag = false;
        for c in text.unwrap_or("").chars() {
            match c {
                '<' => tag = true,
                '>' => tag = false,
                _ if !tag => t.push(c),
                _ => {}
            }
        }
        let t = t
            .replace("&amp;", "&")
            .replace("&nbsp;", " ")
            .replace("&ndash;", "–")
            .replace("&mdash;", "–")
            .replace("&#8211;", "–")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let url = if href.starts_with("http") { href } else { format!("{SITE}{href}") };
        out.push((url, t));
    }
    out
}

/// The full almanac's PDF on LINZ's "full version" page. LINZ has named it differently from one
/// edition to the next ("hydro_202324-almanac_full-nautical-almanac_pdf.pdf", then "New Zealand
/// Nautical Almanac 2025-26 Edition - NZ 204_1.pdf" under /system/files), so it is chosen by what
/// the link says rather than one file name: a PDF about the almanac or NZ 204 that isn't an Annual
/// Notice, an extract or a section, "full" and "edition" preferred. A page with one other PDF only
/// (a plain "Download" link) gives that one.
fn almanac_link(html: &str) -> Option<String> {
    let links = pdf_links(html, "");
    let words = |url: &str, text: &str| format!("{} {}", url.replace("%20", " ").replace(['_', '-'], " "), text).to_ascii_lowercase();
    let other = |w: &str| w.contains("antm") || w.contains("notice to mariners") || w.contains("extract") || w.contains("section");
    let mut best: Option<(i32, String)> = None;
    for (url, text) in &links {
        let w = words(url, text);
        if other(&w) || !(w.contains("almanac") || w.contains("nz 204") || w.contains("nz204")) {
            continue;
        }
        let score = 3 * i32::from(w.contains("full")) + 2 * i32::from(w.contains("edition")) + i32::from(w.contains("204"));
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, url.clone()));
        }
    }
    if best.is_none() {
        let rest: Vec<_> = links.iter().filter(|(u, t)| !other(&words(u, t))).collect();
        if rest.len() == 1 {
            return Some(rest[0].0.clone());
        }
    }
    best.map(|(_, u)| u)
}

/// The Annual Notices listed on LINZ's page: number, title, PDF.
fn parse_notices(html: &str) -> Vec<Notice> {
    let mut v: Vec<Notice> = Vec::new();
    for (url, text) in pdf_links(html, "almanac_ANTM-") {
        let Some(i) = url.find("almanac_ANTM-") else { continue };
        let no: String = url[i + 13..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if no.is_empty() || v.iter().any(|n| n.no == no) {
            continue;
        }
        // "Annual Notice to Mariners No. 19 – Rocket launch … - NZ Nautical Almanac 2026/27": the title
        let mut title = text.split(" - NZ Nautical Almanac").next().unwrap_or("").trim().to_string();
        for sep in [" – ", " - ", ": "] {
            if let Some((head, tail)) = title.split_once(sep) {
                if head.to_ascii_lowercase().contains("notice") {
                    title = tail.trim().to_string();
                    break;
                }
            }
        }
        v.push(Notice { file: format!("ANTM-{no}.pdf"), no, title, url });
    }
    v.sort_by_key(|n| {
        let d: String = n.no.chars().take_while(|c| c.is_ascii_digit()).collect();
        (d.parse::<u32>().unwrap_or(999), n.no.clone())
    });
    v
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder().user_agent(UA).timeout(std::time::Duration::from_secs(60)).build().map_err(|e| e.to_string())
}

fn get_text(c: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    let r = c.get(url).send().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("{url}: HTTP {}", r.status()));
    }
    r.text().map_err(|e| e.to_string())
}

fn get_pdf(c: &reqwest::blocking::Client, url: &str, to: &Path) -> Result<(), String> {
    let r = c.get(url).send().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("{url}: HTTP {}", r.status()));
    }
    let b = r.bytes().map_err(|e| e.to_string())?;
    if !b.starts_with(b"%PDF") {
        return Err(format!("{url}: not a PDF"));
    }
    let tmp = to.with_extension("part");
    std::fs::write(&tmp, &b).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, to).map_err(|e| e.to_string())
}

fn status_of(dir: &Path) -> Value {
    match read_index(dir) {
        Some(i) => json!({ "edition": i.edition, "fetched": i.fetched, "notices": i.notices, "almanac": i.almanac.is_some() }),
        None => json!({ "edition": null, "notices": [] }),
    }
}

/// What has been downloaded: { edition, fetched, notices: [{no, title, url, file}], almanac }.
#[tauri::command]
pub async fn pubs_status(app: tauri::AppHandle) -> Value {
    status_of(&dir(&app))
}

/// Download the current Annual Notices to Mariners (and, with `almanac`, the full
/// Almanac, about 9 MB) from LINZ, replacing an older edition.
#[tauri::command]
pub async fn pubs_download(app: tauri::AppHandle, almanac: bool) -> Result<Value, String> {
    let d = dir(&app);
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let c = client()?;
        let notices = parse_notices(&get_text(&c, NOTICES_PAGE)?);
        if notices.is_empty() {
            return Err("LINZ's Annual Notices to Mariners page has changed: no notices found on it".into());
        }
        let edition = edition_of(&notices[0].url).unwrap_or_default();
        let fresh = d.join(".new");
        let _ = std::fs::remove_dir_all(&fresh);
        std::fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
        for n in &notices {
            get_pdf(&c, &n.url, &fresh.join(&n.file))?;
        }
        let mut alm = None;
        if almanac {
            let page = get_text(&c, ALMANAC_PAGE)?;
            if let Some(url) = almanac_link(&page) {
                get_pdf(&c, &url, &fresh.join("NZ204-almanac.pdf"))?;
                alm = Some("NZ204-almanac.pdf".to_string());
            }
        } else if let Some(old) = read_index(&d).and_then(|i| i.almanac.filter(|_| i.edition == edition)) {
            // the same edition's almanac kept
            let _ = std::fs::copy(d.join(&old), fresh.join(&old));
            alm = Some(old);
        }
        let idx = Index {
            edition,
            fetched: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t| t.as_secs() as i64).unwrap_or(0),
            notices,
            almanac: alm,
        };
        std::fs::write(fresh.join("index.json"), serde_json::to_vec_pretty(&idx).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        // swap the new set in
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.path().is_file() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        for e in std::fs::read_dir(&fresh).map_err(|e| e.to_string())?.flatten() {
            std::fs::rename(e.path(), d.join(e.file_name())).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_dir(&fresh);
        Ok(status_of(&d))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open an Annual Notice ("19", "4A") or the full almanac ("almanac") in the PDF viewer.
/// False when it hasn't been downloaded.
#[tauri::command]
pub async fn pub_open(app: tauri::AppHandle, no: String) -> Result<bool, String> {
    let d = dir(&app);
    let Some(i) = read_index(&d) else { return Ok(false) };
    let file = if no == "almanac" { i.almanac } else { i.notices.iter().find(|n| n.no.eq_ignore_ascii_case(&no)).map(|n| n.file.clone()) };
    let Some(file) = file else { return Ok(false) };
    let p = d.join(file);
    if !p.is_file() {
        return Ok(false);
    }
    // Explorer opens a file as a double-click does (ShellExecute, the PDF program the user chose).
    // rundll32's url.dll FileProtocolHandler, used before, did nothing at all on Chris's PC.
    #[cfg(windows)]
    let r = std::process::Command::new("explorer.exe").arg(&p).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&p).spawn();
    r.map(|_| true).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_almanac_however_linz_names_it() {
        // 2025/26: under /system/files, named for the edition
        let a = r#"<a href="/sites/default/files/2026-06/hydro_202627-almanac_title-section-pages-1-and-2_pdf.pdf">Title section</a>
          <a href="/system/files/2025-06/New%20Zealand%20Nautical%20Almanac%202025-26%20Edition%20-%20NZ%20204_1.pdf">New Zealand Nautical Almanac 2025-26 Edition - NZ 204</a>
          <a href="/sites/default/files/2025-06/hydro_202526-almanac_ANTM-1_pdf.pdf">Annual Notice to Mariners No. 1</a>"#;
        assert_eq!(
            almanac_link(a).as_deref(),
            Some("https://www.linz.govt.nz/system/files/2025-06/New%20Zealand%20Nautical%20Almanac%202025-26%20Edition%20-%20NZ%20204_1.pdf")
        );
        // 2023/24: the old name
        let b = r#"<a href="/sites/default/files/2023-06/hydro_202324-almanac_full-nautical-almanac_pdf.pdf"><span>Download</span></a>"#;
        assert_eq!(almanac_link(b).as_deref(), Some("https://www.linz.govt.nz/sites/default/files/2023-06/hydro_202324-almanac_full-nautical-almanac_pdf.pdf"));
        // a page with one plain download link
        let c = r#"<a href="/system/files/2026-06/nz204_2026.pdf">Download (PDF, 9 MB)</a><a href="/about">About</a>"#;
        assert_eq!(almanac_link(c).as_deref(), Some("https://www.linz.govt.nz/system/files/2026-06/nz204_2026.pdf"));
        // nothing that is the almanac
        let d = r#"<a href="/x/hydro_202627-almanac_ANTM-19_pdf.pdf">Annual Notice to Mariners No. 19</a>"#;
        assert_eq!(almanac_link(d), None);
    }

    #[test]
    fn reads_the_notices_page() {
        let html = r#"<ul><li><a href="/sites/default/files/2026-06/hydro_202627-almanac_ANTM-19_pdf.pdf" type="application/pdf">Annual Notice to Mariners No. 19 - Rocket launch hazard areas and warnings - NZ Nautical Almanac 2026/27</a> <span>(47.47 KB)</span></li>
          <li><a href="/sites/default/files/2026-06/hydro_202627-almanac_ANTM-4A_pdf.pdf"><span class="x">Annual Notice to Mariners No. 4A &ndash; </span>Modernised Australian ship tracking</a></li>
          <li><a href="/sites/default/files/2026-06/hydro_202627-almanac_ANTM-1_pdf_0.pdf">Annual Notice to Mariners No. 1 - New Zealand charting and Notices to Mariners - NZ Nautical Almanac 2026/27</a></li>
          <li><a href="/about">About</a></li></ul>"#;
        let n = parse_notices(html);
        assert_eq!(n.iter().map(|n| n.no.as_str()).collect::<Vec<_>>(), ["1", "4A", "19"]);
        assert_eq!(n[2].title, "Rocket launch hazard areas and warnings");
        assert_eq!(n[2].url, "https://www.linz.govt.nz/sites/default/files/2026-06/hydro_202627-almanac_ANTM-19_pdf.pdf");
        assert_eq!(n[0].title, "New Zealand charting and Notices to Mariners");
        assert_eq!(edition_of(&n[0].url).as_deref(), Some("2026/27"));
    }
}
