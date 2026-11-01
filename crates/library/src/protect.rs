//! Protected library files: a library file (or part of one) given to another boat, locked
//! with a passphrase, since a fisher's marks are valuable. The whole file is encrypted with
//! XChaCha20-Poly1305 under a key made from the passphrase with Argon2id, so without the
//! passphrase it reads as noise and a wrong passphrase (or a damaged file) is refused.
//!
//! File: "TWEEDPLOT-PROTECTED-1\n", 16 bytes of salt, 24 bytes of nonce, then the sealed library.

use std::path::Path;

use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::{Error, Result};

const MAGIC: &[u8] = b"TWEEDPLOT-PROTECTED-1\n";
/// The extension a protected library is written with.
pub const EXT: &str = "tplib";

fn key(pass: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let mut k = [0u8; 32];
    argon2::Argon2::default().hash_password_into(pass.as_bytes(), salt, &mut k).map_err(|e| Error::Store(format!("passphrase: {e}")))?;
    Ok(k)
}

/// Is the file a protected library?
pub fn is_protected(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 22];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head[..] == MAGIC[..]
}

/// Lock the library file `src` with `pass`, written to `dst`.
pub fn protect_file(src: &Path, dst: &Path, pass: &str) -> Result<()> {
    if pass.chars().count() < 6 {
        return Err(Error::Store("the passphrase must be at least 6 characters".into()));
    }
    let data = std::fs::read(src)?;
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let cipher = XChaCha20Poly1305::new(&key(pass, &salt)?.into());
    let sealed = cipher.encrypt(&nonce, data.as_slice()).map_err(|_| Error::Store("couldn't lock the file".into()))?;
    let mut out = Vec::with_capacity(MAGIC.len() + 40 + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    let tmp = dst.with_extension("part");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

/// Unlock a protected library `src` with `pass` into `dst` (a plain library file).
pub fn unprotect_file(src: &Path, dst: &Path, pass: &str) -> Result<()> {
    let data = std::fs::read(src)?;
    if data.len() < MAGIC.len() + 40 || &data[..MAGIC.len()] != MAGIC {
        return Err(Error::Store("that isn't a protected TweedPlot library".into()));
    }
    let rest = &data[MAGIC.len()..];
    let (salt, rest) = rest.split_at(16);
    let (nonce, sealed) = rest.split_at(24);
    let cipher = XChaCha20Poly1305::new(&key(pass, salt)?.into());
    let plain = cipher.decrypt(XNonce::from_slice(nonce), sealed).map_err(|_| Error::Store("wrong passphrase (or the file is damaged)".into()))?;
    let tmp = dst.with_extension("part");
    std::fs::write(&tmp, plain)?;
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn lock_and_unlock() {
        let d = std::env::temp_dir().join(format!("protect-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b, c) = (d.join("a.gpkg"), d.join("b.tplib"), d.join("c.gpkg"));
        std::fs::write(&a, b"SQLite format 3\0 some marks").unwrap();
        super::protect_file(&a, &b, "tuna season").unwrap();
        assert!(super::is_protected(&b));
        assert!(!super::is_protected(&a));
        assert!(super::unprotect_file(&b, &c, "wrong pass").is_err());
        super::unprotect_file(&b, &c, "tuna season").unwrap();
        assert_eq!(std::fs::read(&c).unwrap(), std::fs::read(&a).unwrap());
        assert!(super::protect_file(&a, &b, "abc").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
