//! Persists encryption key across app restarts.
//!
//! Priority order for the org KEK:
//! 1. `SHIMMER_DEV_KEY` env var (hex, 64 chars) — dev/test override, skips keychain
//! 2. macOS Keychain (`com.shimmer.app` / `org-kek`)
//! 3. Migration path: if `key.bin` exists in `app_data_dir`, read it, store in
//!    keychain, then delete the plaintext file
//! 4. Generate a new key and store it in the keychain

use std::path::Path;
use tracing::{info, warn};
use zeroize::{Zeroize, ZeroizeOnDrop};

use shimmer_core::encryption::generate_key;

const KEY_FILE: &str = "key.bin";
const KEY_LEN: usize = 32;

const KEYCHAIN_SERVICE: &str = "com.shimmer.app";
const KEYCHAIN_ACCOUNT_KEK: &str = "org-kek";
const KEYCHAIN_ACCOUNT_ACCESS: &str = "jwt-access-token";
const KEYCHAIN_ACCOUNT_REFRESH: &str = "jwt-refresh-token";

/// Holds the org KEK (key-encryption key).
///
/// Always stored behind `Arc<KeyStore>` in Tauri state.  Implements
/// [`ZeroizeOnDrop`] — the key is securely wiped from memory on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct KeyStore {
    key: [u8; KEY_LEN],
}

// ── Keychain helpers ──────────────────────────────────────────────────────────

fn keychain_load(account: &str) -> Option<Vec<u8>> {
    use security_framework::passwords::get_generic_password;
    match get_generic_password(KEYCHAIN_SERVICE, account) {
        Ok(data) => Some(data),
        Err(e) => {
            // -25300 = errSecItemNotFound — not a real error
            if e.code() != -25300 {
                warn!(error = %e, account, "keychain read failed");
            }
            None
        }
    }
}

fn keychain_store(account: &str, data: &[u8]) {
    use security_framework::passwords::set_generic_password;
    if let Err(e) = set_generic_password(KEYCHAIN_SERVICE, account, data) {
        warn!(error = %e, account, "keychain write failed");
    }
}

fn keychain_delete(account: &str) {
    use security_framework::passwords::delete_generic_password;
    if let Err(e) = delete_generic_password(KEYCHAIN_SERVICE, account) {
        // ignore item-not-found
        if e.code() != -25300 {
            warn!(error = %e, account, "keychain delete failed");
        }
    }
}

// ── KeyStore ──────────────────────────────────────────────────────────────────

impl KeyStore {
    pub fn key(&self) -> &[u8; KEY_LEN] {
        &self.key
    }

    /// Override the in-memory key (e.g. after receiving a KEK from an invite
    /// link).  The new key is also persisted to the keychain.
    pub fn set_key(&mut self, key: [u8; KEY_LEN]) {
        self.key = key;
        keychain_store(KEYCHAIN_ACCOUNT_KEK, &key);
    }

    /// Store JWT tokens in the keychain.
    pub fn store_tokens(&self, access: &str, refresh: &str) {
        keychain_store(KEYCHAIN_ACCOUNT_ACCESS, access.as_bytes());
        keychain_store(KEYCHAIN_ACCOUNT_REFRESH, refresh.as_bytes());
    }

    /// Load JWT tokens from the keychain.  Returns `None` if either token is
    /// missing or cannot be decoded as UTF-8.
    pub fn load_tokens(&self) -> Option<(String, String)> {
        let access_bytes = keychain_load(KEYCHAIN_ACCOUNT_ACCESS)?;
        let refresh_bytes = keychain_load(KEYCHAIN_ACCOUNT_REFRESH)?;
        let access = String::from_utf8(access_bytes).ok()?;
        let refresh = String::from_utf8(refresh_bytes).ok()?;
        Some((access, refresh))
    }

    /// Remove the KEK and all stored tokens from the keychain (logout).
    pub fn clear_all(&self) {
        keychain_delete(KEYCHAIN_ACCOUNT_KEK);
        keychain_delete(KEYCHAIN_ACCOUNT_ACCESS);
        keychain_delete(KEYCHAIN_ACCOUNT_REFRESH);
    }

    /// Load or create the org KEK.
    ///
    /// `app_data_dir` is only used for the one-time migration of the legacy
    /// `key.bin` file — it is safe to pass `None`.
    pub fn load_or_create(app_data_dir: Option<&Path>) -> Self {
        // 1. Dev / test env override — highest priority, skips keychain entirely
        if let Ok(hex_str) = std::env::var("SHIMMER_DEV_KEY") {
            if hex_str.len() == 64 {
                if let Ok(decoded) = hex::decode(&hex_str) {
                    if let Ok(key) = <[u8; KEY_LEN]>::try_from(decoded.as_slice()) {
                        info!("encryption key loaded from SHIMMER_DEV_KEY");
                        return Self { key };
                    }
                }
            }
        }

        // 2. Try loading from macOS keychain
        if let Some(bytes) = keychain_load(KEYCHAIN_ACCOUNT_KEK) {
            if let Ok(key) = <[u8; KEY_LEN]>::try_from(bytes.as_slice()) {
                info!("encryption key loaded from keychain");
                return Self { key };
            }
            warn!("keychain entry for org-kek has unexpected length; ignoring");
        }

        // 3. Migration: key.bin → keychain
        if let Some(dir) = app_data_dir {
            let path = dir.join(KEY_FILE);
            if path.exists() {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(key) = <[u8; KEY_LEN]>::try_from(bytes.as_slice()) {
                        info!("migrating encryption key from key.bin to keychain");
                        keychain_store(KEYCHAIN_ACCOUNT_KEK, &key);
                        if let Err(e) = std::fs::remove_file(&path) {
                            warn!(error = %e, "could not remove legacy key.bin after migration");
                        }
                        return Self { key };
                    }
                }
            }
        }

        // 4. Generate a new key and persist to keychain
        info!("generating new encryption key");
        let key = generate_key();
        keychain_store(KEYCHAIN_ACCOUNT_KEK, &key);

        Self { key }
    }
}
