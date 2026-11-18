<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    width = 220,
    collapsed = false,
    children,
    footer,
  }: {
    width?: number;
    collapsed?: boolean;
    children?: Snippet;
    footer?: Snippet;
  } = $props();

  let customWidth = $state<number | null>(null);
  let sidebarWidth = $derived(customWidth ?? width);
  let isDragging = $state(false);

  function startResize(e: MouseEvent) {
    e.preventDefault();
    isDragging = true;
    const startX = e.clientX;
    const startW = sidebarWidth;

    function onMouseMove(ev: MouseEvent) {
      customWidth = Math.max(160, Math.min(360, startW + (ev.clientX - startX)));
    }

    function onMouseUp() {
      isDragging = false;
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    }

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  }
</script>

{#if !collapsed}
  <aside
    class="sidebar"
    style="width: {sidebarWidth}px;"
    class:resizing={isDragging}
    aria-label="Source List"
  >
    <div class="sidebar-scroll">
      {#if children}
        {@render children()}
      {/if}
    </div>
    {#if footer}
      <div class="sidebar-footer">
        {@render footer()}
      </div>
    {/if}
    <!-- Resizer handle -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="sidebar-resizer"
      role="separator"
      tabindex="-1"
      aria-orientation="vertical"
      onmousedown={startResize}
    ></div>
  </aside>
{/if}

<style>
  .sidebar {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    flex: none;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--separator);
    user-select: none;
    -webkit-user-select: none;
  }

  .sidebar-scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 10px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .sidebar-footer {
    flex: none;
    padding: 8px 12px;
    border-top: 1px solid var(--separator);
    font-size: 11px;
    color: var(--fg-secondary);
  }

  .sidebar-resizer {
    position: absolute;
    top: 0;
    right: -3px;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 10;
  }

  .sidebar-resizer:hover,
  .sidebar.resizing .sidebar-resizer {
    background: var(--accent);
  }
</style>
