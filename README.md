# Shimmer

PHI sharing utility — secure, short-lived clipboard URLs. Tray-only, invisible in workflow.

## Quick Start (No AWS, No MinIO)

```bash
# Use file storage — no S3/MinIO needed
export SHIMMER_USE_FILE_STORAGE=1

npm install
npm run tauri dev
```

Data is stored in `./shimmer-dev-storage/`. Copy a phi:// link from the Settings window or use the hotkey.

## Hotkey

**⌘+⇧+P** (Cmd+Shift+P) — Captures clipboard text, encrypts it, uploads, and copies the `phi://<id>` link to your clipboard.

> **Note:** If Cmd+Shift+P is used by another app (e.g. Cursor's command palette), close that app or change its shortcut. On macOS, grant **Accessibility** permission (System Settings → Privacy & Security → Accessibility) if the hotkey doesn't work. If you hear a low "bonk" (Basso), the clipboard had a phi link — paste it in Fetch instead.

## Testing with MinIO (S3-compatible)

```bash
# Start MinIO
docker run -d -p 9000:9000 -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin -e MINIO_ROOT_PASSWORD=minioadmin \
  minio/minio server /data --console-address ":9001"

# Create bucket (MinIO console at http://localhost:9001 or mc)
# Then:
export SHIMMER_S3_ENDPOINT=http://localhost:9000
export SHIMMER_S3_BUCKET=shimmer
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin

npm run tauri dev
```

## Env Vars

| Variable | Description |
|----------|-------------|
| `SHIMMER_USE_FILE_STORAGE` | Set to `1` for local file storage (no S3) |
| `SHIMMER_STORAGE_PATH` | Base path for file storage (default: `./shimmer-dev-storage`) |
| `SHIMMER_S3_ENDPOINT` | S3 endpoint (e.g. `http://localhost:9000` for MinIO) |
| `SHIMMER_S3_BUCKET` | S3 bucket name (default: `shimmer`) |
| `SHIMMER_USER_PREFIX` | Key prefix for user (default: `dev-user`) |
| `SHIMMER_DEV_KEY` | 64-char hex encryption key (optional; auto-persisted to app data dir if unset) |

**Key persistence:** When `SHIMMER_DEV_KEY` is not set, the encryption key is generated on first run and stored in your app data directory (`~/Library/Application Support/com.shimmer.app/` on macOS). This ensures pastes remain decryptable across app restarts.

## TODO

- **OIDC/JumpCloud SSO** — Replace dev user with real SSO. Derive encryption key from session.
- **Screenshot capture** — Cmd+Shift+S for region screenshot.

## Tech

- **Tauri v2** + **Svelte** (SvelteKit + adapter-static)
- **AES-256-GCM** encryption (aes-gcm crate)
- **S3/MinIO** or **file** storage
- **phi://** protocol for deep links
# shimmer
