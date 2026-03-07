//! API route definitions.

mod paste;

use std::sync::Arc;

use axum::{routing::get, Router};

use crate::AppState;

pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/paste", axum::routing::post(paste::upload))
        .route("/paste/{id}", get(paste::fetch).delete(paste::delete))
        .route("/pastes", get(paste::list))
}

async fn health() -> &'static str {
    "ok"
}
