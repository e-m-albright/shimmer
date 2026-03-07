//! Shared error types for shimmer-core.

use thiserror::Error;

/// Encryption/decryption errors.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum CryptoError {
    #[error("encryption failed: {0}")]
    Encrypt(String),

    #[error("decryption failed: {0}")]
    Decrypt(String),

    #[error("invalid key: {0}")]
    InvalidKey(String),

    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
}

// aes_gcm errors don't implement std::error::Error, so we convert manually.
impl From<aes_gcm::Error> for CryptoError {
    fn from(_: aes_gcm::Error) -> Self {
        CryptoError::Decrypt("AES-GCM operation failed".into())
    }
}

impl From<hmac::digest::InvalidLength> for CryptoError {
    fn from(e: hmac::digest::InvalidLength) -> Self {
        CryptoError::InvalidKey(e.to_string())
    }
}

/// Storage backend errors.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum StorageError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("storage operation failed: {0}")]
    Backend(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
