# Meta's Internal Pastebin vs Shimmer — Feature Comparison

## Meta's Internal Pastebin

Meta uses **Phabricator Paste**, part of their Phabricator stack. It's a web-based paste tool integrated with code review, bug tracking, and source browsing.

### How It Works

- **Web UI + CLI**: `arc paste` (e.g. `echo "content" | arc paste` or `arc paste --lang python`)
- **Conduit API**: `paste.create`, `paste.search`, etc.
- **Auth**: Internal SSO; only employees can access pastes
- **Storage**: Server-side; content is stored in plaintext on Meta's infrastructure

---

## Features Meta/Phabricator Paste Has That Shimmer Doesn't

| Feature | Meta/Phabricator | Shimmer |
|---------|------------------|---------|
| **Syntax highlighting** | Yes (Pygments or built-in, many languages) | No |
| **Language detection** | Yes (auto-detect or explicit `--lang`) | No |
| **Titles** | Yes | No |
| **Search & filters** | Yes (author, language, status, tags, date range, spaces) | No |
| **Archive / active status** | Yes | No |
| **Tags** | Yes | No |
| **Subscribers** | Yes (notifications) | No |
| **Spaces** | Yes (project/org grouping) | No |
| **CLI** | `arc paste` | Hotkey only (clipboard capture) |
| **Per-paste policies** | Yes (view/edit policies) | No |
| **Rich metadata** | Author, creation date, etc. | UUID only |
| **Conduit API** | Yes | No |

---

## What Shimmer Has That Meta's Pastebin Doesn't

| Feature | Shimmer | Meta/Phabricator |
|---------|---------|------------------|
| **Client-side encryption** | AES-256-GCM; server never sees plaintext | Server-side storage; server sees plaintext |
| **Zero-knowledge** | Yes | No |
| **phi:// protocol** | Yes (deep links) | Standard URLs |
| **Tray-only workflow** | Yes (minimal UI, hotkey-driven) | Web + CLI |
| **PHI-focused design** | Yes | General-purpose code sharing |

---

## Summary

Meta's pastebin is optimized for **developer workflow** (search, syntax highlighting, tags, integration with code review). Shimmer is optimized for **PHI security** (client-side encryption, zero-knowledge, minimal exposure).

---

## Potential Additions to Shimmer (in priority order)

1. **Syntax highlighting** (with optional language detection)
2. **Titles** for pastes
3. **Search / listing** (with filters)
4. **CLI** (e.g. `shimmer paste` or `shimmer paste < file.txt`)
5. **Tags** for organization

These can be layered on top of the current encrypted storage without weakening the security model.
