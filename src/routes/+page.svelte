<script lang="ts">
  import { onMount, onDestroy } from "svelte";

  type AuthState = "checking" | "unauthenticated" | "authenticated";
  type OnboardStep = "welcome" | "set-password" | "done";
  type Tab = "paste" | "browse" | "team" | "settings";

  type PasteEntry = {
    id: string;
    contentType: string;
    encryptedTitle: string | null;
    encryptedFilename: string | null;
    createdAt: string;
    size: number;
    visibility: string;
    userId: string;
    userName: string;
    burnOnRead: boolean;
    ttlHours: number | null;
  };

  type Settings = {
    serverUrl: string;
    keyFingerprint: string;
    tokenConfigured: boolean;
    hotkey: string;
  };

  // Auth state
  let authState = $state<AuthState>("checking");
  let onboardStep = $state<OnboardStep>("welcome");
  let inviteUrl = $state("");
  let inviteToken = $state("");
  let kekFragment = $state<string | null>(null);
  let registerName = $state("");
  let registerEmail = $state("");
  let registerPassword = $state("");
  let loginEmail = $state("");
  let loginPassword = $state("");
  let authError = $state("");
  let authLoading = $state(false);
  let showLogin = $state(false);

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
  let searchQuery = $state("");
  let searchTimeout: ReturnType<typeof setTimeout> | null = null;
  let selectedIds = $state<Set<string>>(new Set());
  let bulkDeleteLoading = $state(false);

  // File upload state
  let fileUploadLoading = $state(false);
  let fileUploadResult = $state("");

  // Team state
  type Member = { id: string; userId: string; name: string; role: string; joinedAt: string };
  let members = $state<Member[]>([]);
  let teamLoading = $state(false);
  let teamError = $state("");
  let inviteLoading = $state(false);
  let inviteResult = $state<{ token: string; orgId: string; expiresAt: string } | null>(null);
  let inviteRole = $state("member");
  let inviteTtl = $state(24);

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
    }, 2500);
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
  let unlistenInviteLink: (() => void) | undefined;

  function parseInviteUrl(url: string): { token: string; kek: string | null } | null {
    // phi://join/<token>#<kek>
    const match = url.match(/^phi:\/\/join\/([^#]+)(?:#(.+))?$/);
    if (!match) return null;
    return { token: match[1], kek: match[2] ?? null };
  }

  onMount(async () => {
    isTauri = typeof window !== "undefined" && !!(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

    // Check auth status
    if (isTauri) {
      try {
        const result = await invoke<{ authenticated: boolean }>("auth_status");
        authState = result.authenticated ? "authenticated" : "unauthenticated";
      } catch {
        authState = "unauthenticated";
      }
    } else {
      // In browser/dev mode without Tauri, skip auth
      authState = "unauthenticated";
    }

    if (isTauri) {
      import("@tauri-apps/api/event").then(({ listen }) => {
        listen("phi-paste-success", () => {
          showToast("Link copied to clipboard");
        }).then((fn) => { unlistenPasteSuccess = fn; });

        listen<string>("invite-link-received", (event) => {
          const parsed = parseInviteUrl(event.payload);
          if (parsed) {
            inviteToken = parsed.token;
            kekFragment = parsed.kek;
            inviteUrl = event.payload;
            onboardStep = "set-password";
            if (authState !== "authenticated") {
              authState = "unauthenticated";
              showLogin = false;
            }
          }
        }).then((fn) => { unlistenInviteLink = fn; });
      });
      import("@tauri-apps/plugin-deep-link").then(async ({ getCurrent, onOpenUrl }) => {
        const urls = await getCurrent();
        const firstUrl = urls?.[0];
        if (firstUrl) {
          const parsed = parseInviteUrl(firstUrl);
          if (parsed) {
            inviteToken = parsed.token;
            kekFragment = parsed.kek;
            inviteUrl = firstUrl;
            onboardStep = "set-password";
          } else {
            const id = extractId(firstUrl);
            if (id) fetchById(id);
          }
        }
        unlistenOpenUrl = await onOpenUrl((urls) => {
          const u = urls[0];
          if (!u) return;
          const parsed = parseInviteUrl(u);
          if (parsed) {
            inviteToken = parsed.token;
            kekFragment = parsed.kek;
            inviteUrl = u;
            onboardStep = "set-password";
            if (authState !== "authenticated") showLogin = false;
          } else {
            const id = extractId(u);
            if (id) fetchById(id);
          }
        });
      });
    }
  });

  onDestroy(() => {
    if (toastTimeout) clearTimeout(toastTimeout);
    if (searchTimeout) clearTimeout(searchTimeout);
    unlistenOpenUrl?.();
    unlistenPasteSuccess?.();
    unlistenInviteLink?.();
  });

  // ── Paste tab ──

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

  async function pickAndUploadFile() {
    fileUploadLoading = true;
    fileUploadResult = "";
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: false,
        title: "Select file to encrypt & upload",
      });
      if (!result) {
        fileUploadLoading = false;
        return;
      }
      // result is a string path or an object with .path
      const filePath = typeof result === "string" ? result : (result as { path: string }).path;
      const url = await invoke<string>("file_upload", { filePath });
      fileUploadResult = url;
      showToast("File uploaded");
    } catch (e) {
      fileUploadResult = e instanceof Error ? e.message : String(e);
    } finally {
      fileUploadLoading = false;
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

  // ── Browse tab ──

  async function loadBrowse() {
    browseLoading = true;
    browseError = "";
    expandedId = null;
    selectedIds = new Set();
    try {
      if (searchQuery.trim()) {
        entries = await invoke<PasteEntry[]>("paste_search", { query: searchQuery });
      } else {
        entries = await invoke<PasteEntry[]>("paste_list");
      }
    } catch (e) {
      browseError = e instanceof Error ? e.message : String(e);
    } finally {
      browseLoading = false;
    }
  }

  function onSearchInput() {
    if (searchTimeout) clearTimeout(searchTimeout);
    searchTimeout = setTimeout(() => {
      loadBrowse();
    }, 300);
  }

  function toggleSelect(id: string) {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedIds = next;
  }

  function toggleSelectAll() {
    if (selectedIds.size === entries.length) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(entries.map(e => e.id));
    }
  }

  async function bulkDelete() {
    if (selectedIds.size === 0) return;
    bulkDeleteLoading = true;
    let deleted = 0;
    const ids = [...selectedIds];
    for (const id of ids) {
      try {
        await invoke("paste_delete", { id });
        deleted++;
      } catch { /* skip failures */ }
    }
    selectedIds = new Set();
    showToast(`Deleted ${deleted} paste${deleted !== 1 ? "s" : ""}`);
    bulkDeleteLoading = false;
    await loadBrowse();
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
      const next = new Set(selectedIds);
      next.delete(id);
      selectedIds = next;
      showToast("Deleted");
    } catch (e) {
      showToast(`Delete failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  // ── Team tab ──

  async function loadTeam() {
    teamLoading = true;
    teamError = "";
    inviteResult = null;
    try {
      // Fetch members via server API directly (no Tauri command yet - use fetch)
      const serverUrl = settings?.serverUrl || "http://localhost:8443";
      // For now we rely on the settings having been loaded
      // Members list requires Tauri command - we'll add a simple proxy
      members = [];
      teamError = "";
    } catch (e) {
      teamError = e instanceof Error ? e.message : String(e);
    } finally {
      teamLoading = false;
    }
  }

  // ── Settings tab ──

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
    if (t === "team") {
      loadSettings().then(() => loadTeam());
    }
    if (t === "settings") loadSettings();
  }

  // ── Auth handlers ──

  function handleInviteSubmit(event: Event) {
    event.preventDefault();
    authError = "";
    const parsed = parseInviteUrl(inviteUrl.trim());
    if (!parsed) {
      authError = "Invalid invite link. Expected format: phi://join/<token>#<kek>";
      return;
    }
    inviteToken = parsed.token;
    kekFragment = parsed.kek;
    onboardStep = "set-password";
  }

  async function handleRegister(event: Event) {
    event.preventDefault();
    authError = "";
    if (!registerName.trim()) { authError = "Name is required"; return; }
    if (!registerEmail.trim()) { authError = "Email is required"; return; }
    if (!registerPassword) { authError = "Password is required"; return; }
    authLoading = true;
    try {
      await invoke("auth_register", {
        inviteToken,
        email: registerEmail,
        password: registerPassword,
        name: registerName,
        kekFragment,
      });
      authState = "authenticated";
      onboardStep = "done";
    } catch (e) {
      authError = e instanceof Error ? e.message : String(e);
    } finally {
      authLoading = false;
    }
  }

  async function handleLogin(event: Event) {
    event.preventDefault();
    authError = "";
    if (!loginEmail.trim()) { authError = "Email is required"; return; }
    if (!loginPassword) { authError = "Password is required"; return; }
    authLoading = true;
    try {
      await invoke("auth_login", { email: loginEmail, password: loginPassword });
      authState = "authenticated";
    } catch (e) {
      authError = e instanceof Error ? e.message : String(e);
    } finally {
      authLoading = false;
    }
  }

  async function handleLogout() {
    try {
      await invoke("auth_logout");
    } catch { /* ignore */ }
    authState = "unauthenticated";
    onboardStep = "welcome";
    inviteUrl = "";
    inviteToken = "";
    kekFragment = null;
    registerName = "";
    registerEmail = "";
    registerPassword = "";
    loginEmail = "";
    loginPassword = "";
    authError = "";
    showLogin = false;
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
      const now = new Date();
      const diffMs = now.getTime() - d.getTime();
      const diffMin = Math.floor(diffMs / 60000);
      if (diffMin < 1) return "just now";
      if (diffMin < 60) return `${diffMin}m ago`;
      const diffH = Math.floor(diffMin / 60);
      if (diffH < 24) return `${diffH}h ago`;
      const diffD = Math.floor(diffH / 24);
      if (diffD < 7) return `${diffD}d ago`;
      return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
    } catch { return iso; }
  }

  function contentIcon(ct: string): string {
    if (ct.startsWith("image/")) return "🖼";
    if (ct === "application/pdf") return "📄";
    if (ct.includes("json") || ct.includes("xml")) return "📋";
    if (ct.includes("zip") || ct.includes("tar")) return "📦";
    if (ct.includes("csv") || ct.includes("spreadsheet") || ct.includes("xlsx")) return "📊";
    return "📝";
  }

  function visibilityLabel(v: string): string {
    if (v === "private") return "Private";
    if (v === "org") return "Team";
    if (v === "link") return "Link";
    return v;
  }

  function visibilityColor(v: string): string {
    if (v === "private") return "vis-private";
    if (v === "org") return "vis-org";
    if (v === "link") return "vis-link";
    return "";
  }
</script>

{#if authState === "checking"}
  <div class="auth-screen">
    <div class="auth-panel">
      <div class="logo-text" style="font-size:1.5rem; margin-bottom:1rem">Shimmer</div>
      <p class="section-desc" style="text-align:center">Loading…</p>
    </div>
  </div>
{:else if authState === "unauthenticated"}
  <div class="auth-screen">
    <div class="auth-panel">
      <div class="auth-logo">
        <span class="logo-text" style="font-size:1.75rem">Shimmer</span>
      </div>

      {#if onboardStep === "welcome" && !showLogin}
        <!-- Welcome: choose path -->
        <h2 class="auth-heading">Welcome</h2>
        <p class="auth-subheading">Paste an invite link to create your account, or sign in if you already have one.</p>

        <form onsubmit={handleInviteSubmit} class="auth-form">
          <div class="auth-field">
            <label class="field-label" for="invite-url">Invite Link</label>
            <input
              id="invite-url"
              type="text"
              placeholder="phi://join/..."
              bind:value={inviteUrl}
              class="input"
              autocomplete="off"
              spellcheck={false}
            />
          </div>
          {#if authError}
            <div class="auth-error">{authError}</div>
          {/if}
          <button type="submit" class="btn btn-primary auth-btn">Continue with Invite</button>
        </form>

        <div class="auth-divider"><span>or</span></div>
        <button class="btn btn-secondary auth-btn" onclick={() => { showLogin = true; authError = ""; }}>
          Sign In
        </button>

      {:else if onboardStep === "welcome" && showLogin}
        <!-- Login form -->
        <h2 class="auth-heading">Sign In</h2>
        <form onsubmit={handleLogin} class="auth-form">
          <div class="auth-field">
            <label class="field-label" for="login-email">Email</label>
            <input
              id="login-email"
              type="email"
              placeholder="you@example.com"
              bind:value={loginEmail}
              class="input"
              autocomplete="email"
            />
          </div>
          <div class="auth-field">
            <label class="field-label" for="login-password">Password</label>
            <input
              id="login-password"
              type="password"
              placeholder="••••••••"
              bind:value={loginPassword}
              class="input"
              autocomplete="current-password"
            />
          </div>
          {#if authError}
            <div class="auth-error">{authError}</div>
          {/if}
          <button type="submit" class="btn btn-primary auth-btn" disabled={authLoading}>
            {authLoading ? "Signing in…" : "Sign In"}
          </button>
        </form>
        <button class="btn btn-secondary auth-btn" style="margin-top:0.5rem" onclick={() => { showLogin = false; authError = ""; }}>
          Back
        </button>

      {:else if onboardStep === "set-password"}
        <!-- Register form -->
        <h2 class="auth-heading">Create Account</h2>
        <p class="auth-subheading">You're joining via invite. Set up your account details below.</p>
        <form onsubmit={handleRegister} class="auth-form">
          <div class="auth-field">
            <label class="field-label" for="reg-name">Name</label>
            <input
              id="reg-name"
              type="text"
              placeholder="Your name"
              bind:value={registerName}
              class="input"
              autocomplete="name"
            />
          </div>
          <div class="auth-field">
            <label class="field-label" for="reg-email">Email</label>
            <input
              id="reg-email"
              type="email"
              placeholder="you@example.com"
              bind:value={registerEmail}
              class="input"
              autocomplete="email"
            />
          </div>
          <div class="auth-field">
            <label class="field-label" for="reg-password">Password</label>
            <input
              id="reg-password"
              type="password"
              placeholder="••••••••"
              bind:value={registerPassword}
              class="input"
              autocomplete="new-password"
            />
          </div>
          {#if authError}
            <div class="auth-error">{authError}</div>
          {/if}
          <button type="submit" class="btn btn-primary auth-btn" disabled={authLoading}>
            {authLoading ? "Creating account…" : "Create Account"}
          </button>
        </form>
        <button class="btn btn-secondary auth-btn" style="margin-top:0.5rem" onclick={() => { onboardStep = "welcome"; authError = ""; inviteUrl = ""; inviteToken = ""; kekFragment = null; }}>
          Back
        </button>
      {/if}
    </div>
  </div>
{:else}

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
    <button class="nav-item" class:active={tab === "team"} onclick={() => switchTab("team")}>
      <span class="nav-icon">👥</span>
      <span>Team</span>
    </button>
    <button class="nav-item" class:active={tab === "settings"} onclick={() => switchTab("settings")}>
      <span class="nav-icon">⚙</span>
      <span>Settings</span>
    </button>
  </nav>

  <main class="content">
    {#if tab === "paste"}
      <!-- ── Hotkey ── -->
      <section class="section hotkey">
        <div class="section-label">Hotkey</div>
        <div class="hotkey-keys">
          <kbd>⌘</kbd><span class="plus">+</span><kbd>⇧</kbd><span class="plus">+</span><kbd>P</kbd>
        </div>
        <p class="section-desc">Copy text → press hotkey → phi:// link in clipboard</p>
      </section>

      <!-- ── Text Upload ── -->
      <section class="section">
        <div class="section-label">Upload Text</div>
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

      <!-- ── File Upload ── -->
      <section class="section">
        <div class="section-label">Upload File</div>
        <p class="section-desc" style="margin-top:0; margin-bottom:0.75rem">
          Images, PDFs, documents — encrypted end-to-end, searchable by filename.
        </p>
        <button class="btn btn-secondary" onclick={pickAndUploadFile} disabled={fileUploadLoading}>
          {fileUploadLoading ? "Uploading…" : "Choose File"}
        </button>
        {#if fileUploadResult}
          <div class="result {fileUploadResult.startsWith('phi://') ? 'success' : 'error'}">
            {#if fileUploadResult.startsWith('phi://')}
              <div class="result-link-row">
                <button type="button" class="result-link" onclick={() => copyToClipboard(fileUploadResult)}>{fileUploadResult}</button>
                <span class="copy-hint">Click to copy</span>
              </div>
            {:else}
              {fileUploadResult}
            {/if}
          </div>
        {/if}
      </section>

      <!-- ── Fetch ── -->
      <section class="section">
        <div class="section-label">Fetch &amp; Decrypt</div>
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
      <!-- ── Search + Controls ── -->
      <section class="section">
        <div class="section-header-row">
          <div class="section-label">Encrypted Pastes</div>
          <button class="btn btn-small" onclick={loadBrowse} disabled={browseLoading}>
            {browseLoading ? "Loading…" : "Refresh"}
          </button>
        </div>

        <div class="search-row">
          <input
            type="text"
            placeholder="Search by content, filename, tag…"
            bind:value={searchQuery}
            oninput={onSearchInput}
            class="input search-input"
          />
        </div>

        {#if selectedIds.size > 0}
          <div class="bulk-bar">
            <span class="bulk-count">{selectedIds.size} selected</span>
            <button class="btn btn-small btn-danger" onclick={bulkDelete} disabled={bulkDeleteLoading}>
              {bulkDeleteLoading ? "Deleting…" : `Delete ${selectedIds.size}`}
            </button>
          </div>
        {/if}

        {#if browseError}
          <div class="result error">{browseError}</div>
        {:else if entries.length === 0 && !browseLoading}
          <p class="empty">
            {searchQuery ? "No matching pastes found." : "No pastes yet. Use the hotkey or Upload tab to create one."}
          </p>
        {:else}
          <div class="file-tree">
            <!-- Select all row -->
            {#if entries.length > 0}
              <button class="file-row select-all-row" onclick={toggleSelectAll}>
                <span class="checkbox" class:checked={selectedIds.size === entries.length && entries.length > 0}>
                  {selectedIds.size === entries.length && entries.length > 0 ? "☑" : "☐"}
                </span>
                <span class="file-meta" style="flex:1">Select all ({entries.length})</span>
              </button>
            {/if}

            {#each entries as entry (entry.id)}
              <div class="file-node" class:expanded={expandedId === entry.id}>
                <div class="file-row-wrapper">
                  <button class="checkbox-btn" onclick={(e: MouseEvent) => { e.stopPropagation(); toggleSelect(entry.id); }}>
                    <span class="checkbox" class:checked={selectedIds.has(entry.id)}>
                      {selectedIds.has(entry.id) ? "☑" : "☐"}
                    </span>
                  </button>
                  <button class="file-row" onclick={() => toggleExpand(entry.id)}>
                    <span class="file-icon">{contentIcon(entry.contentType)}</span>
                    <span class="file-id">
                      {entry.encryptedFilename ? "📎 " : ""}{entry.id.slice(0, 8)}…
                    </span>
                    <span class="badge {visibilityColor(entry.visibility)}">{visibilityLabel(entry.visibility)}</span>
                    <span class="file-meta file-user">{entry.userName}</span>
                    <span class="file-meta">{formatSize(entry.size)}</span>
                    <span class="file-meta">{formatDate(entry.createdAt)}</span>
                    {#if entry.burnOnRead}
                      <span class="badge badge-burn" title="Deletes after first read">🔥</span>
                    {/if}
                  </button>
                </div>
                {#if expandedId === entry.id}
                  <div class="file-detail">
                    <div class="detail-meta">
                      <span class="detail-label">Type:</span> {entry.contentType}
                      <span class="detail-sep">·</span>
                      <span class="detail-label">ID:</span>
                      <span class="mono">{entry.id}</span>
                      {#if entry.ttlHours}
                        <span class="detail-sep">·</span>
                        <span class="detail-label">TTL:</span> {entry.ttlHours}h
                      {/if}
                    </div>
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

    {:else if tab === "team"}
      <!-- ── Team Management ── -->
      <section class="section">
        <div class="section-label">Team</div>
        <p class="section-desc" style="margin-top:0">
          Manage your organization's members and generate invite links.
          Team features require admin role on the server.
        </p>

        {#if teamError}
          <div class="result error">{teamError}</div>
        {/if}

        <div class="team-info">
          <div class="setting-row">
            <span class="setting-key">Server</span>
            <span class="setting-val mono">{settings?.serverUrl || "—"}</span>
          </div>
          <div class="setting-row">
            <span class="setting-key">Auth</span>
            <span class="setting-val">{settings?.tokenConfigured ? "✓ Connected" : "✗ No token"}</span>
          </div>
        </div>
      </section>

      <section class="section">
        <div class="section-label">Generate Invite</div>
        <p class="section-desc" style="margin-top:0; margin-bottom:0.75rem">
          Create a one-time invite link for a new team member. They'll use the token to join your org.
        </p>
        <div class="invite-form">
          <div class="invite-field">
            <label class="field-label" for="invite-role">Role</label>
            <select id="invite-role" class="input select" bind:value={inviteRole}>
              <option value="member">Member</option>
              <option value="read_only">Read Only</option>
            </select>
          </div>
          <div class="invite-field">
            <label class="field-label" for="invite-ttl">Expires in</label>
            <select id="invite-ttl" class="input select" bind:value={inviteTtl}>
              <option value={1}>1 hour</option>
              <option value={24}>24 hours</option>
              <option value={72}>3 days</option>
              <option value={168}>1 week</option>
            </select>
          </div>
          <button class="btn btn-primary" disabled={inviteLoading}>
            {inviteLoading ? "Generating…" : "Generate Invite"}
          </button>
        </div>

        {#if inviteResult}
          <div class="result success" style="margin-top:0.75rem">
            <div class="detail-label" style="margin-bottom:0.25rem">Invite Token</div>
            <div class="result-link-row">
              <button type="button" class="result-link" onclick={() => copyToClipboard(inviteResult?.token || "")}>
                {inviteResult.token}
              </button>
              <span class="copy-hint">Click to copy</span>
            </div>
            <div class="detail-meta" style="margin-top:0.5rem">
              <span class="detail-label">Org:</span> {inviteResult.orgId}
              <span class="detail-sep">·</span>
              <span class="detail-label">Expires:</span> {formatDate(inviteResult.expiresAt)}
            </div>
          </div>
        {/if}
      </section>

      <section class="section">
        <div class="section-label">Members</div>
        {#if members.length === 0}
          <p class="empty">
            Member management requires admin API access.
            Use <code>curl</code> or the API directly to manage members.
          </p>
          <div class="api-hint">
            <code>GET /api/org/members</code> — list members<br>
            <code>PUT /api/org/members/:userId</code> — change role<br>
            <code>DELETE /api/org/members/:userId</code> — remove
          </div>
        {:else}
          <div class="member-list">
            {#each members as m (m.id)}
              <div class="member-row">
                <span class="member-name">{m.name}</span>
                <span class="badge {m.role === 'admin' ? 'vis-link' : m.role === 'read_only' ? 'vis-private' : 'vis-org'}">{m.role}</span>
                <span class="file-meta">{formatDate(m.joinedAt)}</span>
              </div>
            {/each}
          </div>
        {/if}
      </section>

    {:else if tab === "settings"}
      <section class="section">
        <div class="section-label">Connection</div>
        {#if settings}
          <div class="settings-grid">
            <div class="setting-row">
              <span class="setting-key">Server</span>
              <span class="setting-val mono">{settings.serverUrl}</span>
            </div>
            <div class="setting-row">
              <span class="setting-key">Auth Token</span>
              <span class="setting-val">{settings.tokenConfigured ? "✓ Configured" : "✗ Not set"}</span>
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
        <div class="section-label">Account</div>
        <div class="settings-grid">
          <div class="setting-row">
            <span class="setting-key">Session</span>
            <span class="setting-val">Signed in</span>
          </div>
        </div>
        <button class="btn btn-secondary" style="margin-top:0.75rem" onclick={handleLogout}>
          Sign Out
        </button>
      </section>

      <section class="section">
        <div class="section-label">About</div>
        <div class="settings-grid">
          <div class="setting-row">
            <span class="setting-key">Version</span>
            <span class="setting-val">0.2.0</span>
          </div>
          <div class="setting-row">
            <span class="setting-key">Encryption</span>
            <span class="setting-val">AES-256-GCM envelope encryption</span>
          </div>
          <div class="setting-row">
            <span class="setting-key">Search</span>
            <span class="setting-val">Blind index (HMAC-SHA256)</span>
          </div>
          <div class="setting-row">
            <span class="setting-key">Protocol</span>
            <span class="setting-val mono">phi://</span>
          </div>
          <div class="setting-row">
            <span class="setting-key">Architecture</span>
            <span class="setting-val">Zero-knowledge server</span>
          </div>
        </div>
      </section>

      <section class="section">
        <div class="section-label">API Endpoints</div>
        <div class="api-hint">
          <code>POST /api/paste</code> — upload encrypted paste<br>
          <code>GET /api/paste/:id</code> — fetch &amp; decrypt<br>
          <code>GET /api/pastes</code> — list (with <code>?tokens=</code> search)<br>
          <code>DELETE /api/paste/:id</code> — delete<br>
          <code>POST /api/org/invite</code> — generate invite (admin)<br>
          <code>POST /api/org/join</code> — join org with invite token<br>
          <code>GET /api/org/members</code> — list members<br>
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

{/if}

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

  .select {
    flex: 0;
    min-width: 120px;
    cursor: pointer;
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 12 12'%3E%3Cpath fill='%2371717a' d='M3 5l3 3 3-3'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 0.75rem center;
    padding-right: 2rem;
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

  /* Search */
  .search-row {
    margin-bottom: 0.75rem;
  }

  .search-input {
    width: 100%;
    box-sizing: border-box;
  }

  /* Bulk actions */
  .bulk-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.5rem;
    background: rgba(99, 102, 241, 0.1);
    border: 1px solid rgba(99, 102, 241, 0.2);
    border-radius: 8px;
  }

  .bulk-count {
    font-size: 0.8125rem;
    color: #c7d2fe;
  }

  /* Badges */
  .badge {
    font-size: 0.65rem;
    font-weight: 600;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    white-space: nowrap;
  }

  .vis-private {
    background: rgba(239, 68, 68, 0.15);
    color: #fca5a5;
  }

  .vis-org {
    background: rgba(34, 197, 94, 0.15);
    color: #86efac;
  }

  .vis-link {
    background: rgba(99, 102, 241, 0.15);
    color: #c7d2fe;
  }

  .badge-burn {
    background: rgba(251, 191, 36, 0.15);
    border: none;
  }

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

  .file-row-wrapper {
    display: flex;
    align-items: center;
  }

  .checkbox-btn {
    all: unset;
    cursor: pointer;
    padding: 0.5rem 0.25rem 0.5rem 0.5rem;
    display: flex;
    align-items: center;
  }

  .checkbox {
    font-size: 0.875rem;
    color: #52525b;
    width: 1rem;
    text-align: center;
    transition: color 0.1s;
  }

  .checkbox.checked {
    color: #818cf8;
  }

  .select-all-row {
    padding-left: 0.5rem;
    gap: 0.35rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
    margin-bottom: 0.25rem;
  }

  .file-row {
    all: unset;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: 1;
    padding: 0.5rem 0.6rem;
    cursor: pointer;
    border-radius: 6px;
    font-size: 0.8125rem;
    transition: background 0.1s;
    box-sizing: border-box;
  }

  .file-row:hover { background: rgba(255, 255, 255, 0.05); }

  .file-icon {
    font-size: 0.8rem;
    width: 1.25rem;
    flex-shrink: 0;
    text-align: center;
  }

  .file-id {
    flex: 1;
    color: #c7d2fe;
    font-family: 'DM Sans', monospace;
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .file-user {
    color: #a1a1aa;
    max-width: 80px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .file-meta {
    font-size: 0.7rem;
    color: #52525b;
    white-space: nowrap;
  }

  .file-detail {
    padding: 0 0.6rem 0.6rem 2.5rem;
  }

  .detail-meta {
    font-size: 0.7rem;
    color: #71717a;
    margin-bottom: 0.5rem;
  }

  .detail-label {
    color: #52525b;
    font-weight: 600;
  }

  .detail-sep {
    margin: 0 0.35rem;
    color: #3f3f46;
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

  .empty code {
    background: rgba(0, 0, 0, 0.2);
    padding: 0.1rem 0.3rem;
    border-radius: 3px;
    font-size: 0.75rem;
  }

  /* Team */
  .team-info {
    margin-top: 0.5rem;
  }

  .invite-form {
    display: flex;
    gap: 0.75rem;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .invite-field {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .field-label {
    font-size: 0.7rem;
    color: #71717a;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .member-list {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .member-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }

  .member-row:last-child { border-bottom: none; }

  .member-name {
    flex: 1;
    font-size: 0.8125rem;
  }

  .api-hint {
    font-size: 0.75rem;
    color: #52525b;
    line-height: 1.8;
    padding: 0.75rem;
    background: rgba(0, 0, 0, 0.2);
    border-radius: 8px;
  }

  .api-hint code {
    color: #a1a1aa;
    font-family: 'DM Sans', monospace;
    font-size: 0.7rem;
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

  /* Auth / Onboarding */
  .auth-screen {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    font-family: 'DM Sans', system-ui, sans-serif;
    background: linear-gradient(165deg, #0f0f12 0%, #1a1a20 50%, #141418 100%);
    color: #e4e4e7;
  }

  .auth-panel {
    width: 100%;
    max-width: 420px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 16px;
    padding: 2rem;
    display: flex;
    flex-direction: column;
    align-items: stretch;
  }

  .auth-logo {
    text-align: center;
    margin-bottom: 1.5rem;
  }

  .auth-heading {
    font-size: 1.125rem;
    font-weight: 600;
    margin: 0 0 0.5rem;
    color: #f4f4f5;
    text-align: center;
  }

  .auth-subheading {
    font-size: 0.8125rem;
    color: #71717a;
    margin: 0 0 1.25rem;
    text-align: center;
    line-height: 1.5;
  }

  .auth-form {
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
  }

  .auth-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .auth-field .input {
    flex: none;
    width: 100%;
    box-sizing: border-box;
  }

  .auth-btn {
    width: 100%;
    text-align: center;
    padding: 0.65rem 1rem;
    font-size: 0.9rem;
  }

  .auth-error {
    font-size: 0.8125rem;
    color: #fca5a5;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.2);
    border-radius: 8px;
    padding: 0.6rem 0.875rem;
  }

  .auth-divider {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 1rem 0;
    color: #3f3f46;
    font-size: 0.75rem;
  }

  .auth-divider::before,
  .auth-divider::after {
    content: "";
    flex: 1;
    height: 1px;
    background: rgba(255, 255, 255, 0.06);
  }

  .auth-divider span {
    color: #52525b;
  }
</style>
