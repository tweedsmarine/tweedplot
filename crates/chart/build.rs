//! Builds in TweedPlot's S-63 manufacturer ID (M_ID) and the public key that activation
//! codes are checked with, from the OEM folder on the build PC: `TWEEDPLOT_OEM_DIR`,
//! else `~/.tweedplot-oem`. The Licence Manager writes `oem.txt` (a line `M_ID=..`) and
//! `signing.pub` there. Neither is in the repository. Without them the build uses the
//! IHO's generic test M_ID and the test signing key.
//!
//! The M_KEY is never built into TweedPlot: only the Licence Manager holds it (in its
//! vault). An `M_KEY=` line left in oem.txt is ignored, with a warning.

use std::path::PathBuf;

const TEST_PUBLIC_KEY: &str = "9ff6c628d655ea920ae5c78a168047bb8570167c0b5a8ce28a4ca453ce8b75b4";

fn oem_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("TWEEDPLOT_OEM_DIR") {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    Some(PathBuf::from(home).join(".tweedplot-oem"))
}

fn main() {
    println!("cargo:rerun-if-env-changed=TWEEDPLOT_OEM_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    let dir = oem_dir();
    let mut m_id = "10".to_string();
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
            if get("M_KEY").is_some() {
                println!(
                    "cargo:warning=oem.txt in {} still has an M_KEY line: it is not built in. Open the Licence Manager to move it into its vault",
                    d.display()
                );
            }
            match get("M_ID") {
                Some(id) if id.len() == 2 && id.is_ascii() && p.trim().len() == 64 => {
                    m_id = id;
                    public = p.trim().to_string();
                    real = true;
                }
                _ => println!("cargo:warning=TweedPlot OEM files in {} are incomplete: building with the IHO test credentials", d.display()),
            }
        }
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("oem.rs");
    let code = format!(
        "pub(crate) const REAL: bool = {real};\n\
         pub(crate) const M_ID: &str = {m_id:?};\n\
         pub(crate) const ACTIVATION_PUBLIC_KEY: &str = {public:?};\n"
    );
    std::fs::write(out, code).unwrap();
}
