<script lang="ts">
  import { onMount, onDestroy } from "svelte";

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

  function playSuccessSound() {
    // Only play in-app (manual copy). Hotkey plays Tink.aiff natively via Rust.
  }

  onMount(() => {
    isTauri = typeof window !== "undefined" && !!(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

    if (isTauri) {
      import("@tauri-apps/api/event").then(({ listen }) => {
        listen("phi-paste-success", () => {
          playSuccessSound();
          showToast("Link copied to clipboard");
        }).then((fn) => {
          unlistenPasteSuccess = fn;
        });
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
    if (!id) {
      fetchResult = "Enter a phi:// ID";
      return;
    }
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

  async function copyResult() {
    if (!pasteResult) return;
    try {
      if (isTauri) {
        const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
        await writeText(pasteResult);
      } else {
        await navigator.clipboard.writeText(pasteResult);
      }
      copied = true;
      showToast("Copied!");
      setTimeout(() => { copied = false; }, 1500);
    } catch {
      showToast("Failed to copy");
    }
  }
</script>

<main class="app">
  <header class="header">
    <h1 class="title">Shimmer</h1>
    <p class="tagline">Secure PHI sharing</p>
  </header>

  <section class="section hotkey">
    <div class="section-label">Hotkey</div>
    <div class="hotkey-keys">
      <kbd>⌘</kbd><span class="plus">+</span><kbd>⇧</kbd><span class="plus">+</span><kbd>P</kbd>
    </div>
    <p class="section-desc">Capture clipboard → encrypt → upload → copy phi:// link</p>
  </section>

  <section class="section">
    <div class="section-label">Upload</div>
    <form onsubmit={testPaste} class="form">
      <input
        type="text"
        placeholder="Paste text to encrypt..."
        bind:value={pasteInput}
        class="input"
      />
      <button type="submit" class="btn btn-primary" disabled={uploadLoading}>
        {uploadLoading ? "Uploading…" : "Upload"}
      </button>
    </form>
    {#if pasteResult}
      <div class="result {pasteResult.startsWith('phi://') ? 'success' : 'error'}">
        {#if pasteResult.startsWith('phi://')}
          <div class="result-link-row">
            <button type="button" class="result-link {copied ? 'copied' : ''}" onclick={copyResult}>{pasteResult}</button>
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
      <input
        type="text"
        placeholder="phi://&lt;id&gt;"
        bind:value={fetchId}
        class="input"
      />
      <button type="submit" class="btn btn-secondary" disabled={fetchLoading}>
        {fetchLoading ? "Fetching…" : "Fetch"}
      </button>
    </form>
    {#if fetchResult}
      <pre class="result fetch-result">{fetchResult}</pre>
    {/if}
  </section>

  {#if toast}
    <div class="toast" role="status">{toast}</div>
  {/if}

  {#if !isTauri}
    <div class="banner">
      Run <code>just dev</code> — Tauri API not available in browser
    </div>
  {/if}
</main>

<style>
  @import url('https://fonts.googleapis.com/css2?family=DM+Sans:ital,opsz,wght@0,9..40,400;0,9..40,500;0,9..40,600;0,9..40,700&display=swap');

  .app {
    min-height: 100vh;
    padding: 2rem 1.5rem;
    font-family: 'DM Sans', system-ui, sans-serif;
    background: linear-gradient(165deg, #0f0f12 0%, #1a1a20 50%, #141418 100%);
    color: #e4e4e7;
  }

  .header {
    margin-bottom: 2rem;
  }

  .title {
    font-size: 1.75rem;
    font-weight: 700;
    letter-spacing: -0.02em;
    margin: 0;
    background: linear-gradient(135deg, #fff 0%, #a1a1aa 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  .tagline {
    font-size: 0.875rem;
    color: #71717a;
    margin: 0.25rem 0 0;
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

  .section-desc {
    font-size: 0.8125rem;
    color: #a1a1aa;
    margin: 0.5rem 0 0;
  }

  .hotkey {
    text-align: center;
  }

  .hotkey-keys {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.25rem;
  }

  .hotkey kbd {
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    padding: 0.35rem 0.6rem;
    border-radius: 6px;
    font-size: 0.875rem;
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
    font-size: 0.9rem;
    color: #e4e4e7;
    font-family: inherit;
  }

  .input::placeholder {
    color: #52525b;
  }

  .input:focus {
    outline: none;
    border-color: rgba(99, 102, 241, 0.5);
    box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.15);
  }

  .btn {
    padding: 0.6rem 1rem;
    border-radius: 8px;
    font-size: 0.875rem;
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

  .btn-secondary:hover {
    background: rgba(255, 255, 255, 0.12);
  }

  .btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
    transform: none;
  }

  .result-link-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .copy-hint {
    font-size: 0.75rem;
    color: #71717a;
  }

  .toast {
    position: fixed;
    bottom: 1.5rem;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.5rem 1rem;
    background: rgba(34, 197, 94, 0.9);
    color: #052e16;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
    z-index: 1000;
    animation: toast-in 0.2s ease-out;
  }

  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateX(-50%) translateY(0);
    }
  }

  .result {
    margin-top: 1rem;
    font-size: 0.875rem;
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

  .result-link {
    all: unset;
    cursor: pointer;
    word-break: break-all;
    color: #86efac;
    font-family: inherit;
    font-size: inherit;
  }

  .result-link:hover {
    text-decoration: underline;
  }

  .result-link.copied {
    color: #4ade80;
    transition: color 0.2s;
  }

  .copy-hint {
    transition: color 0.2s;
  }

  .fetch-result {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.06);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .banner {
    margin-top: 1.5rem;
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
