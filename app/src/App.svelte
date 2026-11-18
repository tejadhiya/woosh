<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import Telemetry from "./views/Telemetry.svelte";
  import Findings from "./views/Findings.svelte";
  import History from "./views/History.svelte";
  import Reap from "./views/Reap.svelte";
  import Gallery from "./views/Gallery.svelte";
  import {
    api,
    onTelemetry,
    onFinding,
    onReceipt,
    DOMAIN_GLYPH,
    DOMAIN_LABEL,
    type Frame,
    type LogLine,
    type ScanResult,
    type DiskStats,
    type Permissions,
    type Receipt,
    type ReapMode,
    type DomainKey,
    type Finding,
  } from "./lib/api";
  import { bytes } from "./lib/format";
  import { RingBuffer } from "./lib/ring.svelte";

  /** Deep link to the exact Settings pane. Full Disk Access cannot be requested
   *  programmatically, so pointing at it is the most we can do (plan §8.3). */
  const FDA_URL =
    "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles";

  let perms = $state<Permissions | null>(null);
  let disk = $state<DiskStats | null>(null);
  let result = $state<ScanResult | null>(null);
  let scanning = $state(false);
  let deep = $state(false);
  let error = $state<string | null>(null);

  let frame = $state<Frame | null>(null);
  const log = new RingBuffer<LogLine>(600);
  let tab = $state<"findings" | "feed">("feed");
  let filter = $state<DomainKey | null>(null);
  let workspace = $state<"reclaim" | "history" | "telemetry">("reclaim");
  let cacheTime = $state<number | null>(null);

  let selected = $state(new SvelteSet<string>());
  let mode = $state<ReapMode>("trash");
  let confirming = $state(false);
  let reaping = $state(false);
  let receipts = $state<Receipt[]>([]);
  let lastRun = $state<{ reclaimed: number; failed: number; mode: ReapMode } | null>(null);

  let showAbout = $state(false);
  let copiedContact = $state(false);
  let contactTimer: ReturnType<typeof setTimeout> | null = null;

  function copyContactEmail() {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      navigator.clipboard.writeText("tejadhiya2@gmail.com").then(() => {
        copiedContact = true;
        if (contactTimer) clearTimeout(contactTimer);
        contactTimer = setTimeout(() => {
          copiedContact = false;
        }, 1800);
      });
    }
  }

  const home = $derived(perms?.home ?? "");
  const findings = $derived(
    (result?.findings ?? []).filter((f) => !filter || f.domain === filter)
  );
  const domains = $derived.by(() => {
    const m = new Map<DomainKey, { bytes: number; count: number }>();
    for (const f of result?.findings ?? []) {
      const e = m.get(f.domain) ?? { bytes: 0, count: 0 };
      e.bytes += f.bytes;
      e.count += 1;
      m.set(f.domain, e);
    }
    return [...m.entries()].sort((a, b) => b[1].bytes - a[1].bytes);
  });
  const findingsById = $derived.by(() => {
    const m = new Map<string, Finding>();
    for (const f of result?.findings ?? []) {
      m.set(f.id, f);
    }
    return m;
  });
  const selectedFindings = $derived.by(() => {
    const list: Finding[] = [];
    for (const id of selected) {
      const f = findingsById.get(id);
      if (f) list.push(f);
    }
    return list;
  });
  const selectedBytes = $derived(
    selectedFindings.reduce((a, f) => a + f.bytes, 0)
  );
  const visibleSelectedCount = $derived(
    findings.filter((f) => selected.has(f.id)).length
  );
  const hiddenSelectedCount = $derived(
    filter ? selected.size - visibleSelectedCount : 0
  );
  const hasCaution = $derived(selectedFindings.some((f) => f.risk === "caution"));
  const hasLargeFile = $derived(
    selectedFindings.some((f) => f.domain === "large_file")
  );

  $effect(() => {
    if (hasLargeFile && mode === "permanent") {
      mode = "trash";
    }
  });

  onMount(() => {
    void refresh();
    const un1 = onTelemetry((f) => {
      frame = f;
      if (f.lines.length) log.pushMany(f.lines);
    });
    const un2 = onReceipt((r) => {
      receipts = [...receipts, r];
      const line: LogLine = {
        t_us: r.micros,
        kind: r.success ? "woosh" : "warn",
        glyph: r.success ? "✦" : "⚠",
        text: r.success
          ? `WOOSH  ${r.name} → ${r.method === "trash" ? "Trash" : r.method}   ${bytes(r.bytes)}`
          : `REFUSED  ${r.name} — ${r.error ?? "unknown"}`,
      };
      log.push(line);
    });
    const un3 = onFinding((f: Finding) => {
      if (result) {
        if (!findingIds.has(f.id)) {
          findingIds.add(f.id);
          result.findings.push(f);
        }
      }
    });

    function handleGlobalKeys(e: KeyboardEvent) {
      if (e.key === "Escape" && showAbout) {
        e.preventDefault();
        e.stopPropagation();
        showAbout = false;
        return;
      }

      const hasMod = e.metaKey || e.ctrlKey;
      if (!hasMod) return;

      if (e.code === "Digit1" || e.key === "1") {
        e.preventDefault();
        e.stopPropagation();
        workspace = "reclaim";
      } else if (e.code === "Digit2" || e.key === "2") {
        e.preventDefault();
        e.stopPropagation();
        workspace = "history";
      } else if (e.code === "Digit3" || e.key === "3") {
        e.preventDefault();
        e.stopPropagation();
        workspace = "telemetry";
      } else if (e.key === "i" || e.key === "I" || e.code === "KeyI") {
        e.preventDefault();
        e.stopPropagation();
        showAbout = !showAbout;
      } else if (e.key === "r" || e.key === "R" || e.code === "KeyR") {
        e.preventDefault();
        e.stopPropagation();
        if (!scanning && !reaping) void sweep();
      }
    }
    window.addEventListener("keydown", handleGlobalKeys, { capture: true });

    return () => {
      void un1.then((f) => f());
      void un2.then((f) => f());
      void un3.then((f) => f());
      window.removeEventListener("keydown", handleGlobalKeys, { capture: true });
    };
  });

  const findingIds = new Set<string>();

  function formatRelativeTime(timestamp: number): string {
    const diffSec = Math.floor((Date.now() - timestamp * 1000) / 1000);
    if (diffSec < 60) return "just now";
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)} min ago`;
    if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} h ago`;
    return `${Math.floor(diffSec / 86400)} d ago`;
  }

  async function refresh() {
    try {
      const [p, d, cached] = await Promise.all([
        api.permissions(),
        api.disk(),
        api.loadCache(),
      ]);
      perms = p;
      disk = d;
      if (cached && !result) {
        result = cached.scan_result;
        cacheTime = cached.timestamp;
        findingIds.clear();
        for (const f of result.findings) {
          findingIds.add(f.id);
        }
        tab = "findings";
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function sweep() {
    if (scanning) return;
    const prevResult = result;
    const prevCacheTime = cacheTime;
    cacheTime = null;
    error = null;
    log.clear();
    frame = null;
    receipts = [];
    lastRun = null;
    selected = new SvelteSet();
    scanning = true;
    tab = "feed";
    findingIds.clear();

    try {
      result = {
        disk: disk ?? {
          mount: "/",
          total: 0,
          used: 0,
          free: 0,
          percent_used: 0,
          arch: "",
          hostname: "",
        },
        findings: [],
        totals: { reclaimable: 0, shared: 0, items: 0, by_domain: [] },
        denied: [],
        duration_us: 0,
        entries_scanned: 0,
        roots: [],
      };
      const finalResult = await api.scan(deep);
      result = finalResult;
      disk = result.disk;
      findingIds.clear();
      for (const f of result.findings) {
        findingIds.add(f.id);
      }
      // Pre-select the unambiguously regenerable wins, never anything costly.
      const auto = result.findings.filter((f) => f.risk === "safe").map((f) => f.id);
      selected = new SvelteSet(auto);
      tab = "findings";
    } catch (e) {
      error = String(e);
      // Restore previous result on error if no new findings arrived (Bug B5)
      if (result && result.findings.length === 0 && prevResult) {
        result = prevResult;
        cacheTime = prevCacheTime;
        findingIds.clear();
        for (const f of prevResult.findings) {
          findingIds.add(f.id);
        }
      }
    } finally {
      scanning = false;
    }
  }

  function toggle(id: string) {
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
  }

  function toggleMany(ids: string[], on: boolean) {
    for (const id of ids) on ? selected.add(id) : selected.delete(id);
  }

  function pick(key: DomainKey | null) {
    filter = key;
    workspace = "reclaim";
    if (result) tab = "findings";
  }

  async function doReap() {
    confirming = false;
    reaping = true;
    receipts = [];
    tab = "feed";
    try {
      const s = await api.reap([...selected], mode);
      lastRun = { reclaimed: s.reclaimed, failed: s.failed, mode };
      if (mode !== "dry_run") {
        const gone = new Set(s.receipts.filter((r) => r.success).map((r) => r.id));
        if (result) {
          result = { ...result, findings: result.findings.filter((f) => !gone.has(f.id)) };
          if (filter && !result.findings.some((f) => f.domain === filter)) filter = null;
        }
        selected = new SvelteSet([...selected].filter((id) => !gone.has(id)));
        disk = await api.disk();
      }
    } catch (e) {
      error = String(e);
    } finally {
      reaping = false;
    }
  }
  const isGallery =
    typeof window !== "undefined" &&
    (window.location.search.includes("gallery") ||
      window.location.hash.includes("gallery"));
</script>

{#if isGallery}
  <Gallery />
{:else}
<div class="app">
  <!-- Unified macOS Titlebar with Native Drag Clearance -->
  <header class="titlebar" data-tauri-drag-region>
    <div class="brand-zone" data-tauri-drag-region>
      <span class="brand-dot"></span>
      <span class="brand-name">WOOSH</span>
    </div>

    <!-- Workspace Mode Switcher (⌘1, ⌘2, ⌘3) -->
    <div class="workspace-switcher" data-tauri-drag-region>
      <button
        class="ws-btn"
        class:active={workspace === "reclaim"}
        onclick={() => (workspace = "reclaim")}
        title="Storage Reclaimer (⌘1)"
      >
        <span>Reclaim</span>
        {#if result && result.totals.reclaimable > 0}
          <span class="ws-pill mono">+{bytes(result.totals.reclaimable)}</span>
        {/if}
      </button>

      <button
        class="ws-btn"
        class:active={workspace === "history"}
        onclick={() => (workspace = "history")}
        title="Reclamation History & Put Back (⌘2)"
      >
        <span>History</span>
        <span class="ws-kbd">⌘2</span>
      </button>

      <button
        class="ws-btn"
        class:active={workspace === "telemetry"}
        onclick={() => (workspace = "telemetry")}
        title="Live Telemetry & Activity (⌘3)"
      >
        <span>Telemetry</span>
        {#if scanning || reaping}
          <span class="live-dot"></span>
        {:else}
          <span class="ws-kbd">⌘3</span>
        {/if}
      </button>
    </div>

    {#if cacheTime && !scanning}
      <div class="cache-indicator mono" data-tauri-drag-region title="Previous scan results">
        <span class="cache-dot">●</span>
        <span>Results from {formatRelativeTime(cacheTime)}</span>
        <span class="pill-divider">·</span>
        <button class="rescan-btn" onclick={sweep}>Rescan</button>
      </div>
    {/if}

    {#if disk}
      <div class="host-pill mono" data-tauri-drag-region>
        <span class="host-icon">#</span>
        <span>{disk.hostname}</span>
        <span class="pill-divider">·</span>
        <span class="dim">{disk.arch}</span>
      </div>
    {/if}

    <button
      type="button"
      class="about-btn"
      onclick={() => (showAbout = true)}
      title="About Woosh (⌘I)"
      aria-label="About Woosh"
    >
      About
    </button>
  </header>

  <div class="main-layout">
    <!-- Pro Command Sidebar -->
    <aside class="sidebar">
      {#if disk}
        <div class="storage-card">
          <div class="storage-head">
            <span class="storage-free-val mono">{bytes(disk.free)}</span>
            <span class="storage-free-lbl">Free of {bytes(disk.total)}</span>
          </div>

          <div class="storage-bar" role="progressbar" aria-valuenow={disk.percent_used} aria-valuemin={0} aria-valuemax={100}>
            <div class="bar-used" style="width: {disk.percent_used}%"></div>
            {#if result && result.totals.reclaimable > 0}
              <div
                class="bar-reclaim"
                style="width: {Math.min(
                  disk.percent_used,
                  (result.totals.reclaimable / disk.total) * 100
                )}%; left: {Math.max(
                  0,
                  disk.percent_used - (result.totals.reclaimable / disk.total) * 100
                )}%"
                title="Reclaimable Space"
              ></div>
            {/if}
          </div>

          {#if result}
            <div class="reclaim-banner">
              <span class="reclaim-icon">●</span>
              <span class="mono bold">+{bytes(result.totals.reclaimable)}</span>
              <span class="dim">reclaimable</span>
            </div>
          {/if}
        </div>
      {/if}

      <!-- Domains Navigation List -->
      <div class="nav-section-title">STORAGE DOMAINS</div>
      <nav class="domains-nav">
        <button
          class="nav-item"
          class:active={filter === null}
          onclick={() => pick(null)}
        >
          <div class="item-icon-wrap icon-all">≡</div>
          <div class="item-info">
            <span class="item-name">Everything</span>
            {#if result}
              <span class="item-count-tag">{result.totals.items}</span>
            {/if}
          </div>
          <span class="item-size mono">{result ? bytes(result.totals.reclaimable) : "—"}</span>
        </button>

        {#each domains as [key, d] (key)}
          <button
            class="nav-item"
            class:active={filter === key}
            onclick={() => pick(key)}
          >
            <div class="item-icon-wrap icon-{key}">{DOMAIN_GLYPH[key]}</div>
            <div class="item-info">
              <span class="item-name">{DOMAIN_LABEL[key]}</span>
              <span class="item-count-tag">{d.count}</span>
            </div>
            <span class="item-size mono">{bytes(d.bytes)}</span>
          </button>
        {/each}
      </nav>

      <!-- Sidebar Foot / Full Disk Access -->
      <div class="sidebar-foot">
        {#if perms && !perms.full_disk_access}
          <button class="fda-banner" onclick={() => openUrl(FDA_URL)}>
            <div class="fda-top">
              <span class="fda-dot">!</span>
              <strong>Full Disk Access Required</strong>
            </div>
            <p>System caches and simulator runtimes are hidden without access.</p>
            <span class="fda-btn">Open Settings ↗</span>
          </button>
        {/if}
        {#if result && result.denied.length > 0}
          <div class="denied-status mono">
            ⚠ {result.denied.length} unreadable path{result.denied.length === 1 ? "" : "s"}
          </div>
        {/if}
      </div>
    </aside>

    <!-- Content Workspace -->
    <section class="workspace">
      {#if workspace === "reclaim"}
        <!-- High-Performance Command Toolbar -->
        <header class="workspace-bar">
          <div class="tab-segments">
            <button
              class="segment-btn"
              class:active={tab === "findings"}
              onclick={() => (tab = "findings")}
            >
              <span>Findings</span>
              {#if result}
                <span class="count-pill">{result.totals.items}</span>
              {/if}
            </button>
            <button
              class="segment-btn"
              class:active={tab === "feed"}
              onclick={() => (tab = "feed")}
            >
              <span>Telemetry</span>
              {#if scanning || reaping}
                <span class="live-dot"></span>
              {/if}
            </button>
          </div>

          <div class="bar-spacer"></div>

          <!-- Deep Scan Option -->
          <label class="deep-toggle" title="Scan entire home directory without depth limits (thorough, takes longer)">
            <input type="checkbox" bind:checked={deep} disabled={scanning} />
            <span>Deep Scan</span>
          </label>

          <!-- Primary Action Sweep Button -->
          <button
            class="btn btn-primary sweep-btn"
            onclick={sweep}
            disabled={scanning || reaping}
          >
            {#if scanning}
              <span class="pulsing">Sweeping…</span>
            {:else}
              <span>Sweep Now</span>
              <span class="kbd">⌘R</span>
            {/if}
          </button>
        </header>
      {/if}

      {#if error}
        <div class="error-banner">
          <span class="error-icon">!</span>
          <span>{error}</span>
        </div>
      {/if}

      <!-- Main Body Pane -->
      <main class="pane-body">
        {#if workspace === "history"}
          <History />
        {:else if workspace === "telemetry" || tab === "feed"}
          <Telemetry {frame} {log} scanning={scanning || reaping} />
        {:else if result}
          <Findings
            {findings}
            {selected}
            {home}
            domainFilter={filter}
            onToggle={toggle}
            onToggleMany={toggleMany}
          />
        {:else}
          <!-- Gorgeous Hero Idle State -->
          <div class="hero-idle">
            <div class="hero-halo">
              <div class="halo-inner">
                <span class="hero-symbol">✦</span>
              </div>
            </div>

            <h1 class="hero-title">High-Performance Storage Reclaimer</h1>
            <p class="hero-desc">
              Scan and recover gigabytes of developer waste — Docker virtual disks,
              Xcode Simulator devices, stale node_modules, Python venvs, and large files.
            </p>

            <button
              class="btn btn-primary hero-cta-btn"
              onclick={sweep}
              disabled={scanning || reaping}
            >
              <span>Start Fast Sweep</span>
              <span class="kbd">⌘R</span>
            </button>

            <div class="hero-specs-row">
              <div class="spec-chip">
                <span class="spec-dot safe"></span>
                <span>Pure-Rust Engine</span>
              </div>
              <div class="spec-chip">
                <span class="spec-dot blue"></span>
                <span>Zero Subprocesses</span>
              </div>
              <div class="spec-chip">
                <span class="spec-dot amber"></span>
                <span>Trash-Backed Safety</span>
              </div>
              <div class="spec-chip">
                <span class="spec-dot violet"></span>
                <span>1M+ Entries/sec</span>
              </div>
            </div>
          </div>
        {/if}
      </main>

      <!-- Bottom Reclaim Floating Action Bar -->
      {#if result && !scanning && workspace === "reclaim"}
        <footer class="action-footer">
          <div class="footer-sel-info">
            {#if selected.size > 0}
              <div class="sel-reclaim-text">
                <span class="sel-bytes mono">{bytes(selectedBytes)}</span>
                <span class="sel-meta">
                  selected across <strong>{selected.size}</strong> item{selected.size === 1 ? "" : "s"}
                  {#if hiddenSelectedCount > 0}
                    <span class="hidden-pill">· {hiddenSelectedCount} hidden</span>
                  {/if}
                </span>
              </div>
            {:else}
              <div class="sel-reclaim-text">
                <span class="sel-none-txt dim">Select items to reclaim storage</span>
              </div>
            {/if}

            {#if lastRun}
              <div class="last-run-pill mono">
                {#if lastRun.mode === "trash"}
                  <span class="mint">✓ {bytes(lastRun.reclaimed)} in Trash</span>
                  <span class="dim">(empty Trash to free disk)</span>
                {:else if lastRun.mode === "permanent"}
                  <span class="mint">✓ {bytes(lastRun.reclaimed)} permanently reaped</span>
                {:else}
                  <span class="dim">✓ {bytes(lastRun.reclaimed)} previewed</span>
                {/if}
                {#if lastRun.failed > 0}
                  <span class="amber">· {lastRun.failed} refused</span>
                {/if}
              </div>
            {/if}
          </div>

          <div class="footer-controls">
            <select class="mode-select" bind:value={mode} aria-label="Deletion Mode">
              <option value="trash">Move to Trash (Put Back supported)</option>
              <option value="dry_run">Dry Run (Touch-free simulation)</option>
              <option value="permanent" disabled={hasLargeFile}>
                Delete Permanently{hasLargeFile ? " (Disabled for large files)" : ""}
              </option>
            </select>

            <button
              class="btn {mode === 'permanent' ? 'btn-danger' : 'btn-primary'} woosh-action-btn"
              disabled={selected.size === 0 || reaping}
              onclick={() => (confirming = true)}
            >
              {#if reaping}
                <span class="pulsing">Wooshing…</span>
              {:else if mode === "dry_run"}
                <span>Preview Run</span>
              {:else}
                <span>Woosh {selected.size > 0 ? `(${bytes(selectedBytes)})` : ""}</span>
              {/if}
            </button>
          </div>
        </footer>
      {/if}
    </section>
  </div>

  {#if confirming}
    <Reap
      count={selected.size}
      total={selectedBytes}
      {mode}
      {hasCaution}
      items={selectedFindings}
      onCancel={() => (confirming = false)}
      onConfirm={doReap}
    />
  {/if}

  {#if showAbout}
    <div class="modal-scrim" onclick={() => (showAbout = false)} role="presentation">
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <!-- svelte-ignore a11y_interactive_supports_focus -->
      <div
        class="about-modal"
        role="dialog"
        tabindex="-1"
        aria-modal="true"
        aria-labelledby="about-dialog-title"
        onclick={(e) => e.stopPropagation()}
      >
        <button
          type="button"
          class="modal-close-btn"
          onclick={() => (showAbout = false)}
          aria-label="Close"
        >
          ×
        </button>

        <div class="about-hero">
          <h2 id="about-dialog-title" class="about-title">Woosh</h2>
          <span class="about-badge-version mono">v0.1.1 · Apple Silicon Native</span>
          <p class="about-tagline">
            Ultra-fast developer storage reclaimer and garbage cleaner for macOS.
          </p>
        </div>

        <div class="about-info-grid">
          <div class="about-info-row">
            <span class="info-label">Developer</span>
            <span class="info-val strong">Tejadhiya</span>
          </div>

          <div class="about-info-row">
            <span class="info-label">Contact / Feedback</span>
            <div class="info-email-group">
              <button
                type="button"
                class="email-link mono"
                onclick={() => openUrl("mailto:tejadhiya2@gmail.com")}
                title="Send email"
              >
                tejadhiya2@gmail.com
              </button>
              <button
                type="button"
                class="email-copy-pill"
                onclick={copyContactEmail}
              >
                {copiedContact ? "✓ Copied" : "Copy"}
              </button>
            </div>
          </div>

          <div class="about-info-row">
            <span class="info-label">Source Code</span>
            <button
              type="button"
              class="url-link mono"
              onclick={() => openUrl("https://github.com/tejadhiya/woosh")}
            >
              github.com/tejadhiya/woosh ↗
            </button>
          </div>

          <div class="about-info-row">
            <span class="info-label">Architecture</span>
            <span class="info-val mono dim">Rust (getattrlistbulk) + Tauri v2 + Svelte 5</span>
          </div>

          <div class="about-info-row">
            <span class="info-label">License</span>
            <span class="info-val">MIT OR Apache-2.0</span>
          </div>
        </div>

        <div class="about-footer-row">
          <button
            type="button"
            class="about-done-btn"
            onclick={() => (showAbout = false)}
          >
            Done
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-canvas);
    color: var(--ink);
  }

  /* macOS Titlebar Chrome */
  .titlebar {
    height: var(--titlebar);
    flex: none;
    display: flex;
    align-items: center;
    gap: 16px;
    padding-left: 88px; /* traffic lights margin */
    padding-right: 18px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--line);
    user-select: none;
  }
  .brand-zone {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .brand-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--mint-bright);
    box-shadow: 0 0 8px var(--mint-glow);
  }
  .brand-name {
    font-weight: 700;
    letter-spacing: 0.12em;
    font-size: 11.5px;
    color: var(--ink);
  }
  .badge-tag {
    font-size: 10px;
    font-weight: 600;
    color: var(--ink-faint);
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    letter-spacing: 0.06em;
  }

  /* Workspace Mode Switcher in Titlebar */
  .workspace-switcher {
    display: flex;
    align-items: center;
    gap: 3px;
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    padding: 2px;
  }
  .ws-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    font-weight: 500;
    color: var(--ink-dim);
    transition: all 120ms ease;
    cursor: pointer;
  }
  .ws-btn:hover {
    color: var(--ink);
  }
  .ws-btn.active {
    background: var(--bg-surface-elevated);
    color: var(--ink);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4), inset 0 1px 0 rgba(255, 255, 255, 0.08);
  }
  .ws-pill {
    font-size: 10.5px;
    color: var(--mint-bright);
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(16, 185, 129, 0.25);
    padding: 0 5px;
    border-radius: var(--radius-pill);
    font-weight: 600;
  }
  .ws-kbd {
    font-size: 9.5px;
    color: var(--ink-faint);
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    padding: 1px 4px;
    border-radius: var(--radius-xs);
  }
  .cache-indicator {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: var(--mint-bright);
    background: rgba(16, 185, 129, 0.1);
    border: 1px solid rgba(16, 185, 129, 0.25);
    padding: 3px 8px;
    border-radius: var(--radius-xs);
    margin-left: 12px;
  }
  .cache-dot {
    font-size: 11px;
  }

  .host-pill {
    margin-left: auto;
    font-size: 11.5px;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--ink-dim);
  }
  .host-icon {
    font-size: 13px;
    color: var(--ink-faint);
  }
  .pill-divider {
    color: var(--ink-ash);
  }

  .about-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-left: 10px;
    padding: 3px 8px;
    background: var(--bg-surface-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--ink-dim);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: all 120ms ease;
  }
  .about-btn:hover {
    color: var(--ink);
    border-color: var(--border-strong);
    background: var(--bg-hover);
  }

  /* Main Split View */
  .main-layout {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 270px minmax(0, 1fr);
  }

  /* Pro Command Sidebar */
  .sidebar {
    background: var(--bg-surface);
    border-right: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
  }

  /* Hero Storage Breakdown Card */
  .storage-card {
    margin: 16px 14px 12px;
    padding: 16px;
    background: linear-gradient(180deg, var(--bg-surface-elevated) 0%, rgba(19, 23, 34, 0.4) 100%);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
  }
  .storage-head {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .storage-free-val {
    font-size: 24px;
    font-weight: 700;
    color: var(--ink);
    letter-spacing: -0.02em;
  }
  .storage-free-lbl {
    font-size: 12.5px;
    color: var(--ink-dim);
  }
  .storage-bar {
    position: relative;
    height: 10px;
    background: #1c2331;
    border-radius: var(--radius-pill);
    margin: 14px 0 10px;
    overflow: hidden;
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.6);
  }
  .bar-used {
    position: absolute;
    inset: 0 auto 0 0;
    background: linear-gradient(90deg, #374151, #4b5563);
    border-radius: var(--radius-pill);
  }
  .bar-reclaim {
    position: absolute;
    top: 0;
    bottom: 0;
    background: linear-gradient(90deg, #10b981, #34d399);
    border-radius: var(--radius-pill);
    box-shadow: 0 0 10px rgba(52, 211, 153, 0.6);
  }
  .reclaim-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--mint-bright);
    margin-top: 4px;
  }
  .reclaim-icon {
    font-size: 14px;
  }
  .bold { font-weight: 600; }

  /* Navigation List */
  .nav-section-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--ink-faint);
    padding: 10px 18px 6px;
  }
  .domains-nav {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 10px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .nav-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-md);
    border: 1px solid transparent;
    text-align: left;
    transition: all 100ms ease;
  }
  .nav-item:hover {
    background: var(--bg-surface-elevated);
    border-color: var(--line-soft);
  }
  .nav-item.active {
    background: linear-gradient(90deg, rgba(16, 185, 129, 0.16) 0%, rgba(16, 185, 129, 0.03) 100%);
    border-color: rgba(16, 185, 129, 0.3);
    color: var(--ink);
  }
  .item-icon-wrap {
    width: 28px;
    height: 28px;
    display: grid;
    place-content: center;
    border-radius: var(--radius-sm);
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    font-size: 12.5px;
    color: var(--ink-dim);
    flex: none;
  }
  .nav-item.active .item-icon-wrap {
    color: var(--mint-bright);
    border-color: rgba(16, 185, 129, 0.4);
    background: rgba(16, 185, 129, 0.1);
  }
  .item-info {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .item-name {
    font-size: 13.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-count-tag {
    font-size: 11px;
    color: var(--ink-faint);
    background: var(--bg-canvas);
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--line);
  }
  .item-size {
    font-size: 12.5px;
    color: var(--ink-dim);
    flex: none;
  }
  .nav-item.active .item-size {
    color: var(--mint-bright);
    font-weight: 600;
  }

  /* Sidebar Footer */
  .sidebar-foot {
    flex: none;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-top: 1px solid var(--line-soft);
  }
  .fda-banner {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-radius: var(--radius-md);
    background: rgba(251, 191, 36, 0.08);
    border: 1px solid rgba(251, 191, 36, 0.25);
    text-align: left;
    transition: background 120ms ease;
  }
  .fda-banner:hover {
    background: rgba(251, 191, 36, 0.12);
  }
  .fda-top {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--amber);
  }
  .fda-dot {
    font-weight: 700;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--amber);
    color: #07080a;
    display: grid;
    place-content: center;
    font-size: 10px;
  }
  .fda-banner p {
    margin: 0;
    font-size: 11.5px;
    color: var(--ink-dim);
    line-height: 1.4;
  }
  .fda-btn {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--amber);
    margin-top: 4px;
  }
  .denied-status {
    font-size: 11.5px;
    color: var(--ink-faint);
    padding-left: 2px;
  }

  /* About Modal Dialog */
  .modal-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    display: grid;
    place-items: center;
    z-index: 9999;
    padding: 20px;
  }
  .about-modal {
    position: relative;
    width: 100%;
    max-width: 440px;
    background: var(--bg-surface);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-sheet);
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    animation: modalPop 150ms cubic-bezier(0.16, 1, 0.3, 1);
  }
  @keyframes modalPop {
    from {
      opacity: 0;
      transform: scale(0.96) translateY(4px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }
  .modal-close-btn {
    position: absolute;
    top: 14px;
    right: 14px;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    background: var(--bg-surface-elevated);
    border: 1px solid var(--separator);
    border-radius: 50%;
    color: var(--ink-dim);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
    transition: all 120ms ease;
  }
  .modal-close-btn:hover {
    color: var(--fg);
    border-color: var(--border-strong);
  }
  .about-hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 6px;
  }
  .about-title {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--fg);
    margin: 0;
  }
  .about-badge-version {
    font-size: 11px;
    color: var(--ink-dim);
    background: var(--bg-surface-elevated);
    border: 1px solid var(--separator);
    padding: 2px 8px;
    border-radius: var(--radius-pill);
  }
  .about-tagline {
    font-size: 12.5px;
    color: var(--ink-dim);
    line-height: 1.45;
    margin: 4px 0 0 0;
    max-width: 340px;
  }
  .about-info-grid {
    display: flex;
    flex-direction: column;
    gap: 10px;
    background: var(--bg);
    border: 1px solid var(--separator);
    border-radius: var(--radius-md);
    padding: 12px 14px;
  }
  .about-info-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
  }
  .info-label {
    color: var(--ink-faint);
    flex: none;
  }
  .info-val {
    color: var(--ink-dim);
    text-align: right;
  }
  .info-val.strong {
    color: var(--fg);
    font-weight: 600;
  }
  .info-val.dim {
    font-size: 11px;
  }
  .info-email-group {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .email-link {
    background: none;
    border: none;
    padding: 0;
    font-size: 12px;
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
  }
  .email-link:hover {
    color: var(--accent-hover);
  }
  .email-copy-pill {
    font-size: 10px;
    font-weight: 600;
    color: var(--ink-dim);
    background: var(--bg-surface-elevated);
    border: 1px solid var(--separator);
    border-radius: var(--radius-xs);
    padding: 2px 6px;
    cursor: pointer;
    transition: all 120ms ease;
  }
  .email-copy-pill:hover {
    color: var(--fg);
    border-color: var(--border-strong);
  }
  .url-link {
    background: none;
    border: none;
    padding: 0;
    font-size: 11.5px;
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
  }
  .url-link:hover {
    color: var(--accent-hover);
  }
  .about-footer-row {
    display: flex;
    justify-content: flex-end;
  }
  .about-done-btn {
    padding: 6px 18px;
    background: var(--accent);
    color: #ffffff;
    border: none;
    border-radius: var(--radius-sm);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .about-done-btn:hover {
    background: var(--accent-hover);
  }

  /* Content Workspace */
  .workspace {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg-canvas);
  }

  .workspace-bar {
    flex: none;
    height: 52px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 18px;
    border-bottom: 1px solid var(--line);
    background: var(--bg-surface);
  }
  .tab-segments {
    display: flex;
    gap: 3px;
    background: var(--bg-canvas);
    padding: 3px;
    border-radius: var(--radius-md);
    border: 1px solid var(--line);
  }
  .segment-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-dim);
    transition: all 100ms ease;
  }
  .segment-btn:hover {
    color: var(--ink);
  }
  .segment-btn.active {
    background: var(--bg-surface-elevated);
    color: var(--ink);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4), inset 0 1px 0 rgba(255, 255, 255, 0.08);
  }
  .count-pill {
    font-size: 11px;
    color: var(--ink-faint);
    background: var(--bg-canvas);
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--line);
  }
  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--mint-bright);
    box-shadow: 0 0 6px var(--mint-bright);
  }
  .bar-spacer {
    flex: 1;
  }
  .deep-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--ink-dim);
    cursor: pointer;
    user-select: none;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
  }
  .deep-toggle:hover {
    color: var(--ink);
  }
  .sweep-btn {
    padding: 8px 16px;
    font-size: 13.5px;
  }

  .error-banner {
    flex: none;
    padding: 10px 18px;
    background: rgba(248, 113, 113, 0.1);
    border-bottom: 1px solid rgba(248, 113, 113, 0.25);
    color: #fca5a5;
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .pane-body {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  /* Hero Idle State */
  .hero-idle {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    text-align: center;
    padding: 0 48px;
  }
  .hero-halo {
    width: 72px;
    height: 72px;
    border-radius: 50%;
    background: radial-gradient(circle, rgba(16, 185, 129, 0.25) 0%, transparent 70%);
    display: grid;
    place-content: center;
    margin-bottom: 4px;
  }
  .halo-inner {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: var(--bg-surface-elevated);
    border: 1px solid rgba(16, 185, 129, 0.35);
    display: grid;
    place-content: center;
    box-shadow: 0 0 16px rgba(16, 185, 129, 0.2);
  }
  .hero-symbol {
    font-size: 22px;
    color: var(--mint-bright);
  }
  .hero-title {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    color: var(--ink);
    letter-spacing: -0.02em;
  }
  .hero-desc {
    margin: 0;
    max-width: 520px;
    color: var(--ink-dim);
    font-size: 14.5px;
    line-height: 1.6;
  }
  .hero-cta-btn {
    margin-top: 8px;
    padding: 10px 22px;
    font-size: 14.5px;
    border-radius: var(--radius-pill);
  }
  .hero-specs-row {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 16px;
  }
  .spec-chip {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--ink-dim);
    padding: 4px 10px;
    border-radius: var(--radius-pill);
    background: var(--bg-surface);
    border: 1px solid var(--line);
  }
  .spec-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
  .spec-dot.safe { background: var(--mint-bright); }
  .spec-dot.blue { background: var(--blue); }
  .spec-dot.amber { background: var(--amber); }
  .spec-dot.violet { background: var(--violet); }

  /* Floating Bottom Action Bar */
  .action-footer {
    flex: none;
    height: 62px;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 20px;
    border-top: 1px solid var(--line);
    background: var(--bg-surface);
    box-shadow: 0 -4px 16px rgba(0, 0, 0, 0.4);
  }
  .footer-sel-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sel-reclaim-text {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .sel-bytes {
    font-size: 19px;
    font-weight: 700;
    color: var(--mint-bright);
    letter-spacing: -0.02em;
  }
  .sel-meta {
    font-size: 13px;
    color: var(--ink-dim);
  }
  .sel-meta strong {
    color: var(--ink);
  }
  .hidden-pill {
    color: var(--amber);
  }
  .sel-none-txt {
    font-size: 13.5px;
  }
  .last-run-pill {
    font-size: 12px;
    display: flex;
    gap: 6px;
  }

  .footer-controls {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .mode-select {
    background: var(--bg-surface-elevated);
    color: var(--ink);
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    padding: 8px 12px;
    font-size: 13px;
    cursor: pointer;
  }
  .mode-select:focus {
    outline: none;
    border-color: var(--mint-bright);
  }
  .woosh-action-btn {
    padding: 9px 18px;
    font-size: 14px;
    min-width: 130px;
  }
</style>
