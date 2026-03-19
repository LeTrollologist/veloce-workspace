<script>
  import { onMount, onDestroy } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { api } from './lib/tauri.js';
  import {
    connected, nodes, templates, meshInfo,
    logLines, resources, resourceHistory,
    traffic, trafficHistory, policy,
  } from './stores.js';

  import NodesTab     from './components/NodesTab.svelte';
  import TemplatesTab from './components/TemplatesTab.svelte';
  import NetworkTab   from './components/NetworkTab.svelte';
  import LogsTab      from './components/LogsTab.svelte';
  import TopologyTab  from './components/TopologyTab.svelte';

  // ── Tab state ──────────────────────────────────────────────────────────────
  let activeTab = 'nodes';
  const TABS = [
    { id: 'nodes',    label: 'Nodes'      },
    { id: 'templates',label: 'Templates'  },
    { id: 'network',  label: 'VeloceNet'  },
    { id: 'logs',     label: 'Logs'       },
    { id: 'topology', label: 'Topology'   },
  ];

  // ── Resource polling state ─────────────────────────────────────────────────
  const resourceBaseline = {};  // nodeId → {cpu_ms, ts}
  const MAX_HISTORY = 120;

  // Interval handles — cleared on disconnect and on component destroy
  let resourcePollId;
  let meshHeartbeatId;

  // ── Connection management ──────────────────────────────────────────────────
  function clearPolling() {
    if (resourcePollId)   { clearInterval(resourcePollId);   resourcePollId   = undefined; }
    if (meshHeartbeatId)  { clearInterval(meshHeartbeatId);  meshHeartbeatId  = undefined; }
  }

  async function toggleConnect() {
    if ($connected) {
      clearPolling();
      await api.disconnect().catch(() => {});
      connected.set(false);
      nodes.set([]);
      meshInfo.set(null);
    } else {
      try {
        await api.connect();
        connected.set(true);
        await refreshAll();
      } catch (e) {
        alert('Connect failed:\n' + e);
      }
    }
  }

  async function refreshAll() {
    await Promise.allSettled([refreshNodes(), refreshTemplates(), refreshMeshInfo(), refreshPolicy()]);
  }

  async function refreshNodes() {
    if (!$connected) return;
    try { nodes.set(await api.listNodes()); } catch {}
  }

  async function refreshTemplates() {
    if (!$connected) return;
    try {
      const names = await api.listTemplates();
      const full  = await Promise.all(names.map(n => api.getTemplate(n)));
      templates.set(full.filter(Boolean));
    } catch {}
  }

  async function refreshMeshInfo() {
    if (!$connected) return;
    try { meshInfo.set(await api.meshInfo()); } catch {}
  }

  async function refreshPolicy() {
    if (!$connected) return;
    try { policy.set(await api.policyShow()); } catch {}
  }

  async function pollResources() {
    if (!$connected) return;
    try {
      const now  = Date.now();
      const rows = await api.queryResources();

      resources.update(cur => {
        const updated = { ...cur };
        for (const r of rows) {
          const prev = resourceBaseline[r.node_id];
          const mem_mb = (r.mem_bytes / (1024 * 1024)).toFixed(1);
          let cpu_pct = prev ? Math.min(100, ((r.cpu_ms - prev.cpu_ms) / (now - prev.ts)) * 100).toFixed(1) : null;
          updated[r.node_id] = { cpu_pct, mem_mb, cpu_ms: r.cpu_ms, mem_bytes: r.mem_bytes };
          resourceBaseline[r.node_id] = { cpu_ms: r.cpu_ms, ts: now };
        }
        // Remove stale
        for (const id of Object.keys(updated)) {
          if (!rows.find(r => r.node_id === id)) delete updated[id];
        }
        return updated;
      });

      // Update history for sparklines
      resourceHistory.update(hist => {
        const updated = { ...hist };
        for (const r of rows) {
          if (!updated[r.node_id]) updated[r.node_id] = [];
          updated[r.node_id] = [
            ...updated[r.node_id],
            { ts: now, cpu_ms: r.cpu_ms, mem_bytes: r.mem_bytes },
          ].slice(-MAX_HISTORY);
        }
        return updated;
      });
    } catch {}
  }

  // ── Tauri event listeners ──────────────────────────────────────────────────
  onMount(async () => {
    // Log chunks
    await listen('node-log', (ev) => {
      const { node_id, stream, text } = ev.payload;
      logLines.update(cur => {
        const lines = [...(cur[node_id] || []), { stream, text, ts: new Date().toLocaleTimeString('en-US', { hour12: false }) }];
        return { ...cur, [node_id]: lines.slice(-5000) };
      });
    });

    // Node lifecycle events
    await listen('node-event', (ev) => {
      const { event: evName } = ev.payload;
      if (evName.includes('Exited') || evName.includes('Restart')) refreshNodes();
    });

    // Traffic snapshots (pushed from Tauri every 2s)
    await listen('traffic-update', (ev) => {
      const snap = ev.payload;
      traffic.set(snap);
      trafficHistory.update(h => [...h, snap].slice(-60));
    });

    // Resource poll every 5 s
    resourcePollId = setInterval(pollResources, 5_000);
    pollResources();

    // Mesh info refresh every 10 s + heartbeat ping
    meshHeartbeatId = setInterval(async () => {
      if (!$connected) return;
      await refreshMeshInfo();
      try {
        const ok = await api.ping();
        if (!ok) connected.set(false);
      } catch { connected.set(false); }
    }, 10_000);
  });

  onDestroy(clearPolling);
</script>

<!-- ── Shell ─────────────────────────────────────────────────────────────── -->
<div class="shell">
  <header>
    <div class="brand">
      <span class="brand-v">V</span>eloceNetwork
    </div>
    <div class="status-bar">
      <span class="dot" class:connected={$connected} class:disconnected={!$connected}></span>
      <span class="status-label">{$connected ? 'Connected' : 'Disconnected'}</span>
      <button class="btn" class:btn-primary={!$connected} class:btn-danger={$connected}
              on:click={toggleConnect}>
        {$connected ? 'Disconnect' : 'Connect'}
      </button>
    </div>
  </header>

  <nav class="tabs">
    {#each TABS as tab}
      <button class="tab-btn" class:active={activeTab === tab.id}
              on:click={() => activeTab = tab.id}>
        {tab.label}
      </button>
    {/each}
  </nav>

  <main class="tab-content">
    {#if activeTab === 'nodes'}
      <NodesTab on:refresh={refreshNodes} />
    {:else if activeTab === 'templates'}
      <TemplatesTab on:refresh={refreshTemplates} />
    {:else if activeTab === 'network'}
      <NetworkTab on:refreshMesh={refreshMeshInfo} on:refreshPolicy={refreshPolicy} />
    {:else if activeTab === 'logs'}
      <LogsTab />
    {:else if activeTab === 'topology'}
      <TopologyTab />
    {/if}
  </main>
</div>

<style>
  :global(*, *::before, *::after) { box-sizing: border-box; margin: 0; padding: 0; }
  :global(body) {
    background: #0d1117;
    color: #e6edf3;
    font-family: system-ui, -apple-system, sans-serif;
    font-size: 14px;
    overflow: hidden;
  }
  :global(::-webkit-scrollbar) { width: 6px; }
  :global(::-webkit-scrollbar-track) { background: #161b22; }
  :global(::-webkit-scrollbar-thumb) { background: #30363d; border-radius: 3px; }

  /* Layout */
  .shell { display: flex; flex-direction: column; height: 100vh; }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px;
    height: 48px;
    background: #161b22;
    border-bottom: 1px solid #21262d;
    flex-shrink: 0;
  }

  .brand { font-size: 16px; font-weight: 700; letter-spacing: 0.5px; }
  .brand-v { color: #6366f1; }

  .status-bar { display: flex; align-items: center; gap: 10px; }

  .dot {
    width: 8px; height: 8px;
    border-radius: 50%;
    background: #6e7681;
    flex-shrink: 0;
  }
  .dot.connected    { background: #3fb950; box-shadow: 0 0 6px #3fb95066; }
  .dot.disconnected { background: #f85149; }

  .status-label { font-size: 13px; color: #8b949e; }

  /* Tabs */
  .tabs {
    display: flex;
    gap: 2px;
    padding: 0 12px;
    background: #161b22;
    border-bottom: 1px solid #21262d;
    flex-shrink: 0;
  }
  .tab-btn {
    padding: 8px 14px;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: #8b949e;
    cursor: pointer;
    font-size: 13px;
    transition: color 0.15s, border-color 0.15s;
  }
  .tab-btn:hover   { color: #e6edf3; }
  .tab-btn.active  { color: #e6edf3; border-bottom-color: #6366f1; }

  .tab-content {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
  }

  /* Global button styles */
  :global(.btn) {
    display: inline-flex; align-items: center; justify-content: center;
    padding: 5px 12px;
    border: 1px solid transparent;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: opacity 0.15s, background 0.15s;
    white-space: nowrap;
  }
  :global(.btn:disabled) { opacity: 0.4; cursor: not-allowed; }
  :global(.btn-primary)  { background: #238636; color: #fff; border-color: #2ea043; }
  :global(.btn-primary:hover:not(:disabled)) { background: #2ea043; }
  :global(.btn-danger)   { background: #da3633; color: #fff; border-color: #f85149; }
  :global(.btn-danger:hover:not(:disabled))  { background: #f85149; }
  :global(.btn-secondary){ background: #21262d; color: #e6edf3; border-color: #30363d; }
  :global(.btn-secondary:hover:not(:disabled)){ background: #30363d; }
  :global(.btn-ghost)    { background: transparent; color: #8b949e; border-color: #30363d; }
  :global(.btn-ghost:hover:not(:disabled)) { color: #e6edf3; background: #21262d; }
  :global(.btn-sm)       { padding: 3px 8px; font-size: 12px; }

  /* Global input */
  :global(.input) {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 6px;
    color: #e6edf3;
    font-size: 13px;
    padding: 5px 10px;
    outline: none;
    transition: border-color 0.15s;
  }
  :global(.input:focus) { border-color: #6366f1; }
  :global(.input.wide)  { flex: 1; min-width: 0; }
  :global(.input.narrow){ width: 80px; }

  /* Global card */
  :global(.card) {
    background: #161b22;
    border: 1px solid #21262d;
    border-radius: 8px;
    padding: 14px 16px;
    margin-bottom: 14px;
  }
  :global(.card h3)  { font-size: 14px; font-weight: 600; margin-bottom: 10px; color: #e6edf3; }
  :global(.card-header) { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }

  /* Form rows */
  :global(.form-row) {
    display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
    margin-bottom: 6px;
  }

  /* Tables */
  :global(.table-wrap) { overflow-x: auto; border-radius: 6px; border: 1px solid #21262d; margin-bottom: 14px; }
  :global(table)       { width: 100%; border-collapse: collapse; font-size: 13px; }
  :global(th)          { background: #161b22; padding: 8px 12px; text-align: left; color: #8b949e; font-weight: 600; font-size: 12px; border-bottom: 1px solid #21262d; white-space: nowrap; }
  :global(td)          { padding: 7px 12px; border-bottom: 1px solid #21262d; vertical-align: middle; }
  :global(tr:last-child td) { border-bottom: none; }
  :global(tr:hover td)      { background: rgba(255,255,255,0.02); }
  :global(.empty-cell)      { text-align: center; color: #484f58; padding: 24px; }

  /* Result messages */
  :global(.result-ok)    { color: #3fb950; font-size: 13px; margin-top: 6px; }
  :global(.result-error) { color: #f85149; font-size: 13px; margin-top: 6px; }

  /* Badges */
  :global(.badge) { display: inline-block; padding: 2px 7px; border-radius: 10px; font-size: 11px; font-weight: 600; }
  :global(.badge-running)  { background: #033a16; color: #3fb950; }
  :global(.badge-starting) { background: #3d2000; color: #d29922; }
  :global(.badge-stopped)  { background: #2d1215; color: #f85149; }
  :global(.mono)            { font-family: monospace; font-size: 12px; color: #8b949e; }
  :global(.panel-row)      { display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px; }
  :global(.panel-row h2)   { font-size: 16px; font-weight: 600; }
</style>
