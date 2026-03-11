/*!
VeloceNetwork Dashboard — Tauri 2 backend.

Holds a single `VeloceClient` in managed state.  Every Tauri command
locks the mutex, calls the SDK, maps errors to `String` for the frontend.
*/

use std::sync::Arc;
use tokio::sync::Mutex;

use serde::Serialize;
use uuid::Uuid;

use veloce_ipc::message::{Body, Capability};
use veloce_sdk::VeloceClient;

// ── Managed state ─────────────────────────────────────────────────────────────

pub struct AppState {
    pub client: Arc<Mutex<Option<VeloceClient>>>,
}

// ── Serialisable response types ───────────────────────────────────────────────

#[derive(Serialize)]
struct NodeRow {
    node_id:    String,
    app_name:   String,
    pid:        u32,
    status:     String,
    spawned_at: String,
}

#[derive(Serialize)]
struct SpawnResult {
    node_id:   String,
    pid:       u32,
    node_pipe: String,
}

// ── Commands ──────────────────────────────────────────────────────────────────
//
// Commands are intentionally not `pub` — only `run()` and the Tauri handler
// macro need them.  Keeping them crate-private avoids macro name conflicts.

#[tauri::command]
async fn connect(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let mut guard = state.client.lock().await;
    if guard.is_some() {
        return Ok("already_connected".into());
    }
    let client = VeloceClient::connect(
        "veloce-dashboard",
        env!("CARGO_PKG_VERSION"),
        vec![
            Capability::SpawnNodes,
            Capability::KillNodes,
            Capability::RegistryRead,
            Capability::RegistryWrite,
            Capability::NetRegister,
            Capability::NetResolve,
        ],
    )
    .await
    .map_err(|e| e.to_string())?;

    *guard = Some(client);
    Ok("connected".into())
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    *state.client.lock().await = None;
    Ok(())
}

#[tauri::command]
async fn ping(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let mut guard = state.client.lock().await;
    match guard.as_mut() {
        None    => Ok(false),
        Some(c) => c.ping().await.map(|_| true).map_err(|_| "ping failed".into()),
    }
}

#[tauri::command]
async fn list_nodes(state: tauri::State<'_, AppState>) -> Result<Vec<NodeRow>, String> {
    let mut guard = state.client.lock().await;
    let c = guard.as_mut().ok_or("Not connected to VeloceCore")?;
    c.list_nodes().await
        .map(|nodes| nodes.into_iter().map(|n| NodeRow {
            node_id:    n.node_id.to_string(),
            app_name:   n.app_name,
            pid:        n.pid,
            status:     format!("{:?}", n.status),
            spawned_at: n.spawned_at.format("%H:%M:%S").to_string(),
        }).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn spawn_node(
    state:      tauri::State<'_, AppState>,
    app_name:   String,
    executable: String,
) -> Result<SpawnResult, String> {
    let mut guard = state.client.lock().await;
    let c = guard.as_mut().ok_or("Not connected to VeloceCore")?;
    c.spawn_node(&app_name, &executable, &[]).await
        .map(|r| SpawnResult {
            node_id:   r.node_id.to_string(),
            pid:       r.pid,
            node_pipe: r.node_pipe,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn kill_node(
    state:   tauri::State<'_, AppState>,
    node_id: String,
) -> Result<u32, String> {
    let id = Uuid::parse_str(&node_id).map_err(|e| e.to_string())?;
    let mut guard = state.client.lock().await;
    let c = guard.as_mut().ok_or("Not connected to VeloceCore")?;
    c.kill_node(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn register_host(
    state:      tauri::State<'_, AppState>,
    hostname:   String,
    node_id:    String,
    local_port: u16,
    ttl_secs:   u64,
) -> Result<(), String> {
    let id = Uuid::parse_str(&node_id).map_err(|e| e.to_string())?;
    let mut guard = state.client.lock().await;
    let c = guard.as_mut().ok_or("Not connected to VeloceCore")?;
    c.register_host(&hostname, id, local_port, ttl_secs).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn unregister_host(
    state:    tauri::State<'_, AppState>,
    hostname: String,
) -> Result<(), String> {
    let mut guard = state.client.lock().await;
    let c = guard.as_mut().ok_or("Not connected to VeloceCore")?;
    c.raw_request(Body::NetUnregisterHost { hostname }).await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ── App entry point ───────────────────────────────────────────────────────────

pub fn run() {
    tauri::Builder::default()
        .manage(AppState { client: Arc::new(Mutex::new(None)) })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            ping,
            list_nodes,
            spawn_node,
            kill_node,
            register_host,
            unregister_host,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VeloceNetwork Dashboard");
}
