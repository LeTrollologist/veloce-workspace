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
use tokio::sync::broadcast;
use uuid::Uuid;

use veloce_ipc::{
    Codec,
    message::{
        Body, Capability, Envelope, ErrorCode, ErrorMsg, Flags,
        HandshakeAckMsg, NodeInfo, NodeKilledMsg, NodeListMsg, NodeSpawnedMsg,
        NodeStatus as IpcNodeStatus,
    },
    PIPE_NAME,
};

use crate::{
    job,
    state::{CoreState, NodeSummary},
};

// ── SERVER ENTRY POINT ────────────────────────────────────────────────────────

pub async fn run(state: Arc<CoreState>) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

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

struct ClientSession<R, W> {
    reader:       R,
    writer:       W,
    state:        Arc<CoreState>,
    client_id:    Option<Uuid>,
    app_name:     String,
    capabilities: Vec<Capability>,
    read_buf:     BytesMut,
}

impl<R, W> ClientSession<R, W>
where
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send,
{
    fn new(reader: R, writer: W, state: Arc<CoreState>) -> Self {
        Self {
            reader,
            writer,
            state,
            client_id:    None,
            app_name:     String::new(),
            capabilities: vec![],
            read_buf:     BytesMut::with_capacity(16 * 1024),
        }
    }

    async fn run(&mut self) -> Result<()> {
        let mut buf = [0u8; 8192];
        loop {
            // Read some bytes
            let n = self.reader.read(&mut buf).await?;
            if n == 0 {
                tracing::debug!(app = %self.app_name, "client disconnected");
                return Ok(());
            }
            self.read_buf.extend_from_slice(&buf[..n]);

            // Drain all complete frames from the buffer
            loop {
                match Codec::decode_safe(&mut self.read_buf) {
                    Ok(Some((_hdr, env))) => {
                        if let Err(e) = self.handle(env).await {
                            tracing::warn!("handler error: {e:#}");
                            self.send_error(None, ErrorCode::InternalError, e.to_string()).await?;
                        }
                    }
                    Ok(None)    => break, // need more data
                    Err(e) => {
                        tracing::warn!("decode error: {e:#}");
                        self.send_error(None, ErrorCode::InvalidMessage, e.to_string()).await?;
                        self.read_buf.clear();
                        break;
                    }
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
                let client_id = Uuid::new_v4();
                self.client_id    = Some(client_id);
                self.app_name     = hs.app_name.clone();
                // Grant all requested capabilities (auth/PSK enforcement future)
                self.capabilities = hs.capabilities.clone();

                tracing::info!(
                    app = %hs.app_name,
                    sdk_version = %hs.sdk_version,
                    %client_id,
                    "client handshake"
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
                let node_id = Uuid::new_v4();

                // Allocate registry slot
                let pipe = format!(r"\\.\pipe\VeloceNode-{}", node_id.simple());
                let slot = self.state.registry()
                    .alloc_node(node_id, &msg.app_name, &pipe)
                    .context("alloc registry slot")?;

                // Spawn under a Job Object
                let handle = job::spawn_node(&msg, node_id, slot, PIPE_NAME).await
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

                // Forward NodeEvents back to this client
                let mut rx = event_tx.subscribe();
                let client_state = self.state.clone();
                // We can't easily write to the writer from two tasks, so we
                // queue events into a channel that the main loop drains.
                // For now log them; the full event-push path requires splitting
                // the write half further — tracked in VELOCE-42.
                tokio::spawn(async move {
                    while let Ok(ev) = rx.recv().await {
                        tracing::info!(node_id = %ev.node_id, event = ?ev.event, "node event");
                    }
                });
            }

            KillNode(msg) => {
                self.require_cap(Capability::KillNodes)?;
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