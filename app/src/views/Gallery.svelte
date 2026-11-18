<script lang="ts">
  import {
    Sidebar,
    SidebarItem,
    Toolbar,
    SegmentedControl,
    SearchField,
    Table,
    Inspector,
    Sheet,
    ProgressStrip,
    EmptyState,
  } from "../ui";
  import { bytes } from "../lib/format";

  // Gallery Controls
  const urlParams =
    typeof window !== "undefined"
      ? new URLSearchParams(window.location.search)
      : null;
  let theme = $state<"system" | "light" | "dark">(
    (urlParams?.get("theme") as any) || "light"
  );
  let activeTab = $state<"search" | "reclaim" | "history">(
    (urlParams?.get("tab") as any) || "reclaim"
  );
  let viewMode = $state<"list" | "projects">("list");
  let searchQuery = $state("");
  let filterChips = $state<string[]>(["ext:dmg", "size:>100MB"]);
  let inspectorVisible = $state(true);
  let sheetOpen = $state(urlParams?.get("sheet") === "1");
  let scanning = $state(urlParams?.get("scanning") === "1");

  // Apply theme to document
  $effect(() => {
    const root = document.documentElement;
    if (theme === "system") {
      root.removeAttribute("data-theme");
    } else {
      root.setAttribute("data-theme", theme);
    }
  });

  // Mock Findings Data
  interface MockFinding {
    id: string;
    name: string;
    domain: string;
    path: string;
    bytes: number;
    risk: "safe" | "caution" | "advisory";
    idleDays: number;
  }

  const mockFindings: MockFinding[] = [
    {
      id: "f1",
      name: "node_modules",
      domain: "node_modules",
      path: "/Users/dev/projects/web-app/node_modules",
      bytes: 842 * 1024 * 1024,
      risk: "safe",
      idleDays: 45,
    },
    {
      id: "f2",
      name: ".venv",
      domain: "venv",
      path: "/Users/dev/ai/model-training/.venv",
      bytes: 2.1 * 1024 * 1024 * 1024,
      risk: "safe",
      idleDays: 90,
    },
    {
      id: "f3",
      name: "Docker.raw",
      domain: "docker",
      path: "/Users/dev/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw",
      bytes: 24.5 * 1024 * 1024 * 1024,
      risk: "caution",
      idleDays: 14,
    },
    {
      id: "f4",
      name: "Xcode DeviceSupport 17.2",
      domain: "simulators",
      path: "/Users/dev/Library/Developer/Xcode/iOS DeviceSupport/17.2",
      bytes: 3.8 * 1024 * 1024 * 1024,
      risk: "advisory",
      idleDays: 120,
    },
    {
      id: "f5",
      name: "target",
      domain: "artifacts",
      path: "/Users/dev/rust/fast-indexer/target",
      bytes: 1.4 * 1024 * 1024 * 1024,
      risk: "safe",
      idleDays: 12,
    },
  ];

  // Mock Search Results Data
  interface MockSearchItem {
    id: string;
    name: string;
    kind: "app" | "folder" | "file";
    path: string;
    bytes: number;
    modified: string;
  }

  const mockSearchResults: MockSearchItem[] = [
    {
      id: "s1",
      name: "Xcode.app",
      kind: "app",
      path: "/Applications/Xcode.app",
      bytes: 14.8 * 1024 * 1024 * 1024,
      modified: "2 days ago",
    },
    {
      id: "s2",
      name: "main.rs",
      kind: "file",
      path: "/Users/dev/rust/fast-indexer/src/main.rs",
      bytes: 12400,
      modified: "1 hour ago",
    },
    {
      id: "s3",
      name: "node_modules",
      kind: "folder",
      path: "/Users/dev/projects/web-app/node_modules",
      bytes: 842 * 1024 * 1024,
      modified: "3 weeks ago",
    },
    {
      id: "s4",
      name: "Installer.dmg",
      kind: "file",
      path: "/Users/dev/Downloads/Installer.dmg",
      bytes: 780 * 1024 * 1024,
      modified: "Yesterday",
    },
  ];

  let selectedFindingIds = $state<Set<string>>(new Set(["f1", "f2"]));
  let selectedSearchIds = $state<Set<string>>(new Set(["s1"]));

  const activeFinding = $derived(
    mockFindings.find((f) => selectedFindingIds.has(f.id)) ?? mockFindings[0]
  );

  const activeSearch = $derived(
    mockSearchResults.find((s) => selectedSearchIds.has(s.id)) ?? mockSearchResults[0]
  );

  const selectedBytes = $derived(
    mockFindings
      .filter((f) => selectedFindingIds.has(f.id))
      .reduce((acc, f) => acc + f.bytes, 0)
  );

  function removeChip(chip: string) {
    filterChips = filterChips.filter((c) => c !== chip);
  }
</script>

<div class="gallery-container">
  <!-- Top App Toolbar -->
  <Toolbar title="Woosh">
    {#snippet leading()}
      <button
        type="button"
        class="icon-btn"
        title="Toggle Sidebar (⌘⌃S)"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect>
          <line x1="9" y1="3" x2="9" y2="21"></line>
        </svg>
      </button>
    {/snippet}

    {#snippet center()}
      {#if activeTab === "reclaim"}
        <SegmentedControl
          options={[
            { value: "list", label: "List" },
            { value: "projects", label: "Projects" },
          ]}
          bind:value={viewMode}
        />
      {/if}
    {/snippet}

    {#snippet trailing()}
      <!-- Theme Switcher for Gallery -->
      <div class="gallery-theme-selector">
        <span class="gallery-label">Theme:</span>
        <SegmentedControl
          options={[
            { value: "light", label: "Light" },
            { value: "dark", label: "Dark" },
            { value: "system", label: "System" },
          ]}
          bind:value={theme}
        />
      </div>

      <SearchField
        bind:value={searchQuery}
        chips={filterChips}
        onremovechip={removeChip}
        placeholder="Filter results… (⌘F)"
        width={220}
      />

      <button
        type="button"
        class="icon-btn"
        class:active={inspectorVisible}
        onclick={() => (inspectorVisible = !inspectorVisible)}
        title="Toggle Inspector (⌘⌥I)"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect>
          <line x1="15" y1="3" x2="15" y2="21"></line>
        </svg>
      </button>
    {/snippet}
  </Toolbar>

  <!-- Progress Strip (when active) -->
  {#if scanning}
    <ProgressStrip
      scanning={true}
      phase="Scanning home directory"
      items={41280}
      bytes={32 * 1024 * 1024 * 1024}
      onstop={() => (scanning = false)}
    />
  {/if}

  <!-- Split View (Sidebar | Content | Inspector) -->
  <div class="split-view">
    <!-- Sidebar -->
    <Sidebar width={210}>
      <div class="sidebar-section">
        <span class="sidebar-header">WORKSPACE</span>
        <SidebarItem
          label="Search"
          active={activeTab === "search"}
          onclick={() => (activeTab = "search")}
        >
          {#snippet icon()}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="11" cy="11" r="8"></circle>
              <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
            </svg>
          {/snippet}
        </SidebarItem>

        <SidebarItem
          label="Reclaim Space"
          active={activeTab === "reclaim"}
          badge={bytes(selectedBytes)}
          badgeColor="safe"
          onclick={() => (activeTab = "reclaim")}
        >
          {#snippet icon()}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M3 6h18m-2 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
            </svg>
          {/snippet}
        </SidebarItem>

        <SidebarItem
          label="History"
          active={activeTab === "history"}
          onclick={() => (activeTab = "history")}
        >
          {#snippet icon()}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="10"></circle>
              <polyline points="12 6 12 12 16 14"></polyline>
            </svg>
          {/snippet}
        </SidebarItem>
      </div>

      <div class="sidebar-section">
        <span class="sidebar-header">DOMAINS</span>
        <SidebarItem label="node_modules" badge="842 MB" badgeColor="default" />
        <SidebarItem label="Python venvs" badge="2.1 GB" badgeColor="default" />
        <SidebarItem label="Docker VM & Images" badge="24.5 GB" badgeColor="caution" />
        <SidebarItem label="Simulators & Caches" badge="3.8 GB" badgeColor="advisory" />
      </div>

      {#snippet footer()}
        <div class="sidebar-disk-footer">
          <div class="disk-bar">
            <div class="disk-used" style="width: 68%;"></div>
          </div>
          <div class="disk-meta tabular">
            <span>342 GB free</span>
            <span class="dim">of 1 TB</span>
          </div>
        </div>
      {/snippet}
    </Sidebar>

    <!-- Main Content Area -->
    <main class="content-area">
      {#if activeTab === "reclaim"}
        <!-- Reclaim View Content -->
        <div class="view-header-strip">
          <div class="header-summary">
            <span class="title">Dev Storage & Cleaners</span>
            <span class="subtitle tabular">{mockFindings.length} items found · {bytes(mockFindings.reduce((a, b) => a + b.bytes, 0))} reclaimable</span>
          </div>
          <div class="header-actions">
            <button
              type="button"
              class="btn-secondary"
              onclick={() => (scanning = !scanning)}
            >
              {scanning ? "Scanning…" : "Scan"}
            </button>
            <button
              type="button"
              class="btn-primary"
              disabled={selectedFindingIds.size === 0}
              onclick={() => (sheetOpen = true)}
            >
              Reclaim {bytes(selectedBytes)}
            </button>
          </div>
        </div>

        <div class="table-wrapper">
          <Table
            items={mockFindings}
            rowHeight={44}
            checkboxes={true}
            bind:selectedIds={selectedFindingIds}
            keyField={(f) => f.id}
            columns={[
              { key: "name", title: "Target & Path", width: "44%" },
              { key: "risk", title: "Risk", width: "16%" },
              { key: "idle", title: "Last Touched", width: "18%" },
              { key: "bytes", title: "Size", width: "15%", align: "right" },
            ]}
          >
            {#snippet cellSnippet(item, col)}
              {#if col.key === "name"}
                <div class="reclaim-name-cell">
                  <span class="name-text">{item.name}</span>
                  <span class="path-subtext mono dim">{item.path}</span>
                </div>
              {:else if col.key === "risk"}
                <div class="risk-indicator risk-{item.risk}">
                  <span class="risk-dot"></span>
                  <span class="risk-text">{item.risk.charAt(0).toUpperCase() + item.risk.slice(1)}</span>
                </div>
              {:else if col.key === "idle"}
                <span class="dim tabular">{item.idleDays} days ago</span>
              {:else if col.key === "bytes"}
                <span class="tabular font-medium">{bytes(item.bytes)}</span>
              {/if}
            {/snippet}
          </Table>
        </div>
      {:else if activeTab === "search"}
        <!-- Search View Content -->
        <div class="view-header-strip">
          <div class="header-summary">
            <span class="title">Instant File Search</span>
            <span class="subtitle tabular">412,880 items indexed · query latency 3.18 ms</span>
          </div>
          <div class="header-actions">
            <button type="button" class="btn-secondary">
              Rebuild Index
            </button>
          </div>
        </div>

        <div class="table-wrapper">
          <Table
            items={mockSearchResults}
            rowHeight={28}
            checkboxes={false}
            bind:selectedIds={selectedSearchIds}
            keyField={(s) => s.id}
            columns={[
              { key: "name", title: "Name", width: "35%" },
              { key: "path", title: "Path", width: "40%" },
              { key: "modified", title: "Modified", width: "12%" },
              { key: "bytes", title: "Size", width: "13%", align: "right" },
            ]}
          >
            {#snippet cellSnippet(item, col)}
              {#if col.key === "name"}
                <span class="search-name">{item.name}</span>
              {:else if col.key === "path"}
                <span class="search-path mono dim">{item.path}</span>
              {:else if col.key === "modified"}
                <span class="dim tabular">{item.modified}</span>
              {:else if col.key === "bytes"}
                <span class="tabular">{bytes(item.bytes)}</span>
              {/if}
            {/snippet}
          </Table>
        </div>
      {:else}
        <EmptyState
          title="No Past Reaps"
          description="Deleted items appear here with full audit logs and Put Back verification."
          actionLabel="Scan Storage"
          onaction={() => (activeTab = "reclaim")}
        />
      {/if}
    </main>

    <!-- Inspector -->
    <Inspector
      visible={inspectorVisible}
      title="Inspector"
      onclose={() => (inspectorVisible = false)}
    >
      {#if activeTab === "reclaim" && activeFinding}
        <div class="inspector-item">
          <h3 class="inspector-item-name">{activeFinding.name}</h3>
          <span class="inspector-item-path mono dim">{activeFinding.path}</span>
        </div>

        <div class="inspector-section">
          <span class="section-label">DETAILS</span>
          <div class="meta-row">
            <span class="meta-key">Reclaimable Size</span>
            <span class="meta-val tabular">{bytes(activeFinding.bytes)}</span>
          </div>
          <div class="meta-row">
            <span class="meta-key">Safety Rating</span>
            <div class="risk-indicator risk-{activeFinding.risk}">
              <span class="risk-dot"></span>
              <span>{activeFinding.risk.toUpperCase()}</span>
            </div>
          </div>
          <div class="meta-row">
            <span class="meta-key">Last Touched</span>
            <span class="meta-val tabular">{activeFinding.idleDays} days ago</span>
          </div>
          <div class="meta-row">
            <span class="meta-key">Domain</span>
            <span class="meta-val">{activeFinding.domain}</span>
          </div>
        </div>

        <div class="inspector-section">
          <span class="section-label">EXPLANATION</span>
          <p class="explanation-text">
            {#if activeFinding.risk === "safe"}
              Project build cache or dependency folder. It can be regenerated at any time by running standard package tools.
            {:else if activeFinding.risk === "caution"}
              Docker VM disk image containing local containers and volumes. Trashing it will reset Docker engine state.
            {:else}
              Simulator runtimes and caches. Freeing this space will require downloading the simulator profile again if needed.
            {/if}
          </p>
        </div>
      {:else if activeTab === "search" && activeSearch}
        <div class="inspector-item">
          <h3 class="inspector-item-name">{activeSearch.name}</h3>
          <span class="inspector-item-path mono dim">{activeSearch.path}</span>
        </div>

        <div class="inspector-section">
          <span class="section-label">METADATA</span>
          <div class="meta-row">
            <span class="meta-key">Kind</span>
            <span class="meta-val">{activeSearch.kind.toUpperCase()}</span>
          </div>
          <div class="meta-row">
            <span class="meta-key">Size</span>
            <span class="meta-val tabular">{bytes(activeSearch.bytes)}</span>
          </div>
          <div class="meta-row">
            <span class="meta-key">Modified</span>
            <span class="meta-val tabular">{activeSearch.modified}</span>
          </div>
        </div>
      {/if}

      {#snippet footer()}
        <div class="inspector-actions">
          <button type="button" class="btn-secondary w-full">Reveal in Finder</button>
          <button type="button" class="btn-secondary w-full">Quick Look (Space)</button>
          <button type="button" class="btn-secondary w-full">Copy Path (⌘C)</button>
        </div>
      {/snippet}
    </Inspector>
  </div>

  <!-- Destructive Confirmation Sheet -->
  <Sheet
    bind:open={sheetOpen}
    title="Move {selectedFindingIds.size} items to Trash?"
    description="Total size: {bytes(selectedBytes)}. Trashed items can be restored with Put Back from History."
    confirmLabel="Move to Trash"
    confirmDestructive={false}
    items={mockFindings.filter((f) => selectedFindingIds.has(f.id))}
    onconfirm={() => {
      sheetOpen = false;
    }}
  />
</div>

<style>
  .gallery-container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    background: var(--bg);
    color: var(--fg);
    overflow: hidden;
  }

  .split-view {
    display: flex;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .content-area {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    background: var(--bg);
    overflow: hidden;
  }

  .sidebar-section {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 12px;
  }

  .sidebar-header {
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-tertiary);
    padding: 4px 8px;
    letter-spacing: 0.04em;
  }

  .sidebar-disk-footer {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .disk-bar {
    height: 4px;
    background: var(--bg-hover);
    border-radius: 2px;
    overflow: hidden;
  }

  .disk-used {
    height: 100%;
    background: var(--accent);
  }

  .disk-meta {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
  }

  .view-header-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--separator);
    background: var(--bg-surface);
    flex: none;
  }

  .header-summary {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .header-summary .title {
    font-size: 14px;
    font-weight: 600;
  }

  .header-summary .subtitle {
    font-size: 12px;
    color: var(--fg-secondary);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .table-wrapper {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .reclaim-name-cell {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .name-text {
    font-weight: 500;
  }

  .path-subtext {
    font-size: 11px;
    color: var(--fg-secondary);
  }

  .search-name {
    font-weight: 500;
  }

  .search-path {
    font-size: 11.5px;
  }

  .dim {
    color: var(--fg-secondary);
  }

  .font-medium {
    font-weight: 500;
  }

  .inspector-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .inspector-item-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--fg);
  }

  .inspector-item-path {
    font-size: 11px;
    word-break: break-all;
  }

  .inspector-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-tertiary);
    letter-spacing: 0.04em;
  }

  .meta-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 12px;
  }

  .meta-key {
    color: var(--fg-secondary);
  }

  .meta-val {
    font-weight: 500;
  }

  .explanation-text {
    font-size: 12px;
    color: var(--fg-secondary);
    line-height: 1.4;
  }

  .inspector-actions {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .w-full {
    width: 100%;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--fg-secondary);
    background: transparent;
    border: none;
    cursor: default;
  }

  .icon-btn:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }

  .icon-btn.active {
    background: var(--bg-hover);
    color: var(--accent);
  }

  .gallery-theme-selector {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .gallery-label {
    font-size: 11px;
    color: var(--fg-secondary);
  }

  .btn-primary {
    padding: 5px 12px;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 13px;
    font-weight: 500;
    cursor: default;
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-secondary {
    padding: 5px 12px;
    border-radius: var(--radius-sm);
    background: var(--bg-hover);
    color: var(--fg);
    border: 1px solid var(--border);
    font-size: 13px;
    font-weight: 500;
    cursor: default;
  }
  .btn-secondary:hover {
    background: var(--bg-active);
  }
</style>
