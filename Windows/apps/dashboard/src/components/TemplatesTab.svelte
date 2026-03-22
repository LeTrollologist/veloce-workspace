<script>
  import { createEventDispatcher } from 'svelte';
  import { templates, connected } from '../stores.js';
  import { api } from '../lib/tauri.js';

  const dispatch = createEventDispatcher();

  let name     = '';
  let appName  = '';
  let exe      = '';
  let args     = '';
  let cpuPct   = '';
  let memMb    = '';
  let restarts = '';
  let sandbox  = false;
  let msg      = '';
  let msgOk    = false;

  function clearForm() {
    name = appName = exe = args = cpuPct = memMb = restarts = '';
    sandbox = false;
  }

  async function save() {
    if (!name.trim() || !appName.trim() || !exe.trim()) {
      msg = 'Template name, app name, and executable are required.';
      msgOk = false;
      return;
    }
    try {
      const spec = {
        name:             name.trim(),
        app_name:         appName.trim(),
        executable:       exe.trim(),
        args:             args.trim() ? args.trim().split(/\s+/) : [],
        cpu_pct:          cpuPct   ? parseInt(cpuPct)   : null,
        mem_mb:           memMb    ? parseInt(memMb)    : null,
        lifetime_secs:    null,
        max_restarts:     restarts ? parseInt(restarts) : null,
        use_appcontainer: sandbox,
      };
      await api.saveTemplate(spec);
      msg   = `Template "${name}" saved.`;
      msgOk = true;
      clearForm();
      dispatch('refresh');
    } catch (e) {
      msg   = 'Error: ' + e;
      msgOk = false;
    }
  }

  async function del(tmplName) {
    if (!confirm(`Delete template "${tmplName}"?`)) return;
    try { await api.deleteTemplate(tmplName); dispatch('refresh'); }
    catch (e) { alert('Delete failed:\n' + e); }
  }

  async function spawn(tmplName) {
    try {
      const r = await api.spawnFromTemplate(tmplName);
      msg   = `Spawned from "${tmplName}" — PID ${r.pid}`;
      msgOk = true;
    } catch (e) {
      msg   = 'Spawn error: ' + e;
      msgOk = false;
    }
  }
</script>

<div class="panel-row">
  <h2>Node Templates</h2>
  <button class="btn btn-ghost" on:click={() => dispatch('refresh')}>↻ Refresh</button>
</div>

<div class="table-wrap">
  <table>
    <thead>
      <tr>
        <th>Name</th><th>App Name</th><th>Executable</th>
        <th>CPU%</th><th>RAM MB</th><th>Restarts</th><th>Sandbox</th><th></th>
      </tr>
    </thead>
    <tbody>
      {#if $templates.length === 0}
        <tr><td colspan="8" class="empty-cell">No templates saved</td></tr>
      {:else}
        {#each $templates as t (t.name)}
          <tr>
            <td><strong>{t.name}</strong></td>
            <td>{t.app_name}</td>
            <td class="mono">{t.executable}</td>
            <td>{t.cpu_pct != null ? t.cpu_pct + '%' : '—'}</td>
            <td>{t.mem_mb  != null ? t.mem_mb + ' MB' : '—'}</td>
            <td>{t.max_restarts != null ? t.max_restarts : '—'}</td>
            <td>{t.use_appcontainer ? '🛡 AC' : '—'}</td>
            <td style="display:flex;gap:4px">
              <button class="btn btn-secondary btn-sm" on:click={() => spawn(t.name)} disabled={!$connected}>▶</button>
              <button class="btn btn-danger btn-sm"    on:click={() => del(t.name)}>✕</button>
            </td>
          </tr>
        {/each}
      {/if}
    </tbody>
  </table>
</div>

<div class="card">
  <h3>Save Template</h3>
  <div class="form-row">
    <input class="input" bind:value={name}    placeholder="Template name" />
    <input class="input" bind:value={appName} placeholder="App name" />
    <input class="input wide" bind:value={exe} placeholder="C:\path\to\app.exe" />
  </div>
  <div class="form-row">
    <input class="input wide" bind:value={args}     placeholder="Args (space-separated, optional)" />
    <input class="input narrow" type="number" bind:value={cpuPct}   placeholder="CPU%" min="1" max="100" />
    <input class="input narrow" type="number" bind:value={memMb}    placeholder="RAM MB" min="1" />
    <input class="input narrow" type="number" bind:value={restarts} placeholder="Restarts" min="0" />
    <label class="checkbox-label">
      <input type="checkbox" bind:checked={sandbox} />
      AppContainer
    </label>
    <button class="btn btn-primary" on:click={save} disabled={!$connected}>Save</button>
  </div>
  {#if msg}
    <p class={msgOk ? 'result-ok' : 'result-error'}>{msg}</p>
  {/if}
</div>

<style>
  .checkbox-label { display: flex; align-items: center; gap: 6px; font-size: 13px; cursor: pointer; }
</style>
