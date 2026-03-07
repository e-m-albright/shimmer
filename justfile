# =============================================================================
# Justfile — Shimmer (Cargo workspace + Tauri v2 + SvelteKit)
# =============================================================================
# Run `just` to see all available commands
# =============================================================================

# Default recipe: show help
default:
    @just --list

# -----------------------------------------------------------------------------
# Development
# -----------------------------------------------------------------------------

# Run Shimmer desktop app (file storage by default — no setup needed)
dev:
    npm run tauri dev

# Run Shimmer with MinIO (requires `just up` first)
dev-s3:
    SHIMMER_S3_ENDPOINT=http://localhost:9000 \
    SHIMMER_S3_BUCKET=shimmer \
    AWS_ACCESS_KEY_ID=minioadmin \
    AWS_SECRET_ACCESS_KEY=minioadmin \
    npm run tauri dev

# Run shimmer-server locally (file storage)
dev-server:
    cargo run -p shimmer-server

# Run shimmer-server with MinIO (requires `just up` first)
dev-server-s3:
    SHIMMER_S3_ENDPOINT=http://localhost:9000 \
    SHIMMER_S3_BUCKET=shimmer \
    AWS_ACCESS_KEY_ID=minioadmin \
    AWS_SECRET_ACCESS_KEY=minioadmin \
    cargo run -p shimmer-server

# Start frontend only (faster, no Rust rebuild)
dev-ui:
    npm run dev

# Build for production
build:
    npm run build
    npm run tauri build

# Build frontend only
build-ui:
    npm run build

# Build server binary (release)
build-server:
    cargo build -p shimmer-server --release

# -----------------------------------------------------------------------------
# Quality
# -----------------------------------------------------------------------------

# Run all checks (fmt, clippy, test) — use before committing
check: fmt-check clippy test

# Lint all Rust crates (treat warnings as errors)
clippy:
    cargo clippy --workspace -- -D warnings

# Fix clippy warnings automatically
clippy-fix:
    cargo clippy --workspace --fix --allow-dirty -- -D warnings

# Format all Rust code
fmt:
    cargo fmt --all

# Check Rust formatting (for CI)
fmt-check:
    cargo fmt --all -- --check

# Type-check frontend
typecheck:
    npm run check

# Run all CI checks (fmt, clippy, test, typecheck)
ci: fmt-check clippy test typecheck

# Check for compile errors without producing output (fastest feedback)
check-fast:
    cargo check --workspace

# -----------------------------------------------------------------------------
# Testing
# -----------------------------------------------------------------------------

# Run all Rust tests (uses nextest if installed, falls back to cargo test)
test:
    @command -v cargo-nextest >/dev/null 2>&1 \
        && cargo nextest run --workspace \
        || cargo test --workspace

# Run tests with output (see println/tracing)
test-v:
    @command -v cargo-nextest >/dev/null 2>&1 \
        && cargo nextest run --workspace --nocapture \
        || cargo test --workspace -- --nocapture

# Run specific test by name
test-filter pattern:
    @command -v cargo-nextest >/dev/null 2>&1 \
        && cargo nextest run --workspace -E 'test({{pattern}})' \
        || cargo test --workspace {{pattern}} -- --nocapture

# Run tests for a specific crate
test-crate crate:
    @command -v cargo-nextest >/dev/null 2>&1 \
        && cargo nextest run -p {{crate}} \
        || cargo test -p {{crate}} -- --nocapture

# Run only shimmer-core tests
test-core:
    @command -v cargo-nextest >/dev/null 2>&1 \
        && cargo nextest run -p shimmer-core \
        || cargo test -p shimmer-core -- --nocapture

# Run only shimmer-server tests
test-server:
    @command -v cargo-nextest >/dev/null 2>&1 \
        && cargo nextest run -p shimmer-server \
        || cargo test -p shimmer-server -- --nocapture

# Run integration tests only
test-integration:
    @command -v cargo-nextest >/dev/null 2>&1 \
        && cargo nextest run --workspace -E 'kind(test)' \
        || cargo test --workspace --test '*' -- --nocapture

# -----------------------------------------------------------------------------
# Dependencies
# -----------------------------------------------------------------------------

# Install all dependencies and dev tools
install:
    npm install
    @echo "Installing Rust dev tools..."
    @command -v cargo-nextest >/dev/null 2>&1 || cargo install cargo-nextest
    @command -v cargo-deny    >/dev/null 2>&1 || cargo install cargo-deny
    @command -v cargo-audit   >/dev/null 2>&1 || cargo install cargo-audit

# Update all Rust dependencies
update-rust:
    cargo update

# Update frontend dependencies
update-ui:
    npm update

# Update all
update: update-rust update-ui

# Security audit (Rust) — install with `cargo install cargo-audit`
audit:
    cargo audit

# License + advisory + duplicate dep check — install with `cargo install cargo-deny`
deny:
    cargo deny check

# Show dependency tree
deps:
    cargo tree --workspace

# Show duplicate dependencies
deps-dupes:
    cargo tree --workspace --duplicates

# -----------------------------------------------------------------------------
# Infrastructure (MinIO)
# -----------------------------------------------------------------------------

# Start MinIO for S3-compatible storage
up:
    @echo "Starting MinIO..."
    docker run -d --name shimmer-minio \
        -p 9000:9000 -p 9001:9001 \
        -e MINIO_ROOT_USER=minioadmin \
        -e MINIO_ROOT_PASSWORD=minioadmin \
        minio/minio server /data --console-address ":9001"
    @echo "MinIO: API http://localhost:9000  Console http://localhost:9001"
    just s3-bucket

# Create the shimmer bucket (run after `just up` if bucket wasn't auto-created)
s3-bucket:
    @echo "Creating bucket 'shimmer'..."
    @sleep 2
    docker run --rm --add-host=host.docker.internal:host-gateway \
        -e MC_HOST_minio=http://minioadmin:minioadmin@host.docker.internal:9000 \
        minio/mc mb minio/shimmer --ignore-existing
    @echo "Bucket ready. Run: just dev-s3"

# Stop infrastructure
down:
    docker stop shimmer-minio 2>/dev/null || true
    docker rm shimmer-minio 2>/dev/null || true
    @echo "MinIO stopped"

# -----------------------------------------------------------------------------
# Git Hooks
# -----------------------------------------------------------------------------

# Install git hooks
hooks-install:
    lefthook install

# Run pre-commit hooks manually
hooks-run:
    lefthook run pre-commit

# Run pre-push hooks manually
hooks-push:
    lefthook run pre-push

# -----------------------------------------------------------------------------
# Utilities
# -----------------------------------------------------------------------------

# Clean all build artifacts
clean:
    cargo clean
    rm -rf node_modules/.vite

# Show workspace member crates
workspace:
    @echo "Workspace members:"
    @cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | "  \(.name) v\(.version) — \(.description // "no description")"'

# Generate a dev encryption key (hex-encoded, for SHIMMER_DEV_KEY)
gen-key:
    @openssl rand -hex 32
