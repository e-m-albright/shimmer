//! API route definitions.

mod invite;
mod org;
mod paste;

use std::sync::Arc;

use axum::{routing::get, routing::post, Router};

use crate::AppState;

pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        // Health
        .route("/health", get(health))
        // Paste CRUD + search
        .route("/paste", post(paste::upload))
        .route("/paste/{id}", get(paste::fetch).delete(paste::delete))
        .route("/pastes", get(paste::list))
        // Org management (admin)
        .route("/org", post(org::create_org))
        .route("/org/members", get(org::list_members))
        .route(
            "/org/members/{user_id}",
            axum::routing::put(org::update_member_role).delete(org::remove_member),
        )
        // Invite flow
        .route("/org/invite", post(invite::generate_invite))
        .route("/org/join", post(invite::join_org))
}

async fn health() -> &'static str {
    "ok"
}
