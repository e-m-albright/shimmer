//! Invite flow — generate invite links + join an org.
//!
//! Security model:
//! - Admin generates invite → gets `shimmer://join/TOKEN#ENCRYPTED_KEK`
//! - TOKEN is server-side, maps to {`org_id`, role, `expires_at`, `single_use`}
//! - `#ENCRYPTED_KEK` is a URL fragment (never sent to server by HTTP spec)
//! - New user sends TOKEN to `POST /api/org/join` → server returns org info
//! - Client-side: decrypts KEK from URL fragment, stores locally
//! - Server never sees the KEK

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::auth::{self, Claims};
use crate::db::{InviteRecord, MemberRecord};
use crate::AppState;

/// Generate invite request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateInviteRequest {
    /// Role for the invited user: "member" (default) or "`read_only`".
    #[serde(default = "default_invite_role")]
    pub role: String,
    /// TTL in hours (default 24, max 168 = 1 week).
    #[serde(default = "default_invite_ttl")]
    pub ttl_hours: u64,
    /// Single-use invite (default true).
    #[serde(default = "default_single_use")]
    pub single_use: bool,
}

fn default_invite_role() -> String {
    "member".into()
}
fn default_invite_ttl() -> u64 {
    24
}
fn default_single_use() -> bool {
    true
}

/// Invite response — token for use in invite link.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateInviteResponse {
    /// The invite token (server side — goes in URL path).
    pub token: String,
    /// Org ID.
    pub org_id: String,
    /// When the invite expires.
    pub expires_at: String,
}

/// Generate an invite link. Admin only.
pub async fn generate_invite(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<GenerateInviteRequest>,
) -> Result<(StatusCode, Json<GenerateInviteResponse>), (StatusCode, String)> {
    if !claims.is_admin() {
        return Err((StatusCode::FORBIDDEN, "admin only".into()));
    }

    let valid_roles = ["member", "read_only"];
    if !valid_roles.contains(&req.role.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("invite role must be one of: {}", valid_roles.join(", ")),
        ));
    }

    if req.ttl_hours > 168 {
        return Err((
            StatusCode::BAD_REQUEST,
            "invite TTL cannot exceed 168 hours (1 week)".into(),
        ));
    }

    let token = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let ttl = i64::try_from(req.ttl_hours).unwrap_or(24);
    let expires_at = (now + chrono::Duration::hours(ttl)).to_rfc3339();

    let invite = InviteRecord {
        token: token.clone(),
        org_id: claims.org.clone(),
        role: req.role.clone(),
        created_by: claims.sub.clone(),
        expires_at: expires_at.clone(),
        used_at: None,
        used_by: None,
        single_use: req.single_use,
    };

    let db_state = state.clone();
    tokio::task::spawn_blocking(move || db_state.db.create_invite(&invite))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        token = %token,
        org_id = %claims.org,
        role = %req.role,
        admin = %claims.sub,
        "invite generated"
    );

    Ok((
        StatusCode::CREATED,
        Json(GenerateInviteResponse {
            token,
            org_id: claims.org,
            expires_at,
        }),
    ))
}

/// Join request — sent by a new user clicking an invite link.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    /// Invite token (from the URL path, NOT the fragment).
    pub token: String,
    /// New user's display name.
    pub name: String,
}

/// Join response — sent after successfully joining an org.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinResponse {
    /// Org ID the user just joined.
    pub org_id: String,
    /// User ID assigned to the new member.
    pub user_id: String,
    /// JWT token for future API calls.
    pub jwt: String,
    /// Role assigned.
    pub role: String,
    /// Server URL (for client config).
    pub server_url: String,
}

/// Join an org using an invite token.
///
/// This is an unauthenticated endpoint — the invite token IS the auth.
/// After joining, the user gets a JWT for future API calls.
pub async fn join_org(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JoinRequest>,
) -> Result<(StatusCode, Json<JoinResponse>), (StatusCode, String)> {
    let user_id = format!("u_{}", uuid::Uuid::new_v4().simple());
    let token = req.token.clone();

    // Consume the invite (validates it's unexpired and unused)
    let db_state = state.clone();
    let join_user_id = user_id.clone();
    let invite = tokio::task::spawn_blocking(move || {
        db_state.db.consume_invite(&token, &join_user_id)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| match e {
        crate::db::DbError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    })?;

    // Add the new member
    let member = MemberRecord {
        id: format!("m_{}", uuid::Uuid::new_v4()),
        org_id: invite.org_id.clone(),
        user_id: user_id.clone(),
        name: req.name.clone(),
        role: invite.role.clone(),
        joined_at: chrono::Utc::now().to_rfc3339(),
    };

    let db_state = state.clone();
    tokio::task::spawn_blocking(move || db_state.db.add_member(&member))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Issue JWT for the new member
    let exp = usize::try_from(
        (chrono::Utc::now() + chrono::Duration::days(30)).timestamp(),
    )
    .unwrap_or(usize::MAX);

    let claims = Claims {
        sub: user_id.clone(),
        name: req.name.clone(),
        role: invite.role.clone(),
        org: invite.org_id.clone(),
        exp,
    };

    let jwt = auth::create_token(&claims, &state.jwt_secret)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let server_url = format!("{}:{}", state.config.host, state.config.port);

    info!(
        user_id = %user_id,
        org_id = %invite.org_id,
        role = %invite.role,
        "user joined org via invite"
    );

    Ok((
        StatusCode::CREATED,
        Json(JoinResponse {
            org_id: invite.org_id,
            user_id,
            jwt,
            role: invite.role,
            server_url,
        }),
    ))
}
