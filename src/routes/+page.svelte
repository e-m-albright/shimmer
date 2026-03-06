<script lang="ts">
  import { onMount, onDestroy } from "svelte";

  type Tab = "paste" | "browse" | "settings";
  type PasteEntry = { id: string; size: number; created: string };
  type Settings = {
    storageType: string;
    storagePath: string;
    s3Endpoint: string;
    s3Bucket: string;
    userPrefix: string;
    keyFingerprint: string;
    hotkey: string;
  };

  let tab = $state<Tab>("paste");
  let pasteInput = $state("");
  let pasteResult = $state("");
  let fetchId = $state("");
  let fetchResult = $state("");
  let isTauri = $state(false);
  let uploadLoading = $state(false);
  let fetchLoading = $state(false);
  let toast = $state("");
  let toastTimeout: ReturnType<typeof setTimeout> | null = null;
  let copied = $state(false);

  // Browse state
  let entries = $state<PasteEntry[]>([]);
  let browseLoading = $state(false);
  let browseError = $state("");
  let expandedId = $state<string | null>(null);
  let expandedContent = $state("");
  let expandLoading = $state(false);

  // Settings state
  let settings = $state<Settings | null>(null);

  async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    if (typeof window === "undefined" || !(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
      throw new Error("Run via Shimmer app (just dev) — not in browser");
    }
    const { invoke: invokeFn } = await import("@tauri-apps/api/core");
    return invokeFn<T>(cmd, args);
  }

  function showToast(msg: string) {
    if (toastTimeout) clearTimeout(toastTimeout);
    toast = msg;
    toastTimeout = setTimeout(() => {
      toast = "";
      toastTimeout = null;
    }, 2000);
  }

  function extractId(url: string): string | null {
    const id = url.replace(/^phi:\/\//, "").split("/")[0]?.trim();
    return id || null;
  }

  async function fetchById(id: string) {
    fetchLoading = true;
    fetchResult = "";
    tab = "paste";
    fetchId = `phi://${id}`;
    try {
      fetchResult = await invoke<string>("paste_fetch", { id });
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      getCurrentWindow().show();
      getCurrentWindow().setFocus();
    } catch (e) {
      fetchResult = e instanceof Error ? e.message : String(e);
    } finally {
      fetchLoading = false;
    }
  }

  let unlistenOpenUrl: (() => void) | undefined;
  let unlistenPasteSuccess: (() => void) | undefined;

  onMount(() => {
    isTauri = typeof window !== "undefined" && !!(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    if (isTauri) {
      import("@tauri-apps/api/event").then(({ listen }) => {
        listen("phi-paste-success", () => {
          showToast("Link copied to clipboard");
        }).then((fn) => { unlistenPasteSuccess = fn; });
      });
      import("@tauri-apps/plugin-deep-link").then(async ({ getCurrent, onOpenUrl }) => {
        const urls = await getCurrent();
        const id = urls?.[0] ? extractId(urls[0]) : null;
        if (id) fetchById(id);
        unlistenOpenUrl = await onOpenUrl((urls) => {
          const id = urls[0] ? extractId(urls[0]) : null;
          if (id) fetchById(id);
        });
      });
    }
  });

  onDestroy(() => {
    if (toastTimeout) clearTimeout(toastTimeout);
    unlistenOpenUrl?.();
    unlistenPasteSuccess?.();
  });

  async function testPaste(event: Event) {
    event.preventDefault();
    uploadLoading = true;
    pasteResult = "";
    try {
      const url = await invoke<string>("paste_upload", { plaintext: pasteInput });
      pasteResult = url;
      pasteInput = "";
    } catch (e) {
      pasteResult = e instanceof Error ? e.message : String(e);
    } finally {
      uploadLoading = false;
    }
  }

  async function testFetch(event: Event) {
    event.preventDefault();
    const id = extractId(fetchId);
    if (!id) { fetchResult = "Enter a phi:// ID"; return; }
    fetchLoading = true;
    fetchResult = "";
    try {
      fetchResult = await invoke<string>("paste_fetch", { id });
    } catch (e) {
      fetchResult = e instanceof Error ? e.message : String(e);
    } finally {
      fetchLoading = false;
    }
  }

  async function copyToClipboard(text: string) {
    try {
      if (isTauri) {
        const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
        await writeText(text);
      } else {
        await navigator.clipboard.writeText(text);
      }
      copied = true;
      showToast("Copied!");
      setTimeout(() => { copied = false; }, 1500);
    } catch {
      showToast("Failed to copy");
    }
  }

  async function loadBrowse() {
    browseLoading = true;
    browseError = "";
    expandedId = null;
    try {
      entries = await invoke<PasteEntry[]>("paste_list");
    } catch (e) {
      browseError = e instanceof Error ? e.message : String(e);
    } finally {
      browseLoading = false;
    }
  }

  async function toggleExpand(id: string) {
    if (expandedId === id) {
      expandedId = null;
      expandedContent = "";
      return;
    }
    expandedId = id;
    expandedContent = "";
    expandLoading = true;
    try {
      expandedContent = await invoke<string>("paste_fetch", { id });
    } catch (e) {
      expandedContent = `Error: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      expandLoading = false;
    }
  }

  async function deletePaste(id: string) {
    try {
      await invoke("paste_delete", { id });
      entries = entries.filter(e => e.id !== id);
      if (expandedId === id) { expandedId = null; expandedContent = ""; }
      showToast("Deleted");
    } catch (e) {
      showToast(`Delete failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  async function loadSettings() {
    try {
      settings = await invoke<Settings>("get_settings");
    } catch {
      settings = null;
    }
  }

  function switchTab(t: Tab) {
    tab = t;
    if (t === "browse") loadBrowse();
    if (t === "settings") loadSettings();
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function formatDate(iso: string): string {
    if (!iso) return "—";
    try {
      const d = new Date(iso);
      return d.toLocaleDateString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
    } catch { return iso; }
  }
</script>

<div class="app">
  <nav class="sidebar">
    <div class="logo">
      <span class="logo-text">Shimmer</span>
    </div>
    <button class="nav-item" class:active={tab === "paste"} onclick={() => switchTab("paste")}>
      <span class="nav-icon">↑↓</span>
      <span>Paste</span>
    </button>
    <button class="nav-item" class:active={tab === "browse"} onclick={() => switchTab("browse")}>
      <span class="nav-icon">☰</span>
      <span>Browse</span>
    </button>
    <button class="nav-item" class:active={tab === "settings"} onclick={() => switchTab("settings")}>
      <span class="nav-icon">⚙</span>
      <span>Settings</span>
    </button>
  </nav>

  <main class="content">
    {#if tab === "paste"}
      <section class="section hotkey">
        <div class="section-label">Hotkey</div>
        <div class="hotkey-keys">
          <kbd>⌘</kbd><span class="plus">+</span><kbd>⇧</kbd><span class="plus">+</span><kbd>P</kbd>
        </div>
        <p class="section-desc">Copy text → press hotkey → phi:// link in clipboard</p>
      </section>

      <section class="section">
        <div class="section-label">Upload</div>
        <form onsubmit={testPaste} class="form">
          <input type="text" placeholder="Paste text to encrypt..." bind:value={pasteInput} class="input" />
          <button type="submit" class="btn btn-primary" disabled={uploadLoading}>
            {uploadLoading ? "Uploading…" : "Upload"}
          </button>
        </form>
        {#if pasteResult}
          <div class="result {pasteResult.startsWith('phi://') ? 'success' : 'error'}">
            {#if pasteResult.startsWith('phi://')}
              <div class="result-link-row">
                <button type="button" class="result-link {copied ? 'copied' : ''}" onclick={() => copyToClipboard(pasteResult)}>{pasteResult}</button>
                <span class="copy-hint">{copied ? '✓ Copied' : 'Click to copy'}</span>
              </div>
            {:else}
              {pasteResult}
            {/if}
          </div>
        {/if}
      </section>

      <section class="section">
        <div class="section-label">Fetch</div>
        <form onsubmit={testFetch} class="form">
          <input type="text" placeholder="phi://<id>" bind:value={fetchId} class="input" />
          <button type="submit" class="btn btn-secondary" disabled={fetchLoading}>
            {fetchLoading ? "Fetching…" : "Fetch"}
          </button>
        </form>
        {#if fetchResult}
          <pre class="result fetch-result">{fetchResult}</pre>
        {/if}
      </section>

    {:else if tab === "browse"}
      <section class="section">
        <div class="section-header-row">
          <div class="section-label">Stored Pastes</div>
          <button class="btn btn-small" onclick={loadBrowse} disabled={browseLoading}>
            {browseLoading ? "Loading…" : "Refresh"}
          </button>
        </div>

        {#if browseError}
          <div class="result error">{browseError}</div>
        {:else if entries.length === 0 && !browseLoading}
          <p class="empty">No pastes yet. Use the hotkey or Upload tab to create one.</p>
        {:else}
          <div class="file-tree">
            {#each entries as entry (entry.id)}
              <div class="file-node" class:expanded={expandedId === entry.id}>
                <button class="file-row" onclick={() => toggleExpand(entry.id)}>
                  <span class="file-icon">{expandedId === entry.id ? "▼" : "▶"}</span>
                  <span class="file-id">{entry.id}</span>
                  <span class="file-meta">{formatSize(entry.size)}</span>
                  <span class="file-meta">{formatDate(entry.created)}</span>
                </button>
                {#if expandedId === entry.id}
                  <div class="file-detail">
                    {#if expandLoading}
                      <p class="loading-text">Decrypting…</p>
                    {:else}
                      <pre class="file-content">{expandedContent}</pre>
                    {/if}
                    <div class="file-actions">
                      <button class="btn btn-small" onclick={() => copyToClipboard(`phi://${entry.id}`)}>Copy Link</button>
                      <button class="btn btn-small" onclick={() => copyToClipboard(expandedContent)}>Copy Content</button>
                      <button class="btn btn-small btn-danger" onclick={() => deletePaste(entry.id)}>Delete</button>
                    </div>
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </section>

    {:else if tab === "settings"}
      <section class="section">
        <div class="section-label">Configuration</div>
        {#if settings}
          <div class="settings-grid">
            <div class="setting-row">
              <span class="setting-key">Storage</span>
              <span class="setting-val">{settings.storageType === "file" ? "Local File" : "S3/MinIO"}</span>
            </div>
            {#if settings.storageType === "file"}
              <div class="setting-row">
                <span class="setting-key">Path</span>
                <span class="setting-val mono">{settings.storagePath}</span>
              </div>
            {:else}
              <div class="setting-row">
                <span class="setting-key">S3 Endpoint</span>
                <span class="setting-val mono">{settings.s3Endpoint}</span>
              </div>
              <div class="setting-row">
                <span class="setting-key">S3 Bucket</span>
                <span class="setting-val mono">{settings.s3Bucket}</span>
              </div>
            {/if}
            <div class="setting-row">
              <span class="setting-key">User Prefix</span>
              <span class="setting-val mono">{settings.userPrefix}</span>
            </div>
            <div class="setting-row">
              <span class="setting-key">Encryption Key</span>
              <span class="setting-val mono">{settings.keyFingerprint}</span>
            </div>
            <div class="setting-row">
              <span class="setting-key">Hotkey</span>
              <span class="setting-val">
                <kbd>⌘</kbd><span class="plus">+</span><kbd>⇧</kbd><span class="plus">+</span><kbd>P</kbd>
              </span>
            </div>
          </div>
        {:else}
          <p class="empty">Loading settings…</p>
        {/if}
      </section>

      <section class="section">
        <div class="section-label">About</div>
        <div class="settings-grid">
          <div class="setting-row">
            <span class="setting-key">Version</span>
            <span class="setting-val">0.1.0</span>
          </div>
          <div class="setting-row">
            <span class="setting-key">Encryption</span>
            <span class="setting-val">AES-256-GCM (client-side)</span>
          </div>
          <div class="setting-row">
            <span class="setting-key">Protocol</span>
            <span class="setting-val mono">phi://</span>
          </div>
        </div>
      </section>
    {/if}

    {#if toast}
      <div class="toast" role="status">{toast}</div>
    {/if}

    {#if !isTauri}
      <div class="banner">
        Run <code>just dev</code> — Tauri API not available in browser
      </div>
    {/if}
  </main>
</div>

<style>
  @import url('https://fonts.googleapis.com/css2?family=DM+Sans:ital,opsz,wght@0,9..40,400;0,9..40,500;0,9..40,600;0,9..40,700&display=swap');

  :global(body) {
    margin: 0;
    padding: 0;
    overflow: hidden;
  }

  .app {
    display: flex;
    height: 100vh;
    font-family: 'DM Sans', system-ui, sans-serif;
    background: linear-gradient(165deg, #0f0f12 0%, #1a1a20 50%, #141418 100%);
    color: #e4e4e7;
  }

  /* Sidebar */
  .sidebar {
    width: 180px;
    min-width: 180px;
    background: rgba(0, 0, 0, 0.25);
    border-right: 1px solid rgba(255, 255, 255, 0.06);
    display: flex;
    flex-direction: column;
    padding: 1rem 0.5rem;
    gap: 0.25rem;
  }

  .logo {
    padding: 0.5rem 0.75rem 1.25rem;
  }

  .logo-text {
    font-size: 1.25rem;
    font-weight: 700;
    letter-spacing: -0.02em;
    background: linear-gradient(135deg, #fff 0%, #a1a1aa 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  .nav-item {
    all: unset;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-radius: 8px;
    font-size: 0.875rem;
    cursor: pointer;
    color: #a1a1aa;
    transition: all 0.15s;
  }

  .nav-item:hover {
    background: rgba(255, 255, 255, 0.06);
    color: #e4e4e7;
  }

  .nav-item.active {
    background: rgba(99, 102, 241, 0.15);
    color: #c7d2fe;
  }

  .nav-icon {
    width: 1.25rem;
    text-align: center;
    font-size: 0.8rem;
  }

  /* Content */
  .content {
    flex: 1;
    overflow-y: auto;
    padding: 1.5rem;
  }

  .section {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    padding: 1.25rem;
    margin-bottom: 1rem;
  }

  .section-label {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #71717a;
    margin-bottom: 0.75rem;
  }

  .section-header-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.75rem;
  }

  .section-header-row .section-label {
    margin-bottom: 0;
  }

  .section-desc {
    font-size: 0.8125rem;
    color: #a1a1aa;
    margin: 0.5rem 0 0;
  }

  .hotkey { text-align: center; }

  .hotkey-keys {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.25rem;
  }

  kbd {
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    padding: 0.25rem 0.5rem;
    border-radius: 6px;
    font-size: 0.8rem;
    font-family: inherit;
  }

  .plus {
    color: #52525b;
    font-size: 0.75rem;
  }

  .form {
    display: flex;
    gap: 0.5rem;
  }

  .input {
    flex: 1;
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    padding: 0.6rem 0.875rem;
    font-size: 0.875rem;
    color: #e4e4e7;
    font-family: inherit;
  }

  .input::placeholder { color: #52525b; }

  .input:focus {
    outline: none;
    border-color: rgba(99, 102, 241, 0.5);
    box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.15);
  }

  .btn {
    padding: 0.5rem 0.875rem;
    border-radius: 8px;
    font-size: 0.8125rem;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    border: none;
    transition: all 0.15s;
  }

  .btn-primary {
    background: linear-gradient(135deg, #6366f1 0%, #4f46e5 100%);
    color: white;
  }

  .btn-primary:hover {
    background: linear-gradient(135deg, #818cf8 0%, #6366f1 100%);
    transform: translateY(-1px);
  }

  .btn-secondary {
    background: rgba(255, 255, 255, 0.08);
    color: #e4e4e7;
    border: 1px solid rgba(255, 255, 255, 0.1);
  }

  .btn-secondary:hover { background: rgba(255, 255, 255, 0.12); }

  .btn-small {
    padding: 0.3rem 0.6rem;
    font-size: 0.75rem;
    background: rgba(255, 255, 255, 0.06);
    color: #a1a1aa;
    border: 1px solid rgba(255, 255, 255, 0.08);
  }

  .btn-small:hover { background: rgba(255, 255, 255, 0.1); color: #e4e4e7; }

  .btn-danger { color: #fca5a5; border-color: rgba(239, 68, 68, 0.2); }
  .btn-danger:hover { background: rgba(239, 68, 68, 0.15); color: #fecaca; }

  .btn:disabled { opacity: 0.5; cursor: not-allowed; transform: none; }

  /* Results */
  .result {
    margin-top: 0.75rem;
    font-size: 0.8125rem;
    padding: 0.75rem;
    border-radius: 8px;
  }

  .result.success {
    background: rgba(34, 197, 94, 0.1);
    border: 1px solid rgba(34, 197, 94, 0.2);
  }

  .result.error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.2);
    color: #fca5a5;
  }

  .result-link-row { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }

  .result-link {
    all: unset;
    cursor: pointer;
    word-break: break-all;
    color: #86efac;
    font-family: inherit;
    font-size: inherit;
  }

  .result-link:hover { text-decoration: underline; }
  .result-link.copied { color: #4ade80; transition: color 0.2s; }

  .copy-hint { font-size: 0.7rem; color: #71717a; transition: color 0.2s; }

  .fetch-result {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.06);
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* File tree */
  .file-tree { display: flex; flex-direction: column; gap: 2px; }

  .file-node {
    border-radius: 8px;
    overflow: hidden;
  }

  .file-node.expanded {
    background: rgba(255, 255, 255, 0.02);
  }

  .file-row {
    all: unset;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.5rem 0.6rem;
    cursor: pointer;
    border-radius: 6px;
    font-size: 0.8125rem;
    transition: background 0.1s;
    box-sizing: border-box;
  }

  .file-row:hover { background: rgba(255, 255, 255, 0.05); }

  .file-icon {
    font-size: 0.625rem;
    color: #52525b;
    width: 0.75rem;
    flex-shrink: 0;
  }

  .file-id {
    flex: 1;
    color: #c7d2fe;
    font-family: 'DM Sans', monospace;
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-meta {
    font-size: 0.7rem;
    color: #52525b;
    white-space: nowrap;
  }

  .file-detail {
    padding: 0 0.6rem 0.6rem 1.85rem;
  }

  .file-content {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 6px;
    padding: 0.6rem;
    font-size: 0.75rem;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 200px;
    overflow-y: auto;
    margin: 0 0 0.5rem;
    color: #d4d4d8;
  }

  .file-actions {
    display: flex;
    gap: 0.35rem;
  }

  .loading-text {
    font-size: 0.75rem;
    color: #71717a;
    margin: 0.25rem 0;
  }

  .empty {
    font-size: 0.8125rem;
    color: #52525b;
    text-align: center;
    padding: 2rem 0;
    margin: 0;
  }

  /* Settings */
  .settings-grid { display: flex; flex-direction: column; gap: 0.5rem; }

  .setting-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.5rem 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }

  .setting-row:last-child { border-bottom: none; }

  .setting-key {
    font-size: 0.8125rem;
    color: #a1a1aa;
  }

  .setting-val {
    font-size: 0.8125rem;
    color: #e4e4e7;
    text-align: right;
  }

  .mono {
    font-family: 'DM Sans', monospace;
    font-size: 0.75rem;
    color: #c7d2fe;
  }

  /* Toast */
  .toast {
    position: fixed;
    bottom: 1.5rem;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.5rem 1rem;
    background: rgba(34, 197, 94, 0.9);
    color: #052e16;
    border-radius: 8px;
    font-size: 0.8125rem;
    font-weight: 500;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
    z-index: 1000;
    animation: toast-in 0.2s ease-out;
  }

  @keyframes toast-in {
    from { opacity: 0; transform: translateX(-50%) translateY(8px); }
    to { opacity: 1; transform: translateX(-50%) translateY(0); }
  }

  .banner {
    margin-top: 1rem;
    padding: 0.75rem 1rem;
    background: rgba(251, 191, 36, 0.1);
    border: 1px solid rgba(251, 191, 36, 0.2);
    border-radius: 8px;
    font-size: 0.8125rem;
    color: #fcd34d;
  }

  .banner code {
    background: rgba(0, 0, 0, 0.2);
    padding: 0.15rem 0.35rem;
    border-radius: 4px;
  }
</style>
