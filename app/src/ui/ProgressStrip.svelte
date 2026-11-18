<script lang="ts">
  import { bytes as formatBytes } from "../lib/format";

  let {
    scanning = false,
    phase = "Scanning",
    items = 0,
    bytes = 0,
    onstop,
  }: {
    scanning?: boolean;
    phase?: string;
    items?: number;
    bytes?: number;
    onstop?: () => void;
  } = $props();
</script>

{#if scanning}
  <div class="progress-strip" role="status" aria-live="polite">
    <div class="progress-left">
      <div class="progress-pulse"></div>
      <span class="progress-phase">{phase}…</span>
      {#if items > 0}
        <span class="progress-items tabular">{items.toLocaleString()} items</span>
      {/if}
      {#if bytes > 0}
        <span class="progress-bytes tabular">({formatBytes(bytes)} found)</span>
      {/if}
    </div>

    {#if onstop}
      <button
        type="button"
        class="progress-stop-btn"
        onclick={onstop}
        title="Stop scan"
      >
        Stop
      </button>
    {/if}
  </div>
{/if}

<style>
  .progress-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 28px;
    padding: 0 12px;
    background: var(--bg-hover);
    border-bottom: 1px solid var(--separator);
    font-size: 12px;
    color: var(--fg);
    flex: none;
  }

  .progress-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .progress-pulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.5s infinite ease-in-out;
  }

  @keyframes pulse {
    0%, 100% {
      opacity: 0.4;
      transform: scale(0.9);
    }
    50% {
      opacity: 1;
      transform: scale(1.15);
    }
  }

  .progress-phase {
    font-weight: 500;
  }

  .progress-items,
  .progress-bytes {
    color: var(--fg-secondary);
  }

  .progress-stop-btn {
    font-size: 11px;
    font-weight: 500;
    color: var(--destructive);
    background: transparent;
    border: none;
    cursor: default;
    padding: 2px 6px;
    border-radius: 4px;
  }
  .progress-stop-btn:hover {
    background: var(--destructive-tint);
  }
</style>

