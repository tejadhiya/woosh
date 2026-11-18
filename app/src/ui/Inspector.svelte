<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    width = 260,
    visible = true,
    title = "Inspector",
    children,
    footer,
    onclose,
  }: {
    width?: number;
    visible?: boolean;
    title?: string;
    children?: Snippet;
    footer?: Snippet;
    onclose?: () => void;
  } = $props();

  let customWidth = $state<number | null>(null);
  let inspectorWidth = $derived(customWidth ?? width);
  let isDragging = $state(false);

  function startResize(e: MouseEvent) {
    e.preventDefault();
    isDragging = true;
    const startX = e.clientX;
    const startW = inspectorWidth;

    function onMouseMove(ev: MouseEvent) {
      customWidth = Math.max(200, Math.min(450, startW - (ev.clientX - startX)));
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

{#if visible}
  <aside
    class="inspector"
    style="width: {inspectorWidth}px;"
    class:resizing={isDragging}
    aria-label="Inspector"
  >
    <!-- Resizer handle on leading edge -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="inspector-resizer"
      role="separator"
      tabindex="-1"
      aria-orientation="vertical"
      onmousedown={startResize}
    ></div>

    <header class="inspector-header">
      <h2 class="inspector-title">{title}</h2>
      {#if onclose}
        <button
          type="button"
          class="close-btn"
          onclick={onclose}
          title="Close Inspector (⌘⌥I)"
          aria-label="Close Inspector"
        >
          ×
        </button>
      {/if}
    </header>

    <div class="inspector-content">
      {#if children}
        {@render children()}
      {/if}
    </div>

    {#if footer}
      <footer class="inspector-footer">
        {@render footer()}
      </footer>
    {/if}
  </aside>
{/if}

<style>
  .inspector {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    flex: none;
    background: var(--bg-surface);
    border-left: 1px solid var(--separator);
    user-select: none;
    -webkit-user-select: none;
    font-size: 13px;
  }

  .inspector-resizer {
    position: absolute;
    top: 0;
    left: -3px;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 10;
  }

  .inspector-resizer:hover,
  .inspector.resizing .inspector-resizer {
    background: var(--accent);
  }

  .inspector-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--separator);
    flex: none;
  }

  .inspector-title {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--fg-secondary);
  }

  .close-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    color: var(--fg-secondary);
    font-size: 14px;
    line-height: 1;
    background: transparent;
    border: none;
    cursor: default;
  }
  .close-btn:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }

  .inspector-content {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .inspector-footer {
    flex: none;
    padding: 10px 14px;
    border-top: 1px solid var(--separator);
  }
</style>
