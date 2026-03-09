//! Integration tests for the shimmer-server API.
//!
//! Uses axum-test to spin up the router in-process with file-based storage
//! in a temp directory + in-memory SQLite. No network, no Docker, fast.

use std::sync::Arc;

use axum_test::TestServer;
use shimmer_core::storage::FileStorage;
use shimmer_server::{
    auth::{create_token, Claims},
    config::ServerConfig,
    db::Database,
    AppState,
};

/// Build a test server with file storage in a temp directory and in-memory DB.
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
        db_path: None,
        org_id: Some("org_test".into()),
        org_name: Some("Test Org".into()),
    };

    let storage = Box::new(FileStorage::new(tmp.path()));
    let db = Database::open_in_memory().expect("open test db");

    // Create test org
    db.create_org(&shimmer_server::db::OrgRecord {
        id: "org_test".into(),
        name: "Test Org".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
    })
    .expect("create test org");

    // Add test member
    db.add_member(&shimmer_server::db::MemberRecord {
        id: "m_test".into(),
        org_id: "org_test".into(),
        user_id: "u_test_user".into(),
        name: "Test User".into(),
        role: "admin".into(),
        joined_at: chrono::Utc::now().to_rfc3339(),
    })
    .expect("add test member");

    let state = Arc::new(AppState {
        storage,
        db,
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

/// Create a second user's JWT for testing visibility.
fn make_user_token(secret: &str, user_id: &str, name: &str, role: &str) -> String {
    let claims = Claims {
        sub: user_id.into(),
        name: name.into(),
        role: role.into(),
        org: "org_test".into(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
    };
    create_token(&claims, secret).expect("create token")
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

#[tokio::test]
async fn org_visibility_sharing() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    // Alice uploads with org visibility (default)
    let upload_resp = server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "org-visible-data",
            "visibility": "org",
        }))
        .await;

    upload_resp.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = upload_resp.json();
    let id = body["id"].as_str().unwrap();

    // Bob (same org) can fetch it
    let bob_token = make_user_token("test-secret", "u_bob", "Bob", "member");
    let fetch_resp = server
        .get(&format!("/api/paste/{id}"))
        .authorization_bearer(&bob_token)
        .await;

    fetch_resp.assert_status_ok();
    assert_eq!(fetch_resp.text(), "org-visible-data");
}

#[tokio::test]
async fn private_paste_not_visible_to_others() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    // Alice uploads private
    let upload_resp = server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "private-data",
            "visibility": "private",
        }))
        .await;

    let body: serde_json::Value = upload_resp.json();
    let id = body["id"].as_str().unwrap();

    // Bob cannot fetch it
    let bob_token = make_user_token("test-secret", "u_bob", "Bob", "member");
    let fetch_resp = server
        .get(&format!("/api/paste/{id}"))
        .authorization_bearer(&bob_token)
        .await;

    fetch_resp.assert_status(axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn search_by_blind_tokens() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    // Upload with search tokens
    server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "data-with-tokens",
            "visibility": "org",
            "searchTokens": ["token_abc", "token_def"],
        }))
        .await;

    // Upload another without matching tokens
    server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "other-data",
            "visibility": "org",
            "searchTokens": ["token_xyz"],
        }))
        .await;

    // Search for token_abc — should find 1
    let search_resp = server
        .get("/api/pastes?tokens=token_abc")
        .authorization_bearer(&token)
        .await;

    search_resp.assert_status_ok();
    let items: Vec<serde_json::Value> = search_resp.json();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn read_only_cannot_upload() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _token) = test_server(&tmp);

    let readonly_token = make_user_token("test-secret", "u_readonly", "ReadOnly", "read_only");

    let resp = server
        .post("/api/paste")
        .authorization_bearer(&readonly_token)
        .json(&serde_json::json!({
            "ciphertext": "should-fail",
        }))
        .await;

    resp.assert_status(axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn file_upload_with_content_type() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, token) = test_server(&tmp);

    let upload_resp = server
        .post("/api/paste")
        .authorization_bearer(&token)
        .json(&serde_json::json!({
            "ciphertext": "encrypted-image-bytes",
            "contentType": "image/png",
            "visibility": "org",
            "filenameEncrypted": "encrypted-filename-base64",
            "filenameTokens": ["token_lab", "token_results"],
        }))
        .await;

    upload_resp.assert_status(axum::http::StatusCode::CREATED);

    // List should show content type
    let list_resp = server.get("/api/pastes").authorization_bearer(&token).await;
    let items: Vec<serde_json::Value> = list_resp.json();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["contentType"], "image/png");
    assert_eq!(
        items[0]["encryptedFilename"],
        "encrypted-filename-base64"
    );
}

#[tokio::test]
async fn invite_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, admin_token) = test_server(&tmp);

    // Admin generates invite
    let invite_resp = server
        .post("/api/org/invite")
        .authorization_bearer(&admin_token)
        .json(&serde_json::json!({
            "role": "member",
            "ttlHours": 24,
        }))
        .await;

    invite_resp.assert_status(axum::http::StatusCode::CREATED);
    let invite: serde_json::Value = invite_resp.json();
    let invite_token = invite["token"].as_str().unwrap();

    // New user joins with invite token (no auth needed)
    let join_resp = server
        .post("/api/org/join")
        .json(&serde_json::json!({
            "token": invite_token,
            "name": "New User",
        }))
        .await;

    join_resp.assert_status(axum::http::StatusCode::CREATED);
    let join: serde_json::Value = join_resp.json();
    assert_eq!(join["orgId"], "org_test");
    assert_eq!(join["role"], "member");
    assert!(join["jwt"].as_str().is_some());
}

#[tokio::test]
async fn member_management() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, admin_token) = test_server(&tmp);

    // List members
    let list_resp = server
        .get("/api/org/members")
        .authorization_bearer(&admin_token)
        .await;
    list_resp.assert_status_ok();
    let members: Vec<serde_json::Value> = list_resp.json();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["role"], "admin");
}
