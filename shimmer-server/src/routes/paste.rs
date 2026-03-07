//! Paste routes — upload, fetch, list, delete.
//!
//! The server handles ciphertext only. It never has the KEK or plaintext.

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
use crate::AppState;

/// Max ciphertext size the server will accept (2 MiB — envelope overhead on top of 1 MiB plaintext).
const MAX_CIPHERTEXT_BYTES: usize = 2 * 1024 * 1024;

/// Allowed visibility values.
const VALID_VISIBILITIES: &[&str] = &["private", "org", "link"];

/// Request body for paste upload.
///
/// Some fields are scaffolded for future phases (search, TTL, burn-on-read)
/// and are deserialized but not yet read in route handlers.
#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UploadRequest {
    /// Encrypted envelope payload (JSON string from client).
    #[validate(length(min = 1, max = 2_097_152, message = "ciphertext too large or empty"))]
    pub ciphertext: String,
    /// Blind index tokens for searchable encryption.
    #[serde(default)]
    #[allow(dead_code)]
    pub search_tokens: Vec<String>,
    /// Encrypted title (base64, for display after client-side decrypt).
    #[serde(default)]
    #[allow(dead_code)]
    pub title_encrypted: Option<String>,
    /// Blind index tokens for title search.
    #[serde(default)]
    #[allow(dead_code)]
    pub title_tokens: Vec<String>,
    /// Visibility: "private", "org", "link".
    #[serde(default = "default_visibility")]
    pub visibility: String,
    /// TTL in hours (None = no expiry).
    #[serde(default)]
    #[validate(range(min = 1, max = 8760, message = "TTL must be 1–8760 hours"))]
    #[allow(dead_code)]
    pub ttl_hours: Option<u64>,
    /// Delete after first read.
    #[serde(default)]
    #[allow(dead_code)]
    pub burn_on_read: bool,
}

fn default_visibility() -> String {
    "private".into()
}

/// Response for paste upload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub id: String,
    pub phi_url: String,
}

/// Paste metadata returned in list responses.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteMeta {
    pub id: String,
    pub created_at: String,
    pub size: usize,
    pub visibility: String,
    pub title_encrypted: Option<String>,
    pub burn_on_read: bool,
    pub ttl_hours: Option<u64>,
}

/// Upload an encrypted paste.
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
            "ciphertext exceeds 2 MiB".into(),
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

    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("{}/{}", claims.sub, id);

    // Store the ciphertext
    state
        .storage
        .put(&key, req.ciphertext.as_bytes())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // TODO: store paste metadata (search_tokens, title, visibility, ttl, burn_on_read)
    // in a lightweight DB or metadata sidecar file

    info!(
        paste_id = %id,
        user_id = %claims.sub,
        size = req.ciphertext.len(),
        visibility = %req.visibility,
        "paste uploaded"
    );

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            phi_url: format!("phi://{}", id),
            id,
        }),
    ))
}

/// Fetch an encrypted paste by ID.
pub async fn fetch(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<String>,
) -> Result<Vec<u8>, (StatusCode, String)> {
    // Validate UUID format
    uuid::Uuid::parse_str(&id).map_err(|_| (StatusCode::BAD_REQUEST, "invalid paste ID".into()))?;

    // TODO: check visibility permissions (private = owner only, org = any member, link = anyone)
    // For now: owner-only
    let key = format!("{}/{}", claims.sub, id);

    let data = state
        .storage
        .get(&key)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    info!(paste_id = %id, user_id = %claims.sub, "paste fetched");

    // TODO: check burn_on_read and delete if true
    // TODO: check TTL and return 410 Gone if expired

    Ok(data)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Used in Phase 6 (search)
pub struct ListQuery {
    #[serde(default)]
    pub tokens: Option<String>, // comma-separated blind index tokens
}

/// List pastes for the authenticated user.
pub async fn list(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(_query): Query<ListQuery>,
) -> Result<Json<Vec<PasteMeta>>, (StatusCode, String)> {
    let entries = state
        .storage
        .list(&claims.sub)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // TODO: filter by blind index tokens if query.tokens is set
    // TODO: return actual metadata from metadata store

    let metas: Vec<PasteMeta> = entries
        .into_iter()
        .map(|e| PasteMeta {
            id: e.id,
            created_at: e.created,
            #[allow(clippy::cast_possible_truncation)]
            size: e.size as usize,
            visibility: "private".into(),
            title_encrypted: None,
            burn_on_read: false,
            ttl_hours: None,
        })
        .collect();

    Ok(Json(metas))
}

/// Delete a paste.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Validate UUID format
    uuid::Uuid::parse_str(&id).map_err(|_| (StatusCode::BAD_REQUEST, "invalid paste ID".into()))?;

    // TODO: check permissions (admin can delete any, member can delete own only)
    let key = format!("{}/{}", claims.sub, id);

    state
        .storage
        .delete(&key)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    info!(paste_id = %id, user_id = %claims.sub, "paste deleted");
    Ok(StatusCode::NO_CONTENT)
}
