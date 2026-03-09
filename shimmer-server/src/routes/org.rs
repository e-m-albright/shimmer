//! Organization and member management routes.
//!
//! Admin-only: create org, manage members, promote/demote roles.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::auth::Claims;
use crate::db::{MemberRecord, OrgRecord};
use crate::AppState;

/// Create org request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrgRequest {
    pub name: String,
}

/// Create org response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrgResponse {
    pub org_id: String,
}

/// Create a new organization. The caller becomes the first admin.
pub async fn create_org(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateOrgRequest>,
) -> Result<(StatusCode, Json<CreateOrgResponse>), (StatusCode, String)> {
    let org_id = format!("org_{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    let org = OrgRecord {
        id: org_id.clone(),
        name: req.name.clone(),
        created_at: now.clone(),
    };

    let member = MemberRecord {
        id: format!("m_{}", uuid::Uuid::new_v4()),
        org_id: org_id.clone(),
        user_id: claims.sub.clone(),
        name: claims.name.clone(),
        role: "admin".into(),
        joined_at: now,
    };

    let db_state = state.clone();
    tokio::task::spawn_blocking(move || {
        db_state.db.create_org(&org)?;
        db_state.db.add_member(&member)?;
        Ok::<(), crate::db::DbError>(())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(org_id = %org_id, user_id = %claims.sub, "org created");

    Ok((StatusCode::CREATED, Json(CreateOrgResponse { org_id })))
}

/// Member info returned in API responses.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfo {
    pub user_id: String,
    pub name: String,
    pub role: String,
    pub joined_at: String,
}

impl From<MemberRecord> for MemberInfo {
    fn from(m: MemberRecord) -> Self {
        Self {
            user_id: m.user_id,
            name: m.name,
            role: m.role,
            joined_at: m.joined_at,
        }
    }
}

/// List all members of the caller's org.
pub async fn list_members(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Result<Json<Vec<MemberInfo>>, (StatusCode, String)> {
    let org_id = claims.org.clone();
    let db_state = state.clone();
    let members = tokio::task::spawn_blocking(move || db_state.db.list_members(&org_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(members.into_iter().map(MemberInfo::from).collect()))
}

/// Update role request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRoleRequest {
    pub role: String,
}

const VALID_ROLES: &[&str] = &["admin", "member", "read_only"];

/// Update a member's role. Admin only.
pub async fn update_member_role(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !claims.is_admin() {
        return Err((StatusCode::FORBIDDEN, "admin only".into()));
    }

    if !VALID_ROLES.contains(&req.role.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "invalid role '{}', must be one of: {}",
                req.role,
                VALID_ROLES.join(", ")
            ),
        ));
    }

    // Don't allow demoting yourself (prevents org lockout)
    if user_id == claims.sub {
        return Err((
            StatusCode::BAD_REQUEST,
            "cannot change your own role".into(),
        ));
    }

    let org_id = claims.org.clone();
    let db_state = state.clone();
    let role = req.role.clone();
    let target_id = user_id.clone();
    let updated =
        tokio::task::spawn_blocking(move || db_state.db.update_member_role(&org_id, &target_id, &role))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !updated {
        return Err((StatusCode::NOT_FOUND, format!("member {user_id} not found")));
    }

    info!(
        user_id = %user_id,
        new_role = %req.role,
        admin = %claims.sub,
        "member role updated"
    );
    Ok(StatusCode::OK)
}

/// Remove a member from the org. Admin only.
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(user_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !claims.is_admin() {
        return Err((StatusCode::FORBIDDEN, "admin only".into()));
    }

    if user_id == claims.sub {
        return Err((
            StatusCode::BAD_REQUEST,
            "cannot remove yourself from the org".into(),
        ));
    }

    let org_id = claims.org.clone();
    let db_state = state.clone();
    let target_id = user_id.clone();
    let removed =
        tokio::task::spawn_blocking(move || db_state.db.remove_member(&org_id, &target_id))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !removed {
        return Err((StatusCode::NOT_FOUND, format!("member {user_id} not found")));
    }

    info!(user_id = %user_id, admin = %claims.sub, "member removed");
    Ok(StatusCode::NO_CONTENT)
}
