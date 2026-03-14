//! shimmer-server entrypoint.

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use clap::{Parser, Subcommand};
use rand::RngCore;
use shimmer_core::storage::{FileStorage, S3Storage, Storage};
use shimmer_server::{build_router, config, db, db::Database, services, tui, AppState};
use tracing::{error, info};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "shimmer-server", about = "Shimmer PHI sharing server")]
struct Cli {
    /// Path to TOML config file (overrides `SHIMMER_CONFIG` env var)
    #[arg(long, global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the API server (default if no subcommand given)
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
        /// Email of the member to remove
        email: String,
    },
    /// Change a member's role
    SetRole {
        /// Email of the member
        email: String,
        /// New role: admin, member, or read_only
        role: String,
    },
}

// ---------------------------------------------------------------------------
// Config loading helper
// ---------------------------------------------------------------------------

/// Load config, using the CLI `--config` path if provided, otherwise the default
/// `SHIMMER_CONFIG` env var / `shimmer-server.toml` fallback.
fn load_config(cli_path: Option<&str>) -> config::ServerConfig {
    if let Some(path) = cli_path {
        // Read and parse the explicit config file, then apply env overlays
        // via the normal load() path by temporarily storing the path.
        // Since load() reads SHIMMER_CONFIG, we replicate its logic here
        // to avoid needing unsafe set_var.
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read config file {path}: {e}"));
        let mut cfg: config::ServerConfig = toml::from_str(&content)
            .unwrap_or_else(|e| panic!("failed to parse config file {path}: {e}"));
        apply_env_overrides(&mut cfg);
        cfg
    } else {
        config::ServerConfig::load()
    }
}

/// Apply environment variable overrides to a config. Mirrors the logic in
/// `ServerConfig::load()` for env var overlays.
fn apply_env_overrides(config: &mut config::ServerConfig) {
    if let Ok(host) = std::env::var("HOST") {
        let port = std::env::var("PORT").unwrap_or_default();
        if port.is_empty() {
            let existing_port = config
                .server
                .bind
                .rsplit_once(':')
                .map(|(_, p)| p.to_string())
                .unwrap_or_else(|| "8443".into());
            config.server.bind = format!("{host}:{existing_port}");
        } else {
            config.server.bind = format!("{host}:{port}");
        }
    } else if let Ok(port) = std::env::var("PORT") {
        let existing_host = config
            .server
            .bind
            .rsplit_once(':')
            .map(|(h, _)| h.to_string())
            .unwrap_or_else(|| "0.0.0.0".into());
        config.server.bind = format!("{existing_host}:{port}");
    }

    if let Ok(secret) = std::env::var("JWT_SECRET") {
        config.server.jwt_secret = secret;
    }
    if let Ok(backend) = std::env::var("SHIMMER_STORAGE_BACKEND") {
        config.storage.backend = backend;
    }
    if let Ok(path) = std::env::var("SHIMMER_STORAGE_PATH") {
        config.storage.path = Some(path);
    }
    if let Ok(path) = std::env::var("SHIMMER_DB_PATH") {
        config.database.path = path;
    }
    if let Ok(org_id) = std::env::var("SHIMMER_ORG_ID") {
        config.org.id = Some(org_id);
    }
    if let Ok(org_name) = std::env::var("SHIMMER_ORG_NAME") {
        config.org.name = Some(org_name);
    }
    if let Ok(endpoint) = std::env::var("SHIMMER_S3_ENDPOINT") {
        let s3 = config
            .storage
            .s3
            .get_or_insert_with(config::S3Section::default);
        s3.endpoint = Some(endpoint);
    }
    if let Ok(bucket) = std::env::var("SHIMMER_S3_BUCKET") {
        let s3 = config
            .storage
            .s3
            .get_or_insert_with(config::S3Section::default);
        s3.bucket = bucket;
    }
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => run_server(cli.config.as_deref()).await,
        Command::Setup => run_setup(),
        Command::Admin { action } => run_admin(cli.config.as_deref(), action),
    }
}

// ---------------------------------------------------------------------------
// Setup wizard
// ---------------------------------------------------------------------------

#[allow(clippy::expect_used)]
fn run_setup() {
    let setup_cfg =
        tui::setup::run_setup_wizard().expect("failed to initialise terminal for setup wizard");

    let Some(setup_cfg) = setup_cfg else {
        eprintln!("Setup cancelled.");
        std::process::exit(0);
    };

    // Generate a random JWT secret
    let jwt_secret = tui::setup::generate_jwt_secret();

    // Write shimmer.toml
    tui::setup::write_config_file(&setup_cfg, &jwt_secret).expect("failed to write shimmer.toml");

    // Initialise the database and create the org
    let db = Database::open(std::path::Path::new(&setup_cfg.db_path))
        .expect("failed to open/create database");

    let org_id = format!("org_{}", uuid::Uuid::new_v4());
    db.create_org(&db::OrgRecord {
        id: org_id.clone(),
        name: setup_cfg.org_name.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    })
    .expect("failed to create org in database");

    // Register the admin user via the auth service
    services::auth::register(
        &db,
        &services::auth::RegisterInput {
            email: setup_cfg.admin_email.clone(),
            password: setup_cfg.admin_password.clone(),
            org_id: org_id.clone(),
            role: "admin".into(),
            name: "Admin".into(),
        },
        &jwt_secret,
    )
    .expect("failed to create admin user");

    // Persist the org.id into shimmer.toml so `serve` can find it
    // Append org.id line — simplest approach to avoid re-parsing
    let existing = std::fs::read_to_string("shimmer.toml").expect("failed to re-read shimmer.toml");
    let updated = existing.replace(
        &format!("[org]\nname = \"{}\"", setup_cfg.org_name),
        &format!(
            "[org]\nname = \"{}\"\nid = \"{}\"",
            setup_cfg.org_name, org_id
        ),
    );
    std::fs::write("shimmer.toml", &updated).expect("failed to update shimmer.toml with org id");

    println!();
    println!("Setup complete!");
    println!();
    println!("  Config written to: shimmer.toml (permissions: 0600)");
    println!("  Organisation:      {} ({})", setup_cfg.org_name, org_id);
    println!("  Admin email:       {}", setup_cfg.admin_email);
    println!();
    println!("Next steps:");
    println!("  shimmer-server serve");
    println!();
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[allow(clippy::expect_used)]
async fn run_server(config_path: Option<&str>) {
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
    let config = load_config(config_path);
    info!(bind = %config.server.bind, "shimmer-server starting");

    // Bind address extracted before config is moved into AppState
    let addr = config.server.bind.clone();

    // Storage backend (blob storage for ciphertext)
    let storage: Box<dyn Storage> = match config.storage.backend.as_str() {
        "s3" => {
            let s3_cfg = config.storage.s3.as_ref();
            let bucket = s3_cfg.map(|s| s.bucket.as_str()).unwrap_or("shimmer");
            let endpoint = s3_cfg.and_then(|s| s.endpoint.as_deref());
            let region = s3_cfg.and_then(|s| s.region.as_deref());
            let s3 = S3Storage::new(bucket, endpoint, region)
                .await
                .expect("failed to initialise S3 storage");
            Box::new(s3)
        }
        _ => {
            let path = config
                .storage
                .path
                .as_deref()
                .unwrap_or("./shimmer-storage");
            Box::new(FileStorage::new(path))
        }
    };

    // Metadata database
    let db = Database::open(std::path::Path::new(&config.database.path))
        .expect("failed to open database");

    // Auto-create the dev org if configured and not already present
    if let Some(ref org_id) = config.org.id {
        if db.get_org(org_id).expect("db error checking org").is_none() {
            let org_name = config.org.name.as_deref().unwrap_or("Development Org");
            db.create_org(&db::OrgRecord {
                id: org_id.clone(),
                name: org_name.to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
            .expect("failed to create dev org");
            info!(org_id, org_name, "auto-created development org");
        }
    }

    let state = Arc::new(AppState {
        storage,
        db,
        config,
    });

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind listener");
    info!(%addr, "listening");
    axum::serve(listener, app)
        .await
        .expect("server exited with error");
}

// ---------------------------------------------------------------------------
// Admin commands
// ---------------------------------------------------------------------------

#[allow(clippy::expect_used)]
fn run_admin(config_path: Option<&str>, action: AdminAction) {
    let config = load_config(config_path);
    let db = Database::open(std::path::Path::new(&config.database.path))
        .expect("failed to open database");

    let org_id = config
        .org
        .id
        .as_deref()
        .expect("org.id must be set in config to use admin commands");

    match action {
        AdminAction::Invite { email } => {
            // Generate a 256-bit random invite token
            let mut bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut bytes);
            let token = URL_SAFE_NO_PAD.encode(bytes);

            let now = chrono::Utc::now();
            let expires_at = (now + chrono::Duration::hours(72)).to_rfc3339();

            let invite = db::InviteRecord {
                token: token.clone(),
                org_id: org_id.to_string(),
                role: "member".to_string(),
                created_by: "admin-cli".to_string(),
                expires_at,
                used_at: None,
                used_by: None,
                single_use: true,
            };

            db.create_invite(&invite).expect("failed to create invite");

            println!("Invite created for {email}");
            println!("Token: {token}");
            println!("Partial invite URL: phi://join/{token}");
            println!();
            println!(
                "Complete this invite in the Shimmer desktop app to attach the encrypted org key."
            );
        }
        AdminAction::ListMembers => {
            let members = db.list_members(org_id).expect("failed to list members");
            if members.is_empty() {
                println!("No members found.");
            } else {
                println!("{:<20} {:<30} {:<10}", "NAME", "USER ID", "ROLE");
                println!("{}", "-".repeat(60));
                for m in members {
                    println!("{:<20} {:<30} {:<10}", m.name, m.user_id, m.role);
                }
            }
        }
        AdminAction::Remove { email } => {
            let user = db
                .get_user_by_email(&email)
                .expect("db error")
                .unwrap_or_else(|| {
                    error!(%email, "user not found");
                    std::process::exit(1);
                });
            db.remove_member(org_id, &user.id)
                .expect("failed to remove member");
            db.delete_refresh_tokens_for_user(&user.id)
                .expect("failed to revoke tokens");
            println!("Removed {email} and revoked all sessions.");
        }
        AdminAction::SetRole { email, role } => {
            let valid_roles = ["admin", "member", "read_only"];
            let role_lower = role.to_lowercase();
            if !valid_roles.contains(&role_lower.as_str()) {
                eprintln!(
                    "Invalid role '{role}'. Must be one of: {}",
                    valid_roles.join(", ")
                );
                std::process::exit(1);
            }
            let user = db
                .get_user_by_email(&email)
                .expect("db error")
                .unwrap_or_else(|| {
                    error!(%email, "user not found");
                    std::process::exit(1);
                });
            db.update_member_role(org_id, &user.id, &role_lower)
                .expect("failed to update role");
            println!("Set {email} role to {role_lower}.");
        }
    }
}
