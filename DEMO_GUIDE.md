# Shimmer — Demo & Testing Guide

## Architecture

```
Tauri desktop app
  ├── KeyStore  (org KEK — shared by all org members, NEVER leaves client)
  ├── encrypt_envelope()  (AES-256-GCM, per-paste DEK wrapped by KEK)
  ├── blind_index_token()  (HMAC-SHA256 search tokens — server can match, can't read)
  └── ShimmerClient ──HTTP──▶ shimmer-server ──▶ S3 / file
                                │
                                ├── SQLite: paste metadata, search tokens, orgs, members, invites
                                └── Blob storage: ciphertext only — no KEK, no plaintext
```

PHI never touches the server in plaintext. The server is a zero-knowledge relay.
Search works via blind index tokens — the server matches opaque HMAC hashes without
knowing what they represent.

---

## Prerequisites

```bash
just install        # npm deps + cargo-nextest, cargo-deny, cargo-audit
```

---

## 1. Quick Smoke Test (30 seconds)

```bash
just check          # fmt-check → clippy → test (32 tests)
```

All 32 tests pass:
- 12 shimmer-core (7 proptest crypto invariants)
- 5 shimmer-server DB unit tests (org, member, paste, search, visibility)
- 15 API integration tests (upload, fetch, delete, list, search, visibility, roles, invites, files)

---

## 2. Full Local Stack

The Tauri app **requires** shimmer-server. It is an HTTP client — it does NOT
access storage directly.

**Option A — one command (auto-starts server, auto-generates token):**
```bash
just dev-full
```

**Option B — manual (two terminals):**
```bash
# Terminal 1: start the server
just dev-server

# Terminal 2: start the Tauri app
export SHIMMER_JWT=$(just gen-token)
export SHIMMER_SERVER_URL=http://localhost:8443
just dev
```

---

## 3. Try It — Text Paste

1. Copy any text to your clipboard
2. Press **⌘+⇧+P** (Cmd+Shift+P)
3. Clipboard becomes `phi://UUID` — tray icon flashes green, sound plays
4. Open Shimmer (click tray icon → Settings)
5. Paste `phi://UUID` into **Fetch** → decrypted content appears
6. **Browse** tab shows your paste history with metadata

**What happened:**
- Text was AES-256-GCM encrypted with your local KEK (per-paste random DEK, wrapped by KEK)
- Blind index search tokens were generated from the text content (HMAC-SHA256)
- Ciphertext + tokens were POSTed to shimmer-server
- Server stored ciphertext to blob storage, metadata + search tokens to SQLite
- `phi://UUID` is a pointer — reveals nothing about the content
- On fetch: server checks visibility permissions → returns ciphertext → client decrypts locally

---

## 4. Try It — File Upload

Files (images, PDFs, screenshots) work the same way as text. The client:

1. Reads the file bytes
2. Detects content type from the file extension
3. Encrypts the filename (it could contain PHI like patient names)
4. Generates search tokens from the filename parts
5. Encrypts the file content with the same envelope encryption
6. Uploads ciphertext + encrypted filename + content type to the server

The `file_upload` Tauri command accepts a file path. The frontend can use a file
picker dialog or drag-and-drop to get the path.

Supported types: PNG, JPEG, GIF, WebP, PDF, DOCX, XLSX, CSV, TXT, JSON, XML, ZIP,
and more. Max file size: 25 MiB (before encryption overhead).

---

## 5. Try It — Search

The `paste_search` Tauri command converts human-readable search terms into blind
index tokens client-side, then sends the opaque tokens to the server.

```
User types:     "patient 4821"
Client computes: HMAC("patient") → "a7f3c2...", HMAC("4821") → "e82f01..."
Server matches:  WHERE token IN ("a7f3c2...", "e82f01...") GROUP BY paste HAVING COUNT = 2
Server returns:  matching paste IDs (never sees "patient" or "4821")
Client fetches + decrypts each match
```

All org members with the same KEK produce the same tokens for the same words.

---

## 6. Try It — Team Sharing

To simulate two team members sharing PHI locally:

```bash
# Generate a shared org key
export SHARED_KEY=$(just gen-key)

# Terminal 1: server
just dev-server

# Terminal 2: "Alice"
export SHIMMER_DEV_KEY=$SHARED_KEY
export SHIMMER_JWT=$(SHIMMER_USER_ID=u_alice just gen-token)
export SHIMMER_SERVER_URL=http://localhost:8443
just dev
# → Copy text, press ⌘+⇧+P, get phi://UUID

# Terminal 3: "Bob"
export SHIMMER_DEV_KEY=$SHARED_KEY
export SHIMMER_JWT=$(SHIMMER_USER_ID=u_bob just gen-token)
export SHIMMER_SERVER_URL=http://localhost:8443
just dev
# → Paste Alice's phi://UUID into Fetch → decrypted content appears
```

**What you're seeing:**
- Server logs show `POST /api/paste` from Alice, `GET /api/paste/:id` from Bob
- `./shimmer-storage/` shows opaque JSON blobs (open one — it's unreadable)
- Bob can fetch + decrypt Alice's paste because they share the same org KEK
- The server never saw the plaintext or the KEK

**Visibility controls:**
- `org` (default): all org members can fetch
- `private`: only the uploader can fetch
- `link`: anyone with auth can fetch

---

## 7. Invite Flow (API-level)

The invite flow is fully functional at the API level:

```bash
# Admin generates an invite
curl -s http://localhost:8443/api/org/invite \
  -H "Authorization: Bearer $SHIMMER_JWT" \
  -H "Content-Type: application/json" \
  -d '{"role": "member", "ttlHours": 24}' | jq .

# New user joins with the invite token (no auth needed)
curl -s http://localhost:8443/api/org/join \
  -H "Content-Type: application/json" \
  -d '{"token": "THE_INVITE_TOKEN", "name": "New User"}' | jq .
# → Returns: { orgId, userId, jwt, role }
```

In production, the invite URL will be `shimmer://join/TOKEN#ENCRYPTED_KEK` —
the `#fragment` (containing the encrypted org KEK) never hits the server.

---

## 8. Role Management (API-level)

```bash
# List org members
curl -s http://localhost:8443/api/org/members \
  -H "Authorization: Bearer $SHIMMER_JWT" | jq .

# Promote/demote (admin only)
curl -s -X PUT http://localhost:8443/api/org/members/u_bob \
  -H "Authorization: Bearer $SHIMMER_JWT" \
  -H "Content-Type: application/json" \
  -d '{"role": "read_only"}' | jq .

# Remove member (admin only)
curl -s -X DELETE http://localhost:8443/api/org/members/u_bob \
  -H "Authorization: Bearer $SHIMMER_JWT"
```

| Action | Admin | Member | Read-only |
|--------|-------|--------|-----------|
| Upload pastes/files | yes | yes | no |
| Read org pastes | yes | yes | yes |
| Search | yes | yes | yes |
| Delete own pastes | yes | yes | no |
| Delete any paste | yes | no | no |
| Invite members | yes | no | no |
| Manage roles | yes | no | no |

---

## 9. Run with S3 (MinIO)

```bash
just up             # start MinIO in Docker, create 'shimmer' bucket

# Terminal 1:
just dev-server-s3  # shimmer-server → MinIO

# Terminal 2:
export SHIMMER_JWT=$(just gen-token)
export SHIMMER_SERVER_URL=http://localhost:8443
just dev
```

MinIO console: http://localhost:9001 (user: `minioadmin`, pass: `minioadmin`).
Encrypted blobs appear in the `shimmer` bucket — completely opaque JSON.

```bash
just down           # stop MinIO
```

---

## 10. Security & Supply Chain

```bash
just audit          # known vulnerability check (cargo-audit)
just deny           # license allowlist + advisory + duplicate dep policy
just deps-dupes     # show duplicate dependency versions
```

---

## 11. Quality Checks

```bash
just fmt-check      # verify formatting
just clippy         # lint all 3 crates (warnings = errors)
just test           # run all 32 tests
just test-core      # crypto + storage unit tests only
just test-server    # DB + API integration tests
just test-v         # tests with stdout (see tracing output)
```

---

## 12. Dev Token

The Tauri app authenticates to shimmer-server using a JWT (`SHIMMER_JWT`).
In dev, generate one signed with the dev secret:

```bash
export SHIMMER_JWT=$(just gen-token)
```

Customize user/org:
```bash
SHIMMER_USER_ID=u_alice SHIMMER_ORG_ID=org_acme just gen-token
```

---

## 13. Environment Variables

### Tauri Desktop App
| Variable | Default | Description |
|----------|---------|-------------|
| `SHIMMER_SERVER_URL` | `http://localhost:8443` | shimmer-server base URL |
| `SHIMMER_JWT` | (empty → 401) | Bearer token for server auth |
| `SHIMMER_DEV_KEY` | (random, persisted) | Hex-encoded 256-bit org KEK |
| `RUST_LOG` | `info` | Tracing filter |

### shimmer-server
| Variable | Default | Description |
|----------|---------|-------------|
| `JWT_SECRET` | `dev-secret-change-in-production` | JWT signing secret |
| `SHIMMER_STORAGE_BACKEND` | `file` | `file` or `s3` |
| `SHIMMER_STORAGE_PATH` | `./shimmer-storage` | Local file storage directory |
| `SHIMMER_DB_PATH` | `./shimmer-metadata.db` | SQLite metadata database path |
| `SHIMMER_ORG_ID` | — | Auto-create this org on startup |
| `SHIMMER_ORG_NAME` | — | Name for auto-created org |
| `SHIMMER_S3_ENDPOINT` | — | S3-compatible endpoint URL |
| `SHIMMER_S3_BUCKET` | `shimmer` | S3 bucket name |
| `AWS_ACCESS_KEY_ID` | — | S3 credentials |
| `AWS_SECRET_ACCESS_KEY` | — | S3 credentials |
| `LOG_FORMAT` | `pretty` | Set to `json` for structured JSON logging |
| `RUST_LOG` | `info` | Tracing filter |

### gen-token
| Variable | Default | Description |
|----------|---------|-------------|
| `JWT_SECRET` | `dev-secret-change-in-production` | Must match server's secret |
| `SHIMMER_USER_ID` | `u_dev_user` | User ID embedded in JWT `sub` |
| `SHIMMER_ORG_ID` | `org_dev` | Org ID embedded in JWT `org` |

---

## 14. API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/api/health` | No | Health check |
| `POST` | `/api/paste` | JWT | Upload encrypted paste/file |
| `GET` | `/api/paste/:id` | JWT | Fetch paste (checks visibility) |
| `DELETE` | `/api/paste/:id` | JWT | Delete paste (owner or admin) |
| `GET` | `/api/pastes` | JWT | List pastes (visibility-filtered) |
| `GET` | `/api/pastes?tokens=a,b` | JWT | Search by blind index tokens |
| `POST` | `/api/org` | JWT | Create organization |
| `GET` | `/api/org/members` | JWT | List org members |
| `PUT` | `/api/org/members/:id` | JWT (admin) | Update member role |
| `DELETE` | `/api/org/members/:id` | JWT (admin) | Remove member |
| `POST` | `/api/org/invite` | JWT (admin) | Generate invite token |
| `POST` | `/api/org/join` | Invite token | Join org (no JWT needed) |

---

## 15. What to Show

| File | What's impressive |
|------|-------------------|
| `shimmer-server/src/db.rs` | SQLite metadata with search token index, visibility filtering, invite lifecycle |
| `shimmer-server/src/routes/paste.rs` | Org-level visibility (private/org/link), burn-on-read, search by blind tokens |
| `shimmer-server/src/routes/invite.rs` | Zero-config invite flow — single-use tokens, role assignment, JWT issuance |
| `shimmer-core/src/encryption.rs` | Envelope crypto + blind index search + 7 proptest invariants |
| `shimmer-server/tests/api_test.rs` | 15 in-process integration tests, zero Docker, sub-second |
| `src-tauri/src/lib.rs` | File upload with encrypted filename + content type detection + search tokens |
| `src-tauri/src/key_store.rs` | `ZeroizeOnDrop` — KEK wiped from memory on drop |
| `Cargo.toml` (root) | `[workspace.lints]` — centralized strict lint policy across 3 crates |
| `deny.toml` | License allowlist + vulnerability denial policy |
