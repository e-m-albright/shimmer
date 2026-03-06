//! Storage trait and S3/MinIO implementation.
//! Works with MinIO (localhost:9000) for local dev - no AWS account needed.
//!
//! Env vars for MinIO:
//!   SHIMMER_S3_ENDPOINT=http://localhost:9000
//!   AWS_ACCESS_KEY_ID=minioadmin
//!   AWS_SECRET_ACCESS_KEY=minioadmin

use aws_config::BehaviorVersion;
use aws_sdk_s3::config::Builder;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::primitives::ByteStream;
use std::error::Error;

/// Metadata for a stored paste.
#[derive(serde::Serialize, Clone)]
pub struct PasteEntry {
    pub id: String,
    pub size: u64,
    pub created: String, // ISO 8601
}

/// Storage backend trait - implement for S3, MinIO, or mock.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), Box<dyn Error + Send + Sync>>;
    async fn get(&self, key: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>;
    async fn list(&self) -> Result<Vec<PasteEntry>, Box<dyn Error + Send + Sync>>;
    async fn delete(&self, key: &str) -> Result<(), Box<dyn Error + Send + Sync>>;
}

/// S3/MinIO-compatible storage.
pub struct S3Storage {
    client: Client,
    bucket: String,
    prefix: String, // e.g. "dev-user" for multi-tenant; TODO(OIDC): use SSO user id
}

impl S3Storage {
    pub async fn new(
        bucket: impl Into<String>,
        prefix: impl Into<String>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let endpoint = std::env::var("SHIMMER_S3_ENDPOINT").ok();
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into());

        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region))
            .load()
            .await;

        let mut s3_builder = Builder::from(&sdk_config);
        if let Some(ref ep) = endpoint {
            s3_builder = s3_builder
                .endpoint_url(ep)
                .force_path_style(true);
        }

        let client = Client::from_conf(s3_builder.build());

        Ok(S3Storage {
            client,
            bucket: bucket.into(),
            prefix: prefix.into(),
        })
    }

    fn full_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{}", self.prefix, key)
        }
    }
}

#[async_trait::async_trait]
impl Storage for S3Storage {
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), Box<dyn Error + Send + Sync>> {
        let full_key = self.full_key(key);
        let body = ByteStream::from(data.to_vec());
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .body(body)
            .send()
            .await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let full_key = self.full_key(key);
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await?;
        let data = resp.body.collect().await?.into_bytes();
        Ok(data.to_vec())
    }

    async fn list(&self) -> Result<Vec<PasteEntry>, Box<dyn Error + Send + Sync>> {
        let prefix = if self.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", self.prefix)
        };
        let resp = self.client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&prefix)
            .send()
            .await?;
        let mut entries = Vec::new();
        if let Some(contents) = resp.contents {
            for obj in contents {
                let key = obj.key.unwrap_or_default();
                let id = key.strip_prefix(&prefix).unwrap_or(&key).to_string();
                if id.is_empty() { continue; }
                entries.push(PasteEntry {
                    id,
                    size: obj.size.unwrap_or(0) as u64,
                    created: obj.last_modified
                        .map(|t| t.fmt(aws_sdk_s3::primitives::DateTimeFormat::DateTime).unwrap_or_default())
                        .unwrap_or_default(),
                });
            }
        }
        Ok(entries)
    }

    async fn delete(&self, key: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let full_key = self.full_key(key);
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await?;
        Ok(())
    }
}

/// File-based storage for dev when MinIO isn't running.
/// Set SHIMMER_USE_FILE_STORAGE=1 to use ./shimmer-dev-storage/
#[derive(Clone)]
pub struct FileStorage {
    base_path: std::path::PathBuf,
    prefix: String,
}

impl FileStorage {
    pub fn new(prefix: impl Into<String>) -> Self {
        let base = std::env::var("SHIMMER_STORAGE_PATH")
            .unwrap_or_else(|_| "./shimmer-dev-storage".into());
        FileStorage {
            base_path: std::path::PathBuf::from(base),
            prefix: prefix.into(),
        }
    }

    fn path_for(&self, key: &str) -> std::path::PathBuf {
        let rel = if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{}", self.prefix, key)
        };
        self.base_path.join(rel)
    }
}

#[async_trait::async_trait]
impl Storage for FileStorage {
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), Box<dyn Error + Send + Sync>> {
        let path = self.path_for(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, data)?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let path = self.path_for(key);
        let data = std::fs::read(&path)?;
        Ok(data)
    }

    async fn list(&self) -> Result<Vec<PasteEntry>, Box<dyn Error + Send + Sync>> {
        let dir = if self.prefix.is_empty() {
            self.base_path.clone()
        } else {
            self.base_path.join(&self.prefix)
        };
        let mut entries = Vec::new();
        if dir.exists() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                if !meta.is_file() { continue; }
                let id = entry.file_name().to_string_lossy().to_string();
                let created = meta.created()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| {
                        let secs = d.as_secs() as i64;
                        chrono::DateTime::from_timestamp(secs, 0)
                            .map(|dt| dt.to_rfc3339())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                entries.push(PasteEntry {
                    id,
                    size: meta.len(),
                    created,
                });
            }
        }
        entries.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(entries)
    }

    async fn delete(&self, key: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let path = self.path_for(key);
        std::fs::remove_file(&path)?;
        Ok(())
    }
}

/// Picks S3 or File storage based on env.
/// File storage (default): SHIMMER_USE_FILE_STORAGE=1 or SHIMMER_S3_ENDPOINT unset
/// S3/MinIO: SHIMMER_S3_ENDPOINT set (e.g. http://localhost:9000)
pub async fn create_storage() -> Result<std::sync::Arc<dyn Storage>, Box<dyn Error + Send + Sync>> {
    let prefix = std::env::var("SHIMMER_USER_PREFIX").unwrap_or_else(|_| DEV_USER_ID.to_string());

    let use_file = std::env::var("SHIMMER_USE_FILE_STORAGE").ok().as_deref() == Some("1");
    let has_s3_endpoint = std::env::var("SHIMMER_S3_ENDPOINT").is_ok();

    if use_file || !has_s3_endpoint {
        Ok(std::sync::Arc::new(FileStorage::new(prefix)))
    } else {
        let bucket = std::env::var("SHIMMER_S3_BUCKET").unwrap_or_else(|_| "shimmer".to_string());
        let s3 = S3Storage::new(bucket, prefix).await?;
        Ok(std::sync::Arc::new(s3))
    }
}

const DEV_USER_ID: &str = "dev-user";
