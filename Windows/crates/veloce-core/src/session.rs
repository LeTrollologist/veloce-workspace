/*!
Shared client session logic.

Used by the Windows named-pipe server (`ipc_server.rs`).  When the Linux
port is compiled, the Unix socket server (`socket_server.rs`) also uses
this module.  Each transport accept loop creates a `ClientSession` after
performing its platform-specific auth check.
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
        HandshakeAckMsg, MeshConnectMsg, MeshDisconnectMsg, MeshGetJoinCodeV3Msg, MeshJoinCodeV3ResultMsg,
        NetAddIngressMsg, NodeEventMsg, NodeInfo, NodeKilledMsg, NodeListMsg,
        NodeLogChunkMsg, NodeResourceMsg, NodeSpawnedMsg, NodeStatus as IpcNodeStatus,
        NodeStatusMsg, TrafficStatsMsg,
    },
};

use crate::{
    job,
    state::CoreState,
};

// ── PLATFORM-SPECIFIC NODE ENDPOINT ──────────────────────────────────────────

/// Returns the per-node IPC endpoint identifier.
/// On Windows: named pipe path.
/// On Unix: Unix socket path under /run/veloce/.
fn node_socket_path(node_id: uuid::Uuid) -> String {
    #[cfg(windows)]
    { format!(r"\\.\pipe\VeloceNode-{}", node_id.simple()) }
    #[cfg(unix)]
    { format!("/run/veloce/node-{}.sock", node_id.simple()) }
    #[cfg(not(any(windows, unix)))]
    { format!("veloce-node-{}", node_id.simple()) }
}

// ── CLIENT SESSION ────────────────────────────────────────────────────────────

/// Capacity of the push-event channel per client session.
const PUSH_CHAN_CAP: usize = 64;

pub(crate) struct ClientSession<R, W> {
    reader:       R,
    writer:       W,
    state:        Arc<CoreState>,
    client_id:    Option<Uuid>,
    /// Kernel-verified image path of the connecting process.
    /// Set once at accept time from the platform auth check;
    /// never overwritten by client-supplied data.
    exe_path:     String,
    /// PID of the connecting process, extracted from the transport endpoint
    /// at accept time.  Used in denied-capability audit log entries.
    client_pid:   u32,
    /// String identifier of the server process's user.
    /// On Windows: SID string.  On Linux: UID string.
    /// Used to enforce that `VELOCE_SKIP_PSK` cannot be honoured when
    /// running as SYSTEM (S-1-5-18) / root (UID 0).
    server_sid:   String,
    app_name:     String,
    capabilities: Vec<Capability>,
    read_buf:     BytesMut,
    /// Sender half — cloned into event-forwarder tasks so they can push
    /// encoded frames to the writer without holding the writer lock.
    push_tx:      mpsc::Sender<Vec<u8>>,
    /// Receiver half — drained by the select! loop in `run()`.
    push_rx:      mpsc::Receiver<Vec<u8>>,
    /// Node IDs spawned with `auto_kill = true` by this session.
    /// On session exit these nodes are terminated automatically.
    auto_kill_nodes: Vec<Uuid>,
}

impl<R, W> ClientSession<R, W>
where
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send,
{
    pub(crate) fn new(
        reader:     R,
        writer:     W,
        state:      Arc<CoreState>,
        exe_path:   String,
        client_pid: u32,
        server_sid: String,
    ) -> Self {
        let (push_tx, push_rx) = mpsc::channel(PUSH_CHAN_CAP);
        Self {
            reader,
            writer,
            state,
            client_id:    None,
            exe_path,
            client_pid,
            server_sid,
            app_name:     String::new(),
            capabilities: vec![],
            read_buf:     BytesMut::with_capacity(16 * 1024),
            push_tx,
            push_rx,
            auto_kill_nodes: vec![],
        }
    }

    pub(crate) async fn run(&mut self) -> Result<()> {
        let result = self.run_impl().await;
        self.cleanup_auto_kill().await;
        result
    }

    async fn cleanup_auto_kill(&mut self) {
        for node_id in std::mem::take(&mut self.auto_kill_nodes) {
            let handle = self.state.node_table().remove(node_id);
            if let Some(h) = handle {
                let slot = h.slot_idx;
                if let Err(e) = job::terminate_node(&h, 1) {
                    tracing::warn!(%node_id, "auto-kill terminate error: {e}");
                }
                let _ = self.state.registry().set_node_status(
                    slot,
                    crate::registry::NodeStatus::Stopped,
                    1,
                );
                tracing::info!(app = %self.app_name, %node_id, "auto-killed node on client disconnect");
            }
        }
    }

    async fn run_impl(&mut self) -> Result<()> {
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
                                // H1: pre-auth gate — only Handshake is allowed before
                                // the session is established.  Drop the connection
                                // immediately on any protocol violation.
                                if self.client_id.is_none() && !matches!(env.body, Body::Handshake(_)) {
                                    tracing::warn!(
                                        exe  = %self.exe_path,
                                        msg  = ?env.body.msg_type(),
                                        "pre-auth protocol violation — dropping connection"
                                    );
                                    return Err(anyhow::anyhow!(
                                        "pre-auth protocol violation: {:?} received before Handshake",
                                        env.body.msg_type()
                                    ));
                                }
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
                // VELOCE_SKIP_PSK=1 disables PSK auth in development.
                // It is silently ignored when the server runs as SYSTEM (S-1-5-18)
                // to prevent accidental exposure in production service deployments.
                const SYSTEM_SID: &str = "S-1-5-18";
                let skip_psk = if std::env::var("VELOCE_SKIP_PSK").as_deref() == Ok("1") {
                    if self.server_sid == SYSTEM_SID {
                        tracing::error!(
                            "VELOCE_SKIP_PSK=1 is IGNORED — \
                             PSK auth is mandatory when running as SYSTEM"
                        );
                        false
                    } else {
                        tracing::error!(
                            exe = %self.exe_path,
                            "VELOCE_SKIP_PSK=1 active — \
                             PSK authentication DISABLED (dev mode only)"
                        );
                        true
                    }
                } else {
                    false
                };
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

                // Server computes the maximum allowed capabilities from the
                // kernel-verified exe path.  The client's declared request is
                // intersected with this set — clients can still ask for less.
                let max_caps = self.state.policy.compute_max_caps(&self.exe_path);
                let granted: Vec<Capability> = hs.capabilities.iter()
                    .filter(|c| max_caps.contains(c))
                    .cloned()
                    .collect();

                let client_id = Uuid::new_v4();
                self.client_id    = Some(client_id);
                self.app_name     = hs.app_name.clone();  // display/logging only
                self.capabilities = granted.clone();

                tracing::info!(
                    exe  = %self.exe_path,
                    app  = %hs.app_name,
                    sdk_version = %hs.sdk_version,
                    %client_id,
                    ?granted,
                    "client handshake accepted"
                );

                self.send_reply(cid, HandshakeAck(HandshakeAckMsg {
                    client_id,
                    core_version:    env!("CARGO_PKG_VERSION").into(),
                    granted,
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
                let pipe = node_socket_path(node_id);
                let slot = self.state.registry()
                    .alloc_node(node_id, &msg.app_name, &pipe)
                    .context("alloc registry slot")?;

                // Spawn under a Job Object
                let handle = job::spawn_node(&msg, node_id, slot, &pipe, None, None).await
                    .context("spawn_node")?;

                let pid       = handle.pid;
                let pipe_path = handle.pipe_path.clone();
                let event_tx  = handle.event_tx.clone();

                self.state.registry().set_node_pid(slot, pid)?;
                // Track nodes with auto_kill for cleanup on session exit.
                if msg.auto_kill {
                    self.auto_kill_nodes.push(node_id);
                }
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
                let handle = self.state.node_table().remove(msg.node_id);
                // Remove from auto-kill list so we don't double-terminate.
                self.auto_kill_nodes.retain(|&id| id != msg.node_id);
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
                let nodes: Vec<NodeInfo> = self.state.registry()
                    .list_nodes()
                    .into_iter()
                    .map(|e| NodeInfo {
                        node_id:    e.node_id,
                        app_name:   e.app_name,
                        pid:        e.pid,
                        status:     match e.status {
                            crate::registry::NodeStatus::Running  => IpcNodeStatus::Running,
                            crate::registry::NodeStatus::Stopping => IpcNodeStatus::Stopping,
                            crate::registry::NodeStatus::Stopped  => IpcNodeStatus::Stopped,
                            crate::registry::NodeStatus::Crashed  => IpcNodeStatus::Crashed { exit_code: e.exit_code },
                            crate::registry::NodeStatus::Empty    => IpcNodeStatus::Running,
                        },
                        spawned_at: e.spawned_at,
                        node_pipe:  e.pipe_path,
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
                self.require_cap(Capability::NetRegister)?;
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

            Body::MeshGetJoinCodeV3(MeshGetJoinCodeV3Msg { ttl_mins, one_time }) => {
                self.require_cap(Capability::MeshManage)?;
                let join_code = self.state.mesh.as_ref()
                    .map(|m| m.join_code_v3(ttl_mins, one_time))
                    .ok_or_else(|| anyhow::anyhow!("mesh not initialised"))?;
                self.send_reply(cid, Body::MeshJoinCodeV3Result(MeshJoinCodeV3ResultMsg { join_code })).await?;
            }

            Body::MeshConnect(MeshConnectMsg { join_code }) => {
                self.require_cap(Capability::MeshManage)?;
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
                self.require_cap(Capability::MeshManage)?;
                if let Some(mesh) = &self.state.mesh {
                    if let Err(e) = mesh.disconnect(peer_id).await {
                        self.send_error(Some(cid), ErrorCode::NotFound,
                            format!("mesh disconnect: {e}")).await?;
                        return Ok(());
                    }
                }
                self.send_reply(cid, Body::Pong).await?;
            }

            Body::MeshPingPeer { peer_id } => {
                let latency_ms = self.state.mesh.as_ref().and_then(|m| {
                    m.peers.blocking_read()
                        .get(&peer_id)
                        .map(|p| {
                            let ms = p.latency_ms.load(std::sync::atomic::Ordering::Relaxed);
                            // 0 means no sample recorded yet
                            if ms == 0 { None } else { Some(ms) }
                        })
                        .flatten()
                });
                self.send_reply(cid, Body::MeshPingResult { peer_id, latency_ms }).await?;
            }

            // ── Policy engine ─────────────────────────────────────────────
            Body::PolicyGetRules => {
                let msg = self.state.policy.to_msg();
                self.send_reply(cid, Body::PolicyRulesResult(msg)).await?;
            }

            Body::PolicyReload => {
                self.require_cap(Capability::PolicyAdmin)?;
                match self.state.policy.reload() {
                    Ok(()) => {
                        // Re-evaluate this session's capabilities under the new policy.
                        // Capabilities can only shrink (intersection with new max_caps).
                        // Other active sessions will be re-evaluated on their next
                        // require_cap call if the policy changes; they should reconnect
                        // to receive an expanded grant.
                        let max_caps = self.state.policy.compute_max_caps(&self.exe_path);
                        self.capabilities = self.capabilities.iter()
                            .filter(|c| max_caps.contains(c))
                            .cloned()
                            .collect();
                        tracing::info!(
                            app = %self.app_name,
                            caps = ?self.capabilities,
                            "session capabilities re-evaluated after policy reload"
                        );
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

            // ── Traffic stats ──────────────────────────────────────────────
            Body::TrafficQuery => {
                let host_stats = self.state.net_registry().traffic_snapshot();
                let stats = self.state.mesh.as_ref()
                    .map(|m| m.query_traffic_stats(host_stats.clone()))
                    .unwrap_or_else(|| TrafficStatsMsg { hosts: host_stats, ..Default::default() });
                self.send_reply(cid, Body::TrafficStatsResult(stats)).await?;
            }

            // TrafficStatsResult is a server→client message; reject it from clients.
            Body::TrafficStatsResult(_) => {
                self.send_error(Some(cid), ErrorCode::InvalidMessage,
                    "TrafficStatsResult is server-to-client only".into()).await?;
            }

            // ── Port forwarding (v1.1) ─────────────────────────────────────
            Body::NetAddPortForward(msg) => {
                self.require_cap(Capability::NetPortForward)?;
                match self.state.port_forward_table()
                    .add(msg.name.clone(), msg.host_port, msg.target_port, msg.node_id)
                    .await
                {
                    Ok(entry) => self.send_reply(cid, Body::NetPortForwardAdded(entry)).await?,
                    Err(e) => self.send_error(Some(cid), ErrorCode::InternalError,
                        format!("port forward: {e}")).await?,
                }
            }

            Body::NetRemovePortForward { name } => {
                self.require_cap(Capability::NetPortForward)?;
                self.state.port_forward_table().remove(&name);
                self.send_reply(cid, Body::Pong).await?;
            }

            Body::NetListPortForwards => {
                let list = self.state.port_forward_table().list();
                self.send_reply(cid, Body::NetPortForwardList(list)).await?;
            }

            // ── Named volumes (v1.2) ───────────────────────────────────────
            Body::VolumeRegister(msg) => {
                self.require_cap(Capability::RegistryWrite)?;
                match self.state.volume_registry().get_or_create(&msg.name) {
                    Ok(path) => self.send_reply(cid, Body::VolumeRegistered(
                        veloce_ipc::message::VolumeRegisteredMsg {
                            name:      msg.name,
                            host_path: path.to_string_lossy().into_owned(),
                        }
                    )).await?,
                    Err(e) => self.send_error(Some(cid), ErrorCode::InternalError,
                        format!("volume register: {e}")).await?,
                }
            }

            Body::VolumeList => {
                let volumes = self.state.volume_registry().list();
                self.send_reply(cid, Body::VolumeListResult(volumes)).await?;
            }

            // ── Secrets (v1.2) ─────────────────────────────────────────────
            Body::SecretSet(msg) => {
                self.require_cap(Capability::SecretsWrite)?;
                match self.state.secrets_vault().set(&msg.name, &msg.plaintext) {
                    Ok(()) => self.send_reply(cid, Body::SecretSetAck { name: msg.name }).await?,
                    Err(e) => self.send_error(Some(cid), ErrorCode::InternalError,
                        format!("secret set: {e}")).await?,
                }
            }

            Body::SecretDelete { name } => {
                self.require_cap(Capability::SecretsWrite)?;
                match self.state.secrets_vault().delete(&name) {
                    Ok(()) => self.send_reply(cid, Body::SecretDeleteAck { name }).await?,
                    Err(e) => self.send_error(Some(cid), ErrorCode::InternalError,
                        format!("secret delete: {e}")).await?,
                }
            }

            Body::SecretList => {
                self.require_cap(Capability::SecretsRead)?;
                let names = self.state.secrets_vault().list();
                self.send_reply(cid, Body::SecretListResult(names)).await?;
            }

            // ── Desired state / reconciler (v1.3) ──────────────────────────
            Body::ApplyDesiredState(spec) => {
                self.require_cap(Capability::DesiredStateManage)?;
                let name = spec.name.clone();
                self.state.reconciler().apply(spec);
                self.send_reply(cid, Body::DesiredStateApplied { name }).await?;
            }

            // ── Ingress (v2.1) ─────────────────────────────────────────────
            Body::NetAddIngress(NetAddIngressMsg { rule }) => {
                self.require_cap(Capability::NetRegister)?;
                let host = rule.host.clone();
                self.state.ingress_router().add_rule(rule).await;
                self.send_reply(cid, Body::NetIngressAdded { host }).await?;
            }

            Body::NetRemoveIngress { host } => {
                self.require_cap(Capability::NetRegister)?;
                self.state.ingress_router().remove_rule(&host).await;
                self.send_reply(cid, Body::NetRemoveIngress { host }).await?;
            }

            Body::NetListIngresses => {
                let rules = self.state.ingress_router().list_rules().await;
                self.send_reply(cid, Body::NetIngressList(rules)).await?;
            }

            // ── Extended node status (v1.3) ────────────────────────────────
            Body::QueryNodeStatus => {
                let statuses: Vec<NodeStatusMsg> = self.state.node_table()
                    .list_live()
                    .into_iter()
                    .map(|s| NodeStatusMsg {
                        node_id:       s.node_id,
                        app_name:      s.app_name,
                        pid:           s.pid,
                        health:        s.health,
                        spawned_at:    chrono::Utc::now(), // best-effort: registry would have exact time
                        service_name:  s.service_name,
                        replica_index: s.replica_index,
                    })
                    .collect();
                self.send_reply(cid, Body::NodeStatusResult(statuses)).await?;
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
        tracing::warn!(
            app  = %self.app_name,
            exe  = %self.exe_path,
            pid  = self.client_pid,
            ?cap,
            "unauthorized capability request"
        );
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
pub(crate) fn spawn_event_forwarder(
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
pub(crate) fn spawn_log_forwarder(
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
