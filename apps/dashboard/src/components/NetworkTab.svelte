<script>
  import { createEventDispatcher } from 'svelte';
  import { meshInfo, policy, connected } from '../stores.js';
  import { api } from '../lib/tauri.js';

  const dispatch = createEventDispatcher();

  // Register host
  let regHostname  = '';
  let regNodeId    = '';
  let regPort      = '';
  let regTtl       = '0';
  let regMsg       = '';
  let regOk        = false;

  // Unregister host
  let unregHostname = '';
  let unregMsg      = '';
  let unregOk       = false;

  // Mesh connect
  let joinCode      = '';
  let connectMsg    = '';
  let connectOk     = false;

  // Policy section open/closed
  let policyExpanded = false;

  async function registerHost() {
    if (!regHostname.trim() || !regNodeId.trim()) {
      regMsg = 'Hostname and Node UUID are required.'; regOk = false; return;
    }
    try {
      await api.registerHost(regHostname.trim(), regNodeId.trim(), parseInt(regPort) || 0, parseInt(regTtl) || 0);
      regMsg = `${regHostname} registered.`; regOk = true;
    } catch (e) { regMsg = 'Error: ' + e; regOk = false; }
  }

  async function unregisterHost() {
    if (!unregHostname.trim()) { unregMsg = 'Hostname required.'; unregOk = false; return; }
    try {
      await api.unregisterHost(unregHostname.trim());
      unregMsg = `${unregHostname} unregistered.`; unregOk = true;
    } catch (e) { unregMsg = 'Error: ' + e; unregOk = false; }
  }

  async function connectPeer() {
    if (!joinCode.trim()) { connectMsg = 'Paste a join code first.'; connectOk = false; return; }
    try {
      const r = await api.meshConnect(joinCode.trim());
      connectMsg = `✓ Connected to ${r.peer_name} (${r.peer_id.slice(0,8)}…)`; connectOk = true;
      joinCode = '';
      dispatch('refreshMesh');
    } catch (e) { connectMsg = 'Connect failed: ' + e; connectOk = false; }
  }

  async function disconnectPeer(peerId) {
    try { await api.meshDisconnect(peerId); dispatch('refreshMesh'); }
    catch (e) { alert('Disconnect failed:\n' + e); }
  }

  async function copyJoinCode() {
    if ($meshInfo?.join_code) await navigator.clipboard.writeText($meshInfo.join_code).catch(() => {});
  }

  async function reloadPolicy() {
    try {
      const p = await api.policyReload();
      policy.set(p);
      dispatch('refreshPolicy');
    } catch (e) { alert('Reload failed:\n' + e); }
  }
</script>

<!-- Register / Unregister host ─────────────────────────────────────────── -->
<div class="card">
  <h3>Register Host</h3>
  <div class="form-row">
    <input class="input" bind:value={regHostname} placeholder="myapp.vln" />
    <input class="input wide" bind:value={regNodeId}  placeholder="Node UUID" />
    <input class="input narrow" type="number" bind:value={regPort} placeholder="Port" />
    <input class="input narrow" type="number" bind:value={regTtl}  placeholder="TTL (s)" />
    <button class="btn btn-primary" on:click={registerHost} disabled={!$connected}>Register</button>
  </div>
  {#if regMsg}<p class={regOk ? 'result-ok' : 'result-error'}>{regMsg}</p>{/if}
</div>

<div class="card">
  <h3>Unregister Host</h3>
  <div class="form-row">
    <input class="input" bind:value={unregHostname} placeholder="myapp.vln" />
    <button class="btn btn-danger" on:click={unregisterHost} disabled={!$connected}>Unregister</button>
  </div>
  {#if unregMsg}<p class={unregOk ? 'result-ok' : 'result-error'}>{unregMsg}</p>{/if}
</div>

<!-- Mesh identity ────────────────────────────────────────────────────────── -->
<div class="card">
  <h3>This Machine</h3>
  {#if $meshInfo}
    <div class="ident-row">
      <span class="label">Machine ID</span>
      <span class="mono-chip" title={$meshInfo.machine_id}>{$meshInfo.machine_id.slice(0, 18)}…</span>
    </div>
    <div class="ident-row">
      <span class="label">Join Code</span>
      <input class="input wide" value={$meshInfo.join_code} readonly />
      <button class="btn btn-secondary btn-sm" on:click={copyJoinCode}>Copy</button>
    </div>
    <div class="ident-row">
      <span class="label">Port</span>
      <span class="mono-chip">{$meshInfo.listen_port}</span>
    </div>
  {:else}
    <p class="muted">Connect to load mesh identity.</p>
  {/if}
</div>

<!-- Connect to peer ──────────────────────────────────────────────────────── -->
<div class="card">
  <h3>Connect to Peer</h3>
  <div class="form-row">
    <input class="input wide" bind:value={joinCode} placeholder="Paste join code (VM1:… or VM2:…)" />
    <button class="btn btn-primary" on:click={connectPeer} disabled={!$connected}>Connect</button>
  </div>
  {#if connectMsg}<p class={connectOk ? 'result-ok' : 'result-error'}>{connectMsg}</p>{/if}
</div>

<!-- Connected peers ─────────────────────────────────────────────────────── -->
<div class="card">
  <div class="card-header">
    <h3>Connected Peers</h3>
    <button class="btn btn-ghost btn-sm" on:click={() => dispatch('refreshMesh')}>↻</button>
  </div>
  <div class="table-wrap" style="margin-bottom:0">
    <table>
      <thead>
        <tr><th>Name</th><th>Peer ID</th><th>Latency</th><th>Remote Hosts</th><th></th></tr>
      </thead>
      <tbody>
        {#if !$meshInfo?.peers?.length}
          <tr><td colspan="5" class="empty-cell">No connected peers</td></tr>
        {:else}
          {#each $meshInfo.peers as p (p.peer_id)}
            <tr>
              <td>{p.peer_name}</td>
              <td class="mono" title={p.peer_id}>{p.peer_id.slice(0, 14)}…</td>
              <td>{p.latency_ms} ms</td>
              <td>
                {#each (p.remote_hosts || []) as h}
                  <span class="badge badge-remote">{h}</span>
                {/each}
                {#if !p.remote_hosts?.length}<span class="muted">(none)</span>{/if}
              </td>
              <td>
                <button class="btn btn-danger btn-sm" on:click={() => disconnectPeer(p.peer_id)}>Leave</button>
              </td>
            </tr>
          {/each}
        {/if}
      </tbody>
    </table>
  </div>
</div>

<!-- Policy panel ─────────────────────────────────────────────────────────── -->
<div class="card">
  <div class="card-header">
    <h3>Policy Engine</h3>
    <div style="display:flex;gap:6px">
      <button class="btn btn-ghost btn-sm" on:click={() => policyExpanded = !policyExpanded}>
        {policyExpanded ? '▲ Collapse' : '▼ Expand'}
      </button>
      <button class="btn btn-secondary btn-sm" on:click={reloadPolicy} disabled={!$connected}>↺ Reload</button>
    </div>
  </div>

  {#if policyExpanded && $policy}
    <div class="policy-section">
      <div class="policy-subtitle">App Rules</div>
      {#if $policy.rules?.length}
        <table style="font-size:12px">
          <thead><tr><th>App</th><th>Allow</th><th>Deny</th></tr></thead>
          <tbody>
            {#each $policy.rules as r}
              <tr>
                <td class="mono">{r.app}</td>
                <td>{r.allow?.join(', ') || '—'}</td>
                <td>{r.deny?.join(', ')  || '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="muted">No app rules (allow-all).</p>
      {/if}
    </div>

    <div class="policy-section">
      <div class="policy-subtitle">Mesh ACLs</div>
      {#if $policy.mesh_acl?.length}
        <table style="font-size:12px">
          <thead><tr><th>Hostname</th><th>From Peer</th><th>Effect</th></tr></thead>
          <tbody>
            {#each $policy.mesh_acl as a}
              <tr>
                <td class="mono">{a.hostname}</td>
                <td class="mono">{a.from_peer || '*'}</td>
                <td><span class="badge" class:badge-running={a.effect==='allow'} class:badge-stopped={a.effect==='deny'}>{a.effect}</span></td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="muted">No mesh ACL rules.</p>
      {/if}
    </div>
  {:else if policyExpanded && !$policy}
    <p class="muted">Connect to load policy.</p>
  {/if}
</div>

<style>
  .ident-row  { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; }
  .label      { width: 90px; font-size: 12px; color: #8b949e; flex-shrink: 0; }
  .mono-chip  { font-family: monospace; font-size: 12px; background: #0d1117; padding: 3px 8px; border-radius: 4px; border: 1px solid #21262d; }
  .muted      { color: #484f58; font-size: 13px; }
  .badge-remote { background: #1f2e3d; color: #58a6ff; font-size: 11px; padding: 2px 6px; border-radius: 10px; margin-right: 4px; }
  .policy-section   { margin-top: 10px; }
  .policy-subtitle  { font-size: 12px; font-weight: 600; color: #8b949e; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 6px; }
</style>
