# AGENTS.md — Shimmer

Cross-platform instructions for AI coding agents.

---

## Quick Reference

```yaml
Language:    Rust (stable, edition 2021)
Workspace:   Cargo workspace (3 crates)
Framework:   Tauri v2 (tray-only desktop app)
Server:      Axum 0.8 (API gateway)
Frontend:    SvelteKit + Svelte 5
Async:       Tokio
Errors:      thiserror (typed) + anyhow (internal propagation)
Logging:     tracing + tracing-subscriber (JSON in prod)
Validation:  validator (derive-based field validation)
Testing:     #[test], #[tokio::test], nextest, axum-test, proptest, insta
Linting:     clippy (workspace-wide deny policy via [lints])
Formatter:   rustfmt (rustfmt.toml)
Git Hooks:   Lefthook
Tasks:       Just
Security:    cargo-deny (licenses, advisories, duplicates)
```

---

## Workspace Structure

```
shimmer/
├── Cargo.toml                 # Workspace root (shared deps, [lints], profiles)
├── deny.toml                  # cargo-deny policy
├── rustfmt.toml               # Formatting rules
├── justfile                   # Task runner
├── lefthook.yml               # Git hooks
├── .env.example               # Env var documentation
│
├── shimmer-core/              # Shared library (encryption, storage, errors)
│   └── src/
│       ├── lib.rs             # Exports + constants (MAX_PASTE_BYTES, KEY_LEN)
│       ├── encryption.rs      # Envelope encryption (AES-256-GCM) + blind index SSE
│       ├── error.rs           # CryptoError + StorageError (thiserror)
│       └── storage.rs         # Storage trait + S3/File impls
│
├── shimmer-server/            # Axum API gateway (zero-knowledge)
│   ├── src/
│   │   ├── lib.rs             # AppState + build_router() for testability
│   │   ├── main.rs            # Entrypoint (logging, config, storage init)
│   │   ├── auth.rs            # JWT claims + FromRequestParts extractor
│   │   ├── config.rs          # TOML config + env var overlay
│   │   └── routes/
│   │       ├── mod.rs         # Route tree
│   │       └── paste.rs       # CRUD handlers + validator
│   └── tests/
│       └── api_test.rs        # Integration tests (axum-test + tempdir)
│
├── src-tauri/                 # Tauri desktop client
│   └── src/
│       ├── main.rs            # Entry point
│       ├── lib.rs             # Commands, tray, hotkey handler
│       ├── error.rs           # CommandError (thiserror + Serialize for IPC)
│       └── key_store.rs       # KEK persistence (OS keychain / file fallback)
│
└── src/                       # SvelteKit frontend
    └── routes/+page.svelte    # Main UI (tabs: Paste, Browse, Settings)
```

---

## Commands

```bash
# Development
just dev                   # Run desktop app (file storage, no setup)
just dev-s3                # Run with MinIO (requires `just up`)
just dev-server            # Run API server locally
just dev-ui                # Frontend only (no Rust rebuild)

# Quality
just check                 # fmt-check + clippy + test (pre-commit gate)
just ci                    # check + typecheck (full CI pipeline)
just clippy                # cargo clippy --workspace -- -D warnings
just fmt                   # cargo fmt --all
just check-fast            # cargo check --workspace (fastest feedback)

# Testing
just test                  # cargo nextest run (or cargo test)
just test-v                # With output (--nocapture)
just test-core             # shimmer-core only
just test-server           # shimmer-server only
just test-integration      # Integration tests only
just test-filter <pattern> # Run matching tests

# Security
just audit                 # cargo audit (CVE scan)
just deny                  # cargo deny check (licenses + advisories)

# Infrastructure
just up                    # Start MinIO
just down                  # Stop MinIO
just gen-key               # Generate a 256-bit hex key
```

---

## Error Handling

**Client (src-tauri)**: Commands use typed `CommandError` enum:

```rust
#[tauri::command]
async fn my_command(...) -> Result<T, CommandError> {
    // Variants: NotFound, Validation, Storage, Encryption, Internal
}
```

**Core (shimmer-core)**: Domain errors `CryptoError` and `StorageError` (thiserror).

**Server (shimmer-server)**: Returns `(StatusCode, String)` tuples. Validates requests with `validator`.

**Rules**:
- Command errors must impl `Serialize` — never use `String` as the error type.
- Use `thiserror` for typed errors, `anyhow` for internal propagation.
- No `.unwrap()` in commands — return appropriate error variant.
- No `expect()` outside of startup/setup code.

---

## Logging

Use `tracing` macros everywhere. Never use `println!` or `eprintln!`.

```rust
use tracing::{info, warn, error, debug};

info!(id = %id, size = bytes.len(), "paste uploaded");
warn!(error = %e, "could not persist key");
error!(error = ?err, "storage operation failed");
```

- Dev: human-readable output (default)
- Prod: `LOG_FORMAT=json` for machine-parseable structured JSON
- Filter: `RUST_LOG` env var (default: `info`)

---

## Serde Conventions

API request/response types use:
- `#[serde(rename_all = "camelCase")]` — consistent JSON naming
- `#[serde(deny_unknown_fields)]` — reject unexpected fields on requests
- `#[serde(default)]` — optional fields with sensible defaults
- `validator` derive for field-level validation (length, range, custom)

---

## Critical Rules

### Always

- Use workspace dependencies (`{ workspace = true }`) for shared crates.
- Command errors must `impl Serialize` — use typed `CommandError`, not strings.
- Use `tracing` for all logging.
- Validate API inputs with `validator` before processing.
- Keep heavy async work in Rust; the frontend only calls `invoke()`.
- Run `just check` before committing.

### Never

- Use `String` as the error type in Tauri commands.
- Use `std::thread::sleep` in async context — use `tokio::time::sleep`.
- Use `Mutex::lock()` across `.await` — use `tokio::sync::Mutex`.
- Use `println!` / `eprintln!` — use `tracing` macros.
- Put secrets in `tauri.conf.json` — it ships with the binary.
- Use `.unwrap()` / `.expect()` in library or handler code.

### Ask First

- Adding new Tauri plugins (require capability permission entries).
- Changing the app identifier — affects OS-level data paths.
- Modifying the encryption scheme — breaks existing encrypted pastes.
- Adding new workspace crates.
