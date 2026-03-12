/*!
VeloceNetwork Dashboard — Tauri 2 backend.

Holds a single `VeloceClient` in managed state.  Every Tauri command
locks the mutex, calls the SDK, maps errors to `String` for the frontend.

Log chunks and node events are pushed to the frontend via Tauri window events:
  - `"node-log"`   — `{ node_id, stream, text }` for captured stdout/stderr
  - `"node-event"` — `{ node_id, event }` for lifecycle events (crash, restart…)
*/

use std::sync::Arc;
use tokio::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tauri::Emitter;
use veloce_ipc::message::{Body, Capability, NodeLimits, RestartPolicy, SpawnNodeMsg};
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

#[derive(Serialize)]
struct ResourceRow {
    node_id:   String,
    /// Cumulative CPU time (kernel + user) in milliseconds.
    cpu_ms:    u64,
    /// Peak memory used by the job in bytes.
    mem_bytes: u64,
}

#[derive(Serialize, Clone)]
struct LogChunkEvent {
    node_id: String,
    stream:  String,
    text:    String,
}

#[derive(Serialize, Clone)]
struct NodeEventPayload {
    node_id: String,
    event:   String,
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn connect(
    app:   tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
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

    // Forward log chunks → "node-log" Tauri events
    let mut log_stream = client.subscribe_all_logs();
    let app_log = app.clone();
    tokio::spawn(async move {
        while let Some(chunk) = log_stream.next().await {
            let stream_str = match chunk.stream {
                veloce_ipc::message::LogStream::Stdout => "stdout",
                veloce_ipc::message::LogStream::Stderr => "stderr",
            };
            let _ = app_log.emit("node-log", LogChunkEvent {
                node_id: chunk.node_id.to_string(),
                stream:  stream_str.into(),
                text:    String::from_utf8_lossy(&chunk.data).into_owned(),
            });
        }
    });

    // Forward node lifecycle events → "node-event" Tauri events
    let mut ev_stream = client.subscribe_all_events();
    let app_ev = app.clone();
    tokio::spawn(async move {
        while let Some(ev) = ev_stream.next().await {
            let _ = app_ev.emit("node-event", NodeEventPayload {
                node_id: ev.node_id.to_string(),
                event:   format!("{:?}", ev.event),
            });
        }
    });

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

#[tauri::command]
async fn subscribe_node_logs(
    state:   tauri::State<'_, AppState>,
    node_id: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&node_id).map_err(|e| e.to_string())?;
    let mut guard = state.client.lock().await;
    let c = guard.as_mut().ok_or("Not connected to VeloceCore")?;
    // Tell Core to start pushing log chunks for this node.
    // The background log_stream task (started in connect) automatically
    // receives them and emits "node-log" Tauri events.
    c.subscribe_node_logs(id).await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ── Resource usage ────────────────────────────────────────────────────────────

#[tauri::command]
async fn query_resources(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ResourceRow>, String> {
    let mut guard = state.client.lock().await;
    let c = guard.as_mut().ok_or("Not connected to VeloceCore")?;
    c.query_node_resources().await
        .map(|list| list.into_iter().map(|r| ResourceRow {
            node_id:   r.node_id.to_string(),
            cpu_ms:    r.cpu_ms,
            mem_bytes: r.mem_bytes,
        }).collect())
        .map_err(|e| e.to_string())
}

// ── Node Templates ────────────────────────────────────────────────────────────

/// Full description of a spawn template, shared between save and get.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct TemplateSpec {
    name:          String,
    app_name:      String,
    executable:    String,
    args:          Vec<String>,
    cpu_pct:       Option<u32>,
    mem_mb:        Option<u64>,
    lifetime_secs: Option<u64>,
    max_restarts:  Option<u32>,
}

impl From<TemplateSpec> for SpawnNodeMsg {
    fn from(t: TemplateSpec) -> Self {
        SpawnNodeMsg {
            app_name:   t.app_name,
            executable: t.executable,
            args:       t.args,
            env:        vec![],
            limits: if t.cpu_pct.is_some() || t.mem_mb.is_some() || t.lifetime_secs.is_some() {
                Some(NodeLimits { cpu_pct: t.cpu_pct, mem_mb: t.mem_mb, max_lifetime_secs: t.lifetime_secs })
            } else {
                None
            },
            auto_kill:      true,
            restart_policy: t.max_restarts.map(|mr| RestartPolicy {
                max_restarts:   mr,
                base_delay_secs: 2,
                max_delay_secs:  60,
            }),
        }
    }
}

#[tauri::command]
async fn list_templates(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let mut g = state.client.lock().await;
    let c = g.as_mut().ok_or("Not connected")?;
    c.list_templates().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_template(
    state: tauri::State<'_, AppState>,
    name:  String,
) -> Result<Option<TemplateSpec>, String> {
    let mut g = state.client.lock().await;
    let c = g.as_mut().ok_or("Not connected")?;
    match c.get_template(&name).await.map_err(|e| e.to_string())? {
        None      => Ok(None),
        Some(msg) => Ok(Some(TemplateSpec {
            name,
            app_name:      msg.app_name,
            executable:    msg.executable,
            args:          msg.args,
            cpu_pct:       msg.limits.as_ref().and_then(|l| l.cpu_pct),
            mem_mb:        msg.limits.as_ref().and_then(|l| l.mem_mb),
            lifetime_secs: msg.limits.as_ref().and_then(|l| l.max_lifetime_secs),
            max_restarts:  msg.restart_policy.map(|rp| rp.max_restarts),
        })),
    }
}

#[tauri::command]
async fn save_template(
    state: tauri::State<'_, AppState>,
    spec:  TemplateSpec,
) -> Result<(), String> {
    let mut g = state.client.lock().await;
    let c = g.as_mut().ok_or("Not connected")?;
    let name = spec.name.clone();
    let msg: SpawnNodeMsg = spec.into();
    c.save_template(&name, msg).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_template(
    state: tauri::State<'_, AppState>,
    name:  String,
) -> Result<(), String> {
    let mut g = state.client.lock().await;
    let c = g.as_mut().ok_or("Not connected")?;
    c.delete_template(&name).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn spawn_from_template(
    state: tauri::State<'_, AppState>,
    name:  String,
) -> Result<SpawnResult, String> {
    let mut g = state.client.lock().await;
    let c = g.as_mut().ok_or("Not connected")?;
    c.spawn_from_template(&name).await
        .map(|r| SpawnResult { node_id: r.node_id.to_string(), pid: r.pid, node_pipe: r.node_pipe })
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
            subscribe_node_logs,
            query_resources,
            list_templates,
            get_template,
            save_template,
            delete_template,
            spawn_from_template,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VeloceNetwork Dashboard");
}
