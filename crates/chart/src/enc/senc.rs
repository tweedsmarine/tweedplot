//! The installed ENCs (the SENC, in S-63's words): cells imported from exchange sets,
//! encrypted (S-63) or plain (S-57), with the SA public key and the cell permits they
//! were checked with.
//!
//! Importing an exchange set follows S-63: each cell file's signature is checked against
//! the installed SA key, it is decrypted with a permit from the exchange set's data
//! server, its CRC is checked against CATALOG.031, and base cells, new editions,
//! re-issues, updates and cancellations are applied in order to what is installed.
//! What happened to each cell is reported, with the S-63 error codes (SSE).
//!
//! Layout, in `<enc folder>/.senc`:
//! - `state.json`: the installed cells (edition, update, data server), the latest
//!   product listing of each data server, the base media installed, and the SA key
//! - `permits/<data server>.TXT`: each data server's cell permits
//! - `cells/<CELL>/`: the cell's base file and updates as delivered (still encrypted),
//!   with their signature, text and picture files

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::iso8211::Module;
use super::s63::{self, CellPermit, PublicKey};

/// The IHO Scheme Administrator's certificate (valid to 2033), installed to begin with.
pub const IHO_CRT: &[u8] = include_bytes!("IHO.CRT");

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Info,
    Warn,
    Error,
}

/// One line of a report: a cell (or "" for the whole operation), how it went, and what.
#[derive(Debug, Clone, Serialize)]
pub struct Msg {
    pub cell: String,
    pub level: Level,
    pub text: String,
}

fn msg(cell: &str, level: Level, text: impl Into<String>) -> Msg {
    Msg { cell: cell.to_string(), level, text: text.into() }
}

const SSE05: &str = "SSE 05 - SA Digital Certificate (X509) file is not available. A valid certificate can be obtained from the IHO website or your data supplier";
const SSE10: &str = "SSE 10 - Permits not available for this data server. Contact your data supplier to obtain the correct permits";
const SSE11: &str = "SSE 11 - Cell permit not found. Load the permit file provided by the data supplier";
const SSE15: &str = "SSE 15 - Subscription service has expired. Please contact your data supplier to renew the subscription licence";
const SSE20: &str = "SSE 20 - Subscription service will expire in less than 30 days. Please contact your data supplier to renew the subscription licence";
const SSE22: &str = "SSE 22 - SA Digital Certificate (X509) has expired. A new SA public key can be obtained from the IHO website or from your data supplier";
const SSE23: &str = "SSE 23 - Non sequential update, previous update(s) missing. Try reloading from the base media. If the problem persists contact your data supplier";
const SSE26: &str = "SSE 26 - This ENC is not authenticated by the IHO acting as the Scheme Administrator";

/// A cell as installed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Installed {
    /// data server of the last file installed
    pub ds: String,
    pub edition: u32,
    pub update: u32,
    pub encrypted: bool,
}

/// A cell's entry in a data server's product listing (PRODUCTS.TXT).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Product {
    /// 0: cancelled
    pub edition: u32,
    pub update: u32,
    /// where the base cell is: "B1", "M1;B2", "U1"
    pub location: String,
    pub replaced_by: Vec<String>,
}

/// The installed SA key (or a key the user installed instead).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyInfo {
    pub name: String,
    /// the IHO's own key
    pub sa: bool,
    pub not_after: Option<String>,
    pub subject: String,
    p: String,
    q: String,
    g: String,
    y: String,
}

impl KeyInfo {
    fn key(&self) -> Option<PublicKey> {
        let n = |s: &str| num_bigint::BigUint::parse_bytes(s.as_bytes(), 16);
        Some(PublicKey { p: n(&self.p)?, q: n(&self.q)?, g: n(&self.g)?, y: n(&self.y)? })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    cells: BTreeMap<String, Installed>,
    /// data server → cell → its latest product listing entry
    products: BTreeMap<String, BTreeMap<String, Product>>,
    /// data server → base media ("B1", "M1") → its date
    bases: BTreeMap<String, BTreeMap<String, String>>,
    key: Option<KeyInfo>,
    /// set once the IHO key has been installed the first time (so removing it sticks)
    #[serde(default)]
    key_set: bool,
}

pub struct Senc {
    root: PathBuf,
    state: State,
}

fn iho_key() -> PublicKey {
    s63::parse_certificate(IHO_CRT).expect("built-in IHO certificate").key
}

fn hex(n: &num_bigint::BigUint) -> String {
    n.to_str_radix(16)
}

/// A cell data file name: 8 characters, the third the usage 1-6, a 3-digit extension.
/// Returns (cell, extension number).
fn cell_file(name: &str) -> Option<(String, u32)> {
    let n = name.to_ascii_uppercase();
    let (stem, ext) = n.split_once('.')?;
    if stem.len() != 8 || ext.len() != 3 || !ext.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !(b'1'..=b'6').contains(&stem.as_bytes()[2]) {
        return None;
    }
    Some((stem.to_string(), ext.parse().ok()?))
}

fn is_plain(data: &[u8]) -> bool {
    data.len() > 24 && data[..5].iter().all(u8::is_ascii_digit) && data[6] == b'L'
}

/// The files under a folder, keyed by their path relative to it in upper case with "/".
fn files_under(dir: &Path) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                out.insert(rel.to_string_lossy().replace('\\', "/").to_ascii_uppercase(), p);
            }
        }
    }
    out
}

/// A file of an exchange set's catalogue.
#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    cell: String,
    ext: u32,
    edition: Option<u32>,
    update: Option<u32>,
    crc: Option<u32>,
}

/// CATALOG.031: each file's path, CRC and (S-63) the DSID edition and update in CATD-COMT.
fn read_catalogue(enc_root: &Path, files: &HashMap<String, PathBuf>) -> Option<Vec<Entry>> {
    let path = files.get("CATALOG.031")?;
    let m = Module::parse(&std::fs::read(path).ok()?).ok()?;
    let mut out = Vec::new();
    for r in &m.records {
        let Some(f) = r.field("CATD") else { continue };
        let text = |l: &str| f.get(l).and_then(|v| v.text()).unwrap_or("").trim().to_string();
        let file = text("FILE").replace('\\', "/").to_ascii_uppercase();
        let name = file.rsplit('/').next().unwrap_or("").to_string();
        let Some((cell, ext)) = cell_file(&name) else { continue };
        let Some(p) = files.get(&file).cloned() else {
            out.push(Entry { path: enc_root.join(&file), cell, ext, edition: None, update: None, crc: None });
            continue;
        };
        let comt = text("COMT");
        let kv = |k: &str| comt.split([',', ';']).find_map(|s| s.trim().strip_prefix(k).and_then(|v| v.strip_prefix('=')).and_then(|v| v.trim().parse().ok()));
        let crc = u32::from_str_radix(&text("CRCS"), 16).ok();
        out.push(Entry { path: p, cell, ext, edition: kv("EDTN"), update: kv("UPDN"), crc });
    }
    Some(out)
}

/// The edition and update of a plain (decrypted) cell file, from its DSID.
fn dsid(data: &[u8]) -> Option<(u32, u32)> {
    let m = Module::parse(data).ok()?;
    let f = m.records.iter().find_map(|r| r.field("DSID"))?;
    let n = |l: &str| f.get(l).and_then(|v| v.text().and_then(|t| t.trim().parse().ok()).or_else(|| v.int().map(|i| i as u32)));
    Some((n("EDTN")?, n("UPDN").unwrap_or(0)))
}

/// SERIAL.ENC: data server, week, date, BASE/UPDATE, exchange set number ("B01X03").
#[derive(Debug, Clone, Default)]
struct Serial {
    ds: String,
    date: String,
    kind: String,
    set: String,
}

fn read_serial(root: &Path) -> Option<Serial> {
    let s = std::fs::read_to_string(root.join("SERIAL.ENC")).ok()?;
    let g = |a: usize, b: usize| s.get(a..b).unwrap_or("").trim().to_string();
    Some(Serial { ds: g(0, 2), date: g(12, 20), kind: g(20, 30), set: g(35, 41) })
}

/// A file in the exchange set's INFO folder, or (large media, several exchange sets on
/// one medium) the medium's INFO folder.
fn find_info(root: &Path, name: &str) -> Option<PathBuf> {
    let look = |dir: &Path| -> Option<PathBuf> {
        let info = std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("INFO"))?.path();
        std::fs::read_dir(info).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
    };
    look(root).or_else(|| root.parent().and_then(look))
}

fn read_products(root: &Path) -> BTreeMap<String, Product> {
    let mut out = BTreeMap::new();
    let Some(p) = find_info(root, "PRODUCTS.TXT") else { return out };
    let text = String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).to_string();
    let mut in_enc = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with(':') {
            in_enc = l.eq_ignore_ascii_case(":ENC");
            continue;
        }
        if !in_enc || l.is_empty() {
            continue;
        }
        let f: Vec<&str> = l.split(',').map(str::trim).collect();
        let cell = f[0].split('.').next().unwrap_or("").to_ascii_uppercase();
        if cell.len() != 8 {
            continue;
        }
        let n = |i: usize| f.get(i).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let location = f.get(34).map(|s| s.to_string()).unwrap_or_default();
        let replaced_by = f.get(35).map(|s| s.split(';').map(|c| c.trim().to_string()).filter(|c| c.len() == 8).collect()).unwrap_or_default();
        out.insert(cell, Product { edition: n(2), update: n(4), location, replaced_by });
    }
    out
}

/// STATUS.LST (update media): the base media the update goes with: (number, date, text).
fn read_status(root: &Path) -> Vec<(String, String, String)> {
    let Some(p) = find_info(root, "STATUS.LST") else { return vec![] };
    let text = String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).to_string();
    let mut out = Vec::new();
    for l in text.lines().skip(1) {
        let f: Vec<&str> = l.split(',').map(str::trim).collect();
        if f.len() >= 5 {
            let info = f[3..f.len() - 1].join(",").trim_matches(|c| c == '\'' || c == '’' || c == '‘').to_string();
            out.push((f[0].to_string(), f[f.len() - 1].to_string(), info));
        }
    }
    out
}

/// "B01X03" → "B1"
fn base_number(set: &str) -> Option<String> {
    let kind = set.chars().next()?;
    let n: u32 = set.get(1..3)?.parse().ok()?;
    Some(format!("{kind}{n}"))
}

/// The exchange sets in a folder: it, or the folders under it, that hold ENC_ROOT.
pub fn exchange_sets(dir: &Path) -> Vec<PathBuf> {
    let has_root = |d: &Path| std::fs::read_dir(d).into_iter().flatten().flatten().any(|e| e.path().is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case("ENC_ROOT"));
    if dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("ENC_ROOT")) {
        return dir.parent().map(|p| vec![p.to_path_buf()]).unwrap_or_default();
    }
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        if has_root(&d) {
            out.push(d);
            continue;
        }
        if depth < 4 {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    stack.push((e.path(), depth + 1));
                }
            }
        }
    }
    out.sort();
    out
}

impl Senc {
    /// The installed ENCs in `enc_dir/.senc`, with the IHO key installed the first time.
    pub fn open(enc_dir: &Path) -> Senc {
        let mut s = Senc::open_bare(enc_dir);
        if !s.state.key_set {
            s.install_key("IHO.CRT", IHO_CRT);
        }
        s
    }

    /// As [`Senc::open`] but without installing the IHO key (for the S-64 tests, which
    /// start with no key installed).
    pub fn open_bare(enc_dir: &Path) -> Senc {
        let root = enc_dir.join(".senc");
        let state = std::fs::read_to_string(root.join("state.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Senc { root, state }
    }

    /// Whether anything has been installed in `enc_dir`.
    pub fn exists(enc_dir: &Path) -> bool {
        enc_dir.join(".senc").join("state.json").is_file()
    }

    fn save(&self) {
        let _ = std::fs::create_dir_all(&self.root);
        if let Ok(t) = serde_json::to_string_pretty(&self.state) {
            let tmp = self.root.join("state.json.tmp");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(&tmp, self.root.join("state.json"));
            }
        }
    }

    pub fn key(&self) -> Option<&KeyInfo> {
        self.state.key.as_ref()
    }

    pub fn cells(&self) -> &BTreeMap<String, Installed> {
        &self.state.cells
    }

    /// Install an SA certificate (IHO.CRT, DER) or public key file (IHO.PUB, text) in
    /// place of the one installed. An expired or badly formed one is not installed.
    pub fn install_key(&mut self, name: &str, bytes: &[u8]) -> Vec<Msg> {
        let today = super::today();
        let parsed = if bytes.first() == Some(&0x30) {
            s63::parse_certificate(bytes).map(|c| (c.key, Some(c.not_after), c.subject))
        } else {
            s63::parse_public_key(bytes).and_then(|k| if s63::key_is_sound(&k) { Ok((k, None, String::new())) } else { Err(s63::S63Error::KeyFormat) })
        };
        let (key, not_after, subject) = match parsed {
            Ok(v) => v,
            Err(e) => return vec![msg("", Level::Error, format!("{name} not installed. {e}"))],
        };
        if not_after.as_deref().is_some_and(|d| d < today.as_str()) {
            return vec![msg("", Level::Error, format!("{name} not installed (expired {}). {SSE22}", fmt_date(&not_after.unwrap_or_default())))];
        }
        let sa = key == iho_key();
        self.state.key = Some(KeyInfo { name: name.to_string(), sa, not_after: not_after.clone(), subject, p: hex(&key.p), q: hex(&key.q), g: hex(&key.g), y: hex(&key.y) });
        self.state.key_set = true;
        self.save();
        let until = not_after.map(|d| format!(", valid to {}", fmt_date(&d))).unwrap_or_default();
        if sa {
            vec![msg("", Level::Ok, format!("{name} installed: the IHO Scheme Administrator's key{until}"))]
        } else {
            vec![msg("", Level::Warn, format!("{name} installed{until}, but it is not the IHO's key. {SSE26}"))]
        }
    }

    /// Remove the installed key (for the S-64 tests).
    pub fn remove_key(&mut self) {
        self.state.key = None;
        self.state.key_set = true;
        self.save();
    }

    fn permit_file(&self, ds: &str) -> PathBuf {
        self.root.join("permits").join(format!("{ds}.TXT"))
    }

    /// The installed permits: data server and its permits (each checked against the HW_ID).
    pub fn permits(&self) -> Vec<(String, Vec<(String, Result<CellPermit, s63::S63Error>)>)> {
        let Some(hw) = super::hw_id() else { return vec![] };
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.root.join("permits")).into_iter().flatten().flatten() {
            let p = e.path();
            let Some(ds) = p.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            out.push((ds, s63::read_permits(&text, &hw).unwrap_or_default()));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Each cell's permits from every data server (for decrypting what is installed).
    pub fn cell_permits(&self) -> HashMap<String, Vec<CellPermit>> {
        let mut out: HashMap<String, Vec<CellPermit>> = HashMap::new();
        for (_, v) in self.permits() {
            for (cell, p) in v {
                if let Ok(p) = p {
                    out.entry(cell).or_default().push(p);
                }
            }
        }
        out
    }

    fn ds_permits(&self, ds: &str) -> HashMap<String, CellPermit> {
        let Some(hw) = super::hw_id() else { return HashMap::new() };
        let text = std::fs::read_to_string(self.permit_file(ds)).unwrap_or_default();
        s63::read_permits(&text, &hw).unwrap_or_default().into_iter().filter_map(|(c, p)| p.ok().map(|p| (c, p))).collect()
    }

    /// Install the permits in a PERMIT.TXT: each one goes with its data server's permits,
    /// replacing an older permit for the same cell.
    pub fn install_permits(&mut self, text: &str) -> Vec<Msg> {
        let Some(hw) = super::hw_id() else { return vec![msg("", Level::Error, s63::S63Error::UserPermit.to_string())] };
        let found = match s63::read_permits(text, &hw) {
            Ok(v) if !v.is_empty() => v,
            _ => return vec![msg("", Level::Error, format!("No cell permits in the file. {SSE11}"))],
        };
        let today = super::today();
        let records: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(':')).collect();
        let mut by_ds: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut out = Vec::new();
        for ((cell, r), line) in found.into_iter().zip(records) {
            match r {
                Ok(p) => {
                    let ds = if p.data_server.is_empty() { cell.get(..2).unwrap_or("XX").to_string() } else { p.data_server.clone() };
                    let store = by_ds.entry(ds.clone()).or_insert_with(|| {
                        let old = std::fs::read_to_string(self.permit_file(&ds)).unwrap_or_default();
                        old.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(':')).map(|l| (l.get(..8).unwrap_or(l).to_string(), l.to_string())).collect()
                    });
                    store.insert(cell.clone(), line.to_string());
                    let days = super::days_between(&today, &p.expiry);
                    let valid = format!("valid to {}", fmt_date(&p.expiry));
                    out.push(if days < 0 {
                        msg(&cell, Level::Warn, format!("{valid}, installed. {SSE15}"))
                    } else if days <= 30 {
                        msg(&cell, Level::Warn, format!("{valid}, installed. {SSE20}"))
                    } else {
                        msg(&cell, Level::Ok, format!("{valid}, installed ({ds})"))
                    });
                }
                Err(e) => out.push(msg(&cell, Level::Error, format!("not installed. {e}"))),
            }
        }
        let _ = std::fs::create_dir_all(self.root.join("permits"));
        for (ds, lines) in by_ds {
            let mut t = format!(":DATE {} 00:00\r\n:VERSION 2\r\n:ENC\r\n", today);
            for l in lines.values() {
                t.push_str(l);
                t.push_str("\r\n");
            }
            if std::fs::write(self.permit_file(&ds), t).is_err() {
                out.push(msg("", Level::Error, format!("Could not save the permits for {ds}")));
            }
        }
        out
    }

    /// Remove one data server's permits.
    pub fn remove_permits(&mut self, ds: &str) -> bool {
        std::fs::remove_file(self.permit_file(ds)).is_ok()
    }

    /// Remove every installed cell (the permits and key stay).
    pub fn remove_cells(&mut self) {
        let _ = std::fs::remove_dir_all(self.root.join("cells"));
        self.state.cells.clear();
        self.state.products.clear();
        self.state.bases.clear();
        self.save();
    }

    /// Remove one installed cell.
    pub fn remove_cell(&mut self, cell: &str) {
        let _ = std::fs::remove_dir_all(self.root.join("cells").join(cell));
        self.state.cells.remove(cell);
        self.save();
    }

    /// The installed cells as base and update files, for converting them.
    pub fn cell_files(&self) -> Vec<super::CellFiles> {
        let dir = self.root.join("cells");
        if !dir.is_dir() {
            return vec![];
        }
        super::find_cells(&dir).into_iter().filter(|c| c.base.parent().and_then(|p| p.file_name()).is_some_and(|n| self.state.cells.contains_key(&n.to_string_lossy().to_string()))).collect()
    }

    /// Warnings for showing the installed cells (S-63 10.8): expired permits (SSE 25) and
    /// cells behind their data server's product listing (SSE 27).
    pub fn display_notes(&self) -> Vec<(String, String)> {
        let today = super::today();
        let permits = self.cell_permits();
        let mut out = Vec::new();
        for (cell, c) in &self.state.cells {
            if c.encrypted {
                if let Some(ps) = permits.get(cell) {
                    if !ps.is_empty() && ps.iter().all(|p| p.expiry.as_str() < today.as_str()) {
                        out.push((cell.clone(), format!("SSE 25 - The permit for ENC {cell} has expired. This cell may be out of date and MUST NOT be used for primary navigation")));
                    }
                }
            }
            if let Some(p) = self.state.products.get(&c.ds).and_then(|m| m.get(cell)) {
                if p.edition > c.edition || (p.edition == c.edition && p.update > c.update) {
                    out.push((cell.clone(), sse27(cell)));
                }
            }
        }
        out
    }

    /// Import the exchange sets in a folder (a set with ENC_ROOT in it, or a medium with
    /// several), telling `progress(done, total, cell)` as it goes. Returns the report.
    pub fn import(&mut self, dir: &Path, progress: &mut dyn FnMut(usize, usize, &str)) -> Vec<Msg> {
        let sets = exchange_sets(dir);
        if sets.is_empty() {
            return vec![msg("", Level::Error, format!("No exchange set (a folder with ENC_ROOT in it) in {}", dir.display()))];
        }
        let mut out = Vec::new();
        for set in sets {
            if let Err(e) = std::fs::create_dir_all(self.root.join("cells")) {
                out.push(msg("", Level::Error, format!("Can't write to {}: {e}", self.root.display())));
                break;
            }
            self.import_set(&set, &mut out, progress);
            self.save();
        }
        out
    }

    fn import_set(&mut self, set: &Path, out: &mut Vec<Msg>, progress: &mut dyn FnMut(usize, usize, &str)) {
        let today = super::today();
        let serial = read_serial(set).unwrap_or_default();
        let set_name = set.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let label = if serial.ds.is_empty() { set_name.clone() } else { format!("{set_name} ({} {} {})", serial.ds, serial.kind, fmt_date(&serial.date)) };
        out.push(msg("", Level::Info, format!("Exchange set {label}")));
        let Some(enc_root) = std::fs::read_dir(set).ok().and_then(|rd| rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("ENC_ROOT")).map(|e| e.path())) else { return };
        let files = files_under(&enc_root);
        let catalogue = read_catalogue(&enc_root, &files);
        let entries: Vec<Entry> = match &catalogue {
            Some(c) => c.clone(),
            None => {
                out.push(msg("", Level::Warn, "No CATALOG.031: the files' CRCs can't be checked"));
                files
                    .values()
                    .filter_map(|p| {
                        let (cell, ext) = cell_file(&p.file_name()?.to_string_lossy())?;
                        Some(Entry { path: p.clone(), cell, ext, edition: None, update: None, crc: None })
                    })
                    .collect()
            }
        };
        let products = read_products(set);
        let mut by_cell: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
        for e in entries {
            by_cell.entry(e.cell.clone()).or_default().push(e);
        }
        let encrypted_set = by_cell.values().flatten().any(|e| std::fs::read(&e.path).is_ok_and(|d| !is_plain(&d)));
        let ds = if serial.ds.is_empty() { by_cell.keys().next().and_then(|c| c.get(..2)).unwrap_or("").to_string() } else { serial.ds.clone() };

        // the SA key, and the data server's permits
        let key = if encrypted_set {
            match self.state.key.clone() {
                None => {
                    for c in by_cell.keys() {
                        out.push(msg(c, Level::Error, format!("not installed. {SSE05}")));
                    }
                    return;
                }
                Some(k) if k.not_after.as_deref().is_some_and(|d| d < today.as_str()) => {
                    for c in by_cell.keys() {
                        out.push(msg(c, Level::Error, format!("not installed. {SSE22}. {SSE05}")));
                    }
                    return;
                }
                Some(k) => Some(k),
            }
        } else {
            None
        };
        let sa_key = key.as_ref().and_then(|k| k.key());
        let permits = self.ds_permits(&ds);
        // other data servers' permits for the same cells are tried after this one's
        let all_permits = self.cell_permits();
        if encrypted_set && permits.is_empty() {
            out.push(msg("", Level::Warn, format!("Data server {ds}: {SSE10}")));
        }
        if encrypted_set && key.as_ref().is_some_and(|k| !k.sa) {
            out.push(msg("", Level::Warn, format!("{SSE26} (the installed key is {})", key.as_ref().map(|k| k.name.as_str()).unwrap_or(""))));
        }

        // an update that goes with newer base media than is installed (STATUS.LST)
        if serial.kind.eq_ignore_ascii_case("UPDATE") {
            let licensed: Vec<&Product> = products.iter().filter(|(c, _)| permits.contains_key(*c)).map(|(_, p)| p).collect();
            let mut need = Vec::new();
            for (num, date, info) in read_status(set) {
                let used = licensed.iter().any(|p| p.location == num || p.location.starts_with(&format!("{num};")) || p.location.ends_with(&format!(";{num}")));
                let have = self.state.bases.get(&ds).and_then(|b| b.get(&num));
                if used && have.is_none_or(|h| h.as_str() < date.as_str()) {
                    need.push(format!("'{info}'"));
                }
            }
            if !need.is_empty() {
                out.push(msg("", Level::Warn, format!("This 'Update Media' is not compatible with the actual installed 'Base Media'. Please install the following 'Base Media' first and then continue with the 'Update Media': {}", need.join(", "))));
            }
        }

        let total = by_cell.len();
        let mut no_permit = Vec::new();
        let mut expired = Vec::new();
        let mut expiring = Vec::new();
        for (i, (cell, mut files)) in by_cell.into_iter().enumerate() {
            progress(i, total, &cell);
            let first = files.first().map(|e| std::fs::read(&e.path).map(|d| !is_plain(&d)).unwrap_or(true)).unwrap_or(false);
            let mut cell_permits: Vec<CellPermit> = permits.get(&cell).cloned().into_iter().collect();
            for p in all_permits.get(&cell).into_iter().flatten() {
                if !cell_permits.contains(p) {
                    cell_permits.push(p.clone());
                }
            }
            if first && cell_permits.is_empty() {
                no_permit.push(cell.clone());
                continue;
            }
            if let Some(p) = cell_permits.first() {
                let days = super::days_between(&today, &p.expiry);
                if days < 0 {
                    expired.push(cell.clone());
                } else if days <= 30 {
                    expiring.push(cell.clone());
                }
            }
            // edition and update of each file: CATD-COMT, else the folders (<edition>/<update>/)
            for e in files.iter_mut() {
                let up = |p: &Path, n: usize| p.ancestors().nth(n).and_then(|d| d.file_name()).and_then(|s| s.to_string_lossy().parse::<u32>().ok());
                if e.update.is_none() {
                    e.update = up(&e.path, 1).or(Some(e.ext));
                }
                if e.edition.is_none() {
                    e.edition = up(&e.path, 2);
                }
            }
            files.sort_by_key(|e| (e.edition == Some(0), e.edition.unwrap_or(0), e.ext));
            self.import_cell(&cell, &ds, &files, &cell_permits, sa_key.as_ref(), &products, out);
        }
        progress(total, total, "");
        if !no_permit.is_empty() {
            out.push(msg("", Level::Info, format!("{} cell{} not installed, no permit: {}", no_permit.len(), if no_permit.len() == 1 { "" } else { "s" }, list(&no_permit))));
        }
        if !expired.is_empty() {
            out.push(msg("", Level::Warn, format!("{SSE15}: {}", list(&expired))));
        }
        if !expiring.is_empty() {
            out.push(msg("", Level::Warn, format!("{SSE20}: {}", list(&expiring))));
        }

        // the product listing: merged with what is known; cancelled and out-of-date cells
        let known = self.state.products.entry(ds.clone()).or_default();
        for (c, p) in &products {
            known.insert(c.clone(), p.clone());
        }
        let mut cancelled = Vec::new();
        for (c, inst) in &self.state.cells {
            if inst.ds != ds && !products.contains_key(c) {
                continue;
            }
            let Some(p) = products.get(c) else { continue };
            if p.edition == 0 {
                cancelled.push(c.clone());
            } else if p.edition > inst.edition || (p.edition == inst.edition && p.update > inst.update) {
                out.push(msg(c, Level::Warn, sse27(c)));
            }
        }
        for c in cancelled {
            let rep = products.get(&c).map(|p| p.replaced_by.clone()).unwrap_or_default();
            out.push(msg(&c, Level::Warn, cancelled_text(&c, &rep)));
            self.remove_cell(&c);
        }
        if serial.kind.eq_ignore_ascii_case("BASE") {
            let bases = self.state.bases.entry(ds).or_default();
            if let Some(b) = base_number(&serial.set) {
                bases.insert(b, serial.date.clone());
            }
            // a medium of several exchange sets: MEDIA.TXT beside them names it ("M1")
            if let Some(m) = set.parent().and_then(|p| std::fs::read_to_string(p.join("MEDIA.TXT")).ok()) {
                let date = m.lines().next().and_then(|l| l.get(12..20)).unwrap_or(&serial.date).to_string();
                if let Some(id) = m.lines().nth(1).and_then(|l| l.split(',').next()) {
                    bases.insert(id.trim().to_string(), date);
                }
            }
        }
    }

    /// Check one file: its signature (encrypted files), decryption and CRC. Returns the plain data.
    fn check(&self, e: &Entry, permits: &[CellPermit], key: Option<&PublicKey>) -> Result<Vec<u8>, String> {
        let name = e.path.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
        let data = std::fs::read(&e.path).map_err(|err| format!("{name} can't be read: {err}"))?;
        let plain = if is_plain(&data) {
            data
        } else {
            let sig_name = s63::signature_name(&name).unwrap_or_default();
            let sig = std::fs::read_dir(e.path.parent().unwrap_or(Path::new(".")))
                .ok()
                .and_then(|rd| rd.flatten().find(|f| f.file_name().to_string_lossy().eq_ignore_ascii_case(&sig_name)))
                .and_then(|f| std::fs::read(f.path()).ok())
                .ok_or_else(|| format!("{name}: {} (no signature file {sig_name})", s63::S63Error::SignatureFormat))?;
            let key = key.ok_or_else(|| format!("{name}: {SSE05}"))?;
            s63::verify_cell(&sig, &data, key).map_err(|err| format!("{name}: {err}"))?;
            if permits.is_empty() {
                return Err(format!("{name}: {SSE11}"));
            }
            permits.iter().find_map(|p| s63::decrypt_cell(&data, p).ok()).ok_or_else(|| format!("{name}: {}", s63::S63Error::Decrypt))?
        };
        if let Some(crc) = e.crc {
            if crc32fast::hash(&plain) != crc {
                return Err(format!("{name}: SSE 16 - ENC {} CRC value is incorrect. Contact your data supplier as ENC(s) may be corrupted or missing data", e.cell));
            }
        }
        Ok(plain)
    }

    #[allow(clippy::too_many_arguments)]
    fn import_cell(&mut self, cell: &str, ds: &str, files: &[Entry], permit: &[CellPermit], key: Option<&PublicKey>, products: &BTreeMap<String, Product>, out: &mut Vec<Msg>) {
        let before = self.state.cells.get(cell).cloned();
        let mut cur = before.clone();
        let mut already = false;
        let mut failed: Option<String> = None;
        let dest = self.root.join("cells").join(cell);
        for (i, e) in files.iter().enumerate() {
            let ed = e.edition.unwrap_or(0);
            let up = e.update.unwrap_or(e.ext);
            if ed == 0 {
                // a cancellation (an update with edition 0)
                if let Err(err) = self.check(e, permit, key) {
                    failed = Some(err);
                    break;
                }
                let rep = products.get(cell).map(|p| p.replaced_by.clone()).unwrap_or_default();
                out.push(msg(cell, Level::Warn, cancelled_text(cell, &rep)));
                self.remove_cell(cell);
                return;
            }
            if e.ext == 0 {
                if cur.as_ref().is_some_and(|c| ed < c.edition || (ed == c.edition && up <= c.update)) {
                    already = true;
                    continue;
                }
                match self.check(e, permit, key) {
                    Ok(plain) => {
                        let (ed, up) = if e.edition.is_some() { (ed, up) } else { dsid(&plain).unwrap_or((ed, up)) };
                        let encrypted = !is_plain(&std::fs::read(&e.path).unwrap_or_default());
                        // a new cell, new edition or re-issue replaces what was there
                        let _ = std::fs::remove_dir_all(&dest);
                        if let Err(err) = copy_with(&e.path, &dest) {
                            failed = Some(format!("can't be copied: {err}"));
                            break;
                        }
                        cur = Some(Installed { ds: ds.to_string(), edition: ed, update: up, encrypted });
                    }
                    Err(err) => {
                        let later = files[i + 1..].iter().any(|f| f.ext > 0 && f.edition == e.edition);
                        failed = Some(if later { format!("{err}. Its updates are not installed: {SSE23}") } else { err });
                        break;
                    }
                }
            } else {
                let Some(c) = cur.as_mut() else {
                    failed = Some(format!("update {} without its base cell. {SSE23}", e.ext));
                    break;
                };
                if ed < c.edition {
                    continue;
                }
                if ed > c.edition {
                    failed = Some(format!("update {} is for edition {ed}, edition {} is installed. {SSE23}", e.ext, c.edition));
                    break;
                }
                if e.ext <= c.update {
                    already = true;
                    continue;
                }
                if e.ext > c.update + 1 {
                    failed = Some(format!("update {} {SSE23}", c.update + 1));
                    break;
                }
                match self.check(e, permit, key) {
                    Ok(_) => {
                        if let Err(err) = copy_with(&e.path, &dest) {
                            failed = Some(format!("can't be copied: {err}"));
                            break;
                        }
                        c.update = e.ext;
                        c.ds = ds.to_string();
                    }
                    Err(err) => {
                        // the later updates can't go on without this one
                        let later = files[i + 1..].iter().any(|f| f.ext > e.ext && f.edition == e.edition);
                        failed = Some(if later { format!("{err}. Updates after {} not installed: {SSE23}", e.ext) } else { err });
                        break;
                    }
                }
            }
        }
        if let Some(c) = &cur {
            self.state.cells.insert(cell.to_string(), c.clone());
        }
        let state = |c: &Installed| format!("edition {} update {}", c.edition, c.update);
        match (&before, &cur, failed) {
            (_, None, Some(err)) => out.push(msg(cell, Level::Error, format!("not installed. {err}"))),
            (b, Some(c), Some(err)) => {
                let what = if b.as_ref() == Some(c) { format!("stays at {}", state(c)) } else { format!("{} installed", state(c)) };
                out.push(msg(cell, Level::Error, format!("{what}. {err}")));
            }
            (b, Some(c), None) if b.as_ref() == Some(c) => out.push(msg(cell, Level::Info, format!("already installed ({})", state(c)))),
            (_, Some(c), None) => out.push(msg(cell, Level::Ok, format!("{} installed{}", state(c), if already { " (some files already installed)" } else { "" }))),
            (_, None, None) => {}
        }
    }
}

fn sse27(cell: &str) -> String {
    format!("SSE 27 - ENC {cell} is not up to date. A New Edition, Re-issue or Update for this cell is missing and therefore MUST NOT be used for primary navigation")
}

fn cancelled_text(cell: &str, rep: &[String]) -> String {
    if rep.is_empty() {
        format!("Cell {cell} has been cancelled and has been removed")
    } else {
        format!("Cell {cell} has been cancelled and has been replaced by cell(s) {}. Please contact your data supplier to obtain the additional ENC permits", rep.join("; "))
    }
}

fn list(v: &[String]) -> String {
    if v.len() <= 20 {
        v.join(", ")
    } else {
        format!("{} and {} more", v[..20].join(", "), v.len() - 20)
    }
}

fn fmt_date(d: &str) -> String {
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    match (d.get(..4), d.get(4..6).and_then(|m| m.parse::<usize>().ok()), d.get(6..8).and_then(|x| x.parse::<u32>().ok())) {
        (Some(y), Some(m), Some(day)) if (1..=12).contains(&m) => format!("{day} {} {y}", M[m - 1]),
        _ => d.to_string(),
    }
}

/// Copy a cell file into the cell's folder with its signature file and the text and
/// picture files beside it (exchange sets keep each cell file in a folder of its own).
fn copy_with(file: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    let dir = file.parent().unwrap_or(Path::new("."));
    let name = file.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
    let sig = s63::signature_name(&name).unwrap_or_default();
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().to_ascii_uppercase();
        let ours = n == name || n == sig;
        // not another cell's file, nor the catalogue
        let extra = !cell_file_like(&n) && n != "CATALOG.031" && n != "README.TXT";
        if p.is_file() && (ours || extra) {
            std::fs::copy(&p, dest.join(&n))?;
        }
    }
    Ok(())
}

/// A cell or signature file name (8 characters and a 3-digit extension).
fn cell_file_like(n: &str) -> bool {
    n.split_once('.').is_some_and(|(s, e)| s.len() == 8 && e.len() == 3 && e.bytes().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_an_exchange_set() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/s63");
        let dir = std::env::temp_dir().join(format!("senc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        super::super::set_today(Some("20120601"));
        let mut s = Senc::open(&dir);
        assert!(s.key().is_some_and(|k| k.sa));
        // no permits yet: the data server has none
        let r = s.import(&src, &mut |_, _, _| {});
        assert!(r.iter().any(|m| m.text.contains("SSE 10")), "{r:?}");
        let r = s.install_permits(&std::fs::read_to_string(src.join("PERMIT.TXT")).unwrap());
        assert!(r.iter().all(|m| m.level == Level::Ok), "{r:?}");
        let r = s.import(&src, &mut |_, _, _| {});
        assert!(r.iter().any(|m| m.cell == "GB61021B" && m.level == Level::Ok), "{r:?}");
        assert_eq!(s.cells()["GB61021B"].update, 1);
        // importing it again changes nothing
        let r = s.import(&src, &mut |_, _, _| {});
        assert!(r.iter().any(|m| m.cell == "GB61021B" && m.text.starts_with("already")), "{r:?}");
        // and it converts from the store
        let files = s.cell_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].updates.len(), 1);
        assert!(!super::super::ensure_gpkgs(&dir).is_empty());
        super::super::set_today(None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_iho_certificate() {
        let c = s63::parse_certificate(IHO_CRT).unwrap();
        assert_eq!(c.not_after, "20330303");
        assert!(c.subject.contains("International Hydrographic Organization"));
        assert_eq!(c.key, s63::parse_public_key(super::super::SA_PUBLIC_KEY.as_bytes()).unwrap());
    }
}
