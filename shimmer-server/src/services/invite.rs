//! Invite service — business logic for invite generation and redemption.

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use tracing::info;

use crate::auth::{self, Claims};
use crate::db::{DbError, InviteRecord, MemberRecord};
use crate::AppState;

/// Errors that can occur in invite service operations.
#[derive(Debug, thiserror::Error)]
pub enum InviteServiceError {
    #[error("database error: {0}")]
    Db(DbError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("forbidden")]
    Forbidden,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal: {0}")]
    Internal(String),
}

impl From<DbError> for InviteServiceError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::NotFound(msg) => Self::NotFound(msg),
            other => Self::Db(other),
        }
    }
}

impl From<tokio::task::JoinError> for InviteServiceError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::Internal(e.to_string())
    }
}

/// Input for creating an invite.
#[derive(Debug)]
pub struct CreateInviteInput {
    pub role: String,
    pub ttl_hours: u64,
    pub single_use: bool,
}

/// Output from creating an invite.
#[derive(Debug)]
pub struct CreateInviteOutput {
    pub token: String,
    pub org_id: String,
    pub expires_at: String,
}

/// Caller identity needed by invite operations.
#[derive(Debug)]
pub struct InviteCaller {
    pub sub: String,
    pub name: String,
    pub org: String,
    pub role: String,
}

impl InviteCaller {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// Generate a 256-bit random invite token, base64url-encoded (no padding).
///
/// The token provides enough entropy for HKDF key derivation in the
/// two-phase KEK transport protocol.
fn generate_invite_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Generate an invite link. Admin only.
///
/// # Errors
///
/// Returns `InviteServiceError` on permission, validation, or database failures.
pub async fn create_invite(
    state: Arc<AppState>,
    caller: &InviteCaller,
    input: CreateInviteInput,
) -> Result<CreateInviteOutput, InviteServiceError> {
    if !caller.is_admin() {
        return Err(InviteServiceError::Forbidden);
    }

    let valid_roles = ["member", "read_only"];
    if !valid_roles.contains(&input.role.as_str()) {
        return Err(InviteServiceError::BadRequest(format!(
            "invite role must be one of: {}",
            valid_roles.join(", ")
        )));
    }

    if input.ttl_hours > 168 {
        return Err(InviteServiceError::BadRequest(
            "invite TTL cannot exceed 168 hours (1 week)".into(),
        ));
    }

    let token = generate_invite_token();
    let now = chrono::Utc::now();
    let ttl = i64::try_from(input.ttl_hours).unwrap_or(24);
    let expires_at = (now + chrono::Duration::hours(ttl)).to_rfc3339();

    let invite = InviteRecord {
        token: token.clone(),
        org_id: caller.org.clone(),
        role: input.role.clone(),
        created_by: caller.sub.clone(),
        expires_at: expires_at.clone(),
        used_at: None,
        used_by: None,
        single_use: input.single_use,
    };

    let db_state = state.clone();
    tokio::task::spawn_blocking(move || db_state.db.create_invite(&invite)).await??;

    info!(
        token = %token,
        org_id = %caller.org,
        role = %input.role,
        admin = %caller.sub,
        "invite generated"
    );

    Ok(CreateInviteOutput {
        token,
        org_id: caller.org.clone(),
        expires_at,
    })
}

/// Input for redeeming an invite.
#[derive(Debug)]
pub struct RedeemInviteInput {
    pub token: String,
    pub name: String,
}

/// Output from redeeming an invite.
#[derive(Debug)]
pub struct RedeemInviteOutput {
    pub org_id: String,
    pub user_id: String,
    pub jwt: String,
    pub role: String,
    pub server_url: String,
}

/// Join an org using an invite token.
///
/// This is an unauthenticated operation — the invite token IS the auth.
/// After joining, the user gets a JWT for future API calls.
///
/// # Errors
///
/// Returns `InviteServiceError` on validation, or database failures.
pub async fn redeem_invite(
    state: Arc<AppState>,
    input: RedeemInviteInput,
) -> Result<RedeemInviteOutput, InviteServiceError> {
    let user_id = format!("u_{}", uuid::Uuid::new_v4().simple());
    let token = input.token.clone();

    // Consume the invite (validates it's unexpired and unused)
    let db_state = state.clone();
    let join_user_id = user_id.clone();
    let invite =
        tokio::task::spawn_blocking(move || db_state.db.consume_invite(&token, &join_user_id))
            .await??;

    // Add the new member
    let member = MemberRecord {
        id: format!("m_{}", uuid::Uuid::new_v4()),
        org_id: invite.org_id.clone(),
        user_id: user_id.clone(),
        name: input.name.clone(),
        role: invite.role.clone(),
        joined_at: chrono::Utc::now().to_rfc3339(),
    };

    let db_state = state.clone();
    tokio::task::spawn_blocking(move || db_state.db.add_member(&member)).await??;

    // Issue JWT for the new member
    let exp = usize::try_from((chrono::Utc::now() + chrono::Duration::days(30)).timestamp())
        .unwrap_or(usize::MAX);

    let claims = Claims {
        sub: user_id.clone(),
        name: input.name.clone(),
        role: invite.role.clone(),
        org: invite.org_id.clone(),
        exp,
    };

    let jwt = auth::create_token(&claims, &state.config.server.jwt_secret)
        .map_err(|e| InviteServiceError::Internal(e.to_string()))?;

    let server_url = state.config.server.bind.clone();

    info!(
        user_id = %user_id,
        org_id = %invite.org_id,
        role = %invite.role,
        "user joined org via invite"
    );

    Ok(RedeemInviteOutput {
        org_id: invite.org_id,
        user_id,
        jwt,
        role: invite.role,
        server_url,
    })
}
