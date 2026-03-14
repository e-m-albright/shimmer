# Shimmer Production Readiness — Design Spec

**Date:** 2026-03-13
**Goal:** Make Shimmer downloadable, installable, and distributable to coworkers at healthcare orgs — no dev tooling required, no BAAs with third parties.

**Model:** Admin-hosted. One person deploys the server, distributes the desktop app + invite links to coworkers.

**Platform:** macOS only for v1.

---

## 1. Server Setup TUI

`shimmer-server setup` launches a Ratatui-based TUI wizard that walks the admin through first-time configuration.

**Steps:**
1. Storage backend — file (pick a path) or S3 (endpoint, bucket, credentials)
2. JWT secret — auto-generate, save to config (never displayed in full; show fingerprint only)
3. Org creation — name the org, create the org record in DB. **The KEK is NOT generated here** — it is generated client-side when the admin first opens the desktop app (preserving zero-knowledge; the server never sees the KEK).
4. Admin account — email + password for the first user
5. SMTP config — server/port/credentials for invite emails (optional; skip = CLI-only invites)
6. Write config — saves `shimmer.toml` with restrictive permissions (0600), prints summary

**Output:** Server is ready to run. TUI prints a one-time admin invite link. Admin opens the desktop app, clicks/pastes the link, and the app generates + stores the org KEK locally.

**Config file permissions:** `shimmer.toml` is created with mode 0600 (owner read/write only) since it contains the JWT secret and SMTP credentials. A warning comment is included at the top of the generated file.

---

## 2. Authentication System

Replaces dev JWTs with real email/password auth.

### Data Model

**`users` table** — credentials only:
- `id` (TEXT PRIMARY KEY, UUID)
- `email` (TEXT UNIQUE NOT NULL)
- `password_hash` (TEXT NOT NULL, argon2)
- `created_at` (TEXT NOT NULL, ISO 8601)

**`members` table** — org membership (already exists, unchanged):
- `id`, `org_id`, `user_id` (FK → users.id), `name`, `role`, `joined_at`

**Relationship:** A user has one `users` row for credentials and one `members` row per org. `role` lives only in `members`. JWT claims are derived by joining `users` + `members` at login/refresh time.

**`refresh_tokens` table** — new:
- `id` (TEXT PRIMARY KEY, UUID)
- `user_id` (TEXT NOT NULL, FK → users.id)
- `token_hash` (TEXT NOT NULL, SHA-256 of opaque token)
- `expires_at` (TEXT NOT NULL, ISO 8601)
- `created_at` (TEXT NOT NULL, ISO 8601)

Refresh tokens are opaque random strings. Only the hash is stored server-side. Each refresh rotates: old token is deleted, new one issued. Revoking a member deletes their refresh tokens, forcing re-auth.

### Server-side Endpoints

- `POST /api/auth/register` — called during invite redemption. Takes: invite token, email, password. Creates `users` row + `members` row. Returns JWT + refresh token. **Replaces** the existing `POST /api/org/join` endpoint and its `JoinRequest` struct entirely (new struct with `token`, `email`, `password` fields instead of `token`, `name`).
- `POST /api/auth/login` — email/password → JWT (1hr) + refresh token (30 days)
- `POST /api/auth/refresh` — opaque refresh token → new JWT + new refresh token (rotation)
- Existing JWT extractor (`FromRequestParts`) stays the same — it validates signatures and reads claims. Claims now come from real user records instead of dev-generated tokens.

### Client-side

- First launch with no credentials → login/invite screen (not main tabs)
- JWT + refresh token stored in macOS keychain via `tauri-plugin-keychain` or `security-framework` crate
- Auto-refresh transparently; user never sees token expiry
- Logout clears keychain entries + KEK

### Migration from dev auth

- `SHIMMER_JWT` and `SHIMMER_DEV_KEY` env vars remain as dev-only overrides (skips login, skips keychain). They are not documented for production use.
- Existing `key_store.rs` file-based storage (`key.bin`) is replaced by keychain storage. On first launch after upgrade, if `key.bin` exists but no keychain entry, migrate the key to keychain and delete `key.bin`.
- The existing `POST /api/org/join` endpoint is removed. The invite table schema stays; the redemption logic moves to `POST /api/auth/register`.

---

## 3. Invite Flow & KEK Transport

This is the most security-critical part of the system. The server must never see the KEK.

### Two-phase invite

**Phase 1 — Server-side (CLI or API):**
The admin runs `shimmer-server admin invite alice@clinic.com`. The server:
1. Generates a random invite token (32 bytes, base64url-encoded). **Note:** This replaces the current UUID-based token generation in `invite.rs` — 256 bits of entropy is required since the token is used to derive the KEK wrapping key.
2. Stores it in the `invites` table (single-use, 72hr expiry)
3. Sends an email (if SMTP configured) with: download link + a **partial** invite URL: `phi://join/<token>`
4. Pending invites are queryable via `GET /api/org/invites` (admin only) so the admin's desktop app can detect and complete them

**Phase 2 — Client-side (admin's desktop app):**
The admin's app detects the pending invite (via `GET /api/org/invites`) and completes the URL by:
1. Deriving a wrapping key from the invite token using HKDF-SHA256 (salt: `"shimmer-kek-wrap"`, info: `"v1"`)
2. Encrypting the org KEK with AES-256-GCM using the derived wrapping key
3. Base64url-encoding the ciphertext (nonce ‖ ciphertext ‖ tag)
4. Appending it as the URL fragment: `phi://join/<token>#<base64url-encrypted-kek>`
5. The complete URL is copied to clipboard or sent via a secondary channel (the server never sees the fragment)

**Recipient's app:**
1. Deep link handler receives `phi://join/<token>#<encrypted-kek>`
2. User sets email + password → `POST /api/auth/register` with the invite token
3. Server validates token, creates user + member, returns JWT
4. App derives the same wrapping key from the invite token (HKDF-SHA256), decrypts the KEK from the fragment
5. Stores JWT + refresh token in keychain, KEK in keychain
6. Done

### URL scheme

The existing `phi://` scheme is kept for all deep links:
- `phi://<paste-id>` — existing paste links (unchanged)
- `phi://join/<token>#<encrypted-kek>` — new invite links

No second URL scheme is needed. The app distinguishes by path prefix.

### Failure modes

- **Corrupted KEK fragment:** Decryption fails (GCM tag mismatch). App shows "Invalid invite link — ask your admin for a new one."
- **Expired invite token:** Server rejects at registration. App shows "This invite has expired."
- **Admin loses KEK:** Unrecoverable in a zero-knowledge system. All org data is lost. The Future Backlog includes key rotation/escrow mechanisms. For v1, the setup wizard warns the admin to back up their KEK (displayed as hex, one time only).
- **SMTP down:** Invite token is still created. Admin can copy the partial URL from CLI output and complete it manually in the desktop app.

---

## 4. Desktop App Distribution & Auto-Update

### Building the .dmg

- `tauri build` → `.dmg` for macOS
- Code signing with Apple Developer certificate (required for Gatekeeper)
- Notarization via `xcrun notarytool`
- GitHub Releases for hosting binaries

### Auto-update

- `tauri-plugin-updater` — checks JSON endpoint for new versions
- Update manifest on GitHub Releases or served by shimmer-server
- Check on launch + periodically, download in background, prompt restart

### User flow

1. Admin sends invite email
2. Email has "Download Shimmer" link (→ GitHub release `.dmg`) + `phi://join/...` deep link (partial — KEK fragment added by admin's app)
3. User installs, opens, clicks join link
4. Future updates happen silently

**Requirement:** $99/year Apple Developer Program for signing + notarization.

---

## 5. First-Launch Onboarding UX

### New user (no stored credentials)

1. Welcome screen: "Paste an invite link or enter your server details"
   - Text field for `phi://join/...` URL (copy-paste fallback)
   - Or manual entry: server URL + invite code
   - Deep link handler auto-fills if they clicked the link
2. Set password — email pre-filled from invite token
3. Confirmation — "You're connected to Acme Clinic" → transitions to tray app

### Returning user

- Normal tray launch (existing behavior)
- If refresh token expired (30+ days inactive) → login screen: email + password

### Deep link registration

- `phi://` scheme already declared in `tauri.conf.json`
- `.dmg` install registers scheme with Launch Services
- Invite email includes download link + join link with install-first instructions

---

## 6. Admin CLI & Service Layer

### Service layer

All admin operations in `shimmer-server::services`:
- `auth` — register, login, refresh, password hashing
- `org` — create org, list members, remove member, change role
- `invite` — create invite, send email, redeem invite
- `paste` — existing logic refactored out of route handlers

Services take `&DbPool`, no HTTP types. Route handlers, CLI, and future web dashboard all call services.

### CLI subcommands

```
shimmer-server setup                          # TUI wizard
shimmer-server serve                          # Run API server
shimmer-server admin invite <email>           # Generate invite token + send email
shimmer-server admin list-members             # Show org members
shimmer-server admin remove <email>           # Revoke access + delete refresh tokens
shimmer-server admin set-role <email> <role>  # admin/member/read_only
```

Role values are lowercase snake_case (`admin`, `member`, `read_only`) matching the DB CHECK constraint. CLI accepts case-insensitive input.

Reads from `shimmer.toml`, connects to SQLite directly, calls service layer (no HTTP round-trip).

**Note on invite CLI:** The CLI creates the server-side invite token only. The encrypted KEK fragment must be attached by the admin's desktop app (since the server never has the KEK). The CLI prints the partial URL and instructs the admin to complete it in the app.

---

## 7. Config File & Environment

### shimmer.toml

Single source of truth, written by TUI wizard. Migrates from the current flat `shimmer-server.toml` / env-var-only approach to a structured TOML format. The existing `ServerConfig` struct in `config.rs` is refactored into nested structs, and `deny_unknown_fields` is removed to allow forward-compatible config evolution.

```toml
# WARNING: Contains secrets. Do not share or commit this file.
# File permissions should be 0600 (owner read/write only).

[server]
bind = "0.0.0.0:8443"
jwt_secret = "generated-during-setup"

[storage]
backend = "file"           # or "s3"
path = "/var/shimmer/data"

[storage.s3]
endpoint = "https://..."
bucket = "shimmer"
access_key_id = "..."
secret_access_key = "..."

[org]
name = "Acme Clinic"

[smtp]
host = "smtp.example.com"
port = 587
username = "..."
password = "..."
from = "shimmer@clinic.com"

[database]
path = "/var/shimmer/shimmer.db"
```

**Precedence:** Env vars override TOML values (for Docker). Existing env var names preserved as aliases.

**Location:** `./shimmer.toml` default, overridable with `--config <path>` or `SHIMMER_CONFIG`.

---

## 8. TLS

PHI must not travel over unencrypted HTTP. Two supported modes:

**Option A — Reverse proxy (recommended for v1):**
The setup wizard asks if the admin has a reverse proxy (nginx, Caddy, etc.). If yes, shimmer-server binds to localhost only (`127.0.0.1:8443`) and the proxy terminates TLS. The wizard prints a sample Caddy/nginx config snippet.

**Option B — Built-in TLS (future):**
Add `tls.cert_path` and `tls.key_path` to `shimmer.toml`. Axum's `axum-server` with `rustls` terminates TLS directly. Useful for simple deployments without a proxy.

For v1, Option A only. The setup wizard warns if no reverse proxy is configured and the bind address is not localhost.

---

## Future Backlog (Post-v1)

### Distribution & Platform
- Homebrew tap for macOS
- Windows support (.msi) + Linux (.AppImage)
- Docker image for server (single `docker run` setup)

### Authentication
- OIDC/SSO integration (Okta, Azure AD, Google Workspace, JumpCloud)
- Token-only auth (no passwords — Tailscale-style device tokens)
- Derive encryption key from SSO session

### Admin
- Web admin dashboard (thin layer over service layer)
- Usage analytics (paste counts, storage, active users)

### Features
- Burn-on-read UI (backend exists, frontend not wired)
- TTL cleanup cron job (backend enforces, no scheduled cleanup)
- Screenshot capture (Cmd+Shift+S for region screenshots)
- Email notifications

### Security Hardening
- CORS lockdown (currently permissive, flagged TODO)
- Rate limiting on auth endpoints
- Audit log export
- Key rotation / escrow mechanism (critical for KEK loss recovery)
- Built-in TLS termination (Option B above)
