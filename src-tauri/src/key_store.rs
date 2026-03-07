//! Persists encryption key across app restarts.
//! Uses app data dir when SHIMMER_DEV_KEY is not set.

use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use crate::encryption::generate_key;

const KEY_FILE: &str = "key.bin";
const KEY_LEN: usize = 32;

/// Holds the encryption key. Persisted to app data dir when not using SHIMMER_DEV_KEY.
#[derive(Clone)]
pub struct KeyStore {
    key: Arc<[u8; KEY_LEN]>,
}

impl KeyStore {
    pub fn key(&self) -> &[u8; KEY_LEN] {
        &self.key
    }

    /// Load or create key: env SHIMMER_DEV_KEY > persisted file > generate new.
    pub fn load_or_create(app_data_dir: Option<&Path>) -> Self {
        // 1. Env override (explicit user config)
        if let Ok(hex) = std::env::var("SHIMMER_DEV_KEY") {
            if hex.len() == 64 {
                if let Ok(decoded) = hex::decode(&hex) {
                    if decoded.len() == KEY_LEN {
                        let mut key = [0u8; KEY_LEN];
                        key.copy_from_slice(&decoded);
                        info!("encryption key loaded from SHIMMER_DEV_KEY");
                        return Self {
                            key: Arc::new(key),
                        };
                    }
                }
            }
        }

        // 2. Load from persisted file
        if let Some(dir) = app_data_dir {
            let path = dir.join(KEY_FILE);
            if path.exists() {
                if let Ok(bytes) = std::fs::read(&path) {
                    if bytes.len() == KEY_LEN {
                        let mut key = [0u8; KEY_LEN];
                        key.copy_from_slice(&bytes);
                        info!("encryption key loaded from persisted file");
                        return Self {
                            key: Arc::new(key),
                        };
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

        Self {
            key: Arc::new(key),
        }
    }
}
