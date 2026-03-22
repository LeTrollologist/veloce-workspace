<script>
  import { onMount, onDestroy, tick } from 'svelte';
  import { listen }                   from '@tauri-apps/api/event';
  import { invoke }                   from '@tauri-apps/api/core';

  // ── State ───────────────────────────────────────────────────────────────────
  let status   = 'unknown';   // "running" | "stopped" | "starting" | "stopping" | "unknown"
  let logPath  = '';
  let lines    = [];          // array of strings
  let busy     = false;
  let autoScroll = true;

  let termEl;
  let unlisten;

  const MAX_LINES = 2000;

  // ── Helpers ─────────────────────────────────────────────────────────────────
  function stateColor(s) {
    if (s === 'running')  return '#3fb950';
    if (s === 'stopped')  return '#f85149';
    if (s === 'starting' || s === 'stopping') return '#d29922';
    return '#6e7681';
  }

  function stateLabel(s) {
    if (s === 'running')  return 'Running';
    if (s === 'stopped')  return 'Stopped';
    if (s === 'starting') return 'Starting…';
    if (s === 'stopping') return 'Stopping…';
    return 'Unknown';
  }

  async function scrollToBottom() {
    await tick();
    if (termEl && autoScroll) {
      termEl.scrollTop = termEl.scrollHeight;
    }
  }

  function appendLine(text) {
    lines = [...lines, text].slice(-MAX_LINES);
    scrollToBottom();
  }

  // ── Refresh service status ───────────────────────────────────────────────────
  async function refresh() {
    try {
      const s = await invoke('core_status');
      status  = s.state;
      logPath = s.log_path;
    } catch { status = 'unknown'; }
  }

  // ── Service control ──────────────────────────────────────────────────────────
  async function startService() {
    busy = true;
    status = 'starting';
    try   { status = await invoke('core_start'); }
    catch { await refresh(); }
    finally { busy = false; }
  }

  async function stopService() {
    busy = true;
    status = 'stopping';
    try   { status = await invoke('core_stop'); }
    catch { await refresh(); }
    finally { busy = false; }
  }

  async function restartService() {
    busy = true;
    status = 'stopping';
    try   { status = await invoke('core_restart'); }
    catch { await refresh(); }
    finally { busy = false; }
  }

  function clearLog() { lines = []; }

  // ── Terminal scroll-lock ─────────────────────────────────────────────────────
  function onTermScroll() {
    if (!termEl) return;
    const atBottom = termEl.scrollHeight - termEl.scrollTop - termEl.clientHeight < 32;
    autoScroll = atBottom;
  }

  // ── Log line classification ───────────────────────────────────────────────
  function lineClass(line) {
    const l = line.toLowerCase();
    if (l.includes(' error') || l.includes('[error]') || l.includes('error:')) return 'line-error';
    if (l.includes(' warn')  || l.includes('[warn]')  || l.includes('warning:')) return 'line-warn';
    if (l.includes(' debug') || l.includes('[debug]')) return 'line-debug';
    if (l.includes(' info')  || l.includes('[info]'))  return 'line-info';
    return '';
  }

  // ── Lifecycle ────────────────────────────────────────────────────────────────
  onMount(async () => {
    await refresh();

    // Load last 200 lines from the log file
    try {
      const initial = await invoke('core_log_read', { lines: 200 });
      lines = initial;
      scrollToBottom();
    } catch {}

    // Listen for live appended lines
    unlisten = await listen('core-log', (ev) => {
      appendLine(ev.payload);
    });

    // Poll service status every 5 s
    const interval = setInterval(refresh, 5000);
    return () => clearInterval(interval);
  });

  onDestroy(() => {
    if (unlisten) unlisten();
  });
</script>

<!-- ── Service status card ─────────────────────────────────────────────────── -->
<div class="core-layout">

  <div class="status-bar">
    <div class="status-left">
      <span class="status-dot" style="background:{stateColor(status)}"></span>
      <span class="status-text">VeloceCore — <strong>{stateLabel(status)}</strong></span>
      {#if logPath}
        <span class="log-path">{logPath}</span>
      {/if}
    </div>
    <div class="status-actions">
      <button class="btn btn-sm btn-primary"
              disabled={busy || status === 'running' || status === 'starting'}
              on:click={startService}>▶ Start</button>
      <button class="btn btn-sm btn-danger"
              disabled={busy || status === 'stopped' || status === 'stopping'}
              on:click={stopService}>■ Stop</button>
      <button class="btn btn-sm btn-secondary"
              disabled={busy || status === 'stopped'}
              on:click={restartService}>↺ Restart</button>
      <button class="btn btn-sm btn-ghost" on:click={refresh}>⟳</button>
      <button class="btn btn-sm btn-ghost" on:click={clearLog}>✕ Clear</button>
    </div>
  </div>

  <!-- ── Terminal pane ────────────────────────────────────────────────────── -->
  <div class="terminal" bind:this={termEl} on:scroll={onTermScroll}>
    {#if lines.length === 0}
      <span class="term-empty">No log output yet. Start VeloceCore to see output here.</span>
    {:else}
      {#each lines as line}
        <div class="term-line {lineClass(line)}">{line}</div>
      {/each}
    {/if}
  </div>

  {#if !autoScroll}
    <button class="scroll-lock-btn" on:click={() => { autoScroll = true; scrollToBottom(); }}>
      ↓ Scroll to bottom
    </button>
  {/if}

</div>

<style>
  .core-layout {
    display: flex;
    flex-direction: column;
    height: calc(100vh - 100px);
    gap: 10px;
  }

  /* Status bar */
  .status-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: #161b22;
    border: 1px solid #21262d;
    border-radius: 8px;
    padding: 10px 14px;
    flex-shrink: 0;
    flex-wrap: wrap;
    gap: 8px;
  }

  .status-left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .status-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
    box-shadow: 0 0 6px currentColor;
  }

  .status-text { font-size: 14px; }
  .status-text strong { color: #e6edf3; }

  .log-path {
    font-size: 11px;
    color: #484f58;
    font-family: monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }

  .status-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  /* Terminal */
  .terminal {
    flex: 1;
    background: #0d1117;
    border: 1px solid #21262d;
    border-radius: 8px;
    padding: 10px 14px;
    overflow-y: auto;
    font-family: 'Cascadia Code', 'Consolas', 'Courier New', monospace;
    font-size: 12px;
    line-height: 1.55;
    color: #c9d1d9;
    min-height: 0;
  }

  .terminal::-webkit-scrollbar       { width: 6px; }
  .terminal::-webkit-scrollbar-track { background: transparent; }
  .terminal::-webkit-scrollbar-thumb { background: #30363d; border-radius: 3px; }

  .term-empty {
    color: #484f58;
    font-style: italic;
    font-size: 13px;
  }

  .term-line {
    white-space: pre-wrap;
    word-break: break-all;
    padding: 1px 0;
  }

  .line-error { color: #f85149; }
  .line-warn  { color: #d29922; }
  .line-info  { color: #79c0ff; }
  .line-debug { color: #484f58; }

  /* Scroll-to-bottom pill */
  .scroll-lock-btn {
    position: absolute;
    bottom: 24px;
    right: 28px;
    padding: 5px 12px;
    background: #21262d;
    border: 1px solid #30363d;
    border-radius: 20px;
    color: #8b949e;
    font-size: 12px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .scroll-lock-btn:hover { background: #30363d; color: #e6edf3; }
</style>
