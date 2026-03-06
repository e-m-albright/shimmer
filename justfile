# Shimmer — PHI sharing utility
# just --list

# Default recipe: show help
default:
    @just --list

# Start infrastructure (MinIO for S3-compatible storage)
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

# Run Shimmer (file storage by default — no `just up` needed)
dev:
    npm run tauri dev

# Run Shimmer with MinIO (requires `just up` first)
dev-s3:
    SHIMMER_S3_ENDPOINT=http://localhost:9000 \
    SHIMMER_S3_BUCKET=shimmer \
    AWS_ACCESS_KEY_ID=minioadmin \
    AWS_SECRET_ACCESS_KEY=minioadmin \
    npm run tauri dev

# Build for production
build:
    npm run build
    npm run tauri build

# Install dependencies
install:
    npm install
