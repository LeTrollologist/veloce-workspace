<script>
  import { nodes, logLines, connected } from '../stores.js';
  import { api } from '../lib/tauri.js';

  let selectedNodeId = '';
  let showStdout  = true;
  let showStderr  = true;
  let showTs      = true;
  let search      = '';
  let autoScroll  = true;
  let viewer;

  // When node selection changes, subscribe to its log stream
  async function onNodeChange() {
    if (selectedNodeId && $connected) {
      try { await api.subscribeLogs(selectedNodeId); } catch {}
    }
  }

  function clearLog() {
    logLines.update(cur => ({ ...cur, [selectedNodeId]: [] }));
  }

  // Auto-scroll logic
  function maybeScroll() {
    if (autoScroll && viewer) {
      requestAnimationFrame(() => {
        viewer.scrollTop = viewer.scrollHeight;
      });
    }
  }

  // Filtered lines reactive
  $: lines = (selectedNodeId && $logLines[selectedNodeId])
    ? $logLines[selectedNodeId].filter(l => {
        if (!showStdout && l.stream === 'stdout') return false;
        if (!showStderr && l.stream === 'stderr') return false;
        if (search.trim() && !l.text.toLowerCase().includes(search.toLowerCase())) return false;
        return true;
      })
    : [];

  $: if (lines) maybeScroll();

  function streamClass(stream) {
    return stream === 'stderr' ? 'log-stderr' : 'log-stdout';
  }
</script>

<div class="panel-row">
  <h2>Node Logs</h2>
  <div class="log-controls">
    <select class="input" bind:value={selectedNodeId} on:change={onNodeChange}>
      <option value="">— select a node —</option>
      {#each $nodes as n (n.node_id)}
        <option value={n.node_id}>{n.app_name} ({n.node_id.slice(0, 8)}…)</option>
      {/each}
    </select>

    <input class="input" style="width:180px" bind:value={search} placeholder="Filter…" />

    <label class="toggle-label">
      <input type="checkbox" bind:checked={showStdout} /> stdout
    </label>
    <label class="toggle-label">
      <input type="checkbox" bind:checked={showStderr} /> stderr
    </label>
    <label class="toggle-label">
      <input type="checkbox" bind:checked={showTs} /> ts
    </label>
    <label class="toggle-label">
      <input type="checkbox" bind:checked={autoScroll} /> auto-scroll
    </label>
    <button class="btn btn-ghost btn-sm" on:click={clearLog} disabled={!selectedNodeId}>Clear</button>
  </div>
</div>

<div class="log-viewer" bind:this={viewer}>
  {#if !selectedNodeId}
    <div class="empty-cell">Select a node to view its output</div>
  {:else if lines.length === 0}
    <div class="empty-cell">{search ? 'No matches' : 'No output yet'}</div>
  {:else}
    {#each lines as l}
      <div class="log-line {streamClass(l.stream)}">
        {#if showTs}<span class="ts">{l.ts}</span>{/if}{l.text}
      </div>
    {/each}
  {/if}
</div>

<style>
  .log-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .log-viewer {
    background: #0d1117;
    border: 1px solid #21262d;
    border-radius: 6px;
    padding: 10px 12px;
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: 12px;
    line-height: 1.5;
    height: calc(100vh - 200px);
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .log-line    { margin-bottom: 1px; }
  .log-stderr  { color: #f85149; }
  .log-stdout  { color: #adbac7; }

  .ts {
    color: #484f58;
    margin-right: 8px;
    user-select: none;
  }

  .toggle-label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: #8b949e;
    cursor: pointer;
    white-space: nowrap;
  }
</style>
