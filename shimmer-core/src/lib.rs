//! shimmer-core — shared encryption, storage, and error types.
//!
//! Used by both the Tauri desktop client and the Axum API server.

pub mod encryption;
pub mod error;
pub mod storage;

/// Max paste size: 1 MiB.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;

/// Key length for AES-256.
pub const KEY_LEN: usize = 32;
