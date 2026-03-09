//! Typed command errors — serialized to the frontend as structured JSON.
//!
//! Tauri commands return `Result<T, CommandError>` instead of `Result<T, String>`,
//! giving the frontend typed error variants to match on.

use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum CommandError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("server error: {0}")]
    Server(String),

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for CommandError {
    fn from(err: anyhow::Error) -> Self {
        CommandError::Internal(err.to_string())
    }
}
