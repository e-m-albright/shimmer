//! shimmer-server entrypoint.

use std::sync::Arc;

use shimmer_core::storage::{FileStorage, S3Storage, Storage};
use shimmer_server::{build_router, config, AppState};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logging — JSON output in production (LOG_FORMAT=json), human-readable otherwise
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if std::env::var("LOG_FORMAT").ok().as_deref() == Some("json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    // Config
    let config = config::ServerConfig::load()?;
    info!(port = config.port, "shimmer-server starting");

    // Bind address extracted before config is moved into AppState
    let addr = format!("{}:{}", config.host, config.port);

    // Storage backend
    let storage: Box<dyn Storage> = match config.storage_backend.as_str() {
        "s3" => {
            let s3 = S3Storage::new(
                &config.s3_bucket,
                config.s3_endpoint.as_deref(),
                config.s3_region.as_deref(),
            )
            .await?;
            Box::new(s3)
        }
        _ => {
            let path = config
                .file_storage_path
                .as_deref()
                .unwrap_or("./shimmer-storage");
            Box::new(FileStorage::new(path))
        }
    };

    let jwt_secret =
        std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-in-production".into());

    let state = Arc::new(AppState {
        storage,
        config,
        jwt_secret,
    });

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "listening");
    axum::serve(listener, app).await?;

    Ok(())
}
