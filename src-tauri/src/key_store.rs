//! Persists encryption key across app restarts.
//! Uses app data dir when `SHIMMER_DEV_KEY` is not set.

use std::path::Path;
use tracing::{info, warn};
use zeroize::{Zeroize, ZeroizeOnDrop};

use shimmer_core::encryption::generate_key;

const KEY_FILE: &str = "key.bin";
const KEY_LEN: usize = 32;

/// Holds the encryption key. Persisted to app data dir when not using `SHIMMER_DEV_KEY`.
///
/// Always stored behind `Arc<KeyStore>` in Tauri state — the key itself is a plain
/// array, no inner `Arc` needed since `KeyStore` is never cloned independently.
///
/// Implements [`ZeroizeOnDrop`] — the key is securely wiped from memory when the
/// `KeyStore` is dropped, preventing key material from lingering on the heap.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct KeyStore {
    key: [u8; KEY_LEN],
}

impl KeyStore {
    pub fn key(&self) -> &[u8; KEY_LEN] {
        &self.key
    }

    /// Load or create key: env `SHIMMER_DEV_KEY` > persisted file > generate new.
    pub fn load_or_create(app_data_dir: Option<&Path>) -> Self {
        // 1. Env override (explicit user config)
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

        // 2. Load from persisted file
        if let Some(dir) = app_data_dir {
            let path = dir.join(KEY_FILE);
            if path.exists() {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(key) = <[u8; KEY_LEN]>::try_from(bytes.as_slice()) {
                        info!("encryption key loaded from persisted file");
                        return Self { key };
                    }
                }
            }
        }

        // 3. Generate and persist
        info!("generating new encryption key");
        let key = generate_key();
        if let Some(dir) = app_data_dir {
            if let Err(e) = std::fs::create_dir_all(dir) {
                warn!(error = %e, "could not create app data dir");
            } else {
                let path = dir.join(KEY_FILE);
                if let Err(e) = std::fs::write(&path, key) {
                    warn!(error = %e, "could not persist key");
                }
            }
        }

        Self { key }
    }
}
