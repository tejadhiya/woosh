<script lang="ts">
  let {
    value = $bindable(""),
    placeholder = "Search…",
    chips = [],
    onremovechip,
    oninput,
    onclear,
    width = 240,
  }: {
    value?: string;
    placeholder?: string;
    chips?: string[];
    onremovechip?: (chip: string) => void;
    oninput?: (val: string) => void;
    onclear?: () => void;
    width?: number | string;
  } = $props();

  let inputEl = $state<HTMLInputElement | null>(null);

  function handleInput(e: Event) {
    const target = e.target as HTMLInputElement;
    value = target.value;
    oninput?.(value);
  }

  function clear() {
    value = "";
    onclear?.();
    oninput?.("");
    inputEl?.focus();
  }
</script>

<div class="search-field" style:width={typeof width === "number" ? `${width}px` : width}>
  <svg
    class="search-glyph"
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2.2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <circle cx="11" cy="11" r="8"></circle>
    <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
  </svg>

  <div class="chips-container">
    {#each chips as chip (chip)}
      <span class="chip">
        <span class="chip-label">{chip}</span>
        <button
          type="button"
          class="chip-remove"
          title="Remove filter"
          onclick={() => onremovechip?.(chip)}
        >
          ×
        </button>
      </span>
    {/each}
    <input
      type="text"
      bind:this={inputEl}
      {value}
      oninput={handleInput}
      {placeholder}
      class="search-input"
      spellcheck="false"
      autocomplete="off"
    />
  </div>

  {#if value || chips.length > 0}
    <button
      type="button"
      class="clear-btn"
      title="Clear search"
      onclick={clear}
      aria-label="Clear search"
    >
      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
        <line x1="18" y1="6" x2="6" y2="18"></line>
        <line x1="6" y1="6" x2="18" y2="18"></line>
      </svg>
    </button>
  {/if}
</div>

<style>
  .search-field {
    display: inline-flex;
    align-items: center;
    background: var(--bg-hover);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 3px 6px;
    gap: 6px;
    font-size: 13px;
    transition: border-color 100ms ease, box-shadow 100ms ease;
  }

  .search-field:focus-within {
    border-color: var(--accent);
    background: var(--bg-surface);
    box-shadow: 0 0 0 2px var(--focus-ring);
  }

  .search-glyph {
    color: var(--fg-tertiary);
    flex: none;
  }

  .chips-container {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .chips-container::-webkit-scrollbar {
    display: none;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    background: var(--bg-surface-elevated);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 4px 1px 6px;
    font-size: 11px;
    color: var(--fg);
    white-space: nowrap;
    flex: none;
  }

  .chip-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    color: var(--fg-secondary);
    font-size: 12px;
    line-height: 1;
    background: transparent;
    border: none;
    cursor: default;
  }

  .chip-remove:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }

  .search-input {
    flex: 1;
    min-width: 60px;
    background: transparent;
    border: none;
    outline: none;
    font-family: inherit;
    font-size: 13px;
    color: var(--fg);
    padding: 0;
  }

  .search-input::placeholder {
    color: var(--fg-tertiary);
  }

  .clear-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--fg-tertiary);
    color: var(--bg);
    border: none;
    cursor: default;
    flex: none;
    opacity: 0.8;
  }

  .clear-btn:hover {
    opacity: 1;
  }
</style>
