/*!
# Embedded Web Status Portal, WebSocket Telemetry & Web Terminal (v3.5)

Zero-dependency embedded dark-mode dashboard, RFC 6455 WebSocket streaming,
live process console, and Mesh KV database management on port `:9090`.
*/

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::metrics::render_prometheus_metrics;
use crate::state::CoreState;

pub const DEFAULT_PORTAL_PORT: u16 = 9090;

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>VeloceNetwork Status & Telemetry Portal</title>
    <style>
        :root {
            --bg: #0d1117;
            --card-bg: #161b22;
            --border: #30363d;
            --text: #c9d1d9;
            --text-muted: #8b949e;
            --accent: #58a6ff;
            --accent-green: #3fb950;
            --accent-orange: #d29922;
            --accent-red: #f85149;
        }
        * { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, monospace; }
        body { background: var(--bg); color: var(--text); padding: 24px; }
        .container { max-width: 1200px; margin: 0 auto; }
        header { display: flex; justify-content: space-between; align-items: center; padding-bottom: 20px; border-bottom: 1px solid var(--border); margin-bottom: 24px; }
        .logo { font-size: 24px; font-weight: 700; color: #fff; display: flex; align-items: center; gap: 10px; }
        .badge { background: #21262d; border: 1px solid var(--border); padding: 4px 10px; border-radius: 20px; font-size: 13px; color: var(--accent); }
        .grid-cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 16px; margin-bottom: 24px; }
        .card { background: var(--card-bg); border: 1px solid var(--border); border-radius: 8px; padding: 16px; }
        .card-title { font-size: 13px; color: var(--text-muted); text-transform: uppercase; margin-bottom: 8px; }
        .card-val { font-size: 28px; font-weight: 700; color: #fff; }
        .section { background: var(--card-bg); border: 1px solid var(--border); border-radius: 8px; padding: 20px; margin-bottom: 24px; }
        .section-title { font-size: 18px; font-weight: 600; margin-bottom: 14px; color: #fff; display: flex; justify-content: space-between; align-items: center; }
        table { width: 100%; border-collapse: collapse; text-align: left; font-size: 14px; }
        th, td { padding: 10px 12px; border-bottom: 1px solid var(--border); }
        th { color: var(--text-muted); font-weight: 600; }
        .pill { display: inline-block; padding: 2px 8px; border-radius: 12px; font-size: 12px; font-weight: 600; }
        .pill-green { background: rgba(63, 185, 80, 0.2); color: var(--accent-green); }
        .pill-blue { background: rgba(88, 166, 255, 0.2); color: var(--accent); }
        .empty { color: var(--text-muted); font-style: italic; padding: 12px 0; }
        .console { background: #05070a; border: 1px solid var(--border); border-radius: 6px; padding: 14px; font-family: monospace; font-size: 13px; color: #7ee787; height: 180px; overflow-y: auto; white-space: pre-wrap; }
        input, button { background: #21262d; border: 1px solid var(--border); color: #fff; padding: 6px 12px; border-radius: 6px; font-size: 13px; }
        button { cursor: pointer; font-weight: 600; }
        button:hover { background: #30363d; }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <div class="logo">⚡ VeloceNetwork <span class="badge" id="vln-ver">v3.5.0</span></div>
            <div style="display:flex; gap:10px; align-items:center;">
                <span id="ws-badge" class="badge" style="color:var(--accent-green)">⚡ WebSocket: Connecting…</span>
                <a href="/metrics" target="_blank" class="badge">Prometheus Metrics</a>
            </div>
        </header>

        <div class="grid-cards">
            <div class="card"><div class="card-title">Live Nodes</div><div class="card-val" id="cnt-nodes">0</div></div>
            <div class="card"><div class="card-title">Mesh Peers</div><div class="card-val" id="cnt-peers">0</div></div>
            <div class="card"><div class="card-title">Ingress Rules</div><div class="card-val" id="cnt-ingress">0</div></div>
            <div class="card"><div class="card-title">HPA Services</div><div class="card-val" id="cnt-hpa">0</div></div>
            <div class="card"><div class="card-title">Cron Tasks</div><div class="card-val" id="cnt-cron">0</div></div>
            <div class="card"><div class="card-title">Mesh KV Keys</div><div class="card-val" id="cnt-kv">0</div></div>
        </div>

        <div class="section">
            <div class="section-title"><span>Active Process Nodes</span></div>
            <table>
                <thead><tr><th>App Name</th><th>Service</th><th>Node ID</th><th>PID</th><th>Health</th></tr></thead>
                <tbody id="tbl-nodes"><tr><td colspan="5" class="empty">No nodes running</td></tr></tbody>
            </table>
        </div>

        <div class="section">
            <div class="section-title"><span>Live Telemetry & Logs Console</span></div>
            <div id="live-console" class="console">[VelocePortal] Connected to local Core stream. Ready.</div>
        </div>

        <div class="section">
            <div class="section-title"><span>Mesh Peers & Topology</span></div>
            <table>
                <thead><tr><th>Peer ID</th><th>Name</th><th>RTT Latency</th><th>TX Bytes</th><th>RX Bytes</th></tr></thead>
                <tbody id="tbl-peers"><tr><td colspan="5" class="empty">No connected peers</td></tr></tbody>
            </table>
        </div>

        <div class="section">
            <div class="section-title">
                <span>Mesh Replicated Database (P2P KV Store)</span>
                <div style="display:flex; gap:8px;">
                    <input id="kv-k" placeholder="Key" style="width:120px;" />
                    <input id="kv-v" placeholder="Value" style="width:160px;" />
                    <button onclick="setKv()">Set Key</button>
                </div>
            </div>
            <table>
                <thead><tr><th>Key</th><th>Value</th><th>Version</th><th>Origin Node</th><th>Action</th></tr></thead>
                <tbody id="tbl-kv"><tr><td colspan="5" class="empty">No mesh KV entries</td></tr></tbody>
            </table>
        </div>

        <div class="section">
            <div class="section-title"><span>Veloce Hub & App Catalog</span> <span class="badge" style="color:var(--accent-green)">1-Click Deploy</span></div>
            <div id="hub-grid" style="display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 16px;">
                <div class="empty">Loading Hub catalog…</div>
            </div>
        </div>

        <div class="section">
            <div class="section-title"><span>Ingress Routes (L7 HTTP/HTTPS)</span></div>
            <table>
                <thead><tr><th>Hostname</th><th>Path Prefix</th><th>Backend Target</th><th>TLS (Port 8443)</th></tr></thead>
                <tbody id="tbl-ingress"><tr><td colspan="4" class="empty">No ingress routes configured</td></tr></tbody>
            </table>
        </div>

        <div class="section">
            <div class="section-title"><span>Scheduled Tasks & CronJobs</span></div>
            <table>
                <thead><tr><th>Task Name</th><th>Schedule</th><th>Concurrency</th><th>Last Status</th><th>Executable</th></tr></thead>
                <tbody id="tbl-cron"><tr><td colspan="5" class="empty">No scheduled tasks</td></tr></tbody>
            </table>
        </div>
    </div>

    <script>
        function log(msg) {
            const c = document.getElementById('live-console');
            const ts = new Date().toLocaleTimeString();
            c.innerText += '\n[' + ts + '] ' + msg;
            c.scrollTop = c.scrollHeight;
        }

        async function deployApp(name) {
            try {
                const res = await fetch('/api/hub/deploy?name=' + encodeURIComponent(name), { method: 'POST' });
                if (res.ok) {
                    log('✓ Deployed application ' + name);
                    refresh();
                } else {
                    const err = await res.text();
                    alert('Deployment failed: ' + err);
                }
            } catch (e) {
                alert('Error deploying: ' + e);
            }
        }

        async function setKv() {
            const k = document.getElementById('kv-k').value.trim();
            const v = document.getElementById('kv-v').value.trim();
            if (!k) return;
            await fetch('/api/mesh/kv?key=' + encodeURIComponent(k) + '&value=' + encodeURIComponent(v), { method: 'POST' });
            log('Set Mesh KV key: ' + k);
            document.getElementById('kv-k').value = '';
            document.getElementById('kv-v').value = '';
            refresh();
        }

        async function delKv(key) {
            await fetch('/api/mesh/kv?key=' + encodeURIComponent(key), { method: 'DELETE' });
            log('Deleted Mesh KV key: ' + key);
            refresh();
        }

        function renderState(d) {
            document.getElementById('vln-ver').innerText = 'v' + d.version;
            document.getElementById('cnt-nodes').innerText = d.nodes.length;
            document.getElementById('cnt-peers').innerText = d.peers.length;
            document.getElementById('cnt-ingress').innerText = d.ingress.length;
            document.getElementById('cnt-hpa').innerText = d.hpa.length;
            document.getElementById('cnt-cron').innerText = d.cron.length;
            document.getElementById('cnt-kv').innerText = (d.mesh_kv || []).length;

            // Nodes
            const nb = document.getElementById('tbl-nodes');
            if (d.nodes.length === 0) nb.innerHTML = '<tr><td colspan="5" class="empty">No nodes running</td></tr>';
            else nb.innerHTML = d.nodes.map(n => `<tr><td><strong>${n.app_name}</strong></td><td>${n.service_name || '-'}</td><td><code>${n.node_id}</code></td><td>${n.pid}</td><td><span class="pill pill-green">${n.health}</span></td></tr>`).join('');

            // Peers
            const pb = document.getElementById('tbl-peers');
            if (d.peers.length === 0) pb.innerHTML = '<tr><td colspan="5" class="empty">No connected peers</td></tr>';
            else pb.innerHTML = d.peers.map(p => `<tr><td><code>${p.peer_id}</code></td><td>${p.peer_name}</td><td><span class="pill pill-blue">${p.latency_ms} ms</span></td><td>${p.tx_bytes} B</td><td>${p.rx_bytes} B</td></tr>`).join('');

            // Mesh KV
            const kb = document.getElementById('tbl-kv');
            const kvs = d.mesh_kv || [];
            if (kvs.length === 0) kb.innerHTML = '<tr><td colspan="5" class="empty">No mesh KV entries</td></tr>';
            else kb.innerHTML = kvs.map(k => `<tr><td><strong>${k.key}</strong></td><td><code>${k.value}</code></td><td>v${k.version}</td><td><code>${k.origin}</code></td><td><button onclick="delKv('${k.key}')" style="color:var(--accent-red); padding:2px 8px;">Delete</button></td></tr>`).join('');

            // Ingress
            const ib = document.getElementById('tbl-ingress');
            if (d.ingress.length === 0) ib.innerHTML = '<tr><td colspan="4" class="empty">No ingress routes configured</td></tr>';
            else {
                let rows = [];
                d.ingress.forEach(r => {
                    const tls = r.tls_enabled ? '<span class="pill pill-green">HTTPS (8443)</span>' : '<span class="pill pill-blue">HTTP (8080)</span>';
                    if (r.paths.length === 0) rows.push(`<tr><td><strong>${r.host}</strong></td><td>/</td><td>127.0.0.1:${r.default_port || 80}</td><td>${tls}</td></tr>`);
                    else r.paths.forEach(p => rows.push(`<tr><td><strong>${r.host}</strong></td><td>${p.path_prefix}</td><td>127.0.0.1:${p.target_port}</td><td>${tls}</td></tr>`));
                });
                ib.innerHTML = rows.join('');
            }

            // Hub Catalog
            const hg = document.getElementById('hub-grid');
            if (d.hub && d.hub.length > 0) {
                hg.innerHTML = d.hub.map(app => `
                    <div class="card" style="display:flex; flex-direction:column; justify-content:space-between; gap:12px;">
                        <div>
                            <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:6px;">
                                <strong style="font-size:16px; color:#fff">${app.name}</strong>
                                <span class="pill pill-blue">${app.category}</span>
                            </div>
                            <p style="font-size:13px; color:var(--text-muted); margin-bottom:8px;">${app.description}</p>
                            <div style="font-size:12px; color:var(--text-muted);">
                                ${app.hostname ? `🌐 <code>${app.hostname}</code>` : ''} 
                                ${app.port ? `• Port <code>${app.port}</code>` : ''}
                            </div>
                        </div>
                        <button onclick="deployApp('${app.name}')" style="background:#238636; color:#fff; border:none; padding:8px 14px; border-radius:6px; font-weight:600; cursor:pointer; width:100%;">🚀 1-Click Deploy</button>
                    </div>
                `).join('');
            }

            // Cron
            const cb = document.getElementById('tbl-cron');
            if (d.cron.length === 0) cb.innerHTML = '<tr><td colspan="5" class="empty">No scheduled tasks</td></tr>';
            else cb.innerHTML = d.cron.map(c => `<tr><td><strong>${c.name}</strong></td><td><code>${c.schedule}</code></td><td>${c.concurrency_policy}</td><td><span class="pill pill-blue">${c.last_run_status || 'Never Run'}</span></td><td><code>${c.executable}</code></td></tr>`).join('');
        }

        async function refresh() {
            try {
                const res = await fetch('/api/status');
                if (res.ok) renderState(await res.json());
            } catch (e) {
                console.error(e);
            }
        }

        // Live WebSocket Telemetry Channel
        function setupWs() {
            const wsProto = location.protocol === 'https:' ? 'wss:' : 'ws:';
            const ws = new WebSocket(`${wsProto}//${location.host}/ws`);
            const badge = document.getElementById('ws-badge');

            ws.onopen = () => {
                badge.innerText = '⚡ WebSocket: Live (Active)';
                badge.style.color = 'var(--accent-green)';
                log('WebSocket telemetry link established.');
            };

            ws.onmessage = (e) => {
                try {
                    const data = JSON.parse(e.data);
                    renderState(data);
                } catch(err) {
                    log(e.data);
                }
            };

            ws.onclose = () => {
                badge.innerText = '⚡ WebSocket: Reconnecting…';
                badge.style.color = 'var(--accent-orange)';
                setTimeout(setupWs, 2000);
            };
        }

        setupWs();
        refresh();
        setInterval(refresh, 5000);
    </script>
</body>
</html>"#;

/// Run the Web Status Portal, WebSocket Telemetry & Metrics HTTP server on `{bind_addr}:{port}`.
pub async fn serve_portal(state: Arc<CoreState>, bind_addr: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind(format!("{bind_addr}:{port}"))
        .await
        .with_context(|| format!("bind Portal HTTP server to {bind_addr}:{port}"))?;

    info!("VeloceNet Web Portal, WebSocket Telemetry & Metrics listening on http://{bind_addr}:{port}");

    loop {
        let (client_stream, client_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("Portal accept error: {e}");
                continue;
            }
        };

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) = handle_portal_client(client_stream, state).await {
                debug!("Portal client ({client_addr}) error: {e}");
            }
        });
    }
}

async fn handle_portal_client(mut client: TcpStream, state: Arc<CoreState>) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = client.read(&mut buf).await.context("read portal request")?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("GET / HTTP/1.1");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    // Check for WebSocket Upgrade
    if req_str.contains("Upgrade: websocket") || req_str.contains("upgrade: websocket") {
        if let Some(sec_key) = extract_header(&req_str, "Sec-WebSocket-Key") {
            let accept_val = compute_ws_accept(sec_key.trim());
            let handshake_resp = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                accept_val
            );
            client.write_all(handshake_resp.as_bytes()).await?;

            // Push state telemetry loop over WebSocket
            loop {
                let status = build_status_json(&state).await;
                let frame = ws_text_frame(&status.to_string());
                if let Err(_) = client.write_all(&frame).await {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
            return Ok(());
        }
    }

    if path == "/" || path == "/index.html" {
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            DASHBOARD_HTML.len(),
            DASHBOARD_HTML
        );
        client.write_all(resp.as_bytes()).await?;
    } else if path == "/metrics" {
        let metrics_text = render_prometheus_metrics(&state).await;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            metrics_text.len(),
            metrics_text
        );
        client.write_all(resp.as_bytes()).await?;
    } else if path == "/api/status" {
        let status_json = build_status_json(&state).await;
        let body = status_json.to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        client.write_all(resp.as_bytes()).await?;
    } else if path == "/api/auth/status" {
        let auth_info = state.oidc().get_auth_info();
        let body = serde_json::to_string(&auth_info).unwrap_or_default();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        client.write_all(resp.as_bytes()).await?;
    } else if path == "/api/auth/logout" {
        let _ = state.oidc().clear_session();
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
        client.write_all(resp.as_bytes()).await?;
    } else if path == "/api/traces" {
        let traces = state.otel.query_traces(Some(50), None);
        let body = serde_json::to_string(&traces).unwrap_or_default();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        client.write_all(resp.as_bytes()).await?;
    } else if path.starts_with("/api/traces/") {
        let trace_id = &path["/api/traces/".len()..];
        let detail = state.otel.get_trace(trace_id);
        let body = serde_json::to_string(&detail).unwrap_or_default();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        client.write_all(resp.as_bytes()).await?;
    } else if path.starts_with("/api/mesh/kv") {
        if method == "POST" {
            let key = extract_query_param(path, "key").unwrap_or_default();
            let value = extract_query_param(path, "value").unwrap_or_default();
            if let Some(mesh) = &state.mesh {
                mesh.kv.set(&key, &value);
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                client.write_all(resp.as_bytes()).await?;
            } else {
                let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 17\r\nConnection: close\r\n\r\nMesh Unavailable";
                client.write_all(resp.as_bytes()).await?;
            }
        } else if method == "DELETE" {
            let key = extract_query_param(path, "key").unwrap_or_default();
            if let Some(mesh) = &state.mesh {
                mesh.kv.delete(&key);
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                client.write_all(resp.as_bytes()).await?;
            } else {
                let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 17\r\nConnection: close\r\n\r\nMesh Unavailable";
                client.write_all(resp.as_bytes()).await?;
            }
        }
    } else if path.starts_with("/api/hub/deploy") {
        let app_name = extract_query_param(path, "name").unwrap_or_default();
        if let Some(app) = state.hub().get(&app_name) {
            let node_id = uuid::Uuid::new_v4();
            let limits = if app.cpu.is_some() || app.mem.is_some() {
                Some(veloce_ipc::message::NodeLimits {
                    cpu_pct: app.cpu.map(|c| c as u32),
                    mem_mb: app.mem,
                    max_lifetime_secs: None,
                })
            } else {
                None
            };

            let spawn_msg = veloce_ipc::message::SpawnNodeMsg {
                app_name: app.name.clone(),
                executable: app.executable.clone(),
                args: app.args.clone(),
                env: app.env.clone(),
                limits,
                auto_kill: false,
                restart_policy: None,
                use_appcontainer: false,
                health_check: None,
                volume_mounts: vec![],
                secret_refs: vec![],
                service_name: Some(app.name.clone()),
                replica_index: Some(0),
            };

            let pipe = format!("veloce-node-{node_id}");
            let slot = state.registry()
                .alloc_node(node_id, &spawn_msg.app_name, &pipe)
                .unwrap_or(0);

            if let (Some(hostname), Some(port)) = (&app.hostname, app.port) {
                state.net_registry().register(hostname.clone(), node_id, port, 0);
            }

            match crate::job::spawn_node(&spawn_msg, node_id, slot, &pipe, None, None).await {
                Ok(handle) => {
                    let pid = handle.pid;
                    let _ = state.registry().set_node_pid(slot, pid);
                    state.node_table().insert(handle);
                    let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                    client.write_all(resp.as_bytes()).await?;
                }
                Err(e) => {
                    let err_msg = format!("Spawn failed: {e}");
                    let resp = format!("HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", err_msg.len(), err_msg);
                    client.write_all(resp.as_bytes()).await?;
                }
            }
        } else {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 17\r\nConnection: close\r\n\r\nApp Not in Hub";
            client.write_all(resp.as_bytes()).await?;
        }
    } else {
        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found";
        client.write_all(resp.as_bytes()).await?;
    }

    Ok(())
}

async fn build_status_json(state: &Arc<CoreState>) -> serde_json::Value {
    let live_nodes = state.node_table().list_live().into_iter().map(|n| {
        serde_json::json!({
            "node_id": n.node_id,
            "app_name": n.app_name,
            "service_name": n.service_name,
            "pid": n.pid,
            "health": format!("{:?}", n.health),
        })
    }).collect::<Vec<_>>();

    let peers = if let Some(mesh) = &state.mesh {
        let map = mesh.peers.read().await;
        map.values().map(|p| {
            let (tx, rx) = p.traffic_snapshot();
            serde_json::json!({
                "peer_id": p.peer_id,
                "peer_name": p.peer_name,
                "latency_ms": p.latency_ms.load(std::sync::atomic::Ordering::Relaxed),
                "tx_bytes": tx,
                "rx_bytes": rx,
            })
        }).collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let mesh_kv = if let Some(mesh) = &state.mesh {
        mesh.kv.list().into_iter().map(|e| {
            serde_json::json!({
                "key": e.key,
                "value": e.value,
                "version": e.version,
                "origin": e.origin.to_string(),
            })
        }).collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let ingress = state.ingress_router().list_rules().await;
    let hpa = state.autoscale().list_policies().into_iter().map(|p| p.to_msg()).collect::<Vec<_>>();
    let cron = state.cron().list_jobs().into_iter().map(|c| c.to_msg()).collect::<Vec<_>>();
    let hub = state.hub().list();
    let auth = state.oidc().get_auth_info();

    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "nodes": live_nodes,
        "peers": peers,
        "mesh_kv": mesh_kv,
        "ingress": ingress,
        "hpa": hpa,
        "cron": cron,
        "hub": hub,
        "auth": auth,
    })
}

fn extract_header<'a>(req: &'a str, name: &str) -> Option<&'a str> {
    for line in req.lines() {
        if let Some(idx) = line.find(':') {
            let key = &line[..idx].trim();
            if key.eq_ignore_ascii_case(name) {
                return Some(&line[idx + 1..]);
            }
        }
    }
    None
}

fn extract_query_param(path: &str, param_name: &str) -> Option<String> {
    let prefix = format!("{}=", param_name);
    path.split('?').nth(1)?
        .split('&')
        .find(|param| param.starts_with(&prefix))
        .and_then(|param| param.strip_prefix(&prefix))
        .map(|val| val.to_string())
}

fn compute_ws_accept(sec_key: &str) -> String {
    let mut combined = sec_key.as_bytes().to_vec();
    combined.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let digest = sha1_digest(&combined);
    base64_encode(&digest)
}

fn ws_text_frame(text: &str) -> Vec<u8> {
    let payload = text.as_bytes();
    let len = payload.len();
    let mut frame = Vec::with_capacity(len + 10);
    frame.push(0x81); // FIN + Text
    if len <= 125 {
        frame.push(len as u8);
    } else if len <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    frame
}

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let mut msg = data.to_vec();
    let bit_len = (msg.len() as u64) * 8;
    msg.push(0x80);
    while (msg.len() + 8) % 64 != 0 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..80 {
            w[i] = (w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        result.push(CHARS[(b0 >> 2) as usize] as char);
        result.push(CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(b2 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
