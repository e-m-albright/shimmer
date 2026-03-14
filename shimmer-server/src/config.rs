//! Server configuration — loaded from TOML file or env vars.

use serde::Deserialize;

/// Top-level server configuration with nested sections.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub server: ServerSection,
    #[serde(default)]
    pub storage: StorageSection,
    #[serde(default)]
    pub database: DatabaseSection,
    #[serde(default)]
    pub org: OrgSection,
    #[serde(default)]
    pub smtp: Option<SmtpSection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_jwt_secret")]
    pub jwt_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageSection {
    #[serde(default = "default_storage_backend")]
    pub backend: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub s3: Option<S3Section>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct S3Section {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default = "default_s3_bucket")]
    pub bucket: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub access_key_id: Option<String>,
    #[serde(default)]
    pub secret_access_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSection {
    #[serde(default = "default_db_path")]
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrgSection {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpSection {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
}

// --- Defaults ---

fn default_bind() -> String {
    "0.0.0.0:8443".into()
}

fn default_jwt_secret() -> String {
    "dev-secret-change-in-production".into()
}

fn default_storage_backend() -> String {
    "file".into()
}

fn default_s3_bucket() -> String {
    "shimmer".into()
}

fn default_db_path() -> String {
    "./shimmer-metadata.db".into()
}

// --- Default impls ---

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            server: ServerSection::default(),
            storage: StorageSection::default(),
            database: DatabaseSection::default(),
            org: OrgSection::default(),
            smtp: None,
        }
    }
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            jwt_secret: default_jwt_secret(),
        }
    }
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            backend: default_storage_backend(),
            path: None,
            s3: None,
        }
    }
}

impl Default for S3Section {
    fn default() -> Self {
        Self {
            endpoint: None,
            bucket: default_s3_bucket(),
            region: None,
            access_key_id: None,
            secret_access_key: None,
        }
    }
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

impl Default for OrgSection {
    fn default() -> Self {
        Self {
            name: None,
            id: None,
        }
    }
}

impl ServerConfig {
    /// Parse a TOML string into a `ServerConfig`.
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML is malformed.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Load config: SHIMMER_CONFIG env → TOML file → defaults, then overlay env var overrides.
    ///
    /// Panics on fatal errors (unreadable/unparseable config file) since config loading
    /// failure should halt startup.
    pub fn load() -> Self {
        let config_path =
            std::env::var("SHIMMER_CONFIG").unwrap_or_else(|_| "shimmer-server.toml".into());

        let mut config: ServerConfig = if std::path::Path::new(&config_path).exists() {
            let content = std::fs::read_to_string(&config_path)
                .unwrap_or_else(|e| panic!("failed to read config file {config_path}: {e}"));
            toml::from_str(&content)
                .unwrap_or_else(|e| panic!("failed to parse config file {config_path}: {e}"))
        } else {
            ServerConfig::default()
        };

        // Apply env var overrides
        if let Ok(host) = std::env::var("HOST") {
            let port = std::env::var("PORT").unwrap_or_default();
            if port.is_empty() {
                // Extract existing port from bind if present
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
            let s3 = config.storage.s3.get_or_insert_with(S3Section::default);
            s3.endpoint = Some(endpoint);
        }

        if let Ok(bucket) = std::env::var("SHIMMER_S3_BUCKET") {
            let s3 = config.storage.s3.get_or_insert_with(S3Section::default);
            s3.bucket = bucket;
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nested_toml() {
        let toml_str = r#"
[server]
bind = "127.0.0.1:9000"
jwt_secret = "my-secret"

[storage]
backend = "s3"
path = "/data/blobs"

[storage.s3]
endpoint = "http://localhost:9000"
bucket = "my-bucket"
region = "us-east-1"

[database]
path = "/data/shimmer.db"

[org]
name = "Acme Corp"
id = "org_acme"
"#;

        let config = ServerConfig::from_toml(toml_str).expect("parse nested TOML");

        assert_eq!(config.server.bind, "127.0.0.1:9000");
        assert_eq!(config.server.jwt_secret, "my-secret");
        assert_eq!(config.storage.backend, "s3");
        assert_eq!(config.storage.path.as_deref(), Some("/data/blobs"));

        let s3 = config.storage.s3.as_ref().expect("s3 section");
        assert_eq!(s3.endpoint.as_deref(), Some("http://localhost:9000"));
        assert_eq!(s3.bucket, "my-bucket");
        assert_eq!(s3.region.as_deref(), Some("us-east-1"));

        assert_eq!(config.database.path, "/data/shimmer.db");
        assert_eq!(config.org.name.as_deref(), Some("Acme Corp"));
        assert_eq!(config.org.id.as_deref(), Some("org_acme"));
    }

    #[test]
    fn defaults_when_empty_toml() {
        let config = ServerConfig::from_toml("").expect("parse empty TOML");

        assert_eq!(config.server.bind, "0.0.0.0:8443");
        assert_eq!(config.server.jwt_secret, "dev-secret-change-in-production");
        assert_eq!(config.storage.backend, "file");
        assert!(config.storage.path.is_none());
        assert!(config.storage.s3.is_none());
        assert_eq!(config.database.path, "./shimmer-metadata.db");
        assert!(config.org.name.is_none());
        assert!(config.org.id.is_none());
        assert!(config.smtp.is_none());
    }

    #[test]
    fn env_var_overrides() {
        // Set env vars for this test
        std::env::set_var("SHIMMER_CONFIG", "/nonexistent/path.toml");
        std::env::set_var("HOST", "10.0.0.1");
        std::env::set_var("PORT", "3000");
        std::env::set_var("JWT_SECRET", "env-secret");
        std::env::set_var("SHIMMER_STORAGE_BACKEND", "s3");
        std::env::set_var("SHIMMER_STORAGE_PATH", "/mnt/storage");
        std::env::set_var("SHIMMER_DB_PATH", "/mnt/db/meta.db");
        std::env::set_var("SHIMMER_ORG_ID", "org_env");
        std::env::set_var("SHIMMER_ORG_NAME", "Env Org");
        std::env::set_var("SHIMMER_S3_ENDPOINT", "http://minio:9000");
        std::env::set_var("SHIMMER_S3_BUCKET", "env-bucket");

        let config = ServerConfig::load();

        assert_eq!(config.server.bind, "10.0.0.1:3000");
        assert_eq!(config.server.jwt_secret, "env-secret");
        assert_eq!(config.storage.backend, "s3");
        assert_eq!(config.storage.path.as_deref(), Some("/mnt/storage"));
        assert_eq!(config.database.path, "/mnt/db/meta.db");
        assert_eq!(config.org.id.as_deref(), Some("org_env"));
        assert_eq!(config.org.name.as_deref(), Some("Env Org"));

        let s3 = config.storage.s3.as_ref().expect("s3 section from env");
        assert_eq!(s3.endpoint.as_deref(), Some("http://minio:9000"));
        assert_eq!(s3.bucket, "env-bucket");

        // Cleanup
        std::env::remove_var("SHIMMER_CONFIG");
        std::env::remove_var("HOST");
        std::env::remove_var("PORT");
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("SHIMMER_STORAGE_BACKEND");
        std::env::remove_var("SHIMMER_STORAGE_PATH");
        std::env::remove_var("SHIMMER_DB_PATH");
        std::env::remove_var("SHIMMER_ORG_ID");
        std::env::remove_var("SHIMMER_ORG_NAME");
        std::env::remove_var("SHIMMER_S3_ENDPOINT");
        std::env::remove_var("SHIMMER_S3_BUCKET");
    }
}
