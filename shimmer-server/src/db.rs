//! `SQLite` metadata store.
//!
//! Ciphertext lives in blob storage (S3/file). This database holds:
//! - paste metadata (content type, visibility, TTL, etc.)
//! - blind index search tokens
//! - org membership and invites
//!
//! All methods are synchronous — callers should wrap in `spawn_blocking`.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Database error types.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("database lock poisoned")]
    Lock,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),
}

/// Metadata store backed by `SQLite`.
pub struct Database {
    conn: std::sync::Mutex<Connection>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// Paste metadata stored in `SQLite` (ciphertext is in blob storage).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteRecord {
    pub id: String,
    pub org_id: String,
    pub user_id: String,
    pub user_name: String,
    pub content_type: String,
    pub encrypted_title: Option<String>,
    pub encrypted_filename: Option<String>,
    pub visibility: String,
    pub size_bytes: i64,
    pub ttl_hours: Option<i64>,
    pub burn_on_read: bool,
    pub created_at: String,
    pub expires_at: Option<String>,
}

/// Organization record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgRecord {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// Org member record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberRecord {
    pub id: String,
    pub org_id: String,
    pub user_id: String,
    pub name: String,
    pub role: String,
    pub joined_at: String,
}

/// User record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRecord {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: String,
}

/// Refresh token record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenRecord {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub expires_at: String,
}

/// Invite record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteRecord {
    pub token: String,
    pub org_id: String,
    pub role: String,
    pub created_by: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub used_by: Option<String>,
    pub single_use: bool,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl Database {
    /// Open (or create) the database at `path`.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` if the file cannot be opened or migrations fail.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;

        // Performance: WAL mode for concurrent reads during async serving
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        let db = Self {
            conn: std::sync::Mutex::new(conn),
        };
        db.run_migrations()?;
        info!(?path, "database opened");
        Ok(db)
    }

    /// Open an in-memory database (for tests and integration tests).
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` if the database cannot be created.
    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self {
            conn: std::sync::Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, DbError> {
        self.conn.lock().map_err(|_| DbError::Lock)
    }

    fn run_migrations(&self) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS orgs (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS members (
                id        TEXT PRIMARY KEY,
                org_id    TEXT NOT NULL REFERENCES orgs(id),
                user_id   TEXT NOT NULL,
                name      TEXT NOT NULL,
                role      TEXT NOT NULL DEFAULT 'member'
                              CHECK (role IN ('admin', 'member', 'read_only')),
                joined_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(org_id, user_id)
            );

            CREATE TABLE IF NOT EXISTS invites (
                token      TEXT PRIMARY KEY,
                org_id     TEXT NOT NULL REFERENCES orgs(id),
                role       TEXT NOT NULL DEFAULT 'member',
                created_by TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                used_at    TEXT,
                used_by    TEXT,
                single_use INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS pastes (
                id                 TEXT PRIMARY KEY,
                org_id             TEXT NOT NULL,
                user_id            TEXT NOT NULL,
                user_name          TEXT NOT NULL DEFAULT '',
                content_type       TEXT NOT NULL DEFAULT 'text/plain',
                encrypted_title    TEXT,
                encrypted_filename TEXT,
                visibility         TEXT NOT NULL DEFAULT 'org'
                                       CHECK (visibility IN ('private', 'org', 'link')),
                size_bytes         INTEGER NOT NULL,
                ttl_hours          INTEGER,
                burn_on_read       INTEGER NOT NULL DEFAULT 0,
                created_at         TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at         TEXT
            );

            CREATE TABLE IF NOT EXISTS search_tokens (
                paste_id   TEXT NOT NULL REFERENCES pastes(id) ON DELETE CASCADE,
                token      TEXT NOT NULL,
                token_type TEXT NOT NULL DEFAULT 'content'
                               CHECK (token_type IN ('content', 'title', 'tag', 'filename')),
                PRIMARY KEY (paste_id, token)
            );

            CREATE INDEX IF NOT EXISTS idx_search_tokens_token
                ON search_tokens(token);

            CREATE INDEX IF NOT EXISTS idx_pastes_org_id
                ON pastes(org_id);

            CREATE INDEX IF NOT EXISTS idx_pastes_user_id
                ON pastes(user_id);

            CREATE INDEX IF NOT EXISTS idx_members_org_user
                ON members(org_id, user_id);

            CREATE TABLE IF NOT EXISTS users (
                id            TEXT PRIMARY KEY,
                email         TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                created_at    TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS refresh_tokens (
                id         TEXT PRIMARY KEY,
                user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )?;
        Ok(())
    }

    // =======================================================================
    // Orgs
    // =======================================================================

    /// Create a new organization.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Conflict` if the org ID already exists.
    pub fn create_org(&self, org: &OrgRecord) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO orgs (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![org.id, org.name, org.created_at],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                DbError::Conflict(format!("org {} already exists", org.id))
            }
            other => DbError::Sqlite(other),
        })?;
        Ok(())
    }

    /// Get an org by ID.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn get_org(&self, id: &str) -> Result<Option<OrgRecord>, DbError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, name, created_at FROM orgs WHERE id = ?1",
                params![id],
                |row| {
                    Ok(OrgRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    // =======================================================================
    // Members
    // =======================================================================

    /// Add a member to an org.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Conflict` if the user is already a member.
    pub fn add_member(&self, member: &MemberRecord) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO members (id, org_id, user_id, name, role, joined_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                member.id,
                member.org_id,
                member.user_id,
                member.name,
                member.role,
                member.joined_at,
            ],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                DbError::Conflict(format!(
                    "user {} is already a member of org {}",
                    member.user_id, member.org_id
                ))
            }
            other => DbError::Sqlite(other),
        })?;
        Ok(())
    }

    /// Get a member by org + user ID.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn get_member(&self, org_id: &str, user_id: &str) -> Result<Option<MemberRecord>, DbError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, org_id, user_id, name, role, joined_at
                 FROM members WHERE org_id = ?1 AND user_id = ?2",
                params![org_id, user_id],
                |row| {
                    Ok(MemberRecord {
                        id: row.get(0)?,
                        org_id: row.get(1)?,
                        user_id: row.get(2)?,
                        name: row.get(3)?,
                        role: row.get(4)?,
                        joined_at: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// List all members of an org.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn list_members(&self, org_id: &str) -> Result<Vec<MemberRecord>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, org_id, user_id, name, role, joined_at
             FROM members WHERE org_id = ?1 ORDER BY joined_at",
        )?;
        let rows = stmt
            .query_map(params![org_id], |row| {
                Ok(MemberRecord {
                    id: row.get(0)?,
                    org_id: row.get(1)?,
                    user_id: row.get(2)?,
                    name: row.get(3)?,
                    role: row.get(4)?,
                    joined_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Update a member's role. Returns `true` if a row was updated.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn update_member_role(
        &self,
        org_id: &str,
        user_id: &str,
        role: &str,
    ) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let count = conn.execute(
            "UPDATE members SET role = ?1 WHERE org_id = ?2 AND user_id = ?3",
            params![role, org_id, user_id],
        )?;
        Ok(count > 0)
    }

    /// Remove a member from an org. Returns `true` if a row was deleted.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn remove_member(&self, org_id: &str, user_id: &str) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let count = conn.execute(
            "DELETE FROM members WHERE org_id = ?1 AND user_id = ?2",
            params![org_id, user_id],
        )?;
        Ok(count > 0)
    }

    // =======================================================================
    // Users
    // =======================================================================

    /// Create a new user.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Conflict` if the email is already registered.
    pub fn create_user(&self, id: &str, email: &str, password_hash: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO users (id, email, password_hash) VALUES (?1, ?2, ?3)",
            params![id, email, password_hash],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                DbError::Conflict(format!("email {email} already registered"))
            }
            other => DbError::Sqlite(other),
        })?;
        Ok(())
    }

    /// Get a user by email.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DbError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, email, password_hash, created_at FROM users WHERE email = ?1",
                params![email],
                |row| {
                    Ok(UserRecord {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        password_hash: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Get a user by ID.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn get_user_by_id(&self, id: &str) -> Result<Option<UserRecord>, DbError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, email, password_hash, created_at FROM users WHERE id = ?1",
                params![id],
                |row| {
                    Ok(UserRecord {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        password_hash: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Get the first member record for a user (any org).
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn get_member_by_user_id(&self, user_id: &str) -> Result<Option<MemberRecord>, DbError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, org_id, user_id, name, role, joined_at
                 FROM members WHERE user_id = ?1 LIMIT 1",
                params![user_id],
                |row| {
                    Ok(MemberRecord {
                        id: row.get(0)?,
                        org_id: row.get(1)?,
                        user_id: row.get(2)?,
                        name: row.get(3)?,
                        role: row.get(4)?,
                        joined_at: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    // =======================================================================
    // Refresh tokens
    // =======================================================================

    /// Store a refresh token hash.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn store_refresh_token(
        &self,
        id: &str,
        user_id: &str,
        token_hash: &str,
        expires_at: &str,
    ) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO refresh_tokens (id, user_id, token_hash, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, user_id, token_hash, expires_at],
        )?;
        Ok(())
    }

    /// Look up a refresh token by its hash.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRecord>, DbError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, user_id, token_hash, expires_at
                 FROM refresh_tokens WHERE token_hash = ?1",
                params![token_hash],
                |row| {
                    Ok(RefreshTokenRecord {
                        id: row.get(0)?,
                        user_id: row.get(1)?,
                        token_hash: row.get(2)?,
                        expires_at: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Delete a single refresh token by ID.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn delete_refresh_token(&self, id: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM refresh_tokens WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete all refresh tokens for a user.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn delete_refresh_tokens_for_user(&self, user_id: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM refresh_tokens WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    }

    // =======================================================================
    // Invites
    // =======================================================================

    /// Create an invite.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn create_invite(&self, invite: &InviteRecord) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO invites (token, org_id, role, created_by, expires_at, single_use)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                invite.token,
                invite.org_id,
                invite.role,
                invite.created_by,
                invite.expires_at,
                invite.single_use,
            ],
        )?;
        Ok(())
    }

    /// Consume an invite token. Returns the invite if valid and not yet used.
    ///
    /// For single-use invites, marks it as used. For multi-use invites, leaves it.
    ///
    /// # Errors
    ///
    /// Returns `DbError::NotFound` if the token doesn't exist or is expired/used.
    pub fn consume_invite(&self, token: &str, user_id: &str) -> Result<InviteRecord, DbError> {
        let conn = self.conn()?;

        let invite = conn
            .query_row(
                "SELECT token, org_id, role, created_by, expires_at, used_at, used_by, single_use
                 FROM invites
                 WHERE token = ?1
                   AND expires_at > datetime('now')
                   AND (used_at IS NULL OR single_use = 0)",
                params![token],
                |row| {
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
                },
            )
            .optional()?
            .ok_or_else(|| {
                DbError::NotFound("invite not found, expired, or already used".into())
            })?;

        // Mark as used
        if invite.single_use {
            conn.execute(
                "UPDATE invites SET used_at = datetime('now'), used_by = ?1 WHERE token = ?2",
                params![user_id, token],
            )?;
        }

        Ok(invite)
    }

    // =======================================================================
    // Pastes
    // =======================================================================

    /// Insert paste metadata + search tokens in a single transaction.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn insert_paste(
        &self,
        paste: &PasteRecord,
        search_tokens: &[(String, String)], // (token, token_type)
    ) -> Result<(), DbError> {
        let conn = self.conn()?;
        let tx = conn.unchecked_transaction()?;

        tx.execute(
            "INSERT INTO pastes (
                id, org_id, user_id, user_name, content_type,
                encrypted_title, encrypted_filename,
                visibility, size_bytes, ttl_hours, burn_on_read,
                created_at, expires_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                paste.id,
                paste.org_id,
                paste.user_id,
                paste.user_name,
                paste.content_type,
                paste.encrypted_title,
                paste.encrypted_filename,
                paste.visibility,
                paste.size_bytes,
                paste.ttl_hours,
                paste.burn_on_read,
                paste.created_at,
                paste.expires_at,
            ],
        )?;

        if !search_tokens.is_empty() {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO search_tokens (paste_id, token, token_type)
                 VALUES (?1, ?2, ?3)",
            )?;
            for (token, token_type) in search_tokens {
                stmt.execute(params![paste.id, token, token_type])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Get paste metadata by ID.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn get_paste(&self, id: &str) -> Result<Option<PasteRecord>, DbError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, org_id, user_id, user_name, content_type,
                        encrypted_title, encrypted_filename,
                        visibility, size_bytes, ttl_hours, burn_on_read,
                        created_at, expires_at
                 FROM pastes WHERE id = ?1",
                params![id],
                Self::row_to_paste,
            )
            .optional()?;
        Ok(row)
    }

    /// List pastes visible to a user in their org.
    ///
    /// Returns pastes where:
    /// - visibility = 'org' and `paste.org_id` matches, OR
    /// - visibility = 'private' and `paste.user_id` matches, OR
    /// - visibility = 'link' (anyone)
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn list_pastes(
        &self,
        org_id: &str,
        user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PasteRecord>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, org_id, user_id, user_name, content_type,
                    encrypted_title, encrypted_filename,
                    visibility, size_bytes, ttl_hours, burn_on_read,
                    created_at, expires_at
             FROM pastes
             WHERE (org_id = ?1 AND visibility = 'org')
                OR (user_id = ?2 AND visibility = 'private')
                OR visibility = 'link'
             ORDER BY created_at DESC
             LIMIT ?3 OFFSET ?4",
        )?;
        let rows = stmt
            .query_map(params![org_id, user_id, limit, offset], Self::row_to_paste)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Search pastes by blind index tokens.
    ///
    /// Returns pastes that match ALL provided tokens (AND semantics),
    /// filtered by visibility rules.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn search_pastes(
        &self,
        org_id: &str,
        user_id: &str,
        tokens: &[String],
    ) -> Result<Vec<PasteRecord>, DbError> {
        if tokens.is_empty() {
            return self.list_pastes(org_id, user_id, 50, 0);
        }

        let conn = self.conn()?;

        // Build parameterized query for N tokens with AND semantics
        let placeholders: Vec<String> = (0..tokens.len()).map(|i| format!("?{}", i + 3)).collect();
        let token_count = tokens.len();

        let sql = format!(
            "SELECT p.id, p.org_id, p.user_id, p.user_name, p.content_type,
                    p.encrypted_title, p.encrypted_filename,
                    p.visibility, p.size_bytes, p.ttl_hours, p.burn_on_read,
                    p.created_at, p.expires_at
             FROM pastes p
             INNER JOIN search_tokens st ON st.paste_id = p.id
             WHERE st.token IN ({placeholders})
               AND (
                   (p.org_id = ?1 AND p.visibility = 'org')
                   OR (p.user_id = ?2 AND p.visibility = 'private')
                   OR p.visibility = 'link'
               )
             GROUP BY p.id
             HAVING COUNT(DISTINCT st.token) = ?{having_param}
             ORDER BY p.created_at DESC
             LIMIT 50",
            placeholders = placeholders.join(", "),
            having_param = tokens.len() + 3,
        );

        let mut stmt = conn.prepare(&sql)?;

        // Bind parameters: ?1=org_id, ?2=user_id, ?3..?N=tokens, ?N+1=token_count
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(org_id.to_string()));
        param_values.push(Box::new(user_id.to_string()));
        for t in tokens {
            param_values.push(Box::new(t.clone()));
        }
        param_values.push(Box::new(token_count as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), Self::row_to_paste)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete paste metadata + cascades to search tokens. Returns `true` if deleted.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn delete_paste(&self, id: &str) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let count = conn.execute("DELETE FROM pastes WHERE id = ?1", params![id])?;
        Ok(count > 0)
    }

    /// Mark a paste as burned (set `burn_on_read` consumed).
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlite` on database errors.
    pub fn mark_burned(&self, id: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM pastes WHERE id = ?1", params![id])?;
        Ok(())
    }

    // Helper to map a row to PasteRecord
    fn row_to_paste(row: &rusqlite::Row<'_>) -> Result<PasteRecord, rusqlite::Error> {
        Ok(PasteRecord {
            id: row.get(0)?,
            org_id: row.get(1)?,
            user_id: row.get(2)?,
            user_name: row.get(3)?,
            content_type: row.get(4)?,
            encrypted_title: row.get(5)?,
            encrypted_filename: row.get(6)?,
            visibility: row.get(7)?,
            size_bytes: row.get(8)?,
            ttl_hours: row.get(9)?,
            burn_on_read: row.get(10)?,
            created_at: row.get(11)?,
            expires_at: row.get(12)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn test_org() -> OrgRecord {
        OrgRecord {
            id: "org_test".into(),
            name: "Test Org".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn create_and_get_org() {
        let db = test_db();
        let org = test_org();
        db.create_org(&org).unwrap();
        let fetched = db.get_org("org_test").unwrap().unwrap();
        assert_eq!(fetched.name, "Test Org");
    }

    #[test]
    fn duplicate_org_fails() {
        let db = test_db();
        let org = test_org();
        db.create_org(&org).unwrap();
        assert!(db.create_org(&org).is_err());
    }

    #[test]
    fn member_lifecycle() {
        let db = test_db();
        db.create_org(&test_org()).unwrap();

        let member = MemberRecord {
            id: "m_1".into(),
            org_id: "org_test".into(),
            user_id: "u_alice".into(),
            name: "Alice".into(),
            role: "admin".into(),
            joined_at: chrono::Utc::now().to_rfc3339(),
        };
        db.add_member(&member).unwrap();

        let fetched = db.get_member("org_test", "u_alice").unwrap().unwrap();
        assert_eq!(fetched.role, "admin");

        db.update_member_role("org_test", "u_alice", "member")
            .unwrap();
        let updated = db.get_member("org_test", "u_alice").unwrap().unwrap();
        assert_eq!(updated.role, "member");

        let members = db.list_members("org_test").unwrap();
        assert_eq!(members.len(), 1);

        db.remove_member("org_test", "u_alice").unwrap();
        assert!(db.get_member("org_test", "u_alice").unwrap().is_none());
    }

    #[test]
    fn paste_insert_and_search() {
        let db = test_db();
        db.create_org(&test_org()).unwrap();

        let paste = PasteRecord {
            id: "paste_1".into(),
            org_id: "org_test".into(),
            user_id: "u_alice".into(),
            user_name: "Alice".into(),
            content_type: "text/plain".into(),
            encrypted_title: None,
            encrypted_filename: None,
            visibility: "org".into(),
            size_bytes: 100,
            ttl_hours: None,
            burn_on_read: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
        };

        let tokens = vec![
            ("token_abc".to_string(), "content".to_string()),
            ("token_def".to_string(), "content".to_string()),
        ];

        db.insert_paste(&paste, &tokens).unwrap();

        // Get by ID
        let fetched = db.get_paste("paste_1").unwrap().unwrap();
        assert_eq!(fetched.user_id, "u_alice");

        // Search by token — should find it
        let results = db
            .search_pastes("org_test", "u_alice", &["token_abc".into()])
            .unwrap();
        assert_eq!(results.len(), 1);

        // Search by wrong token — should not find it
        let results = db
            .search_pastes("org_test", "u_alice", &["token_zzz".into()])
            .unwrap();
        assert!(results.is_empty());

        // Search by multiple tokens (AND) — should find it
        let results = db
            .search_pastes(
                "org_test",
                "u_alice",
                &["token_abc".into(), "token_def".into()],
            )
            .unwrap();
        assert_eq!(results.len(), 1);

        // Delete
        assert!(db.delete_paste("paste_1").unwrap());
        assert!(db.get_paste("paste_1").unwrap().is_none());
    }

    #[test]
    fn visibility_filtering() {
        let db = test_db();
        db.create_org(&test_org()).unwrap();

        // Alice's private paste
        let private_paste = PasteRecord {
            id: "p_private".into(),
            org_id: "org_test".into(),
            user_id: "u_alice".into(),
            user_name: "Alice".into(),
            content_type: "text/plain".into(),
            encrypted_title: None,
            encrypted_filename: None,
            visibility: "private".into(),
            size_bytes: 50,
            ttl_hours: None,
            burn_on_read: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
        };
        db.insert_paste(&private_paste, &[]).unwrap();

        // Alice's org paste
        let org_paste = PasteRecord {
            id: "p_org".into(),
            visibility: "org".into(),
            ..private_paste.clone()
        };
        db.insert_paste(&org_paste, &[]).unwrap();

        // Alice sees both (her private + org)
        let alice_view = db.list_pastes("org_test", "u_alice", 50, 0).unwrap();
        assert_eq!(alice_view.len(), 2);

        // Bob sees only the org paste (not Alice's private)
        let bob_view = db.list_pastes("org_test", "u_bob", 50, 0).unwrap();
        assert_eq!(bob_view.len(), 1);
        assert_eq!(bob_view[0].id, "p_org");
    }

    #[test]
    fn create_and_get_user() {
        let db = test_db();
        db.create_user("u_1", "alice@example.com", "hashed_pw")
            .unwrap();
        let user = db.get_user_by_email("alice@example.com").unwrap().unwrap();
        assert_eq!(user.id, "u_1");
        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.password_hash, "hashed_pw");
        assert!(!user.created_at.is_empty());
    }

    #[test]
    fn get_user_by_email_not_found() {
        let db = test_db();
        let result = db.get_user_by_email("nobody@example.com").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn refresh_token_lifecycle() {
        let db = test_db();
        db.create_user("u_1", "alice@example.com", "hashed_pw")
            .unwrap();

        let expires = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();
        db.store_refresh_token("rt_1", "u_1", "hash_abc", &expires)
            .unwrap();

        // Retrieve by hash
        let token = db.get_refresh_token_by_hash("hash_abc").unwrap().unwrap();
        assert_eq!(token.id, "rt_1");
        assert_eq!(token.user_id, "u_1");

        // Delete it
        db.delete_refresh_token("rt_1").unwrap();
        assert!(db.get_refresh_token_by_hash("hash_abc").unwrap().is_none());

        // Store again and delete by user
        db.store_refresh_token("rt_2", "u_1", "hash_def", &expires)
            .unwrap();
        db.delete_refresh_tokens_for_user("u_1").unwrap();
        assert!(db.get_refresh_token_by_hash("hash_def").unwrap().is_none());
    }
}
