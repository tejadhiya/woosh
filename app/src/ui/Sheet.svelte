<script lang="ts">
  import type { Snippet } from "svelte";
  import { bytes as formatBytes } from "../lib/format";

  let {
    open = $bindable(false),
    title = "Confirm Action",
    description,
    items = [],
    confirmLabel = "Move to Trash",
    confirmDestructive = false,
    cancelLabel = "Cancel",
    onconfirm,
    oncancel,
    children,
  }: {
    open?: boolean;
    title?: string;
    description?: string;
    items?: { id: string; name: string; bytes?: number; path?: string }[];
    confirmLabel?: string;
    confirmDestructive?: boolean;
    cancelLabel?: string;
    onconfirm?: () => void;
    oncancel?: () => void;
    children?: Snippet;
  } = $props();

  function handleCancel() {
    open = false;
    oncancel?.();
  }

  function handleConfirm() {
    open = false;
    onconfirm?.();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.stopPropagation();
      handleCancel();
    } else if (e.key === "Enter") {
      e.stopPropagation();
      handleConfirm();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <div class="sheet-scrim" onclick={handleCancel} role="presentation">
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="sheet-window"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="sheet-title"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <header class="sheet-header">
        <h2 id="sheet-title" class="sheet-title">{title}</h2>
        {#if description}
          <p class="sheet-desc">{description}</p>
        {/if}
      </header>

      {#if children}
        <div class="sheet-body">
          {@render children()}
        </div>
      {:else if items.length > 0}
        <div class="sheet-items-list">
          {#each items as it (it.id)}
            <div class="sheet-item-row">
              <span class="sheet-item-name">{it.name}</span>
              {#if it.bytes !== undefined}
                <span class="sheet-item-size tabular">{formatBytes(it.bytes)}</span>
              {/if}
            </div>
          {/each}
        </div>
      {/if}

      <footer class="sheet-actions">
        <button
          type="button"
          class="btn btn-secondary"
          onclick={handleCancel}
        >
          {cancelLabel}
        </button>
        <button
          type="button"
          class="btn"
          class:btn-primary={!confirmDestructive}
          class:btn-destructive={confirmDestructive}
          onclick={handleConfirm}
        >
          {confirmLabel}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .sheet-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.40);
    backdrop-filter: blur(2px);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 40px;
    z-index: 100;
    user-select: none;
    -webkit-user-select: none;
  }

  .sheet-window {
    width: 480px;
    max-width: 90vw;
    background: var(--bg-surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sheet);
    border: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: slideDown 150ms cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes slideDown {
    from {
      opacity: 0;
      transform: translateY(-16px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .sheet-header {
    padding: 20px 24px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .sheet-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--fg);
  }

  .sheet-desc {
    font-size: 13px;
    color: var(--fg-secondary);
    line-height: 1.4;
  }

  .sheet-body {
    padding: 8px 24px 16px;
  }

  .sheet-items-list {
    max-height: 220px;
    overflow-y: auto;
    border-top: 1px solid var(--separator);
    border-bottom: 1px solid var(--separator);
    padding: 6px 24px;
    background: var(--bg-hover);
  }

  .sheet-item-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 5px 0;
    font-size: 12px;
  }

  .sheet-item-name {
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }

  .sheet-item-size {
    color: var(--fg-secondary);
  }

  .sheet-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding: 16px 24px;
    background: var(--bg-surface-elevated);
    border-top: 1px solid var(--separator);
  }

  .btn {
    padding: 5px 14px;
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 500;
    cursor: default;
    transition: all 100ms ease;
  }

  .btn-secondary {
    background: var(--bg-hover);
    color: var(--fg);
    border: 1px solid var(--border);
  }
  .btn-secondary:hover {
    background: var(--bg-active);
  }

  .btn-primary {
    background: var(--accent);
    color: var(--accent-fg);
    border: 1px solid transparent;
  }
  .btn-primary:hover {
    background: var(--accent-hover);
  }

  .btn-destructive {
    background: var(--destructive);
    color: #ffffff;
    border: 1px solid transparent;
  }
  .btn-destructive:hover {
    filter: brightness(0.9);
  }
</style>
