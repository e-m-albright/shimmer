//! Paste service — business logic for paste CRUD and search.
//!
//! All functions accept `Arc<AppState>` to support `spawn_blocking` internally.
//! Route handlers become thin wrappers that call these functions.

use std::sync::Arc;

use tracing::info;

use crate::db::{DbError, PasteRecord};
use crate::AppState;

/// Errors that can occur in paste service operations.
#[derive(Debug, thiserror::Error)]
pub enum PasteServiceError {
    #[error("storage error: {0}")]
    Storage(shimmer_core::error::StorageError),

    #[error("database error: {0}")]
    Db(DbError),

    #[error("not found")]
    NotFound,

    #[error("forbidden")]
    Forbidden,

    #[error("internal: {0}")]
    Internal(String),
}

impl From<DbError> for PasteServiceError {
    fn from(e: DbError) -> Self {
        Self::Db(e)
    }
}

impl From<shimmer_core::error::StorageError> for PasteServiceError {
    fn from(e: shimmer_core::error::StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<tokio::task::JoinError> for PasteServiceError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::Internal(e.to_string())
    }
}

/// Input for creating a paste.
#[derive(Debug)]
pub struct CreatePasteInput {
    pub ciphertext: String,
    pub search_tokens: Vec<String>,
    pub title_encrypted: Option<String>,
    pub title_tokens: Vec<String>,
    pub content_type: String,
    pub filename_encrypted: Option<String>,
    pub filename_tokens: Vec<String>,
    pub visibility: String,
    pub ttl_hours: Option<u64>,
    pub burn_on_read: bool,
    pub tag_tokens: Vec<String>,
}

/// Output from creating a paste.
#[derive(Debug)]
pub struct CreatePasteOutput {
    pub id: String,
    pub phi_url: String,
}

/// Caller identity needed by paste operations.
#[derive(Debug)]
pub struct PasteCaller {
    pub sub: String,
    pub name: String,
    pub org: String,
    pub role: String,
}

/// Create a new encrypted paste.
///
/// # Errors
///
/// Returns `PasteServiceError` on permission, storage, or database failures.
pub async fn create_paste(
    state: Arc<AppState>,
    caller: &PasteCaller,
    input: CreatePasteInput,
) -> Result<CreatePasteOutput, PasteServiceError> {
    // Check role: read_only users cannot upload
    if caller.role == "read_only" {
        return Err(PasteServiceError::Forbidden);
    }

    let id = uuid::Uuid::new_v4().to_string();
    let storage_key = format!("{}/{}", caller.sub, id);
    let now = chrono::Utc::now();

    // Compute expiry
    let expires_at = input.ttl_hours.map(|hours| {
        let h = i64::try_from(hours).unwrap_or(i64::MAX);
        (now + chrono::Duration::hours(h)).to_rfc3339()
    });

    // Store ciphertext blob
    state
        .storage
        .put(&storage_key, input.ciphertext.as_bytes())
        .await
        .map_err(PasteServiceError::Storage)?;

    // Build search token pairs: (token, type)
    let mut token_pairs: Vec<(String, String)> = Vec::new();
    for t in &input.search_tokens {
        token_pairs.push((t.clone(), "content".into()));
    }
    for t in &input.title_tokens {
        token_pairs.push((t.clone(), "title".into()));
    }
    for t in &input.filename_tokens {
        token_pairs.push((t.clone(), "filename".into()));
    }
    for t in &input.tag_tokens {
        token_pairs.push((t.clone(), "tag".into()));
    }

    // Store metadata in DB
    let paste = PasteRecord {
        id: id.clone(),
        org_id: caller.org.clone(),
        user_id: caller.sub.clone(),
        user_name: caller.name.clone(),
        content_type: input.content_type.clone(),
        encrypted_title: input.title_encrypted.clone(),
        encrypted_filename: input.filename_encrypted.clone(),
        visibility: input.visibility.clone(),
        #[allow(clippy::cast_possible_truncation)]
        size_bytes: input.ciphertext.len() as i64,
        ttl_hours: input
            .ttl_hours
            .map(|h| i64::try_from(h).unwrap_or(i64::MAX)),
        burn_on_read: input.burn_on_read,
        created_at: now.to_rfc3339(),
        expires_at,
    };

    let num_tokens = token_pairs.len();
    let db_state = state.clone();
    tokio::task::spawn_blocking(move || db_state.db.insert_paste(&paste, &token_pairs)).await??;

    info!(
        paste_id = %id,
        user_id = %caller.sub,
        content_type = %input.content_type,
        size = input.ciphertext.len(),
        visibility = %input.visibility,
        tokens = num_tokens,
        "paste uploaded"
    );

    Ok(CreatePasteOutput {
        phi_url: format!("phi://{id}"),
        id,
    })
}

/// Fetch an encrypted paste by ID. Returns the raw ciphertext bytes.
///
/// Checks visibility permissions:
/// - private: only owner
/// - org: any member of the same org
/// - link: any authenticated user
///
/// Handles burn-on-read by scheduling deletion after fetch.
///
/// # Errors
///
/// Returns `PasteServiceError` on permission, storage, or database failures.
pub async fn fetch_paste(
    state: Arc<AppState>,
    caller: &PasteCaller,
    id: &str,
) -> Result<Vec<u8>, PasteServiceError> {
    // Look up paste metadata for permissions + storage key
    let db_state = state.clone();
    let paste_id = id.to_string();
    let paste = tokio::task::spawn_blocking(move || db_state.db.get_paste(&paste_id))
        .await??
        .ok_or(PasteServiceError::NotFound)?;

    // Check visibility permissions
    match paste.visibility.as_str() {
        "private" => {
            if paste.user_id != caller.sub {
                return Err(PasteServiceError::Forbidden);
            }
        }
        "org" => {
            if paste.org_id != caller.org {
                return Err(PasteServiceError::Forbidden);
            }
        }
        "link" => {
            // Anyone with auth can access link-shared pastes
        }
        _ => {
            return Err(PasteServiceError::Internal("unknown visibility".into()));
        }
    }

    // Fetch from blob storage using the owner's key path
    let storage_key = format!("{}/{}", paste.user_id, id);
    let data = state
        .storage
        .get(&storage_key)
        .await
        .map_err(|_| PasteServiceError::NotFound)?;

    info!(paste_id = %id, user_id = %caller.sub, "paste fetched");

    // Handle burn-on-read: delete after first fetch
    if paste.burn_on_read {
        let del_state = state.clone();
        let del_key = storage_key;
        let del_id = id.to_string();
        tokio::spawn(async move {
            let _ = del_state.storage.delete(&del_key).await;
            let _ = tokio::task::spawn_blocking(move || del_state.db.delete_paste(&del_id)).await;
        });
        info!(paste_id = %id, "burn-on-read: paste scheduled for deletion");
    }

    Ok(data)
}

/// List pastes visible to the caller.
///
/// # Errors
///
/// Returns `PasteServiceError` on database failures.
pub async fn list_pastes(
    state: Arc<AppState>,
    org_id: &str,
    user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<PasteRecord>, PasteServiceError> {
    let limit = limit.min(200);
    let offset = offset.max(0);
    let db_state = state.clone();
    let org = org_id.to_string();
    let user = user_id.to_string();
    let records =
        tokio::task::spawn_blocking(move || db_state.db.list_pastes(&org, &user, limit, offset))
            .await??;
    Ok(records)
}

/// Search pastes by blind index tokens.
///
/// # Errors
///
/// Returns `PasteServiceError` on database failures.
pub async fn search_pastes(
    state: Arc<AppState>,
    org_id: &str,
    user_id: &str,
    tokens: &[String],
) -> Result<Vec<PasteRecord>, PasteServiceError> {
    let db_state = state.clone();
    let org = org_id.to_string();
    let user = user_id.to_string();
    let tokens = tokens.to_vec();
    let records =
        tokio::task::spawn_blocking(move || db_state.db.search_pastes(&org, &user, &tokens))
            .await??;
    Ok(records)
}

/// Delete a paste. Only the owner or an admin in the same org can delete.
///
/// # Errors
///
/// Returns `PasteServiceError` on permission, storage, or database failures.
pub async fn delete_paste(
    state: Arc<AppState>,
    caller: &PasteCaller,
    id: &str,
) -> Result<(), PasteServiceError> {
    // Look up paste for permission check
    let db_state = state.clone();
    let paste_id = id.to_string();
    let paste = tokio::task::spawn_blocking(move || db_state.db.get_paste(&paste_id))
        .await??
        .ok_or(PasteServiceError::NotFound)?;

    // Permission: owner can delete own, admin can delete any in org
    let is_owner = paste.user_id == caller.sub;
    let is_admin = caller.role == "admin" && paste.org_id == caller.org;
    if !is_owner && !is_admin {
        return Err(PasteServiceError::Forbidden);
    }

    // Delete blob
    let storage_key = format!("{}/{}", paste.user_id, id);
    state
        .storage
        .delete(&storage_key)
        .await
        .map_err(PasteServiceError::Storage)?;

    // Delete metadata
    let db_state = state.clone();
    let del_id = id.to_string();
    tokio::task::spawn_blocking(move || db_state.db.delete_paste(&del_id)).await??;

    info!(paste_id = %id, user_id = %caller.sub, "paste deleted");
    Ok(())
}
