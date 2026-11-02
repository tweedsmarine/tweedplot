//! Signed plugin packages (`.tpplugin`).
//!
//! A package is a zip with `manifest.json`, `manifest.sig` (an Ed25519 signature of the
//! manifest's bytes, in hex) and the plugin program. The manifest holds the program's
//! SHA-256, so the signature covers the program too. The app has Tweedsmarine's release
//! public key built in and refuses a package, or an installed plugin, that doesn't check
//! out. A download index (`index.json`) says which version is current and where it is.

use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// "TweedPlot S-63 Charts"
    pub name: String,
    pub version: String,
    /// the interface version it speaks
    pub api: u32,
    /// the program's file name
    pub exe: String,
    /// SHA-256 of the program, hex
    pub sha256: String,
    /// "windows-x86_64", "windows-aarch64", …
    pub platform: String,
}

/// What the download page lists, per platform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    pub version: String,
    pub api: u32,
    pub platform: String,
    pub url: String,
    /// SHA-256 of the package file, hex
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("not a TweedPlot plugin package: {0}")]
    Format(String),
    #[error("the package isn't signed by Tweedsmarine (or was changed after signing)")]
    Signature,
    #[error("the plugin program doesn't match its signed manifest (changed or damaged)")]
    Hash,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

/// This build's platform name, as packages are labelled.
pub fn this_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn sign(manifest: &[u8], key: &[u8; 32]) -> String {
    use ed25519_dalek::Signer;
    hex(&ed25519_dalek::SigningKey::from_bytes(key).sign(manifest).to_bytes())
}

fn check(manifest: &[u8], sig_hex: &str, public: &[u8; 32]) -> Result<Manifest, PackageError> {
    let sig: [u8; 64] = unhex(sig_hex).and_then(|v| v.try_into().ok()).ok_or(PackageError::Signature)?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(public).map_err(|_| PackageError::Signature)?;
    key.verify_strict(manifest, &ed25519_dalek::Signature::from_bytes(&sig)).map_err(|_| PackageError::Signature)?;
    serde_json::from_slice(manifest).map_err(|e| PackageError::Format(e.to_string()))
}

/// Make a package (the Licence Manager's side).
pub fn pack(name: &str, version: &str, platform: &str, exe_name: &str, exe: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, PackageError> {
    let m = Manifest {
        name: name.into(),
        version: version.into(),
        api: crate::API_VERSION,
        exe: exe_name.into(),
        sha256: sha256_hex(exe),
        platform: platform.into(),
    };
    let mbytes = serde_json::to_vec_pretty(&m).map_err(|e| PackageError::Format(e.to_string()))?;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let err = |e: zip::result::ZipError| PackageError::Format(e.to_string());
        z.start_file("manifest.json", opt).map_err(err)?;
        z.write_all(&mbytes)?;
        z.start_file("manifest.sig", opt).map_err(err)?;
        z.write_all(sign(&mbytes, key).as_bytes())?;
        z.start_file(exe_name, opt).map_err(err)?;
        z.write_all(exe)?;
        z.finish().map_err(err)?;
    }
    Ok(buf.into_inner())
}

/// Open a package and check it: the manifest's signature and the program's hash.
/// Returns the manifest, the manifest's bytes and signature (kept beside the installed
/// program so it can be checked again at each start) and the program.
pub fn open(bytes: &[u8], public: &[u8; 32]) -> Result<(Manifest, Vec<u8>, String, Vec<u8>), PackageError> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| PackageError::Format(e.to_string()))?;
    let mut read = |n: &str| -> Result<Vec<u8>, PackageError> {
        let mut f = z.by_name(n).map_err(|_| PackageError::Format(format!("no {n} in it")))?;
        let mut v = Vec::new();
        f.read_to_end(&mut v)?;
        Ok(v)
    };
    let mbytes = read("manifest.json")?;
    let sig = String::from_utf8_lossy(&read("manifest.sig")?).trim().to_string();
    let m = check(&mbytes, &sig, public)?;
    if m.exe.contains(['/', '\\']) || m.exe.starts_with('.') {
        return Err(PackageError::Format("odd program name".into()));
    }
    let exe = read(&m.exe)?;
    if sha256_hex(&exe) != m.sha256 {
        return Err(PackageError::Hash);
    }
    Ok((m, mbytes, sig, exe))
}

/// Install a checked package into `dir` (replacing what is there): the program,
/// `manifest.json` and `manifest.sig`.
pub fn install(bytes: &[u8], public: &[u8; 32], dir: &Path) -> Result<Manifest, PackageError> {
    let (m, mbytes, sig, exe) = open(bytes, public)?;
    let tmp = dir.with_extension("new");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    std::fs::write(tmp.join(&m.exe), &exe)?;
    std::fs::write(tmp.join("manifest.json"), &mbytes)?;
    std::fs::write(tmp.join("manifest.sig"), &sig)?;
    let old = dir.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    if dir.exists() {
        std::fs::rename(dir, &old)?;
    }
    std::fs::rename(&tmp, dir)?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(m)
}

/// Check an installed plugin in `dir` before starting it: the signature of its manifest
/// and the program's hash. Returns the manifest and the program's path.
pub fn verify_installed(dir: &Path, public: &[u8; 32]) -> Result<(Manifest, std::path::PathBuf), PackageError> {
    let mbytes = std::fs::read(dir.join("manifest.json"))?;
    let sig = std::fs::read_to_string(dir.join("manifest.sig"))?;
    let m = check(&mbytes, &sig, public)?;
    let exe = dir.join(&m.exe);
    if sha256_hex(&std::fs::read(&exe)?) != m.sha256 {
        return Err(PackageError::Hash);
    }
    Ok((m, exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_are_checked() {
        let key = [7u8; 32];
        let public = ed25519_dalek::SigningKey::from_bytes(&key).verifying_key().to_bytes();
        let pkg = pack("TweedPlot S-63 Charts", "1.0.0", "windows-x86_64", "tweedplot-charts.exe", b"MZ program", &key).unwrap();
        let (m, _, _, exe) = open(&pkg, &public).unwrap();
        assert_eq!((m.version.as_str(), m.api, exe.as_slice()), ("1.0.0", crate::API_VERSION, &b"MZ program"[..]));
        // another key
        assert!(matches!(open(&pkg, &ed25519_dalek::SigningKey::from_bytes(&[8u8; 32]).verifying_key().to_bytes()), Err(PackageError::Signature)));
        // installed, checked again, then the program changed on disk
        let dir = std::env::temp_dir().join(format!("tp_pkg_{}", std::process::id())).join("s63-charts");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        install(&pkg, &public, &dir).unwrap();
        let (m2, exe_path) = verify_installed(&dir, &public).unwrap();
        assert_eq!(m2, m);
        std::fs::write(&exe_path, b"MZ changed").unwrap();
        assert!(matches!(verify_installed(&dir, &public), Err(PackageError::Hash)));
        // a newer package replaces it
        let pkg2 = pack("TweedPlot S-63 Charts", "1.0.1", "windows-x86_64", "tweedplot-charts.exe", b"MZ program 2", &key).unwrap();
        assert_eq!(install(&pkg2, &public, &dir).unwrap().version, "1.0.1");
        assert!(verify_installed(&dir, &public).is_ok());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
