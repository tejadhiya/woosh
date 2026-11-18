<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    label,
    active = false,
    badge,
    badgeColor = "default",
    icon,
    children,
    onclick,
  }: {
    label: string;
    active?: boolean;
    badge?: string | number;
    badgeColor?: "default" | "safe" | "caution" | "advisory" | "accent";
    icon?: Snippet;
    children?: Snippet;
    onclick?: () => void;
  } = $props();
</script>

<button
  type="button"
  class="sidebar-item"
  class:active
  {onclick}
  role="treeitem"
  aria-selected={active}
>
  {#if icon}
    <span class="item-icon">
      {@render icon()}
    </span>
  {/if}
  <span class="item-label">{label}</span>
  {#if children}
    {@render children()}
  {/if}
  {#if badge !== undefined && badge !== null && badge !== ""}
    <span class="item-badge tabular badge-{badgeColor}">{badge}</span>
  {/if}
</button>

<style>
  .sidebar-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 5px 8px;
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 400;
    color: var(--fg);
    background: transparent;
    border: none;
    text-align: left;
    cursor: default;
    transition: background 80ms ease, color 80ms ease;
  }

  .sidebar-item:hover:not(.active) {
    background: var(--bg-hover);
  }

  .sidebar-item.active {
    background: var(--accent);
    color: var(--accent-fg);
    font-weight: 500;
  }

  .item-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex: none;
    opacity: 0.85;
  }

  .sidebar-item.active .item-icon {
    opacity: 1;
  }

  .item-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-badge {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    background: var(--bg-hover);
    color: var(--fg-secondary);
    font-weight: 500;
  }

  .sidebar-item.active .item-badge {
    background: rgba(255, 255, 255, 0.25);
    color: #ffffff;
  }

  .item-badge.badge-safe {
    color: var(--safe);
  }
  .item-badge.badge-caution {
    color: var(--caution);
  }
  .item-badge.badge-advisory {
    color: var(--advisory);
  }
  .item-badge.badge-accent {
    color: var(--accent);
  }
  .sidebar-item.active .item-badge.badge-safe,
  .sidebar-item.active .item-badge.badge-caution,
  .sidebar-item.active .item-badge.badge-advisory,
  .sidebar-item.active .item-badge.badge-accent {
    color: #ffffff;
  }
</style>

