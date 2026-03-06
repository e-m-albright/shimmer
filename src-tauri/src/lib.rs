//! Shimmer - PHI sharing utility. Tray-only, invisible in workflow.
//!
//! TODO(OIDC): Integrate JumpCloud OIDC for SSO. Use dev user id until then.

mod encryption;
mod key_store;
mod storage;

use std::sync::Arc;
use tauri::{image::Image, menu::{Menu, MenuItem}, tray::TrayIconBuilder, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

const TRAY_ID: &str = "shimmer-tray";

use encryption::{decrypt, encrypt};
use key_store::KeyStore;
use storage::{create_storage, PasteEntry, Storage};

/// Max paste size: 1 MiB to prevent abuse and memory issues.
const MAX_PASTE_BYTES: usize = 1024 * 1024;

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
fn play_success_sound() {
    // Event to frontend handles sound on other platforms
}

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
            // Short arm: (8,16)→(14,22), Long arm: (13,21)→(25,10)
            let on_short = (ix - 8 - (iy - 16)).unsigned_abs() < 2 && (8..=14).contains(&ix) && (16..=22).contains(&iy);
            let on_long = (ix - 13 + (iy - 21)).unsigned_abs() < 2 && (13..=25).contains(&ix) && (10..=22).contains(&iy);
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
    let Some(tray) = handle.tray_by_id(TRAY_ID) else { return };
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

#[tauri::command]
async fn paste_upload(
    plaintext: String,
    storage: tauri::State<'_, Arc<dyn Storage>>,
    key_store: tauri::State<'_, Arc<KeyStore>>,
) -> Result<String, String> {
    let trimmed = plaintext.trim();
    if trimmed.starts_with("phi://") {
        return Err("That's a phi link — paste it in Fetch to view the content, not Upload".to_string());
    }
    let bytes = plaintext.as_bytes();
    if bytes.len() > MAX_PASTE_BYTES {
        return Err(format!("Paste too large (max {} MiB)", MAX_PASTE_BYTES / 1024 / 1024));
    }

    let key = key_store.key();
    let encrypted = encrypt(bytes, key).map_err(|e| e.to_string())?;

    let id = uuid::Uuid::new_v4().to_string();
    storage.put(&id, encrypted.as_bytes()).await.map_err(|e| e.to_string())?;

    Ok(format!("phi://{}", id))
}

#[tauri::command]
async fn paste_fetch(
    id: String,
    storage: tauri::State<'_, Arc<dyn Storage>>,
    key_store: tauri::State<'_, Arc<KeyStore>>,
) -> Result<String, String> {
    let id = id.trim().trim_start_matches("phi://").split('/').next().unwrap_or("").trim();
    if id.is_empty() {
        return Err("Invalid phi:// ID".to_string());
    }
    if uuid::Uuid::parse_str(id).is_err() {
        return Err("Invalid UUID format".to_string());
    }

    let key = key_store.key();
    let encrypted = storage.get(id).await.map_err(|e| e.to_string())?;
    let encoded = String::from_utf8(encrypted).map_err(|e| e.to_string())?;
    let decrypted = decrypt(&encoded, key).map_err(|e| e.to_string())?;
    String::from_utf8(decrypted).map_err(|e| e.to_string())
}

#[tauri::command]
async fn paste_list(
    storage: tauri::State<'_, Arc<dyn Storage>>,
) -> Result<Vec<PasteEntry>, String> {
    storage.list().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn paste_delete(
    id: String,
    storage: tauri::State<'_, Arc<dyn Storage>>,
) -> Result<(), String> {
    let id = id.trim().trim_start_matches("phi://").split('/').next().unwrap_or("").trim();
    if id.is_empty() {
        return Err("Invalid ID".to_string());
    }
    storage.delete(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings(
    key_store: tauri::State<'_, Arc<KeyStore>>,
) -> Result<serde_json::Value, String> {
    let storage_type = if std::env::var("SHIMMER_USE_FILE_STORAGE").ok().as_deref() == Some("1")
        || std::env::var("SHIMMER_S3_ENDPOINT").is_err()
    {
        "file"
    } else {
        "s3"
    };
    let prefix = std::env::var("SHIMMER_USER_PREFIX").unwrap_or_else(|_| "dev-user".into());
    let key_hex = hex::encode(&key_store.key()[..4]);

    Ok(serde_json::json!({
        "storageType": storage_type,
        "storagePath": std::env::var("SHIMMER_STORAGE_PATH").unwrap_or_else(|_| "./shimmer-dev-storage".into()),
        "s3Endpoint": std::env::var("SHIMMER_S3_ENDPOINT").unwrap_or_default(),
        "s3Bucket": std::env::var("SHIMMER_S3_BUCKET").unwrap_or_else(|_| "shimmer".into()),
        "userPrefix": prefix,
        "keyFingerprint": format!("{}…", key_hex),
        "hotkey": "⌘+⇧+P",
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let storage = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(create_storage())
        .expect("create storage");

    let shortcut_handler = move |app: &tauri::AppHandle<_>, _shortcut: &tauri_plugin_global_shortcut::Shortcut, _event: tauri_plugin_global_shortcut::ShortcutEvent| {
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
                if text.as_bytes().len() <= MAX_PASTE_BYTES {
                    if let (Some(storage), Some(key_store)) = (
                        handle.try_state::<Arc<dyn Storage>>(),
                        handle.try_state::<Arc<KeyStore>>(),
                    ) {
                        let key = key_store.key();
                        if let Ok(encrypted) = encrypt(text.as_bytes(), key) {
                            let id = uuid::Uuid::new_v4().to_string();
                            if storage.put(&id, encrypted.as_bytes()).await.is_ok() {
                                let url = format!("phi://{}", id);
                                let _ = clipboard.write_text(url);
                                let _ = handle.emit("phi-paste-success", ());
                                play_success_sound();
                                flash_tray_success(&handle);
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
        .manage(storage)
        .invoke_handler(tauri::generate_handler![paste_upload, paste_fetch, paste_list, paste_delete, get_settings])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().ok();
            let key_store = Arc::new(KeyStore::load_or_create(app_data_dir.as_deref()));
            app.manage(key_store);

            let quit = MenuItem::with_id(app, "quit", "Quit Shimmer", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let menu = Menu::new(app)?;
            menu.append(&quit)?;
            menu.append(&settings)?;

            let _tray = TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().unwrap().clone())
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
