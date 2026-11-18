<script lang="ts" generics="T extends string">
  let {
    options,
    value = $bindable(),
    onchange,
  }: {
    options: { value: T; label: string; count?: number }[];
    value: T;
    onchange?: (value: T) => void;
  } = $props();

  function select(v: T) {
    value = v;
    onchange?.(v);
  }
</script>

<div class="segmented-control" role="tablist">
  {#each options as opt (opt.value)}
    {@const active = opt.value === value}
    <button
      type="button"
      role="tab"
      aria-selected={active}
      class="segment-button"
      class:active
      onclick={() => select(opt.value)}
    >
      <span class="segment-label">{opt.label}</span>
      {#if opt.count !== undefined}
        <span class="segment-count tabular">{opt.count}</span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .segmented-control {
    display: inline-flex;
    align-items: center;
    padding: 2px;
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    gap: 2px;
    border: 1px solid var(--border);
  }

  .segment-button {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 10px;
    border-radius: 4px;
    font-size: 12px;
    font-weight: 400;
    color: var(--fg-secondary);
    background: transparent;
    border: none;
    cursor: default;
    transition: all 100ms ease;
    line-height: 1.3;
  }

  .segment-button:hover:not(.active) {
    color: var(--fg);
  }

  .segment-button.active {
    background: var(--bg-surface);
    color: var(--fg);
    font-weight: 500;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }

  .segment-count {
    font-size: 11px;
    color: var(--fg-tertiary);
  }

  .segment-button.active .segment-count {
    color: var(--fg-secondary);
  }
</style>

