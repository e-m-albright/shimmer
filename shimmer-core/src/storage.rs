//! Storage trait and S3/File implementations.
//!
//! The server uses these directly. The Tauri client may use them
//! in standalone/dev mode, or call the server API instead.

use crate::error::StorageError;
use aws_config::BehaviorVersion;
use aws_sdk_s3::config::{Builder, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;

/// Metadata for a stored paste.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PasteEntry {
    pub id: String,
    pub size: u64,
    pub created: String,
    pub user_id: Option<String>,
}

/// Storage backend trait — implement for S3, file, or mock.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), StorageError>;
    async fn get(&self, key: &str) -> Result<Vec<u8>, StorageError>;
    async fn list(&self, prefix: &str) -> Result<Vec<PasteEntry>, StorageError>;
    async fn delete(&self, key: &str) -> Result<(), StorageError>;
}

/// S3-compatible storage (AWS S3, `MinIO`).
#[derive(Debug)]
pub struct S3Storage {
    client: Client,
    bucket: String,
}

impl S3Storage {
    /// Create a new S3 storage backend.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Backend` if the AWS SDK config fails to load.
    pub async fn new(
        bucket: impl Into<String>,
        endpoint: Option<&str>,
        region: Option<&str>,
    ) -> Result<Self, StorageError> {
        let region = Region::new(region.unwrap_or("us-east-1").to_string());
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(region)
            .load()
            .await;

        let mut s3_builder = Builder::from(&sdk_config);
        if let Some(ep) = endpoint {
            s3_builder = s3_builder.endpoint_url(ep).force_path_style(true);
        }

        let client = Client::from_conf(s3_builder.build());

        Ok(S3Storage {
            client,
            bucket: bucket.into(),
        })
    }
}

#[async_trait::async_trait]
impl Storage for S3Storage {
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), StorageError> {
        let body = ByteStream::from(data.to_vec());
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .send()
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StorageError> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        let data = resp
            .body
            .collect()
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?
            .into_bytes();
        Ok(data.to_vec())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<PasteEntry>, StorageError> {
        let list_prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", prefix)
        };
        let resp = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&list_prefix)
            .send()
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        let mut entries = Vec::new();
        if let Some(contents) = resp.contents {
            for obj in contents {
                let key = obj.key.unwrap_or_default();
                let id = key.strip_prefix(&list_prefix).unwrap_or(&key).to_string();
                if id.is_empty() || id.starts_with("_org/") {
                    continue;
                }
                entries.push(PasteEntry {
                    id,
                    size: u64::try_from(obj.size.unwrap_or(0)).unwrap_or(0),
                    created: obj
                        .last_modified
                        .map(|t| {
                            t.fmt(aws_sdk_s3::primitives::DateTimeFormat::DateTime)
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    user_id: None,
                });
            }
        }
        Ok(entries)
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }
}

/// File-based storage for dev when cloud storage isn't configured.
#[derive(Clone, Debug)]
pub struct FileStorage {
    base_path: std::path::PathBuf,
}

impl FileStorage {
    pub fn new(base_path: impl Into<std::path::PathBuf>) -> Self {
        FileStorage {
            base_path: base_path.into(),
        }
    }
}

#[async_trait::async_trait]
impl Storage for FileStorage {
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), StorageError> {
        let path = self.base_path.join(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, data)?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StorageError> {
        let path = self.base_path.join(key);
        std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StorageError::NotFound(key.to_string()),
            _ => StorageError::Io(e),
        })
    }

    async fn list(&self, prefix: &str) -> Result<Vec<PasteEntry>, StorageError> {
        // Avoid cloning base_path when prefix is empty — borrow instead.
        let owned_dir;
        let dir: &std::path::Path = if prefix.is_empty() {
            &self.base_path
        } else {
            owned_dir = self.base_path.join(prefix);
            &owned_dir
        };

        let mut entries = Vec::new();
        if dir.exists() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                if !meta.is_file() {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().into_owned();
                let created = meta
                    .created()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| {
                        chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                            .map(|dt| dt.to_rfc3339())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                entries.push(PasteEntry {
                    id,
                    size: meta.len(),
                    created,
                    user_id: None,
                });
            }
        }
        entries.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(entries)
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let path = self.base_path.join(key);
        std::fs::remove_file(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StorageError::NotFound(key.to_string()),
            _ => StorageError::Io(e),
        })
    }
}
