//! HTTP client for shimmer-server.
//!
//! The Tauri app encrypts data locally (the KEK never leaves the device),
//! then ships the opaque ciphertext to shimmer-server over HTTP.
//! The server stores ciphertext to S3 — it never sees plaintext or the KEK.
//!
//! Architecture:
//! ```text
//! Tauri (encrypt w/ local KEK) ──HTTP──▶ shimmer-server ──▶ S3
//!                                         (ciphertext only)
//! ```

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::CommandError;

/// HTTP client bound to a specific shimmer-server instance.
#[derive(Debug, Clone)]
pub struct ShimmerClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

/// Outgoing upload body — mirrors `UploadRequest` on the server (camelCase).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadBody<'a> {
    /// Opaque ciphertext: the JSON-serialized `EnvelopePayload` as a string.
    ciphertext: &'a str,
    /// Content type (e.g., "text/plain", "image/png").
    content_type: &'a str,
    /// Visibility: "private", "org", "link".
    visibility: &'a str,
    /// Blind index tokens for content search.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    search_tokens: Vec<String>,
    /// Encrypted title (base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    title_encrypted: Option<String>,
    /// Blind tokens for title search.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    title_tokens: Vec<String>,
    /// Encrypted filename (base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    filename_encrypted: Option<String>,
    /// Blind tokens for filename search.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    filename_tokens: Vec<String>,
    /// Tags (blind-indexed).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tag_tokens: Vec<String>,
    /// TTL in hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl_hours: Option<u64>,
    /// Delete after first read.
    burn_on_read: bool,
}

/// Server's upload response.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponseBody {
    id: String,
}

/// Paste summary returned by the list endpoint.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PasteSummary {
    pub id: String,
    pub content_type: String,
    pub encrypted_title: Option<String>,
    pub encrypted_filename: Option<String>,
    pub created_at: String,
    pub size: i64,
    pub visibility: String,
    pub user_id: String,
    pub user_name: String,
    pub burn_on_read: bool,
    pub ttl_hours: Option<i64>,
}

/// Options for an upload request.
#[derive(Debug, Default)]
pub struct UploadOptions {
    pub content_type: String,
    pub visibility: String,
    pub search_tokens: Vec<String>,
    pub title_encrypted: Option<String>,
    pub title_tokens: Vec<String>,
    pub filename_encrypted: Option<String>,
    pub filename_tokens: Vec<String>,
    pub tag_tokens: Vec<String>,
    pub ttl_hours: Option<u64>,
    pub burn_on_read: bool,
}

impl ShimmerClient {
    /// Create a client pointed at `base_url`, authenticated with `token`.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be built.
    pub fn new(base_url: String, token: String) -> Result<Self, CommandError> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| CommandError::Internal(format!("HTTP client init: {e}")))?;
        Ok(Self {
            base_url,
            token,
            http,
        })
    }

    /// Update the auth token (after joining an org).
    #[allow(dead_code)] // Used when invite flow UI is wired
    pub fn set_token(&mut self, token: String) {
        self.token = token;
    }

    /// Upload ciphertext with metadata. Returns the new paste UUID.
    ///
    /// # Errors
    ///
    /// Returns `CommandError::Server` on HTTP error or non-2xx response.
    pub async fn upload(
        &self,
        ciphertext_json: &str,
        opts: &UploadOptions,
    ) -> Result<String, CommandError> {
        let body = UploadBody {
            ciphertext: ciphertext_json,
            content_type: &opts.content_type,
            visibility: &opts.visibility,
            search_tokens: opts.search_tokens.clone(),
            title_encrypted: opts.title_encrypted.clone(),
            title_tokens: opts.title_tokens.clone(),
            filename_encrypted: opts.filename_encrypted.clone(),
            filename_tokens: opts.filename_tokens.clone(),
            tag_tokens: opts.tag_tokens.clone(),
            ttl_hours: opts.ttl_hours,
            burn_on_read: opts.burn_on_read,
        };

        let resp = self
            .http
            .post(format!("{}/api/paste", self.base_url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CommandError::Server(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CommandError::Server(format!("{status}: {body}")));
        }

        let body: UploadResponseBody = resp
            .json()
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))?;

        debug!(paste_id = %body.id, "uploaded to server");
        Ok(body.id)
    }

    /// Fetch raw ciphertext bytes for a paste ID.
    ///
    /// # Errors
    ///
    /// Returns `CommandError::NotFound` on 404, `CommandError::Server` on other errors.
    pub async fn fetch(&self, id: &str) -> Result<Vec<u8>, CommandError> {
        let resp = self
            .http
            .get(format!("{}/api/paste/{}", self.base_url, id))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| CommandError::Server(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(CommandError::NotFound(format!("paste {id} not found")));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(CommandError::Server("access denied".into()));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CommandError::Server(format!("{status}: {body}")));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| CommandError::Internal(e.to_string()))
    }

    /// List/search pastes for the authenticated user's org.
    ///
    /// If `search_tokens` is provided, performs blind-index search.
    /// Otherwise returns all visible pastes.
    ///
    /// # Errors
    ///
    /// Returns `CommandError::Server` on HTTP error.
    pub async fn list(
        &self,
        search_tokens: Option<&[String]>,
    ) -> Result<Vec<PasteSummary>, CommandError> {
        let mut url = format!("{}/api/pastes", self.base_url);

        if let Some(tokens) = search_tokens {
            if !tokens.is_empty() {
                url = format!("{}?tokens={}", url, tokens.join(","));
            }
        }

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| CommandError::Server(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CommandError::Server(format!("{status}: {body}")));
        }

        resp.json()
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))
    }

    /// Delete a paste by ID.
    ///
    /// # Errors
    ///
    /// Returns `CommandError::NotFound` on 404, `CommandError::Server` on other errors.
    pub async fn delete(&self, id: &str) -> Result<(), CommandError> {
        let resp = self
            .http
            .delete(format!("{}/api/paste/{}", self.base_url, id))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| CommandError::Server(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(CommandError::NotFound(format!("paste {id} not found")));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CommandError::Server(format!("{status}: {body}")));
        }

        Ok(())
    }

    /// Join an org using an invite token.
    ///
    /// # Errors
    ///
    /// Returns `CommandError::Server` on failure.
    #[allow(dead_code)] // Used when invite flow UI is wired
    pub async fn join_org(
        &self,
        invite_token: &str,
        name: &str,
    ) -> Result<JoinResponse, CommandError> {
        let resp = self
            .http
            .post(format!("{}/api/org/join", self.base_url))
            .json(&serde_json::json!({
                "token": invite_token,
                "name": name,
            }))
            .send()
            .await
            .map_err(|e| CommandError::Server(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CommandError::Server(format!("{status}: {body}")));
        }

        resp.json()
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))
    }
}

/// Response from joining an org.
#[allow(dead_code)] // Used when invite flow UI is wired
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinResponse {
    pub org_id: String,
    pub user_id: String,
    pub jwt: String,
    pub role: String,
    pub server_url: String,
}
