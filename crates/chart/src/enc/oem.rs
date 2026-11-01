//! TweedPlot's S-63 manufacturer identity (built in by `build.rs`), installation codes,
//! activation codes and release codes.
//!
//! S-63 has the manufacturer (OEM) give each installation a HW_ID, random, recorded in a
//! register and kept from the user, and a user permit made from it with the M_KEY.
//! TweedPlot does this with activation codes made by Tweedsmarine's Licence Manager:
//!
//! - The app shows an **installation code** (TP2-…): this computer's fingerprint and the
//!   public half of this installation's device key ([`super::device`]).
//! - The Licence Manager picks the HW_ID (or reuses the licence's one for another seat on
//!   the same vessel), makes the user permit with the M_KEY, encrypts the HW_ID to the
//!   device key and signs it all: the **activation code** (TPA2-…). It also names the
//!   vessel, MMSI, licence and seat.
//! - The app accepts the code only on the computer and installation it was made for, and
//!   reads the HW_ID out of it with its device key. The HW_ID is never shown.
//! - **The M_KEY is not in the app at all.** Only the Licence Manager has it.
//! - Removing an activation gives a **release code** (TPR2-…), keyed with the HW_ID, so
//!   the Licence Manager can free the seat knowing it came from that installation.

use base64::Engine;
use hmac::Mac;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::device::Device;

mod built {
    include!(concat!(env!("OUT_DIR"), "/oem.rs"));
}

/// The signing key that goes with the test public key in build.rs: for test builds and
/// the Licence Manager's test mode (it is public, like the IHO test credentials).
pub const TEST_SIGNING_KEY: &str = "516407dd9d626dde9462eedbaa853dc0bca8af1430eeb37a16b35c2e9db64a5f";

/// Whether this build has TweedPlot's own identity (else the IHO test credentials).
pub fn is_real() -> bool {
    built::REAL
}

/// The manufacturer ID, two characters ("10" for the IHO test credentials).
pub fn m_id() -> &'static str {
    built::M_ID
}

/// The M_ID as it ends a user permit: its characters in hex ("10" → "3130").
pub fn m_id_hex() -> String {
    hex_of(built::M_ID)
}

/// Any M_ID as it ends a user permit.
pub fn hex_of(m_id: &str) -> String {
    m_id.bytes().map(|b| format!("{b:02X}")).collect()
}

/// The public key activation codes are checked with in this build.
pub fn public_key() -> [u8; 32] {
    unhex(built::ACTIVATION_PUBLIC_KEY).and_then(|v| v.try_into().ok()).unwrap_or([0; 32])
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

// ---- base32 (Crockford): easy to read out, O/I/L read as 0/1 ------------------------

const B32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn b32_encode(b: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &x in b {
        acc = (acc << 8) | x as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn b32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.chars() {
        let c = match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            '-' | ' ' | '\t' | '\r' | '\n' => continue,
            c => c,
        };
        let v = B32.iter().position(|&x| x as char == c)? as u32;
        acc = ((acc << 5) | v) & 0xFFFF;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn grouped(s: &str, n: usize) -> String {
    s.as_bytes().chunks(n).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

fn check2(b: &[u8]) -> [u8; 2] {
    let d = Sha256::digest(b);
    [d[0], d[1]]
}

// ---- installation codes -----------------------------------------------------------

/// What an installation code says: the computer's fingerprint and the installation's
/// device public key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Installation {
    pub fingerprint: [u8; 16],
    pub device: [u8; 32],
}

/// The installation code the user sends: "TP2-" and 16 groups of five characters.
pub fn installation_code(fingerprint: &[u8; 16], device: &[u8; 32]) -> String {
    let mut b = fingerprint.to_vec();
    b.extend_from_slice(device);
    let c = check2(&b);
    b.extend_from_slice(&c);
    format!("TP2-{}", grouped(&b32_encode(&b), 5))
}

/// Read an installation code (the Licence Manager's side). None if it isn't whole.
pub fn read_installation_code(code: &str) -> Option<Installation> {
    let t = code.trim();
    let body = t.strip_prefix("TP2-").or_else(|| t.strip_prefix("tp2-"))?;
    let b = b32_decode(body)?;
    if b.len() != 50 || check2(&b[..48]) != b[48..50] {
        return None;
    }
    Some(Installation { fingerprint: b[..16].try_into().ok()?, device: b[16..48].try_into().ok()? })
}

// ---- activation codes -------------------------------------------------------------

const PREFIX: &str = "TPA2-";
const VERSION: u8 = 2;
const MAX_VESSEL: usize = 60;

/// What the Licence Manager grants one installation (everything in an activation code
/// except the HW_ID, which goes in encrypted).
#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub user_permit: String,
    pub installation: Installation,
    /// YYYYMMDD
    pub issued: String,
    pub vessel: String,
    /// nine digits, or empty
    pub mmsi: String,
    pub licence: u32,
    pub seat: u8,
}

/// A checked activation code.
#[derive(Debug, Clone, PartialEq)]
pub struct Activation {
    pub grant: Grant,
    /// The ephemeral public key and the encrypted HW_ID (5 bytes + 16 tag).
    sealed: ([u8; 32], Vec<u8>),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ActivationError {
    #[error("That isn't a TweedPlot activation code (it starts TPA2-). Check it was copied whole")]
    Format,
    #[error("The activation code isn't signed by Tweedsmarine: check it was copied whole")]
    Signature,
    #[error("This activation code is for another computer. Send this computer's installation code to Tweedsmarine for its own")]
    OtherComputer,
    #[error("This activation code was made for an earlier installation of TweedPlot on this computer. Send the installation code shown now to Tweedsmarine")]
    OtherInstallation,
    #[error("The user permit in the activation code isn't for this build of TweedPlot (SSE 17)")]
    UserPermit,
}

fn hw_key(shared: &[u8; 32], eph: &[u8; 32], inst: &Installation) -> Zeroizing<[u8; 32]> {
    let mut h = Sha256::new();
    h.update(b"TweedPlot HW_ID 2");
    h.update(shared);
    h.update(eph);
    h.update(inst.device);
    h.update(inst.fingerprint);
    Zeroizing::new(h.finalize().into())
}

fn seal_hw(hw: &[u8; 5], inst: &Installation) -> ([u8; 32], Vec<u8>) {
    use chacha20poly1305::aead::{Aead, KeyInit};
    let mut s = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut s[..]).expect("random numbers");
    let eph = Device::from_secret(*s);
    let key = hw_key(&eph.shared(&inst.device), &eph.public, inst);
    let c = chacha20poly1305::ChaCha20Poly1305::new(key[..].into());
    // the key is new for every code, so a fixed nonce is safe
    let ct = c.encrypt(&[0u8; 12].into(), &hw[..]).expect("encrypt");
    (eph.public, ct)
}

fn body(g: &Grant, sealed: &([u8; 32], Vec<u8>)) -> Result<Vec<u8>, ActivationError> {
    if g.user_permit.len() != 28 || g.issued.len() != 8 || !(g.mmsi.is_empty() || g.mmsi.len() == 9) || sealed.1.len() != 21 {
        return Err(ActivationError::Format);
    }
    let mut v = g.vessel.trim().to_string();
    while v.len() > MAX_VESSEL {
        v.pop();
    }
    let mut b = vec![VERSION];
    b.extend_from_slice(g.user_permit.as_bytes());
    b.extend_from_slice(&g.installation.fingerprint);
    b.extend_from_slice(&g.installation.device);
    b.extend_from_slice(g.issued.as_bytes());
    b.extend_from_slice(&g.licence.to_be_bytes());
    b.push(g.seat);
    b.extend_from_slice(format!("{:9}", g.mmsi).as_bytes());
    b.push(v.len() as u8);
    b.extend_from_slice(v.as_bytes());
    b.extend_from_slice(&sealed.0);
    b.extend_from_slice(&sealed.1);
    Ok(b)
}

/// Make an activation code (the Licence Manager's side) with Tweedsmarine's signing key.
pub fn make_activation(g: &Grant, hw: &[u8; 5], signing_key: &[u8; 32]) -> Result<String, ActivationError> {
    use ed25519_dalek::Signer;
    let sealed = seal_hw(hw, &g.installation);
    let mut b = body(g, &sealed)?;
    let key = ed25519_dalek::SigningKey::from_bytes(signing_key);
    b.extend_from_slice(&key.sign(&b).to_bytes());
    Ok(format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)))
}

/// Read an activation code and check its signature against this build's public key.
pub fn read_activation(code: &str) -> Result<Activation, ActivationError> {
    read_activation_with(code, &public_key())
}

/// [`read_activation`] with a given public key.
pub fn read_activation_with(code: &str, public: &[u8; 32]) -> Result<Activation, ActivationError> {
    let compact: String = code.split_whitespace().collect();
    let b = compact.strip_prefix(PREFIX).and_then(|t| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t).ok()).ok_or(ActivationError::Format)?;
    // fixed part: 1 + 28 + 16 + 32 + 8 + 4 + 1 + 9 + 1, then the vessel, 32 + 21, 64
    const FIXED: usize = 100;
    if b.len() < FIXED + 53 + 64 || b[0] != VERSION {
        return Err(ActivationError::Format);
    }
    let vl = b[FIXED - 1] as usize;
    let end = FIXED + vl + 53;
    if b.len() != end + 64 {
        return Err(ActivationError::Format);
    }
    let key = ed25519_dalek::VerifyingKey::from_bytes(public).map_err(|_| ActivationError::Signature)?;
    let sig = ed25519_dalek::Signature::from_bytes(b[end..].try_into().unwrap());
    key.verify_strict(&b[..end], &sig).map_err(|_| ActivationError::Signature)?;
    let text = |r: std::ops::Range<usize>| String::from_utf8(b[r].to_vec()).map_err(|_| ActivationError::Format);
    let grant = Grant {
        user_permit: text(1..29)?,
        installation: Installation { fingerprint: b[29..45].try_into().unwrap(), device: b[45..77].try_into().unwrap() },
        issued: text(77..85)?,
        licence: u32::from_be_bytes(b[85..89].try_into().unwrap()),
        seat: b[89],
        mmsi: text(90..99)?.trim().to_string(),
        vessel: text(FIXED..FIXED + vl)?,
    };
    let s = FIXED + vl;
    Ok(Activation { grant, sealed: (b[s..s + 32].try_into().unwrap(), b[s + 32..s + 53].to_vec()) })
}

/// Whether a user permit holds together (its check sum over the encrypted HW_ID) and
/// carries this M_ID. Needs no M_KEY.
pub fn user_permit_ok(up: &str, m_id_hex: &str) -> bool {
    let p = up.trim();
    p.len() == 28
        && p.is_ascii()
        && format!("{:08X}", crc32fast::hash(&p.as_bytes()[..16])) == p[16..24].to_ascii_uppercase()
        && p[24..].eq_ignore_ascii_case(m_id_hex)
}

impl Activation {
    /// Check the activation for this computer, installation and build, and read the
    /// HW_ID out of it with the device key.
    pub fn open(&self, fingerprint: &[u8; 16], device: &Device) -> Result<Zeroizing<[u8; 5]>, ActivationError> {
        use chacha20poly1305::aead::{Aead, KeyInit};
        let inst = &self.grant.installation;
        if &inst.fingerprint != fingerprint {
            return Err(ActivationError::OtherComputer);
        }
        if inst.device != device.public {
            return Err(ActivationError::OtherInstallation);
        }
        if !user_permit_ok(&self.grant.user_permit, &m_id_hex()) {
            return Err(ActivationError::UserPermit);
        }
        let key = hw_key(&device.shared(&self.sealed.0), &self.sealed.0, inst);
        let c = chacha20poly1305::ChaCha20Poly1305::new(key[..].into());
        let plain = Zeroizing::new(c.decrypt(&[0u8; 12].into(), &self.sealed.1[..]).map_err(|_| ActivationError::OtherInstallation)?);
        let hw: [u8; 5] = plain[..].try_into().map_err(|_| ActivationError::Format)?;
        Ok(Zeroizing::new(hw))
    }
}

// ---- release codes ----------------------------------------------------------------

/// A seat given back: which installation, licence and seat, and when.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub fingerprint: [u8; 16],
    pub licence: u32,
    pub seat: u8,
    /// YYYYMMDD
    pub date: String,
    mac: [u8; 16],
}

fn release_mac(hw: &[u8; 5], b: &[u8]) -> [u8; 16] {
    let mut m = <hmac::Hmac<Sha256> as Mac>::new_from_slice(hw).expect("any key length");
    m.update(b"TweedPlot release 2");
    m.update(b);
    m.finalize().into_bytes()[..16].try_into().unwrap()
}

fn release_body(fingerprint: &[u8; 16], licence: u32, seat: u8, date: &str) -> Vec<u8> {
    let mut b = fingerprint.to_vec();
    b.extend_from_slice(&licence.to_be_bytes());
    b.push(seat);
    b.extend_from_slice(date.as_bytes());
    b
}

/// The release code an installation shows when its activation is removed.
pub fn release_code(g: &Grant, hw: &[u8; 5], date: &str) -> String {
    let mut b = release_body(&g.installation.fingerprint, g.licence, g.seat, date);
    let mac = release_mac(hw, &b);
    b.extend_from_slice(&mac);
    let c = check2(&b);
    b.extend_from_slice(&c);
    format!("TPR2-{}", grouped(&b32_encode(&b), 5))
}

/// Read a release code (the Licence Manager's side); check it with [`Release::verify`].
pub fn read_release(code: &str) -> Option<Release> {
    let t = code.trim();
    let body = t.strip_prefix("TPR2-").or_else(|| t.strip_prefix("tpr2-"))?;
    let b = b32_decode(body)?;
    if b.len() != 47 || check2(&b[..45]) != b[45..47] {
        return None;
    }
    Some(Release {
        fingerprint: b[..16].try_into().ok()?,
        licence: u32::from_be_bytes(b[16..20].try_into().ok()?),
        seat: b[20],
        date: String::from_utf8(b[21..29].to_vec()).ok()?,
        mac: b[29..45].try_into().ok()?,
    })
}

impl Release {
    /// Whether the release was made by an installation holding this HW_ID.
    pub fn verify(&self, hw: &[u8; 5]) -> bool {
        release_mac(hw, &release_body(&self.fingerprint, self.licence, self.seat, &self.date)) == self.mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        unhex(TEST_SIGNING_KEY).unwrap().try_into().unwrap()
    }

    fn grant(dev: &Device) -> Grant {
        Grant {
            user_permit: super::super::TEST_USER_PERMIT.into(),
            installation: Installation { fingerprint: [7u8; 16], device: dev.public },
            issued: "20261102".into(),
            vessel: "Morgane".into(),
            mmsi: "512000123".into(),
            licence: 42,
            seat: 2,
        }
    }

    #[test]
    fn installation_codes_round_trip() {
        let d = Device::from_secret([3u8; 32]);
        let c = installation_code(&[0x12u8; 16], &d.public);
        assert!(c.starts_with("TP2-"));
        assert_eq!(c.len(), 4 + 16 * 5 + 15);
        let i = read_installation_code(&c).unwrap();
        assert_eq!((i.fingerprint, i.device), ([0x12u8; 16], d.public));
        // lower case, O for 0 and line breaks are fine
        assert_eq!(read_installation_code(&c.to_lowercase().replace('0', "o").replace('-', "-\r\n")), Some(i));
        // a typo is caught
        let mut bad: Vec<char> = c.chars().collect();
        bad[10] = if bad[10] == 'A' { 'B' } else { 'A' };
        assert_eq!(read_installation_code(&bad.into_iter().collect::<String>()), None);
    }

    #[test]
    fn activation_round_trip() {
        let dev = Device::from_secret([5u8; 32]);
        let g = grant(&dev);
        let code = make_activation(&g, b"12345", &test_key()).unwrap();
        assert!(code.starts_with("TPA2-"));
        let pubkey = ed25519_dalek::SigningKey::from_bytes(&test_key()).verifying_key().to_bytes();
        let a = read_activation_with(&code, &pubkey).unwrap();
        assert_eq!(a.grant, g);
        if !is_real() {
            assert_eq!(read_activation(&code).unwrap(), a);
            assert_eq!(*a.open(&[7u8; 16], &dev).unwrap(), *b"12345");
        }
        // the HW_ID is not in the code in the clear
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&code[5..]).unwrap();
        assert!(!raw.windows(5).any(|w| w == b"12345"));
        // another computer, another installation on this computer
        assert_eq!(a.open(&[8u8; 16], &dev), Err(ActivationError::OtherComputer));
        assert_eq!(a.open(&[7u8; 16], &Device::from_secret([6u8; 32])), Err(ActivationError::OtherInstallation));
        // one character changed: the signature fails
        let mut bad: Vec<char> = code.chars().collect();
        let i = bad.len() / 2;
        bad[i] = if bad[i] == 'A' { 'B' } else { 'A' };
        assert!(read_activation_with(&bad.into_iter().collect::<String>(), &pubkey).is_err());
        // a long vessel name is cut, no MMSI is fine
        let g2 = Grant { vessel: "x".repeat(80), mmsi: String::new(), ..g.clone() };
        let a2 = read_activation_with(&make_activation(&g2, b"12345", &test_key()).unwrap(), &pubkey).unwrap();
        assert_eq!((a2.grant.vessel.len(), a2.grant.mmsi.as_str()), (60, ""));
    }

    #[test]
    fn release_codes() {
        let dev = Device::from_secret([5u8; 32]);
        let g = grant(&dev);
        let c = release_code(&g, b"12345", "20261103");
        let r = read_release(&c).unwrap();
        assert_eq!((r.licence, r.seat, r.date.as_str(), r.fingerprint), (42, 2, "20261103", [7u8; 16]));
        assert!(r.verify(b"12345"));
        assert!(!r.verify(b"12346"));
        assert!(read_release(&c.replacen('2', "3", 2)).is_none());
    }

    #[test]
    fn user_permit_check() {
        assert!(user_permit_ok(super::super::TEST_USER_PERMIT, "3130"));
        assert!(!user_permit_ok(super::super::TEST_USER_PERMIT, "3131"));
        assert!(!user_permit_ok("66B5CBFDF7E4139D5B6086C33130", "3130"));
    }
}
