<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    leading,
    center,
    trailing,
    children,
  }: {
    title?: string;
    leading?: Snippet;
    center?: Snippet;
    trailing?: Snippet;
    children?: Snippet;
  } = $props();
</script>

<header class="toolbar" data-tauri-drag-region>
  <div class="toolbar-traffic-lights-spacer"></div>

  <div class="toolbar-leading">
    {#if leading}
      {@render leading()}
    {/if}
    {#if title}
      <h1 class="toolbar-title">{title}</h1>
    {/if}
  </div>

  <div class="toolbar-center">
    {#if center}
      {@render center()}
    {/if}
    {#if children}
      {@render children()}
    {/if}
  </div>

  <div class="toolbar-trailing">
    {#if trailing}
      {@render trailing()}
    {/if}
  </div>
</header>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    height: var(--titlebar-height);
    background: var(--bg);
    border-bottom: 1px solid var(--separator);
    padding: 0 12px 0 0;
    gap: 12px;
    flex: none;
    user-select: none;
    -webkit-user-select: none;
    z-index: 20;
  }

  .toolbar-traffic-lights-spacer {
    width: 76px;
    height: 100%;
    flex: none;
  }

  .toolbar-leading {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }

  .toolbar-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
    letter-spacing: -0.01em;
  }

  .toolbar-center {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
  }

  .toolbar-trailing {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
</style>

