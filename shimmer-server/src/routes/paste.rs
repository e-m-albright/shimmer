//! Paste routes — upload, fetch, list, search, delete.
//!
//! Thin wrappers around `services::paste` — validate HTTP, call service, map errors.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::auth::Claims;
use crate::db::PasteRecord;
use crate::services::paste::{self, CreatePasteInput, PasteCaller, PasteServiceError};
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

/// Map `PasteServiceError` to HTTP status + message.
fn map_paste_err(e: PasteServiceError) -> (StatusCode, String) {
    match e {
        PasteServiceError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        PasteServiceError::Forbidden => (StatusCode::FORBIDDEN, e.to_string()),
        PasteServiceError::Storage(_)
        | PasteServiceError::Db(_)
        | PasteServiceError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
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

    let caller = PasteCaller {
        sub: claims.sub,
        name: claims.name,
        org: claims.org,
        role: claims.role,
    };

    let input = CreatePasteInput {
        ciphertext: req.ciphertext,
        search_tokens: req.search_tokens,
        title_encrypted: req.title_encrypted,
        title_tokens: req.title_tokens,
        content_type: req.content_type,
        filename_encrypted: req.filename_encrypted,
        filename_tokens: req.filename_tokens,
        visibility: req.visibility,
        ttl_hours: req.ttl_hours,
        burn_on_read: req.burn_on_read,
        tag_tokens: req.tag_tokens,
    };

    let output = paste::create_paste(state, &caller, input)
        .await
        .map_err(map_paste_err)?;

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            id: output.id,
            phi_url: output.phi_url,
        }),
    ))
}

/// Fetch an encrypted paste by ID.
pub async fn fetch(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<String>,
) -> Result<Vec<u8>, (StatusCode, String)> {
    uuid::Uuid::parse_str(&id).map_err(|_| (StatusCode::BAD_REQUEST, "invalid paste ID".into()))?;

    let caller = PasteCaller {
        sub: claims.sub,
        name: claims.name,
        org: claims.org,
        role: claims.role,
    };

    paste::fetch_paste(state, &caller, &id)
        .await
        .map_err(map_paste_err)
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
    let records = if let Some(ref token_str) = query.tokens {
        // Search mode: split comma-separated tokens
        let tokens: Vec<String> = token_str
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();

        paste::search_pastes(state, &claims.org, &claims.sub, &tokens)
            .await
            .map_err(map_paste_err)?
    } else {
        paste::list_pastes(state, &claims.org, &claims.sub, query.limit, query.offset)
            .await
            .map_err(map_paste_err)?
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

    let caller = PasteCaller {
        sub: claims.sub,
        name: claims.name,
        org: claims.org,
        role: claims.role,
    };

    paste::delete_paste(state, &caller, &id)
        .await
        .map_err(map_paste_err)?;

    Ok(StatusCode::NO_CONTENT)
}
