<script lang="ts">
  import { onMount } from "svelte";
  import { api, type Summary, type Receipt } from "../lib/api";
  import { bytes } from "../lib/format";

  let summaries = $state<Summary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let restoringId = $state<string | null>(null);
  let restoredIds = $state<Set<string>>(new Set());
  let restoreError = $state<string | null>(null);

  onMount(() => {
    void loadHistory();
  });

  async function loadHistory() {
    loading = true;
    error = null;
    try {
      summaries = await api.history(50);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function handlePutBack(receipt: Receipt) {
    restoringId = receipt.id;
    restoreError = null;
    try {
      await api.putBack(receipt);
      restoredIds = new Set([...restoredIds, receipt.id]);
    } catch (e) {
      restoreError = String(e);
    } finally {
      restoringId = null;
    }
  }
</script>

<div class="history-view">
  <header class="history-header">
    <div class="history-title-area">
      <h2 class="history-title">Reclamation History & Put Back</h2>
      <p class="history-subtitle">
        Audit log receipts recorded by the safety reaper. Items moved to Finder Trash can be restored to their original location.
      </p>
    </div>
    <button class="btn-refresh" onclick={loadHistory} disabled={loading}>
      <span>{loading ? "Refreshing…" : "Refresh"}</span>
    </button>
  </header>

  {#if restoreError}
    <div class="restore-error-banner">
      <span class="err-tag">!</span>
      <span>{restoreError}</span>
      <button class="banner-close" onclick={() => (restoreError = null)}>Close</button>
    </div>
  {/if}

  {#if error}
    <div class="restore-error-banner">
      <span class="err-tag">!</span>
      <span>Failed to load audit history: {error}</span>
      <button class="banner-close" onclick={() => (error = null)}>Close</button>
    </div>
  {/if}

  {#if loading && summaries.length === 0}
    <div class="history-loading">
      <span class="pulsing">Loading audit history…</span>
    </div>
  {:else if summaries.length === 0}
    <div class="history-empty">
      <div class="empty-icon">↺</div>
      <h3 class="empty-title">No Audit History Yet</h3>
      <p class="empty-desc">
        When you reclaim space using Woosh, every action and receipt is cryptographically logged and stored here.
      </p>
    </div>
  {:else}
    <div class="history-runs-list">
      {#each summaries as summary (summary.timestamp)}
        <section class="summary-card">
          <header class="summary-card-header">
            <div class="summary-meta-left">
              <span class="summary-mode-tag" class:trash={summary.mode === "trash"} class:perm={summary.mode === "permanent"}>
                {summary.mode.toUpperCase()}
              </span>
              <time class="summary-timestamp mono">{summary.timestamp}</time>
            </div>
            <div class="summary-stats-right">
              <span class="summary-reclaimed-badge mono bold">
                +{bytes(summary.reclaimed)} freed
              </span>
              <span class="summary-counts mono">
                {summary.succeeded} / {summary.total} items
              </span>
            </div>
          </header>

          <div class="receipts-list">
            {#each summary.receipts as receipt (receipt.id)}
              <div class="receipt-row" class:restored={restoredIds.has(receipt.id)}>
                <div class="receipt-main">
                  <span class="receipt-status-dot" class:success={receipt.success} class:failed={!receipt.success}></span>
                  <div class="receipt-info">
                    <div class="receipt-name-line">
                      <span class="receipt-name bold">{receipt.name}</span>
                      <span class="receipt-method mono">{receipt.method}</span>
                    </div>
                    {#if receipt.path}
                      <span class="receipt-path mono truncate">{receipt.path}</span>
                    {/if}
                    {#if receipt.error}
                      <span class="receipt-error mono">Refusal: {receipt.error}</span>
                    {/if}
                  </div>
                </div>

                <div class="receipt-actions">
                  <span class="receipt-bytes mono bold">{bytes(receipt.bytes)}</span>

                  {#if receipt.method === "trash" && receipt.path}
                    {#if restoredIds.has(receipt.id)}
                      <span class="restored-tag mono">Restored</span>
                    {:else}
                      <button
                        class="btn-putback"
                        disabled={restoringId === receipt.id}
                        onclick={() => handlePutBack(receipt)}
                        title="Restore this item from Finder Trash back to its original location"
                      >
                        {restoringId === receipt.id ? "Putting back…" : "Put Back"}
                      </button>
                    {/if}
                  {/if}

                  {#if receipt.path}
                    <button
                      class="btn-reveal"
                      onclick={() => receipt.path && api.reveal(receipt.path)}
                      title="Reveal destination in Finder"
                    >
                      Finder
                    </button>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .history-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
    padding: 24px 32px 64px;
    background: var(--bg-canvas);
    color: var(--text-normal);
  }

  .history-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 24px;
    padding-bottom: 18px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .history-title {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-bright);
    margin: 0 0 6px 0;
  }

  .history-subtitle {
    font-size: 13.5px;
    color: var(--text-muted);
    margin: 0;
    max-width: 650px;
    line-height: 1.45;
  }

  .btn-refresh {
    background: var(--bg-elevated);
    border: 1px solid var(--border-normal);
    color: var(--text-normal);
    border-radius: var(--radius-sm);
    padding: 6px 14px;
    font-size: 12.5px;
    cursor: pointer;
    transition: all var(--duration-fast) ease;
  }

  .btn-refresh:hover:not(:disabled) {
    background: var(--bg-hover);
    border-color: var(--border-strong);
    color: var(--text-bright);
  }

  .restore-error-banner {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: #f87171;
    padding: 10px 16px;
    border-radius: var(--radius-md);
    margin-bottom: 18px;
    font-size: 13px;
  }

  .banner-close {
    background: transparent;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 14px;
  }

  .history-loading, .history-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 64px 20px;
    text-align: center;
  }

  .empty-icon {
    font-size: 40px;
    color: var(--text-faint);
    margin-bottom: 14px;
  }

  .empty-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-bright);
    margin: 0 0 8px 0;
  }

  .empty-desc {
    font-size: 13px;
    color: var(--text-muted);
    max-width: 440px;
    margin: 0;
    line-height: 1.5;
  }

  .history-runs-list {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .summary-card {
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .summary-card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 18px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
  }

  .summary-meta-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .summary-mode-tag {
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 2px 7px;
    border-radius: var(--radius-xs);
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-muted);
  }

  .summary-mode-tag.trash {
    background: rgba(16, 185, 129, 0.12);
    color: var(--accent-mint);
    border: 1px solid rgba(16, 185, 129, 0.25);
  }

  .summary-mode-tag.perm {
    background: rgba(239, 68, 68, 0.12);
    color: #f87171;
    border: 1px solid rgba(239, 68, 68, 0.25);
  }

  .summary-timestamp {
    font-size: 12px;
    color: var(--text-muted);
  }

  .summary-stats-right {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .summary-reclaimed-badge {
    font-size: 13.5px;
    color: var(--accent-mint);
  }

  .summary-counts {
    font-size: 12px;
    color: var(--text-muted);
  }

  .receipts-list {
    display: flex;
    flex-direction: column;
  }

  .receipt-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 18px;
    border-bottom: 1px solid var(--border-subtle);
    transition: background var(--duration-fast) ease;
  }

  .receipt-row:last-child {
    border-bottom: none;
  }

  .receipt-row:hover {
    background: var(--bg-hover);
  }

  .receipt-row.restored {
    opacity: 0.65;
  }

  .receipt-main {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    flex: 1;
  }

  .receipt-status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .receipt-status-dot.success {
    background: var(--accent-mint);
  }

  .receipt-status-dot.failed {
    background: #ef4444;
  }

  .receipt-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .receipt-name-line {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .receipt-name {
    font-size: 13.5px;
    color: var(--text-bright);
  }

  .receipt-method {
    font-size: 11px;
    color: var(--text-muted);
    background: rgba(255, 255, 255, 0.04);
    padding: 1px 6px;
    border-radius: var(--radius-xs);
  }

  .receipt-path {
    font-size: 11.5px;
    color: var(--text-faint);
  }

  .receipt-error {
    font-size: 11.5px;
    color: #f87171;
  }

  .receipt-actions {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-shrink: 0;
    margin-left: 16px;
  }

  .receipt-bytes {
    font-size: 13px;
    color: var(--text-normal);
    min-width: 75px;
    text-align: right;
  }

  .btn-putback {
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(16, 185, 129, 0.3);
    color: var(--accent-mint);
    border-radius: var(--radius-xs);
    padding: 4px 10px;
    font-size: 11.5px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) ease;
  }

  .btn-putback:hover:not(:disabled) {
    background: rgba(16, 185, 129, 0.22);
    border-color: var(--accent-mint);
  }

  .restored-tag {
    font-size: 11px;
    font-weight: 600;
    color: var(--accent-mint);
    padding: 4px 8px;
  }

  .btn-reveal {
    background: transparent;
    border: 1px solid var(--border-subtle);
    color: var(--text-muted);
    border-radius: var(--radius-xs);
    padding: 4px 9px;
    font-size: 11px;
    cursor: pointer;
    transition: all var(--duration-fast) ease;
  }

  .btn-reveal:hover {
    background: var(--bg-hover);
    color: var(--text-bright);
    border-color: var(--border-normal);
  }
</style>
