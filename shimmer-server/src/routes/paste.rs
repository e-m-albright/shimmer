//! Paste routes — upload, fetch, list, search, delete.
//!
//! The server handles ciphertext only. It never has the KEK or plaintext.
//! Metadata (visibility, content type, search tokens) lives in `SQLite`.
//! Ciphertext blobs live in S3/file storage.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::info;
use validator::Validate;

use crate::auth::Claims;
use crate::db::PasteRecord;
use crate::AppState;

/// Max ciphertext size the server will accept (50 MiB — file uploads).
const MAX_CIPHERTEXT_BYTES: usize = 50 * 1024 * 1024;

/// Allowed visibility values.
const VALID_VISIBILITIES: &[&str] = &["private", "org", "link"];

/// Request body for paste upload (text or file).
#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UploadRequest {
    /// Encrypted envelope payload (JSON string from client).
    #[validate(length(min = 1, message = "ciphertext cannot be empty"))]
    pub ciphertext: String,

    /// Blind index tokens for searchable encryption (content).
    #[serde(default)]
    pub search_tokens: Vec<String>,

    /// Encrypted title (base64, for display after client-side decrypt).
    #[serde(default)]
    pub title_encrypted: Option<String>,

    /// Blind index tokens for title search.
    #[serde(default)]
    pub title_tokens: Vec<String>,

    /// Content type (e.g., `text/plain`, `image/png`, `application/pdf`).
    #[serde(default = "default_content_type")]
    pub content_type: String,

    /// Encrypted original filename (base64).
    #[serde(default)]
    pub filename_encrypted: Option<String>,

    /// Blind tokens for filename search.
    #[serde(default)]
    pub filename_tokens: Vec<String>,

    /// Visibility: "private", "org", "link".
    #[serde(default = "default_visibility")]
    pub visibility: String,

    /// TTL in hours (None = no expiry).
    #[serde(default)]
    #[validate(range(min = 1, max = 8760, message = "TTL must be 1–8760 hours"))]
    pub ttl_hours: Option<u64>,

    /// Delete after first read.
    #[serde(default)]
    pub burn_on_read: bool,

    /// Tags (blind-indexed, not encrypted — opaque tokens only).
    #[serde(default)]
    pub tag_tokens: Vec<String>,
}

fn default_visibility() -> String {
    "org".into()
}

fn default_content_type() -> String {
    "text/plain".into()
}

/// Response for paste upload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub id: String,
    pub phi_url: String,
}

/// Paste metadata returned in list/search responses.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteMeta {
    pub id: String,
    pub content_type: String,
    pub encrypted_title: Option<String>,
    pub encrypted_filename: Option<String>,
    pub created_at: String,
    pub size: i64,
    pub visibility: String,
    pub user_id: String,
    pub user_name: String,
    pub burn_on_read: bool,
    pub ttl_hours: Option<i64>,
}

impl From<PasteRecord> for PasteMeta {
    fn from(r: PasteRecord) -> Self {
        Self {
            id: r.id,
            content_type: r.content_type,
            encrypted_title: r.encrypted_title,
            encrypted_filename: r.encrypted_filename,
            created_at: r.created_at,
            size: r.size_bytes,
            visibility: r.visibility,
            user_id: r.user_id,
            user_name: r.user_name,
            burn_on_read: r.burn_on_read,
            ttl_hours: r.ttl_hours,
        }
    }
}

/// Upload an encrypted paste (text or file).
pub async fn upload(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<UploadRequest>,
) -> Result<(StatusCode, Json<UploadResponse>), (StatusCode, String)> {
    // Validate request fields
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    if req.ciphertext.len() > MAX_CIPHERTEXT_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "ciphertext exceeds 50 MiB".into(),
        ));
    }

    if !VALID_VISIBILITIES.contains(&req.visibility.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "invalid visibility '{}', must be one of: {}",
                req.visibility,
                VALID_VISIBILITIES.join(", ")
            ),
        ));
    }

    // Check role: read_only users cannot upload
    if claims.role == "read_only" {
        return Err((
            StatusCode::FORBIDDEN,
            "read-only users cannot upload".into(),
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let storage_key = format!("{}/{}", claims.sub, id);
    let now = chrono::Utc::now();

    // Compute expiry
    let expires_at = req.ttl_hours.map(|hours| {
        let h = i64::try_from(hours).unwrap_or(i64::MAX);
        (now + chrono::Duration::hours(h)).to_rfc3339()
    });

    // Store ciphertext blob
    state
        .storage
        .put(&storage_key, req.ciphertext.as_bytes())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Build search token pairs: (token, type)
    let mut token_pairs: Vec<(String, String)> = Vec::new();
    for t in &req.search_tokens {
        token_pairs.push((t.clone(), "content".into()));
    }
    for t in &req.title_tokens {
        token_pairs.push((t.clone(), "title".into()));
    }
    for t in &req.filename_tokens {
        token_pairs.push((t.clone(), "filename".into()));
    }
    for t in &req.tag_tokens {
        token_pairs.push((t.clone(), "tag".into()));
    }

    // Store metadata in DB
    let paste = PasteRecord {
        id: id.clone(),
        org_id: claims.org.clone(),
        user_id: claims.sub.clone(),
        user_name: claims.name.clone(),
        content_type: req.content_type.clone(),
        encrypted_title: req.title_encrypted.clone(),
        encrypted_filename: req.filename_encrypted.clone(),
        visibility: req.visibility.clone(),
        #[allow(clippy::cast_possible_truncation)]
        size_bytes: req.ciphertext.len() as i64,
        ttl_hours: req.ttl_hours.map(|h| i64::try_from(h).unwrap_or(i64::MAX)),
        burn_on_read: req.burn_on_read,
        created_at: now.to_rfc3339(),
        expires_at,
    };

    let num_tokens = token_pairs.len();
    let db_state = state.clone();
    tokio::task::spawn_blocking(move || db_state.db.insert_paste(&paste, &token_pairs))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        paste_id = %id,
        user_id = %claims.sub,
        content_type = %req.content_type,
        size = req.ciphertext.len(),
        visibility = %req.visibility,
        tokens = num_tokens,
        "paste uploaded"
    );

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            phi_url: format!("phi://{id}"),
            id,
        }),
    ))
}

/// Fetch an encrypted paste by ID.
///
/// Checks visibility permissions:
/// - private: only owner
/// - org: any member of the same org
/// - link: any authenticated user
pub async fn fetch(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<String>,
) -> Result<Vec<u8>, (StatusCode, String)> {
    uuid::Uuid::parse_str(&id).map_err(|_| (StatusCode::BAD_REQUEST, "invalid paste ID".into()))?;

    // Look up paste metadata for permissions + storage key
    let db_state = state.clone();
    let paste_id = id.clone();
    let paste = tokio::task::spawn_blocking(move || db_state.db.get_paste(&paste_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("paste {id} not found")))?;

    // Check visibility permissions
    match paste.visibility.as_str() {
        "private" => {
            if paste.user_id != claims.sub {
                return Err((StatusCode::FORBIDDEN, "private paste".into()));
            }
        }
        "org" => {
            if paste.org_id != claims.org {
                return Err((
                    StatusCode::FORBIDDEN,
                    "paste belongs to different org".into(),
                ));
            }
        }
        "link" => {
            // Anyone with auth can access link-shared pastes
        }
        _ => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "unknown visibility".into(),
            ));
        }
    }

    // Fetch from blob storage using the owner's key path
    let storage_key = format!("{}/{}", paste.user_id, id);
    let data = state
        .storage
        .get(&storage_key)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    info!(paste_id = %id, user_id = %claims.sub, "paste fetched");

    // Handle burn-on-read: delete after first fetch
    if paste.burn_on_read {
        let del_state = state.clone();
        let del_key = storage_key;
        let del_id = id.clone();
        tokio::spawn(async move {
            let _ = del_state.storage.delete(&del_key).await;
            let _ = tokio::task::spawn_blocking(move || del_state.db.delete_paste(&del_id)).await;
        });
        info!(paste_id = %id, "burn-on-read: paste scheduled for deletion");
    }

    Ok(data)
}

/// Query parameters for list/search.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    /// Comma-separated blind index tokens for search.
    #[serde(default)]
    pub tokens: Option<String>,
    /// Max results (default 50).
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// List or search pastes for the authenticated user's org.
pub async fn list(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<PasteMeta>>, (StatusCode, String)> {
    let org_id = claims.org.clone();
    let user_id = claims.sub.clone();

    let records = if let Some(ref token_str) = query.tokens {
        // Search mode: split comma-separated tokens
        let tokens: Vec<String> = token_str
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();

        let db_state = state.clone();
        tokio::task::spawn_blocking(move || db_state.db.search_pastes(&org_id, &user_id, &tokens))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        // List mode
        let limit = query.limit.min(200);
        let offset = query.offset.max(0);
        let db_state = state.clone();
        tokio::task::spawn_blocking(move || {
            db_state.db.list_pastes(&org_id, &user_id, limit, offset)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let metas: Vec<PasteMeta> = records.into_iter().map(PasteMeta::from).collect();
    Ok(Json(metas))
}

/// Delete a paste.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    uuid::Uuid::parse_str(&id).map_err(|_| (StatusCode::BAD_REQUEST, "invalid paste ID".into()))?;

    // Look up paste for permission check
    let db_state = state.clone();
    let paste_id = id.clone();
    let paste = tokio::task::spawn_blocking(move || db_state.db.get_paste(&paste_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("paste {id} not found")))?;

    // Permission: owner can delete own, admin can delete any in org
    let is_owner = paste.user_id == claims.sub;
    let is_admin = claims.is_admin() && paste.org_id == claims.org;
    if !is_owner && !is_admin {
        return Err((StatusCode::FORBIDDEN, "cannot delete this paste".into()));
    }

    // Delete blob
    let storage_key = format!("{}/{}", paste.user_id, id);
    state
        .storage
        .delete(&storage_key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Delete metadata
    let db_state = state.clone();
    let del_id = id.clone();
    tokio::task::spawn_blocking(move || db_state.db.delete_paste(&del_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(paste_id = %id, user_id = %claims.sub, "paste deleted");
    Ok(StatusCode::NO_CONTENT)
}
