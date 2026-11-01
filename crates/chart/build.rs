//! Builds in TweedPlot's S-63 manufacturer credentials (M_ID, M_KEY) and the public
//! key that activation codes are checked with, from the OEM folder on the build PC:
//! `TWEEDPLOT_OEM_DIR`, else `~/.tweedplot-oem`. It holds `oem.txt` (lines `M_ID=..`
//! and `M_KEY=..`) and `signing.pub` (written by `tweedplot-oem init`). Neither is in the
//! repository. Without them the build uses the IHO's generic test credentials.
//!
//! The M_KEY is not stored as text: it is masked with bytes made at build time and put
//! back together only when it is used, so it can't be found by searching the program.

use std::path::PathBuf;

const TEST_PUBLIC_KEY: &str = "9ff6c628d655ea920ae5c78a168047bb8570167c0b5a8ce28a4ca453ce8b75b4";

fn oem_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("TWEEDPLOT_OEM_DIR") {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    Some(PathBuf::from(home).join(".tweedplot-oem"))
}

fn random_bytes(n: usize) -> Vec<u8> {
    use std::hash::{BuildHasher, Hasher};
    let mut out = Vec::new();
    while out.len() < n {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_usize(out.len());
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        out.extend_from_slice(&h.finish().to_le_bytes());
    }
    out.truncate(n);
    out
}

fn main() {
    println!("cargo:rerun-if-env-changed=TWEEDPLOT_OEM_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    let dir = oem_dir();
    let mut m_id = "10".to_string();
    let mut m_key = "10121".to_string();
    let mut public = TEST_PUBLIC_KEY.to_string();
    let mut real = false;
    if let Some(d) = &dir {
        let oem = d.join("oem.txt");
        let pubf = d.join("signing.pub");
        println!("cargo:rerun-if-changed={}", oem.display());
        println!("cargo:rerun-if-changed={}", pubf.display());
        if let (Ok(text), Ok(p)) = (std::fs::read_to_string(&oem), std::fs::read_to_string(&pubf)) {
            let get =
                |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).and_then(|v| v.trim_start().strip_prefix('=')).map(|v| v.trim().to_string()));
            match (get("M_ID"), get("M_KEY")) {
                (Some(id), Some(key)) if id.len() == 2 && key.len() == 5 && p.trim().len() == 64 => {
                    m_id = id;
                    m_key = key;
                    public = p.trim().to_string();
                    real = true;
                }
                _ => println!("cargo:warning=TweedPlot OEM files in {} are incomplete: building with the IHO test credentials", d.display()),
            }
        }
    }
    let mask = random_bytes(5);
    let masked: Vec<u8> = m_key.bytes().zip(&mask).map(|(a, b)| a ^ b).collect();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("oem.rs");
    let code = format!(
        "pub(crate) const REAL: bool = {real};\n\
         pub(crate) const M_ID: &str = {m_id:?};\n\
         pub(crate) const M_KEY_MASKED: [u8; 5] = {masked:?};\n\
         pub(crate) const M_KEY_MASK: [u8; 5] = {mask:?};\n\
         pub(crate) const ACTIVATION_PUBLIC_KEY: &str = {public:?};\n"
    );
    std::fs::write(out, code).unwrap();
}
