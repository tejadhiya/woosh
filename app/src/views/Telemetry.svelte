<script lang="ts">
  import type { Frame, LogLine } from "../lib/api";
  import { bytes, stamp, thousands } from "../lib/format";

  let { frame, log, scanning }: {
    frame: Frame | null;
    log: Iterable<LogLine> & { length: number };
    scanning: boolean;
  } = $props();

  let feed = $state<HTMLDivElement | null>(null);
  let pinned = $state(true);

  // Follow the tail unless the user has scrolled up to read something.
  $effect(() => {
    void log.length;
    if (pinned && feed) feed.scrollTop = feed.scrollHeight;
  });

  function onScroll() {
    if (!feed) return;
    pinned = feed.scrollHeight - feed.scrollTop - feed.clientHeight < 32;
  }
</script>

<div class="wrap">
  <div class="gauges">
    <div class="gauge">
      <span class="label">Elapsed</span>
      <span class="value mono">{frame ? stamp(frame.elapsed_us) : "0.0000s"}</span>
    </div>
    <div class="gauge">
      <span class="label">Entries Scanned</span>
      <span class="value mono">{thousands(frame?.entries ?? 0)}</span>
    </div>
    <div class="gauge">
      <span class="label">Scan Velocity</span>
      <span class="value mono accent">
        {thousands(frame?.rate ?? 0)}<small>/s</small>
      </span>
    </div>
    <div class="gauge">
      <span class="label">Storage Measured</span>
      <span class="value mono">{bytes(frame?.bytes ?? 0)}</span>
    </div>
    <div class="gauge">
      <span class="label">Findings Claimed</span>
      <span class="value mono">{frame?.findings ?? 0}</span>
    </div>
  </div>

  <div class="feed mono" bind:this={feed} onscroll={onScroll}>
    {#each log as line (line.t_us + line.text)}
      <div class="line {line.kind}">
        <span class="t">{stamp(line.t_us)}</span>
        <span class="g">{line.glyph}</span>
        <span class="txt">{line.text}</span>
      </div>
    {/each}
    {#if scanning}
      <div class="line cursor"><span class="t"></span><span class="g">▍</span></div>
    {/if}
    {#if log.length === 0 && !scanning}
      <div class="empty">
        Telemetry stream is idle. Press <strong>⌘R</strong> or click <strong>Sweep Now</strong> to watch live Apple Silicon filesystem traversal.
      </div>
    {/if}
  </div>

  {#if frame && frame.paths.length > 0 && scanning}
    <div class="probes mono">
      <div class="probe-title">Active Concurrent Walkers:</div>
      {#each frame.paths.slice(0, 3) as p (p)}
        <div class="probe">{p}</div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }

  .gauges {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 1px;
    background: var(--line);
    border-bottom: 1px solid var(--line);
    flex: none;
  }
  .gauge {
    background: var(--bg-surface);
    padding: 12px 18px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--ink-faint);
  }
  .value {
    font-size: 19px;
    font-weight: 700;
    color: var(--ink);
    letter-spacing: -0.02em;
  }
  .value small {
    font-size: 12px;
    color: var(--ink-dim);
    font-weight: 500;
    margin-left: 2px;
  }
  .accent { color: var(--mint-bright); }

  .feed {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px 0 18px;
    font-size: 13px;
    line-height: 1.8;
    background:
      radial-gradient(900px 300px at 15% -10%, rgba(16, 185, 129, 0.04), transparent 70%),
      var(--bg-canvas);
  }

  .line {
    display: flex;
    gap: 12px;
    padding: 0 20px;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .t { color: #4b5563; flex: none; font-size: 12px; }
  .g { flex: none; width: 14px; text-align: center; font-size: 13px; }
  .txt { flex: 1; }

  .line.boot  .g, .line.boot  .txt { color: var(--blue); }
  .line.scan  .txt { color: var(--ink-faint); }
  .line.scan  .g   { color: #059669; }
  .line.found .g   { color: var(--mint-bright); }
  .line.found .txt { color: var(--ink); }
  .line.woosh .g, .line.woosh .txt { color: var(--violet); }
  .line.warn  .g, .line.warn  .txt { color: var(--amber); }
  .line.done  .g, .line.done  .txt { color: var(--mint-bright); font-weight: 600; }

  .cursor .g { color: var(--mint-bright); animation: blink 1s steps(2) infinite; }
  @keyframes blink { 50% { opacity: 0; } }

  .empty {
    padding: 36px 24px;
    color: var(--ink-faint);
    font-family: var(--sans);
    font-size: 14px;
    line-height: 1.6;
  }
  .empty strong {
    color: var(--ink);
  }

  .probes {
    flex: none;
    border-top: 1px solid var(--line);
    background: var(--bg-surface);
    padding: 8px 20px;
    font-size: 12px;
    color: var(--ink-faint);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .probe-title {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--ink-ash);
    margin-bottom: 2px;
  }
  .probe { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
