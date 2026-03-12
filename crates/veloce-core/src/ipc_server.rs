/*!
Named-pipe IPC server.

One server instance per pipe endpoint; creates a new instance (overlapping)
for each incoming client.  Tokio's `tokio::net::windows::named_pipe` handles
the async I/O.

Each client runs a `ClientSession` task that:
1. Validates the Handshake
2. Dispatches all subsequent messages to handlers
3. Forwards NodeEvent broadcasts back to the client
*/

use anyhow::{bail, Context, Result};
use bytes::BytesMut;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use veloce_ipc::{
    Codec,
    message::{
        Body, Capability, Envelope, ErrorCode, ErrorMsg, Flags,
        HandshakeAckMsg, MeshConnectMsg, MeshDisconnectMsg,
        NodeEventMsg, NodeInfo, NodeKilledMsg, NodeListMsg,
        NodeLogChunkMsg, NodeResourceMsg, NodeSpawnedMsg, NodeStatus as IpcNodeStatus,
    },
    PIPE_NAME,
};

use crate::{
    job,
    pipe_security,
    state::{CoreState, NodeSummary},
};

// ── SERVER ENTRY POINT ────────────────────────────────────────────────────────

pub async fn run(state: Arc<CoreState>) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    // Compute server's own user SID once; used to gate every incoming connection.
    let server_sid = pipe_security::server_user_sid()
        .context("resolve server user SID")?;
    tracing::debug!("pipe ACL: accepting connections from SID {server_sid}");

    tracing::info!("IPC server listening on {PIPE_NAME}");

    loop {
        if state.is_shutting_down() { break; }

        // Create a new pipe instance waiting for the next client
        let pipe = ServerOptions::new()
            .first_pipe_instance(false)
            .create(PIPE_NAME)
            .context("create named pipe instance")?;

        // Wait for a client to connect
        pipe.connect().await.context("pipe connect")?;

        // ── ACL gate: reject any process not running as the server's user ──
        if let Err(e) = pipe_security::assert_client_is_owner(&pipe, &server_sid) {
            tracing::warn!("pipe ACL rejected connection: {e:#}");
            // Dropping `pipe` here disconnects the client immediately.
            continue;
        }

        let client_state = state.clone();
        tokio::spawn(async move {
            let (read, write) = tokio::io::split(pipe);
            let mut session = ClientSession::new(read, write, client_state);
            if let Err(e) = session.run().await {
                tracing::warn!("client session error: {e:#}");
            }
        });
    }

    tracing::info!("IPC server stopped");
    Ok(())
}

// ── CLIENT SESSION ────────────────────────────────────────────────────────────

/// Capacity of the push-event channel per client session.
const PUSH_CHAN_CAP: usize = 64;

struct ClientSession<R, W> {
    reader:       R,
    writer:       W,
    state:        Arc<CoreState>,
    client_id:    Option<Uuid>,
    app_name:     String,
    capabilities: Vec<Capability>,
    read_buf:     BytesMut,
    /// Sender half — cloned into event-forwarder tasks so they can push
    /// encoded frames to the writer without holding the writer lock.
    push_tx:      mpsc::Sender<Vec<u8>>,
    /// Receiver half — drained by the select! loop in `run()`.
    push_rx:      mpsc::Receiver<Vec<u8>>,
}

impl<R, W> ClientSession<R, W>
where
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send,
{
    fn new(reader: R, writer: W, state: Arc<CoreState>) -> Self {
        let (push_tx, push_rx) = mpsc::channel(PUSH_CHAN_CAP);
        Self {
            reader,
            writer,
            state,
            client_id:    None,
            app_name:     String::new(),
            capabilities: vec![],
            read_buf:     BytesMut::with_capacity(16 * 1024),
            push_tx,
            push_rx,
        }
    }

    async fn run(&mut self) -> Result<()> {
        let mut buf = [0u8; 8192];
        loop {
            tokio::select! {
                // ── Incoming request frames from the client ──────────────────
                result = self.reader.read(&mut buf) => {
                    let n = result?;
                    if n == 0 {
                        tracing::debug!(app = %self.app_name, "client disconnected");
                        return Ok(());
                    }
                    self.read_buf.extend_from_slice(&buf[..n]);

                    loop {
                        match Codec::decode_safe(&mut self.read_buf) {
                            Ok(Some((_hdr, env))) => {
                                if let Err(e) = self.handle(env).await {
                                    tracing::warn!("handler error: {e:#}");
                                    self.send_error(None, ErrorCode::InternalError, e.to_string()).await?;
                                }
                            }
                            Ok(None)  => break,
                            Err(e) => {
                                tracing::warn!("decode error: {e:#}");
                                self.send_error(None, ErrorCode::InvalidMessage, e.to_string()).await?;
                                self.read_buf.clear();
                                break;
                            }
                        }
                    }
                }

                // ── Outbound push frames from event-forwarder tasks ──────────
                Some(frame) = self.push_rx.recv() => {
                    self.writer.write_all(&frame).await?;
                }
            }
        }
    }

    async fn handle(&mut self, env: Envelope) -> Result<()> {
        use Body::*;
        let cid = env.correlation_id;

        match env.body {
            // ── Heartbeat ──────────────────────────────────────────────────
            Ping => self.send_reply(cid, Pong).await?,
            Pong => {}

            // ── Handshake ──────────────────────────────────────────────────
            Handshake(hs) => {
                if self.client_id.is_some() {
                    return self.send_error(Some(cid), ErrorCode::ProtocolMismatch,
                        "already handshaked".into()).await;
                }

                // ── PSK gate ───────────────────────────────────────────────
                // Skip in dev mode: set VELOCE_SKIP_PSK=1 in the environment.
                let skip_psk = std::env::var("VELOCE_SKIP_PSK").as_deref() == Ok("1");
                if !skip_psk {
                    let ok = match hs.psk_hash {
                        Some(received) => received == *self.state.psk(),
                        None           => false,
                    };
                    if !ok {
                        tracing::warn!(app = %hs.app_name, "PSK mismatch — rejecting connection");
                        self.send_error(Some(cid), ErrorCode::Unauthorized,
                            "invalid or missing PSK".into()).await?;
                        bail!("PSK rejected");
                    }
                }

                let client_id = Uuid::new_v4();
                self.client_id    = Some(client_id);
                self.app_name     = hs.app_name.clone();
                self.capabilities = hs.capabilities.clone();

                tracing::info!(
                    app = %hs.app_name,
                    sdk_version = %hs.sdk_version,
                    %client_id,
                    "client handshake accepted"
                );

                self.send_reply(cid, HandshakeAck(HandshakeAckMsg {
                    client_id,
                    core_version:    env!("CARGO_PKG_VERSION").into(),
                    granted:         hs.capabilities,
                    core_started_at: chrono::Utc::now(),
                })).await?;
            }

            Goodbye { reason } => {
                tracing::info!(app = %self.app_name, ?reason, "client goodbye");
                return Ok(()); // let the task exit
            }

            // ── Node management ────────────────────────────────────────────
            SpawnNode(msg) => {
                self.require_cap(Capability::SpawnNodes)?;
                if !self.state.policy.check_capability(&self.app_name, "SpawnNodes") {
                    return self.send_error(Some(cid), ErrorCode::PolicyDenied,
                        format!("policy denies SpawnNodes for app '{}'", self.app_name)).await;
                }
                let node_id = Uuid::new_v4();

                // Allocate registry slot
                let pipe = format!(r"\\.\pipe\VeloceNode-{}", node_id.simple());
                let slot = self.state.registry()
                    .alloc_node(node_id, &msg.app_name, &pipe)
                    .context("alloc registry slot")?;

                // Spawn under a Job Object
                let handle = job::spawn_node(&msg, node_id, slot, PIPE_NAME, None, None).await
                    .context("spawn_node")?;

                let pid       = handle.pid;
                let pipe_path = handle.pipe_path.clone();
                let event_tx  = handle.event_tx.clone();

                self.state.registry().set_node_pid(slot, pid)?;
                self.state.node_table().insert(handle);

                let spawned_at = chrono::Utc::now();
                self.send_reply(cid, NodeSpawned(NodeSpawnedMsg {
                    node_id, pid, node_pipe: pipe_path.clone(), spawned_at,
                })).await?;

                // Forward NodeEvents to this client via the push channel.
                spawn_event_forwarder(event_tx.subscribe(), self.push_tx.clone());
            }

            KillNode(msg) => {
                self.require_cap(Capability::KillNodes)?;
                if !self.state.policy.check_capability(&self.app_name, "KillNodes") {
                    return self.send_error(Some(cid), ErrorCode::PolicyDenied,
                        format!("policy denies KillNodes for app '{}'", self.app_name)).await;
                }
                let handle = self.state.node_table().remove(msg.node_id);
                match handle {
                    None => self.send_error(Some(cid), ErrorCode::NotFound,
                        format!("node {} not found", msg.node_id)).await?,
                    Some(h) => {
                        let exit = msg.exit_code.unwrap_or(1);
                        job::terminate_node(&h, exit).context("terminate_node")?;
                        let slot = h.slot_idx;
                        self.state.registry().set_node_status(
                            slot,
                            crate::registry::NodeStatus::Stopped,
                            exit,
                        )?;
                        self.send_reply(cid, NodeKilled(NodeKilledMsg {
                            node_id:   msg.node_id,
                            exit_code: exit,
                        })).await?;
                    }
                }
            }

            QueryNodes => {
                let nodes: Vec<NodeInfo> = self.state.node_table()
                    .list_live()
                    .into_iter()
                    .map(|s| NodeInfo {
                        node_id:    s.node_id,
                        app_name:   s.app_name,
                        pid:        s.pid,
                        status:     IpcNodeStatus::Running,
                        spawned_at: chrono::Utc::now(),
                        node_pipe:  s.pipe_path,
                    })
                    .collect();
                self.send_reply(cid, NodeList(NodeListMsg { nodes })).await?;
            }

            // ── Registry ───────────────────────────────────────────────────
            RegistryGet { key } => {
                self.require_cap(Capability::RegistryRead)?;
                let value = self.state.registry().kv_get(&key);
                self.send_reply(cid, RegistryValue(
                    veloce_ipc::message::RegistryValueMsg { key, value }
                )).await?;
            }

            RegistrySet { key, value } => {
                self.require_cap(Capability::RegistryWrite)?;
                self.state.registry().kv_set(&key, &value)
                    .context("kv_set")?;
                self.send_reply(cid, RegistryAck { key }).await?;
            }

            // ── VeloceNet ──────────────────────────────────────────────────
            NetRegisterHost(msg) => {
                self.require_cap(Capability::NetRegister)?;
                if !self.state.policy.check_capability(&self.app_name, "NetRegister") {
                    return self.send_error(Some(cid), ErrorCode::PolicyDenied,
                        format!("policy denies NetRegister for app '{}'", self.app_name)).await;
                }
                self.state.net_registry().register(
                    msg.hostname.clone(), msg.node_id, msg.local_port, msg.ttl_secs,
                );
                self.send_reply(cid, NetHostRegistered {
                    hostname: msg.hostname,
                    addr:     format!("127.0.0.1:{}", msg.local_port),
                }).await?;
            }

            NetUnregisterHost { hostname } => {
                self.state.net_registry().unregister(&hostname);
                self.send_reply(cid, Body::Pong).await?; // ack
            }

            NetResolve { hostname } => {
                self.require_cap(Capability::NetResolve)?;
                let result = self.state.net_registry().resolve(&hostname);
                self.send_reply(cid, NetResolveResult(
                    veloce_ipc::message::NetResolveResultMsg {
                        hostname: hostname.clone(),
                        address:  result.as_ref().map(|r| format!("127.0.0.1:{}", r.local_port)),
                        node_id:  result.map(|r| r.node_id),
                    }
                )).await?;
            }

            // ── Resource usage ─────────────────────────────────────────────
            QueryNodeResources => {
                let resources: Vec<NodeResourceMsg> = self.state.node_table()
                    .query_all_resources()
                    .into_iter()
                    .map(|(node_id, _pid, cpu_ms, mem_bytes)| NodeResourceMsg {
                        node_id, cpu_ms, mem_bytes,
                    })
                    .collect();
                self.send_reply(cid, NodeResourceList(resources)).await?;
            }

            // ── Push event subscriptions ───────────────────────────────────
            SubscribeNodeEvents { node_id } => {
                let event_tx = self.state.node_table()
                    .list_live()
                    .into_iter()
                    .find(|s| s.node_id == node_id)
                    .map(|s| s.event_tx);

                match event_tx {
                    None => self.send_error(Some(cid), ErrorCode::NotFound,
                        format!("node {} not found", node_id)).await?,
                    Some(tx) => {
                        spawn_event_forwarder(tx.subscribe(), self.push_tx.clone());
                        self.send_reply(cid, Body::Pong).await?;
                    }
                }
            }

            UnsubscribeNodeEvents { node_id } => {
                tracing::debug!(%node_id, "client unsubscribed from node events");
                self.send_reply(cid, Body::Pong).await?;
            }

            // ── Log subscriptions ──────────────────────────────────────────
            SubscribeNodeLogs { node_id } => {
                let log_tx = self.state.node_table()
                    .list_live()
                    .into_iter()
                    .find(|s| s.node_id == node_id)
                    .map(|s| s.log_tx);

                match log_tx {
                    None => self.send_error(Some(cid), ErrorCode::NotFound,
                        format!("node {} not found", node_id)).await?,
                    Some(tx) => {
                        spawn_log_forwarder(tx.subscribe(), self.push_tx.clone());
                        self.send_reply(cid, Body::Pong).await?;
                    }
                }
            }

            UnsubscribeNodeLogs { node_id } => {
                // Log forwarder tasks auto-exit when the node's broadcast sender
                // is dropped; explicit unsubscribe just logs intent.
                tracing::debug!(%node_id, "client unsubscribed from node logs");
                self.send_reply(cid, Body::Pong).await?;
            }

            // ── Mesh P2P ──────────────────────────────────────────────────
            Body::MeshGetInfo => {
                let info = self.state.mesh.as_ref()
                    .map(|m| m.mesh_info())
                    .ok_or_else(|| anyhow::anyhow!("mesh not initialised"))?;
                self.send_reply(cid, Body::MeshInfo(info)).await?;
            }

            Body::MeshConnect(MeshConnectMsg { join_code }) => {
                let mesh = self.state.mesh.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("mesh not initialised"))?;
                match mesh.connect_to_peer(&join_code).await {
                    Ok(result) => self.send_reply(cid, Body::MeshConnectResult(result)).await?,
                    Err(e) => self.send_error(Some(cid), ErrorCode::InternalError,
                        format!("mesh connect: {e}")).await?,
                }
            }

            Body::MeshPeerList => {
                let peers = self.state.mesh.as_ref()
                    .map(|m| m.peer_list())
                    .unwrap_or_default();
                self.send_reply(cid, Body::MeshPeerListResult(peers)).await?;
            }

            Body::MeshDisconnect(MeshDisconnectMsg { peer_id }) => {
                if let Some(mesh) = &self.state.mesh {
                    if let Err(e) = mesh.disconnect(peer_id).await {
                        self.send_error(Some(cid), ErrorCode::NotFound,
                            format!("mesh disconnect: {e}")).await?;
                        return Ok(());
                    }
                }
                self.send_reply(cid, Body::Pong).await?;
            }

            // ── Policy engine ─────────────────────────────────────────────
            Body::PolicyGetRules => {
                let msg = self.state.policy.to_msg();
                self.send_reply(cid, Body::PolicyRulesResult(msg)).await?;
            }

            Body::PolicyReload => {
                match self.state.policy.reload() {
                    Ok(()) => {
                        let msg = self.state.policy.to_msg();
                        self.send_reply(cid, Body::PolicyRulesResult(msg)).await?;
                    }
                    Err(e) => self.send_error(Some(cid), ErrorCode::InternalError,
                        format!("policy reload: {e}")).await?,
                }
            }

            // PolicyRulesResult is a server→client message; reject it from clients.
            Body::PolicyRulesResult(_) => {
                self.send_error(Some(cid), ErrorCode::InvalidMessage,
                    "PolicyRulesResult is server-to-client only".into()).await?;
            }

            other => {
                tracing::warn!("unhandled message type: {:?}", other.msg_type());
                self.send_error(Some(cid), ErrorCode::InvalidMessage,
                    format!("unhandled: {:?}", other.msg_type())).await?;
            }
        }

        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn require_cap(&self, cap: Capability) -> Result<()> {
        if self.capabilities.contains(&cap) { return Ok(()); }
        bail!("unauthorized: missing capability {:?}", cap);
    }

    async fn send_reply(&mut self, cid: Uuid, body: Body) -> Result<()> {
        let env  = Envelope::reply(cid, body);
        let buf  = Codec::encode(&env, Flags::empty())?;
        self.writer.write_all(&buf).await?;
        Ok(())
    }

    async fn send_error(
        &mut self,
        context_id: Option<Uuid>,
        code:        ErrorCode,
        message:     String,
    ) -> Result<()> {
        let cid = context_id.unwrap_or_else(Uuid::new_v4);
        let env = Envelope::reply(cid, Body::Error(ErrorMsg { code, message, context_id }));
        let buf = Codec::encode(&env, Flags::URGENT)?;
        self.writer.write_all(&buf).await?;
        Ok(())
    }
}

// ── EVENT FORWARDER ────────────────────────────────────────────────────────────

/// Forwards node lifecycle events to a client's push channel.
fn spawn_event_forwarder(
    mut rx:  tokio::sync::broadcast::Receiver<crate::job::NodeEventMsg>,
    push_tx: mpsc::Sender<Vec<u8>>,
) {
    tokio::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(e)  => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("event forwarder lagged by {n} messages");
                    continue;
                }
                Err(_) => break,
            };

            let env = Envelope::new(Body::NodeEvent(NodeEventMsg {
                node_id: ev.node_id,
                event:   ev.event,
            }));
            match Codec::encode(&env, Flags::PUSH) {
                Ok(frame) => {
                    if push_tx.send(frame.to_vec()).await.is_err() { break; }
                }
                Err(e) => tracing::warn!("event encode error: {e:#}"),
            }
        }
    });
}

/// Forwards captured stdout/stderr chunks to a client's push channel.
fn spawn_log_forwarder(
    mut rx:  tokio::sync::broadcast::Receiver<NodeLogChunkMsg>,
    push_tx: mpsc::Sender<Vec<u8>>,
) {
    tokio::spawn(async move {
        loop {
            let chunk = match rx.recv().await {
                Ok(c)  => c,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("log forwarder lagged by {n} chunks");
                    continue;
                }
                Err(_) => break,
            };

            let env = Envelope::new(Body::NodeLogChunk(chunk));
            match Codec::encode(&env, Flags::PUSH) {
                Ok(frame) => {
                    if push_tx.send(frame.to_vec()).await.is_err() { break; }
                }
                Err(e) => tracing::warn!("log encode error: {e:#}"),
            }
        }
    });
}