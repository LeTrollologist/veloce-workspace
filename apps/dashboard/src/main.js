import { invoke } from "@tauri-apps/api/core";

// ── State ──────────────────────────────────────────────────────────────────

let connected = false;

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

window.addEventListener("DOMContentLoaded", () => {
  initTabs();
  setStatus(false);

  document.getElementById("btn-connect").addEventListener("click",      toggleConnect);
  document.getElementById("btn-refresh").addEventListener("click",      refreshNodes);
  document.getElementById("btn-spawn").addEventListener("click",        spawnNode);
  document.getElementById("btn-register").addEventListener("click",     registerHost);
  document.getElementById("btn-unregister").addEventListener("click",   unregisterHost);

  // Auto-ping every 10 s to keep status indicator accurate
  setInterval(async () => {
    if (!connected) return;
    try {
      const ok = await invoke("ping");
      if (!ok) setStatus(false);
    } catch { setStatus(false); }
  }, 10_000);
});
