//! Organization and member management routes.
//!
//! Thin wrappers around `services::org` — validate HTTP, call service, map errors.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::auth::Claims;
use crate::db::MemberRecord;
use crate::services::org::{self, OrgCaller, OrgServiceError};
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

/// Update role request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRoleRequest {
    pub role: String,
}

/// Map `OrgServiceError` to HTTP status + message.
fn map_org_err(e: OrgServiceError) -> (StatusCode, String) {
    match e {
        OrgServiceError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        OrgServiceError::Forbidden => (StatusCode::FORBIDDEN, "admin only".into()),
        OrgServiceError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
        OrgServiceError::Db(_) | OrgServiceError::Internal(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

fn caller_from_claims(claims: &Claims) -> OrgCaller {
    OrgCaller {
        sub: claims.sub.clone(),
        name: claims.name.clone(),
        org: claims.org.clone(),
        role: claims.role.clone(),
    }
}

/// Create a new organization. The caller becomes the first admin.
pub async fn create_org(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateOrgRequest>,
) -> Result<(StatusCode, Json<CreateOrgResponse>), (StatusCode, String)> {
    let caller = caller_from_claims(&claims);
    let output = org::create_org(state, &caller, &req.name)
        .await
        .map_err(map_org_err)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateOrgResponse {
            org_id: output.org_id,
        }),
    ))
}

/// List all members of the caller's org.
pub async fn list_members(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Result<Json<Vec<MemberInfo>>, (StatusCode, String)> {
    let members = org::list_members(state, &claims.org)
        .await
        .map_err(map_org_err)?;

    Ok(Json(members.into_iter().map(MemberInfo::from).collect()))
}

/// Update a member's role. Admin only.
pub async fn update_member_role(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let caller = caller_from_claims(&claims);
    org::update_role(state, &caller, &user_id, &req.role)
        .await
        .map_err(|e| match e {
            OrgServiceError::NotFound => {
                (StatusCode::NOT_FOUND, format!("member {user_id} not found"))
            }
            other => map_org_err(other),
        })?;

    Ok(StatusCode::OK)
}

/// Remove a member from the org. Admin only.
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(user_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let caller = caller_from_claims(&claims);
    org::remove_member(state, &caller, &user_id)
        .await
        .map_err(|e| match e {
            OrgServiceError::NotFound => {
                (StatusCode::NOT_FOUND, format!("member {user_id} not found"))
            }
            other => map_org_err(other),
        })?;

    Ok(StatusCode::NO_CONTENT)
}
