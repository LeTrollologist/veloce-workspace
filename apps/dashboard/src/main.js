import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ── State ──────────────────────────────────────────────────────────────────

let connected = false;
let selectedLogNodeId = null;      // node currently shown in log viewer
const logLines = {};               // node_id → array of {stream, text, ts}
const MAX_LOG_LINES = 2000;        // per node

// ── UI helpers ─────────────────────────────────────────────────────────────

function setStatus(isConnected) {
  connected = isConnected;
  const dot   = document.getElementById("status-dot");
  const label = document.getElementById("status-label");
  const btn   = document.getElementById("btn-connect");
  dot.className   = "dot " + (isConnected ? "connected" : "disconnected");
  label.textContent = isConnected ? "Connected" : "Disconnected";
  btn.textContent   = isConnected ? "Disconnect" : "Connect";

  document.getElementById("btn-refresh").disabled = !isConnected;
  document.getElementById("btn-spawn").disabled    = !isConnected;
  document.getElementById("btn-register").disabled = !isConnected;
  document.getElementById("btn-unregister").disabled = !isConnected;
}

function showResult(id, text, isOk) {
  const el = document.getElementById(id);
  el.textContent = text;
  el.className   = "result " + (isOk ? "ok" : "error");
}

function hideResult(id) {
  document.getElementById(id).className = "result hidden";
}

function statusBadge(status) {
  const s = status.toLowerCase();
  const cls = s.includes("running") ? "running"
            : s.includes("start")   ? "starting"
            : "stopped";
  return `<span class="badge badge-${cls}">${status}</span>`;
}

// ── Node table ─────────────────────────────────────────────────────────────

function renderNodes(nodes) {
  const tbody = document.getElementById("node-list");
  const sel   = document.getElementById("log-node-select");

  // Preserve current log selection
  const prevSel = sel.value;

  // Rebuild log node dropdown
  sel.innerHTML = `<option value="">— select a node —</option>`;
  if (!nodes || nodes.length === 0) {
    tbody.innerHTML = `<tr id="node-empty-row">
      <td colspan="6" class="empty-cell">No nodes running</td>
    </tr>`;
    return;
  }

  tbody.innerHTML = nodes.map(n => `
    <tr>
      <td><strong>${escHtml(n.app_name)}</strong></td>
      <td>${n.pid}</td>
      <td>${statusBadge(n.status)}</td>
      <td>${escHtml(n.spawned_at)}</td>
      <td class="mono">${escHtml(n.node_id.slice(0, 8))}…</td>
      <td>
        <button class="btn btn-danger btn-sm" data-kill="${escHtml(n.node_id)}">Kill</button>
      </td>
    </tr>
  `).join("");

  nodes.forEach(n => {
    const opt = document.createElement("option");
    opt.value = n.node_id;
    opt.textContent = `${n.app_name} (${n.node_id.slice(0, 8)}…)`;
    sel.appendChild(opt);
  });

  // Restore selection
  if (prevSel) sel.value = prevSel;

  tbody.querySelectorAll("[data-kill]").forEach(btn => {
    btn.addEventListener("click", () => killNode(btn.dataset.kill));
  });
}

function escHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// ── Log viewer ─────────────────────────────────────────────────────────────

function appendLog(nodeId, stream, text) {
  if (!logLines[nodeId]) logLines[nodeId] = [];
  const lines = logLines[nodeId];

  // Split on newlines but keep partial lines
  const parts = text.split("\n");
  for (const part of parts) {
    if (part === "") continue;
    lines.push({ stream, text: part, ts: new Date().toLocaleTimeString("en-US", { hour12: false }) });
  }

  // Cap buffer
  if (lines.length > MAX_LOG_LINES) {
    lines.splice(0, lines.length - MAX_LOG_LINES);
  }

  if (nodeId === selectedLogNodeId) {
    renderLogViewer();
  }
}

function renderLogViewer() {
  const viewer = document.getElementById("log-viewer");
  const empty  = document.getElementById("log-empty");

  if (!selectedLogNodeId || !logLines[selectedLogNodeId]?.length) {
    viewer.innerHTML = `<div class="empty-cell" id="log-empty">
      ${selectedLogNodeId ? "No output yet" : "Select a node to view its output"}
    </div>`;
    return;
  }

  const wasAtBottom = viewer.scrollHeight - viewer.scrollTop <= viewer.clientHeight + 40;

  viewer.innerHTML = logLines[selectedLogNodeId]
    .map(l => `<div class="log-line ${escHtml(l.stream)}"><span class="ts">${escHtml(l.ts)}</span>${escHtml(l.text)}</div>`)
    .join("");

  // Auto-scroll if already at bottom
  if (wasAtBottom) {
    viewer.scrollTop = viewer.scrollHeight;
  }
}

// ── Commands ───────────────────────────────────────────────────────────────

async function toggleConnect() {
  if (connected) {
    await invoke("disconnect");
    setStatus(false);
    renderNodes([]);
  } else {
    try {
      await invoke("connect");
      setStatus(true);
      refreshNodes();
      refreshTemplates();
    } catch (e) {
      alert("Connect failed:\n" + e);
    }
  }
}

async function refreshNodes() {
  if (!connected) return;
  try {
    const nodes = await invoke("list_nodes");
    renderNodes(nodes);
  } catch (e) {
    renderNodes([]);
    console.error("list_nodes:", e);
  }
}

async function spawnNode() {
  const name = document.getElementById("spawn-name").value.trim();
  const exe  = document.getElementById("spawn-exe").value.trim();
  if (!name || !exe) { showResult("spawn-result", "App name and executable are required.", false); return; }
  hideResult("spawn-result");
  try {
    const r = await invoke("spawn_node", { appName: name, executable: exe });
    showResult("spawn-result", `Spawned — PID ${r.pid}  Node ${r.node_id}`, true);
    refreshNodes();
  } catch (e) {
    showResult("spawn-result", "Error: " + e, false);
  }
}

async function killNode(nodeId) {
  try {
    await invoke("kill_node", { nodeId });
    refreshNodes();
  } catch (e) {
    alert("Kill failed:\n" + e);
  }
}

async function registerHost() {
  const hostname  = document.getElementById("net-hostname").value.trim();
  const nodeId    = document.getElementById("net-node-id").value.trim();
  const localPort = parseInt(document.getElementById("net-port").value) || 0;
  const ttlSecs   = parseInt(document.getElementById("net-ttl").value)  || 0;
  if (!hostname || !nodeId) { showResult("reg-result", "Hostname and Node UUID are required.", false); return; }
  try {
    await invoke("register_host", { hostname, nodeId, localPort, ttlSecs });
    showResult("reg-result", `${hostname} registered.`, true);
  } catch (e) {
    showResult("reg-result", "Error: " + e, false);
  }
}

async function unregisterHost() {
  const hostname = document.getElementById("unreg-hostname").value.trim();
  if (!hostname) { showResult("unreg-result", "Hostname is required.", false); return; }
  try {
    await invoke("unregister_host", { hostname });
    showResult("unreg-result", `${hostname} unregistered.`, true);
  } catch (e) {
    showResult("unreg-result", "Error: " + e, false);
  }
}

// ── Templates ──────────────────────────────────────────────────────────────

function renderTemplates(templates) {
  const tbody = document.getElementById("template-list");
  if (!templates || templates.length === 0) {
    tbody.innerHTML = `<tr id="template-empty-row">
      <td colspan="7" class="empty-cell">No templates saved</td>
    </tr>`;
    return;
  }
  tbody.innerHTML = templates.map(t => `
    <tr data-tmpl-name="${escHtml(t.name)}">
      <td><strong>${escHtml(t.name)}</strong></td>
      <td>${escHtml(t.app_name)}</td>
      <td class="mono">${escHtml(t.executable)}</td>
      <td>${t.cpu_pct != null ? t.cpu_pct + "%" : "—"}</td>
      <td>${t.mem_mb  != null ? t.mem_mb  + " MB" : "—"}</td>
      <td>${t.max_restarts != null ? t.max_restarts : "—"}</td>
      <td style="display:flex;gap:4px">
        <button class="btn-spawn-tmpl" data-spawn-tmpl="${escHtml(t.name)}" ${connected ? "" : "disabled"}>▶ Spawn</button>
        <button class="btn btn-danger btn-sm" data-del-tmpl="${escHtml(t.name)}">✕</button>
      </td>
    </tr>
  `).join("");

  tbody.querySelectorAll("[data-spawn-tmpl]").forEach(btn => {
    btn.addEventListener("click", () => spawnFromTemplate(btn.dataset.spawnTmpl));
  });
  tbody.querySelectorAll("[data-del-tmpl]").forEach(btn => {
    btn.addEventListener("click", () => deleteTemplate(btn.dataset.delTmpl));
  });
}

async function refreshTemplates() {
  if (!connected) return;
  try {
    const templates = await invoke("list_templates");
    // list_templates returns names; fetch each to get full details
    const full = await Promise.all(
      templates.map(name => invoke("get_template", { name }))
    );
    renderTemplates(full.filter(Boolean));
  } catch (e) {
    console.error("list_templates:", e);
    renderTemplates([]);
  }
}

async function saveTemplate() {
  const name     = document.getElementById("tmpl-name").value.trim();
  const appName  = document.getElementById("tmpl-app").value.trim();
  const exe      = document.getElementById("tmpl-exe").value.trim();
  const argsRaw  = document.getElementById("tmpl-args").value.trim();
  const cpuRaw   = document.getElementById("tmpl-cpu").value.trim();
  const memRaw   = document.getElementById("tmpl-mem").value.trim();
  const rstRaw   = document.getElementById("tmpl-restarts").value.trim();

  if (!name || !appName || !exe) {
    showResult("tmpl-result", "Template name, app name, and executable are required.", false);
    return;
  }
  hideResult("tmpl-result");

  const spec = {
    name,
    app_name:     appName,
    executable:   exe,
    args:         argsRaw ? argsRaw.split(/\s+/) : [],
    cpu_pct:      cpuRaw   ? parseInt(cpuRaw)   : null,
    mem_mb:       memRaw   ? parseInt(memRaw)   : null,
    lifetime_secs: null,
    max_restarts: rstRaw   ? parseInt(rstRaw)   : null,
  };

  try {
    await invoke("save_template", { spec });
    showResult("tmpl-result", `Template "${name}" saved.`, true);
    // Clear form
    ["tmpl-name","tmpl-app","tmpl-exe","tmpl-args","tmpl-cpu","tmpl-mem","tmpl-restarts"]
      .forEach(id => { document.getElementById(id).value = ""; });
    refreshTemplates();
  } catch (e) {
    showResult("tmpl-result", "Error: " + e, false);
  }
}

async function spawnFromTemplate(name) {
  try {
    const r = await invoke("spawn_from_template", { name });
    showResult("tmpl-result", `Spawned from "${name}" — PID ${r.pid}`, true);
    refreshNodes();
  } catch (e) {
    showResult("tmpl-result", "Spawn error: " + e, false);
  }
}

async function deleteTemplate(name) {
  if (!confirm(`Delete template "${name}"?`)) return;
  try {
    await invoke("delete_template", { name });
    refreshTemplates();
  } catch (e) {
    alert("Delete failed:\n" + e);
  }
}

// ── Tab switching ──────────────────────────────────────────────────────────

function initTabs() {
  document.querySelectorAll(".tab-btn").forEach(btn => {
    btn.addEventListener("click", () => {
      document.querySelectorAll(".tab-btn").forEach(b => b.classList.remove("active"));
      document.querySelectorAll(".tab-panel").forEach(p => p.classList.remove("active"));
      btn.classList.add("active");
      document.getElementById("tab-" + btn.dataset.tab).classList.add("active");
    });
  });
}

// ── Init ───────────────────────────────────────────────────────────────────

window.addEventListener("DOMContentLoaded", async () => {
  initTabs();
  setStatus(false);

  document.getElementById("btn-connect").addEventListener("click",            toggleConnect);
  document.getElementById("btn-refresh").addEventListener("click",            refreshNodes);
  document.getElementById("btn-spawn").addEventListener("click",              spawnNode);
  document.getElementById("btn-register").addEventListener("click",           registerHost);
  document.getElementById("btn-unregister").addEventListener("click",         unregisterHost);
  document.getElementById("btn-refresh-templates").addEventListener("click",  refreshTemplates);
  document.getElementById("btn-save-template").addEventListener("click",      saveTemplate);

  // Log node selector
  document.getElementById("log-node-select").addEventListener("change", async (e) => {
    const nodeId = e.target.value;
    selectedLogNodeId = nodeId || null;
    if (nodeId && connected) {
      try {
        await invoke("subscribe_node_logs", { nodeId });
      } catch (err) {
        console.warn("subscribe_node_logs:", err);
      }
    }
    renderLogViewer();
  });

  // Clear log button
  document.getElementById("btn-clear-log").addEventListener("click", () => {
    if (selectedLogNodeId) {
      logLines[selectedLogNodeId] = [];
      renderLogViewer();
    }
  });

  // Listen for log chunks pushed from the Tauri backend
  await listen("node-log", (event) => {
    const { node_id, stream, text } = event.payload;
    appendLog(node_id, stream, text);
  });

  // Listen for node lifecycle events — refresh table on crash/restart
  await listen("node-event", (event) => {
    const { node_id, event: evName } = event.payload;
    if (evName.startsWith("Restarting") || evName.startsWith("Exited")) {
      refreshNodes();
    }
  });

  // Auto-ping every 10 s to keep status indicator accurate
  setInterval(async () => {
    if (!connected) return;
    try {
      const ok = await invoke("ping");
      if (!ok) setStatus(false);
    } catch { setStatus(false); }
  }, 10_000);
});
