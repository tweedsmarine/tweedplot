//! IHO S-63 data protection: user permits, cell permits (PERMIT.TXT), decrypting and
//! unzipping encrypted ENC cells, and checking their signatures against the Scheme
//! Administrator's public key.
//!
//! - The user permit holds the system's hardware ID (HW_ID), Blowfish-encrypted with the
//!   manufacturer key (M_KEY), a CRC32 of that, and the manufacturer ID (M_ID).
//! - Each cell permit holds two cell keys Blowfish-encrypted with HW_ID6 (HW_ID with
//!   its first byte appended) and an encrypted CRC32 of the permit.
//! - A cell file is Blowfish-encrypted with a cell key after being zipped.
//! - Its signature file (the cell name with the usage digit 1-6 changed to I-N) holds the
//!   data server's DSA signature of the cell, and the Scheme Administrator's signature
//!   of the data server's public key (its certificate).

use blowfish::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use blowfish::Blowfish;
use num_bigint::BigUint;
use sha1::{Digest, Sha1};
use std::io::Read;

/// S-63 errors, with the scheme's error codes (S-63 section 11).
#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum S63Error {
    #[error("SSE 08 - The IHO public key is not in the right format")]
    KeyFormat,
    #[error("SSE 12 - Cell permit format is incorrect. Contact your data supplier and obtain a new permit file")]
    PermitFormat,
    #[error("SSE 13 - Cell permit is invalid (check sum is incorrect) or the cell permit is for a different system. Contact your data supplier and obtain a new or valid permit file")]
    PermitChecksum,
    #[error("SSE 17 - User permit is invalid (check sum is incorrect)")]
    UserPermit,
    #[error("SSE 21 - Decryption failed, no valid cell permit found. Permits may be for another system or new permits may be required: contact your supplier to obtain a new licence")]
    Decrypt,
    #[error("SSE 06 - The SA signed data server certificate is invalid. The SA may have issued a new public key or the ENC may originate from another service")]
    Certificate,
    #[error("SSE 09 - ENC signature is invalid")]
    Signature,
    #[error("SSE 24 - ENC signature format incorrect, contact your data supplier")]
    SignatureFormat,
}

impl S63Error {
    /// "SSE 13" and so on.
    pub fn code(&self) -> String {
        self.to_string().get(..6).unwrap_or("").to_string()
    }
}

fn hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn cipher(key: &[u8]) -> Blowfish {
    Blowfish::new_from_slice(key).expect("blowfish key 4-56 bytes")
}

/// Blowfish ECB over whole 8-byte blocks.
fn decrypt_ecb(key: &[u8], data: &[u8]) -> Vec<u8> {
    let c = cipher(key);
    let mut out = data[..data.len() / 8 * 8].to_vec();
    for block in out.chunks_mut(8) {
        c.decrypt_block(GenericArray::from_mut_slice(block));
    }
    out
}

/// Blowfish one block, the data padded to 8 bytes with the pad count (as S-63's tools do).
fn encrypt_padded(key: &[u8], data: &[u8]) -> [u8; 8] {
    let mut b = [(8 - data.len()) as u8; 8];
    b[..data.len()].copy_from_slice(data);
    cipher(key).encrypt_block(GenericArray::from_mut_slice(&mut b));
    b
}

/// The system's hardware ID from its user permit and the manufacturer key.
pub fn hw_id_from_user_permit(permit: &str, m_key: &str) -> Result<[u8; 5], S63Error> {
    let p = permit.trim();
    if p.len() != 28 {
        return Err(S63Error::UserPermit);
    }
    let enc = hex(&p[..16]).ok_or(S63Error::UserPermit)?;
    let crc = crc32fast::hash(p[..16].as_bytes());
    if format!("{crc:08X}") != p[16..24].to_ascii_uppercase() {
        return Err(S63Error::UserPermit);
    }
    let plain = decrypt_ecb(m_key.as_bytes(), &enc);
    let mut hw = [0u8; 5];
    hw.copy_from_slice(&plain[..5]);
    Ok(hw)
}

/// A checked cell permit.
#[derive(Debug, Clone, PartialEq)]
pub struct CellPermit {
    pub cell: String,
    /// YYYYMMDD
    pub expiry: String,
    pub keys: [[u8; 5]; 2],
    /// 0 subscription, 1 single purchase
    pub service: u8,
    pub edition: String,
    pub data_server: String,
}

fn hw_id6(hw: &[u8; 5]) -> [u8; 6] {
    [hw[0], hw[1], hw[2], hw[3], hw[4], hw[0]]
}

/// Check one 64-character cell permit string and take out its cell keys.
pub fn cell_permit(s: &str, hw: &[u8; 5]) -> Result<([u8; 5], [u8; 5]), S63Error> {
    let s = s.trim();
    if s.len() != 64 || !s.is_ascii() {
        return Err(S63Error::PermitFormat);
    }
    let key = hw_id6(hw);
    let crc = crc32fast::hash(s[..48].as_bytes());
    let want = hex(&s[48..]).ok_or(S63Error::PermitFormat)?;
    if encrypt_padded(&key, &crc.to_be_bytes()).as_slice() != want.as_slice() {
        return Err(S63Error::PermitChecksum);
    }
    let ck = |h: &str| -> Result<[u8; 5], S63Error> {
        let b = decrypt_ecb(&key, &hex(h).ok_or(S63Error::PermitFormat)?);
        let mut k = [0u8; 5];
        k.copy_from_slice(&b[..5]);
        Ok(k)
    };
    Ok((ck(&s[16..32])?, ck(&s[32..48])?))
}

/// Every record of a PERMIT.TXT file (under :ENC and :ECS), each checked against the HW_ID:
/// the cell name with its permit, or with why it can't be used.
pub fn read_permits(text: &str, hw: &[u8; 5]) -> Result<Vec<(String, Result<CellPermit, S63Error>)>, S63Error> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().ok_or(S63Error::PermitFormat)?;
    if !first.starts_with(":DATE") {
        return Err(S63Error::PermitFormat);
    }
    let mut out = Vec::new();
    for l in lines {
        if l.starts_with(':') {
            continue;
        }
        let f: Vec<&str> = l.split(',').collect();
        let p = f[0].trim();
        let cell = p.get(..8).unwrap_or(p).to_string();
        let r = cell_permit(p, hw).map(|(k1, k2)| CellPermit {
            cell: cell.clone(),
            expiry: p[8..16].to_string(),
            keys: [k1, k2],
            service: f.get(1).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            edition: f.get(2).map(|v| v.trim().to_string()).unwrap_or_default(),
            data_server: f.get(3).map(|v| v.trim().to_string()).unwrap_or_default(),
        });
        out.push((cell, r));
    }
    Ok(out)
}

/// Decrypt and unzip an encrypted cell (base or update) with the permit's keys, trying
/// cell key 1 then cell key 2.
pub fn decrypt_cell(data: &[u8], permit: &CellPermit) -> Result<Vec<u8>, S63Error> {
    for key in &permit.keys {
        let plain = decrypt_ecb(key, data);
        if !plain.starts_with(b"PK\x03\x04") {
            continue;
        }
        let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(plain)) else { continue };
        if z.is_empty() {
            continue;
        }
        let Ok(mut f) = z.by_index(0) else { continue };
        let mut out = Vec::new();
        if f.read_to_end(&mut out).is_ok() {
            return Ok(out);
        }
    }
    Err(S63Error::Decrypt)
}

/// A DSA public key (p, q, g, y).
#[derive(Debug, Clone, PartialEq)]
pub struct PublicKey {
    pub p: BigUint,
    pub q: BigUint,
    pub g: BigUint,
    pub y: BigUint,
}

/// The values in an S-63 key or signature text: each "// <label>" line followed by hex
/// groups ending with ".", in order: (label, bytes, byte offset where its header starts).
fn values(text: &[u8]) -> Vec<(String, BigUint, usize)> {
    let s = String::from_utf8_lossy(text);
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = s[pos..].find("//") {
        let start = pos + i;
        let line_end = s[start..].find('\n').map_or(s.len(), |e| start + e);
        let label = s[start + 2..line_end].trim().to_string();
        let rest = &s[line_end..];
        let end = rest.find('.').map_or(rest.len(), |e| e);
        let digits: String = rest[..end].chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if let Some(v) = BigUint::parse_bytes(digits.as_bytes(), 16) {
            out.push((label, v, start));
        }
        pos = line_end + end.min(rest.len());
        if pos >= s.len() {
            break;
        }
    }
    out
}

/// Read a public key text (IHO.PUB, "Scheme Administrator Public Key.txt").
pub fn parse_public_key(text: &[u8]) -> Result<PublicKey, S63Error> {
    let v = values(text);
    let get = |name: &str| v.iter().find(|(l, _, _)| l.to_ascii_uppercase().contains(name)).map(|x| x.1.clone());
    Ok(PublicKey {
        p: get("BIG P").ok_or(S63Error::KeyFormat)?,
        q: get("BIG Q").ok_or(S63Error::KeyFormat)?,
        g: get("BIG G").ok_or(S63Error::KeyFormat)?,
        y: get("BIG Y").ok_or(S63Error::KeyFormat)?,
    })
}

fn dsa_verify(key: &PublicKey, digest: &[u8], r: &BigUint, s: &BigUint) -> bool {
    let zero = BigUint::from(0u32);
    if r == &zero || s == &zero || r >= &key.q || s >= &key.q {
        return false;
    }
    let Some(w) = s.modinv(&key.q) else { return false };
    let h = BigUint::from_bytes_be(digest);
    let u1 = (&h * &w) % &key.q;
    let u2 = (r * &w) % &key.q;
    let v = (key.g.modpow(&u1, &key.p) * key.y.modpow(&u2, &key.p)) % &key.p % &key.q;
    &v == r
}

/// Check a cell (or update) file against its signature file: the data server's
/// certificate against the SA key, then the cell against the data server's key.
pub fn verify_cell(signature: &[u8], cell: &[u8], sa: &PublicKey) -> Result<(), S63Error> {
    let v = values(signature);
    if v.len() < 8 {
        return Err(S63Error::SignatureFormat);
    }
    let (r1, s1) = (&v[0].1, &v[1].1);
    let (r2, s2) = (&v[2].1, &v[3].1);
    // the data server's public key file: everything from the third value's header on
    let ds_text = &signature[v[4].2..];
    let ds = parse_public_key(ds_text).map_err(|_| S63Error::SignatureFormat)?;
    if !dsa_verify(sa, &Sha1::digest(ds_text), r2, s2) {
        return Err(S63Error::Certificate);
    }
    if !dsa_verify(&ds, &Sha1::digest(cell), r1, s1) {
        return Err(S63Error::Signature);
    }
    Ok(())
}

/// The signature file name for a cell file name ("GB61021A.000" → "GBN1021A.000").
pub fn signature_name(cell_file: &str) -> Option<String> {
    let mut c: Vec<char> = cell_file.chars().collect();
    let d = c.get(2)?.to_digit(10)?;
    if !(1..=6).contains(&d) {
        return None;
    }
    c[2] = (b'I' + (d as u8 - 1)) as char;
    Some(c.into_iter().collect())
}

/// For tests and tools: the user permit for a HW_ID (the OEM's side).
pub fn make_user_permit(hw: &[u8; 5], m_key: &str, m_id: &str) -> String {
    let enc = to_hex(&encrypt_padded(m_key.as_bytes(), hw));
    let crc = crc32fast::hash(enc.as_bytes());
    format!("{enc}{crc:08X}{m_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked examples in S-63 edition 1.2.
    #[test]
    fn spec_examples() {
        // 10.4: user permit for HW_ID 12348, M_KEY 98765, M_ID 01
        assert_eq!(make_user_permit(b"12348", "98765", "3031"), "73871727080876A07E450C043031");
        assert_eq!(hw_id_from_user_permit("73871727080876A07E450C043031", "98765").unwrap(), *b"12348");
        // 10.5.4 and 10.7.2: cell permit check sum and cell keys
        let (k1, k2) = cell_permit("NO4D061320000830BEB9BFE3C7C6CE68B16411FD09F96982795C77B204F54D48", b"12348").unwrap();
        assert_eq!(to_hex(&k1), "C1CB518E9C");
        assert_eq!(to_hex(&k2), "421571CC66");
        assert_eq!(cell_permit("NO4D061320000830BEB9BFE3C7C6CE68B16411FD09F96982795C77B204F54D48", b"12345"), Err(S63Error::PermitChecksum));
        // the IHO's generic S-64 credentials
        assert_eq!(hw_id_from_user_permit("66B5CBFDF7E4139D5B6086C23130", "10121").unwrap(), *b"12345");
        assert_eq!(signature_name("GB61032A.002").as_deref(), Some("GBN1032A.002"));
        assert_eq!(signature_name("GB100001.000").as_deref(), Some("GBI00001.000"));
    }
}
