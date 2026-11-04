//! Getting a free Open Waters token (https://openwaters.io/api/ais/): a personal token is
//! made by POSTing an Ed25519 public key, signed by the same key, to `/v1/keys`. The
//! private key (a 32-byte seed) is kept by the caller, so the same station can be asked for again.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;

pub const KEYS_URL: &str = "https://ais.openwaters.io/v1/keys";

/// A new random private key, base64url.
pub fn new_seed() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(B64.encode(b))
}

/// The JSON body for `POST /v1/keys` for this seed at unix time `ts`. `name` is the public
/// station name (blank for none). The signature covers `aiscast-key`, the public key, the
/// time, `0` (no IP binding), the name and an empty vessel name, joined by newlines.
pub fn key_request(seed: &str, ts: i64, name: &str) -> Result<String, String> {
    let raw = B64.decode(seed.trim()).map_err(|e| format!("bad key: {e}"))?;
    let raw: [u8; 32] = raw.try_into().map_err(|_| "bad key length".to_string())?;
    let key = SigningKey::from_bytes(&raw);
    let public = B64.encode(key.verifying_key().to_bytes());
    let name = name.trim();
    let msg = format!("aiscast-key\n{public}\n{ts}\n0\n{name}\n");
    let sig = B64.encode(key.sign(msg.as_bytes()).to_bytes());
    let mut body = json!({ "pubkey": public, "ts": ts, "sig": sig });
    if !name.is_empty() {
        body["name"] = json!(name);
    }
    Ok(body.to_string())
}

/// The token from a `/v1/keys` response, or the server's reason it refused.
pub fn token_of(response: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(response).map_err(|_| "unexpected reply from Open Waters".to_string())?;
    if let Some(t) = v.get("token").and_then(|t| t.as_str()) {
        return Ok(t.to_string());
    }
    Err(v.get("error").and_then(|e| e.as_str()).unwrap_or("Open Waters refused the request").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};

    #[test]
    fn request_is_signed_over_the_documented_lines() {
        let seed = new_seed().unwrap();
        let body: serde_json::Value = serde_json::from_str(&key_request(&seed, 1790000000, "").unwrap()).unwrap();
        let public = body["pubkey"].as_str().unwrap();
        assert_eq!(public.len(), 43);
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&B64.decode(public).unwrap().try_into().unwrap()).unwrap();
        let sig = Signature::from_bytes(&B64.decode(body["sig"].as_str().unwrap()).unwrap().try_into().unwrap());
        // the example in the API docs: printf 'aiscast-key\n%s\n%s\n0\n\n' (name and vessel name empty)
        assert!(vk.verify(format!("aiscast-key\n{public}\n1790000000\n0\n\n").as_bytes(), &sig).is_ok());
        assert!(body.get("name").is_none());
    }

    #[test]
    fn name_is_signed_and_sent() {
        let seed = new_seed().unwrap();
        let body: serde_json::Value = serde_json::from_str(&key_request(&seed, 5, " Tweed ").unwrap()).unwrap();
        assert_eq!(body["name"], "Tweed");
        let public = body["pubkey"].as_str().unwrap();
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&B64.decode(public).unwrap().try_into().unwrap()).unwrap();
        let sig = Signature::from_bytes(&B64.decode(body["sig"].as_str().unwrap()).unwrap().try_into().unwrap());
        assert!(vk.verify(format!("aiscast-key\n{public}\n5\n0\nTweed\n").as_bytes(), &sig).is_ok());
    }

    #[test]
    fn replies() {
        assert_eq!(token_of(r#"{"token":"ak1.x","claims":{}}"#).unwrap(), "ak1.x");
        assert_eq!(token_of(r#"{"error":"too many requests"}"#).unwrap_err(), "too many requests");
    }
}
