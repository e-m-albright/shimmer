# =============================================================================
# Justfile — Shimmer (Tauri v2 + SvelteKit)
# =============================================================================
# Run `just` to see all available commands
# =============================================================================

# Default recipe: show help
default:
    @just --list

# -----------------------------------------------------------------------------
# Development
# -----------------------------------------------------------------------------

# Run Shimmer (file storage by default — no setup needed)
dev:
    npm run tauri dev

# Run Shimmer with MinIO (requires `just up` first)
dev-s3:
    SHIMMER_S3_ENDPOINT=http://localhost:9000 \
    SHIMMER_S3_BUCKET=shimmer \
    AWS_ACCESS_KEY_ID=minioadmin \
    AWS_SECRET_ACCESS_KEY=minioadmin \
    npm run tauri dev

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

# -----------------------------------------------------------------------------
# Quality
# -----------------------------------------------------------------------------

# Run all checks (fmt, clippy, test)
check: fmt-check clippy test

# Lint Rust (treat warnings as errors)
clippy:
    cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

# Fix clippy warnings automatically
clippy-fix:
    cargo clippy --manifest-path src-tauri/Cargo.toml --fix -- -D warnings

# Format Rust
fmt:
    cargo fmt --manifest-path src-tauri/Cargo.toml

# Check Rust formatting (for CI)
fmt-check:
    cargo fmt --manifest-path src-tauri/Cargo.toml -- --check

# Type-check frontend
typecheck:
    npm run check

# Run all CI checks
ci: fmt-check clippy test typecheck

# -----------------------------------------------------------------------------
# Testing
# -----------------------------------------------------------------------------

# Run Rust tests
test:
    cargo test --manifest-path src-tauri/Cargo.toml

# Run Rust tests with output
test-v:
    cargo test --manifest-path src-tauri/Cargo.toml -- --nocapture

# Run specific test
test-filter pattern:
    cargo test --manifest-path src-tauri/Cargo.toml {{pattern}} -- --nocapture

# -----------------------------------------------------------------------------
# Dependencies
# -----------------------------------------------------------------------------

# Install frontend dependencies
install:
    npm install

# Update Rust dependencies
update-rust:
    cargo update --manifest-path src-tauri/Cargo.toml

# Update frontend dependencies
update-ui:
    npm update

# Update all
update: update-rust update-ui

# Security audit (Rust)
audit:
    cargo audit

# Check for compile errors without producing output (fastest check)
check-fast:
    cargo check --manifest-path src-tauri/Cargo.toml

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
