/*!
`VeloceClient` — fully async, multiplexed IPC client.

One background reader task drains the pipe and routes replies by
`correlation_id` into per-request `oneshot` channels, so callers
can fire concurrent requests without locking.
*/

use anyhow::{bail, Context, Result};
use bytes::BytesMut;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use uuid::Uuid;

use veloce_ipc::{
    Codec,
    message::{
        Body, Capability, DesiredStateSpec, Envelope, Flags,
        MeshConnectMsg, MeshConnectResultMsg, MeshDisconnectMsg, MeshInfoMsg,
        NetPortForwardMsg, NodeLogChunkMsg, NodeSpawnedMsg, NodeStatusMsg,
        NodeEventMsg, PeerInfoMsg, PolicyRulesMsg, PortForwardEntry, SpawnNodeMsg,
        TrafficStatsMsg, VolumeEntry, VolumeRegisteredMsg,
    },
    PIPE_NAME,
};

// ── REQUEST TABLE ─────────────────────────────────────────────────────────────

type PendingMap = Arc<Mutex<HashMap<Uuid, oneshot::Sender<Body>>>>;
type EventBus   = Arc<tokio::sync::broadcast::Sender<NodeEventMsg>>;
type LogBus     = Arc<tokio::sync::broadcast::Sender<NodeLogChunkMsg>>;

// ── CLIENT ────────────────────────────────────────────────────────────────────

pub struct VeloceClient {
    /// Write half — protected so concurrent callers can send without races.
    writer:    Arc<AsyncMutex<tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>>>,
    pending:   PendingMap,
    /// Broadcast bus for unsolicited NodeEvent push frames from Core.
    event_bus: EventBus,
    /// Broadcast bus for NodeLogChunk push frames from Core.
    log_bus:   LogBus,
    /// Our assigned client ID (set after handshake).
    pub client_id: Uuid,
}

// ── NODE EVENT STREAM ─────────────────────────────────────────────────────────

/// Async-friendly stream of `NodeEventMsg` values pushed by VeloceCore.
pub struct NodeEventStream {
    inner:  tokio::sync::broadcast::Receiver<NodeEventMsg>,
    filter: Option<Uuid>,
}

impl NodeEventStream {
    pub async fn next(&mut self) -> Option<NodeEventMsg> {
        loop {
            match self.inner.recv().await {
                Ok(msg) => {
                    if let Some(id) = self.filter {
                        if msg.node_id != id { continue; }
                    }
                    return Some(msg);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("NodeEventStream lagged by {n}");
                    continue;
                }
                Err(_) => return None,
            }
        }
    }
}

/// Async-friendly stream of `NodeLogChunkMsg` values pushed by VeloceCore.
pub struct NodeLogStream {
    inner:  tokio::sync::broadcast::Receiver<NodeLogChunkMsg>,
    filter: Option<Uuid>,
}

impl NodeLogStream {
    pub async fn next(&mut self) -> Option<NodeLogChunkMsg> {
        loop {
            match self.inner.recv().await {
                Ok(msg) => {
                    if let Some(id) = self.filter {
                        if msg.node_id != id { continue; }
                    }
                    return Some(msg);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("NodeLogStream lagged by {n}");
                    continue;
                }
                Err(_) => return None,
            }
        }
    }
}

impl VeloceClient {
    /// Connect to VeloceCore and perform the handshake.
    pub async fn connect(
        app_name:     &str,
        sdk_version:  &str,
        capabilities: Vec<Capability>,
    ) -> Result<Self> {
        #[cfg(not(windows))]
        {
            bail!("VeloceClient is only supported on Windows");
        }

        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ClientOptions;

            // Retry loop — Core might not be up yet
            let pipe = retry_connect(PIPE_NAME, Duration::from_secs(10)).await?;
            let (reader, writer_half) = tokio::io::split(pipe);

            let pending:   PendingMap = Arc::new(Mutex::new(HashMap::new()));
            let writer     = Arc::new(AsyncMutex::new(writer_half));
            let (event_tx, _) = tokio::sync::broadcast::channel(128);
            let event_bus: EventBus = Arc::new(event_tx);
            let (log_tx, _) = tokio::sync::broadcast::channel(512);
            let log_bus: LogBus = Arc::new(log_tx);

            // Spawn background reader
            let bg_pending   = pending.clone();
            let bg_event_bus = event_bus.clone();
            let bg_log_bus   = log_bus.clone();
            tokio::spawn(async move {
                if let Err(e) = reader_loop(reader, bg_pending, bg_event_bus, bg_log_bus).await {
                    tracing::warn!("VeloceClient reader error: {e:#}");
                }
            });

            let mut client = Self { writer, pending, event_bus, log_bus, client_id: Uuid::nil() };

            // Read the per-session PSK that Core wrote at startup.
            let psk_hash = read_psk().context("read session PSK")?;

            // Handshake
            let ack = client.request(Body::Handshake(
                veloce_ipc::message::HandshakeMsg {
                    app_name:    app_name.into(),
                    sdk_version: sdk_version.into(),
                    capabilities,
                    psk_hash: Some(psk_hash),
                }
            )).await?;

            match ack {
                Body::HandshakeAck(a) => {
                    client.client_id = a.client_id;
                    tracing::info!(?client.client_id, "VeloceCore handshake OK");
                }
                Body::Error(e) => bail!("handshake rejected: {}", e.message),
                other          => bail!("unexpected handshake reply: {:?}", other.msg_type()),
            }

            Ok(client)
        }
    }

    // ── Node management ───────────────────────────────────────────────────────

    pub async fn spawn_node(
        &mut self,
        app_name:   &str,
        executable: &str,
        args:       &[&str],
    ) -> Result<NodeSpawnedMsg> {
        self.spawn_node_with(SpawnNodeMsg {
            app_name:         app_name.into(),
            executable:       executable.into(),
            args:             args.iter().map(|s| s.to_string()).collect(),
            env:              vec![],
            limits:           None,
            auto_kill:        true,
            restart_policy:   None,
            use_appcontainer: false,
            health_check:     None,
            volume_mounts:    vec![],
            secret_refs:      vec![],
            service_name:     None,
            replica_index:    None,
        }).await
    }

    pub async fn spawn_node_with(&mut self, msg: SpawnNodeMsg) -> Result<NodeSpawnedMsg> {
        match self.request(Body::SpawnNode(msg)).await? {
            Body::NodeSpawned(s) => Ok(s),
            Body::Error(e)       => bail!("spawn failed: {}", e.message),
            other                => bail!("unexpected reply: {:?}", other.msg_type()),
        }
    }

    pub async fn kill_node(&mut self, node_id: Uuid) -> Result<u32> {
        match self.request(Body::KillNode(veloce_ipc::message::KillNodeMsg {
            node_id, exit_code: None,
        })).await? {
            Body::NodeKilled(k) => Ok(k.exit_code),
            Body::Error(e)      => bail!("kill failed: {}", e.message),
            other               => bail!("unexpected reply: {:?}", other.msg_type()),
        }
    }

    pub async fn list_nodes(&mut self) -> Result<Vec<veloce_ipc::message::NodeInfo>> {
        match self.request(Body::QueryNodes).await? {
            Body::NodeList(l) => Ok(l.nodes),
            Body::Error(e)    => bail!("query failed: {}", e.message),
            other             => bail!("unexpected reply: {:?}", other.msg_type()),
        }
    }

    // ── Registry ──────────────────────────────────────────────────────────────

    pub async fn registry_get(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.request(Body::RegistryGet { key: key.into() }).await? {
            Body::RegistryValue(v) => Ok(v.value),
            Body::Error(e)         => bail!("{}", e.message),
            other                  => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    pub async fn registry_set(&mut self, key: &str, value: Vec<u8>) -> Result<()> {
        match self.request(Body::RegistrySet { key: key.into(), value }).await? {
            Body::RegistryAck { .. } => Ok(()),
            Body::Error(e)           => bail!("{}", e.message),
            other                    => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── Node Templates ────────────────────────────────────────────────────────
    // Templates are stored in the Core registry as JSON under two key schemes:
    //   "template:__index__"  → JSON Vec<String> of template names
    //   "template:<name>"     → JSON SpawnNodeMsg

    const TMPL_INDEX: &'static str = "template:__index__";

    fn tmpl_key(name: &str) -> String { format!("template:{name}") }

    /// Persist a named spawn template into the Core registry.
    pub async fn save_template(&mut self, name: &str, spec: SpawnNodeMsg) -> Result<()> {
        let value = serde_json::to_vec(&spec)?;
        self.registry_set(&Self::tmpl_key(name), value).await?;
        let mut names = self.list_templates().await?;
        if !names.contains(&name.to_owned()) {
            names.push(name.to_owned());
            self.registry_set(Self::TMPL_INDEX, serde_json::to_vec(&names)?).await?;
        }
        Ok(())
    }

    /// Return the names of all saved templates.
    pub async fn list_templates(&mut self) -> Result<Vec<String>> {
        match self.registry_get(Self::TMPL_INDEX).await? {
            Some(b) if !b.is_empty() => Ok(serde_json::from_slice(&b)?),
            _                        => Ok(vec![]),
        }
    }

    /// Retrieve a single template by name, or `None` if it doesn't exist.
    pub async fn get_template(&mut self, name: &str) -> Result<Option<SpawnNodeMsg>> {
        match self.registry_get(&Self::tmpl_key(name)).await? {
            Some(b) if !b.is_empty() => Ok(Some(serde_json::from_slice(&b)?)),
            _                        => Ok(None),
        }
    }

    /// Delete a template by name (removes from index and clears its value).
    pub async fn delete_template(&mut self, name: &str) -> Result<()> {
        let mut names = self.list_templates().await?;
        names.retain(|n| n != name);
        self.registry_set(Self::TMPL_INDEX, serde_json::to_vec(&names)?).await?;
        self.registry_set(&Self::tmpl_key(name), vec![]).await?;
        Ok(())
    }

    /// Instantiate a node from a saved template.
    pub async fn spawn_from_template(&mut self, name: &str) -> Result<NodeSpawnedMsg> {
        let spec = self.get_template(name).await?
            .ok_or_else(|| anyhow::anyhow!("template '{}' not found", name))?;
        self.spawn_node_with(spec).await
    }

    // ── Resource usage ────────────────────────────────────────────────────────

    /// Query live CPU and memory usage for all running nodes.
    pub async fn query_node_resources(
        &mut self,
    ) -> Result<Vec<veloce_ipc::message::NodeResourceMsg>> {
        match self.request(Body::QueryNodeResources).await? {
            Body::NodeResourceList(list) => Ok(list),
            Body::Error(e)               => bail!("{}", e.message),
            other                        => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── VeloceNet ─────────────────────────────────────────────────────────────

    pub async fn register_host(
        &mut self,
        hostname:   &str,
        node_id:    Uuid,
        local_port: u16,
        ttl_secs:   u64,
    ) -> Result<()> {
        match self.request(Body::NetRegisterHost(
            veloce_ipc::message::NetRegisterHostMsg {
                hostname:   hostname.into(),
                node_id,
                local_port,
                ttl_secs,
            }
        )).await? {
            Body::NetHostRegistered { .. } => Ok(()),
            Body::Error(e)                  => bail!("{}", e.message),
            other                           => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    pub async fn resolve_host(&mut self, hostname: &str) -> Result<Option<String>> {
        match self.request(Body::NetResolve { hostname: hostname.into() }).await? {
            Body::NetResolveResult(r) => Ok(r.address),
            Body::Error(e)            => bail!("{}", e.message),
            other                     => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── Push event subscriptions ──────────────────────────────────────────────

    /// Subscribe to ALL node events pushed by VeloceCore.
    pub fn subscribe_all_events(&self) -> NodeEventStream {
        NodeEventStream { inner: self.event_bus.subscribe(), filter: None }
    }

    /// Subscribe to events for a specific node.
    pub async fn subscribe_node_events(&mut self, node_id: Uuid) -> Result<NodeEventStream> {
        match self.request(Body::SubscribeNodeEvents { node_id }).await? {
            Body::Pong => {}
            Body::Error(e) => anyhow::bail!("subscribe failed: {}", e.message),
            other => anyhow::bail!("unexpected reply: {:?}", other.msg_type()),
        }
        Ok(NodeEventStream { inner: self.event_bus.subscribe(), filter: Some(node_id) })
    }

    /// Subscribe to stdout/stderr log chunks from ALL nodes.
    pub fn subscribe_all_logs(&self) -> NodeLogStream {
        NodeLogStream { inner: self.log_bus.subscribe(), filter: None }
    }

    /// Subscribe to stdout/stderr log chunks from a specific node.
    /// Sends `SubscribeNodeLogs` to Core so it begins forwarding chunks.
    pub async fn subscribe_node_logs(&mut self, node_id: Uuid) -> Result<NodeLogStream> {
        match self.request(Body::SubscribeNodeLogs { node_id }).await? {
            Body::Pong => {}
            Body::Error(e) => anyhow::bail!("subscribe logs failed: {}", e.message),
            other => anyhow::bail!("unexpected reply: {:?}", other.msg_type()),
        }
        Ok(NodeLogStream { inner: self.log_bus.subscribe(), filter: Some(node_id) })
    }

    // ── Ping ──────────────────────────────────────────────────────────────────

    // ── Mesh P2P ──────────────────────────────────────────────────────────────

    /// Get this machine's mesh identity (join code, listen port, connected peers).
    pub async fn mesh_info(&mut self) -> Result<MeshInfoMsg> {
        match self.request(Body::MeshGetInfo).await? {
            Body::MeshInfo(m) => Ok(m),
            other => bail!("mesh_info: unexpected response {:?}", other.msg_type()),
        }
    }

    /// Connect to a remote peer using a join code from `mesh identity`.
    pub async fn mesh_connect(&mut self, join_code: &str) -> Result<MeshConnectResultMsg> {
        match self.request(Body::MeshConnect(MeshConnectMsg {
            join_code: join_code.to_owned(),
        })).await? {
            Body::MeshConnectResult(r) => Ok(r),
            other => bail!("mesh_connect: unexpected response {:?}", other.msg_type()),
        }
    }

    /// List all connected peers and their hosted .vln entries.
    pub async fn mesh_peers(&mut self) -> Result<Vec<PeerInfoMsg>> {
        match self.request(Body::MeshPeerList).await? {
            Body::MeshPeerListResult(p) => Ok(p),
            other => bail!("mesh_peers: unexpected response {:?}", other.msg_type()),
        }
    }

    /// Disconnect from a specific peer by UUID.
    pub async fn mesh_disconnect(&mut self, peer_id: Uuid) -> Result<()> {
        match self.request(Body::MeshDisconnect(MeshDisconnectMsg { peer_id })).await? {
            Body::Pong => Ok(()),
            other => bail!("mesh_disconnect: unexpected response {:?}", other.msg_type()),
        }
    }

    /// Return the last-measured round-trip latency (ms) to a connected peer.
    ///
    /// The latency is sampled by the 30-second background keepalive ping loop;
    /// returns `None` if the peer UUID is not found or if no measurement has
    /// been recorded yet (e.g. the peer connected very recently).
    pub async fn mesh_ping_peer(&mut self, peer_id: Uuid) -> Result<Option<u32>> {
        match self.request(Body::MeshPingPeer { peer_id }).await? {
            Body::MeshPingResult { latency_ms, .. } => Ok(latency_ms),
            Body::Error(e) => bail!("mesh_ping_peer: {}", e.message),
            other => bail!("mesh_ping_peer: unexpected response {:?}", other.msg_type()),
        }
    }

    pub async fn ping(&mut self) -> Result<()> {
        match self.request(Body::Ping).await? {
            Body::Pong => Ok(()),
            other      => bail!("unexpected pong: {:?}", other.msg_type()),
        }
    }

    /// Retrieve the current policy rules from the core.
    pub async fn policy_get_rules(&mut self) -> Result<PolicyRulesMsg> {
        match self.request(Body::PolicyGetRules).await? {
            Body::PolicyRulesResult(msg) => Ok(msg),
            Body::Error(e) => bail!("policy_get_rules error: {}", e.message),
            other => bail!("policy_get_rules: unexpected response {:?}", other.msg_type()),
        }
    }

    /// Ask the core to reload `veloce-policy.toml` from disk, then return the
    /// newly active rules.
    pub async fn policy_reload(&mut self) -> Result<PolicyRulesMsg> {
        match self.request(Body::PolicyReload).await? {
            Body::PolicyRulesResult(msg) => Ok(msg),
            Body::Error(e) => bail!("policy_reload error: {}", e.message),
            other => bail!("policy_reload: unexpected response {:?}", other.msg_type()),
        }
    }

    // ── Port forwarding (v1.1) ────────────────────────────────────────────────

    /// Add a TCP port-forward rule: bind `host_port` → `target_port` on the node.
    pub async fn add_port_forward(
        &mut self,
        msg: NetPortForwardMsg,
    ) -> Result<PortForwardEntry> {
        match self.request(Body::NetAddPortForward(msg)).await? {
            Body::NetPortForwardAdded(e) => Ok(e),
            Body::Error(e)               => bail!("add_port_forward: {}", e.message),
            other                        => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    /// Remove a port-forward rule by name.
    pub async fn remove_port_forward(&mut self, name: &str) -> Result<()> {
        match self.request(Body::NetRemovePortForward { name: name.into() }).await? {
            Body::Pong     => Ok(()),
            Body::Error(e) => bail!("remove_port_forward: {}", e.message),
            other          => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    /// List active port-forward rules.
    pub async fn list_port_forwards(&mut self) -> Result<Vec<PortForwardEntry>> {
        match self.request(Body::NetListPortForwards).await? {
            Body::NetPortForwardList(v) => Ok(v),
            Body::Error(e)              => bail!("list_port_forwards: {}", e.message),
            other                       => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── Named volumes (v1.2) ──────────────────────────────────────────────────

    /// Get-or-create a named volume; returns its host path.
    pub async fn volume_register(&mut self, name: &str) -> Result<VolumeRegisteredMsg> {
        match self.request(Body::VolumeRegister(
            veloce_ipc::message::VolumeRegisterMsg { name: name.into() }
        )).await? {
            Body::VolumeRegistered(v) => Ok(v),
            Body::Error(e)            => bail!("volume_register: {}", e.message),
            other                     => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    /// List all known named volumes.
    pub async fn volume_list(&mut self) -> Result<Vec<VolumeEntry>> {
        match self.request(Body::VolumeList).await? {
            Body::VolumeListResult(v) => Ok(v),
            Body::Error(e)            => bail!("volume_list: {}", e.message),
            other                     => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── Secrets (v1.2) ────────────────────────────────────────────────────────

    /// Encrypt and store a named secret in the DPAPI vault.
    pub async fn secret_set(&mut self, name: &str, plaintext: &str) -> Result<()> {
        match self.request(Body::SecretSet(
            veloce_ipc::message::SecretSetMsg { name: name.into(), plaintext: plaintext.into() }
        )).await? {
            Body::SecretSetAck { .. } => Ok(()),
            Body::Error(e)            => bail!("secret_set: {}", e.message),
            other                     => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    /// Delete a named secret from the vault.
    pub async fn secret_delete(&mut self, name: &str) -> Result<()> {
        match self.request(Body::SecretDelete { name: name.into() }).await? {
            Body::SecretDeleteAck { .. } => Ok(()),
            Body::Error(e)               => bail!("secret_delete: {}", e.message),
            other                        => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    /// List all secret names (values are never returned over IPC).
    pub async fn secret_list(&mut self) -> Result<Vec<String>> {
        match self.request(Body::SecretList).await? {
            Body::SecretListResult(v) => Ok(v),
            Body::Error(e)            => bail!("secret_list: {}", e.message),
            other                     => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── Desired state / reconciler (v1.3) ─────────────────────────────────────

    /// Apply a desired-state specification; the reconciler takes over management.
    pub async fn apply_desired_state(&mut self, spec: DesiredStateSpec) -> Result<()> {
        match self.request(Body::ApplyDesiredState(spec)).await? {
            Body::DesiredStateApplied { .. } => Ok(()),
            Body::Error(e)                   => bail!("apply_desired_state: {}", e.message),
            other                            => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    /// Query extended node status including health and service labels.
    pub async fn query_node_status(&mut self) -> Result<Vec<NodeStatusMsg>> {
        match self.request(Body::QueryNodeStatus).await? {
            Body::NodeStatusResult(v) => Ok(v),
            Body::Error(e)            => bail!("query_node_status: {}", e.message),
            other                     => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── Traffic stats ─────────────────────────────────────────────────────────

    /// Query per-tunnel (Noise) and per-.vln host byte counters from Core.
    ///
    /// Returns cumulative counters since the connection was established /
    /// hostname was registered.  Compute bytes-per-second in the caller by
    /// diffing successive snapshots using the `ts_ms` timestamp field.
    pub async fn query_traffic(&mut self) -> Result<TrafficStatsMsg> {
        match self.request(Body::TrafficQuery).await? {
            Body::TrafficStatsResult(s) => Ok(s),
            Body::Error(e) => bail!("query_traffic error: {}", e.message),
            other => bail!("query_traffic: unexpected {:?}", other.msg_type()),
        }
    }

    // ── Core send / receive ───────────────────────────────────────────────────

    /// Send a request and wait for a correlated reply (5-second timeout).
    pub async fn raw_request(&self, body: Body) -> Result<Body> { self.request(body).await }

    async fn request(&self, body: Body) -> Result<Body> {
        let env    = Envelope::new(body);
        let cid    = env.correlation_id;
        let (tx, rx) = oneshot::channel();

        self.pending.lock().insert(cid, tx);

        let frame = Codec::encode(&env, Flags::EXPECTS_ACK)
            .context("encode request")?;

        self.writer.lock().await
            .write_all(&frame).await
            .context("write to pipe")?;

        let reply = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .context("response timeout")?
            .context("sender dropped")?;

        Ok(reply)
    }
}

// ── BACKGROUND READER ─────────────────────────────────────────────────────────

#[cfg(windows)]
async fn reader_loop<R>(
    mut reader: R,
    pending:    PendingMap,
    event_bus:  EventBus,
    log_bus:    LogBus,
) -> Result<()>
where
    R: AsyncReadExt + Unpin,
{
    let mut buf = [0u8; 8192];
    let mut acc = BytesMut::with_capacity(32 * 1024);

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 { break; }
        acc.extend_from_slice(&buf[..n]);

        loop {
            match Codec::decode_safe(&mut acc) {
                Ok(Some((_hdr, env))) => {
                    if let Some(tx) = pending.lock().remove(&env.correlation_id) {
                        let _ = tx.send(env.body);
                    } else {
                        match env.body {
                            Body::NodeEvent(msg)    => { let _ = event_bus.send(msg); }
                            Body::NodeLogChunk(msg) => { let _ = log_bus.send(msg); }
                            other => tracing::debug!("unsolicited push: {:?}", other.msg_type()),
                        }
                    }
                }
                Ok(None)  => break,
                Err(e) => {
                    tracing::warn!("decode error in reader loop: {e:#}");
                    acc.clear();
                    break;
                }
            }
        }
    }

    tracing::info!("VeloceClient reader loop exited");
    Ok(())
}

// ── PSK READER ────────────────────────────────────────────────────────────────

/// Read and decode the 32-byte PSK from the file Core wrote at startup.
fn read_psk() -> Result<[u8; 32]> {
    let path = veloce_ipc::psk_path();
    let hex  = std::fs::read_to_string(&path)
        .with_context(|| format!("open PSK file {} — is VeloceCore running?", path.display()))?;
    let hex = hex.trim();
    anyhow::ensure!(hex.len() == 64, "PSK file has unexpected length ({})", hex.len());
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let nibble = std::str::from_utf8(chunk).context("PSK file not UTF-8")?;
        out[i] = u8::from_str_radix(nibble, 16).context("PSK file not hex")?;
    }
    Ok(out)
}

// ── CONNECT WITH RETRY ────────────────────────────────────────────────────────

#[cfg(windows)]
async fn retry_connect(
    pipe: &str,
    timeout: Duration,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match ClientOptions::new().open(pipe) {
            Ok(c)  => return Ok(c),
            Err(e) if e.raw_os_error() == Some(231 /* ERROR_PIPE_BUSY */) => {
                // Pipe exists but no available instances; wait
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) if e.raw_os_error() == Some(2 /* ERROR_FILE_NOT_FOUND */) => {
                // Core not up yet
                if tokio::time::Instant::now() >= deadline {
                    bail!("VeloceCore pipe not found after {}s — is the service running?", timeout.as_secs());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(e) => return Err(e).context("open named pipe"),
        }
    }
}