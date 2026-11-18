<script lang="ts">
  import type { Finding, ReapMode } from "../lib/api";
  import { bytes } from "../lib/format";

  let { count, total, mode, hasCaution, items, onCancel, onConfirm }: {
    count: number;
    total: number;
    mode: ReapMode;
    hasCaution: boolean;
    items: Finding[];
    onCancel: () => void;
    onConfirm: () => void;
  } = $props();

  const permanent = $derived(mode === "permanent");
  const dry = $derived(mode === "dry_run");

  // Permanent deletion is the one action with no undo, so it is typed out.
  let typed = $state("");
  const armed = $derived(!permanent || typed.trim().toLowerCase() === "delete");

  const venvs = $derived(items.filter((f) => f.domain === "venv"));
  const restorable = $derived(items.filter((f) => f.restore).length);

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onCancel();
    if (e.key === "Enter" && armed) onConfirm();
  }
</script>

<svelte:window onkeydown={onKey} />

<div
  class="scrim"
  role="button"
  tabindex="-1"
  onclick={onCancel}
  onkeydown={(e) => e.key === "Enter" && onCancel()}
></div>

<div class="sheet" role="dialog" aria-modal="true" aria-label="Confirm Deletion">
  <header class:danger={permanent}>
    <h2>
      {#if dry}Preview {count} item{count === 1 ? "" : "s"}
      {:else if permanent}Delete {count} item{count === 1 ? "" : "s"} Permanently
      {:else}Move {count} item{count === 1 ? "" : "s"} to Trash{/if}
    </h2>
    <span class="total mono">{bytes(total)}</span>
  </header>

  <div class="body">
    {#if dry}
      <p class="lead">
        Nothing will be modified or deleted. Woosh runs a dry-run check and produces
        receipts previewing exactly what would happen.
      </p>
    {:else if permanent}
      <p class="lead warn">
        This permanently deletes the selected items from disk without moving them to the Trash. There is no Put Back.
      </p>
    {:else}
      <p class="lead">
        Items are moved to your macOS Finder Trash. You can inspect or use "Put Back" to restore them anytime until your Trash is emptied.
      </p>
    {/if}

    <ul class="facts">
      {#if venvs.length > 0}
        <li>
          <span class="k">◆</span>
          <span>
            {venvs.length} virtual environment{venvs.length === 1 ? "" : "s"} —
            <code>req-inplace.txt</code> will be written into each project root first.
            If that write fails, deletion is aborted.
          </span>
        </li>
      {/if}
      {#if restorable > 0}
        <li>
          <span class="k">↺</span>
          <span>{restorable} of {count} selected item{count === 1 ? "" : "s"} include an instant restore command.</span>
        </li>
      {/if}
      {#if hasCaution}
        <li>
          <span class="k amber">!</span>
          <span>
            Some items are marked Caution: recoverable, but rebuilding or redownloading them may take developer time.
          </span>
        </li>
      {/if}
    </ul>

    {#if permanent}
      <label class="confirm">
        <span>Type <strong>delete</strong> to confirm permanent deletion:</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          type="text"
          bind:value={typed}
          autofocus
          autocomplete="off"
          spellcheck="false"
          placeholder="delete"
        />
      </label>
    {/if}
  </div>

  <footer>
    <button class="btn" onclick={onCancel}>Cancel</button>
    <button
      class="btn {permanent ? 'btn-danger' : 'btn-primary'}"
      disabled={!armed}
      onclick={onConfirm}
    >
      {#if dry}Run Preview{:else if permanent}Delete Permanently{:else}Woosh to Trash{/if}
    </button>
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(4, 6, 8, 0.75);
    backdrop-filter: blur(6px);
    border: 0;
    padding: 0;
    cursor: default;
    z-index: 100;
  }
  .sheet {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(540px, calc(100vw - 48px));
    background: var(--bg-surface-elevated);
    border: 1px solid var(--line-strong);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-popup);
    overflow: hidden;
    z-index: 101;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 14px;
    padding: 16px 22px;
    border-bottom: 1px solid var(--line);
    background: var(--bg-surface);
  }
  header.danger {
    background: rgba(248, 113, 113, 0.08);
    border-bottom-color: rgba(248, 113, 113, 0.25);
  }
  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    flex: 1;
    color: var(--ink);
  }
  .total {
    font-size: 17px;
    font-weight: 700;
    color: var(--mint-bright);
  }
  header.danger .total {
    color: var(--red);
  }

  .body {
    padding: 20px 22px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .lead {
    margin: 0;
    font-size: 13.5px;
    line-height: 1.55;
    color: var(--ink-dim);
  }
  .lead.warn {
    color: #fca5a5;
  }

  .facts {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .facts li {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    gap: 10px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--ink-dim);
  }
  .k {
    color: var(--mint-bright);
    text-align: center;
    font-size: 14px;
  }
  .k.amber {
    color: var(--amber);
    font-weight: 700;
  }
  code {
    font-family: var(--mono);
    font-size: 12px;
    background: var(--bg-canvas);
    border: 1px solid var(--line);
    border-radius: var(--radius-xs);
    padding: 1px 6px;
    color: var(--ink);
  }

  .confirm {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 4px;
  }
  .confirm span {
    font-size: 13px;
    color: var(--ink-dim);
  }
  .confirm strong {
    color: var(--red);
  }
  .confirm input {
    background: var(--bg-canvas);
    border: 1.5px solid var(--line-strong);
    border-radius: var(--radius-md);
    padding: 10px 14px;
    color: var(--ink);
    font-family: var(--mono);
    font-size: 13.5px;
  }
  .confirm input:focus {
    outline: none;
    border-color: var(--red);
    box-shadow: 0 0 8px rgba(248, 113, 113, 0.3);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding: 16px 22px;
    border-top: 1px solid var(--line);
    background: var(--bg-surface);
  }
</style>
