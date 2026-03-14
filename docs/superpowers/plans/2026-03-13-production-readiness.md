# Shimmer Production Readiness — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Shimmer downloadable, installable, and distributable — real auth, invite flow with KEK transport, admin CLI/TUI, .dmg distribution with auto-update.

**Architecture:** Extract a service layer from existing route handlers, add email/password auth with refresh tokens, rework the invite flow for zero-knowledge KEK transport, add a Ratatui TUI setup wizard, migrate key storage to macOS keychain, and add a first-launch onboarding screen.

**Tech Stack:** Rust (Axum, Ratatui, argon2, hkdf), Tauri v2 (keychain plugin, updater plugin), SvelteKit, SQLite

**Spec:** `docs/superpowers/specs/2026-03-13-production-readiness-design.md`

---

## Chunk 1: Config File Migration & Service Layer Extraction

### Task 1: Migrate Config from Flat to Nested TOML

**Files:**
- Modify: `shimmer-server/src/config.rs`
- Modify: `shimmer-server/src/main.rs`
- Modify: `shimmer-server/src/lib.rs`
- Modify: `shimmer-server/Cargo.toml`

- [ ] **Step 1: Write failing test for nested config parsing**

Add to `shimmer-server/src/config.rs` (in `#[cfg(test)] mod tests`):

```rust
#[test]
fn parse_nested_toml() {
    let toml = r#"
[server]
bind = "127.0.0.1:9000"
jwt_secret = "test-secret"

[storage]
backend = "file"
path = "/tmp/shimmer"

[database]
path = "/tmp/shimmer.db"

[org]
name = "Test Clinic"
"#;
    let config = ServerConfig::from_toml(toml).unwrap();
    assert_eq!(config.server.bind, "127.0.0.1:9000");
    assert_eq!(config.server.jwt_secret, "test-secret");
    assert_eq!(config.storage.backend, "file");
    assert_eq!(config.storage.path, Some("/tmp/shimmer".into()));
    assert_eq!(config.database.path, "/tmp/shimmer.db");
    assert_eq!(config.org.name, Some("Test Clinic".into()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `just test-filter parse_nested_toml`
Expected: FAIL — `from_toml` method doesn't exist, structs don't match

- [ ] **Step 3: Implement nested config structs**

Replace the flat `ServerConfig` in `shimmer-server/src/config.rs` with:

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub server: ServerSection,
    #[serde(default)]
    pub storage: StorageSection,
    #[serde(default)]
    pub database: DatabaseSection,
    #[serde(default)]
    pub org: OrgSection,
    #[serde(default)]
    pub smtp: Option<SmtpSection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_jwt_secret")]
    pub jwt_secret: String,
}

fn default_bind() -> String { "0.0.0.0:8443".into() }
fn default_jwt_secret() -> String { "dev-secret-change-in-production".into() }

impl Default for ServerSection {
    fn default() -> Self {
        Self { bind: default_bind(), jwt_secret: default_jwt_secret() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageSection {
    #[serde(default = "default_storage_backend")]
    pub backend: String,
    pub path: Option<String>,
    pub s3: Option<S3Section>,
}

fn default_storage_backend() -> String { "file".into() }

impl Default for StorageSection {
    fn default() -> Self {
        Self { backend: default_storage_backend(), path: None, s3: None }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct S3Section {
    pub endpoint: Option<String>,
    #[serde(default = "default_s3_bucket")]
    pub bucket: String,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
}

fn default_s3_bucket() -> String { "shimmer".into() }

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSection {
    #[serde(default = "default_db_path")]
    pub path: String,
}

fn default_db_path() -> String { "./shimmer-metadata.db".into() }

impl Default for DatabaseSection {
    fn default() -> Self {
        Self { path: default_db_path() }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OrgSection {
    pub name: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpSection {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
}

impl ServerConfig {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Load config: SHIMMER_CONFIG file → env var overrides → defaults.
    pub fn load() -> Self {
        let mut config = if let Ok(path) = std::env::var("SHIMMER_CONFIG") {
            let contents = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read config file {path}: {e}"));
            Self::from_toml(&contents)
                .unwrap_or_else(|e| panic!("invalid config file {path}: {e}"))
        } else {
            Self::default()
        };

        // Env var overrides (backwards compat)
        if let Ok(v) = std::env::var("HOST") {
            let port = std::env::var("PORT").unwrap_or_else(|_| "8443".into());
            config.server.bind = format!("{v}:{port}");
        }
        if let Ok(v) = std::env::var("JWT_SECRET") {
            config.server.jwt_secret = v;
        }
        if let Ok(v) = std::env::var("SHIMMER_STORAGE_BACKEND") {
            config.storage.backend = v;
        }
        if let Ok(v) = std::env::var("SHIMMER_STORAGE_PATH") {
            config.storage.path = Some(v);
        }
        if let Ok(v) = std::env::var("SHIMMER_DB_PATH") {
            config.database.path = v;
        }
        if let Ok(v) = std::env::var("SHIMMER_ORG_ID") {
            config.org.id = Some(v);
        }
        if let Ok(v) = std::env::var("SHIMMER_ORG_NAME") {
            config.org.name = Some(v);
        }
        // S3 env vars
        if let Ok(v) = std::env::var("SHIMMER_S3_ENDPOINT") {
            let s3 = config.storage.s3.get_or_insert(S3Section {
                endpoint: None, bucket: default_s3_bucket(),
                region: None, access_key_id: None, secret_access_key: None,
            });
            s3.endpoint = Some(v);
        }
        if let Ok(v) = std::env::var("SHIMMER_S3_BUCKET") {
            if let Some(s3) = config.storage.s3.as_mut() {
                s3.bucket = v;
            }
        }

        config
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            server: ServerSection::default(),
            storage: StorageSection::default(),
            database: DatabaseSection::default(),
            org: OrgSection::default(),
            smtp: None,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `just test-filter parse_nested_toml`
Expected: PASS

- [ ] **Step 5: Add test for env var overrides**

```rust
#[test]
fn env_vars_override_toml() {
    // Set env vars in a scoped manner
    std::env::set_var("JWT_SECRET", "from-env");
    let toml = r#"
[server]
jwt_secret = "from-toml"
"#;
    let mut config = ServerConfig::from_toml(toml).unwrap();
    // Simulate the override logic
    if let Ok(v) = std::env::var("JWT_SECRET") {
        config.server.jwt_secret = v;
    }
    assert_eq!(config.server.jwt_secret, "from-env");
    std::env::remove_var("JWT_SECRET");
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `just test-filter env_vars_override`
Expected: PASS

- [ ] **Step 7: Update main.rs and lib.rs to use new config**

Update `shimmer-server/src/main.rs` to use `ServerConfig::load()` and access fields via the new nested structure: `config.server.bind`, `config.server.jwt_secret`, `config.storage.backend`, etc.

Update `shimmer-server/src/lib.rs` `AppState` to store the full `ServerConfig` (it already does — just ensure the field accesses compile).

Key changes:
- `config.host` / `config.port` → `config.server.bind` (already a `host:port` string)
- `config.storage_backend` → `config.storage.backend`
- `config.s3_endpoint` → `config.storage.s3.as_ref().and_then(|s| s.endpoint.as_deref())`
- `config.db_path` → `config.database.path`
- `state.jwt_secret` → `state.config.server.jwt_secret` (remove the separate `jwt_secret` field from AppState)

- [ ] **Step 8: Run full test suite**

Run: `just test`
Expected: All tests pass

- [ ] **Step 9: Commit**

```bash
git add shimmer-server/src/config.rs shimmer-server/src/main.rs shimmer-server/src/lib.rs
git commit -m "refactor: migrate config from flat to nested TOML structure"
```

---

### Task 2: Extract Service Layer from Route Handlers

**Files:**
- Create: `shimmer-server/src/services/mod.rs`
- Create: `shimmer-server/src/services/paste.rs`
- Create: `shimmer-server/src/services/org.rs`
- Create: `shimmer-server/src/services/invite.rs`
- Modify: `shimmer-server/src/routes/paste.rs`
- Modify: `shimmer-server/src/routes/org.rs`
- Modify: `shimmer-server/src/routes/invite.rs`
- Modify: `shimmer-server/src/lib.rs` (add `pub mod services`)

The service layer takes `&Database` (and `&dyn Storage` where needed), contains the business logic, and returns domain types. Route handlers become thin wrappers that extract HTTP params, call the service, and map results to HTTP responses.

- [ ] **Step 1: Create services module skeleton**

Create `shimmer-server/src/services/mod.rs`:

```rust
pub mod invite;
pub mod org;
pub mod paste;
```

Add `pub mod services;` to `shimmer-server/src/lib.rs`.

- [ ] **Step 2: Extract paste service**

Create `shimmer-server/src/services/paste.rs`. Move the core logic from `routes/paste.rs` handlers into free functions:

```rust
use crate::db::Database;
use shimmer_core::storage::Storage;
use uuid::Uuid;
use chrono::Utc;

pub struct CreatePasteInput {
    pub ciphertext: String,
    pub content_type: String,
    pub visibility: String,
    pub search_tokens: Vec<(String, String)>,  // (token, token_type) pairs
    pub encrypted_title: Option<String>,
    pub encrypted_filename: Option<String>,
    pub ttl_hours: Option<u64>,
    pub burn_on_read: bool,
    pub user_id: String,
    pub user_name: String,
    pub org_id: String,
}

pub struct CreatePasteOutput {
    pub id: String,
    pub phi_url: String,
}

pub async fn create_paste(
    db: &Database,
    storage: &dyn Storage,
    input: CreatePasteInput,
) -> Result<CreatePasteOutput, PasteServiceError> {
    let id = Uuid::new_v4().to_string();
    let storage_key = format!("{}/{}", input.user_id, id);
    let ciphertext_bytes = input.ciphertext.as_bytes();

    storage.put(&storage_key, ciphertext_bytes).await
        .map_err(PasteServiceError::Storage)?;

    let now = Utc::now();
    let record = PasteRecord {
        id: id.clone(),
        org_id: input.org_id,
        user_id: input.user_id,
        user_name: input.user_name,
        content_type: input.content_type,
        encrypted_title: input.encrypted_title,
        encrypted_filename: input.encrypted_filename,
        visibility: input.visibility,
        size_bytes: ciphertext_bytes.len() as i64,
        ttl_hours: input.ttl_hours.map(|h| h as i64),
        burn_on_read: input.burn_on_read,
        created_at: now.to_rfc3339(),
        expires_at: input.ttl_hours.map(|h| {
            (now + chrono::Duration::hours(h as i64)).to_rfc3339()
        }),
    };

    // Note: Database wraps Mutex<Connection> and is not Clone.
    // The service layer receives a reference; for spawn_blocking, pass the
    // Arc<AppState> from the route handler and access db inside the closure.
    // Alternatively, restructure Database to be Arc-wrapped at the AppState level.
    // The exact pattern should follow the existing route handlers (clone state Arc, access db inside closure).
    let tokens = input.search_tokens;
    // Pseudocode — adapt to match actual AppState pattern:
    tokio::task::spawn_blocking(move || db_state.db.insert_paste(&record, &tokens))
        .await
        .map_err(|e| PasteServiceError::Internal(e.to_string()))?
        .map_err(PasteServiceError::Db)?;

    Ok(CreatePasteOutput {
        phi_url: format!("phi://{id}"),
        id,
    })
}

// Similarly extract: fetch_paste, list_pastes, search_pastes, delete_paste
// Each takes &Database + &dyn Storage + the relevant inputs, returns Result<Output, PasteServiceError>

#[derive(Debug, thiserror::Error)]
pub enum PasteServiceError {
    #[error("storage error: {0}")]
    Storage(shimmer_core::error::StorageError),
    #[error("database error: {0}")]
    Db(crate::db::DbError),
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("internal: {0}")]
    Internal(String),
}
```

- [ ] **Step 3: Update paste route handlers to call service**

Slim down `routes/paste.rs` handlers to:
1. Validate request (keep `validator` usage)
2. Extract claims
3. Build `CreatePasteInput` from request + claims
4. Call `services::paste::create_paste(&state.db, &*state.storage, input).await`
5. Map `PasteServiceError` to `(StatusCode, String)` responses

- [ ] **Step 4: Run tests to verify paste routes still work**

Run: `just test-server`
Expected: All paste tests pass

- [ ] **Step 5: Extract org and invite services**

Same pattern — move business logic from `routes/org.rs` → `services/org.rs` and `routes/invite.rs` → `services/invite.rs`. Handlers become thin wrappers.

Key service functions:
- `services::org::create_org(db, name, creator_id, creator_name) -> Result<String>`
- `services::org::list_members(db, org_id) -> Result<Vec<MemberInfo>>`
- `services::org::update_role(db, org_id, user_id, new_role, caller_id) -> Result<()>`
- `services::org::remove_member(db, org_id, user_id) -> Result<()>`
- `services::invite::create_invite(db, org_id, role, ttl_hours, single_use, created_by) -> Result<InviteRecord>`
- `services::invite::redeem_invite(db, token, user_id, name) -> Result<RedeemResult>`

- [ ] **Step 6: Run full test suite**

Run: `just test`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add shimmer-server/src/services/ shimmer-server/src/routes/ shimmer-server/src/lib.rs
git commit -m "refactor: extract service layer from route handlers"
```

---

## Chunk 2: Authentication System

### Task 3: Add Users Table and Auth Service

**Files:**
- Modify: `shimmer-server/src/db.rs` (add users + refresh_tokens tables)
- Create: `shimmer-server/src/services/auth.rs`
- Modify: `shimmer-server/src/services/mod.rs`
- Modify: `shimmer-server/Cargo.toml` (add argon2 dep)
- Modify: `Cargo.toml` (add argon2 to workspace deps)

- [ ] **Step 1: Add argon2 dependency**

Add to workspace `Cargo.toml` under `[workspace.dependencies]`:
```toml
argon2 = "0.5"
```

Add to `shimmer-server/Cargo.toml` under `[dependencies]`:
```toml
argon2 = { workspace = true }
```

- [ ] **Step 2: Write failing test for users table**

Add to `shimmer-server/src/db.rs` tests:

```rust
#[test]
fn create_and_get_user() {
    let db = Database::open_in_memory().unwrap();
    let user_id = "u_test123";
    let email = "alice@clinic.com";
    let password_hash = "$argon2id$v=19$m=19456,t=2,p=1$fake_hash";

    db.create_user(user_id, email, password_hash).unwrap();

    let user = db.get_user_by_email(email).unwrap().unwrap();
    assert_eq!(user.id, user_id);
    assert_eq!(user.email, email);
    assert_eq!(user.password_hash, password_hash);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `just test-filter create_and_get_user`
Expected: FAIL — `create_user` and `get_user_by_email` don't exist

- [ ] **Step 4: Implement users table and methods**

Add to the `init_tables` method in `db.rs` (after existing CREATE TABLE statements):

```sql
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS refresh_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Add structs and methods:

```rust
#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub expires_at: String,
}

impl Database {
    // NOTE: All methods use self.conn()? to lock the Mutex<Connection>,
    // matching the existing Database method pattern.

    pub fn create_user(&self, id: &str, email: &str, password_hash: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO users (id, email, password_hash) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, email, password_hash],
        )?;
        Ok(())
    }

    pub fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, email, password_hash, created_at FROM users WHERE email = ?1"
        )?;
        let mut rows = stmt.query_map(rusqlite::params![email], |row| {
            Ok(UserRecord {
                id: row.get(0)?,
                email: row.get(1)?,
                password_hash: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn get_user_by_id(&self, id: &str) -> Result<Option<UserRecord>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, email, password_hash, created_at FROM users WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], |row| {
            Ok(UserRecord {
                id: row.get(0)?,
                email: row.get(1)?,
                password_hash: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn store_refresh_token(&self, id: &str, user_id: &str, token_hash: &str, expires_at: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO refresh_tokens (id, user_id, token_hash, expires_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, user_id, token_hash, expires_at],
        )?;
        Ok(())
    }

    pub fn get_refresh_token_by_hash(&self, token_hash: &str) -> Result<Option<RefreshTokenRecord>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, user_id, token_hash, expires_at FROM refresh_tokens WHERE token_hash = ?1"
        )?;
        let mut rows = stmt.query_map(rusqlite::params![token_hash], |row| {
            Ok(RefreshTokenRecord {
                id: row.get(0)?,
                user_id: row.get(1)?,
                token_hash: row.get(2)?,
                expires_at: row.get(3)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn delete_refresh_token(&self, id: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM refresh_tokens WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    pub fn delete_refresh_tokens_for_user(&self, user_id: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM refresh_tokens WHERE user_id = ?1", rusqlite::params![user_id])?;
        Ok(())
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `just test-filter create_and_get_user`
Expected: PASS

- [ ] **Step 6: Write failing test for auth service — registration**

Create `shimmer-server/src/services/auth.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn register_user_creates_user_and_member() {
        let db = Database::open_in_memory().unwrap();
        // Create an org first
        db.create_org(&crate::db::OrgRecord {
            id: "org_test".into(),
            name: "Test Clinic".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }).unwrap();

        let result = register(
            &db,
            RegisterInput {
                email: "alice@clinic.com".into(),
                password: "strongP@ssw0rd!".into(),
                org_id: "org_test".into(),
                role: "member".into(),
                name: "Alice".into(),
            },
            "test-jwt-secret",
        ).unwrap();

        assert!(!result.user_id.is_empty());
        assert!(!result.access_token.is_empty());
        assert!(!result.refresh_token.is_empty());

        // Verify user was created
        let user = db.get_user_by_email("alice@clinic.com").unwrap().unwrap();
        assert_eq!(user.email, "alice@clinic.com");

        // Verify member was created
        let members = db.list_members("org_test").unwrap();
        assert_eq!(members.len(), 1);
    }
}
```

- [ ] **Step 7: Run test to verify it fails**

Run: `just test-filter register_user_creates`
Expected: FAIL — `register` function doesn't exist

- [ ] **Step 8: Implement auth service**

In `shimmer-server/src/services/auth.rs`:

```rust
use argon2::{Argon2, PasswordHasher, PasswordVerifier, password_hash::{SaltString, rand_core::OsRng}};
use crate::auth::{Claims, create_token};
use crate::db::Database;
use sha2::{Sha256, Digest};
use uuid::Uuid;
use chrono::{Utc, Duration};

pub struct RegisterInput {
    pub email: String,
    pub password: String,
    pub org_id: String,
    pub role: String,
    pub name: String,
}

pub struct AuthTokens {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

pub struct LoginInput {
    pub email: String,
    pub password: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("email already registered")]
    EmailTaken,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("invalid or expired refresh token")]
    InvalidRefreshToken,
    #[error("database error: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("password hashing error: {0}")]
    Hash(String),
}

pub fn register(db: &Database, input: RegisterInput, jwt_secret: &str) -> Result<AuthTokens, AuthError> {
    // Check email not taken
    if db.get_user_by_email(&input.email)?.is_some() {
        return Err(AuthError::EmailTaken);
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(input.password.as_bytes(), &salt)
        .map_err(|e| AuthError::Hash(e.to_string()))?
        .to_string();

    // Create user
    let user_id = format!("u_{}", Uuid::new_v4().simple());
    db.create_user(&user_id, &input.email, &password_hash)?;

    // Add as member
    let member = crate::db::MemberRecord {
        id: Uuid::new_v4().to_string(),
        org_id: input.org_id.clone(),
        user_id: user_id.clone(),
        name: input.name.clone(),
        role: input.role.clone(),
        joined_at: Utc::now().to_rfc3339(),
    };
    db.add_member(&member)?;

    // Issue tokens
    let tokens = issue_tokens(db, &user_id, &input.name, &input.role, &input.org_id, jwt_secret)?;
    Ok(tokens)
}

pub fn login(db: &Database, input: LoginInput, jwt_secret: &str) -> Result<AuthTokens, AuthError> {
    let user = db.get_user_by_email(&input.email)?
        .ok_or(AuthError::InvalidCredentials)?;

    // Verify password
    let parsed_hash = argon2::PasswordHash::new(&user.password_hash)
        .map_err(|e| AuthError::Hash(e.to_string()))?;
    Argon2::default()
        .verify_password(input.password.as_bytes(), &parsed_hash)
        .map_err(|_| AuthError::InvalidCredentials)?;

    // Look up membership to get role + org
    // For v1 we assume single-org: take the first membership
    let member = db.get_member_by_user_id(&user.id)?
        .ok_or(AuthError::InvalidCredentials)?;

    let tokens = issue_tokens(db, &user.id, &member.name, &member.role, &member.org_id, jwt_secret)?;
    Ok(tokens)
}

pub fn refresh(db: &Database, refresh_token: &str, jwt_secret: &str) -> Result<AuthTokens, AuthError> {
    let token_hash = hash_token(refresh_token);
    let record = db.get_refresh_token_by_hash(&token_hash)?
        .ok_or(AuthError::InvalidRefreshToken)?;

    // Check expiry
    let expires_at = chrono::DateTime::parse_from_rfc3339(&record.expires_at)
        .map_err(|_| AuthError::InvalidRefreshToken)?;
    if Utc::now() > expires_at {
        db.delete_refresh_token(&record.id)?;
        return Err(AuthError::InvalidRefreshToken);
    }

    // Delete old token (rotation)
    db.delete_refresh_token(&record.id)?;

    // Look up user + membership
    let user = db.get_user_by_id(&record.user_id)?
        .ok_or(AuthError::InvalidRefreshToken)?;
    let member = db.get_member_by_user_id(&user.id)?
        .ok_or(AuthError::InvalidRefreshToken)?;

    let tokens = issue_tokens(db, &user.id, &member.name, &member.role, &member.org_id, jwt_secret)?;
    Ok(tokens)
}

fn issue_tokens(
    db: &Database,
    user_id: &str,
    name: &str,
    role: &str,
    org_id: &str,
    jwt_secret: &str,
) -> Result<AuthTokens, AuthError> {
    // Access token: 1 hour
    let claims = Claims {
        sub: user_id.to_string(),
        name: name.to_string(),
        role: role.to_string(),
        org: org_id.to_string(),
        exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
    };
    let access_token = create_token(&claims, jwt_secret)
        .map_err(|e| AuthError::Hash(e.to_string()))?;

    // Refresh token: 30 days, opaque
    let refresh_raw = Uuid::new_v4().to_string();
    let refresh_hash = hash_token(&refresh_raw);
    let expires_at = (Utc::now() + Duration::days(30)).to_rfc3339();
    let token_id = Uuid::new_v4().to_string();
    db.store_refresh_token(&token_id, user_id, &refresh_hash, &expires_at)?;

    Ok(AuthTokens {
        user_id: user_id.to_string(),
        access_token,
        refresh_token: refresh_raw,
    })
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}
```

Add `pub mod auth;` to `services/mod.rs`.

- [ ] **Step 9: Add `get_member_by_user_id` to db.rs**

This method is needed by the auth service for login/refresh:

```rust
pub fn get_member_by_user_id(&self, user_id: &str) -> Result<Option<MemberRecord>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, org_id, user_id, name, role, joined_at FROM members WHERE user_id = ?1 LIMIT 1"
    )?;
    let mut rows = stmt.query_map(rusqlite::params![user_id], |row| {
        Ok(MemberRecord {
            id: row.get(0)?,
            org_id: row.get(1)?,
            user_id: row.get(2)?,
            name: row.get(3)?,
            role: row.get(4)?,
            joined_at: row.get(5)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}
```

- [ ] **Step 10: Run test to verify it passes**

Run: `just test-filter register_user_creates`
Expected: PASS

- [ ] **Step 11: Write and run tests for login and refresh**

Add to `services/auth.rs` tests:

```rust
#[test]
fn login_with_valid_credentials() {
    let db = Database::open_in_memory().unwrap();
    db.create_org(&crate::db::OrgRecord {
        id: "org_test".into(), name: "Test".into(), created_at: "2026-01-01T00:00:00Z".into(),
    }).unwrap();

    // Register first
    register(&db, RegisterInput {
        email: "bob@clinic.com".into(), password: "P@ssw0rd!".into(),
        org_id: "org_test".into(), role: "member".into(), name: "Bob".into(),
    }, "secret").unwrap();

    // Login
    let result = login(&db, LoginInput {
        email: "bob@clinic.com".into(), password: "P@ssw0rd!".into(),
    }, "secret").unwrap();

    assert!(!result.access_token.is_empty());
    assert!(!result.refresh_token.is_empty());
}

#[test]
fn login_with_wrong_password_fails() {
    let db = Database::open_in_memory().unwrap();
    db.create_org(&crate::db::OrgRecord {
        id: "org_test".into(), name: "Test".into(), created_at: "2026-01-01T00:00:00Z".into(),
    }).unwrap();

    register(&db, RegisterInput {
        email: "carol@clinic.com".into(), password: "correct".into(),
        org_id: "org_test".into(), role: "member".into(), name: "Carol".into(),
    }, "secret").unwrap();

    let result = login(&db, LoginInput {
        email: "carol@clinic.com".into(), password: "wrong".into(),
    }, "secret");

    assert!(matches!(result, Err(AuthError::InvalidCredentials)));
}

#[test]
fn refresh_token_rotation() {
    let db = Database::open_in_memory().unwrap();
    db.create_org(&crate::db::OrgRecord {
        id: "org_test".into(), name: "Test".into(), created_at: "2026-01-01T00:00:00Z".into(),
    }).unwrap();

    let reg = register(&db, RegisterInput {
        email: "dave@clinic.com".into(), password: "P@ss".into(),
        org_id: "org_test".into(), role: "member".into(), name: "Dave".into(),
    }, "secret").unwrap();

    // Refresh with the token we got
    let refreshed = refresh(&db, &reg.refresh_token, "secret").unwrap();
    assert!(!refreshed.access_token.is_empty());

    // Old refresh token is consumed — using it again should fail
    let result = refresh(&db, &reg.refresh_token, "secret");
    assert!(matches!(result, Err(AuthError::InvalidRefreshToken)));
}
```

- [ ] **Step 12: Run all auth tests**

Run: `just test-filter "login_with\|refresh_token_rotation\|register_user"`
Expected: All PASS

- [ ] **Step 13: Commit**

```bash
git add shimmer-server/src/db.rs shimmer-server/src/services/auth.rs shimmer-server/src/services/mod.rs shimmer-server/Cargo.toml Cargo.toml
git commit -m "feat: add users table, refresh tokens, and auth service (register/login/refresh)"
```

---

### Task 4: Add Auth API Routes

**Files:**
- Create: `shimmer-server/src/routes/auth.rs`
- Modify: `shimmer-server/src/routes/mod.rs`
- Modify: `shimmer-server/tests/api_test.rs`

- [ ] **Step 1: Write failing integration test for register endpoint**

Add to `shimmer-server/tests/api_test.rs`:

```rust
#[tokio::test]
async fn register_with_valid_invite() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, admin_token) = test_server(&tmp);

    // Admin creates invite
    let invite_resp = server.post("/api/org/invite")
        .add_header("Authorization".parse().unwrap(), format!("Bearer {admin_token}").parse().unwrap())
        .json(&serde_json::json!({
            "role": "member",
            "ttlHours": 24,
            "singleUse": true
        }))
        .await;
    invite_resp.assert_status(StatusCode::CREATED);
    let invite: serde_json::Value = invite_resp.json();
    let token = invite["token"].as_str().unwrap();

    // Register with the invite token
    let reg_resp = server.post("/api/auth/register")
        .json(&serde_json::json!({
            "inviteToken": token,
            "email": "newuser@clinic.com",
            "password": "Str0ngP@ss!",
            "name": "New User"
        }))
        .await;
    reg_resp.assert_status_ok();
    let body: serde_json::Value = reg_resp.json();
    assert!(body["accessToken"].is_string());
    assert!(body["refreshToken"].is_string());
    assert!(body["userId"].is_string());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `just test-filter register_with_valid_invite`
Expected: FAIL — route doesn't exist (404)

- [ ] **Step 3: Implement auth routes**

Create `shimmer-server/src/routes/auth.rs`:

```rust
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use validator::Validate;
use std::sync::Arc;
use crate::AppState;
use crate::services::auth::{self, RegisterInput, LoginInput, AuthError};

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

#[derive(Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

pub async fn register_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, String)> {
    req.validate().map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Redeem invite first (reuse existing invite service logic)
    // Note: Database is not Clone (wraps Mutex<Connection>). Clone the Arc<AppState> instead.
    let db_state = state.clone();
    let token = req.invite_token.clone();
    let invite = tokio::task::spawn_blocking(move || db_state.db.consume_invite(&token, "pending"))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid invite: {e}")))?;

    // Register user with org/role from invite
    let db_state = state.clone();
    let jwt_secret = state.config.server.jwt_secret.clone();
    let input = RegisterInput {
        email: req.email,
        password: req.password,
        org_id: invite.org_id,
        role: invite.role,
        name: req.name,
    };

    let tokens = tokio::task::spawn_blocking(move || auth::register(&db_state.db, input, &jwt_secret))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| match e {
            AuthError::EmailTaken => (StatusCode::CONFLICT, "email already registered".into()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    Ok((StatusCode::OK, Json(AuthResponse {
        user_id: tokens.user_id,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    })))
}

pub async fn login_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    req.validate().map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let db_state = state.clone();
    let jwt_secret = state.config.server.jwt_secret.clone();
    let input = LoginInput { email: req.email, password: req.password };

    let tokens = tokio::task::spawn_blocking(move || auth::login(&db_state.db, input, &jwt_secret))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| match e {
            AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid credentials".into()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    Ok(Json(AuthResponse {
        user_id: tokens.user_id,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

pub async fn refresh_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    let db_state = state.clone();
    let jwt_secret = state.config.server.jwt_secret.clone();
    let refresh_token = req.refresh_token;

    let tokens = tokio::task::spawn_blocking(move || auth::refresh(&db_state.db, &refresh_token, &jwt_secret))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| match e {
            AuthError::InvalidRefreshToken => (StatusCode::UNAUTHORIZED, "invalid or expired refresh token".into()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    Ok(Json(AuthResponse {
        user_id: tokens.user_id,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}
```

- [ ] **Step 4: Register auth routes and remove old join endpoint in mod.rs**

Add to `shimmer-server/src/routes/mod.rs`:

```rust
pub mod auth;
```

Add to the router in `build_router`:
```rust
.route("/api/auth/register", post(auth::register_handler))
.route("/api/auth/login", post(auth::login_handler))
.route("/api/auth/refresh", post(auth::refresh_handler))
```

**Remove** the old unauthenticated join endpoint from the router:
```rust
// DELETE this line:
.route("/api/org/join", post(invite::join_org))
```

This is critical — the old endpoint issues JWTs without password authentication and must be replaced by `/api/auth/register`.

- [ ] **Step 5: Run integration test**

Run: `just test-filter register_with_valid_invite`
Expected: PASS

- [ ] **Step 6: Write and run integration tests for login and refresh**

Add login and refresh integration tests to `api_test.rs` following the same pattern — create user via register, then test login, then test refresh with the returned token. Also test error cases (wrong password → 401, bad refresh token → 401).

- [ ] **Step 7: Run full test suite**

Run: `just test`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add shimmer-server/src/routes/auth.rs shimmer-server/src/routes/mod.rs shimmer-server/tests/api_test.rs
git commit -m "feat: add auth API routes (register, login, refresh)"
```

---

## Chunk 3: KEK Transport & Invite Flow Rework

### Task 5: Add KEK Wrapping to shimmer-core

**Files:**
- Modify: `shimmer-core/src/encryption.rs`
- Modify: `shimmer-core/Cargo.toml`
- Modify: `Cargo.toml` (workspace deps — add hkdf)

- [ ] **Step 1: Add hkdf dependency**

Add to workspace `Cargo.toml`:
```toml
hkdf = "0.12"
```

Add to `shimmer-core/Cargo.toml`:
```toml
hkdf = { workspace = true }
```

- [ ] **Step 2: Write failing test for KEK wrapping**

Add to `shimmer-core/src/encryption.rs` tests:

```rust
#[test]
fn wrap_and_unwrap_kek_with_invite_token() {
    let kek = generate_key();
    let invite_token = "dGVzdC1pbnZpdGUtdG9rZW4tMzItYnl0ZXMtbG9uZw"; // base64url

    let wrapped = wrap_kek_for_invite(&kek, invite_token).unwrap();
    let unwrapped = unwrap_kek_from_invite(&wrapped, invite_token).unwrap();

    assert_eq!(kek, unwrapped);
}

#[test]
fn unwrap_kek_with_wrong_token_fails() {
    let kek = generate_key();
    let wrapped = wrap_kek_for_invite(&kek, "correct-token").unwrap();
    let result = unwrap_kek_from_invite(&wrapped, "wrong-token");
    assert!(result.is_err());
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `just test-filter wrap_and_unwrap_kek`
Expected: FAIL — functions don't exist

- [ ] **Step 4: Implement KEK wrapping functions**

Add to `shimmer-core/src/encryption.rs`:

```rust
use hkdf::Hkdf;

const KEK_WRAP_SALT: &[u8] = b"shimmer-kek-wrap";
const KEK_WRAP_INFO: &[u8] = b"v1";

/// Derive a wrapping key from an invite token using HKDF-SHA256.
fn derive_wrapping_key(invite_token: &str) -> [u8; KEY_LEN] {
    let hk = Hkdf::<sha2::Sha256>::new(Some(KEK_WRAP_SALT), invite_token.as_bytes());
    let mut okm = [0u8; KEY_LEN];
    hk.expand(KEK_WRAP_INFO, &mut okm)
        .expect("HKDF expand should not fail for 32-byte output");
    okm
}

/// Encrypt the org KEK for transport in an invite URL fragment.
/// Returns base64url-encoded `nonce || ciphertext || tag`.
pub fn wrap_kek_for_invite(kek: &[u8; KEY_LEN], invite_token: &str) -> Result<String, CryptoError> {
    use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
    use aes_gcm::Nonce;

    let wrapping_key = derive_wrapping_key(invite_token);
    let cipher = Aes256Gcm::new_from_slice(&wrapping_key)
        .map_err(|e| CryptoError::InvalidKey(e.to_string()))?;

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, kek.as_ref())
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;

    // nonce (12 bytes) || ciphertext+tag
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    Ok(URL_SAFE_NO_PAD.encode(&combined))
}

/// Decrypt the org KEK from an invite URL fragment.
pub fn unwrap_kek_from_invite(wrapped: &str, invite_token: &str) -> Result<[u8; KEY_LEN], CryptoError> {
    use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
    use aes_gcm::Nonce;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    let combined = URL_SAFE_NO_PAD.decode(wrapped)
        .map_err(|e| CryptoError::Decrypt(format!("invalid base64: {e}")))?;

    if combined.len() < 12 {
        return Err(CryptoError::Decrypt("payload too short".into()));
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let wrapping_key = derive_wrapping_key(invite_token);
    let cipher = Aes256Gcm::new_from_slice(&wrapping_key)
        .map_err(|e| CryptoError::InvalidKey(e.to_string()))?;

    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::Decrypt("KEK decryption failed — invalid invite token or corrupted data".into()))?;

    if plaintext.len() != KEY_LEN {
        return Err(CryptoError::InvalidKey("decrypted KEK has wrong length".into()));
    }

    let mut kek = [0u8; KEY_LEN];
    kek.copy_from_slice(&plaintext);
    Ok(kek)
}
```

Note: `generate_nonce()` should already exist or be a simple 12-byte random generation. If not, add:
```rust
fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill(&mut nonce);
    nonce
}
```

- [ ] **Step 5: Run tests**

Run: `just test-filter "wrap_and_unwrap_kek\|unwrap_kek_with_wrong"`
Expected: Both PASS

- [ ] **Step 6: Commit**

```bash
git add shimmer-core/src/encryption.rs shimmer-core/Cargo.toml Cargo.toml
git commit -m "feat: add KEK wrapping for invite URL transport (HKDF-SHA256 + AES-256-GCM)"
```

---

### Task 6: Rework Invite Flow for Two-Phase KEK Transport

**Files:**
- Modify: `shimmer-server/src/routes/invite.rs`
- Modify: `shimmer-server/src/services/invite.rs`
- Modify: `shimmer-server/src/db.rs` (update invite token generation)
- Modify: `shimmer-server/tests/api_test.rs`

- [ ] **Step 1: Update invite token generation to use 32-byte random tokens**

In `services/invite.rs`, change invite token generation from UUID to:

```rust
use rand::RngCore;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

fn generate_invite_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
```

- [ ] **Step 2: Add `GET /api/org/invites` endpoint for pending invites**

This lets the admin's desktop app discover pending invites so it can attach the encrypted KEK.

Add to `services/invite.rs`:
```rust
pub fn list_pending_invites(db: &Database, org_id: &str) -> Result<Vec<InviteRecord>, rusqlite::Error> {
    db.list_pending_invites(org_id)
}
```

Add to `db.rs`:
```rust
pub fn list_pending_invites(&self, org_id: &str) -> Result<Vec<InviteRecord>> {
    let mut stmt = self.conn.prepare(
        "SELECT token, org_id, role, created_by, expires_at, used_at, used_by, single_use
         FROM invites
         WHERE org_id = ?1 AND used_at IS NULL AND expires_at > datetime('now')
         ORDER BY expires_at ASC"
    )?;
    let rows = stmt.query_map(rusqlite::params![org_id], |row| {
        Ok(InviteRecord {
            token: row.get(0)?,
            org_id: row.get(1)?,
            role: row.get(2)?,
            created_by: row.get(3)?,
            expires_at: row.get(4)?,
            used_at: row.get(5)?,
            used_by: row.get(6)?,
            single_use: row.get(7)?,
        })
    })?;
    rows.collect()
}
```

Add route in `routes/invite.rs`:
```rust
pub async fn list_invites_handler(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Result<Json<Vec<InviteResponse>>, (StatusCode, String)> {
    if !claims.is_admin() {
        return Err((StatusCode::FORBIDDEN, "admin only".into()));
    }
    let db = state.db.clone();
    let org_id = claims.org.clone();
    let invites = tokio::task::spawn_blocking(move || {
        crate::services::invite::list_pending_invites(&db, &org_id)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(invites.into_iter().map(|i| InviteResponse {
        token: i.token,
        org_id: i.org_id,
        expires_at: i.expires_at,
    }).collect()))
}
```

Register in `routes/mod.rs`:
```rust
.route("/api/org/invites", get(invite::list_invites_handler))
```

- [ ] **Step 3: Update the register endpoint to work with new token format**

The `POST /api/auth/register` endpoint already consumes an invite token. Since we changed from UUID to base64url, update `db.consume_invite` to work with the new format (it should already work since it's just a string comparison — but verify the test).

- [ ] **Step 4: Write integration test for the full two-phase flow**

```rust
#[tokio::test]
async fn two_phase_invite_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, admin_token) = test_server(&tmp);

    // Phase 1: Admin creates invite
    let invite_resp = server.post("/api/org/invite")
        .add_header("Authorization".parse().unwrap(), format!("Bearer {admin_token}").parse().unwrap())
        .json(&serde_json::json!({ "role": "member", "ttlHours": 24, "singleUse": true }))
        .await;
    invite_resp.assert_status(StatusCode::CREATED);
    let invite_token = invite_resp.json::<serde_json::Value>()["token"].as_str().unwrap().to_string();

    // Admin can list pending invites
    let list_resp = server.get("/api/org/invites")
        .add_header("Authorization".parse().unwrap(), format!("Bearer {admin_token}").parse().unwrap())
        .await;
    list_resp.assert_status_ok();
    let invites: Vec<serde_json::Value> = list_resp.json();
    assert_eq!(invites.len(), 1);

    // Phase 2: New user registers with invite
    let reg_resp = server.post("/api/auth/register")
        .json(&serde_json::json!({
            "inviteToken": invite_token,
            "email": "newbie@clinic.com",
            "password": "Str0ngP@ss!",
            "name": "Newbie"
        }))
        .await;
    reg_resp.assert_status_ok();

    // Invite is now consumed — listing should be empty
    let list_resp2 = server.get("/api/org/invites")
        .add_header("Authorization".parse().unwrap(), format!("Bearer {admin_token}").parse().unwrap())
        .await;
    let invites2: Vec<serde_json::Value> = list_resp2.json();
    assert_eq!(invites2.len(), 0);
}
```

- [ ] **Step 5: Run tests**

Run: `just test-filter two_phase_invite`
Expected: PASS

- [ ] **Step 6: Run full test suite**

Run: `just test`
Expected: All pass (update any existing invite tests that break due to token format change)

- [ ] **Step 7: Commit**

```bash
git add shimmer-server/src/services/invite.rs shimmer-server/src/routes/invite.rs shimmer-server/src/routes/mod.rs shimmer-server/src/db.rs shimmer-server/tests/api_test.rs
git commit -m "feat: rework invite flow for two-phase KEK transport with 256-bit tokens"
```

---

## Chunk 4: Admin CLI & TUI Setup Wizard

### Task 7: Add CLI Subcommand Structure

**Files:**
- Modify: `shimmer-server/src/main.rs`
- Modify: `shimmer-server/Cargo.toml` (add clap)
- Modify: `Cargo.toml` (workspace dep)

- [ ] **Step 1: Add clap dependency**

Add to workspace `Cargo.toml`:
```toml
clap = { version = "4", features = ["derive"] }
```

Add to `shimmer-server/Cargo.toml`:
```toml
clap = { workspace = true }
```

- [ ] **Step 2: Implement CLI argument parser**

Replace the direct server startup in `shimmer-server/src/main.rs` with a `clap`-based CLI:

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "shimmer-server", about = "Shimmer PHI sharing server")]
struct Cli {
    /// Path to config file
    #[arg(long, global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the API server
    Serve,
    /// Interactive setup wizard
    Setup,
    /// Admin operations
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
}

#[derive(Subcommand)]
enum AdminAction {
    /// Send an invite to join the org
    Invite {
        /// Email address to invite
        email: String,
    },
    /// List org members
    ListMembers,
    /// Remove a member
    Remove {
        /// Email of member to remove
        email: String,
    },
    /// Change a member's role
    SetRole {
        /// Email of member
        email: String,
        /// New role: admin, member, or read_only
        role: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Set SHIMMER_CONFIG if --config provided
    if let Some(path) = &cli.config {
        std::env::set_var("SHIMMER_CONFIG", path);
    }

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => run_server().await,
        Command::Setup => run_setup().await,
        Command::Admin { action } => run_admin(action).await,
    }
}

async fn run_server() {
    // Move existing main() logic here
}

async fn run_setup() {
    todo!("TUI setup wizard — Task 8")
}

async fn run_admin(action: AdminAction) {
    let config = config::ServerConfig::load();
    let db = db::Database::open(&config.database.path)
        .expect("failed to open database");

    match action {
        AdminAction::Invite { email } => {
            let org = config.org.id.as_deref()
                .expect("org.id must be set in config");
            let invite = services::invite::create_invite(
                &db, org, "member", 72, true, "admin-cli",
            ).expect("failed to create invite");
            println!("Invite created for {email}");
            println!("Token: {}", invite.token);
            println!("Partial invite URL: phi://join/{}", invite.token);
            println!("\nComplete this invite in the desktop app to attach the encrypted KEK.");
        }
        AdminAction::ListMembers => {
            let org = config.org.id.as_deref()
                .expect("org.id must be set in config");
            let members = db.list_members(org).expect("failed to list members");
            println!("{:<20} {:<30} {:<10}", "NAME", "USER ID", "ROLE");
            for m in members {
                println!("{:<20} {:<30} {:<10}", m.name, m.user_id, m.role);
            }
        }
        AdminAction::Remove { email } => {
            let user = db.get_user_by_email(&email)
                .expect("db error")
                .expect("user not found");
            let org = config.org.id.as_deref()
                .expect("org.id must be set in config");
            db.remove_member(org, &user.id).expect("failed to remove");
            db.delete_refresh_tokens_for_user(&user.id).expect("failed to revoke tokens");
            println!("Removed {email} and revoked all sessions");
        }
        AdminAction::SetRole { email, role } => {
            let valid_roles = ["admin", "member", "read_only"];
            let role_lower = role.to_lowercase();
            if !valid_roles.contains(&role_lower.as_str()) {
                eprintln!("Invalid role. Must be one of: {}", valid_roles.join(", "));
                std::process::exit(1);
            }
            let user = db.get_user_by_email(&email)
                .expect("db error")
                .expect("user not found");
            let org = config.org.id.as_deref()
                .expect("org.id must be set in config");
            db.update_member_role(org, &user.id, &role_lower).expect("failed to update role");
            println!("Set {email} role to {role_lower}");
        }
    }
}
```

- [ ] **Step 3: Run to verify it compiles**

Run: `cargo build -p shimmer-server`
Expected: Compiles (the `todo!()` for setup is fine for now)

- [ ] **Step 4: Run full test suite**

Run: `just test`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add shimmer-server/src/main.rs shimmer-server/Cargo.toml Cargo.toml
git commit -m "feat: add CLI subcommand structure (serve, setup, admin)"
```

---

### Task 8: TUI Setup Wizard

**Files:**
- Create: `shimmer-server/src/tui/mod.rs`
- Create: `shimmer-server/src/tui/setup.rs`
- Modify: `shimmer-server/src/main.rs` (wire up `run_setup`)
- Modify: `shimmer-server/src/lib.rs` (add `pub mod tui`)
- Modify: `shimmer-server/Cargo.toml` (add ratatui, crossterm)

- [ ] **Step 1: Add TUI dependencies**

Add to workspace `Cargo.toml`:
```toml
ratatui = "0.29"
crossterm = "0.28"
```

Add to `shimmer-server/Cargo.toml`:
```toml
ratatui = { workspace = true }
crossterm = { workspace = true }
```

- [ ] **Step 2: Create TUI module skeleton**

Create `shimmer-server/src/tui/mod.rs`:
```rust
pub mod setup;
```

Create `shimmer-server/src/tui/setup.rs` with the setup wizard implementation. The wizard is a multi-step form using Ratatui:

```rust
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use ratatui::{prelude::*, widgets::*};
use std::io;

#[derive(Debug, Clone, Default)]
pub struct SetupConfig {
    pub bind: String,
    pub storage_backend: String,
    pub storage_path: String,
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub db_path: String,
    pub org_name: String,
    pub admin_email: String,
    pub admin_password: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_from: String,
    pub skip_smtp: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    StorageBackend,
    StoragePath,
    S3Config,
    DbPath,
    OrgName,
    AdminEmail,
    AdminPassword,
    SmtpChoice,
    SmtpConfig,
    Confirm,
    Done,
}

pub fn run_setup_wizard() -> io::Result<Option<SetupConfig>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut config = SetupConfig {
        bind: "0.0.0.0:8443".into(),
        storage_backend: "file".into(),
        storage_path: "./shimmer-storage".into(),
        s3_bucket: "shimmer".into(),
        db_path: "./shimmer-metadata.db".into(),
        ..Default::default()
    };
    let mut step = Step::StorageBackend;
    let mut input = String::new();
    let mut selected = 0usize; // for choice fields
    let mut error_msg: Option<String> = None;

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::vertical([
                Constraint::Length(3),  // title
                Constraint::Length(3),  // progress
                Constraint::Min(10),   // content
                Constraint::Length(3),  // help
            ]).split(area);

            // Title
            let title = Paragraph::new("Shimmer Server Setup")
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::BOTTOM));
            f.render_widget(title, chunks[0]);

            // Progress
            let steps = ["Storage", "Database", "Org", "Admin", "SMTP", "Confirm"];
            let step_idx = match step {
                Step::StorageBackend | Step::StoragePath | Step::S3Config => 0,
                Step::DbPath => 1,
                Step::OrgName => 2,
                Step::AdminEmail | Step::AdminPassword => 3,
                Step::SmtpChoice | Step::SmtpConfig => 4,
                Step::Confirm | Step::Done => 5,
            };
            let progress = steps.iter().enumerate().map(|(i, s)| {
                if i < step_idx { format!("[x] {s}") }
                else if i == step_idx { format!("[>] {s}") }
                else { format!("[ ] {s}") }
            }).collect::<Vec<_>>().join("  ");
            let progress_widget = Paragraph::new(progress)
                .alignment(Alignment::Center);
            f.render_widget(progress_widget, chunks[1]);

            // Content — varies by step
            let content = match step {
                Step::StorageBackend => {
                    let items = vec!["File storage (local disk)", "S3-compatible (AWS, MinIO)"];
                    let list = List::new(items.iter().enumerate().map(|(i, item)| {
                        let prefix = if i == selected { "▸ " } else { "  " };
                        ListItem::new(format!("{prefix}{item}"))
                    }).collect::<Vec<_>>())
                    .block(Block::default().title("Storage Backend").borders(Borders::ALL));
                    list
                }
                _ => {
                    // Text input steps rendered as paragraphs with input field
                    let label = match step {
                        Step::StoragePath => "Storage path:",
                        Step::DbPath => "Database path:",
                        Step::OrgName => "Organization name:",
                        Step::AdminEmail => "Admin email:",
                        Step::AdminPassword => "Admin password:",
                        Step::SmtpChoice => "Configure SMTP for email invites?",
                        _ => "",
                    };
                    List::new(vec![
                        ListItem::new(label),
                        ListItem::new(format!("> {input}█")),
                    ]).block(Block::default().borders(Borders::ALL))
                }
            };
            f.render_widget(content, chunks[2]);

            // Help bar
            let help = match step {
                Step::StorageBackend => "↑↓ select  Enter confirm  q quit",
                Step::SmtpChoice => "y/n  q quit",
                Step::Confirm => "Enter save config  q quit",
                _ => "Enter confirm  Esc back  q quit",
            };
            let help_widget = Paragraph::new(help)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(help_widget, chunks[3]);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press { continue; }
            match key.code {
                KeyCode::Char('q') => {
                    disable_raw_mode()?;
                    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                    return Ok(None);
                }
                KeyCode::Enter => {
                    // Advance to next step, saving current input
                    match step {
                        Step::StorageBackend => {
                            config.storage_backend = if selected == 0 { "file".into() } else { "s3".into() };
                            step = if selected == 0 { Step::StoragePath } else { Step::S3Config };
                            input.clear();
                        }
                        Step::StoragePath => {
                            if !input.is_empty() { config.storage_path = input.clone(); }
                            step = Step::DbPath;
                            input.clear();
                        }
                        Step::S3Config => {
                            // Simplified: would need sub-steps for each S3 field
                            step = Step::DbPath;
                            input.clear();
                        }
                        Step::DbPath => {
                            if !input.is_empty() { config.db_path = input.clone(); }
                            step = Step::OrgName;
                            input.clear();
                        }
                        Step::OrgName => {
                            if input.is_empty() {
                                error_msg = Some("Org name is required".into());
                            } else {
                                config.org_name = input.clone();
                                step = Step::AdminEmail;
                                input.clear();
                            }
                        }
                        Step::AdminEmail => {
                            if input.is_empty() || !input.contains('@') {
                                error_msg = Some("Valid email required".into());
                            } else {
                                config.admin_email = input.clone();
                                step = Step::AdminPassword;
                                input.clear();
                            }
                        }
                        Step::AdminPassword => {
                            if input.len() < 8 {
                                error_msg = Some("Password must be 8+ characters".into());
                            } else {
                                config.admin_password = input.clone();
                                step = Step::SmtpChoice;
                                input.clear();
                            }
                        }
                        Step::SmtpChoice => {
                            step = Step::Confirm;
                        }
                        Step::Confirm => {
                            step = Step::Done;
                        }
                        Step::Done => {}
                    }
                }
                KeyCode::Up if step == Step::StorageBackend => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down if step == Step::StorageBackend => {
                    selected = (selected + 1).min(1);
                }
                KeyCode::Char('y') if step == Step::SmtpChoice => {
                    config.skip_smtp = false;
                    step = Step::SmtpConfig;
                    input.clear();
                }
                KeyCode::Char('n') if step == Step::SmtpChoice => {
                    config.skip_smtp = true;
                    step = Step::Confirm;
                }
                KeyCode::Char(c) => { input.push(c); }
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Esc => {
                    // Go back one step (simplified)
                    step = match step {
                        Step::StoragePath | Step::S3Config => Step::StorageBackend,
                        Step::DbPath => Step::StoragePath,
                        Step::OrgName => Step::DbPath,
                        Step::AdminEmail => Step::OrgName,
                        Step::AdminPassword => Step::AdminEmail,
                        Step::SmtpChoice => Step::AdminPassword,
                        Step::Confirm => Step::SmtpChoice,
                        other => other,
                    };
                    input.clear();
                }
                _ => {}
            }
        }

        if step == Step::Done {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(Some(config))
}
```

- [ ] **Step 3: Wire up config file writing**

Add a function to `tui/setup.rs` that takes a `SetupConfig` and writes `shimmer.toml`:

```rust
pub fn write_config_file(config: &SetupConfig, path: &str) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let jwt_secret = generate_jwt_secret();
    let mut toml = format!(
        r#"# WARNING: Contains secrets. Do not share or commit this file.
# File permissions: 0600 (owner read/write only).

[server]
bind = "{}"
jwt_secret = "{}"

[storage]
backend = "{}"
"#,
        config.bind, jwt_secret, config.storage_backend,
    );

    if config.storage_backend == "file" {
        toml.push_str(&format!("path = \"{}\"\n", config.storage_path));
    } else {
        toml.push_str(&format!(
            r#"
[storage.s3]
endpoint = "{}"
bucket = "{}"
access_key_id = "{}"
secret_access_key = "{}"
"#,
            config.s3_endpoint, config.s3_bucket,
            config.s3_access_key, config.s3_secret_key,
        ));
    }

    toml.push_str(&format!(
        r#"
[database]
path = "{}"

[org]
name = "{}"
"#,
        config.db_path, config.org_name,
    ));

    if !config.skip_smtp {
        toml.push_str(&format!(
            r#"
[smtp]
host = "{}"
port = {}
username = "{}"
password = "{}"
from = "{}"
"#,
            config.smtp_host, config.smtp_port,
            config.smtp_username, config.smtp_password, config.smtp_from,
        ));
    }

    // Write with 0600 permissions
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    use std::io::Write;
    file.write_all(toml.as_bytes())?;

    Ok(())
}

fn generate_jwt_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
```

- [ ] **Step 4: Wire `run_setup` in main.rs**

```rust
async fn run_setup() {
    use tui::setup;

    println!("Starting Shimmer setup wizard...\n");

    match setup::run_setup_wizard() {
        Ok(Some(setup_config)) => {
            let config_path = "shimmer.toml";
            setup::write_config_file(&setup_config, config_path)
                .expect("failed to write config file");
            println!("\nConfig written to {config_path}");

            // Initialize DB and create org + admin
            let config = config::ServerConfig::load();
            let db = db::Database::open(&config.database.path)
                .expect("failed to open database");

            let org_id = format!("org_{}", uuid::Uuid::new_v4().simple());

            // Create org
            db.create_org(&db::OrgRecord {
                id: org_id.clone(),
                name: setup_config.org_name.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
            }).expect("failed to create org");

            // Register admin
            let tokens = services::auth::register(
                &db,
                services::auth::RegisterInput {
                    email: setup_config.admin_email.clone(),
                    password: setup_config.admin_password,
                    org_id: org_id.clone(),
                    role: "admin".into(),
                    name: setup_config.admin_email.clone(),
                },
                &config.server.jwt_secret,
            ).expect("failed to create admin user");

            // Update config with org_id
            // (In practice: rewrite shimmer.toml with org.id set)

            println!("\nOrg '{}' created (ID: {org_id})", setup_config.org_name);
            println!("Admin user created: {}", setup_config.admin_email);
            println!("\nGenerate the org encryption key by opening the Shimmer desktop app.");
            println!("Run `shimmer-server serve` to start the server.");
        }
        Ok(None) => {
            println!("Setup cancelled.");
        }
        Err(e) => {
            eprintln!("Setup error: {e}");
        }
    }
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo build -p shimmer-server`
Expected: Compiles

- [ ] **Step 6: Commit**

```bash
git add shimmer-server/src/tui/ shimmer-server/src/main.rs shimmer-server/src/lib.rs shimmer-server/Cargo.toml Cargo.toml
git commit -m "feat: add Ratatui TUI setup wizard and config file generation"
```

---

## Chunk 5: Desktop App — Keychain Migration & Onboarding

### Task 9: Migrate Key Storage to macOS Keychain

**Files:**
- Modify: `src-tauri/src/key_store.rs`
- Modify: `src-tauri/Cargo.toml` (add security-framework)

- [ ] **Step 1: Add keychain dependency**

Add to `src-tauri/Cargo.toml`:
```toml
security-framework = "3"
```

- [ ] **Step 2: Update KeyStore to use keychain with file fallback**

Refactor `key_store.rs` to:
1. Try loading from macOS keychain (service: `com.shimmer.app`, account: `org-kek`)
2. If not in keychain but `key.bin` exists → migrate: read file, store in keychain, delete file
3. If `SHIMMER_DEV_KEY` is set → use that (dev override, skip keychain)
4. If nothing found → generate new, store in keychain

```rust
use security_framework::passwords::{get_generic_password, set_generic_password, delete_generic_password};
use zeroize::{Zeroize, ZeroizeOnDrop};
use shimmer_core::KEY_LEN;

const KEYCHAIN_SERVICE: &str = "com.shimmer.app";
const KEYCHAIN_ACCOUNT_KEK: &str = "org-kek";
const KEYCHAIN_ACCOUNT_JWT: &str = "jwt-access-token";
const KEYCHAIN_ACCOUNT_REFRESH: &str = "jwt-refresh-token";

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct KeyStore {
    key: [u8; KEY_LEN],
}

impl KeyStore {
    pub fn load_or_create(app_data_dir: &std::path::Path) -> Self {
        // Dev override
        if let Ok(hex_key) = std::env::var("SHIMMER_DEV_KEY") {
            let key = hex_to_key(&hex_key);
            return Self { key };
        }

        // Try keychain
        if let Ok(bytes) = get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_KEK) {
            if bytes.len() == KEY_LEN {
                let mut key = [0u8; KEY_LEN];
                key.copy_from_slice(&bytes);
                return Self { key };
            }
        }

        // Try migrating from file
        let key_file = app_data_dir.join("key.bin");
        if key_file.exists() {
            if let Ok(bytes) = std::fs::read(&key_file) {
                if bytes.len() == KEY_LEN {
                    let mut key = [0u8; KEY_LEN];
                    key.copy_from_slice(&bytes);
                    // Migrate to keychain
                    let _ = set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_KEK, &bytes);
                    // Delete old file
                    let _ = std::fs::remove_file(&key_file);
                    tracing::info!("migrated KEK from file to keychain");
                    return Self { key };
                }
            }
        }

        // Generate new
        let key = shimmer_core::encryption::generate_key();
        let _ = set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_KEK, &key);
        tracing::info!("generated new KEK and stored in keychain");
        Self { key }
    }

    pub fn key(&self) -> &[u8; KEY_LEN] {
        &self.key
    }

    /// Store KEK received from an invite link
    pub fn set_key(&mut self, key: [u8; KEY_LEN]) {
        self.key = key;
        let _ = set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_KEK, &key);
    }

    /// Store auth tokens in keychain
    pub fn store_tokens(access: &str, refresh: &str) {
        let _ = set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_JWT, access.as_bytes());
        let _ = set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_REFRESH, refresh.as_bytes());
    }

    /// Load auth tokens from keychain
    pub fn load_tokens() -> Option<(String, String)> {
        let access = get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_JWT).ok()?;
        let refresh = get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_REFRESH).ok()?;
        Some((
            String::from_utf8(access.to_vec()).ok()?,
            String::from_utf8(refresh.to_vec()).ok()?,
        ))
    }

    /// Clear all stored credentials (logout)
    pub fn clear_all() {
        let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_KEK);
        let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_JWT);
        let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_REFRESH);
    }
}

fn hex_to_key(hex: &str) -> [u8; KEY_LEN] {
    let bytes = hex::decode(hex).expect("SHIMMER_DEV_KEY must be valid hex");
    assert_eq!(bytes.len(), KEY_LEN, "SHIMMER_DEV_KEY must be 64 hex chars (32 bytes)");
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes);
    key
}
```

- [ ] **Step 3: Update lib.rs to pass app_data_dir to KeyStore**

In `src-tauri/src/lib.rs`, update the `KeyStore::load_or_create` call to pass the Tauri app data dir.

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p shimmer-app` (or whatever the tauri crate is named)
Expected: Compiles

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/key_store.rs src-tauri/src/lib.rs src-tauri/Cargo.toml
git commit -m "feat: migrate key storage from file to macOS keychain with auto-migration"
```

---

### Task 10: Add Auth Commands to Tauri Client

**Files:**
- Modify: `src-tauri/src/client.rs` (add auth methods)
- Modify: `src-tauri/src/lib.rs` (add login/register/logout commands)

- [ ] **Step 1: Add auth methods to ShimmerClient**

Add to `src-tauri/src/client.rs`:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

impl ShimmerClient {
    pub async fn register(
        &self, invite_token: &str, email: &str, password: &str, name: &str,
    ) -> Result<AuthResponse, reqwest::Error> {
        self.http.post(format!("{}/api/auth/register", self.base_url))
            .json(&serde_json::json!({
                "inviteToken": invite_token,
                "email": email,
                "password": password,
                "name": name,
            }))
            .send().await?
            .error_for_status()?
            .json().await
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<AuthResponse, reqwest::Error> {
        self.http.post(format!("{}/api/auth/login", self.base_url))
            .json(&serde_json::json!({ "email": email, "password": password }))
            .send().await?
            .error_for_status()?
            .json().await
    }

    pub async fn refresh_token(&self, refresh: &str) -> Result<AuthResponse, reqwest::Error> {
        self.http.post(format!("{}/api/auth/refresh", self.base_url))
            .json(&serde_json::json!({ "refreshToken": refresh }))
            .send().await?
            .error_for_status()?
            .json().await
    }
}
```

- [ ] **Step 2: Add Tauri commands for auth**

Add to `src-tauri/src/lib.rs`:

```rust
#[tauri::command]
async fn auth_register(
    invite_token: String,
    email: String,
    password: String,
    name: String,
    kek_fragment: Option<String>, // encrypted KEK from URL fragment
    client: State<'_, Arc<tokio::sync::RwLock<ShimmerClient>>>,
    key_store: State<'_, Arc<tokio::sync::RwLock<KeyStore>>>,
) -> Result<serde_json::Value, CommandError> {
    let client_read = client.read().await;
    let auth = client_read.register(&invite_token, &email, &password, &name).await
        .map_err(|e| CommandError::Internal(e.to_string()))?;

    // Update client with new token
    drop(client_read);
    client.write().await.set_token(&auth.access_token);

    // Decrypt and store KEK from invite fragment
    if let Some(fragment) = kek_fragment {
        let kek = shimmer_core::encryption::unwrap_kek_from_invite(&fragment, &invite_token)
            .map_err(|e| CommandError::Encryption(e.to_string()))?;
        key_store.write().await.set_key(kek);
    }

    // Store tokens in keychain
    KeyStore::store_tokens(&auth.access_token, &auth.refresh_token);

    Ok(serde_json::json!({
        "userId": auth.user_id,
        "success": true,
    }))
}

#[tauri::command]
async fn auth_login(
    email: String,
    password: String,
    client: State<'_, Arc<tokio::sync::RwLock<ShimmerClient>>>,
) -> Result<serde_json::Value, CommandError> {
    let client_read = client.read().await;
    let auth = client_read.login(&email, &password).await
        .map_err(|e| CommandError::Internal(e.to_string()))?;

    drop(client_read);
    client.write().await.set_token(&auth.access_token);
    KeyStore::store_tokens(&auth.access_token, &auth.refresh_token);

    Ok(serde_json::json!({ "userId": auth.user_id, "success": true }))
}

#[tauri::command]
async fn auth_logout(
    key_store: State<'_, Arc<tokio::sync::RwLock<KeyStore>>>,
) -> Result<(), CommandError> {
    KeyStore::clear_all();
    Ok(())
}

#[tauri::command]
async fn auth_status() -> Result<serde_json::Value, CommandError> {
    let has_tokens = KeyStore::load_tokens().is_some();
    Ok(serde_json::json!({ "authenticated": has_tokens }))
}
```

Register these commands in the Tauri builder's `invoke_handler`.

Note: `ShimmerClient` needs to be wrapped in `Arc<RwLock<>>` instead of `Arc<>` so the token can be updated. This requires updating how it's managed in the Tauri setup.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p shimmer-app`
Expected: Compiles (with warnings about unused — that's fine)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/client.rs src-tauri/src/lib.rs
git commit -m "feat: add auth commands to Tauri client (register, login, logout)"
```

---

### Task 11: First-Launch Onboarding UI

**Files:**
- Modify: `src/routes/+page.svelte`

- [ ] **Step 1: Add auth state and onboarding view**

Add to the top of `+page.svelte`, before the main tab UI:

```svelte
<script lang="ts">
  // Add to existing script
  let authState: 'checking' | 'unauthenticated' | 'authenticated' = 'checking';
  let onboardingStep: 'welcome' | 'set-password' | 'done' = 'welcome';
  let inviteUrl = '';
  let loginEmail = '';
  let loginPassword = '';
  let registerName = '';
  let registerPassword = '';
  let authError = '';
  let serverUrl = '';

  // Check auth status on mount
  import { onMount } from 'svelte';

  onMount(async () => {
    try {
      const status = await invoke<{ authenticated: boolean }>('auth_status');
      authState = status.authenticated ? 'authenticated' : 'unauthenticated';
    } catch {
      authState = 'unauthenticated';
    }
  });

  // Parse phi://join/<token>#<kek> from invite URL
  function parseInviteUrl(url: string): { token: string; kekFragment: string | null } | null {
    const match = url.match(/^phi:\/\/join\/([^#]+)(?:#(.+))?$/);
    if (!match) return null;
    return { token: match[1], kekFragment: match[2] || null };
  }

  async function handleInviteSubmit() {
    authError = '';
    const parsed = parseInviteUrl(inviteUrl);
    if (!parsed) {
      authError = 'Invalid invite link. Expected format: phi://join/<token>#<key>';
      return;
    }
    // Move to password step, store parsed data
    onboardingStep = 'set-password';
  }

  async function handleRegister() {
    authError = '';
    const parsed = parseInviteUrl(inviteUrl);
    if (!parsed) return;

    try {
      await invoke('auth_register', {
        inviteToken: parsed.token,
        email: loginEmail,
        password: registerPassword,
        name: registerName,
        kekFragment: parsed.kekFragment,
      });
      authState = 'authenticated';
    } catch (e: any) {
      authError = e.toString();
    }
  }

  async function handleLogin() {
    authError = '';
    try {
      await invoke('auth_login', { email: loginEmail, password: loginPassword });
      authState = 'authenticated';
    } catch (e: any) {
      authError = 'Invalid email or password';
    }
  }

  async function handleLogout() {
    await invoke('auth_logout');
    authState = 'unauthenticated';
    onboardingStep = 'welcome';
  }
</script>
```

- [ ] **Step 2: Add onboarding HTML**

Before the existing main tab UI, add a conditional block:

```svelte
{#if authState === 'checking'}
  <div class="flex items-center justify-center h-screen">
    <p class="text-gray-400">Loading...</p>
  </div>
{:else if authState === 'unauthenticated'}
  <div class="flex flex-col items-center justify-center h-screen p-8 gap-6">
    <h1 class="text-2xl font-bold">Welcome to Shimmer</h1>

    {#if onboardingStep === 'welcome'}
      <div class="w-full max-w-md space-y-4">
        <div>
          <label class="block text-sm mb-1">Paste an invite link</label>
          <input
            type="text"
            bind:value={inviteUrl}
            placeholder="phi://join/..."
            class="w-full p-2 rounded bg-gray-800 border border-gray-600"
          />
        </div>
        <button onclick={handleInviteSubmit} class="w-full p-2 rounded bg-blue-600 hover:bg-blue-500">
          Continue with Invite
        </button>

        <div class="text-center text-gray-500 text-sm">— or sign in —</div>

        <div class="space-y-2">
          <input type="email" bind:value={loginEmail} placeholder="Email" class="w-full p-2 rounded bg-gray-800 border border-gray-600" />
          <input type="password" bind:value={loginPassword} placeholder="Password" class="w-full p-2 rounded bg-gray-800 border border-gray-600" />
          <button onclick={handleLogin} class="w-full p-2 rounded bg-gray-700 hover:bg-gray-600">Sign In</button>
        </div>
      </div>

    {:else if onboardingStep === 'set-password'}
      <div class="w-full max-w-md space-y-4">
        <p class="text-sm text-gray-400">Set up your account</p>
        <input type="text" bind:value={registerName} placeholder="Your name" class="w-full p-2 rounded bg-gray-800 border border-gray-600" />
        <input type="email" bind:value={loginEmail} placeholder="Email" class="w-full p-2 rounded bg-gray-800 border border-gray-600" />
        <input type="password" bind:value={registerPassword} placeholder="Choose a password (8+ chars)" class="w-full p-2 rounded bg-gray-800 border border-gray-600" />
        <button onclick={handleRegister} class="w-full p-2 rounded bg-blue-600 hover:bg-blue-500">Create Account</button>
        <button onclick={() => onboardingStep = 'welcome'} class="w-full p-2 rounded bg-gray-700">Back</button>
      </div>
    {/if}

    {#if authError}
      <p class="text-red-400 text-sm">{authError}</p>
    {/if}
  </div>
{:else}
  <!-- Existing tab UI goes here (unchanged) -->
```

Also add a logout button somewhere in the Settings tab.

- [ ] **Step 3: Handle deep link for phi://join/...**

In `src-tauri/src/lib.rs`, add a deep link handler that emits an event to the frontend when a `phi://join/` URL is received:

```rust
// In the Tauri setup, after deep_link plugin is registered:
app.listen("deep-link://new-url", |event| {
    // Parse the URL and emit to frontend
    if let Some(url) = event.payload().as_str() {
        if url.starts_with("phi://join/") {
            // Emit to frontend
            app.emit("invite-link-received", url).ok();
        }
    }
});
```

In the Svelte frontend, listen for this event:
```typescript
import { listen } from '@tauri-apps/api/event';

onMount(async () => {
  await listen('invite-link-received', (event) => {
    inviteUrl = event.payload as string;
    onboardingStep = 'welcome'; // or auto-advance
  });
});
```

- [ ] **Step 4: Verify frontend compiles**

Run: `npm run check` (TypeScript check)
Expected: No type errors

- [ ] **Step 5: Commit**

```bash
git add src/routes/+page.svelte src-tauri/src/lib.rs
git commit -m "feat: add first-launch onboarding UI with invite link + login"
```

---

## Chunk 6: Distribution & Auto-Update

### Task 12: Configure Tauri Auto-Update

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `package.json`

- [ ] **Step 1: Add updater plugin**

Add to `src-tauri/Cargo.toml`:
```toml
tauri-plugin-updater = "2"
```

- [ ] **Step 2: Configure updater in tauri.conf.json**

Add to the `plugins` section of `tauri.conf.json`:
```json
"updater": {
  "endpoints": [
    "https://github.com/YOUR_ORG/shimmer/releases/latest/download/latest.json"
  ],
  "pubkey": "YOUR_PUBLIC_KEY_HERE"
}
```

Note: The actual public key is generated during the first `tauri build` with signing enabled. This is a placeholder.

- [ ] **Step 3: Register updater plugin in lib.rs**

Add to the Tauri builder:
```rust
.plugin(tauri_plugin_updater::Builder::new().build())
```

Add auto-update check on startup:
```rust
// In setup closure:
let handle = app.handle().clone();
tauri::async_runtime::spawn(async move {
    match tauri_plugin_updater::UpdaterExt::updater(&handle).check().await {
        Ok(Some(update)) => {
            tracing::info!(version = %update.version, "update available");
            // Could emit event to frontend to show update prompt
        }
        Ok(None) => tracing::debug!("no update available"),
        Err(e) => tracing::warn!(error = %e, "update check failed"),
    }
});
```

- [ ] **Step 4: Add build script for .dmg**

Add to `justfile`:
```makefile
# Build .dmg for distribution (requires Apple Developer certificate)
build-dmg:
    npm run tauri build

# Generate updater keys (one-time)
updater-keys:
    npm run tauri signer generate -- -w ~/.tauri/shimmer.key
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/src/lib.rs justfile
git commit -m "feat: add Tauri auto-update plugin and .dmg build config"
```

---

### Task 13: Add GitHub Actions CI for Release Builds

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create release workflow**

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build-macos:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Install dependencies
        run: npm install

      - name: Import signing certificate
        env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          KEYCHAIN_PASSWORD: ${{ secrets.KEYCHAIN_PASSWORD }}
        run: |
          echo $APPLE_CERTIFICATE | base64 --decode > certificate.p12
          security create-keychain -p "$KEYCHAIN_PASSWORD" build.keychain
          security default-keychain -s build.keychain
          security unlock-keychain -p "$KEYCHAIN_PASSWORD" build.keychain
          security import certificate.p12 -k build.keychain -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
          security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" build.keychain

      - name: Build Tauri app
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
        run: npm run tauri build

      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: macos-dmg
          path: |
            src-tauri/target/release/bundle/dmg/*.dmg
            src-tauri/target/release/bundle/macos/*.app.tar.gz
            src-tauri/target/release/bundle/macos/*.app.tar.gz.sig

  create-release:
    needs: build-macos
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/download-artifact@v4

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            macos-dmg/*.dmg
            macos-dmg/*.app.tar.gz
            macos-dmg/*.app.tar.gz.sig
          generate_release_notes: true
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add GitHub Actions release workflow for macOS .dmg"
```

---

## Summary

| Chunk | Tasks | Description |
|-------|-------|-------------|
| 1 | 1-2 | Config migration + service layer extraction |
| 2 | 3-4 | Users/auth DB + auth service + auth API routes |
| 3 | 5-6 | KEK wrapping in shimmer-core + invite flow rework |
| 4 | 7-8 | CLI subcommands + TUI setup wizard |
| 5 | 9-11 | Keychain migration + auth commands + onboarding UI |
| 6 | 12-13 | Auto-update plugin + release CI |

**Dependencies:** Chunk 1 → Chunk 2 → Chunk 3 (sequential). Chunk 4 depends on Chunks 1-2. Chunk 5 depends on Chunks 2-3. Chunk 6 is independent.

**Parallelizable:** Chunk 4 and Chunk 6 can run in parallel once their dependencies are met. Within chunks, tasks are sequential.
