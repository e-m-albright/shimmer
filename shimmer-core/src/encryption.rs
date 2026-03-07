//! AES-256-GCM encryption — envelope encryption for PHI at rest.
//!
//! Each paste gets a unique Data Encryption Key (DEK), which is wrapped
//! by the org's Key Encryption Key (KEK). The server never has the KEK.

use aes_gcm::{aead::Aead, aead::KeyInit, Aes256Gcm};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::Mac;
use rand::RngCore;
use zeroize::Zeroize;

use crate::error::CryptoError;
use crate::KEY_LEN;

const NONCE_LEN: usize = 12;
type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// Envelope-encrypted paste payload — JSON-serialized and stored in cloud storage.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[must_use]
pub struct EnvelopePayload {
    /// Format version.
    pub v: u8,
    /// DEK encrypted with KEK (base64).
    pub wrapped_dek: String,
    /// Nonce used for DEK wrapping (base64).
    pub dek_nonce: String,
    /// Nonce used for data encryption (base64).
    pub data_nonce: String,
    /// Encrypted paste content (base64).
    pub ciphertext: String,
    /// Who created this paste.
    pub user_id: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// Encrypt plaintext using envelope encryption.
///
/// 1. Generate random DEK
/// 2. Encrypt plaintext with DEK
/// 3. Wrap DEK with KEK
///
/// # Errors
///
/// Returns `CryptoError::Encrypt` if encryption fails, or
/// `CryptoError::InvalidKey` if the KEK is malformed.
pub fn encrypt_envelope(
    plaintext: &[u8],
    kek: &[u8; KEY_LEN],
    user_id: &str,
) -> Result<EnvelopePayload, CryptoError> {
    // Generate random DEK — zeroized on drop so it never lingers in memory.
    let mut dek = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut dek);

    // Encrypt data with DEK
    let data_cipher = Aes256Gcm::new_from_slice(&dek)?;
    let mut data_nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut data_nonce);
    let ciphertext = data_cipher.encrypt(&data_nonce.into(), plaintext)?;

    // Wrap DEK with KEK
    let kek_cipher = Aes256Gcm::new_from_slice(kek)?;
    let mut dek_nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut dek_nonce);
    let wrapped_dek = kek_cipher.encrypt(&dek_nonce.into(), dek.as_slice())?;

    // Securely wipe plaintext DEK from memory
    dek.zeroize();

    Ok(EnvelopePayload {
        v: 1,
        wrapped_dek: BASE64.encode(&wrapped_dek),
        dek_nonce: BASE64.encode(dek_nonce),
        data_nonce: BASE64.encode(data_nonce),
        ciphertext: BASE64.encode(&ciphertext),
        user_id: user_id.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Decrypt an envelope payload using the org KEK.
///
/// # Errors
///
/// Returns `CryptoError::Decrypt` if the KEK is wrong or data is corrupt,
/// `CryptoError::Base64` if any base64 field is malformed.
pub fn decrypt_envelope(
    payload: &EnvelopePayload,
    kek: &[u8; KEY_LEN],
) -> Result<Vec<u8>, CryptoError> {
    // Unwrap DEK
    let wrapped_dek = BASE64.decode(&payload.wrapped_dek)?;
    let dek_nonce_bytes = BASE64.decode(&payload.dek_nonce)?;
    let dek_nonce = aes_gcm::Nonce::from_slice(&dek_nonce_bytes);

    let kek_cipher = Aes256Gcm::new_from_slice(kek)?;
    let mut dek_bytes = kek_cipher.decrypt(dek_nonce, wrapped_dek.as_slice())?;

    if dek_bytes.len() != KEY_LEN {
        dek_bytes.zeroize();
        return Err(CryptoError::InvalidKey("DEK has wrong length".into()));
    }

    // Decrypt data with DEK
    let ciphertext = BASE64.decode(&payload.ciphertext)?;
    let data_nonce_bytes = BASE64.decode(&payload.data_nonce)?;
    let data_nonce = aes_gcm::Nonce::from_slice(&data_nonce_bytes);

    let data_cipher = Aes256Gcm::new_from_slice(&dek_bytes)?;
    let plaintext = data_cipher.decrypt(data_nonce, ciphertext.as_slice())?;

    // Securely wipe unwrapped DEK from memory
    dek_bytes.zeroize();

    Ok(plaintext)
}

/// Generate a random 256-bit key.
#[must_use]
pub fn generate_key() -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Derive a search key from the KEK for blind index tokens.
///
/// Deterministic — same KEK always produces same search key.
#[must_use]
pub fn derive_search_key(kek: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    // HMAC key length is unconstrained for HMAC-SHA256, so this cannot fail.
    #[allow(clippy::expect_used)]
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(kek).expect("HMAC-SHA256 accepts any key length");
    mac.update(b"shimmer-search-key-v1");
    let result = mac.finalize().into_bytes();
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&result);
    key
}

/// Compute a blind index token for a search term.
///
/// Deterministic — same term + `search_key` always produces same token.
#[must_use]
pub fn blind_index_token(search_key: &[u8; KEY_LEN], term: &str) -> String {
    let normalized = term.to_lowercase();
    // HMAC key length is unconstrained for HMAC-SHA256, so this cannot fail.
    #[allow(clippy::expect_used)]
    let mut mac = <HmacSha256 as Mac>::new_from_slice(search_key)
        .expect("HMAC-SHA256 accepts any key length");
    mac.update(normalized.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Extract searchable terms from plaintext and return blind index tokens.
#[must_use]
pub fn extract_blind_tokens(plaintext: &str, search_key: &[u8; KEY_LEN]) -> Vec<String> {
    plaintext
        .split(|c: char| c.is_whitespace() || c == '|' || c == ',' || c == ':' || c == ';')
        .filter(|t| t.len() >= 2) // skip single chars
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_'))
        .filter(|t| !t.is_empty())
        .map(|t| blind_index_token(search_key, t))
        .collect::<std::collections::HashSet<_>>() // deduplicate
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn envelope_roundtrip() {
        let kek = generate_key();
        let plain = b"Patient: John Smith, MRN-12345, Type 2 Diabetes";
        let envelope = encrypt_envelope(plain, &kek, "u_test").unwrap();
        let dec = decrypt_envelope(&envelope, &kek).unwrap();
        assert_eq!(dec, plain);
    }

    #[test]
    fn envelope_wrong_kek_fails() {
        let kek1 = generate_key();
        let kek2 = generate_key();
        let envelope = encrypt_envelope(b"secret", &kek1, "u_test").unwrap();
        assert!(decrypt_envelope(&envelope, &kek2).is_err());
    }

    #[test]
    fn blind_index_deterministic() {
        let kek = generate_key();
        let search_key = derive_search_key(&kek);
        let token1 = blind_index_token(&search_key, "diabetes");
        let token2 = blind_index_token(&search_key, "Diabetes");
        let token3 = blind_index_token(&search_key, "DIABETES");
        // All normalized to lowercase → same token
        assert_eq!(token1, token2);
        assert_eq!(token2, token3);
    }

    #[test]
    fn blind_index_different_terms_differ() {
        let kek = generate_key();
        let search_key = derive_search_key(&kek);
        let t1 = blind_index_token(&search_key, "diabetes");
        let t2 = blind_index_token(&search_key, "hypertension");
        assert_ne!(t1, t2);
    }

    #[test]
    fn extract_tokens_from_text() {
        let kek = generate_key();
        let search_key = derive_search_key(&kek);
        let tokens = extract_blind_tokens("John Smith MRN-12345", &search_key);
        assert_eq!(tokens.len(), 3); // "john", "smith", "mrn-12345"
    }

    // =========================================================================
    // Property-based tests — prove invariants hold for arbitrary inputs
    // =========================================================================

    proptest! {
        /// Envelope encryption is a perfect roundtrip for any plaintext.
        #[test]
        fn prop_envelope_roundtrip(plaintext in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let kek = generate_key();
            let envelope = encrypt_envelope(&plaintext, &kek, "u_prop").unwrap();
            let decrypted = decrypt_envelope(&envelope, &kek).unwrap();
            prop_assert_eq!(decrypted, plaintext);
        }

        /// Different plaintexts always produce different ciphertexts (no collisions).
        #[test]
        fn prop_different_plaintexts_different_ciphertexts(
            a in proptest::collection::vec(any::<u8>(), 1..256),
            b in proptest::collection::vec(any::<u8>(), 1..256),
        ) {
            prop_assume!(a != b);
            let kek = generate_key();
            let env_a = encrypt_envelope(&a, &kek, "u_prop").unwrap();
            let env_b = encrypt_envelope(&b, &kek, "u_prop").unwrap();
            prop_assert_ne!(env_a.ciphertext, env_b.ciphertext);
        }

        /// Same plaintext encrypted twice produces different ciphertexts (random nonces).
        #[test]
        fn prop_nonce_uniqueness(plaintext in proptest::collection::vec(any::<u8>(), 1..256)) {
            let kek = generate_key();
            let e1 = encrypt_envelope(&plaintext, &kek, "u_prop").unwrap();
            let e2 = encrypt_envelope(&plaintext, &kek, "u_prop").unwrap();
            // Nonces should differ (with overwhelming probability)
            prop_assert_ne!(e1.data_nonce, e2.data_nonce);
            // Ciphertexts should differ too
            prop_assert_ne!(e1.ciphertext, e2.ciphertext);
        }

        /// Wrong KEK always fails decryption — no partial or silent corruption.
        #[test]
        fn prop_wrong_kek_always_fails(plaintext in proptest::collection::vec(any::<u8>(), 1..256)) {
            let kek1 = generate_key();
            let kek2 = generate_key();
            let envelope = encrypt_envelope(&plaintext, &kek1, "u_prop").unwrap();
            prop_assert!(decrypt_envelope(&envelope, &kek2).is_err());
        }

        /// Blind index tokens are deterministic: same key + term → same token.
        #[test]
        fn prop_blind_index_deterministic(term in "[a-zA-Z0-9]{2,32}") {
            let kek = generate_key();
            let search_key = derive_search_key(&kek);
            let t1 = blind_index_token(&search_key, &term);
            let t2 = blind_index_token(&search_key, &term);
            prop_assert_eq!(t1, t2);
        }

        /// Blind index is case-insensitive: "Foo" and "foo" produce same token.
        #[test]
        fn prop_blind_index_case_insensitive(term in "[a-zA-Z]{2,32}") {
            let kek = generate_key();
            let search_key = derive_search_key(&kek);
            let lower = blind_index_token(&search_key, &term.to_lowercase());
            let upper = blind_index_token(&search_key, &term.to_uppercase());
            prop_assert_eq!(lower, upper);
        }

        /// Envelope payload is valid JSON that can roundtrip through serde.
        #[test]
        fn prop_envelope_serialization(plaintext in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let kek = generate_key();
            let envelope = encrypt_envelope(&plaintext, &kek, "u_prop").unwrap();
            let json = serde_json::to_string(&envelope).unwrap();
            let parsed: EnvelopePayload = serde_json::from_str(&json).unwrap();
            let decrypted = decrypt_envelope(&parsed, &kek).unwrap();
            prop_assert_eq!(decrypted, plaintext);
        }
    }
}
