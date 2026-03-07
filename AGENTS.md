# AGENTS.md — Shimmer

Cross-platform instructions for AI coding agents.

---

## Quick Reference

```yaml
Language:    Rust (stable, edition 2021)
Framework:   Tauri v2 (tray-only desktop app)
Frontend:    SvelteKit + Svelte 5
Async:       Tokio
Errors:      thiserror (typed CommandError) + anyhow (internal propagation)
Logging:     tracing + tracing-subscriber
Testing:     built-in #[test] + #[tokio::test]
Linting:     clippy
Formatter:   rustfmt
Git Hooks:   Lefthook
Tasks:       Just
```

---

## Commands

```bash
# Development
just dev                   # Run with file storage (default)
just dev-s3                # Run with MinIO (requires `just up`)
just dev-ui                # Frontend only (no Rust rebuild)
just build                 # Production build

# Quality
just check                 # Run all checks (fmt, clippy, test)
just clippy                # cargo clippy -- -D warnings
just fmt                   # cargo fmt
just fmt-check             # cargo fmt -- --check
just ci                    # All CI checks (fmt, clippy, test, typecheck)

# Testing
just test                  # cargo test
just test-v                # cargo test -- --nocapture

# Infrastructure
just up                    # Start MinIO
just down                  # Stop MinIO
```

---

## Error Handling

Commands use a typed `CommandError` enum (in `src-tauri/src/error.rs`):

```rust
#[tauri::command]
async fn my_command(...) -> Result<T, CommandError> {
    // Use CommandError variants, NOT .map_err(|e| e.to_string())
}
```

**Variants**: `NotFound`, `Validation`, `Storage`, `Encryption`, `Internal`

**Rules**:
- Command errors must impl `Serialize` — never use `String` as the error type.
- Use `thiserror` for typed errors, `anyhow` for internal propagation via `From<anyhow::Error>`.
- No `.unwrap()` in commands — return `CommandError` instead.

---

## Logging

Use `tracing` macros everywhere. Never use `println!` or `eprintln!`.

```rust
use tracing::{info, warn, error, debug};

info!(id = %id, size = bytes.len(), "paste uploaded");
warn!(error = %e, "could not persist key");
error!(error = ?err, "storage operation failed");
```

Set log level via `RUST_LOG` env var (default: `info`).

---

## Project Structure

```
src/                         # SvelteKit frontend
└── routes/+page.svelte      # Main UI (tabs: Paste, Browse, Settings)

src-tauri/                   # Rust backend
├── Cargo.toml
├── tauri.conf.json
├── capabilities/default.json
└── src/
    ├── main.rs              # Entry point
    ├── lib.rs               # App builder, commands, tray setup
    ├── error.rs             # CommandError enum (thiserror + Serialize)
    ├── encryption.rs        # AES-256-GCM encrypt/decrypt
    ├── storage.rs           # Storage trait + S3/File implementations
    └── key_store.rs         # Encryption key persistence
```

---

## Critical Rules

### Always

- Command errors must `impl Serialize` — use typed `CommandError`, not strings.
- Use `tracing` for all logging.
- Keep heavy async work in Rust; the frontend only calls `invoke()`.
- Grant only minimum permissions in `capabilities/`.
- Run `just check` before committing.

### Never

- Use `String` as the error type in Tauri commands.
- Use `std::thread::sleep` in async context — use `tokio::time::sleep`.
- Use `Mutex::lock()` across `.await` — use `tokio::sync::Mutex`.
- Use `println!` / `eprintln!` — use `tracing` macros.
- Put secrets in `tauri.conf.json` — it ships with the binary.

### Ask First

- Adding new Tauri plugins (require capability permission entries).
- Changing the app identifier — affects OS-level data paths.
- Modifying the encryption scheme — breaks existing encrypted pastes.
