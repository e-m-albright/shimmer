# Shimmer Demo Guide

End-to-end walkthrough: server setup, admin onboarding, inviting a team member, and sharing PHI — all zero-knowledge.

---

## Prerequisites

```bash
# From the shimmer repo root
just install          # npm install + cargo dev tools
```

You need two terminal windows (or tabs). The demo uses local file storage — no Docker/MinIO needed.

---

## Act 1: Server Admin Sets Up Shimmer

### 1a. Run the TUI Setup Wizard

```bash
cargo run -p shimmer-server -- setup
```

This launches a full-screen terminal wizard. Walk through each step:

| Step | What to enter | Notes |
|------|--------------|-------|
| Storage Backend | **File storage** (arrow keys, Enter) | S3 works too but file is simpler for demo |
| Storage path | Press Enter for default (`./shimmer-storage`) | |
| Database path | Press Enter for default (`./shimmer-metadata.db`) | |
| Org name | `Acme Clinic` | Whatever you want |
| Admin email | `admin@acme.com` | This becomes the admin login |
| Admin password | `DemoP@ss123` | 8+ chars |
| SMTP? | `n` | Skip email for demo |
| Confirm | Enter | |

On success you'll see:

```
Setup complete!

  Config written to: shimmer.toml (permissions: 0600)
  Organisation:      Acme Clinic (org_abc123...)
  Admin email:       admin@acme.com

Next steps:
  shimmer-server serve
```

What just happened:
- `shimmer.toml` was created with a random JWT secret (0600 permissions — only owner can read)
- SQLite database initialized with `users`, `members`, `orgs`, `invites`, `refresh_tokens` tables
- Admin user created with argon2-hashed password
- Admin added as the first org member with `admin` role

### 1b. Start the Server

```bash
cargo run -p shimmer-server -- serve
```

You'll see:

```
INFO shimmer-server starting bind="0.0.0.0:8443"
INFO listening addr="0.0.0.0:8443"
```

Leave this running.

### 1c. Verify with curl (optional)

In a second terminal:

```bash
# Login as admin
curl -s http://localhost:8443/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"email":"admin@acme.com","password":"DemoP@ss123"}' | jq .

# You'll get back:
# {
#   "userId": "u_abc123...",
#   "accessToken": "eyJ...",
#   "refreshToken": "abc-def-..."
# }
```

---

## Act 2: Admin Creates an Invite

### 2a. Via CLI

```bash
cargo run -p shimmer-server -- admin invite nurse@acme.com
```

Output:

```
Invite created for nurse@acme.com
Token: dGVzdC1pbnZ...  (base64url, 256-bit random)
Partial invite URL: phi://join/dGVzdC1pbnZ...

Complete this invite in the Shimmer desktop app to attach the encrypted org key.
```

Copy that token — the new user will need it.

### 2b. Via API (alternative)

```bash
# Use the admin's access token from the login step
export TOKEN="eyJ..."

curl -s http://localhost:8443/api/org/invite \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"role":"member","ttlHours":24,"singleUse":true}' | jq .

# Returns: { "token": "...", "orgId": "org_...", "expiresAt": "..." }
```

### 2c. List pending invites

```bash
curl -s http://localhost:8443/api/org/invites \
  -H "Authorization: Bearer $TOKEN" | jq .
```

---

## Act 3: New User Joins via Desktop App

### 3a. Launch the Desktop App

In a second terminal:

```bash
# Set the server URL and a dev encryption key (in production, the KEK
# is generated/received automatically — this is just for dev convenience)
export SHIMMER_SERVER_URL=http://localhost:8443
export SHIMMER_DEV_KEY=$(openssl rand -hex 32)

just dev
```

### 3b. Onboarding Screen

The app opens to the **Welcome to Shimmer** screen with two options:

1. **Paste an invite link** — enter `phi://join/<token>` (the token from Act 2)
2. **Sign in** — if you already have an account

For a new user demo:

1. Paste the invite URL into the invite field
2. Click **Continue with Invite**
3. Fill in name, email, password on the next screen
4. Click **Create Account**

The app calls `POST /api/auth/register` with the invite token, creates the user, consumes the invite, and stores JWT tokens in the macOS keychain.

You're now logged in and see the main Shimmer UI.

### 3c. Existing User Login

If you restart the app and the keychain tokens are cleared, you'll see the login form. Enter email + password.

---

## Act 4: Share PHI

### 4a. Hotkey (fastest)

1. Copy some text to your clipboard (any text — pretend it's a patient note)
2. Press **Cmd+Shift+P**
3. A `phi://` link replaces your clipboard contents
4. You hear a system sound and the tray icon flashes green

### 4b. Upload via UI

1. Click the **Paste** tab
2. Type or paste text into the **Upload Text** field
3. Click **Upload**
4. A `phi://` link appears — click it to copy

### 4c. Upload a File

1. Click **Choose File** in the Upload File section
2. Pick any file (PDF, image, document)
3. Get back a `phi://` link

### 4d. Fetch & Decrypt

1. Paste a `phi://` link into the **Fetch & Decrypt** field
2. Click **Fetch**
3. The decrypted content appears — decrypted locally, server never saw plaintext

### 4e. Browse & Search

1. Click the **Browse** tab
2. See all pastes visible to your org
3. Type in the search box — searches use blind index tokens (server can match without seeing content)
4. Click a row to expand and see decrypted content
5. Select multiple and bulk delete

---

## Act 5: Admin Management

### 5a. List Members

```bash
cargo run -p shimmer-server -- admin list-members
```

```
NAME                 USER ID                        ROLE
------------------------------------------------------------
Admin                u_abc123...                    admin
Nurse                u_def456...                    member
```

### 5b. Change a Role

```bash
cargo run -p shimmer-server -- admin set-role nurse@acme.com read_only
```

### 5c. Remove a Member

```bash
cargo run -p shimmer-server -- admin remove nurse@acme.com
# Removes member AND revokes all refresh tokens (immediate session kill)
```

---

## Act 6: Show the Zero-Knowledge Architecture

This is the "wow" moment for a demo audience.

### 6a. Look at what the server stores

```bash
# Open the SQLite database
sqlite3 shimmer-metadata.db

-- Show a paste record — note: no plaintext anywhere
SELECT id, content_type, size_bytes, visibility, encrypted_title FROM pastes LIMIT 3;

-- Show search tokens — opaque HMAC hashes, server can't reverse them
SELECT paste_id, token, token_type FROM search_tokens LIMIT 5;

-- Show users — password is argon2 hash
SELECT id, email, password_hash FROM users;

-- Show the blob storage — just opaque ciphertext files
.quit
```

```bash
ls shimmer-storage/
# Shows directories like: u_abc123/
#   containing files like: <uuid>
#   Each file is an AES-256-GCM encrypted JSON envelope

# Try to read one — it's gibberish
cat shimmer-storage/u_*/$(ls shimmer-storage/u_*/ | head -1) | head -c 200
```

### 6b. Key Point for the Audience

> "The server stores ciphertext, opaque search tokens, and hashed passwords.
> Even if this database were fully compromised, an attacker gets nothing —
> no plaintext, no encryption key, no way to search. The KEK lives only on
> the user's Mac, in the system keychain."

---

## Quick Reset (Start Over)

```bash
rm -f shimmer-metadata.db shimmer.toml
rm -rf shimmer-storage/
# Then re-run: cargo run -p shimmer-server -- setup
```

---

## Architecture Diagram (for slides)

```
┌─────────────────────────────────┐
│  Shimmer Desktop App (Tauri)    │
│                                 │
│  KEK stored in macOS Keychain   │
│  ↓                              │
│  AES-256-GCM encrypt locally    │
│  HMAC-SHA256 blind index tokens │
│  ↓                              │
│  Send ciphertext + tokens only  │
└─────────────┬───────────────────┘
              │ HTTPS
              ▼
┌─────────────────────────────────┐
│  shimmer-server (Axum)          │
│                                 │
│  Stores ciphertext → file/S3    │
│  Stores search tokens → SQLite  │
│  Never sees plaintext or KEK    │
│  Auth: argon2 + JWT + refresh   │
└─────────────────────────────────┘
```

---

## Demo Script Timing

| Act | Time | What |
|-----|------|------|
| 1. Setup wizard | 2 min | Run TUI, show config file created |
| 2. Create invite | 30 sec | CLI one-liner |
| 3. Desktop onboarding | 1 min | Paste invite, create account |
| 4. Share PHI | 2 min | Hotkey, upload, fetch, search |
| 5. Admin management | 1 min | List, role change, remove |
| 6. Zero-knowledge proof | 2 min | SQLite + blob inspection |
| **Total** | **~9 min** | |
