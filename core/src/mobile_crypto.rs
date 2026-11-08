// SPDX-License-Identifier: Apache-2.0
//! Authenticated application-layer transport for mobile-to-host traffic.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MOBILE_CRYPTO_AAD: &[u8] = b"nola-mobile-bridge-v1";
const MAX_MOBILE_PLAINTEXT_BYTES: usize = 8 * 1024 * 1024;
pub const MOBILE_CRYPTO_VERSION: u32 = 1;
pub const MOBILE_CRYPTO_ALGORITHM: &str = "AES-256-GCM";
pub const MOBILE_AUTH_VERSION: &str = "v1";

pub fn generate_mobile_access_token() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

pub fn generate_protocol_nonce(prefix: &str) -> String {
    format!(
        "{}-{}",
        prefix.trim_matches('-'),
        URL_SAFE_NO_PAD.encode(random_bytes::<24>())
    )
}

pub fn mobile_body_sha256(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

pub fn constant_time_secret_eq(left: &str, right: &str) -> bool {
    let left_digest = Sha256::digest(left.as_bytes());
    let right_digest = Sha256::digest(right.as_bytes());
    let mut difference = left.len() ^ right.len();
    for (left, right) in left_digest.iter().zip(right_digest.iter()) {
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MobileEncryptedEnvelope {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub v: u32,
    pub alg: String,
    pub nonce: String,
    pub ciphertext: String,
    pub tag: String,
}

pub fn encrypt_mobile_json(value: &Value, token: &str) -> Result<MobileEncryptedEnvelope, String> {
    let plaintext =
        serde_json::to_vec(value).map_err(|_| "failed to encode mobile JSON".to_string())?;
    if plaintext.len() > MAX_MOBILE_PLAINTEXT_BYTES {
        return Err("mobile JSON payload is too large".to_string());
    }
    let key = Sha256::digest(token.as_bytes());
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "invalid mobile encryption key".to_string())?;
    let nonce_bytes: [u8; 12] = rand_nonce();
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: &plaintext,
                aad: MOBILE_CRYPTO_AAD,
            },
        )
        .map_err(|_| "failed to encrypt mobile JSON".to_string())?;
    let tag_offset = sealed
        .len()
        .checked_sub(16)
        .ok_or_else(|| "invalid encrypted mobile JSON".to_string())?;
    Ok(MobileEncryptedEnvelope {
        kind: Some("encrypted".to_string()),
        v: MOBILE_CRYPTO_VERSION,
        alg: MOBILE_CRYPTO_ALGORITHM.to_string(),
        nonce: URL_SAFE_NO_PAD.encode(nonce_bytes),
        ciphertext: URL_SAFE_NO_PAD.encode(&sealed[..tag_offset]),
        tag: URL_SAFE_NO_PAD.encode(&sealed[tag_offset..]),
    })
}

pub fn decrypt_mobile_json(
    envelope: &MobileEncryptedEnvelope,
    token: &str,
) -> Result<Value, String> {
    if envelope.v != MOBILE_CRYPTO_VERSION || envelope.alg != MOBILE_CRYPTO_ALGORITHM {
        return Err("unsupported encrypted mobile payload".to_string());
    }
    let nonce = URL_SAFE_NO_PAD
        .decode(&envelope.nonce)
        .map_err(|_| "invalid mobile nonce".to_string())?;
    let ciphertext = URL_SAFE_NO_PAD
        .decode(&envelope.ciphertext)
        .map_err(|_| "invalid mobile ciphertext".to_string())?;
    let tag = URL_SAFE_NO_PAD
        .decode(&envelope.tag)
        .map_err(|_| "invalid mobile tag".to_string())?;
    if nonce.len() != 12 || tag.len() != 16 || ciphertext.len() > MAX_MOBILE_PLAINTEXT_BYTES {
        return Err("invalid encrypted mobile payload sizes".to_string());
    }
    let key = Sha256::digest(token.as_bytes());
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "invalid mobile encryption key".to_string())?;
    let mut sealed = ciphertext;
    sealed.extend_from_slice(&tag);
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &sealed,
                aad: MOBILE_CRYPTO_AAD,
            },
        )
        .map_err(|_| "mobile payload authentication failed".to_string())?;
    if plaintext.len() > MAX_MOBILE_PLAINTEXT_BYTES {
        return Err("decrypted mobile payload is too large".to_string());
    }
    serde_json::from_slice(&plaintext)
        .map_err(|_| "decrypted mobile payload is not JSON".to_string())
}

pub fn mobile_auth_message(
    method: &str,
    path: &str,
    timestamp: u64,
    nonce: &str,
    body_sha256: &str,
) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        MOBILE_AUTH_VERSION,
        method.trim().to_ascii_uppercase(),
        path.trim(),
        timestamp,
        nonce.trim(),
        body_sha256.trim().to_ascii_lowercase()
    )
}

pub fn mobile_auth_proof(
    token: &str,
    method: &str,
    path: &str,
    timestamp: u64,
    nonce: &str,
    body_sha256: &str,
) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(mobile_auth_message(method, path, timestamp, nonce, body_sha256).as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

pub fn verify_mobile_auth_proof(
    token: &str,
    method: &str,
    path: &str,
    timestamp: u64,
    nonce: &str,
    body_sha256: &str,
    proof: &str,
) -> bool {
    let Ok(proof) = URL_SAFE_NO_PAD.decode(proof) else {
        return false;
    };
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(mobile_auth_message(method, path, timestamp, nonce, body_sha256).as_bytes());
    mac.verify_slice(&proof).is_ok()
}

fn rand_nonce() -> [u8; 12] {
    random_bytes::<12>()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    use aes_gcm::aead::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encrypted_mobile_json_round_trips_and_rejects_tampering() {
        let value = json!({"message":"private","count":2});
        let mut envelope = encrypt_mobile_json(&value, "token").unwrap();
        assert_eq!(decrypt_mobile_json(&envelope, "token").unwrap(), value);
        envelope.tag.replace_range(
            0..1,
            if envelope.tag.starts_with('A') {
                "B"
            } else {
                "A"
            },
        );
        assert!(decrypt_mobile_json(&envelope, "token").is_err());
    }

    #[test]
    fn request_proof_binds_method_path_timestamp_and_nonce() {
        let body = hex::encode(Sha256::digest(b"body"));
        let proof = mobile_auth_proof("token", "POST", "/api/mobile/chat", 100, "nonce", &body);
        assert!(verify_mobile_auth_proof(
            "token",
            "POST",
            "/api/mobile/chat",
            100,
            "nonce",
            &body,
            &proof
        ));
        assert!(!verify_mobile_auth_proof(
            "token",
            "GET",
            "/api/mobile/chat",
            100,
            "nonce",
            &body,
            &proof
        ));
        assert!(!verify_mobile_auth_proof(
            "token",
            "POST",
            "/api/mobile/other",
            100,
            "nonce",
            &body,
            &proof
        ));
        assert!(!verify_mobile_auth_proof(
            "token",
            "POST",
            "/api/mobile/chat",
            100,
            "nonce",
            &"0".repeat(64),
            &proof
        ));
    }

    #[test]
    fn secret_comparison_rejects_length_and_content_changes() {
        assert!(constant_time_secret_eq(
            "pairing-secret-123",
            "pairing-secret-123"
        ));
        assert!(!constant_time_secret_eq(
            "pairing-secret-123",
            "pairing-secret-124"
        ));
        assert!(!constant_time_secret_eq(
            "pairing-secret-123",
            "pairing-secret-1234"
        ));
    }

    #[test]
    fn generated_tokens_and_nonces_are_random_and_url_safe() {
        let first = generate_mobile_access_token();
        let second = generate_mobile_access_token();
        assert_ne!(first, second);
        assert!(first.len() >= 43);
        let nonce = generate_protocol_nonce("permission");
        assert!(nonce.starts_with("permission-"));
        assert!(nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')));
    }
}
