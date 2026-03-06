//! AES-256-GCM client-side encryption for PHI at rest.

use aes_gcm::{aead::Aead, aead::KeyInit, Aes256Gcm};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use std::error::Error;

const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// Encrypt plaintext with AES-256-GCM.
/// Output format: nonce (12 bytes) || ciphertext || tag (16 bytes), all base64-encoded.
pub fn encrypt(plaintext: &[u8], key: &[u8; KEY_LEN]) -> Result<String, Box<dyn Error + Send + Sync>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);

    let ciphertext = cipher
        .encrypt(&nonce.into(), plaintext)
        .map_err(|e| e.to_string())?;

    let mut combined = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    combined.extend_from_slice(&nonce);
    combined.extend_from_slice(&ciphertext);

    Ok(BASE64.encode(&combined))
}

/// Decrypt ciphertext produced by encrypt().
pub fn decrypt(encoded: &str, key: &[u8; KEY_LEN]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let combined = BASE64.decode(encoded).map_err(|e| e.to_string())?;
    if combined.len() < NONCE_LEN {
        return Err("Invalid ciphertext".into());
    }

    let (nonce_bytes, ciphertext) = combined.split_at(NONCE_LEN);
    let nonce = aes_gcm::Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| e.to_string())?;

    Ok(plaintext)
}

/// Generate a random 256-bit key. In production, derive from OIDC token or KMS.
/// TODO(OIDC): Derive key from SSO session / HSM.
pub fn generate_key() -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = generate_key();
        let plain = b"Hello PHI";
        let enc = encrypt(plain, &key).unwrap();
        let dec = decrypt(&enc, &key).unwrap();
        assert_eq!(dec, plain);
    }
}
