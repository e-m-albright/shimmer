//! Auth service — registration, login, and token refresh.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use sha2::{Digest, Sha256};
use tracing::info;

use crate::auth::{self, Claims};
use crate::db::{Database, DbError, MemberRecord};

/// Errors that can occur in auth service operations.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("email already registered")]
    EmailTaken,

    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("invalid or expired refresh token")]
    InvalidRefreshToken,

    #[error("database error: {0}")]
    Db(#[from] DbError),

    #[error("password hashing error: {0}")]
    Hash(String),
}

/// Input for user registration.
#[derive(Debug)]
pub struct RegisterInput {
    pub email: String,
    pub password: String,
    pub org_id: String,
    pub role: String,
    pub name: String,
}

/// Input for user login.
#[derive(Debug)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
}

/// Tokens returned from auth operations.
#[derive(Debug)]
pub struct AuthTokens {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

/// SHA-256 hex hash of a token string.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Issue an access token (JWT, 1hr) and refresh token (opaque, 30 days).
fn issue_tokens(
    db: &Database,
    user_id: &str,
    member: &MemberRecord,
    jwt_secret: &str,
) -> Result<AuthTokens, AuthError> {
    // Access token: 1 hour
    let exp = usize::try_from((chrono::Utc::now() + chrono::Duration::hours(1)).timestamp())
        .unwrap_or(usize::MAX);

    let claims = Claims {
        sub: user_id.to_string(),
        name: member.name.clone(),
        role: member.role.clone(),
        org: member.org_id.clone(),
        exp,
    };

    let access_token =
        auth::create_token(&claims, jwt_secret).map_err(|e| AuthError::Hash(e.to_string()))?;

    // Refresh token: opaque UUID, store SHA-256 hash in DB
    let raw_refresh = uuid::Uuid::new_v4().to_string();
    let token_hash = hash_token(&raw_refresh);
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    let token_id = format!("rt_{}", uuid::Uuid::new_v4());

    db.store_refresh_token(&token_id, user_id, &token_hash, &expires_at)?;

    Ok(AuthTokens {
        user_id: user_id.to_string(),
        access_token,
        refresh_token: raw_refresh,
    })
}

/// Register a new user with email and password.
///
/// Creates the user, adds them as a member of the specified org, and issues tokens.
///
/// # Errors
///
/// Returns `AuthError::EmailTaken` if the email is already registered.
/// Returns `AuthError::Hash` if password hashing fails.
/// Returns `AuthError::Db` on database errors.
pub fn register(
    db: &Database,
    input: &RegisterInput,
    jwt_secret: &str,
) -> Result<AuthTokens, AuthError> {
    // Check email not taken
    if db.get_user_by_email(&input.email)?.is_some() {
        return Err(AuthError::EmailTaken);
    }

    // Hash password with argon2
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(input.password.as_bytes(), &salt)
        .map_err(|e| AuthError::Hash(e.to_string()))?
        .to_string();

    // Create user
    let user_id = format!("u_{}", uuid::Uuid::new_v4());
    db.create_user(&user_id, &input.email, &password_hash)?;

    // Add as member to org
    let member = MemberRecord {
        id: format!("m_{}", uuid::Uuid::new_v4()),
        org_id: input.org_id.clone(),
        user_id: user_id.clone(),
        name: input.name.clone(),
        role: input.role.clone(),
        joined_at: chrono::Utc::now().to_rfc3339(),
    };
    db.add_member(&member)?;

    info!(user_id = %user_id, email = %input.email, org_id = %input.org_id, "user registered");

    issue_tokens(db, &user_id, &member, jwt_secret)
}

/// Log in with email and password.
///
/// # Errors
///
/// Returns `AuthError::InvalidCredentials` if the email or password is wrong.
/// Returns `AuthError::Db` on database errors.
pub fn login(db: &Database, input: &LoginInput, jwt_secret: &str) -> Result<AuthTokens, AuthError> {
    let user = db
        .get_user_by_email(&input.email)?
        .ok_or(AuthError::InvalidCredentials)?;

    // Verify password
    let parsed_hash =
        PasswordHash::new(&user.password_hash).map_err(|e| AuthError::Hash(e.to_string()))?;
    Argon2::default()
        .verify_password(input.password.as_bytes(), &parsed_hash)
        .map_err(|_| AuthError::InvalidCredentials)?;

    // Look up membership
    let member = db
        .get_member_by_user_id(&user.id)?
        .ok_or(AuthError::InvalidCredentials)?;

    info!(user_id = %user.id, email = %input.email, "user logged in");

    issue_tokens(db, &user.id, &member, jwt_secret)
}

/// Refresh an access token using a refresh token.
///
/// Performs token rotation: the old refresh token is deleted and a new one is issued.
///
/// # Errors
///
/// Returns `AuthError::InvalidRefreshToken` if the token is invalid or expired.
/// Returns `AuthError::Db` on database errors.
pub fn refresh(
    db: &Database,
    refresh_token: &str,
    jwt_secret: &str,
) -> Result<AuthTokens, AuthError> {
    let token_hash = hash_token(refresh_token);

    let stored = db
        .get_refresh_token_by_hash(&token_hash)?
        .ok_or(AuthError::InvalidRefreshToken)?;

    // Check expiry
    let expires_at = chrono::DateTime::parse_from_rfc3339(&stored.expires_at)
        .map_err(|e| AuthError::Hash(format!("bad expiry timestamp: {e}")))?;
    if expires_at < chrono::Utc::now() {
        db.delete_refresh_token(&stored.id)?;
        return Err(AuthError::InvalidRefreshToken);
    }

    // Rotate: delete old token
    db.delete_refresh_token(&stored.id)?;

    // Look up user + membership
    let user = db
        .get_user_by_id(&stored.user_id)?
        .ok_or(AuthError::InvalidRefreshToken)?;
    let member = db
        .get_member_by_user_id(&user.id)?
        .ok_or(AuthError::InvalidRefreshToken)?;

    info!(user_id = %user.id, "token refreshed");

    issue_tokens(db, &user.id, &member, jwt_secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, OrgRecord};

    fn setup_db() -> Database {
        let db = Database::open_in_memory().expect("open in-memory DB");
        let org = OrgRecord {
            id: "org_test".into(),
            name: "Test Org".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        db.create_org(&org).expect("create org");
        db
    }

    fn test_register_input() -> RegisterInput {
        RegisterInput {
            email: "alice@example.com".into(),
            password: "hunter2".into(),
            org_id: "org_test".into(),
            role: "member".into(),
            name: "Alice".into(),
        }
    }

    #[test]
    fn register_user_creates_user_and_member() {
        let db = setup_db();
        let tokens = register(&db, &test_register_input(), "test-secret").expect("register");

        // User exists
        let user = db
            .get_user_by_email("alice@example.com")
            .expect("get user")
            .expect("user exists");
        assert_eq!(user.id, tokens.user_id);

        // Member exists
        let member = db
            .get_member_by_user_id(&tokens.user_id)
            .expect("get member")
            .expect("member exists");
        assert_eq!(member.org_id, "org_test");
        assert_eq!(member.role, "member");
        assert_eq!(member.name, "Alice");

        // Tokens are non-empty
        assert!(!tokens.access_token.is_empty());
        assert!(!tokens.refresh_token.is_empty());
    }

    #[test]
    fn login_with_valid_credentials() {
        let db = setup_db();
        register(&db, &test_register_input(), "test-secret").expect("register");

        let tokens = login(
            &db,
            &LoginInput {
                email: "alice@example.com".into(),
                password: "hunter2".into(),
            },
            "test-secret",
        )
        .expect("login");

        assert!(!tokens.access_token.is_empty());
        assert!(!tokens.refresh_token.is_empty());
    }

    #[test]
    fn login_with_wrong_password_fails() {
        let db = setup_db();
        register(&db, &test_register_input(), "test-secret").expect("register");

        let result = login(
            &db,
            &LoginInput {
                email: "alice@example.com".into(),
                password: "wrong-password".into(),
            },
            "test-secret",
        );

        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[test]
    fn refresh_token_rotation() {
        let db = setup_db();
        let tokens = register(&db, &test_register_input(), "test-secret").expect("register");

        // Refresh with the first token
        let new_tokens = refresh(&db, &tokens.refresh_token, "test-secret").expect("refresh");
        assert!(!new_tokens.access_token.is_empty());
        assert_ne!(new_tokens.refresh_token, tokens.refresh_token);

        // Old token should no longer work (rotation)
        let reuse = refresh(&db, &tokens.refresh_token, "test-secret");
        assert!(matches!(reuse, Err(AuthError::InvalidRefreshToken)));
    }
}
