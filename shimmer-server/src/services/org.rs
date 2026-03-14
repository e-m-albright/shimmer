//! Org service — business logic for organization and member management.

use std::sync::Arc;

use tracing::info;

use crate::db::{DbError, MemberRecord, OrgRecord};
use crate::AppState;

/// Errors that can occur in org service operations.
#[derive(Debug, thiserror::Error)]
pub enum OrgServiceError {
    #[error("database error: {0}")]
    Db(DbError),

    #[error("not found")]
    NotFound,

    #[error("forbidden")]
    Forbidden,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal: {0}")]
    Internal(String),
}

impl From<DbError> for OrgServiceError {
    fn from(e: DbError) -> Self {
        Self::Db(e)
    }
}

impl From<tokio::task::JoinError> for OrgServiceError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::Internal(e.to_string())
    }
}

/// Output from creating an org.
#[derive(Debug)]
pub struct CreateOrgOutput {
    pub org_id: String,
}

/// Caller identity needed by org operations.
#[derive(Debug)]
pub struct OrgCaller {
    pub sub: String,
    pub name: String,
    pub org: String,
    pub role: String,
}

impl OrgCaller {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// Create a new organization. The caller becomes the first admin.
///
/// # Errors
///
/// Returns `OrgServiceError` on database failures.
pub async fn create_org(
    state: Arc<AppState>,
    caller: &OrgCaller,
    org_name: &str,
) -> Result<CreateOrgOutput, OrgServiceError> {
    let org_id = format!("org_{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    let org = OrgRecord {
        id: org_id.clone(),
        name: org_name.to_string(),
        created_at: now.clone(),
    };

    let member = MemberRecord {
        id: format!("m_{}", uuid::Uuid::new_v4()),
        org_id: org_id.clone(),
        user_id: caller.sub.clone(),
        name: caller.name.clone(),
        role: "admin".into(),
        joined_at: now,
    };

    let db_state = state.clone();
    tokio::task::spawn_blocking(move || {
        db_state.db.create_org(&org)?;
        db_state.db.add_member(&member)?;
        Ok::<(), DbError>(())
    })
    .await??;

    info!(org_id = %org_id, user_id = %caller.sub, "org created");

    Ok(CreateOrgOutput { org_id })
}

/// List all members of an org.
///
/// # Errors
///
/// Returns `OrgServiceError` on database failures.
pub async fn list_members(
    state: Arc<AppState>,
    org_id: &str,
) -> Result<Vec<MemberRecord>, OrgServiceError> {
    let db_state = state.clone();
    let org = org_id.to_string();
    let members = tokio::task::spawn_blocking(move || db_state.db.list_members(&org)).await??;
    Ok(members)
}

/// Valid roles for members.
const VALID_ROLES: &[&str] = &["admin", "member", "read_only"];

/// Update a member's role. Admin only.
///
/// # Errors
///
/// Returns `OrgServiceError` on permission, validation, or database failures.
pub async fn update_role(
    state: Arc<AppState>,
    caller: &OrgCaller,
    target_user_id: &str,
    new_role: &str,
) -> Result<(), OrgServiceError> {
    if !caller.is_admin() {
        return Err(OrgServiceError::Forbidden);
    }

    if !VALID_ROLES.contains(&new_role) {
        return Err(OrgServiceError::BadRequest(format!(
            "invalid role '{}', must be one of: {}",
            new_role,
            VALID_ROLES.join(", ")
        )));
    }

    // Don't allow demoting yourself (prevents org lockout)
    if target_user_id == caller.sub {
        return Err(OrgServiceError::BadRequest(
            "cannot change your own role".into(),
        ));
    }

    let org_id = caller.org.clone();
    let db_state = state.clone();
    let role = new_role.to_string();
    let target_id = target_user_id.to_string();
    let updated = tokio::task::spawn_blocking(move || {
        db_state.db.update_member_role(&org_id, &target_id, &role)
    })
    .await??;

    if !updated {
        return Err(OrgServiceError::NotFound);
    }

    info!(
        user_id = %target_user_id,
        new_role = %new_role,
        admin = %caller.sub,
        "member role updated"
    );
    Ok(())
}

/// Remove a member from the org. Admin only.
///
/// # Errors
///
/// Returns `OrgServiceError` on permission or database failures.
pub async fn remove_member(
    state: Arc<AppState>,
    caller: &OrgCaller,
    target_user_id: &str,
) -> Result<(), OrgServiceError> {
    if !caller.is_admin() {
        return Err(OrgServiceError::Forbidden);
    }

    if target_user_id == caller.sub {
        return Err(OrgServiceError::BadRequest(
            "cannot remove yourself from the org".into(),
        ));
    }

    let org_id = caller.org.clone();
    let db_state = state.clone();
    let target_id = target_user_id.to_string();
    let removed =
        tokio::task::spawn_blocking(move || db_state.db.remove_member(&org_id, &target_id))
            .await??;

    if !removed {
        return Err(OrgServiceError::NotFound);
    }

    info!(user_id = %target_user_id, admin = %caller.sub, "member removed");
    Ok(())
}
