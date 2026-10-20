//! tweedplot-oem: Tweedsmarine's side of S-63 for TweedPlot. Not shipped with the app.
//!
//!   tweedplot-oem init                         make the OEM folder: signing key, oem.txt to fill in, register
//!   tweedplot-oem issue <installation code> --name "<boat / owner>" [--hwid XXXXX]
//!                                              pick a HW_ID, record it, make the user permit and activation code
//!   tweedplot-oem list                         the HW_ID register
//!
//! The OEM folder is TWEEDPLOT_OEM_DIR, else ~/.tweedplot-oem (%USERPROFILE%\.tweedplot-oem):
//! - oem.txt      M_ID=.. and M_KEY=.. from the IHO (keep this folder private and backed up)
//! - signing.key  the private key activation codes are signed with (never share it)
//! - signing.pub  its public key, built into TweedPlot
//! - register.csv every HW_ID issued (S-63 10.10.5: available to the IHO on request)
//!
//! This tool uses the credentials built into the chart crate, so build it on the same PC
//! as the app. Until oem.txt and signing.pub exist, both use the IHO test credentials and
//! a test signing key, and `issue` gives HW_ID 12345 (the S-64 test data's) unless told
//! otherwise.

use chart::enc::{machine, oem, s63};
use std::io::Write;
use std::path::PathBuf;

/// The test signing key that goes with the test public key in crates/chart/build.rs.
const TEST_SIGNING: &str = "516407dd9d626dde9462eedbaa853dc0bca8af1430eeb37a16b35c2e9db64a5f";

fn dir() -> PathBuf {
    if let Ok(d) = std::env::var("TWEEDPLOT_OEM_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".tweedplot-oem")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    (s.len() % 2 == 0).then(|| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect::<Option<Vec<u8>>>()).flatten()
}

fn fail(msg: &str) -> ! {
    eprintln!("tweedplot-oem: {msg}");
    std::process::exit(1)
}

fn init() {
    let d = dir();
    std::fs::create_dir_all(&d).unwrap_or_else(|e| fail(&format!("can't make {}: {e}", d.display())));
    let key = d.join("signing.key");
    if key.exists() {
        println!("{} already exists: kept (a new key would make every activation code issued so far invalid in new builds)", key.display());
    } else {
        let mut sk = [0u8; 32];
        getrandom::fill(&mut sk).unwrap_or_else(|_| fail("no random numbers"));
        let public = ed25519_dalek::SigningKey::from_bytes(&sk).verifying_key().to_bytes();
        std::fs::write(&key, hex(&sk)).unwrap_or_else(|e| fail(&e.to_string()));
        std::fs::write(d.join("signing.pub"), hex(&public)).unwrap_or_else(|e| fail(&e.to_string()));
        println!("made {} and signing.pub", key.display());
    }
    let o = d.join("oem.txt");
    if !o.exists() {
        std::fs::write(&o, "# TweedPlot S-63 manufacturer credentials from the IHO. Fill in and rebuild TweedPlot.\r\n# M_ID=XX\r\n# M_KEY=XXXXX\r\n").unwrap_or_else(|e| fail(&e.to_string()));
        println!("made {}: put the M_ID and M_KEY from the IHO in it (without the #), then rebuild TweedPlot and this tool", o.display());
    }
    let r = d.join("register.csv");
    if !r.exists() {
        std::fs::write(&r, "issued,hw_id,user_permit,installation_code,name\r\n").unwrap_or_else(|e| fail(&e.to_string()));
        println!("made {}", r.display());
    }
}

fn signing_key() -> [u8; 32] {
    if !oem::is_real() {
        return unhex(TEST_SIGNING).unwrap().try_into().unwrap();
    }
    let t = std::fs::read_to_string(dir().join("signing.key")).unwrap_or_else(|_| fail("no signing.key: run tweedplot-oem init"));
    let k: [u8; 32] = unhex(&t).and_then(|v| v.try_into().ok()).unwrap_or_else(|| fail("signing.key is damaged"));
    k
}

/// The HW_IDs issued so far.
fn register() -> Vec<Vec<String>> {
    let t = std::fs::read_to_string(dir().join("register.csv")).unwrap_or_default();
    t.lines().skip(1).filter(|l| !l.trim().is_empty()).map(|l| l.split(',').map(|f| f.trim().trim_matches('"').to_string()).collect()).collect()
}

fn random_hw_id(taken: &[String]) -> String {
    loop {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).unwrap_or_else(|_| fail("no random numbers"));
        let id = format!("{:05X}", u32::from_le_bytes(b) & 0xF_FFFF);
        // not the IHO's test HW_ID, nothing already issued
        if id != "12345" && !taken.contains(&id) {
            return id;
        }
    }
}

fn issue(args: &[String]) {
    let code = args.first().unwrap_or_else(|| fail("give the installation code (TP1-...)"));
    let fp = machine::read_code(code).unwrap_or_else(|| fail("that isn't a whole installation code (TP1- and nine groups of four; check for a typo)"));
    let mut name = String::new();
    let mut hw: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                name = args.get(i + 1).cloned().unwrap_or_default();
                i += 1;
            }
            "--hwid" => {
                hw = args.get(i + 1).map(|s| s.to_ascii_uppercase());
                i += 1;
            }
            other => fail(&format!("don't know {other}")),
        }
        i += 1;
    }
    let reg = register();
    let taken: Vec<String> = reg.iter().filter_map(|r| r.get(1).cloned()).collect();
    let hw = match hw {
        // the same HW_ID again: a second computer on the same boat (S-63 4.2.2 note 2,
        // with the data server's consent)
        Some(h) => {
            if h.len() != 5 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
                fail("a HW_ID is five hex digits");
            }
            if oem::is_real() && !taken.contains(&h) {
                fail(&format!("{h} isn't in the register: leave out --hwid for a new one"));
            }
            h
        }
        None if !oem::is_real() => "12345".to_string(),
        None => random_hw_id(&taken),
    };
    let hw5: [u8; 5] = hw.as_bytes().try_into().unwrap();
    let user_permit = s63::make_user_permit(&hw5, &oem::m_key(), &oem::m_id_hex());
    let issued = chrono::Utc::now().format("%Y%m%d").to_string();
    let a = oem::Activation { user_permit: user_permit.clone(), fingerprint: fp, issued: issued.clone() };
    let key = signing_key();
    let activation = oem::make_activation(&a, &key);
    // the app will accept it: same public key as built in
    if oem::read_activation(&activation).is_err() {
        fail("this signing.key doesn't match the public key built into TweedPlot: rebuild after tweedplot-oem init");
    }
    if oem::is_real() {
        let path = dir().join("register.csv");
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap_or_else(|_| fail("no register.csv: run tweedplot-oem init"));
        let clean = name.replace([',', '"', '\r', '\n'], " ");
        writeln!(f, "{issued},{hw},{user_permit},{},{clean}\r", machine::code_for(&fp)).unwrap_or_else(|e| fail(&e.to_string()));
    } else {
        println!("(IHO test credentials: nothing recorded)");
    }
    println!("User permit: {user_permit}");
    println!("Activation code for {}:\n\n{activation}\n", if name.is_empty() { "this installation" } else { &name });
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("TweedPlot OEM tool: {} (M_ID {}), folder {}", if oem::is_real() { "TweedPlot's own credentials" } else { "IHO TEST credentials" }, oem::m_id(), dir().display());
    match args.first().map(String::as_str) {
        Some("init") => init(),
        Some("issue") => issue(&args[1..]),
        Some("list") => {
            for r in register() {
                println!("{}", r.join("  "));
            }
        }
        Some("fingerprint") => {
            // this computer, for checking
            println!("{}", machine::installation_code());
            for l in machine::describe() {
                println!("  {l}");
            }
        }
        _ => println!("usage: tweedplot-oem init | issue <installation code> --name \"<boat / owner>\" [--hwid XXXXX] | list | fingerprint"),
    }
}
