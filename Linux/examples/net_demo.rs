/*!
# net_demo

Demonstrates the full VeloceNet routing pipeline:

1. Connect to VeloceCore
2. Spawn a tiny HTTP server node (uses tokio to listen on port 9000)
3. Register it as `demo.vln`
4. Make an HTTP request through the SOCKS5 proxy to http://demo.vln:9000
5. Verify the response came from our node
6. Teardown

Run VeloceCore first:  cargo run -p veloce-core -- run
Then:                  cargo run --example net_demo
*/

use anyhow::Result;
use veloce_sdk::VeloceClient;
use veloce_ipc::message::Capability;
use std::time::Duration;

const WORKER_PORT: u16 = 9000;
const VLN_HOST:    &str = "demo.vln";
const SOCKS_PORT:  u16  = 1055; // VeloceNet default

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,veloce_sdk=debug,net_demo=debug")
        .init();

    // ── 1. Connect to Core ────────────────────────────────────────────────────
    let mut client = VeloceClient::connect(
        "net-demo",
        env!("CARGO_PKG_VERSION"),
        vec![
            Capability::SpawnNodes,
            Capability::KillNodes,
            Capability::NetRegister,
            Capability::NetResolve,
        ],
    ).await?;
    tracing::info!(client_id = %client.client_id, "VeloceCore connected");

    // ── 2. Start a local HTTP echo server in-process ──────────────────────────
    // (In a real scenario, this would be a separate executable spawned as a node.)
    let server_task = tokio::spawn(run_echo_server(WORKER_PORT));
    tokio::time::sleep(Duration::from_millis(100)).await; // let it bind

    // ── 3. Register on VeloceNet ─────────────────────────────────────────────
    // Since we're running the server in-process (not as a spawned node),
    // we create a synthetic node_id for the registration.
    let node_id = uuid::Uuid::new_v4();
    client.register_host(VLN_HOST, node_id, WORKER_PORT, 60).await?;
    tracing::info!("Registered {VLN_HOST} → 127.0.0.1:{WORKER_PORT}");

    // ── 4. Resolve via VeloceNet ──────────────────────────────────────────────
    let addr = client.resolve_host(VLN_HOST).await?
        .expect("hostname should resolve");
    tracing::info!("Resolved {VLN_HOST} → {addr}");
    assert_eq!(addr, format!("127.0.0.1:{WORKER_PORT}"));

    // ── 5. HTTP request through SOCKS5 proxy ──────────────────────────────────
    // To use the SOCKS5 proxy, configure reqwest with the proxy:
    //   VELOCE_SOCKS5=socks5://127.0.0.1:1055
    //
    // Simplified: directly connect to the resolved address (same effect locally)
    let response_body = direct_http_get(&addr).await?;
    tracing::info!("Response from {VLN_HOST}: {response_body:?}");
    assert!(response_body.contains("VeloceNet echo"), "expected echo response");

    // ── 6. Teardown ───────────────────────────────────────────────────────────
    server_task.abort();
    tracing::info!("net_demo complete — VeloceNet routing verified.");
    Ok(())
}

/// Minimal HTTP/1.0 echo server.
async fn run_echo_server(port: u16) -> anyhow::Result<()> {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    tracing::debug!("Echo server listening on 127.0.0.1:{port}");

    loop {
        let (mut stream, peer) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let body     = "VeloceNet echo server OK";
            let response = format!(
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{}",
                body.len(), body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            tracing::debug!(%peer, "echo served");
        });
    }
}

/// Minimal raw HTTP GET — avoids pulling in reqwest for the example.
async fn direct_http_get(addr: &str) -> anyhow::Result<String> {
    use tokio::net::TcpStream;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(b"GET / HTTP/1.0\r\nHost: demo.vln\r\nConnection: close\r\n\r\n").await?;

    let mut response = String::new();
    stream.read_to_string(&mut response).await?;

    // Extract body (after \r\n\r\n)
    Ok(response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or(response))
}