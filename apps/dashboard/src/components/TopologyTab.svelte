<script>
  import { onMount, onDestroy } from 'svelte';
  import { nodes, meshInfo, traffic, trafficHistory, topoPositions } from '../stores.js';
  import { drawCircle, drawRect, drawEdge, trafficColor, bytesPerSec, clearCanvas } from '../lib/canvas.js';

  // ── Canvas refs ──────────────────────────────────────────────────────────────
  let topoCanvas;
  let heatCanvas;
  let rafId;
  let resizeObserver;

  // ── Interaction state ─────────────────────────────────────────────────────────
  let drag = null;  // { nodeKey, ox, oy }
  let hoveredKey = null;

  // ── Layout helpers ────────────────────────────────────────────────────────────
  const NODE_R       = 30;
  const HOST_W       = 80;
  const HOST_H       = 22;
  const HEAT_CELL_W  = 10;
  const HEAT_CELL_H  = 20;
  const HEAT_COLS    = 60;

  // Build canonical node list from store values
  // Each entry: { key, label, type: 'machine'|'host', x, y }
  function buildNodes(nodeList, info, tpos) {
    const result = [];
    const used   = new Set();

    // This machine (always shown if we have mesh info)
    if (info) {
      const key = `machine:${info.machine_id}`;
      used.add(key);
      const pos = tpos[key] || defaultPos(0, nodeList.length + 1);
      result.push({ key, label: 'This Machine', type: 'machine', ...pos });
    }

    // Connected peers
    const peers = info?.peers || [];
    peers.forEach((p, i) => {
      const key = `peer:${p.peer_id}`;
      used.add(key);
      const pos = tpos[key] || defaultPos(i + 1, peers.length);
      result.push({ key, label: p.peer_name || p.peer_id.slice(0, 8), type: 'machine', ...pos });
    });

    // Running local nodes as .vln hosts
    nodeList.forEach((n, i) => {
      const key = `node:${n.node_id}`;
      used.add(key);
      const pos = tpos[key] || { x: 60 + i * 100, y: 300 };
      result.push({ key, label: n.app_name, type: 'host', ...pos });
    });

    return result;
  }

  function defaultPos(idx, total) {
    const cx = 400, cy = 200, r = 150;
    const angle = (idx / Math.max(total, 1)) * Math.PI * 2 - Math.PI / 2;
    return { x: cx + Math.cos(angle) * r, y: cy + Math.sin(angle) * r };
  }

  // Bytes/sec from last two traffic history snapshots for a given tunnel peer_id
  function tunnelBps(peerId) {
    const h = $trafficHistory;
    if (h.length < 2) return 0;
    const cur  = h[h.length - 1];
    const prev = h[h.length - 2];
    const cT   = cur.tunnels?.find(t => t.peer_id === peerId);
    const pT   = prev.tunnels?.find(t => t.peer_id === peerId);
    if (!cT || !pT) return 0;
    return bytesPerSec(cT.tx_bytes + cT.rx_bytes, pT.tx_bytes + pT.rx_bytes, cur.ts_ms, prev.ts_ms);
  }

  // Max observed bps (for colour normalization)
  $: maxBps = (() => {
    const h = $trafficHistory;
    if (h.length < 2) return 1;
    let m = 1;
    const cur  = h[h.length - 1];
    const prev = h[h.length - 2];
    for (const t of cur.tunnels || []) {
      const pT = prev.tunnels?.find(p => p.peer_id === t.peer_id);
      if (!pT) continue;
      const bps = bytesPerSec(t.tx_bytes + t.rx_bytes, pT.tx_bytes + pT.rx_bytes, cur.ts_ms, prev.ts_ms);
      if (bps > m) m = bps;
    }
    return m;
  })();

  // ── Canvas render ─────────────────────────────────────────────────────────────
  function renderTopo() {
    if (!topoCanvas) return;
    // Use logical (CSS) dimensions for drawing coordinates since ctx is scaled by DPR
    const dpr = window.devicePixelRatio || 1;
    const w   = topoCanvas.width  / dpr;
    const h   = topoCanvas.height / dpr;
    const ctx = topoCanvas.getContext('2d');
    clearCanvas(ctx, w, h);

    const nodesData = buildNodes($nodes, $meshInfo, $topoPositions);

    // Draw edges (tunnels = peer-to-peer connections)
    const peers = $meshInfo?.peers || [];
    const thisKey = $meshInfo ? `machine:${$meshInfo.machine_id}` : null;
    const thisNode = nodesData.find(n => n.key === thisKey);

    for (const p of peers) {
      const peerNode = nodesData.find(n => n.key === `peer:${p.peer_id}`);
      if (!thisNode || !peerNode) continue;
      const bps  = tunnelBps(p.peer_id);
      const frac = Math.min(1, bps / maxBps);
      const edgeW = 1 + Math.log2(1 + bps / 1024) * 0.8;
      const color = trafficColor(frac);
      drawEdge(ctx, thisNode.x, thisNode.y, peerNode.x, peerNode.y, edgeW, color);

      // bps label on edge midpoint
      if (bps > 0) {
        const mx = (thisNode.x + peerNode.x) / 2;
        const my = (thisNode.y + peerNode.y) / 2;
        ctx.fillStyle = '#8b949e';
        ctx.font = '10px system-ui';
        ctx.textAlign = 'center';
        ctx.fillText(fmtBps(bps), mx, my - 4);
      }
    }

    // Draw nodes
    for (const n of nodesData) {
      if (n.type === 'machine') {
        const fill = n.key === thisKey ? '#1f3a5f' : '#2d1b69';
        const border = hoveredKey === n.key ? '#6366f1' : '#30363d';
        drawCircle(ctx, n.x, n.y, NODE_R, fill, n.label);
        // Hovered ring
        if (hoveredKey === n.key) {
          ctx.beginPath();
          ctx.arc(n.x, n.y, NODE_R + 2, 0, Math.PI * 2);
          ctx.strokeStyle = '#6366f1';
          ctx.lineWidth = 2;
          ctx.stroke();
        }
      } else {
        drawRect(ctx, n.x - HOST_W / 2, n.y - HOST_H / 2, HOST_W, HOST_H, '#1e2a1e', n.label);
      }
    }
  }

  // ── Heatmap render ────────────────────────────────────────────────────────────
  function renderHeatmap() {
    if (!heatCanvas) return;
    const ctx = heatCanvas.getContext('2d');
    const peers = $meshInfo?.peers || [];
    const rows  = peers.length;
    if (rows === 0) { clearCanvas(ctx, heatCanvas.width, heatCanvas.height); return; }

    const W = HEAT_COLS * HEAT_CELL_W;
    const H = rows * (HEAT_CELL_H + 2);
    heatCanvas.width  = W;
    heatCanvas.height = H + 20; // +20 for peer name labels
    clearCanvas(ctx, heatCanvas.width, heatCanvas.height);

    peers.forEach((p, row) => {
      const y = row * (HEAT_CELL_H + 2) + 20;

      // Peer name
      ctx.fillStyle = '#8b949e';
      ctx.font      = '10px system-ui';
      ctx.textAlign = 'left';
      ctx.fillText(p.peer_name.slice(0, 12), 0, y - 4);

      // History cells
      for (let col = 0; col < HEAT_COLS; col++) {
        const snapIdx = $trafficHistory.length - HEAT_COLS + col;
        if (snapIdx < 1) {
          ctx.fillStyle = '#161b22';
          ctx.fillRect(col * HEAT_CELL_W, y, HEAT_CELL_W - 1, HEAT_CELL_H);
          continue;
        }
        const cur  = $trafficHistory[snapIdx];
        const prev = $trafficHistory[snapIdx - 1];
        const cT   = cur.tunnels?.find(t => t.peer_id === p.peer_id);
        const pT   = prev.tunnels?.find(t => t.peer_id === p.peer_id);
        const bps  = (cT && pT) ? bytesPerSec(cT.tx_bytes + cT.rx_bytes, pT.tx_bytes + pT.rx_bytes, cur.ts_ms, prev.ts_ms) : 0;
        const frac = Math.min(1, bps / maxBps);
        ctx.fillStyle = bps > 0 ? trafficColor(frac) : '#0d1117';
        ctx.fillRect(col * HEAT_CELL_W, y, HEAT_CELL_W - 1, HEAT_CELL_H);
      }
    });
  }

  // ── Drag-and-drop ─────────────────────────────────────────────────────────────
  function topoHitTest(ex, ey) {
    const nodesData = buildNodes($nodes, $meshInfo, $topoPositions);
    for (const n of [...nodesData].reverse()) {
      const r = n.type === 'machine' ? NODE_R : Math.max(HOST_W, HOST_H) / 2;
      const dx = ex - n.x;
      const dy = ey - n.y;
      if (dx * dx + dy * dy <= r * r) return n;
    }
    return null;
  }

  function onMouseDown(e) {
    const rect = topoCanvas.getBoundingClientRect();
    const ex = (e.clientX - rect.left) * (topoCanvas.width  / rect.width);
    const ey = (e.clientY - rect.top)  * (topoCanvas.height / rect.height);
    const hit = topoHitTest(ex, ey);
    if (hit) drag = { key: hit.key, ox: ex - hit.x, oy: ey - hit.y };
  }

  function onMouseMove(e) {
    const rect = topoCanvas.getBoundingClientRect();
    const ex = (e.clientX - rect.left) * (topoCanvas.width  / rect.width);
    const ey = (e.clientY - rect.top)  * (topoCanvas.height / rect.height);

    if (drag) {
      topoPositions.update(pos => ({
        ...pos,
        [drag.key]: { x: ex - drag.ox, y: ey - drag.oy },
      }));
    } else {
      const hit = topoHitTest(ex, ey);
      hoveredKey = hit ? hit.key : null;
    }
  }

  function onMouseUp() { drag = null; }

  // ── Animation loop ────────────────────────────────────────────────────────────
  function loop() {
    renderTopo();
    renderHeatmap();
    rafId = requestAnimationFrame(loop);
  }

  function resizeTopoCanvas() {
    const dpr = window.devicePixelRatio || 1;
    const w   = topoCanvas.offsetWidth  || 800;
    const h   = topoCanvas.offsetHeight || 400;
    topoCanvas.width  = w * dpr;
    topoCanvas.height = h * dpr;
    const ctx = topoCanvas.getContext('2d');
    ctx.scale(dpr, dpr);
  }

  onMount(() => {
    resizeTopoCanvas();
    loop();

    resizeObserver = new ResizeObserver(() => resizeTopoCanvas());
    resizeObserver.observe(topoCanvas);
  });

  onDestroy(() => {
    if (rafId) cancelAnimationFrame(rafId);
    if (resizeObserver) resizeObserver.disconnect();
  });

  // ── Utilities ─────────────────────────────────────────────────────────────────
  function fmtBps(bps) {
    if (bps < 1024)       return bps.toFixed(0) + ' B/s';
    if (bps < 1024 * 1024) return (bps / 1024).toFixed(1) + ' KB/s';
    return (bps / (1024 * 1024)).toFixed(2) + ' MB/s';
  }
</script>

<div class="topo-shell">
  <div class="topo-header">
    <h2>Network Topology</h2>
    <span class="hint">Drag nodes to rearrange · edge width = traffic</span>
  </div>

  <!-- Main topology canvas -->
  <canvas
    class="topo-canvas"
    bind:this={topoCanvas}
    on:mousedown={onMouseDown}
    on:mousemove={onMouseMove}
    on:mouseup={onMouseUp}
    on:mouseleave={onMouseUp}
  ></canvas>

  <!-- Traffic heatmap -->
  {#if ($meshInfo?.peers?.length ?? 0) > 0}
    <div class="heat-section">
      <div class="heat-title">
        Tunnel Traffic Heatmap
        <span class="heat-sub">60 cells · 2 s intervals · 2-min window</span>
      </div>
      <div class="heat-scroll">
        <canvas bind:this={heatCanvas}></canvas>
      </div>
    </div>
  {:else}
    <div class="no-peers-hint">Connect to peers to see the traffic heatmap.</div>
  {/if}

  <!-- Live traffic table -->
  {#if $traffic}
    <div class="traffic-section">
      <div class="traffic-title">Live Counters</div>
      <div class="traffic-cols">
        <!-- Tunnel stats -->
        {#if $traffic.tunnels?.length}
          <div class="traffic-block">
            <div class="traffic-subtitle">Tunnels (Noise)</div>
            <table>
              <thead><tr><th>Peer</th><th>↑ TX</th><th>↓ RX</th><th>Rate</th></tr></thead>
              <tbody>
                {#each $traffic.tunnels as t}
                  <tr>
                    <td>{t.peer_name}</td>
                    <td class="mono">{fmtBytes(t.tx_bytes)}</td>
                    <td class="mono">{fmtBytes(t.rx_bytes)}</td>
                    <td class="mono">{fmtBps(tunnelBps(t.peer_id))}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}

        <!-- Host stats -->
        {#if $traffic.hosts?.length}
          <div class="traffic-block">
            <div class="traffic-subtitle">.vln Hosts (SOCKS5)</div>
            <table>
              <thead><tr><th>Hostname</th><th>Bytes Proxied</th></tr></thead>
              <tbody>
                {#each $traffic.hosts as h}
                  <tr>
                    <td class="mono">{h.hostname}</td>
                    <td class="mono">{fmtBytes(h.bytes_proxied)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}

        {#if !$traffic.tunnels?.length && !$traffic.hosts?.length}
          <p class="muted-hint">No traffic data yet. Connect peers and route some traffic.</p>
        {/if}
      </div>
    </div>
  {/if}
</div>

<script context="module">
  function fmtBytes(b) {
    if (b < 1024)              return b + ' B';
    if (b < 1024 * 1024)       return (b / 1024).toFixed(1) + ' KB';
    if (b < 1024 * 1024 * 1024) return (b / (1024 * 1024)).toFixed(2) + ' MB';
    return (b / (1024 * 1024 * 1024)).toFixed(3) + ' GB';
  }
</script>

<style>
  .topo-shell  { display: flex; flex-direction: column; gap: 14px; }
  .topo-header { display: flex; align-items: baseline; gap: 14px; }
  .topo-header h2 { font-size: 16px; font-weight: 600; }
  .hint        { font-size: 12px; color: #484f58; }

  .topo-canvas {
    width: 100%;
    height: 420px;
    border-radius: 8px;
    border: 1px solid #21262d;
    background: #0d1117;
    cursor: crosshair;
    display: block;
  }

  .heat-section   { background: #161b22; border: 1px solid #21262d; border-radius: 8px; padding: 12px 14px; }
  .heat-title     { font-size: 13px; font-weight: 600; margin-bottom: 8px; }
  .heat-sub       { font-size: 11px; color: #484f58; font-weight: 400; margin-left: 8px; }
  .heat-scroll    { overflow-x: auto; }

  .no-peers-hint  { color: #484f58; font-size: 13px; text-align: center; padding: 16px; }

  .traffic-section   { background: #161b22; border: 1px solid #21262d; border-radius: 8px; padding: 12px 14px; }
  .traffic-title     { font-size: 13px; font-weight: 600; margin-bottom: 10px; }
  .traffic-cols      { display: flex; gap: 24px; flex-wrap: wrap; }
  .traffic-block     { flex: 1; min-width: 260px; }
  .traffic-subtitle  { font-size: 11px; color: #8b949e; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 6px; }
  .muted-hint        { color: #484f58; font-size: 13px; }
</style>
