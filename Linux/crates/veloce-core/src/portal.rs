/*!
# Embedded Web Status Portal & Metrics Server for VeloceCore (v3.3)

Serves an embedded zero-dependency responsive dark-mode cluster management portal,
JSON status API (`/api/status`), and Prometheus metrics exposition (`/metrics`) on `:9090`.
*/

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::state::CoreState;
use crate::metrics::render_prometheus_metrics;

pub const DEFAULT_PORTAL_PORT: u16 = 9090;

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>VeloceNetwork Status Portal</title>
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
        .grid-cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin-bottom: 24px; }
        .card { background: var(--card-bg); border: 1px solid var(--border); border-radius: 8px; padding: 16px; }
        .card-title { font-size: 13px; color: var(--text-muted); text-transform: uppercase; margin-bottom: 8px; }
        .card-val { font-size: 28px; font-weight: 700; color: #fff; }
        .section { background: var(--card-bg); border: 1px solid var(--border); border-radius: 8px; padding: 20px; margin-bottom: 24px; }
        .section-title { font-size: 18px; font-weight: 600; margin-bottom: 14px; color: #fff; display: flex; justify-content: space-between; }
        table { width: 100%; border-collapse: collapse; text-align: left; font-size: 14px; }
        th, td { padding: 10px 12px; border-bottom: 1px solid var(--border); }
        th { color: var(--text-muted); font-weight: 600; }
        .pill { display: inline-block; padding: 2px 8px; border-radius: 12px; font-size: 12px; font-weight: 600; }
        .pill-green { background: rgba(63, 185, 80, 0.2); color: var(--accent-green); }
        .pill-blue { background: rgba(88, 166, 255, 0.2); color: var(--accent); }
        .empty { color: var(--text-muted); font-style: italic; padding: 12px 0; }
        a { color: var(--accent); text-decoration: none; }
        a:hover { text-decoration: underline; }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <div class="logo">⚡ VeloceNetwork <span class="badge" id="vln-ver">v3.3.0</span></div>
            <div>
                <a href="/metrics" target="_blank" class="badge">📊 /metrics</a>
                <span class="badge" style="color: var(--accent-green);">● Core Online</span>
            </div>
        </header>

        <div class="grid-cards">
            <div class="card"><div class="card-title">Live Nodes</div><div class="card-val" id="cnt-nodes">0</div></div>
            <div class="card"><div class="card-title">Mesh Peers</div><div class="card-val" id="cnt-peers">0</div></div>
            <div class="card"><div class="card-title">Ingress Routes</div><div class="card-val" id="cnt-ingress">0</div></div>
            <div class="card"><div class="card-title">HPA Policies</div><div class="card-val" id="cnt-hpa">0</div></div>
            <div class="card"><div class="card-title">Cron Tasks</div><div class="card-val" id="cnt-cron">0</div></div>
        </div>

        <div class="section">
            <div class="section-title"><span>Supervised Nodes</span></div>
            <table>
                <thead><tr><th>App Name</th><th>Service</th><th>Node ID</th><th>PID</th><th>Health</th></tr></thead>
                <tbody id="tbl-nodes"><tr><td colspan="5" class="empty">No nodes running</td></tr></tbody>
            </table>
        </div>

        <div class="section">
            <div class="section-title"><span>Mesh Peers & Topology</span></div>
            <table>
                <thead><tr><th>Peer ID</th><th>Name</th><th>RTT Latency</th><th>TX Bytes</th><th>RX Bytes</th></tr></thead>
                <tbody id="tbl-peers"><tr><td colspan="5" class="empty">No connected peers</td></tr></tbody>
            </table>
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
        async function refresh() {
            try {
                const res = await fetch('/api/status');
                if (!res.ok) return;
                const d = await res.json();

                document.getElementById('vln-ver').innerText = 'v' + d.version;
                document.getElementById('cnt-nodes').innerText = d.nodes.length;
                document.getElementById('cnt-peers').innerText = d.peers.length;
                document.getElementById('cnt-ingress').innerText = d.ingress.length;
                document.getElementById('cnt-hpa').innerText = d.hpa.length;
                document.getElementById('cnt-cron').innerText = d.cron.length;

                // Nodes
                const nb = document.getElementById('tbl-nodes');
                if (d.nodes.length === 0) nb.innerHTML = '<tr><td colspan="5" class="empty">No nodes running</td></tr>';
                else nb.innerHTML = d.nodes.map(n => `<tr><td><strong>${n.app_name}</strong></td><td>${n.service_name || '-'}</td><td><code>${n.node_id}</code></td><td>${n.pid}</td><td><span class="pill pill-green">${n.health}</span></td></tr>`).join('');

                // Peers
                const pb = document.getElementById('tbl-peers');
                if (d.peers.length === 0) pb.innerHTML = '<tr><td colspan="5" class="empty">No connected peers</td></tr>';
                else pb.innerHTML = d.peers.map(p => `<tr><td><code>${p.peer_id}</code></td><td>${p.peer_name}</td><td><span class="pill pill-blue">${p.latency_ms} ms</span></td><td>${p.tx_bytes} B</td><td>${p.rx_bytes} B</td></tr>`).join('');

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

                // Cron
                const cb = document.getElementById('tbl-cron');
                if (d.cron.length === 0) cb.innerHTML = '<tr><td colspan="5" class="empty">No scheduled tasks</td></tr>';
                else cb.innerHTML = d.cron.map(c => `<tr><td><strong>${c.name}</strong></td><td><code>${c.schedule}</code></td><td>${c.concurrency_policy}</td><td><span class="pill pill-blue">${c.last_run_status || 'Never Run'}</span></td><td><code>${c.executable}</code></td></tr>`).join('');
            } catch (e) {
                console.error(e);
            }
        }
        setInterval(refresh, 3000);
        refresh();
    </script>
</body>
</html>"#;

/// Run the Web Status Portal & Prometheus Metrics HTTP server on `{bind_addr}:{port}`.
pub async fn serve_portal(state: Arc<CoreState>, bind_addr: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind(format!("{bind_addr}:{port}"))
        .await
        .with_context(|| format!("bind Portal HTTP server to {bind_addr}:{port}"))?;

    info!("VeloceNet Web Portal & Metrics server listening on http://{bind_addr}:{port}");

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
    let mut buf = [0u8; 2048];
    let n = client.read(&mut buf).await.context("read portal request")?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("GET / HTTP/1.1");
    let mut parts = first_line.split_whitespace();
    let _method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    match path {
        "/" | "/index.html" => {
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                DASHBOARD_HTML.len(),
                DASHBOARD_HTML
            );
            client.write_all(resp.as_bytes()).await?;
        }
        "/metrics" => {
            let metrics_text = render_prometheus_metrics(&state).await;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                metrics_text.len(),
                metrics_text
            );
            client.write_all(resp.as_bytes()).await?;
        }
        "/api/status" => {
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

            let ingress = state.ingress_router().list_rules().await;
            let hpa = state.autoscale().list_policies().into_iter().map(|p| p.to_msg()).collect::<Vec<_>>();
            let cron = state.cron().list_jobs().into_iter().map(|c| c.to_msg()).collect::<Vec<_>>();

            let status_json = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "nodes": live_nodes,
                "peers": peers,
                "ingress": ingress,
                "hpa": hpa,
                "cron": cron,
            });

            let body = status_json.to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            client.write_all(resp.as_bytes()).await?;
        }
        _ => {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found";
            client.write_all(resp.as_bytes()).await?;
        }
    }

    Ok(())
}
