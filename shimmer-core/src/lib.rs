//! shimmer-core — shared encryption, storage, and error types.
//!
//! Used by both the Tauri desktop client and the Axum API server.

pub mod encryption;
pub mod error;
pub mod storage;

/// Max paste size for text: 1 MiB.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;

/// Max file size: 25 MiB (before encryption overhead).
pub const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;

/// Key length for AES-256.
pub const KEY_LEN: usize = 32;

/// Infer content type from file extension.
///
/// Returns a MIME type string. Defaults to `application/octet-stream` for
/// unknown extensions.
#[must_use]
pub fn content_type_from_extension(filename: &str) -> &'static str {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "heic" | "heif" => "image/heic",
        // Documents
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "csv" => "text/csv",
        // Text
        "txt" => "text/plain",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        // Archives
        "zip" => "application/zip",
        // Screenshots (common macOS format)
        "psd" => "image/vnd.adobe.photoshop",
        _ => "application/octet-stream",
    }
}
