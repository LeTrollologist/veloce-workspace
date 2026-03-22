<script>
  import { createEventDispatcher } from 'svelte';
  import { nodes, resources, resourceHistory, connected } from '../stores.js';
  import { api } from '../lib/tauri.js';
  import { drawSparkline, clearCanvas } from '../lib/canvas.js';

  const dispatch = createEventDispatcher();

  let spawnName = '';
  let spawnExe  = '';
  let spawnMsg  = '';
  let spawnOk   = false;

  // Expanded detail panel
  let expandedNodeId = null;

  async function spawnNode() {
    if (!spawnName.trim() || !spawnExe.trim()) {
      spawnMsg = 'App name and executable are required.';
      spawnOk  = false;
      return;
    }
    try {
      const r = await api.spawnNode(spawnName.trim(), spawnExe.trim());
      spawnMsg = `Spawned — PID ${r.pid}  ·  ${r.node_id.slice(0, 8)}…`;
      spawnOk  = true;
      spawnName = spawnExe = '';
      dispatch('refresh');
    } catch (e) {
      spawnMsg = 'Error: ' + e;
      spawnOk  = false;
    }
  }

  async function killNode(nodeId) {
    try {
      await api.killNode(nodeId);
      dispatch('refresh');
    } catch (e) {
      alert('Kill failed:\n' + e);
    }
  }

  function toggleExpand(nodeId) {
    expandedNodeId = expandedNodeId === nodeId ? null : nodeId;
  }

  function statusClass(status) {
    const s = status.toLowerCase();
    if (s.includes('running')) return 'badge-running';
    if (s.includes('start'))   return 'badge-starting';
    return 'badge-stopped';
  }

  // Canvas sparkline action — draws inline CPU% sparkline for a node row.
  function sparklineAction(canvas, nodeId) {
    let raf;
    function draw() {
      const ctx  = canvas.getContext('2d');
      const hist = $resourceHistory[nodeId];
      clearCanvas(ctx, canvas.width, canvas.height);
      if (!hist || hist.length < 2) return;
      // Compute cpu_pct series from cumulative cpu_ms deltas
      const pts = [];
      for (let i = 1; i < hist.length; i++) {
        const dt_wall = hist[i].ts - hist[i - 1].ts;
        const dt_cpu  = hist[i].cpu_ms - hist[i - 1].cpu_ms;
        pts.push(dt_wall > 0 ? Math.min(100, (dt_cpu / dt_wall) * 100) : 0);
      }
      drawSparkline(ctx, 0, 1, canvas.width, canvas.height - 2, pts.slice(-30), '#6366f1');
    }
    function loop() { draw(); raf = requestAnimationFrame(loop); }
    loop();
    return { destroy() { cancelAnimationFrame(raf); } };
  }

  // Canvas detail graph action — draws CPU% + Memory for a node detail panel.
  function detailGraphAction(canvas, { nodeId, type }) {
    let raf;
    function draw() {
      const ctx  = canvas.getContext('2d');
      const hist = ($resourceHistory[nodeId] || []);
      clearCanvas(ctx, canvas.width, canvas.height);
      if (hist.length < 2) return;

      let pts, color, maxVal;
      if (type === 'cpu') {
        pts = [];
        for (let i = 1; i < hist.length; i++) {
          const dt_wall = hist[i].ts - hist[i - 1].ts;
          const dt_cpu  = hist[i].cpu_ms - hist[i - 1].cpu_ms;
          pts.push(dt_wall > 0 ? Math.min(100, (dt_cpu / dt_wall) * 100) : 0);
        }
        color  = '#6366f1';
        maxVal = 100;
      } else {
        pts    = hist.map(h => h.mem_bytes / (1024 * 1024));
        color  = '#f59e0b';
        maxVal = Math.max(...pts, 1);
      }
      drawSparkline(ctx, 0, 4, canvas.width, canvas.height - 8, pts.slice(-120), color, 0, maxVal);
    }
    function loop() { draw(); raf = requestAnimationFrame(loop); }
    loop();
    return { destroy() { cancelAnimationFrame(raf); } };
  }
</script>

<div class="panel-row">
  <h2>Running Nodes</h2>
  <button class="btn btn-ghost" on:click={() => dispatch('refresh')}>↻ Refresh</button>
</div>

<div class="table-wrap">
  <table>
    <thead>
      <tr>
        <th>Name</th>
        <th>PID</th>
        <th>Status</th>
        <th>CPU%</th>
        <th style="width:90px">CPU trend</th>
        <th>Memory</th>
        <th>Spawned</th>
        <th>Node ID</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#if $nodes.length === 0}
        <tr><td colspan="9" class="empty-cell">No nodes running</td></tr>
      {:else}
        {#each $nodes as node (node.node_id)}
          <tr class="node-row" class:expanded={expandedNodeId === node.node_id}
              on:click={() => toggleExpand(node.node_id)} style="cursor:pointer">
            <td><strong>{node.app_name}</strong></td>
            <td>{node.pid}</td>
            <td><span class="badge {statusClass(node.status)}">{node.status}</span></td>
            <td class="res-cell">
              {#if $resources[node.node_id]?.cpu_pct != null}
                <span class="res-val">{$resources[node.node_id].cpu_pct}%</span>
              {:else}
                <span class="res-muted">—</span>
              {/if}
            </td>
            <td>
              <canvas width="80" height="22" use:sparklineAction={node.node_id}
                      style="display:block;border-radius:2px;background:rgba(255,255,255,0.04)">
              </canvas>
            </td>
            <td class="res-cell">
              {#if $resources[node.node_id]?.mem_mb != null}
                <span class="res-val">{$resources[node.node_id].mem_mb} MB</span>
              {:else}
                <span class="res-muted">—</span>
              {/if}
            </td>
            <td class="mono">{node.spawned_at}</td>
            <td class="mono">{node.node_id.slice(0, 8)}…</td>
            <td on:click|stopPropagation>
              <button class="btn btn-danger btn-sm" on:click={() => killNode(node.node_id)}
                      disabled={!$connected}>Kill</button>
            </td>
          </tr>

          {#if expandedNodeId === node.node_id}
            <tr class="detail-row" on:click|stopPropagation>
              <td colspan="9">
                <div class="detail-panel">
                  <div class="detail-graph">
                    <div class="graph-label">CPU %  (last 120 polls)</div>
                    <canvas width="400" height="70" use:detailGraphAction={{ nodeId: node.node_id, type: 'cpu' }}
                            style="display:block;width:100%;height:70px;border-radius:4px;background:rgba(255,255,255,0.03)">
                    </canvas>
                  </div>
                  <div class="detail-graph">
                    <div class="graph-label">Memory MB  (last 120 polls)</div>
                    <canvas width="400" height="70" use:detailGraphAction={{ nodeId: node.node_id, type: 'mem' }}
                            style="display:block;width:100%;height:70px;border-radius:4px;background:rgba(255,255,255,0.03)">
                    </canvas>
                  </div>
                  <div class="detail-meta">
                    <span>Node ID: <span class="mono">{node.node_id}</span></span>
                  </div>
                </div>
              </td>
            </tr>
          {/if}
        {/each}
      {/if}
    </tbody>
  </table>
</div>

<div class="card">
  <h3>Spawn Node</h3>
  <div class="form-row">
    <input class="input" bind:value={spawnName} placeholder="App name" disabled={!$connected} />
    <input class="input wide" bind:value={spawnExe} placeholder="C:\path\to\app.exe" disabled={!$connected} />
    <button class="btn btn-primary" on:click={spawnNode} disabled={!$connected}>Spawn</button>
  </div>
  {#if spawnMsg}
    <p class={spawnOk ? 'result-ok' : 'result-error'}>{spawnMsg}</p>
  {/if}
</div>

<style>
  .res-cell  { white-space: nowrap; }
  .res-val   { font-weight: 600; }
  .res-muted { color: #484f58; }

  .node-row:hover { background: rgba(99,102,241,0.04); }
  .node-row.expanded { background: rgba(99,102,241,0.06); }

  .detail-row td { padding: 0; }
  .detail-panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 16px 14px;
    background: rgba(255,255,255,0.02);
    border-top: 1px solid #21262d;
  }
  .detail-graph { flex: 1; }
  .graph-label { font-size: 11px; color: #8b949e; margin-bottom: 4px; }
  .detail-meta { font-size: 12px; color: #484f58; }
</style>
