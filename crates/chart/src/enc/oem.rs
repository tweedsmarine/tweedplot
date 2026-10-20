//! TweedPlot's S-63 manufacturer credentials, built in by `build.rs` (the IHO's generic
//! test credentials when the build PC has no OEM files), and activation codes.
//!
//! S-63 has the manufacturer (OEM) give each installation a HW_ID, random and recorded
//! in a register, kept from the user, and a user permit made from it. TweedPlot does
//! this with an activation code: the app shows an installation code (a fingerprint of
//! the computer), the OEM's `tweedplot-oem` tool picks a HW_ID, records it, makes the
//! user permit and signs the two together, and the app accepts the code only on the
//! computer with that fingerprint. The HW_ID comes from the user permit (decrypted with
//! the M_KEY) and is never shown.

use base64::Engine;

mod built {
    include!(concat!(env!("OUT_DIR"), "/oem.rs"));
}

/// Whether these are TweedPlot's own credentials (else the IHO test ones).
pub fn is_real() -> bool {
    built::REAL
}

/// The manufacturer ID, two characters ("10" for the IHO test credentials).
pub fn m_id() -> &'static str {
    built::M_ID
}

/// The M_ID as it ends a user permit: its characters in hex ("10" → "3130").
pub fn m_id_hex() -> String {
    built::M_ID.bytes().map(|b| format!("{b:02X}")).collect()
}

/// The manufacturer key, put back together from its masked form.
#[inline(never)]
pub fn m_key() -> String {
    let m = std::hint::black_box(built::M_KEY_MASKED);
    let k = std::hint::black_box(built::M_KEY_MASK);
    m.iter().zip(k.iter()).map(|(a, b)| (a ^ b) as char).collect()
}

const PREFIX: &str = "TPA1-";
const BODY: usize = 1 + 28 + 16 + 8;

/// A checked activation: the user permit, for which computer, and when it was issued.
#[derive(Debug, Clone, PartialEq)]
pub struct Activation {
    pub user_permit: String,
    pub fingerprint: [u8; 16],
    /// YYYYMMDD
    pub issued: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ActivationError {
    #[error("That isn't a TweedPlot activation code (it starts TPA1-). Check it was copied whole")]
    Format,
    #[error("The activation code isn't signed by Tweedsmarine: check it was copied whole")]
    Signature,
    #[error("This activation code is for another computer. Send this computer's installation code to Tweedsmarine for its own")]
    OtherComputer,
    #[error("The user permit in the activation code isn't for this build of TweedPlot (SSE 17)")]
    UserPermit,
}

fn body(a: &Activation) -> Vec<u8> {
    let mut b = vec![1u8];
    b.extend_from_slice(a.user_permit.as_bytes());
    b.extend_from_slice(&a.fingerprint);
    b.extend_from_slice(a.issued.as_bytes());
    b
}

/// Make an activation code (the OEM tool's side) with the OEM's signing key.
pub fn make_activation(a: &Activation, signing_key: &[u8; 32]) -> String {
    use ed25519_dalek::Signer;
    let key = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let mut b = body(a);
    let sig = key.sign(&b);
    b.extend_from_slice(&sig.to_bytes());
    format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b))
}

/// Read an activation code and check its signature against TweedPlot's built-in key.
pub fn read_activation(code: &str) -> Result<Activation, ActivationError> {
    let hex = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect::<Option<Vec<u8>>>();
    let public = hex(built::ACTIVATION_PUBLIC_KEY).and_then(|v| <[u8; 32]>::try_from(v).ok()).ok_or(ActivationError::Signature)?;
    read_activation_with(code, &public)
}

/// [`read_activation`] with a given public key.
pub fn read_activation_with(code: &str, public: &[u8; 32]) -> Result<Activation, ActivationError> {
    let compact: String = code.split_whitespace().collect();
    let b = compact.strip_prefix(PREFIX).and_then(|t| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t).ok()).ok_or(ActivationError::Format)?;
    if b.len() != BODY + 64 || b[0] != 1 {
        return Err(ActivationError::Format);
    }
    let key = ed25519_dalek::VerifyingKey::from_bytes(public).map_err(|_| ActivationError::Signature)?;
    let sig = ed25519_dalek::Signature::from_bytes(b[BODY..].try_into().unwrap());
    key.verify_strict(&b[..BODY], &sig).map_err(|_| ActivationError::Signature)?;
    let text = |r: std::ops::Range<usize>| String::from_utf8(b[r].to_vec()).map_err(|_| ActivationError::Format);
    Ok(Activation { user_permit: text(1..29)?, fingerprint: b[29..45].try_into().unwrap(), issued: text(45..53)? })
}

/// Check an activation for this computer and build: its fingerprint, and that its user
/// permit opens with this build's M_KEY and carries its M_ID. Returns the user permit.
pub fn check_activation(a: &Activation, fingerprint: &[u8; 16]) -> Result<String, ActivationError> {
    if &a.fingerprint != fingerprint {
        return Err(ActivationError::OtherComputer);
    }
    if !a.user_permit.ends_with(&m_id_hex()) || super::s63::hw_id_from_user_permit(&a.user_permit, &m_key()).is_err() {
        return Err(ActivationError::UserPermit);
    }
    Ok(a.user_permit.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test signing key that goes with the test public key in build.rs.
    const TEST_SIGNING: &str = "516407dd9d626dde9462eedbaa853dc0bca8af1430eeb37a16b35c2e9db64a5f";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn activation_round_trip() {
        let key: [u8; 32] = unhex(TEST_SIGNING).try_into().unwrap();
        let fp = [7u8; 16];
        let a = Activation { user_permit: super::super::TEST_USER_PERMIT.into(), fingerprint: fp, issued: "20261021".into() };
        let code = make_activation(&a, &key);
        assert!(code.starts_with("TPA1-"));
        if !is_real() {
            let back = read_activation(&code).unwrap();
            assert_eq!(back, a);
            assert_eq!(check_activation(&back, &fp).unwrap(), super::super::TEST_USER_PERMIT);
            assert_eq!(check_activation(&back, &[8u8; 16]), Err(ActivationError::OtherComputer));
            assert_eq!(m_key(), "10121");
            assert_eq!(m_id_hex(), "3130");
        }
        // one character changed: the signature fails
        let mut bad: Vec<char> = code.chars().collect();
        let i = bad.len() / 2;
        bad[i] = if bad[i] == 'A' { 'B' } else { 'A' };
        let pubkey = ed25519_dalek::SigningKey::from_bytes(&key).verifying_key().to_bytes();
        assert!(read_activation_with(&bad.into_iter().collect::<String>(), &pubkey).is_err());
        assert_eq!(read_activation_with(&code, &pubkey).unwrap(), a);
    }
}
