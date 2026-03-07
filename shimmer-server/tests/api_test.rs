//! Integration tests for the shimmer-server API.
//!
//! Uses axum-test to spin up the router in-process with file-based storage
//! in a temp directory. No network, no Docker, fast.

use std::sync::Arc;

use axum_test::TestServer;
use shimmer_core::storage::FileStorage;
use shimmer_server::{
    auth::{create_token, Claims},
    config::ServerConfig,
    AppState,
};

/// Build a test server with file storage in a temp directory.
fn test_server(tmp: &tempfile::TempDir) -> (TestServer, String) {
    let jwt_secret = "test-secret".to_string();

    let config = ServerConfig {
        host: "127.0.0.1".into(),
        port: 0,
        storage_backend: "file".into(),
        s3_endpoint: None,
        s3_bucket: "test".into(),
        s3_region: None,
        file_storage_path: Some(tmp.path().to_string_lossy().into_owned()),
        org_id: Some("org_test".into()),
        org_name: Some("Test Org".into()),
    };

    let storage = Box::new(FileStorage::new(tmp.path()));

    let state = Arc::new(AppState {
        storage,
        config,
        jwt_secret: jwt_secret.clone(),
    });

    let app = shimmer_server::build_router(state);
    let server = TestServer::new(app);

    // Create a test JWT
    let claims = Claims {
        sub: "u_test_user".into(),
        name: "Test User".into(),
        role: "admin".into(),
        org: "org_test".into(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
    };
    let token = create_token(&claims, &jwt_secret).expect("create test token");

    (server, token)
}

#[tokio::test]
async fn health_check() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _token) = test_server(&tmp);

    let resp = server.get("/api/health").await;
    resp.assert_status_ok();
    resp.assert_text("ok");
}

#[tokio::test]
async fn upload_and_fetch_paste() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    // Upload
    let upload_resp = server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "encrypted-data-here",
            "visibility": "private",
        }))
        .await;

    upload_resp.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = upload_resp.json();
    let id = body["id"].as_str().expect("response has id");
    assert!(body["phiUrl"].as_str().unwrap().starts_with("phi://"));

    // Fetch
    let fetch_resp = server
        .get(&format!("/api/paste/{id}"))
        .authorization_bearer(&token)
        .await;

    fetch_resp.assert_status_ok();
    assert_eq!(fetch_resp.text(), "encrypted-data-here");
}

#[tokio::test]
async fn upload_requires_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _token) = test_server(&tmp);

    let resp = server
        .post("/api/paste")
        .json(&serde_json::json!({
            "ciphertext": "data",
        }))
        .await;

    resp.assert_status_unauthorized();
}

#[tokio::test]
async fn upload_rejects_empty_ciphertext() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    let resp = server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "",
        }))
        .await;

    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_rejects_invalid_visibility() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    let resp = server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "data",
            "visibility": "public",
        }))
        .await;

    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_paste() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    // Upload first
    let upload_resp = server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "to-be-deleted",
        }))
        .await;

    let body: serde_json::Value = upload_resp.json();
    let id = body["id"].as_str().unwrap();

    // Delete
    let del_resp = server
        .delete(&format!("/api/paste/{id}"))
        .authorization_bearer(&token)
        .await;

    del_resp.assert_status(axum::http::StatusCode::NO_CONTENT);

    // Fetch should fail
    let fetch_resp = server
        .get(&format!("/api/paste/{id}"))
        .authorization_bearer(&token)
        .await;

    fetch_resp.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_pastes() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    // Upload two pastes
    for i in 0..2 {
        server
            .post("/api/paste")
            .authorization_bearer(&token)
            .json(&serde_json::json!({
                "ciphertext": format!("paste-{i}"),
            }))
            .await;
    }

    let list_resp = server.get("/api/pastes").authorization_bearer(&token).await;

    list_resp.assert_status_ok();
    let items: Vec<serde_json::Value> = list_resp.json();
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn fetch_invalid_uuid_returns_bad_request() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    let resp = server
        .get("/api/paste/not-a-uuid")
        .authorization_bearer(&token)
        .await;

    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}
