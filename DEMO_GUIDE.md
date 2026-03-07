# Shimmer — Demo & Testing Guide

## Prerequisites

```bash
just install        # installs npm deps + cargo-nextest, cargo-deny, cargo-audit
```

---

## 1. Quick Smoke Test (30 seconds)

```bash
just check          # fmt-check → clippy → test (all 20 tests)
```

If green, the entire workspace compiles, passes strict lints (`unsafe_code = "forbid"`,
`unwrap_used = "deny"`), and all crypto + API tests pass — including 7 property-based
tests that fuzz the encryption with random inputs.

---

## 2. Run Individual Quality Checks

```bash
just fmt-check      # verify formatting (rustfmt with 100-col width)
just clippy         # lint all 3 crates (warnings = errors)
just test           # run all tests (uses nextest if installed)
just test-v         # run tests with stdout visible (see tracing output)
just test-core      # run only shimmer-core tests (crypto + storage)
just test-server    # run only shimmer-server tests (API integration)
```

---

## 3. Run the Desktop App (No Setup Needed)

```bash
just dev            # launches Shimmer with local file storage
```

**Try it:**
1. Copy any text to your clipboard
2. Press **⌘+⇧+P** (Cmd+Shift+P)
3. Your clipboard is now a `phi://` link — tray icon flashes green, sound plays
4. Open Shimmer window (click tray → Settings)
5. Paste the `phi://` link into the **Fetch** tab → decrypted content appears
6. **Browse** tab shows your paste history with list/delete

**What just happened:** The text was encrypted client-side with AES-256-GCM
envelope encryption (random per-paste DEK, wrapped by your org KEK),
stored as ciphertext, and the `phi://` link is just a UUID pointer.
The plaintext never touches disk unencrypted.

---

## 4. Run the API Server

```bash
just dev-server     # starts shimmer-server on :8443 with file storage
```

In another terminal:

```bash
# Health check
curl http://localhost:8443/api/health
# → "ok"

# Unauthenticated request (should 401)
curl -s -o /dev/null -w "%{http_code}" \
  -X POST http://localhost:8443/api/paste \
  -H "Content-Type: application/json" \
  -d '{"ciphertext": "test"}'
# → 401 (auth is working)
```

---

## 5. Run with S3 (MinIO)

```bash
just up             # starts MinIO in Docker + creates 'shimmer' bucket
just dev-s3         # launches desktop app → MinIO
# or
just dev-server-s3  # launches API server → MinIO
```

MinIO console: http://localhost:9001 (user: `minioadmin`, pass: `minioadmin`).
Browse the `shimmer` bucket to see encrypted paste blobs — they are opaque
JSON envelopes. No plaintext anywhere.

```bash
just down           # stops and removes MinIO container
```

---

## 6. Security & Supply Chain Checks

```bash
just audit          # cargo-audit: check for known vulnerabilities in deps
just deny           # cargo-deny: license allowlist + advisory + duplicate deps
just deps-dupes     # show any duplicate dependency versions
```

---

## 7. Explore the Workspace

```bash
just workspace      # lists all crates with versions and descriptions
just deps           # full dependency tree
just gen-key        # generate a random 256-bit hex key (for SHIMMER_DEV_KEY)
```

---

## 8. Run Pre-commit Hooks

```bash
just hooks-install  # install lefthook git hooks
just hooks-run      # run pre-commit checks manually
just hooks-push     # run pre-push checks manually
```

---

## 9. What to Inspect

| File | What's impressive |
|------|-------------------|
| `Cargo.toml` (root) | `[workspace.lints]` — centralized strict clippy policy across 3 crates |
| `shimmer-core/src/encryption.rs` | Envelope encryption + 7 proptest property-based tests |
| `shimmer-server/tests/api_test.rs` | 8 in-process integration tests, zero Docker, sub-second |
| `deny.toml` | License allowlist + vulnerability policy |
| `justfile` | One-command DX for everything |
| `rustfmt.toml` | Opinionated formatting (100-col, field init shorthand) |

### Crypto property tests prove:
- Roundtrip: encrypt → decrypt = original (for any random plaintext up to 4 KiB)
- Nonce uniqueness: same plaintext encrypted twice → different ciphertexts
- Wrong key: always fails decryption (no silent corruption)
- Blind index: deterministic + case-insensitive
- Serialization: envelope survives JSON roundtrip

---

## 10. Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SHIMMER_DEV_KEY` | (random) | Hex-encoded 256-bit encryption key |
| `SHIMMER_STORAGE_PATH` | `./shimmer-dev-storage` | File storage directory |
| `SHIMMER_S3_ENDPOINT` | — | S3-compatible endpoint URL |
| `SHIMMER_S3_BUCKET` | `shimmer` | S3 bucket name |
| `AWS_ACCESS_KEY_ID` | — | S3 credentials |
| `AWS_SECRET_ACCESS_KEY` | — | S3 credentials |
| `AWS_REGION` | `us-east-1` | S3 region |
| `SHIMMER_USER_PREFIX` | `dev-user` | User ID prefix for storage scoping |
| `LOG_FORMAT` | `pretty` | Set to `json` for structured JSON logging |
| `RUST_LOG` | `info` | Tracing filter (e.g., `debug`, `shimmer=trace`) |
