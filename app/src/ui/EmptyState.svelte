<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    description,
    actionLabel,
    onaction,
    children,
  }: {
    title: string;
    description?: string;
    actionLabel?: string;
    onaction?: () => void;
    children?: Snippet;
  } = $props();
</script>

<div class="empty-state">
  <div class="empty-icon-circle">
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="11" cy="11" r="8"></circle>
      <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
    </svg>
  </div>
  <h3 class="empty-title">{title}</h3>
  {#if description}
    <p class="empty-desc">{description}</p>
  {/if}
  {#if actionLabel}
    <button type="button" class="empty-action-btn" onclick={onaction}>
      {actionLabel}
    </button>
  {/if}
  {#if children}
    {@render children()}
  {/if}
</div>

<style>
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 48px 24px;
    gap: 8px;
    height: 100%;
    color: var(--fg);
    user-select: none;
    -webkit-user-select: none;
  }

  .empty-icon-circle {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: var(--bg-hover);
    color: var(--fg-tertiary);
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 4px;
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg);
  }

  .empty-desc {
    font-size: 13px;
    color: var(--fg-secondary);
    max-width: 360px;
    line-height: 1.4;
  }

  .empty-action-btn {
    margin-top: 8px;
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 13px;
    font-weight: 500;
    cursor: default;
    transition: background 100ms ease;
  }
  .empty-action-btn:hover {
    background: var(--accent-hover);
  }
</style>

