<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    text,
    position = "top",
    children,
  }: {
    text: string;
    position?: "top" | "bottom" | "left" | "right";
    children?: Snippet;
  } = $props();

  let visible = $state(false);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="tooltip-wrapper"
  onmouseenter={() => (visible = true)}
  onmouseleave={() => (visible = false)}
  onfocusin={() => (visible = true)}
  onfocusout={() => (visible = false)}
>
  {#if children}
    {@render children()}
  {/if}

  {#if visible && text}
    <div class="tooltip-bubble tooltip-{position}" role="tooltip">
      {text}
    </div>
  {/if}
</div>

<style>
  .tooltip-wrapper {
    position: relative;
    display: inline-flex;
    align-items: center;
  }

  .tooltip-bubble {
    position: absolute;
    background: var(--fg);
    color: var(--bg);
    padding: 3px 7px;
    border-radius: var(--radius-xs);
    font-size: 11px;
    font-weight: 400;
    white-space: nowrap;
    z-index: 1000;
    pointer-events: none;
    box-shadow: var(--shadow-sm);
  }

  .tooltip-top {
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(-4px);
  }

  .tooltip-bottom {
    top: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(4px);
  }

  .tooltip-left {
    right: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(-4px);
  }

  .tooltip-right {
    left: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(4px);
  }
</style>

