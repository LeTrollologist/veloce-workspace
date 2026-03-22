//! hello_node — end-to-end integration smoke test.
//!
//! Requires VeloceCore running in another terminal:
//!   cargo run -p veloce-core -- run
//!
//! Run (from workspace root):
//!   cargo run -p veloce-sdk --example hello_node

use anyhow::Result;
use veloce_sdk::VeloceClient;
use veloce_ipc::message::Capability;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    // ── 1. Connect ────────────────────────────────────────────────────────────
    println!("[1/6] Connecting to VeloceCore…");
    let mut client = VeloceClient::connect(
        "hello_node",
        "0.1.0",
        vec![
            Capability::SpawnNodes,
            Capability::KillNodes,
            Capability::RegistryRead,
            Capability::RegistryWrite,
            Capability::NetRegister,
            Capability::NetResolve,
        ],
    ).await?;
    println!("      client_id = {}", client.client_id);

    // ── 2. Ping ───────────────────────────────────────────────────────────────
    println!("[2/6] Ping…");
    client.ping().await?;
    println!("      Pong ✓");

    // ── 3. Registry write / read ──────────────────────────────────────────────
    println!("[3/6] Registry write…");
    client.registry_set("hello/greeting", b"Hello, VeloceNet!".to_vec()).await?;
    let val = client.registry_get("hello/greeting").await?;
    let text = val.map(|v| String::from_utf8_lossy(&v).to_string())
                  .unwrap_or_else(|| "<not found>".into());
    println!("      registry[hello/greeting] = {:?}", text);
    assert_eq!(text, "Hello, VeloceNet!", "registry roundtrip failed");

    // ── 4. Spawn a node ───────────────────────────────────────────────────────
    println!("[4/6] Spawning node (cmd.exe /c echo hello_node_spawned)…");
    let spawned = client.spawn_node(
        "hello-child",
        "cmd.exe",
        &["/c", "echo hello_node_spawned"],
    ).await?;
    println!("      node_id = {}", spawned.node_id);

    // ── 5. Register on VeloceNet ──────────────────────────────────────────────
    println!("[5/6] Registering hello.vln → port 9000…");
    client.register_host("hello.vln", spawned.node_id, 9000, 300).await?;
    let resolved = client.resolve_host("hello.vln").await?;
    println!("      resolve(hello.vln) = {:?}", resolved);
    assert!(resolved.is_some(), "hostname did not resolve");

    // ── 6. Kill node ──────────────────────────────────────────────────────────
    println!("[6/6] Killing node…");
    let exit_code = client.kill_node(spawned.node_id).await?;
    println!("      exit_code = {}", exit_code);

    println!("\nAll checks passed ✓");
    Ok(())
}
