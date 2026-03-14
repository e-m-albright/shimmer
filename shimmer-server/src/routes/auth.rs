//! Auth routes — register, login, refresh.
//!
//! All endpoints are unauthenticated (no JWT required).

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::services::auth::{self, AuthError, LoginInput, RegisterInput};
use crate::AppState;

/// Register request — requires a valid invite token.
#[derive(Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub invite_token: String,
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8))]
    pub password: String,
    #[validate(length(min = 1, max = 100))]
    pub name: String,
}

/// Login request — email + password.
#[derive(Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    pub password: String,
}

/// Refresh request — rotate a refresh token.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Shared auth response with tokens.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

/// Map `AuthError` to HTTP status + message.
fn map_auth_err(e: AuthError) -> (StatusCode, String) {
    match e {
        AuthError::EmailTaken => (StatusCode::CONFLICT, e.to_string()),
        AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, e.to_string()),
        AuthError::InvalidRefreshToken => (StatusCode::UNAUTHORIZED, e.to_string()),
        AuthError::Db(_) | AuthError::Hash(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /api/auth/register — register via invite token.
pub async fn register_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Consume the invite token
    let token = req.invite_token.clone();
    let state_for_invite = state.clone();
    let invite =
        tokio::task::spawn_blocking(move || state_for_invite.db.consume_invite(&token, "pending"))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Build register input from request + invite data
    let input = RegisterInput {
        email: req.email,
        password: req.password,
        org_id: invite.org_id,
        role: invite.role,
        name: req.name,
    };

    let jwt_secret = state.config.server.jwt_secret.clone();
    let state_for_register = state.clone();
    let tokens = tokio::task::spawn_blocking(move || {
        auth::register(&state_for_register.db, &input, &jwt_secret)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(map_auth_err)?;

    Ok(Json(AuthResponse {
        user_id: tokens.user_id,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

/// POST /api/auth/login — email/password login.
pub async fn login_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let input = LoginInput {
        email: req.email,
        password: req.password,
    };

    let jwt_secret = state.config.server.jwt_secret.clone();
    let state_for_login = state.clone();
    let tokens =
        tokio::task::spawn_blocking(move || auth::login(&state_for_login.db, &input, &jwt_secret))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(map_auth_err)?;

    Ok(Json(AuthResponse {
        user_id: tokens.user_id,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

/// POST /api/auth/refresh — rotate refresh token.
pub async fn refresh_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    let jwt_secret = state.config.server.jwt_secret.clone();
    let state_for_refresh = state.clone();
    let tokens = tokio::task::spawn_blocking(move || {
        auth::refresh(&state_for_refresh.db, &req.refresh_token, &jwt_secret)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(map_auth_err)?;

    Ok(Json(AuthResponse {
        user_id: tokens.user_id,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}
