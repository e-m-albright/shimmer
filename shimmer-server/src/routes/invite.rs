//! Invite flow routes — generate invite links + join an org.
//!
//! Thin wrappers around `services::invite` — validate HTTP, call service, map errors.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use crate::auth::Claims;
use crate::services::invite::{
    self, CreateInviteInput, InviteCaller, InviteServiceError, RedeemInviteInput,
};
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

/// Map `InviteServiceError` to HTTP status + message.
fn map_invite_err(e: InviteServiceError) -> (StatusCode, String) {
    match e {
        InviteServiceError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        InviteServiceError::Forbidden => (StatusCode::FORBIDDEN, "admin only".into()),
        InviteServiceError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
        InviteServiceError::Db(_) | InviteServiceError::Internal(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

/// Generate an invite link. Admin only.
pub async fn generate_invite(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<GenerateInviteRequest>,
) -> Result<(StatusCode, Json<GenerateInviteResponse>), (StatusCode, String)> {
    let caller = InviteCaller {
        sub: claims.sub,
        name: claims.name,
        org: claims.org,
        role: claims.role,
    };

    let input = CreateInviteInput {
        role: req.role,
        ttl_hours: req.ttl_hours,
        single_use: req.single_use,
    };

    let output = invite::create_invite(state, &caller, input)
        .await
        .map_err(map_invite_err)?;

    Ok((
        StatusCode::CREATED,
        Json(GenerateInviteResponse {
            token: output.token,
            org_id: output.org_id,
            expires_at: output.expires_at,
        }),
    ))
}

/// Join an org using an invite token.
pub async fn join_org(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JoinRequest>,
) -> Result<(StatusCode, Json<JoinResponse>), (StatusCode, String)> {
    let input = RedeemInviteInput {
        token: req.token,
        name: req.name,
    };

    let output = invite::redeem_invite(state, input)
        .await
        .map_err(map_invite_err)?;

    Ok((
        StatusCode::CREATED,
        Json(JoinResponse {
            org_id: output.org_id,
            user_id: output.user_id,
            jwt: output.jwt,
            role: output.role,
            server_url: output.server_url,
        }),
    ))
}
