//! shimmer-server entrypoint.

use std::sync::Arc;

use shimmer_core::storage::{FileStorage, S3Storage, Storage};
use shimmer_server::{build_router, config, db::Database, AppState};
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

    // Storage backend (blob storage for ciphertext)
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

    // Metadata database
    let db_path = config
        .db_path
        .as_deref()
        .unwrap_or("./shimmer-metadata.db");
    let db = Database::open(std::path::Path::new(db_path))?;

    // Auto-create the dev org if configured and not already present
    if let Some(ref org_id) = config.org_id {
        if db.get_org(org_id)?.is_none() {
            let org_name = config
                .org_name
                .as_deref()
                .unwrap_or("Development Org");
            db.create_org(&shimmer_server::db::OrgRecord {
                id: org_id.clone(),
                name: org_name.to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })?;
            info!(org_id, org_name, "auto-created development org");
        }
    }

    let jwt_secret =
        std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-in-production".into());

    let state = Arc::new(AppState {
        storage,
        db,
        config,
        jwt_secret,
    });

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "listening");
    axum::serve(listener, app).await?;

    Ok(())
}
