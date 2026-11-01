//! This installation's device key, and Windows DPAPI for keeping secrets on disk.
//!
//! Each installation makes an X25519 key pair the first time it is asked for one. The
//! public half goes into the installation code; Tweedsmarine's Licence Manager encrypts
//! the HW_ID to it, so only this installation can read the HW_ID out of its activation
//! code. The secret half also goes into the key that seals the decrypted chart data.
//!
//! On Windows the secret is kept with DPAPI in machine scope: the file can only be
//! opened on this computer (copied to another one it is useless). Elsewhere (development
//! on Linux or macOS) it is kept as a private file, and [`Device::protected`] is false.

use std::path::Path;
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"TPD1";
const ENTROPY: &[u8] = b"TweedPlot device key 1";

/// This installation's device key pair.
pub struct Device {
    secret: Zeroizing<[u8; 32]>,
    pub public: [u8; 32],
    /// Whether the secret is kept with DPAPI (Windows) rather than as a plain file.
    pub protected: bool,
}

impl Device {
    /// A device from a known secret (tests, and the Licence Manager's test mode).
    pub fn from_secret(secret: [u8; 32]) -> Device {
        let s = x25519_dalek::StaticSecret::from(secret);
        let public = x25519_dalek::PublicKey::from(&s).to_bytes();
        Device { secret: Zeroizing::new(secret), public, protected: false }
    }

    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    /// The X25519 shared secret with another party's public key.
    pub fn shared(&self, other: &[u8; 32]) -> Zeroizing<[u8; 32]> {
        let s = x25519_dalek::StaticSecret::from(*self.secret);
        Zeroizing::new(s.diffie_hellman(&x25519_dalek::PublicKey::from(*other)).to_bytes())
    }

    /// Read the device key from `path`, or make one and save it there.
    pub fn load_or_create(path: &Path) -> Result<Device, String> {
        if path.is_file() {
            let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            if b.len() < 5 || &b[..4] != MAGIC {
                return Err(format!("{} isn't a TweedPlot device key", path.display()));
            }
            let (secret, protected) = match b[4] {
                1 => (dpapi::unprotect(&b[5..], ENTROPY)?, true),
                0 => (Zeroizing::new(b[5..].to_vec()), false),
                _ => return Err(format!("{} isn't a TweedPlot device key", path.display())),
            };
            let s: [u8; 32] = secret[..].try_into().map_err(|_| format!("{} is damaged", path.display()))?;
            let mut d = Device::from_secret(s);
            d.protected = protected;
            return Ok(d);
        }
        let mut s = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut s[..]).map_err(|e| e.to_string())?;
        let mut d = Device::from_secret(*s);
        let mut out = MAGIC.to_vec();
        if dpapi::AVAILABLE {
            out.push(1);
            out.extend(dpapi::protect(&s[..], true, ENTROPY)?);
            d.protected = true;
        } else {
            out.push(0);
            out.extend_from_slice(&s[..]);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        write_private(path, &out)?;
        Ok(d)
    }
}

/// Write a file only this user can read (on Unix; on Windows the folder's rights apply
/// and the contents are DPAPI-protected anyway).
pub fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(data).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Windows DPAPI (CryptProtectData / CryptUnprotectData). `machine` scope can be opened
/// by any user of this computer and by no other computer; user scope only by the same
/// Windows user (and, with a domain or Microsoft account, that user's other computers).
pub mod dpapi {
    use zeroize::Zeroizing;

    pub const AVAILABLE: bool = cfg!(windows);

    #[cfg(windows)]
    pub fn protect(data: &[u8], machine: bool, entropy: &[u8]) -> Result<Vec<u8>, String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let ent = CRYPT_INTEGER_BLOB { cbData: entropy.len() as u32, pbData: entropy.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let flags = CRYPTPROTECT_UI_FORBIDDEN | if machine { CRYPTPROTECT_LOCAL_MACHINE } else { 0 };
        // SAFETY: the blobs point at live slices for the call; the output is copied, then freed with LocalFree.
        unsafe {
            if CryptProtectData(&input, std::ptr::null(), &ent, std::ptr::null(), std::ptr::null(), flags, &mut out) == 0 {
                return Err(format!("Windows couldn't protect the key (DPAPI error {})", std::io::Error::last_os_error()));
            }
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData as _);
            Ok(v)
        }
    }

    #[cfg(windows)]
    pub fn unprotect(blob: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
        let input = CRYPT_INTEGER_BLOB { cbData: blob.len() as u32, pbData: blob.as_ptr() as *mut u8 };
        let ent = CRYPT_INTEGER_BLOB { cbData: entropy.len() as u32, pbData: entropy.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: as in protect; the plain copy is zeroed in place before it is freed.
        unsafe {
            if CryptUnprotectData(&input, std::ptr::null_mut(), &ent, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) == 0 {
                return Err(format!(
                    "Windows couldn't open the key: it was made on another computer or by another Windows user ({})",
                    std::io::Error::last_os_error()
                ));
            }
            let s = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
            let v = Zeroizing::new(s.to_vec());
            zeroize::Zeroize::zeroize(s);
            LocalFree(out.pbData as _);
            Ok(v)
        }
    }

    #[cfg(not(windows))]
    pub fn protect(_data: &[u8], _machine: bool, _entropy: &[u8]) -> Result<Vec<u8>, String> {
        Err("DPAPI is only on Windows".into())
    }

    #[cfg(not(windows))]
    pub fn unprotect(_blob: &[u8], _entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        Err("this key was protected with Windows DPAPI: it can only be opened on the Windows computer that made it".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_is_kept() {
        let dir = std::env::temp_dir().join(format!("tp_device_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("device.key");
        let a = Device::load_or_create(&p).unwrap();
        let b = Device::load_or_create(&p).unwrap();
        assert_eq!(a.public, b.public);
        assert_eq!(a.secret(), b.secret());
        // two devices agree on a shared secret
        let c = Device::from_secret([9u8; 32]);
        assert_eq!(*a.shared(&c.public), *c.shared(&a.public));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
