//! TweedPlot Licence Manager: Tweedsmarine's side of S-63 for TweedPlot. Not shipped.
//!
//! It keeps the manufacturer key (M_KEY), the key activation codes are signed with, the
//! customers, licences and seats, and the HW_ID register, all in one database sealed on
//! disk (the vault). The vault's key is kept with Windows DPAPI under the Windows user
//! who made it, so the vault file is useless anywhere else; a passphrase backup is how it
//! moves to a new PC.
//!
//! One licence is one vessel: one HW_ID and one user permit, used on up to three
//! computers on board (seats). Each seat is one installation of TweedPlot, activated with
//! its own activation code made from its installation code. A seat is given back with
//! the release code the app shows when its activation is removed.
//!
//! The OEM folder is `TWEEDPLOT_OEM_DIR`, else `%USERPROFILE%\.tweedplot-oem`:
//! - `vault.key`    the vault's key, DPAPI-protected (user scope)
//! - `licences.tpv` the vault (`licences-test.tpv` in test mode)
//! - `history\`     the last 20 versions of the vault, sealed the same way
//! - `oem.txt`      `M_ID=..` (read when TweedPlot is built; no secrets)
//! - `signing.pub`  the public key built into TweedPlot (no secrets)

use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit};
use chart::enc::{device, oem, s63};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use zeroize::Zeroizing;

pub const SEATS: u32 = 3;
const VAULT_MAGIC: &[u8; 4] = b"TPV1";
const BACKUP_MAGIC: &[u8; 5] = b"TPLB1";
const KEY_ENTROPY: &[u8] = b"TweedPlot Licence Manager vault 1";
const HISTORY: usize = 20;

pub type Result<T> = std::result::Result<T, String>;

/// The OEM folder.
pub fn oem_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TWEEDPLOT_OEM_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".tweedplot-oem")
}

fn today() -> String {
    chrono::Utc::now().format("%Y%m%d").to_string()
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn seal(key: &[u8; 32], magic: &[u8], extra: &[u8], plain: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).expect("random numbers");
    let c = chacha20poly1305::ChaCha20Poly1305::new(key.into());
    let body = c.encrypt(&nonce.into(), plain).expect("encrypt");
    let mut out = magic.to_vec();
    out.extend_from_slice(extra);
    out.extend_from_slice(&nonce);
    out.extend(body);
    out
}

fn unseal(key: &[u8; 32], data: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if data.len() < 12 {
        return None;
    }
    let c = chacha20poly1305::ChaCha20Poly1305::new(key.into());
    c.decrypt(data[..12].into(), &data[12..]).ok().map(Zeroizing::new)
}

fn passphrase_key(pass: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let mut k = Zeroizing::new([0u8; 32]);
    argon2::Argon2::default().hash_password_into(pass.as_bytes(), salt, &mut k[..]).map_err(|e| format!("passphrase: {e}"))?;
    Ok(k)
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS settings (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL DEFAULT '',
  phone TEXT NOT NULL DEFAULT '', notes TEXT NOT NULL DEFAULT '', created TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS licences (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id),
  vessel TEXT NOT NULL, mmsi TEXT NOT NULL DEFAULT '', call_sign TEXT NOT NULL DEFAULT '', hw_id TEXT NOT NULL,
  user_permit TEXT NOT NULL, seats INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'active', created TEXT NOT NULL,
  notes TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS seats (id INTEGER PRIMARY KEY, licence_id INTEGER NOT NULL REFERENCES licences(id), seat INTEGER NOT NULL,
  fingerprint TEXT NOT NULL, device TEXT NOT NULL, installation_code TEXT NOT NULL, label TEXT NOT NULL DEFAULT '',
  activation_code TEXT NOT NULL, issued TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'active',
  ended TEXT NOT NULL DEFAULT '', end_reason TEXT NOT NULL DEFAULT '', release_code TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS log (id INTEGER PRIMARY KEY, at TEXT NOT NULL, action TEXT NOT NULL,
  licence_id INTEGER, seat INTEGER, detail TEXT NOT NULL DEFAULT '');
";

/// The vault: the database, open in memory, sealed back to disk after every change.
pub struct Vault {
    dir: PathBuf,
    test: bool,
    key: Zeroizing<[u8; 32]>,
    db: Connection,
    /// whether the vault key is protected with DPAPI (false off Windows)
    pub protected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub dir: String,
    pub test: bool,
    pub protected: bool,
    pub has_credentials: bool,
    pub m_id: String,
    /// whether signing.pub in the OEM folder is the vault's signing key
    pub pub_matches: bool,
    /// whether this tool was built with the same signing key (TweedPlot builds from this PC)
    pub build_matches: bool,
    pub build_real: bool,
    pub last_backup: String,
    pub changes_since_backup: i64,
    pub licences: i64,
    pub active_seats: i64,
    /// files left by the old command-line tool that hold secrets
    pub old_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Customer {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Seat {
    pub id: i64,
    pub seat: u32,
    pub label: String,
    pub installation_code: String,
    pub fingerprint: String,
    pub activation_code: String,
    pub issued: String,
    pub status: String,
    pub ended: String,
    pub end_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Licence {
    pub id: i64,
    pub customer: Customer,
    pub vessel: String,
    pub mmsi: String,
    pub call_sign: String,
    pub user_permit: String,
    pub seats: u32,
    pub active: u32,
    pub status: String,
    pub created: String,
    pub notes: String,
    pub seat_list: Vec<Seat>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub at: String,
    pub action: String,
    pub licence_id: Option<i64>,
    pub seat: Option<i64>,
    pub detail: String,
}

/// A new licence's details.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct NewLicence {
    /// an existing customer, else `customer_name` etc. make a new one
    pub customer_id: Option<i64>,
    #[serde(default)]
    pub customer_name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    pub vessel: String,
    #[serde(default)]
    pub mmsi: String,
    #[serde(default)]
    pub call_sign: String,
    pub seats: Option<u32>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct IssueRequest {
    pub installation_code: String,
    /// an existing licence, else `new_licence`
    pub licence_id: Option<i64>,
    pub new_licence: Option<NewLicence>,
    #[serde(default)]
    pub label: String,
    /// going over the licence's seats: why (logged)
    pub over_limit_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issued {
    pub licence_id: i64,
    pub seat: u32,
    pub activation_code: String,
    pub user_permit: String,
    /// "new seat", "same installation again", "reinstalled on the same computer"
    pub how: String,
    pub warnings: Vec<String>,
    pub email_subject: String,
    pub email_body: String,
    pub email_to: String,
}

/// What an installation code says, and where it is already in use.
#[derive(Debug, Clone, Serialize)]
pub struct CodeInfo {
    pub fingerprint: String,
    pub in_use: Vec<(i64, String, u32, String)>,
}

impl Vault {
    fn vault_file(&self) -> PathBuf {
        self.dir.join(if self.test { "licences-test.tpv" } else { "licences.tpv" })
    }

    /// Open (or make) the vault in `dir`. `test`: the IHO test credentials and a separate
    /// test database, for trying things out with test builds of TweedPlot.
    pub fn open(dir: &Path, test: bool) -> Result<Vault> {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
        let (key, protected) = vault_key(dir)?;
        let mut v = Vault { dir: dir.to_path_buf(), test, key, db: Connection::open_in_memory().map_err(|e| e.to_string())?, protected };
        let f = v.vault_file();
        if f.is_file() {
            let data = std::fs::read(&f).map_err(|e| e.to_string())?;
            if data.len() < 4 || &data[..4] != VAULT_MAGIC {
                return Err(format!("{} isn't a Licence Manager vault", f.display()));
            }
            let plain = unseal(&v.key, &data[4..]).ok_or_else(|| {
                format!("{} can't be opened with this PC's vault key (made by another Windows user or PC?): restore a backup instead", f.display())
            })?;
            let n = plain.len();
            v.db.deserialize_read_exact("main", std::io::Cursor::new(&plain[..]), n, false).map_err(|e| e.to_string())?;
        }
        v.db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        if test {
            v.set("m_id", "10")?;
            v.set("m_key", s63_test_m_key())?;
            v.set("signing_key", oem::TEST_SIGNING_KEY)?;
        }
        Ok(v)
    }

    fn get(&self, k: &str) -> Option<String> {
        self.db.query_row("SELECT v FROM settings WHERE k = ?1", [k], |r| r.get(0)).optional().ok().flatten()
    }

    fn set(&self, k: &str, v: &str) -> Result<()> {
        self.db.execute("INSERT INTO settings (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v", [k, v]).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn log(&self, action: &str, licence: Option<i64>, seat: Option<u32>, detail: &str) -> Result<()> {
        self.db
            .execute("INSERT INTO log (at, action, licence_id, seat, detail) VALUES (?1, ?2, ?3, ?4, ?5)", params![now(), action, licence, seat, detail])
            .map_err(|e| e.to_string())?;
        let n: i64 = self.get("changes").and_then(|s| s.parse().ok()).unwrap_or(0);
        self.set("changes", &(n + 1).to_string())
    }

    /// Seal the database to disk (and keep the version it replaces in `history`).
    fn save(&self) -> Result<()> {
        let data = Zeroizing::new(self.db.serialize("main").map_err(|e| e.to_string())?.to_vec());
        let sealed = seal(&self.key, VAULT_MAGIC, &[], &data);
        let f = self.vault_file();
        if f.is_file() {
            let h = self.dir.join("history");
            let _ = std::fs::create_dir_all(&h);
            let stem = f.file_stem().unwrap().to_string_lossy().to_string();
            let _ = std::fs::copy(&f, h.join(format!("{stem}-{}.tpv", chrono::Utc::now().format("%Y%m%d-%H%M%S%3f"))));
            let mut old: Vec<PathBuf> = std::fs::read_dir(&h)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&format!("{stem}-"))))
                .collect();
            old.sort();
            while old.len() > HISTORY {
                let _ = std::fs::remove_file(old.remove(0));
            }
        }
        device::write_private(&f, &sealed)
    }

    fn m_key(&self) -> Result<Zeroizing<String>> {
        self.get("m_key").map(Zeroizing::new).ok_or_else(|| "No M_KEY yet: enter the credentials from the IHO in Settings".into())
    }

    fn signing_key(&self) -> Result<Zeroizing<[u8; 32]>> {
        let k = self.get("signing_key").ok_or("No signing key yet: enter the credentials from the IHO in Settings")?;
        let b: [u8; 32] = oem::unhex(&k).and_then(|v| v.try_into().ok()).ok_or("the signing key in the vault is damaged")?;
        Ok(Zeroizing::new(b))
    }

    fn public_key(&self) -> Option<[u8; 32]> {
        self.signing_key().ok().map(|k| ed25519_dalek::SigningKey::from_bytes(&k).verifying_key().to_bytes())
    }

    fn m_id(&self) -> String {
        self.get("m_id").unwrap_or_default()
    }

    pub fn status(&self) -> Status {
        let count = |sql: &str| self.db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
        let public = self.public_key();
        let on_disk = std::fs::read_to_string(self.dir.join("signing.pub")).ok().and_then(|t| oem::unhex(&t)).and_then(|v| <[u8; 32]>::try_from(v).ok());
        let mut old = Vec::new();
        if self.dir.join("signing.key").is_file() {
            old.push("signing.key".to_string());
        }
        if std::fs::read_to_string(self.dir.join("oem.txt")).is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("M_KEY"))) {
            old.push("oem.txt (M_KEY line)".to_string());
        }
        if self.dir.join("register.csv").is_file() {
            old.push("register.csv".to_string());
        }
        Status {
            dir: self.dir.display().to_string(),
            test: self.test,
            protected: self.protected,
            has_credentials: self.get("m_key").is_some() && public.is_some(),
            m_id: self.m_id(),
            pub_matches: self.test || (public.is_some() && public == on_disk),
            build_matches: public == Some(oem::public_key()),
            build_real: oem::is_real(),
            last_backup: self.get("last_backup").unwrap_or_default(),
            changes_since_backup: self.get("changes").and_then(|s| s.parse().ok()).unwrap_or(0),
            licences: count("SELECT COUNT(*) FROM licences"),
            active_seats: count("SELECT COUNT(*) FROM seats WHERE status = 'active'"),
            old_files: if self.test { vec![] } else { old },
        }
    }

    /// Put in the credentials from the IHO (real mode). A signing key is made the first
    /// time; `signing.pub` and `oem.txt` (M_ID only) are written for TweedPlot's build.
    pub fn set_credentials(&mut self, m_id: &str, m_key: &str) -> Result<()> {
        if self.test {
            return Err("Test mode uses the IHO test credentials".into());
        }
        let (m_id, m_key) = (m_id.trim(), m_key.trim());
        if m_id.len() != 2 || !m_id.is_ascii() || m_id.contains(['#', '=', ' ']) {
            return Err("The M_ID is two characters".into());
        }
        if m_key.len() != 5 || !m_key.is_ascii() {
            return Err("The M_KEY is five characters".into());
        }
        if m_id == "10" && m_key == s63_test_m_key() {
            return Err("Those are the IHO test credentials: use test mode for them".into());
        }
        if let Some(old) = self.get("m_key") {
            if old != m_key && self.count_licences() > 0 {
                return Err("This vault already has licences made with another M_KEY. Their user permits would stop matching: not changed".into());
            }
        }
        self.set("m_id", m_id)?;
        self.set("m_key", m_key)?;
        if self.get("signing_key").is_none() {
            let mut sk = Zeroizing::new([0u8; 32]);
            getrandom::fill(&mut sk[..]).map_err(|e| e.to_string())?;
            self.set("signing_key", &hex(&sk[..]))?;
        }
        self.write_build_files()?;
        self.log("credentials", None, None, &format!("M_ID {m_id} and M_KEY set"))?;
        self.save()
    }

    /// signing.pub and oem.txt for building TweedPlot (no secrets in either).
    pub fn write_build_files(&self) -> Result<()> {
        let public = self.public_key().ok_or("no signing key")?;
        std::fs::write(self.dir.join("signing.pub"), hex(&public)).map_err(|e| e.to_string())?;
        std::fs::write(
            self.dir.join("oem.txt"),
            format!(
                "# TweedPlot's M_ID, built into TweedPlot. Written by the Licence Manager: the M_KEY is in its vault, not here.\r\nM_ID={}\r\n",
                self.m_id()
            ),
        )
        .map_err(|e| e.to_string())
    }

    fn count_licences(&self) -> i64 {
        self.db.query_row("SELECT COUNT(*) FROM licences", [], |r| r.get(0)).unwrap_or(0)
    }

    /// Move what the old command-line tool left in the OEM folder into the vault: the
    /// M_KEY in oem.txt, signing.key, and register.csv (as licences without seats). The
    /// plain files are then overwritten and removed.
    pub fn import_old_files(&mut self) -> Result<Vec<String>> {
        if self.test {
            return Err("Switch to your own credentials first".into());
        }
        let mut done = Vec::new();
        let get = |text: &str, k: &str| {
            text.lines().find_map(|l| l.trim().strip_prefix(k).and_then(|v| v.trim_start().strip_prefix('=')).map(|v| v.trim().to_string()))
        };
        let oem_txt = std::fs::read_to_string(self.dir.join("oem.txt")).unwrap_or_default();
        if let (Some(id), Some(key)) = (get(&oem_txt, "M_ID"), get(&oem_txt, "M_KEY")) {
            if self.get("m_key").is_none() {
                self.set("m_id", &id)?;
                self.set("m_key", &key)?;
                done.push("M_ID and M_KEY from oem.txt".to_string());
            }
        }
        let sk = self.dir.join("signing.key");
        if let Ok(t) = std::fs::read_to_string(&sk) {
            if self.get("signing_key").is_none() && oem::unhex(&t).is_some_and(|v| v.len() == 32) {
                self.set("signing_key", t.trim())?;
                done.push("signing.key".to_string());
            }
        }
        let reg = self.dir.join("register.csv");
        if let Ok(t) = std::fs::read_to_string(&reg) {
            for l in t.lines().skip(1).filter(|l| !l.trim().is_empty()) {
                let f: Vec<&str> = l.split(',').map(str::trim).collect();
                let (Some(issued), Some(hw), Some(up)) = (f.first(), f.get(1), f.get(2)) else { continue };
                let name = f.get(4).copied().filter(|n| !n.is_empty()).unwrap_or("(from register.csv)");
                if self.db.query_row("SELECT 1 FROM licences WHERE hw_id = ?1", [hw], |_| Ok(())).optional().ok().flatten().is_some() {
                    continue;
                }
                let cid = self.add_customer(name, "", "", "from the old register.csv")?;
                self.db
                    .execute(
                        "INSERT INTO licences (customer_id, vessel, hw_id, user_permit, seats, created, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'from the old register.csv: its activation codes (TPA1) no longer work; issue new ones')",
                        params![cid, name, hw, up, SEATS, issued],
                    )
                    .map_err(|e| e.to_string())?;
            }
            done.push("register.csv".to_string());
        }
        if self.get("m_key").is_some() && self.get("signing_key").is_some() {
            self.write_build_files()?;
            // overwrite then remove the plain secret
            if sk.is_file() {
                let _ = std::fs::write(&sk, vec![0u8; 64]);
                let _ = std::fs::remove_file(&sk);
            }
            if reg.is_file() {
                let _ = std::fs::rename(&reg, self.dir.join("register-imported.csv"));
            }
        }
        self.log("import", None, None, &format!("from the old tool's files: {}", done.join(", ")))?;
        self.save()?;
        Ok(done)
    }

    fn add_customer(&self, name: &str, email: &str, phone: &str, notes: &str) -> Result<i64> {
        if name.trim().is_empty() {
            return Err("The customer needs a name".into());
        }
        self.db
            .execute(
                "INSERT INTO customers (name, email, phone, notes, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![name.trim(), email.trim(), phone.trim(), notes, today()],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn customers(&self) -> Vec<Customer> {
        let mut st = self.db.prepare("SELECT id, name, email, phone, notes FROM customers ORDER BY name COLLATE NOCASE").unwrap();
        st.query_map([], |r| Ok(Customer { id: r.get(0)?, name: r.get(1)?, email: r.get(2)?, phone: r.get(3)?, notes: r.get(4)? }))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn update_customer(&self, c: &Customer) -> Result<()> {
        if c.name.trim().is_empty() {
            return Err("The customer needs a name".into());
        }
        self.db
            .execute(
                "UPDATE customers SET name = ?2, email = ?3, phone = ?4, notes = ?5 WHERE id = ?1",
                params![c.id, c.name.trim(), c.email.trim(), c.phone.trim(), c.notes],
            )
            .map_err(|e| e.to_string())?;
        self.log("customer", None, None, &format!("customer {} updated", c.name.trim()))?;
        self.save()
    }

    fn seats_of(&self, licence: i64) -> Vec<Seat> {
        let mut st = self
            .db
            .prepare(
                "SELECT id, seat, label, installation_code, fingerprint, activation_code, issued, status, ended, end_reason FROM seats
                 WHERE licence_id = ?1 ORDER BY status = 'active' DESC, seat, id DESC",
            )
            .unwrap();
        st.query_map([licence], |r| {
            Ok(Seat {
                id: r.get(0)?,
                seat: r.get(1)?,
                label: r.get(2)?,
                installation_code: r.get(3)?,
                fingerprint: r.get(4)?,
                activation_code: r.get(5)?,
                issued: r.get(6)?,
                status: r.get(7)?,
                ended: r.get(8)?,
                end_reason: r.get(9)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Every licence (newest first), with its customer and seats.
    pub fn licences(&self) -> Vec<Licence> {
        let ids: Vec<i64> = {
            let mut st = self.db.prepare("SELECT id FROM licences ORDER BY id DESC").unwrap();
            st.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect()
        };
        ids.into_iter().filter_map(|id| self.licence(id).ok()).collect()
    }

    pub fn licence(&self, id: i64) -> Result<Licence> {
        let l = self
            .db
            .query_row(
                "SELECT l.id, l.vessel, l.mmsi, l.call_sign, l.user_permit, l.seats, l.status, l.created, l.notes,
                        c.id, c.name, c.email, c.phone, c.notes
                 FROM licences l JOIN customers c ON c.id = l.customer_id WHERE l.id = ?1",
                [id],
                |r| {
                    Ok(Licence {
                        id: r.get(0)?,
                        vessel: r.get(1)?,
                        mmsi: r.get(2)?,
                        call_sign: r.get(3)?,
                        user_permit: r.get(4)?,
                        seats: r.get(5)?,
                        status: r.get(6)?,
                        created: r.get(7)?,
                        notes: r.get(8)?,
                        customer: Customer { id: r.get(9)?, name: r.get(10)?, email: r.get(11)?, phone: r.get(12)?, notes: r.get(13)? },
                        active: 0,
                        seat_list: vec![],
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("No licence {id}"))?;
        let seat_list = self.seats_of(id);
        let active = seat_list.iter().filter(|s| s.status == "active").count() as u32;
        Ok(Licence { active, seat_list, ..l })
    }

    fn hw_id(&self, licence: i64) -> Result<Zeroizing<[u8; 5]>> {
        let h: String = self.db.query_row("SELECT hw_id FROM licences WHERE id = ?1", [licence], |r| r.get(0)).map_err(|e| e.to_string())?;
        let b: [u8; 5] = h.as_bytes().try_into().map_err(|_| "a HW_ID in the vault is damaged".to_string())?;
        Ok(Zeroizing::new(b))
    }

    fn new_hw_id(&self) -> Result<String> {
        if self.test {
            // the S-64 test data's: test builds can then open the IHO test cells
            return Ok("12345".into());
        }
        loop {
            let mut b = [0u8; 4];
            getrandom::fill(&mut b).map_err(|e| e.to_string())?;
            let id = format!("{:05X}", u32::from_le_bytes(b) & 0xF_FFFF);
            let taken = self.db.query_row("SELECT 1 FROM licences WHERE hw_id = ?1", [&id], |_| Ok(())).optional().map_err(|e| e.to_string())?.is_some();
            if id != "12345" && id != "12348" && !taken {
                return Ok(id);
            }
        }
    }

    pub fn update_licence(&self, id: i64, vessel: &str, mmsi: &str, call_sign: &str, seats: u32, notes: &str) -> Result<()> {
        check_vessel(vessel, mmsi)?;
        if seats == 0 {
            return Err("A licence has at least one seat".into());
        }
        self.db
            .execute(
                "UPDATE licences SET vessel = ?2, mmsi = ?3, call_sign = ?4, seats = ?5, notes = ?6 WHERE id = ?1",
                params![id, vessel.trim(), mmsi.trim(), call_sign.trim(), seats, notes],
            )
            .map_err(|e| e.to_string())?;
        self.log(
            "licence",
            Some(id),
            None,
            &format!(
                "details changed (vessel {}, MMSI {}, {seats} seats); activation codes issued before keep the old vessel details",
                vessel.trim(),
                mmsi.trim()
            ),
        )?;
        self.save()
    }

    /// Where an installation code is already in use: (licence, vessel, seat, status).
    pub fn code_info(&self, code: &str) -> Result<CodeInfo> {
        let inst = oem::read_installation_code(code).ok_or("That isn't a whole installation code (TP2- and 16 groups of five): check it was copied whole")?;
        let mut st = self
            .db
            .prepare("SELECT s.licence_id, l.vessel, s.seat, s.status FROM seats s JOIN licences l ON l.id = s.licence_id WHERE s.fingerprint = ?1 AND s.status = 'active'")
            .unwrap();
        let in_use = st.query_map([hex(&inst.fingerprint)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().filter_map(|r| r.ok()).collect();
        Ok(CodeInfo { fingerprint: hex(&inst.fingerprint), in_use })
    }

    /// Issue an activation code for an installation: on a new licence, or as a seat on an
    /// existing one. The same installation again gets its seat back; a new installation
    /// on a computer already in the licence (TweedPlot reinstalled) takes over that
    /// computer's seat. A seat beyond the licence's number needs a reason.
    pub fn issue(&mut self, req: &IssueRequest) -> Result<Issued> {
        let inst = oem::read_installation_code(&req.installation_code)
            .ok_or("That isn't a whole installation code (TP2- and 16 groups of five): check it was copied whole")?;
        let m_key = self.m_key()?;
        let signing = self.signing_key()?;
        let m_id = self.m_id();
        let mut warnings = Vec::new();
        let fp = hex(&inst.fingerprint);
        let dev = hex(&inst.device);
        self.db.execute_batch("SAVEPOINT issue").map_err(|e| e.to_string())?;
        let r = (|| -> Result<Issued> {
            let licence_id = match (req.licence_id, &req.new_licence) {
                (Some(id), _) => {
                    let l = self.licence(id)?;
                    if l.status != "active" {
                        return Err(format!("Licence {id} is {}: no new seats", l.status));
                    }
                    id
                }
                (None, Some(n)) => {
                    check_vessel(&n.vessel, &n.mmsi)?;
                    let cid = match n.customer_id {
                        Some(c) => c,
                        None => self.add_customer(&n.customer_name, &n.email, &n.phone, "")?,
                    };
                    let hw = self.new_hw_id()?;
                    let hw5: [u8; 5] = hw.as_bytes().try_into().unwrap();
                    let up = s63::make_user_permit(&hw5, &m_key, &oem::hex_of(&m_id));
                    self.db
                        .execute(
                            "INSERT INTO licences (customer_id, vessel, mmsi, call_sign, hw_id, user_permit, seats, created, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                            params![cid, n.vessel.trim(), n.mmsi.trim(), n.call_sign.trim(), hw, up, n.seats.unwrap_or(SEATS).max(1), today(), n.notes],
                        )
                        .map_err(|e| e.to_string())?;
                    let id = self.db.last_insert_rowid();
                    self.log(
                        "new licence",
                        Some(id),
                        None,
                        &format!("{} ({})", n.vessel.trim(), if n.mmsi.trim().is_empty() { "no MMSI" } else { n.mmsi.trim() }),
                    )?;
                    id
                }
                (None, None) => return Err("Choose a licence, or give the new licence's details".into()),
            };
            let l = self.licence(licence_id)?;
            // the same computer on another vessel's licence: allowed (a laptop moved between boats), but said
            for (other, vessel, seat, _) in self.code_info(&req.installation_code)?.in_use {
                if other != licence_id {
                    warnings.push(format!("This computer also has seat {seat} on licence {other} ({vessel}). A licence is for one vessel: release that seat if the computer has moved boats."));
                }
            }
            let active: Vec<&Seat> = l.seat_list.iter().filter(|s| s.status == "active").collect();
            let same_inst = self
                .db
                .query_row("SELECT seat FROM seats WHERE licence_id = ?1 AND device = ?2 AND status = 'active'", params![licence_id, dev], |r| {
                    r.get::<_, u32>(0)
                })
                .optional()
                .map_err(|e| e.to_string())?;
            let same_pc = active.iter().find(|s| s.fingerprint == fp).map(|s| s.seat);
            let (seat, how) = if let Some(s) = same_inst {
                self.end_seats(licence_id, s, "replaced", "issued again for the same installation")?;
                (s, "same installation again".to_string())
            } else if let Some(s) = same_pc {
                self.end_seats(licence_id, s, "replaced", "TweedPlot reinstalled on the same computer")?;
                (s, "reinstalled on the same computer".to_string())
            } else {
                if active.len() as u32 >= l.seats {
                    let why = req.over_limit_reason.as_deref().map(str::trim).filter(|r| !r.is_empty()).ok_or_else(|| {
                        format!(
                            "All {} seats on licence {licence_id} ({}) are in use. Release one with its release code, end one by hand, or give a reason to go over",
                            l.seats, l.vessel
                        )
                    })?;
                    warnings.push(format!("Over the licence's {} seats: {why}", l.seats));
                    self.log("over limit", Some(licence_id), None, why)?;
                }
                let used: Vec<u32> = active.iter().map(|s| s.seat).collect();
                ((1..).find(|n| !used.contains(n)).unwrap(), "new seat".to_string())
            };
            let hw = self.hw_id(licence_id)?;
            let grant = oem::Grant {
                user_permit: l.user_permit.clone(),
                installation: inst,
                issued: today(),
                vessel: l.vessel.clone(),
                mmsi: l.mmsi.clone(),
                licence: licence_id as u32,
                seat: seat as u8,
            };
            let code = oem::make_activation(&grant, &hw, &signing).map_err(|e| e.to_string())?;
            // check it reads back with this key, as TweedPlot will
            let public = ed25519_dalek::SigningKey::from_bytes(&signing).verifying_key().to_bytes();
            oem::read_activation_with(&code, &public).map_err(|e| format!("the new activation code doesn't read back: {e}"))?;
            if !self.test && public != oem::public_key() {
                warnings.push("This Licence Manager was built without the vault's signing.pub: TweedPlot builds from this PC may be the same. Rebuild both after Settings › Credentials.".into());
            }
            self.db
                .execute(
                    "INSERT INTO seats (licence_id, seat, fingerprint, device, installation_code, label, activation_code, issued) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![licence_id, seat, fp, dev, req.installation_code.trim(), req.label.trim(), code, grant.issued],
                )
                .map_err(|e| e.to_string())?;
            self.log(
                "issued",
                Some(licence_id),
                Some(seat),
                &format!("{how}{}", if req.label.trim().is_empty() { String::new() } else { format!(" ({})", req.label.trim()) }),
            )?;
            let (subject, body) = email(&l, seat, &code);
            Ok(Issued {
                licence_id,
                seat,
                activation_code: code,
                user_permit: l.user_permit.clone(),
                how,
                warnings,
                email_subject: subject,
                email_body: body,
                email_to: l.customer.email.clone(),
            })
        })();
        match r {
            Ok(v) => {
                self.db.execute_batch("RELEASE issue").map_err(|e| e.to_string())?;
                self.save()?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.db.execute_batch("ROLLBACK TO issue; RELEASE issue");
                Err(e)
            }
        }
    }

    fn end_seats(&self, licence: i64, seat: u32, status: &str, why: &str) -> Result<()> {
        self.db
            .execute(
                "UPDATE seats SET status = ?3, ended = ?4, end_reason = ?5 WHERE licence_id = ?1 AND seat = ?2 AND status = 'active'",
                params![licence, seat, status, today(), why],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Free a seat with the release code TweedPlot showed when its activation was removed.
    pub fn release(&mut self, code: &str) -> Result<String> {
        let r = oem::read_release(code).ok_or("That isn't a whole release code (TPR2-…): check it was copied whole")?;
        let licence = r.licence as i64;
        let hw = self.hw_id(licence).map_err(|_| format!("The release code is for licence {licence}, which isn't in this vault"))?;
        if !r.verify(&hw) {
            return Err("The release code doesn't check out for that licence: was it copied whole?".into());
        }
        let n = self
            .db
            .execute(
                "UPDATE seats SET status = 'released', ended = ?4, end_reason = 'release code', release_code = ?5
                 WHERE licence_id = ?1 AND seat = ?2 AND fingerprint = ?3 AND status = 'active'",
                params![licence, r.seat, hex(&r.fingerprint), r.date, code.trim()],
            )
            .map_err(|e| e.to_string())?;
        let l = self.licence(licence)?;
        if n == 0 {
            return Ok(format!("Seat {} of licence {licence} ({}) was already free", r.seat, l.vessel));
        }
        self.log("released", Some(licence), Some(r.seat as u32), &format!("release code of {}", r.date))?;
        self.save()?;
        Ok(format!("Seat {} of licence {licence} ({}) is free: {} of {} in use", r.seat, l.vessel, l.active, l.seats))
    }

    /// End a seat without a release code (a computer lost or broken): logged with why.
    pub fn end_seat(&mut self, seat_id: i64, why: &str) -> Result<()> {
        if why.trim().is_empty() {
            return Err("Say why the seat is ended (it goes in the log)".into());
        }
        let (licence, seat): (i64, u32) = self
            .db
            .query_row("SELECT licence_id, seat FROM seats WHERE id = ?1 AND status = 'active'", [seat_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|_| "No such active seat")?;
        self.db
            .execute("UPDATE seats SET status = 'ended', ended = ?2, end_reason = ?3 WHERE id = ?1", params![seat_id, today(), why.trim()])
            .map_err(|e| e.to_string())?;
        self.log("ended", Some(licence), Some(seat), why.trim())?;
        self.save()
    }

    /// Revoke a licence (refund, stolen boat…): no new seats, and it is marked in the
    /// register. Installations already activated keep working offline: say so to the customer.
    pub fn revoke(&mut self, licence: i64, why: &str) -> Result<()> {
        if why.trim().is_empty() {
            return Err("Say why the licence is revoked (it goes in the log)".into());
        }
        self.db.execute("UPDATE licences SET status = 'revoked' WHERE id = ?1", [licence]).map_err(|e| e.to_string())?;
        self.db
            .execute(
                "UPDATE seats SET status = 'revoked', ended = ?2, end_reason = ?3 WHERE licence_id = ?1 AND status = 'active'",
                params![licence, today(), why.trim()],
            )
            .map_err(|e| e.to_string())?;
        self.log("revoked", Some(licence), None, why.trim())?;
        self.save()
    }

    pub fn log_entries(&self, licence: Option<i64>, limit: u32) -> Vec<LogEntry> {
        let mut st =
            self.db.prepare("SELECT at, action, licence_id, seat, detail FROM log WHERE ?1 IS NULL OR licence_id = ?1 ORDER BY id DESC LIMIT ?2").unwrap();
        st.query_map(params![licence, limit], |r| Ok(LogEntry { at: r.get(0)?, action: r.get(1)?, licence_id: r.get(2)?, seat: r.get(3)?, detail: r.get(4)? }))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    /// The HW_ID register (S-63: available to the IHO on request), as CSV.
    pub fn register_csv(&self) -> String {
        let mut out = String::from("licence,created,hw_id,user_permit,status,customer,vessel,mmsi,seats_in_use\r\n");
        let mut st = self
            .db
            .prepare(
                "SELECT l.id, l.created, l.hw_id, l.user_permit, l.status, c.name, l.vessel, l.mmsi,
                   (SELECT COUNT(*) FROM seats s WHERE s.licence_id = l.id AND s.status = 'active')
                 FROM licences l JOIN customers c ON c.id = l.customer_id ORDER BY l.id",
            )
            .unwrap();
        let q = |s: String| if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s };
        let rows = st
            .query_map([], |r| {
                Ok(format!(
                    "{},{},{},{},{},{},{},{},{}\r\n",
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    q(r.get(5)?),
                    q(r.get(6)?),
                    r.get::<_, String>(7)?,
                    r.get::<_, i64>(8)?
                ))
            })
            .unwrap();
        for r in rows.flatten() {
            out.push_str(&r);
        }
        out
    }

    /// A backup that opens anywhere with the passphrase (for a new PC or a disaster).
    pub fn export_backup(&self, path: &Path, pass: &str) -> Result<()> {
        if pass.chars().count() < 12 {
            return Err("Use a passphrase of at least 12 characters (a few words is good)".into());
        }
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
        let key = passphrase_key(pass, &salt)?;
        let data = Zeroizing::new(self.db.serialize("main").map_err(|e| e.to_string())?.to_vec());
        let mut extra = vec![self.test as u8];
        extra.extend_from_slice(&salt);
        let out = seal(&key, BACKUP_MAGIC, &extra, &data);
        std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))?;
        // check it opens before saying it's done
        read_backup(path, pass)?;
        self.set("last_backup", &now())?;
        self.log("backup", None, None, &format!("passphrase backup to {}", path.display()))?;
        self.set("changes", "0")?;
        self.save()
    }

    /// Replace this vault's contents with a passphrase backup (the current version is
    /// kept in `history`).
    pub fn restore_backup(&mut self, path: &Path, pass: &str) -> Result<()> {
        let (test, plain) = read_backup(path, pass)?;
        if test != self.test {
            return Err(format!(
                "That is a {} backup: switch to {} first",
                if test { "test mode" } else { "real" },
                if test { "test mode" } else { "your own credentials" }
            ));
        }
        self.save()?;
        let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
        let n = plain.len();
        db.deserialize_read_exact("main", std::io::Cursor::new(&plain[..]), n, false).map_err(|e| e.to_string())?;
        db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        self.db = db;
        self.log("restore", None, None, &format!("restored from {}", path.display()))?;
        if !self.test && self.get("signing_key").is_some() {
            self.write_build_files()?;
        }
        self.save()
    }
}

fn read_backup(path: &Path, pass: &str) -> Result<(bool, Zeroizing<Vec<u8>>)> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if b.len() < 5 + 1 + 16 + 12 || &b[..5] != BACKUP_MAGIC {
        return Err(format!("{} isn't a Licence Manager backup", path.display()));
    }
    let key = passphrase_key(pass, &b[6..22])?;
    let plain = unseal(&key, &b[22..]).ok_or("Wrong passphrase, or the backup is damaged")?;
    Ok((b[5] == 1, plain))
}

fn check_vessel(vessel: &str, mmsi: &str) -> Result<()> {
    if vessel.trim().is_empty() {
        return Err("A licence is for one vessel: give its name".into());
    }
    let m = mmsi.trim();
    if !m.is_empty() && (m.len() != 9 || !m.bytes().all(|b| b.is_ascii_digit())) {
        return Err("An MMSI is nine digits".into());
    }
    Ok(())
}

fn s63_test_m_key() -> &'static str {
    chart::enc::TEST_M_KEY
}

/// The vault key: made once, kept with DPAPI (user scope) on Windows.
fn vault_key(dir: &Path) -> Result<(Zeroizing<[u8; 32]>, bool)> {
    let f = dir.join("vault.key");
    if f.is_file() {
        let b = std::fs::read(&f).map_err(|e| e.to_string())?;
        if b.len() < 5 || &b[..4] != b"TPK1" {
            return Err(format!("{} isn't a vault key", f.display()));
        }
        let (k, protected) = match b[4] {
            1 => (device::dpapi::unprotect(&b[5..], KEY_ENTROPY)?, true),
            0 => (Zeroizing::new(b[5..].to_vec()), false),
            _ => return Err(format!("{} isn't a vault key", f.display())),
        };
        let key: [u8; 32] = k[..].try_into().map_err(|_| format!("{} is damaged", f.display()))?;
        return Ok((Zeroizing::new(key), protected));
    }
    let mut k = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut k[..]).map_err(|e| e.to_string())?;
    let mut out = b"TPK1".to_vec();
    let protected = device::dpapi::AVAILABLE;
    if protected {
        out.push(1);
        out.extend(device::dpapi::protect(&k[..], false, KEY_ENTROPY)?);
    } else {
        out.push(0);
        out.extend_from_slice(&k[..]);
    }
    device::write_private(&f, &out)?;
    Ok((k, protected))
}

/// The email to send with an activation code.
fn email(l: &Licence, seat: u32, code: &str) -> (String, String) {
    let first = l.customer.name.split_whitespace().next().unwrap_or("there");
    let subject = format!("TweedPlot activation for {} (seat {seat})", l.vessel);
    let body = format!(
        "Kia ora {first},\r\n\r\n\
         Here is the activation code for TweedPlot on {vessel}, seat {seat} of {seats}:\r\n\r\n{code}\r\n\r\n\
         In TweedPlot, open Chart › Encrypted charts, paste the code into Activation code and press Activate. \
         It only works on the computer whose installation code you sent.\r\n\r\n\
         Your user permit is {up}. Give it to your chart supplier when you order ENC permits: it is the same on every \
         computer on {vessel}, so one set of chart permits covers them all.\r\n\r\n\
         The licence is for {vessel}{mmsi} and up to {seats} computers on board. If you replace a computer, remove the \
         activation on the old one first (Chart › Encrypted charts › Remove) and send me the release code it shows.\r\n\r\n\
         Ngā mihi,\r\nChris\r\nTweedsmarine · tweedsmarine.nz\r\n",
        vessel = l.vessel,
        seats = l.seats,
        up = l.user_permit,
        mmsi = if l.mmsi.is_empty() { String::new() } else { format!(" (MMSI {})", l.mmsi) },
    );
    (subject, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chart::enc::device::Device;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tp_licence_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn inst(fp: u8, dev: u8) -> (String, Device) {
        let d = Device::from_secret([dev; 32]);
        (oem::installation_code(&[fp; 16], &d.public), d)
    }

    fn new_licence(vessel: &str) -> NewLicence {
        NewLicence {
            customer_name: "Sam Skipper".into(),
            email: "sam@example.com".into(),
            vessel: vessel.into(),
            mmsi: "512000123".into(),
            ..Default::default()
        }
    }

    #[test]
    fn seats_on_one_vessel() {
        let d = dir("seats");
        let mut v = Vault::open(&d, true).unwrap();
        assert!(v.status().has_credentials);
        let (c1, d1) = inst(1, 1);
        let a = v
            .issue(&IssueRequest {
                installation_code: c1.clone(),
                licence_id: None,
                new_licence: Some(new_licence("Morgane")),
                label: "chart table".into(),
                over_limit_reason: None,
            })
            .unwrap();
        assert_eq!((a.seat, a.how.as_str()), (1, "new seat"));
        assert_eq!(a.user_permit, chart::enc::TEST_USER_PERMIT);
        // the app reads it and gets the HW_ID out
        let act = oem::read_activation(&a.activation_code).unwrap();
        assert_eq!(*act.open(&[1; 16], &d1).unwrap(), *b"12345");
        assert_eq!((act.grant.vessel.as_str(), act.grant.mmsi.as_str(), act.grant.seat), ("Morgane", "512000123", 1));
        let id = a.licence_id;
        let req = |code: &str| IssueRequest {
            installation_code: code.into(),
            licence_id: Some(id),
            new_licence: None,
            label: String::new(),
            over_limit_reason: None,
        };
        // the same installation again: same seat
        assert_eq!(v.issue(&req(&c1)).unwrap().how, "same installation again");
        // TweedPlot reinstalled on the same computer: same seat
        let (c1b, _) = inst(1, 9);
        let r = v.issue(&req(&c1b)).unwrap();
        assert_eq!((r.seat, r.how.as_str()), (1, "reinstalled on the same computer"));
        // two more computers, then the fourth is refused
        assert_eq!(v.issue(&req(&inst(2, 2).0)).unwrap().seat, 2);
        assert_eq!(v.issue(&req(&inst(3, 3).0)).unwrap().seat, 3);
        let (c4, _) = inst(4, 4);
        let e = v.issue(&req(&c4)).unwrap_err();
        assert!(e.contains("All 3 seats"), "{e}");
        assert_eq!(v.licence(id).unwrap().active, 3);
        // the seat-1 computer releases: its code frees seat 1, and the fourth gets it
        let hw = v.hw_id(id).unwrap();
        let g = act.grant.clone();
        let rel = oem::release_code(&g, &hw, "20261103");
        // (the code came from the first installation; seat 1 now belongs to the reinstall
        // on the same computer, same fingerprint, so it is that seat that is freed)
        assert!(v.release(&rel).unwrap().contains("is free"));
        assert!(v.release(&rel).unwrap().contains("already free"));
        assert_eq!(v.issue(&req(&c4)).unwrap().seat, 1);
        // going over with a reason
        let r = v.issue(&IssueRequest { over_limit_reason: Some("spare laptop for delivery trip".into()), ..req(&inst(5, 5).0) }).unwrap();
        assert_eq!(r.seat, 4);
        assert!(r.warnings.iter().any(|w| w.contains("Over")));
        // a forged release code fails
        let bad = oem::release_code(&g, b"54321", "20261103");
        assert!(v.release(&bad).is_err());
        // the vault reopens with everything in it
        drop(v);
        let v = Vault::open(&d, true).unwrap();
        let l = v.licence(id).unwrap();
        assert_eq!((l.active, l.vessel.as_str()), (4, "Morgane"));
        assert!(v.log_entries(Some(id), 100).iter().any(|e| e.action == "released"));
        assert!(v.register_csv().contains("12345"));
        // and it isn't readable on disk
        let raw = std::fs::read(d.join("licences-test.tpv")).unwrap();
        assert!(!raw.windows(7).any(|w| w == b"Morgane"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn same_computer_on_two_vessels_is_warned() {
        let d = dir("two");
        let mut v = Vault::open(&d, true).unwrap();
        let (c, _) = inst(1, 1);
        v.issue(&IssueRequest {
            installation_code: c.clone(),
            licence_id: None,
            new_licence: Some(new_licence("Morgane")),
            label: String::new(),
            over_limit_reason: None,
        })
        .unwrap();
        let r = v
            .issue(&IssueRequest {
                installation_code: c,
                licence_id: None,
                new_licence: Some(new_licence("Thelma G")),
                label: String::new(),
                over_limit_reason: None,
            })
            .unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("one vessel")), "{:?}", r.warnings);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn real_credentials_and_backup() {
        let d = dir("real");
        let mut v = Vault::open(&d, false).unwrap();
        assert!(!v.status().has_credentials);
        let (c, dev) = inst(1, 1);
        assert!(v
            .issue(&IssueRequest {
                installation_code: c.clone(),
                licence_id: None,
                new_licence: Some(new_licence("Morgane")),
                label: String::new(),
                over_limit_reason: None
            })
            .is_err());
        assert!(v.set_credentials("10", "10121").is_err());
        v.set_credentials("TW", "98765").unwrap();
        let st = v.status();
        assert!(st.has_credentials && st.pub_matches);
        assert!(!std::fs::read_to_string(d.join("oem.txt")).unwrap().contains("M_KEY="));
        let a = v
            .issue(&IssueRequest {
                installation_code: c,
                licence_id: None,
                new_licence: Some(new_licence("Morgane")),
                label: String::new(),
                over_limit_reason: None,
            })
            .unwrap();
        assert!(a.user_permit.ends_with("5457"));
        let hw = v.hw_id(a.licence_id).unwrap();
        assert_ne!(*hw, *b"12345");
        // the user permit opens with the M_KEY to the licence's HW_ID
        assert_eq!(s63::hw_id_from_user_permit(&a.user_permit, "98765").unwrap(), *hw);
        let public = ed25519_dalek::SigningKey::from_bytes(&v.signing_key().unwrap()).verifying_key().to_bytes();
        let act = oem::read_activation_with(&a.activation_code, &public).unwrap();
        // a build of TweedPlot with another M_ID refuses it (SSE 17); one with M_ID TW opens it
        match act.open(&[1; 16], &dev) {
            Ok(h) => assert_eq!((*h, oem::m_id()), (*hw, "TW")),
            Err(e) => assert_eq!(e, oem::ActivationError::UserPermit),
        }
        // a passphrase backup restores into a vault on a "new PC"
        let bk = d.join("backup.tplb");
        assert!(v.export_backup(&bk, "short").is_err());
        v.export_backup(&bk, "correct horse battery staple").unwrap();
        assert_eq!(v.status().changes_since_backup, 0);
        let d2 = dir("real2");
        let mut v2 = Vault::open(&d2, false).unwrap();
        assert!(v2.restore_backup(&bk, "wrong passphrase here").is_err());
        v2.restore_backup(&bk, "correct horse battery staple").unwrap();
        assert_eq!(v2.licences().len(), 1);
        assert!(v2.status().has_credentials && v2.status().pub_matches);
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d2);
    }

    #[test]
    fn old_tool_files_move_into_the_vault() {
        let d = dir("old");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("oem.txt"), "M_ID=TW\r\nM_KEY=98765\r\n").unwrap();
        std::fs::write(d.join("signing.key"), oem::TEST_SIGNING_KEY).unwrap();
        std::fs::write(d.join("register.csv"), "issued,hw_id,user_permit,installation_code,name\r\n20261021,ABCDE,X,TP1-..,Morgane\r\n").unwrap();
        let mut v = Vault::open(&d, false).unwrap();
        assert_eq!(v.status().old_files.len(), 3);
        let done = v.import_old_files().unwrap();
        assert_eq!(done.len(), 3, "{done:?}");
        assert!(!d.join("signing.key").exists());
        assert!(!std::fs::read_to_string(d.join("oem.txt")).unwrap().contains("M_KEY="));
        assert!(v.status().old_files.is_empty());
        assert_eq!(v.licences().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
