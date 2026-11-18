<script lang="ts" generics="T">
  import type { Snippet } from "svelte";
  import type { Column } from "./types";

  let {
    items,
    columns,
    selectedIds = $bindable(new Set<string>()),
    keyField,
    rowHeight = 28,
    checkboxes = false,
    emptyMessage = "No items",
    onrowclick,
    onrowdblclick,
    onselectchange,
    cellSnippet,
  }: {
    items: T[];
    columns: Column<T>[];
    selectedIds?: Set<string>;
    keyField: (item: T) => string;
    rowHeight?: number;
    checkboxes?: boolean;
    emptyMessage?: string;
    onrowclick?: (item: T, e: MouseEvent) => void;
    onrowdblclick?: (item: T) => void;
    onselectchange?: (selected: Set<string>) => void;
    cellSnippet?: Snippet<[T, Column<T>]>;
  } = $props();

  let activeIndex = $state<number>(0);

  // Sorting
  let sortKey = $state<string | null>(null);
  let sortDir = $state<"asc" | "desc">("asc");

  function toggleSort(col: Column<T>) {
    if (!col.sortable) return;
    if (sortKey === col.key) {
      sortDir = sortDir === "asc" ? "desc" : "asc";
    } else {
      sortKey = col.key;
      sortDir = "asc";
    }
  }

  function handleRowClick(item: T, idx: number, e: MouseEvent) {
    activeIndex = idx;
    const id = keyField(item);

    if (e.metaKey || e.ctrlKey) {
      const next = new Set(selectedIds);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      selectedIds = next;
    } else if (e.shiftKey && items.length > 0) {
      const start = Math.min(activeIndex, idx);
      const end = Math.max(activeIndex, idx);
      const next = new Set(selectedIds);
      for (let i = start; i <= end; i++) {
        next.add(keyField(items[i]));
      }
      selectedIds = next;
    } else {
      selectedIds = new Set([id]);
    }

    onselectchange?.(selectedIds);
    onrowclick?.(item, e);
  }

  function handleCheckboxToggle(item: T, e: MouseEvent) {
    e.stopPropagation();
    const id = keyField(item);
    const next = new Set(selectedIds);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    selectedIds = next;
    onselectchange?.(selectedIds);
  }

  function handleSelectAll() {
    if (selectedIds.size === items.length) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(items.map((it) => keyField(it)));
    }
    onselectchange?.(selectedIds);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (items.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      activeIndex = Math.min(items.length - 1, activeIndex + 1);
      const item = items[activeIndex];
      if (!e.metaKey && !e.shiftKey) {
        selectedIds = new Set([keyField(item)]);
        onselectchange?.(selectedIds);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      activeIndex = Math.max(0, activeIndex - 1);
      const item = items[activeIndex];
      if (!e.metaKey && !e.shiftKey) {
        selectedIds = new Set([keyField(item)]);
        onselectchange?.(selectedIds);
      }
    } else if (e.key === "Enter") {
      const item = items[activeIndex];
      if (item) onrowdblclick?.(item);
    }
  }
</script>

<div
  class="table-container"
  tabindex="0"
  role="grid"
  onkeydown={handleKeydown}
>
  <!-- Table Header -->
  <div class="table-header" role="row">
    {#if checkboxes}
      <div class="table-cell checkbox-cell header-cell">
        <input
          type="checkbox"
          checked={items.length > 0 && selectedIds.size === items.length}
          indeterminate={selectedIds.size > 0 && selectedIds.size < items.length}
          onclick={handleSelectAll}
          aria-label="Select all"
        />
      </div>
    {/if}
    {#each columns as col (col.key)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_interactive_supports_focus -->
      <div
        class="table-cell header-cell"
        class:sortable={col.sortable}
        class:align-right={col.align === "right"}
        class:align-center={col.align === "center"}
        style={col.width ? `width: ${col.width}; flex: none;` : "flex: 1;"}
        onclick={() => toggleSort(col)}
        role="columnheader"
        tabindex={col.sortable ? 0 : -1}
      >
        <span>{col.title}</span>
        {#if col.sortable && sortKey === col.key}
          <span class="sort-indicator">{sortDir === "asc" ? "▲" : "▼"}</span>
        {/if}
      </div>
    {/each}
  </div>

  <!-- Table Body -->
  <div class="table-body" role="rowgroup">
    {#each items as item, idx (keyField(item))}
      {@const id = keyField(item)}
      {@const isSelected = selectedIds.has(id)}
      {@const isActive = activeIndex === idx}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_interactive_supports_focus -->
      <div
        class="table-row"
        class:selected={isSelected}
        class:active={isActive}
        style="min-height: {rowHeight}px;"
        role="row"
        onclick={(e) => handleRowClick(item, idx, e)}
        ondblclick={() => onrowdblclick?.(item)}
      >
        {#if checkboxes}
          <div class="table-cell checkbox-cell">
            <input
              type="checkbox"
              checked={isSelected}
              onclick={(e) => handleCheckboxToggle(item, e)}
            />
          </div>
        {/if}
        {#each columns as col (col.key)}
          <div
            class="table-cell"
            class:align-right={col.align === "right"}
            class:align-center={col.align === "center"}
            style={col.width ? `width: ${col.width}; flex: none;` : "flex: 1;"}
            role="gridcell"
          >
            {#if cellSnippet}
              {@render cellSnippet(item, col)}
            {:else if col.render}
              <span class="cell-text">{col.render(item)}</span>
            {:else}
              <span class="cell-text">{(item as any)[col.key] ?? ""}</span>
            {/if}
          </div>
        {/each}
      </div>
    {/each}

    {#if items.length === 0}
      <div class="empty-table-row">
        <span>{emptyMessage}</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .table-container {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 100%;
    overflow: hidden;
    background: var(--bg);
    outline: none;
    user-select: none;
    -webkit-user-select: none;
  }

  .table-header {
    display: flex;
    align-items: center;
    border-bottom: 1px solid var(--separator);
    background: var(--bg-surface-elevated);
    flex: none;
    font-size: 11px;
    font-weight: 500;
    color: var(--fg-secondary);
    min-height: 24px;
  }

  .table-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .table-row {
    display: flex;
    align-items: center;
    border-bottom: 1px solid var(--separator);
    font-size: 13px;
    color: var(--fg);
    cursor: default;
    transition: background 60ms ease;
  }

  .table-row:hover:not(.selected) {
    background: var(--bg-hover);
  }

  .table-row.selected {
    background: var(--bg-selected);
    color: var(--bg-selected-fg);
  }

  .table-row.selected :global(.dim),
  .table-row.selected :global(.text-secondary) {
    color: rgba(255, 255, 255, 0.75) !important;
  }

  .table-cell {
    display: flex;
    align-items: center;
    padding: 0 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .table-cell:last-child {
    padding-right: 12px;
  }

  .header-cell {
    padding: 3px 8px;
  }

  .header-cell.sortable {
    cursor: default;
  }
  .header-cell.sortable:hover {
    color: var(--fg);
  }

  .sort-indicator {
    font-size: 9px;
    margin-left: 4px;
    opacity: 0.7;
  }

  .checkbox-cell {
    width: 32px;
    flex: none;
    justify-content: center;
    padding: 0;
  }

  .align-right {
    justify-content: flex-end;
    text-align: right;
  }

  .align-center {
    justify-content: center;
    text-align: center;
  }

  .cell-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty-table-row {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px 16px;
    color: var(--fg-tertiary);
    font-size: 13px;
  }
</style>
