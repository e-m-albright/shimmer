//! Shimmer — PHI sharing utility. Tray-only, invisible in workflow.
//!
//! Architecture: encrypt locally → ship ciphertext to shimmer-server → S3.
//! The KEK never leaves this process. The server never sees plaintext.

mod client;
mod error;
mod key_store;

use std::sync::Arc;

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use client::{PasteSummary, ShimmerClient, UploadOptions};
use error::CommandError;
use key_store::KeyStore;
use shimmer_core::encryption::{
    blind_index_token, decrypt_envelope, derive_search_key, encrypt_envelope, extract_blind_tokens,
    EnvelopePayload,
};

const TRAY_ID: &str = "shimmer-tray";

/// Build a [`ShimmerClient`] from environment variables.
///
/// - `SHIMMER_SERVER_URL` — base URL (default: `http://localhost:8443`)
/// - `SHIMMER_JWT` — Bearer token (required; generate with `just gen-token`)
fn build_client() -> Result<ShimmerClient, anyhow::Error> {
    let base_url =
        std::env::var("SHIMMER_SERVER_URL").unwrap_or_else(|_| "http://localhost:8443".into());
    let token = std::env::var("SHIMMER_JWT").unwrap_or_else(|_| {
        warn!("SHIMMER_JWT not set — API calls will fail with 401. Run `just gen-token`.");
        String::new()
    });
    Ok(ShimmerClient::new(base_url, token)?)
}

/// Play a subtle system sound for hotkey success feedback.
#[cfg(target_os = "macos")]
fn play_success_sound() {
    let _ = std::process::Command::new("afplay")
        .arg("/System/Library/Sounds/Tink.aiff")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn play_success_sound() {}

/// Play when we skip (e.g. clipboard has phi link) — so user knows hotkey fired.
#[cfg(target_os = "macos")]
fn play_skip_sound() {
    let _ = std::process::Command::new("afplay")
        .arg("/System/Library/Sounds/Basso.aiff")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn play_skip_sound() {}

/// Build a 32x32 green checkmark icon (RGBA).
fn make_success_icon() -> Image<'static> {
    let (w, h) = (32u32, 32u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let green: [u8; 4] = [76, 217, 100, 255];
    for y in 0..h {
        for x in 0..w {
            let ix = x as i32;
            let iy = y as i32;
            let on_short = (ix - 8 - (iy - 16)).unsigned_abs() < 2
                && (8..=14).contains(&ix)
                && (16..=22).contains(&iy);
            let on_long = (ix - 13 + (iy - 21)).unsigned_abs() < 2
                && (13..=25).contains(&ix)
                && (10..=22).contains(&iy);
            if on_short || on_long {
                let off = ((y * w + x) * 4) as usize;
                rgba[off..off + 4].copy_from_slice(&green);
            }
        }
    }
    Image::new_owned(rgba, w, h)
}

/// Briefly swap the tray icon to a checkmark, then restore.
fn flash_tray_success(handle: &tauri::AppHandle) {
    let Some(tray) = handle.tray_by_id(TRAY_ID) else {
        return;
    };
    let _ = tray.set_icon(Some(make_success_icon()));

    let restore_handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        if let Some(tray) = restore_handle.tray_by_id(TRAY_ID) {
            if let Some(default_icon) = restore_handle.default_window_icon().cloned() {
                let _ = tray.set_icon(Some(default_icon));
            }
        }
    });
}

/// Extract and validate a paste UUID from a raw `phi://` or plain UUID string.
fn parse_paste_id(raw: &str) -> Result<&str, CommandError> {
    let id = raw
        .trim()
        .trim_start_matches("phi://")
        .split('/')
        .next()
        .unwrap_or("")
        .trim();
    if id.is_empty() {
        return Err(CommandError::Validation("Invalid phi:// ID".to_string()));
    }
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(CommandError::Validation("Invalid UUID format".to_string()));
    }
    Ok(id)
}

/// Upload plaintext text: encrypt locally, generate search tokens, ship to server.
#[tauri::command]
async fn paste_upload(
    plaintext: String,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<String, CommandError> {
    if plaintext.trim().starts_with("phi://") {
        return Err(CommandError::Validation(
            "That's a phi link — paste it in Fetch to view the content, not Upload".to_string(),
        ));
    }
    let bytes = plaintext.as_bytes();
    if bytes.len() > shimmer_core::MAX_PASTE_BYTES {
        return Err(CommandError::Validation(format!(
            "Paste too large (max {} MiB)",
            shimmer_core::MAX_PASTE_BYTES / 1024 / 1024
        )));
    }

    let kek = *key_store.read().await.key();

    // Generate blind index search tokens from plaintext content
    let search_key = derive_search_key(&kek);
    let search_tokens = extract_blind_tokens(&plaintext, &search_key);

    // Encrypt locally — ciphertext is JSON-serialized EnvelopePayload
    let envelope = encrypt_envelope(bytes, &kek, "tauri-client")
        .map_err(|e| CommandError::Encryption(e.to_string()))?;
    let ciphertext_json =
        serde_json::to_string(&envelope).map_err(|e| CommandError::Internal(e.to_string()))?;

    let opts = UploadOptions {
        content_type: "text/plain".into(),
        visibility: "org".into(),
        search_tokens,
        ..UploadOptions::default()
    };

    let id = client.read().await.upload(&ciphertext_json, &opts).await?;

    info!(paste_id = %id, size = bytes.len(), "paste uploaded via server");
    Ok(format!("phi://{id}"))
}

/// Upload a file: read from disk, encrypt, generate search tokens, ship to server.
#[tauri::command]
async fn file_upload(
    file_path: String,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<String, CommandError> {
    let path = std::path::Path::new(&file_path);

    // Read file bytes
    let bytes =
        std::fs::read(path).map_err(|e| CommandError::Internal(format!("read file: {e}")))?;

    if bytes.len() > shimmer_core::MAX_FILE_BYTES {
        return Err(CommandError::Validation(format!(
            "File too large (max {} MiB)",
            shimmer_core::MAX_FILE_BYTES / 1024 / 1024
        )));
    }

    let kek = *key_store.read().await.key();
    let search_key = derive_search_key(&kek);

    // Detect content type from extension
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".into());
    let content_type = shimmer_core::content_type_from_extension(&filename).to_string();

    // Generate filename search tokens
    let filename_tokens: Vec<String> = filename
        .split(['.', '_', '-', ' '])
        .filter(|t| t.len() >= 2)
        .map(|t| blind_index_token(&search_key, t))
        .collect();

    // Encrypt the filename (it could contain PHI)
    let filename_envelope = encrypt_envelope(filename.as_bytes(), &kek, "tauri-client")
        .map_err(|e| CommandError::Encryption(e.to_string()))?;
    let filename_encrypted = serde_json::to_string(&filename_envelope)
        .map_err(|e| CommandError::Internal(e.to_string()))?;

    // Encrypt the file content
    let envelope = encrypt_envelope(&bytes, &kek, "tauri-client")
        .map_err(|e| CommandError::Encryption(e.to_string()))?;
    let ciphertext_json =
        serde_json::to_string(&envelope).map_err(|e| CommandError::Internal(e.to_string()))?;

    let opts = UploadOptions {
        content_type,
        visibility: "org".into(),
        filename_encrypted: Some(filename_encrypted),
        filename_tokens,
        ..UploadOptions::default()
    };

    let id = client.read().await.upload(&ciphertext_json, &opts).await?;

    info!(paste_id = %id, file = %filename, size = bytes.len(), "file uploaded via server");
    Ok(format!("phi://{id}"))
}

/// Fetch a paste: retrieve ciphertext from server, decrypt locally.
///
/// Returns raw bytes as a base64-encoded string for binary content,
/// or a UTF-8 string for text content.
#[tauri::command]
async fn paste_fetch(
    id: String,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<String, CommandError> {
    let id = parse_paste_id(&id)?;

    let ciphertext_bytes = client.read().await.fetch(id).await?;

    let envelope: EnvelopePayload = serde_json::from_slice(&ciphertext_bytes)
        .map_err(|e| CommandError::Internal(format!("envelope parse: {e}")))?;

    let kek = *key_store.read().await.key();
    let plaintext =
        decrypt_envelope(&envelope, &kek).map_err(|e| CommandError::Encryption(e.to_string()))?;

    debug!(paste_id = %id, "paste decrypted");

    // Try UTF-8 first (text content), fall back to base64 (binary)
    match String::from_utf8(plaintext.clone()) {
        Ok(text) => Ok(text),
        Err(_) => {
            use base64::Engine;
            Ok(base64::engine::general_purpose::STANDARD.encode(&plaintext))
        }
    }
}

/// Fetch raw bytes of a paste (for file downloads / binary content).
#[tauri::command]
async fn paste_fetch_bytes(
    id: String,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<Vec<u8>, CommandError> {
    let id = parse_paste_id(&id)?;

    let ciphertext_bytes = client.read().await.fetch(id).await?;
    let envelope: EnvelopePayload = serde_json::from_slice(&ciphertext_bytes)
        .map_err(|e| CommandError::Internal(format!("envelope parse: {e}")))?;

    let kek = *key_store.read().await.key();
    let plaintext =
        decrypt_envelope(&envelope, &kek).map_err(|e| CommandError::Encryption(e.to_string()))?;

    debug!(paste_id = %id, size = plaintext.len(), "file decrypted");
    Ok(plaintext)
}

/// List pastes visible to the current user (org + own private).
#[tauri::command]
async fn paste_list(
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
) -> Result<Vec<PasteSummary>, CommandError> {
    client.read().await.list(None).await
}

/// Search pastes by search terms (converted to blind index tokens).
#[tauri::command]
async fn paste_search(
    query: String,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<Vec<PasteSummary>, CommandError> {
    if query.trim().is_empty() {
        return client.read().await.list(None).await;
    }

    let kek = *key_store.read().await.key();
    let search_key = derive_search_key(&kek);

    // Convert search terms to blind index tokens
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .map(|t| blind_index_token(&search_key, t))
        .collect();

    if tokens.is_empty() {
        return client.read().await.list(None).await;
    }

    client.read().await.list(Some(&tokens)).await
}

/// Delete a paste by ID.
#[tauri::command]
async fn paste_delete(
    id: String,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
) -> Result<(), CommandError> {
    let id = parse_paste_id(&id)?;
    client.read().await.delete(id).await?;
    info!(paste_id = %id, "paste deleted via server");
    Ok(())
}

/// Return runtime configuration visible in the Settings UI.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command extractors must be by-value
fn get_settings(
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<serde_json::Value, CommandError> {
    let server_url =
        std::env::var("SHIMMER_SERVER_URL").unwrap_or_else(|_| "http://localhost:8443".into());
    // key_store is behind RwLock; for settings we only need the hex fingerprint
    // We use try_read() here — this is a sync fn so we can't await
    let key_hex = key_store
        .try_read()
        .map(|ks| hex::encode(&ks.key()[..4]))
        .unwrap_or_else(|_| "????".into());
    let token_set = !std::env::var("SHIMMER_JWT").unwrap_or_default().is_empty();

    Ok(serde_json::json!({
        "serverUrl": server_url,
        "keyFingerprint": format!("{}…", key_hex),
        "tokenConfigured": token_set,
        "hotkey": "⌘+⇧+P",
    }))
}

/// Register a new user account using an invite token.
///
/// After successful registration, updates the client bearer token and stores
/// the JWT pair in the keychain.  If `kek_fragment` is present it is used to
/// unwrap the org KEK that was embedded in the invite link.
#[tauri::command]
async fn auth_register(
    invite_token: String,
    email: String,
    password: String,
    name: String,
    kek_fragment: Option<String>,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<(), CommandError> {
    let auth = client
        .read()
        .await
        .register(&invite_token, &email, &password, &name)
        .await?;

    // Update client bearer token
    client.write().await.set_token(auth.access_token.clone());

    // Unwrap KEK from invite fragment if provided
    if let Some(fragment) = kek_fragment {
        let kek = shimmer_core::encryption::unwrap_kek_from_invite(&fragment, &invite_token)
            .map_err(|e| CommandError::Encryption(e.to_string()))?;
        key_store.write().await.set_key(kek);
    }

    // Persist tokens in keychain
    key_store
        .read()
        .await
        .store_tokens(&auth.access_token, &auth.refresh_token);

    info!(user_id = %auth.user_id, "user registered");
    Ok(())
}

/// Authenticate with email and password.
///
/// Updates the client bearer token and stores the JWT pair in the keychain.
#[tauri::command]
async fn auth_login(
    email: String,
    password: String,
    client: tauri::State<'_, Arc<RwLock<ShimmerClient>>>,
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<(), CommandError> {
    let auth = client.read().await.login(&email, &password).await?;

    // Update client bearer token
    client.write().await.set_token(auth.access_token.clone());

    // Persist tokens in keychain
    key_store
        .read()
        .await
        .store_tokens(&auth.access_token, &auth.refresh_token);

    info!(user_id = %auth.user_id, "user logged in");
    Ok(())
}

/// Clear all stored credentials from the keychain (logout).
#[tauri::command]
async fn auth_logout(
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<(), CommandError> {
    key_store.read().await.clear_all();
    info!("user logged out");
    Ok(())
}

/// Check whether valid JWT tokens are stored in the keychain.
#[tauri::command]
async fn auth_status(
    key_store: tauri::State<'_, Arc<RwLock<KeyStore>>>,
) -> Result<bool, CommandError> {
    let authenticated = key_store.read().await.load_tokens().is_some();
    Ok(authenticated)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used)] // Startup code — crash on misconfiguration is correct
pub fn run() {
    // Structured logging: RUST_LOG env filter, pretty output for dev
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("shimmer starting");

    let client = Arc::new(RwLock::new(build_client().expect("build shimmer client")));

    let shortcut_handler =
        move |app: &tauri::AppHandle<_>,
              _shortcut: &tauri_plugin_global_shortcut::Shortcut,
              _event: tauri_plugin_global_shortcut::ShortcutEvent| {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let clipboard = handle.clipboard();
                if let Ok(text) = clipboard.read_text() {
                    let trimmed = text.trim();
                    if trimmed.starts_with("phi://") {
                        play_skip_sound();
                        return;
                    }
                    if trimmed.is_empty() {
                        return;
                    }
                    if text.len() <= shimmer_core::MAX_PASTE_BYTES {
                        if let (Some(client), Some(key_store)) = (
                            handle.try_state::<Arc<RwLock<ShimmerClient>>>(),
                            handle.try_state::<Arc<RwLock<KeyStore>>>(),
                        ) {
                            let kek = *key_store.read().await.key();

                            // Generate search tokens from clipboard content
                            let search_key = derive_search_key(&kek);
                            let search_tokens = extract_blind_tokens(&text, &search_key);

                            if let Ok(envelope) =
                                encrypt_envelope(text.as_bytes(), &kek, "tauri-client")
                            {
                                if let Ok(ciphertext_json) = serde_json::to_string(&envelope) {
                                    let opts = UploadOptions {
                                        content_type: "text/plain".into(),
                                        visibility: "org".into(),
                                        search_tokens,
                                        ..UploadOptions::default()
                                    };
                                    if let Ok(id) =
                                        client.read().await.upload(&ciphertext_json, &opts).await
                                    {
                                        let url = format!("phi://{id}");
                                        let _ = clipboard.write_text(url);
                                        let _ = handle.emit("phi-paste-success", ());
                                        play_success_sound();
                                        flash_tray_success(&handle);
                                    }
                                }
                            }
                        }
                    }
                }
            });
        };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(shortcut_handler)
                .with_shortcut("CommandOrControl+Shift+P")
                .expect("register shortcut")
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(client)
        .invoke_handler(tauri::generate_handler![
            paste_upload,
            paste_fetch,
            paste_fetch_bytes,
            paste_list,
            paste_search,
            paste_delete,
            file_upload,
            get_settings,
            auth_register,
            auth_login,
            auth_logout,
            auth_status,
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().ok();
            let key_store = Arc::new(RwLock::new(KeyStore::load_or_create(
                app_data_dir.as_deref(),
            )));
            app.manage(key_store);

            let quit = MenuItem::with_id(app, "quit", "Quit Shimmer", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let menu = Menu::new(app)?;
            menu.append(&quit)?;
            menu.append(&settings)?;

            let default_icon = app
                .default_window_icon()
                .ok_or_else(|| anyhow::anyhow!("no default window icon configured"))?
                .clone();
            let _tray = TrayIconBuilder::with_id(TRAY_ID)
                .icon(default_icon)
                .menu(&menu)
                .on_menu_event(move |app, event| {
                    if event.id.as_ref() == "quit" {
                        app.exit(0);
                    } else if event.id.as_ref() == "settings" {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Protect main window from screenshots (displays PHI)
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_content_protected(true);
            }

            // Background update check on startup
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match tauri_plugin_updater::UpdaterExt::updater(&handle) {
                    Ok(updater) => match updater.check().await {
                        Ok(Some(update)) => {
                            tracing::info!(version = %update.version, "update available");
                        }
                        Ok(None) => tracing::debug!("no update available"),
                        Err(e) => tracing::warn!(error = %e, "update check failed"),
                    },
                    Err(e) => tracing::debug!(error = %e, "updater not configured"),
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
