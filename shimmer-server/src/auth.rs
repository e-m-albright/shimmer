//! JWT authentication middleware.

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

/// JWT claims embedded in every auth token.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// User ID (e.g., `u_abc123`).
    pub sub: String,
    /// Display name.
    pub name: String,
    /// Role: "admin" or "member".
    pub role: String,
    /// Org ID.
    pub org: String,
    /// Expiration (Unix timestamp).
    pub exp: usize,
}

impl Claims {
    #[allow(dead_code)] // Used in Phase 4 (permissions)
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// Create a signed JWT token.
///
/// # Errors
///
/// Returns `jsonwebtoken::errors::Error` if signing fails.
#[allow(dead_code)] // Used in Phase 3 (invite flow) and tests
pub fn create_token(claims: &Claims, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// Verify and decode a JWT token.
///
/// # Errors
///
/// Returns `jsonwebtoken::errors::Error` if the token is invalid or expired.
pub fn verify_token(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

/// Extractor: pull authenticated user claims from the Authorization header.
impl FromRequestParts<Arc<AppState>> for Claims {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "missing authorization header"))?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "invalid authorization format"))?;

        verify_token(token, &state.config.server.jwt_secret)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid token"))
    }
}
