//! Server configuration — loaded from TOML file or env vars.

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,

    // Storage
    #[serde(default = "default_storage_backend")]
    pub storage_backend: String,
    #[serde(default)]
    pub s3_endpoint: Option<String>,
    #[serde(default = "default_s3_bucket")]
    pub s3_bucket: String,
    #[serde(default)]
    pub s3_region: Option<String>,
    #[serde(default)]
    pub file_storage_path: Option<String>,

    // Org
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub org_name: Option<String>,
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8443
}
fn default_storage_backend() -> String {
    "file".into()
}
fn default_s3_bucket() -> String {
    "shimmer".into()
}

impl ServerConfig {
    /// Load config from `shimmer-server.toml` if it exists, then overlay env vars.
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML file exists but cannot be read or parsed.
    pub fn load() -> Result<Self> {
        let config_path =
            std::env::var("SHIMMER_CONFIG").unwrap_or_else(|_| "shimmer-server.toml".into());

        let config: ServerConfig = if std::path::Path::new(&config_path).exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            // Fall back to env vars
            ServerConfig {
                host: std::env::var("HOST").unwrap_or_else(|_| default_host()),
                port: std::env::var("PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or_else(default_port),
                storage_backend: std::env::var("SHIMMER_STORAGE_BACKEND")
                    .unwrap_or_else(|_| default_storage_backend()),
                s3_endpoint: std::env::var("SHIMMER_S3_ENDPOINT").ok(),
                s3_bucket: std::env::var("SHIMMER_S3_BUCKET")
                    .unwrap_or_else(|_| default_s3_bucket()),
                s3_region: std::env::var("AWS_REGION").ok(),
                file_storage_path: std::env::var("SHIMMER_STORAGE_PATH").ok(),
                org_id: std::env::var("SHIMMER_ORG_ID").ok(),
                org_name: std::env::var("SHIMMER_ORG_NAME").ok(),
            }
        };

        Ok(config)
    }
}
