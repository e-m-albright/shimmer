//! shimmer-server — Zero-knowledge PHI storage API gateway.
//!
//! The server sits between Tauri clients and cloud storage.
//! It enforces auth, permissions, audit logging, TTL, and burn-on-read.
//! It NEVER sees plaintext or the org KEK.

pub mod auth;
pub mod config;
pub mod db;
pub mod routes;
pub mod services;
pub mod tui;

use std::fmt;
use std::sync::Arc;

use axum::Router;
use shimmer_core::storage::Storage;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Shared application state available to all route handlers.
pub struct AppState {
    pub storage: Box<dyn Storage>,
    pub db: db::Database,
    pub config: config::ServerConfig,
}

impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState")
            .field("config", &self.config)
            .field("db", &self.db)
            .finish()
    }
}

/// Build the Axum router. Usable from both `main()` and integration tests.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/api", routes::api_routes())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive()) // TODO: restrict in production
        .with_state(state)
}
