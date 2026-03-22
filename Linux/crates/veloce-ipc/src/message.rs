use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

// ── MESSAGE TYPE DISCRIMINANT ─────────────────────────────────────────────────

/// Every message sent over the pipe is tagged with one of these.
/// Values are stable — never renumber existing variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageType {
    // ── Heartbeat ────────────────────────────────────────────
    Ping            = 0x00,
    Pong            = 0x01,

    // ── Session lifecycle ─────────────────────────────────────
    /// Client → Core: prove identity and negotiate capabilities.
    Handshake       = 0x10,
    /// Core → Client: accepted, return assigned client_id.
    HandshakeAck    = 0x11,
    /// Either party: clean shutdown.
    Goodbye         = 0x12,

    // ── Node management ───────────────────────────────────────
    /// Client → Core: request a new isolated node.
    SpawnNode       = 0x20,
    /// Core → Client: node is live, here's its metadata.
    NodeSpawned     = 0x21,
    /// Client → Core: terminate a node by id.
    KillNode        = 0x22,
    /// Core → Client: node terminated.
    NodeKilled      = 0x23,
    /// Client → Core: list all nodes this client owns.
    QueryNodes      = 0x24,
    /// Core → Client: node list response.
    NodeList        = 0x25,
    /// Core → Client: unsolicited node status change event.
    NodeEvent       = 0x26,
    /// Client → Core: subscribe to push events for a specific node.
    SubscribeNodeEvents   = 0x27,
    /// Client → Core: cancel an event subscription.
    UnsubscribeNodeEvents = 0x28,
    /// Client → Core: subscribe to stdout/stderr log chunks for a node.
    SubscribeNodeLogs     = 0x29,
    /// Client → Core: cancel a log subscription.
    UnsubscribeNodeLogs   = 0x2A,
    /// Core → Client: push a chunk of captured stdout or stderr.
    NodeLogChunk          = 0x2B,

    // ── Registry queries ──────────────────────────────────────
    /// Client → Core: read a key from the mmap registry.
    RegistryGet     = 0x30,
    /// Core → Client: value response (or not-found).
    RegistryValue   = 0x31,
    /// Client → Core: write a key.
    RegistrySet     = 0x32,
    /// Core → Client: write acknowledged.
    RegistryAck     = 0x33,

    // ── Resource usage ────────────────────────────────────────
    /// Client → Core: query live CPU and memory usage for all nodes.
    QueryNodeResources = 0x60,
    /// Core → Client: resource usage list response.
    NodeResourceList   = 0x61,

    // ── VeloceNet: private namespace ──────────────────────────
    /// Client → Net: register a *.vln hostname → node_id mapping.
    NetRegisterHost = 0x40,
    /// Net → Client: registration confirmed.
    NetHostRegistered = 0x41,
    /// Client → Net: remove a *.vln hostname.
    NetUnregisterHost = 0x42,
    /// Client → Net: resolve a *.vln hostname to an internal address.
    NetResolve      = 0x43,
    /// Net → Client: resolution result.
    NetResolveResult = 0x44,
    /// Net → Client: push a list of all currently registered hosts.
    NetHostList     = 0x45,

    // ── Mesh (P2P multi-machine) ──────────────────────────────
    /// Client → Core: get this machine's mesh identity + peer list.
    MeshGetInfo         = 0x50,
    /// Core → Client: mesh identity response.
    MeshInfo            = 0x51,
    /// Client → Core: connect to a remote peer using a join code.
    MeshConnect         = 0x52,
    /// Core → Client: peer connection result.
    MeshConnectResult   = 0x53,
    /// Client → Core: list connected peers.
    MeshPeerList        = 0x54,
    /// Core → Client: peer list response.
    MeshPeerListResult  = 0x55,
    /// Client → Core: disconnect from a peer.
    MeshDisconnect      = 0x56,
    /// Core → Client (push): a peer went offline.
    MeshPeerGone        = 0x57,
    /// Client → Core: return the last-measured round-trip latency to a specific peer.
    MeshPingPeer        = 0x58,
    /// Core → Client: RTT result — `None` if peer UUID is not found.
    MeshPingResult      = 0x59,

    // ── Policy engine ─────────────────────────────────────────
    /// Client → Core: query the currently loaded policy rules.
    PolicyGetRules    = 0x70,
    /// Core → Client: active policy rules snapshot.
    PolicyRulesResult = 0x71,
    /// Client → Core: hot-reload policy from veloce-policy.toml.
    PolicyReload      = 0x72,

    // ── Traffic stats ──────────────────────────────────────────
    /// Client → Core: request cumulative byte counters for all tunnels and .vln hosts.
    TrafficQuery      = 0x80,
    /// Core → Client: traffic counters snapshot.
    TrafficStatsResult = 0x81,

    // ── Port forwarding (v1.1) ────────────────────────────────
    /// Client → Core: add a host-port → node-port TCP forward rule.
    NetAddPortForward    = 0x90,
    /// Core → Client: forward is live.
    NetPortForwardAdded  = 0x91,
    /// Client → Core: remove a port forward rule by name.
    NetRemovePortForward = 0x92,
    /// Client → Core: list active port forward rules.
    NetListPortForwards  = 0x93,
    /// Core → Client: port forward list response.
    NetPortForwardList   = 0x94,

    // ── Extended node status (v1.3) ───────────────────────────
    /// Client → Core: query running node status including health.
    QueryNodeStatus  = 0xA0,
    /// Core → Client: extended node status list.
    NodeStatusResult = 0xA1,

    // ── Desired state / reconciler (v1.3) ─────────────────────
    /// Client → Core: apply a full desired-state specification.
    ApplyDesiredState  = 0xB0,
    /// Core → Client: desired state accepted by reconciler.
    DesiredStateApplied = 0xB1,

    // ── Named volumes (v1.2) ──────────────────────────────────
    /// Client → Core: get-or-create a named volume, returns its host path.
    VolumeRegister   = 0xC0,
    /// Core → Client: volume path confirmed.
    VolumeRegistered = 0xC1,
    /// Client → Core: list all known volumes.
    VolumeList       = 0xC2,
    /// Core → Client: volume list response.
    VolumeListResult = 0xC3,

    // ── Secrets (v1.2) ────────────────────────────────────────
    /// Client → Core: encrypt and store a named secret (DPAPI-backed).
    SecretSet        = 0xD0,
    /// Core → Client: secret stored.
    SecretSetAck     = 0xD1,
    /// Client → Core: delete a named secret.
    SecretDelete     = 0xD2,
    /// Core → Client: secret deleted.
    SecretDeleteAck  = 0xD3,
    /// Client → Core: list known secret names (values never returned).
    SecretList       = 0xD4,
    /// Core → Client: secret name list.
    SecretListResult = 0xD5,

    // ── Error ─────────────────────────────────────────────────
    Error           = 0xFF,
}

impl TryFrom<u8> for MessageType {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        use MessageType::*;
        Ok(match v {
            0x00 => Ping,            0x01 => Pong,
            0x10 => Handshake,       0x11 => HandshakeAck,    0x12 => Goodbye,
            0x20 => SpawnNode,       0x21 => NodeSpawned,     0x22 => KillNode,
            0x23 => NodeKilled,      0x24 => QueryNodes,      0x25 => NodeList,
            0x26 => NodeEvent,
            0x27 => SubscribeNodeEvents,
            0x28 => UnsubscribeNodeEvents,
            0x29 => SubscribeNodeLogs,
            0x2A => UnsubscribeNodeLogs,
            0x2B => NodeLogChunk,
            0x30 => RegistryGet,     0x31 => RegistryValue,   0x32 => RegistrySet,
            0x33 => RegistryAck,
            0x60 => QueryNodeResources, 0x61 => NodeResourceList,
            0x40 => NetRegisterHost, 0x41 => NetHostRegistered,
            0x42 => NetUnregisterHost,0x43 => NetResolve,
            0x44 => NetResolveResult, 0x45 => NetHostList,
            0x50 => MeshGetInfo,     0x51 => MeshInfo,
            0x52 => MeshConnect,     0x53 => MeshConnectResult,
            0x54 => MeshPeerList,    0x55 => MeshPeerListResult,
            0x56 => MeshDisconnect,  0x57 => MeshPeerGone,
            0x58 => MeshPingPeer,    0x59 => MeshPingResult,
            0x70 => PolicyGetRules,  0x71 => PolicyRulesResult,
            0x72 => PolicyReload,
            0x80 => TrafficQuery,    0x81 => TrafficStatsResult,
            0x90 => NetAddPortForward,    0x91 => NetPortForwardAdded,
            0x92 => NetRemovePortForward, 0x93 => NetListPortForwards,
            0x94 => NetPortForwardList,
            0xA0 => QueryNodeStatus,      0xA1 => NodeStatusResult,
            0xB0 => ApplyDesiredState,    0xB1 => DesiredStateApplied,
            0xC0 => VolumeRegister,       0xC1 => VolumeRegistered,
            0xC2 => VolumeList,           0xC3 => VolumeListResult,
            0xD0 => SecretSet,            0xD1 => SecretSetAck,
            0xD2 => SecretDelete,         0xD3 => SecretDeleteAck,
            0xD4 => SecretList,           0xD5 => SecretListResult,
            0xFF => Error,
            other => return Err(other),
        })
    }
}

// ── FLAGS BITFIELD ────────────────────────────────────────────────────────────

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Flags: u16 {
        /// Payload is compressed (reserved for future use).
        const COMPRESSED  = 0b0000_0001;
        /// This message requires a correlated reply (use correlation_id).
        const EXPECTS_ACK = 0b0000_0010;
        /// This is an unsolicited push from Core to client.
        const PUSH        = 0b0000_0100;
        /// Emergency / priority message — process before queue.
        const URGENT      = 0b0000_1000;
    }
}

// ── ENVELOPE ─────────────────────────────────────────────────────────────────

/// The typed outer wrapper that combines a correlation ID with the payload body.
/// Serialised into the frame's variable-length payload region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// UUID linking a request to its reply.
    pub correlation_id: Uuid,
    /// The actual message body.
    pub body: Body,
}

impl Envelope {
    pub fn new(body: Body) -> Self {
        Self { correlation_id: Uuid::new_v4(), body }
    }
    pub fn reply(correlation_id: Uuid, body: Body) -> Self {
        Self { correlation_id, body }
    }
    pub fn msg_type(&self) -> MessageType {
        self.body.msg_type()
    }
}

// ── MESSAGE BODY ──────────────────────────────────────────────────────────────

/// All possible message bodies — exhaustive enum so the compiler enforces handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Body {
    // Heartbeat
    Ping,
    Pong,

    // Session
    Handshake(HandshakeMsg),
    HandshakeAck(HandshakeAckMsg),
    Goodbye { reason: Option<String> },

    // Nodes
    SpawnNode(SpawnNodeMsg),
    NodeSpawned(NodeSpawnedMsg),
    KillNode(KillNodeMsg),
    NodeKilled(NodeKilledMsg),
    QueryNodes,
    NodeList(NodeListMsg),
    NodeEvent(NodeEventMsg),
    /// Subscribe to push events for a specific node (client → Core).
    SubscribeNodeEvents { node_id: Uuid },
    /// Cancel event subscription (client → Core).
    UnsubscribeNodeEvents { node_id: Uuid },
    /// Subscribe to stdout/stderr log chunks for a node (client → Core).
    SubscribeNodeLogs { node_id: Uuid },
    /// Cancel log subscription (client → Core).
    UnsubscribeNodeLogs { node_id: Uuid },
    /// Push a chunk of captured stdout or stderr (Core → Client).
    NodeLogChunk(NodeLogChunkMsg),

    // Registry
    RegistryGet { key: String },
    RegistryValue(RegistryValueMsg),
    RegistrySet { key: String, value: Vec<u8> },
    RegistryAck { key: String },

    // Resource usage
    /// Query CPU/memory for all running nodes (client → Core).
    QueryNodeResources,
    /// Resource usage response (Core → client).
    NodeResourceList(Vec<NodeResourceMsg>),

    // VeloceNet
    NetRegisterHost(NetRegisterHostMsg),
    NetHostRegistered { hostname: String, addr: String },
    NetUnregisterHost { hostname: String },
    NetResolve { hostname: String },
    NetResolveResult(NetResolveResultMsg),
    NetHostList(Vec<NetHostEntry>),

    // Policy engine
    /// Query active policy rules (client → Core).
    PolicyGetRules,
    /// Active policy rules snapshot (Core → client).
    PolicyRulesResult(PolicyRulesMsg),
    /// Hot-reload policy from disk (client → Core). Returns PolicyRulesResult.
    PolicyReload,

    // Traffic stats
    /// Request cumulative byte counters for all tunnels and .vln hosts (client → Core).
    TrafficQuery,
    /// Traffic counters snapshot (Core → client).
    TrafficStatsResult(TrafficStatsMsg),

    // Mesh P2P
    /// Request this machine's mesh identity + peer list.
    MeshGetInfo,
    /// Mesh identity response.
    MeshInfo(MeshInfoMsg),
    /// Connect to a remote peer using a join code.
    MeshConnect(MeshConnectMsg),
    /// Peer connection result.
    MeshConnectResult(MeshConnectResultMsg),
    /// List connected peers.
    MeshPeerList,
    /// Peer list response.
    MeshPeerListResult(Vec<PeerInfoMsg>),
    /// Disconnect from a peer.
    MeshDisconnect(MeshDisconnectMsg),
    /// Push: a peer went offline.
    MeshPeerGone(MeshPeerGoneMsg),
    /// Request last-measured RTT latency to a specific peer (client → Core).
    MeshPingPeer { peer_id: Uuid },
    /// RTT measurement result (Core → client).  `latency_ms` is `None` if the
    /// peer UUID was not found or no latency sample has been recorded yet.
    MeshPingResult { peer_id: Uuid, latency_ms: Option<u32> },

    // Error
    Error(ErrorMsg),

    // ── Port forwarding (v1.1) ────────────────────────────────────────────────
    NetAddPortForward(NetPortForwardMsg),
    NetPortForwardAdded(PortForwardEntry),
    NetRemovePortForward { name: String },
    NetListPortForwards,
    NetPortForwardList(Vec<PortForwardEntry>),

    // ── Extended node status (v1.3) ───────────────────────────────────────────
    QueryNodeStatus,
    NodeStatusResult(Vec<NodeStatusMsg>),

    // ── Desired state (v1.3) ──────────────────────────────────────────────────
    ApplyDesiredState(DesiredStateSpec),
    DesiredStateApplied { name: String },

    // ── Volumes (v1.2) ────────────────────────────────────────────────────────
    VolumeRegister(VolumeRegisterMsg),
    VolumeRegistered(VolumeRegisteredMsg),
    VolumeList,
    VolumeListResult(Vec<VolumeEntry>),

    // ── Secrets (v1.2) ────────────────────────────────────────────────────────
    SecretSet(SecretSetMsg),
    SecretSetAck { name: String },
    SecretDelete { name: String },
    SecretDeleteAck { name: String },
    SecretList,
    SecretListResult(Vec<String>),
}

impl Body {
    pub fn msg_type(&self) -> MessageType {
        use Body::*;
        match self {
            Ping                   => MessageType::Ping,
            Pong                   => MessageType::Pong,
            Handshake(_)           => MessageType::Handshake,
            HandshakeAck(_)        => MessageType::HandshakeAck,
            Goodbye { .. }         => MessageType::Goodbye,
            SpawnNode(_)           => MessageType::SpawnNode,
            NodeSpawned(_)         => MessageType::NodeSpawned,
            KillNode(_)            => MessageType::KillNode,
            NodeKilled(_)          => MessageType::NodeKilled,
            QueryNodes             => MessageType::QueryNodes,
            NodeList(_)            => MessageType::NodeList,
            NodeEvent(_)               => MessageType::NodeEvent,
            SubscribeNodeEvents { .. }  => MessageType::SubscribeNodeEvents,
            UnsubscribeNodeEvents { .. } => MessageType::UnsubscribeNodeEvents,
            SubscribeNodeLogs { .. }    => MessageType::SubscribeNodeLogs,
            UnsubscribeNodeLogs { .. }  => MessageType::UnsubscribeNodeLogs,
            NodeLogChunk(_)             => MessageType::NodeLogChunk,
            RegistryGet { .. }     => MessageType::RegistryGet,
            RegistryValue(_)       => MessageType::RegistryValue,
            RegistrySet { .. }     => MessageType::RegistrySet,
            RegistryAck { .. }     => MessageType::RegistryAck,
            QueryNodeResources     => MessageType::QueryNodeResources,
            NodeResourceList(_)    => MessageType::NodeResourceList,
            NetRegisterHost(_)     => MessageType::NetRegisterHost,
            NetHostRegistered{..}  => MessageType::NetHostRegistered,
            NetUnregisterHost{..}  => MessageType::NetUnregisterHost,
            NetResolve{..}         => MessageType::NetResolve,
            NetResolveResult(_)    => MessageType::NetResolveResult,
            NetHostList(_)         => MessageType::NetHostList,
            PolicyGetRules         => MessageType::PolicyGetRules,
            PolicyRulesResult(_)   => MessageType::PolicyRulesResult,
            PolicyReload           => MessageType::PolicyReload,
            TrafficQuery           => MessageType::TrafficQuery,
            TrafficStatsResult(_)  => MessageType::TrafficStatsResult,
            MeshGetInfo            => MessageType::MeshGetInfo,
            MeshInfo(_)            => MessageType::MeshInfo,
            MeshConnect(_)         => MessageType::MeshConnect,
            MeshConnectResult(_)   => MessageType::MeshConnectResult,
            MeshPeerList           => MessageType::MeshPeerList,
            MeshPeerListResult(_)  => MessageType::MeshPeerListResult,
            MeshDisconnect(_)      => MessageType::MeshDisconnect,
            MeshPeerGone(_)        => MessageType::MeshPeerGone,
            MeshPingPeer { .. }    => MessageType::MeshPingPeer,
            MeshPingResult { .. }  => MessageType::MeshPingResult,
            Error(_)               => MessageType::Error,
            // Port forwarding
            NetAddPortForward(_)       => MessageType::NetAddPortForward,
            NetPortForwardAdded(_)     => MessageType::NetPortForwardAdded,
            NetRemovePortForward{..}   => MessageType::NetRemovePortForward,
            NetListPortForwards        => MessageType::NetListPortForwards,
            NetPortForwardList(_)      => MessageType::NetPortForwardList,
            // Extended status
            QueryNodeStatus            => MessageType::QueryNodeStatus,
            NodeStatusResult(_)        => MessageType::NodeStatusResult,
            // Desired state
            ApplyDesiredState(_)       => MessageType::ApplyDesiredState,
            DesiredStateApplied{..}    => MessageType::DesiredStateApplied,
            // Volumes
            VolumeRegister(_)          => MessageType::VolumeRegister,
            VolumeRegistered(_)        => MessageType::VolumeRegistered,
            VolumeList                 => MessageType::VolumeList,
            VolumeListResult(_)        => MessageType::VolumeListResult,
            // Secrets
            SecretSet(_)               => MessageType::SecretSet,
            SecretSetAck{..}           => MessageType::SecretSetAck,
            SecretDelete{..}           => MessageType::SecretDelete,
            SecretDeleteAck{..}        => MessageType::SecretDeleteAck,
            SecretList                 => MessageType::SecretList,
            SecretListResult(_)        => MessageType::SecretListResult,
        }
    }
}

// ── CONCRETE MESSAGE TYPES ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeMsg {
    /// The app name (e.g. "velocefocus").
    pub app_name: String,
    /// Semver of the calling SDK.
    pub sdk_version: String,
    /// Requested capabilities (node spawning, net registration, …).
    pub capabilities: Vec<Capability>,
    /// Optional pre-shared key for elevated trust.
    pub psk_hash: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeAckMsg {
    /// Assigned client ID for this session.
    pub client_id: Uuid,
    /// Core version.
    pub core_version: String,
    /// Which of the requested capabilities were granted.
    pub granted: Vec<Capability>,
    /// VeloceCore boot timestamp.
    pub core_started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Capability {
    SpawnNodes,
    KillNodes,
    RegistryRead,
    RegistryWrite,
    NetRegister,
    NetResolve,
    /// Connect/disconnect mesh peers and manage the P2P overlay.
    MeshManage,
    /// Hot-reload the policy file from disk.
    PolicyAdmin,
    /// Read secret names (list); secrets values are never returned over IPC.
    SecretsRead,
    /// Set or delete named secrets in the vault.
    SecretsWrite,
    /// Add / remove TCP port-forward rules (v1.1).
    NetPortForward,
    /// Apply a desired-state spec and trigger the reconciler (v1.3).
    DesiredStateManage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnNodeMsg {
    /// Human name for this node (used in the registry and the dashboard).
    pub app_name: String,
    /// Executable path to launch under the Job Object.
    pub executable: String,
    /// Arguments passed to the child process.
    pub args: Vec<String>,
    /// Environment variables (merged with a safe baseline).
    pub env: Vec<(String, String)>,
    /// Optional resource limits.
    pub limits: Option<NodeLimits>,
    /// If true, the node is automatically killed when the client disconnects.
    pub auto_kill: bool,
    /// Optional restart-on-crash policy.
    pub restart_policy: Option<RestartPolicy>,
    /// If true, spawn the process inside a Windows AppContainer (filesystem + network
    /// sandbox).  The container is created fresh per-node and deleted on exit.
    /// Falls back to a standard Job-Object spawn if AppContainer creation fails.
    #[serde(default)]
    pub use_appcontainer: bool,

    // ── v1.1 additions ────────────────────────────────────────────────────────
    /// Optional health-check configuration. The health loop probes this after
    /// the node starts; transitions to Healthy / Unhealthy update NodeTable.
    #[serde(default)]
    pub health_check: Option<HealthCheck>,

    // ── v1.2 additions ────────────────────────────────────────────────────────
    /// Named volume or bind-mount entries to inject as env vars at spawn.
    /// Each entry injects `VELOCE_VOLUME_{env_suffix}=<resolved_path>`.
    #[serde(default)]
    pub volume_mounts: Vec<VolumeMountSpec>,

    /// Secrets to decrypt and inject as environment variables at spawn.
    /// Each entry maps a vault secret name to an env var name.
    #[serde(default)]
    pub secret_refs: Vec<SecretRef>,

    // ── v1.3 additions ────────────────────────────────────────────────────────
    /// Which compose service this node belongs to (set by reconciler).
    #[serde(default)]
    pub service_name: Option<String>,

    /// Replica index within the service (set by reconciler, 0-based).
    #[serde(default)]
    pub replica_index: Option<u32>,
}

/// Automatic restart policy for nodes that exit unexpectedly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartPolicy {
    /// Maximum number of restart attempts before giving up.
    pub max_restarts: u32,
    /// Initial back-off delay in seconds. Doubles each attempt.
    pub base_delay_secs: u64,
    /// Maximum back-off delay in seconds.
    pub max_delay_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLimits {
    /// CPU throttle in percent (1–100). None = unlimited.
    pub cpu_pct: Option<u32>,
    /// Working-set cap in megabytes. None = unlimited.
    pub mem_mb: Option<u64>,
    /// Wall-clock lifetime. None = unlimited.
    pub max_lifetime_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpawnedMsg {
    pub node_id: Uuid,
    pub pid: u32,
    /// Named pipe the node is listening on (Core-assigned).
    pub node_pipe: String,
    pub spawned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KillNodeMsg {
    pub node_id: Uuid,
    /// Exit code to inject. None = SIGTERM equivalent (TerminateJobObject).
    pub exit_code: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeKilledMsg {
    pub node_id: Uuid,
    pub exit_code: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeListMsg {
    pub nodes: Vec<NodeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: Uuid,
    pub app_name: String,
    pub pid: u32,
    pub status: NodeStatus,
    pub spawned_at: DateTime<Utc>,
    pub node_pipe: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Crashed { exit_code: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEventMsg {
    pub node_id: Uuid,
    pub event: NodeEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeEvent {
    Started { pid: u32 },
    Exited { exit_code: u32 },
    MemThresholdExceeded { current_mb: u64, limit_mb: u64 },
    CpuThrottled { current_pct: u32 },
    LifetimeExpired,
    /// Node crashed and Core is scheduling a restart.
    Restarting { attempt: u32, delay_secs: u64 },
    /// Health check probe succeeded and the node has transitioned to Healthy.
    HealthCheckPassed,
    /// Health check probe failed. `probe` is a human-readable description of
    /// the probe type; `reason` explains the failure.
    HealthCheckFailed { probe: String, reason: String },
}

/// Which output stream a log chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogStream { Stdout, Stderr }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLogChunkMsg {
    pub node_id: Uuid,
    pub stream:  LogStream,
    /// Raw bytes captured from the process — typically UTF-8 text.
    pub data:    Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryValueMsg {
    pub key: String,
    pub value: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetRegisterHostMsg {
    /// The *.vln hostname to claim (e.g. "myapp.vln").
    pub hostname: String,
    /// The node_id whose pipe handles this traffic.
    pub node_id: Uuid,
    /// TCP port the node is actually listening on locally.
    pub local_port: u16,
    /// TTL in seconds. 0 = permanent (until unregister or node dies).
    pub ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetResolveResultMsg {
    pub hostname: String,
    /// 127.0.0.1:<local_port> if found.
    pub address: Option<String>,
    pub node_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetHostEntry {
    pub hostname: String,
    pub node_id: Uuid,
    pub local_port: u16,
    pub registered_at: DateTime<Utc>,
    pub ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeResourceMsg {
    pub node_id:   Uuid,
    /// Cumulative CPU time (kernel + user) in milliseconds.
    pub cpu_ms:    u64,
    /// Peak memory used by the job's processes in bytes (0 if unavailable).
    pub mem_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMsg {
    pub code: ErrorCode,
    pub message: String,
    /// The correlation_id of the request that caused this error, if applicable.
    pub context_id: Option<Uuid>,
}

// ── MESH P2P MESSAGE TYPES ────────────────────────────────────────────────────

/// Information about this machine's mesh identity and peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshInfoMsg {
    pub machine_id:   Uuid,
    /// Base64 join code that can be given to another machine.
    pub join_code:    String,
    /// TCP port the mesh server is listening on.
    pub listen_port:  u16,
    pub peers:        Vec<PeerInfoMsg>,
}

/// Snapshot of a connected peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfoMsg {
    pub peer_id:          Uuid,
    pub peer_name:        String,
    /// Unix timestamp when the connection was established.
    pub connected_since:  u64,
    pub latency_ms:       u32,
    /// .vln hostnames hosted on that peer.
    pub remote_hosts:     Vec<String>,
}

/// Connect to a remote peer using a one-time join code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshConnectMsg {
    pub join_code: String,
}

/// Result of a successful peer connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshConnectResultMsg {
    pub peer_id:   Uuid,
    pub peer_name: String,
}

/// Disconnect from a specific peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshDisconnectMsg {
    pub peer_id: Uuid,
}

/// Push notification: a peer disconnected or went offline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshPeerGoneMsg {
    pub peer_id:   Uuid,
    pub peer_name: String,
}

// ── POLICY ENGINE MESSAGE TYPES ───────────────────────────────────────────────

/// A single per-app RBAC rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRuleMsg {
    /// Verified executable path pattern (takes precedence over `app` when set).
    #[serde(default)]
    pub exe:   Option<String>,
    /// App name this rule applies to. `"*"` matches any app.
    pub app:   String,
    /// Capabilities explicitly allowed. `None` means "not specified".
    pub allow: Option<Vec<String>>,
    /// Capabilities explicitly denied. `None` means "not specified".
    pub deny:  Option<Vec<String>>,
}

/// A mesh gossip ACL entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshAclMsg {
    /// Hostname pattern (exact or `"*.suffix"`).
    pub hostname:  String,
    /// Restrict to gossip from this peer name. `None` = any peer.
    pub from_peer: Option<String>,
    /// `"allow"` or `"deny"`.
    pub effect:    String,
}

/// Snapshot of the currently loaded policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRulesMsg {
    /// Default effect when no rule matches: `"allow"` (default) or `"deny"`.
    pub default_effect: String,
    pub rules:     Vec<PolicyRuleMsg>,
    pub mesh_acls: Vec<MeshAclMsg>,
}

// ── TRAFFIC STATS MESSAGE TYPES ───────────────────────────────────────────────

/// Cumulative byte counters for a single encrypted Noise tunnel to a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelTrafficMsg {
    pub peer_id:   Uuid,
    pub peer_name: String,
    /// Total bytes written to the Noise transport since connection established.
    pub tx_bytes:  u64,
    /// Total bytes read from the Noise transport since connection established.
    pub rx_bytes:  u64,
}

/// Cumulative bytes proxied through the SOCKS5 forwarder for a `.vln` hostname.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostTrafficMsg {
    pub hostname:      String,
    /// Total bytes proxied via SOCKS5 since this hostname was registered.
    pub bytes_proxied: u64,
}

/// Snapshot of traffic counters across all tunnels and registered `.vln` hosts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrafficStatsMsg {
    pub tunnels: Vec<TunnelTrafficMsg>,
    pub hosts:   Vec<HostTrafficMsg>,
    /// Unix epoch milliseconds at the time this snapshot was taken.
    pub ts_ms:   u64,
}

// ── HEALTH CHECKS (v1.1) ─────────────────────────────────────────────────────

/// How the health loop should probe a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub probe: HealthProbe,
    /// Seconds between probe attempts. Default: 10.
    #[serde(default = "default_hc_interval")]
    pub interval_secs: u64,
    /// Seconds before a single probe is declared timed out. Default: 5.
    #[serde(default = "default_hc_timeout")]
    pub timeout_secs: u64,
    /// Consecutive successes required to enter Healthy state. Default: 1.
    #[serde(default = "default_hc_success_threshold")]
    pub success_threshold: u32,
    /// Consecutive failures required to enter Unhealthy state. Default: 3.
    #[serde(default = "default_hc_failure_threshold")]
    pub failure_threshold: u32,
}

fn default_hc_interval()          -> u64 { 10 }
fn default_hc_timeout()           -> u64 { 5 }
fn default_hc_success_threshold() -> u32 { 1 }
fn default_hc_failure_threshold() -> u32 { 3 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthProbe {
    /// HTTP GET — connects to 127.0.0.1:port, sends a minimal HTTP/1.0 request,
    /// considers any 2xx response as healthy.
    Http { port: u16, path: String },
    /// TCP connect to 127.0.0.1:port succeeds = healthy.
    Tcp  { port: u16 },
    /// Run a command; exit code 0 = healthy.
    Exec { command: String, args: Vec<String> },
}

/// Observed health state of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HealthStatus {
    /// No health check configured, or first probe not yet run.
    #[default]
    Unknown,
    /// Node just started; grace period before probing begins.
    Starting,
    /// Node is passing health checks.
    Healthy,
    /// Node is failing health checks.
    Unhealthy,
}

// ── VOLUME MOUNTS (v1.2) ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMountSpec {
    /// Named volume (managed by VolumeRegistry) or an absolute bind-mount path.
    pub source: VolumeMountSource,
    /// Suffix for the injected env var: `VELOCE_VOLUME_{env_suffix}`.
    pub env_suffix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VolumeMountSource {
    /// A named volume managed by Veloce (created on demand).
    Named(String),
    /// A host filesystem path exposed directly to the node.
    BindPath(String),
}

// ── SECRET REFERENCES (v1.2) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRef {
    /// Name of the secret in the SecretsVault.
    pub secret_name: String,
    /// Environment variable to inject the decrypted plaintext value into.
    pub env_var: String,
}

// ── PORT FORWARDING (v1.1) ────────────────────────────────────────────────────

/// Request to add a TCP port-forward rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetPortForwardMsg {
    /// Unique rule name (e.g. `"web:8080"`).
    pub name: String,
    /// Host-side port to bind on `0.0.0.0`.
    pub host_port: u16,
    /// Node-side port to forward to on `127.0.0.1`.
    pub target_port: u16,
    /// Owning node (informational; not enforced).
    pub node_id: Option<Uuid>,
}

/// A live port-forward rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForwardEntry {
    pub name: String,
    pub host_port: u16,
    pub target_port: u16,
    pub node_id: Option<Uuid>,
}

// ── EXTENDED NODE STATUS (v1.3) ───────────────────────────────────────────────

/// Extended node info including health state and compose metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatusMsg {
    pub node_id:       Uuid,
    pub app_name:      String,
    pub pid:           u32,
    pub health:        HealthStatus,
    pub spawned_at:    DateTime<Utc>,
    /// Compose service this node belongs to, if any.
    pub service_name:  Option<String>,
    /// Replica index within the service (0-based), if managed by reconciler.
    pub replica_index: Option<u32>,
}

// ── DESIRED STATE (v1.3) ──────────────────────────────────────────────────────

/// Full desired-state specification submitted to the reconciler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesiredStateSpec {
    /// Unique name for this stack (e.g. `"my-app"`).
    pub name: String,
    pub services: Vec<ServiceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub service_name:    String,
    /// Template for spawning replicas of this service.
    pub spawn_msg:       SpawnNodeMsg,
    /// Desired replica count.
    pub replicas:        u32,
    pub update_strategy: UpdateStrategy,
    /// Services that must be ready before this one starts.
    pub depends_on:      Vec<DependsOnSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateStrategy {
    /// Stop all old replicas, then start new ones.
    Recreate,
    /// Replace one replica at a time; wait for Healthy before continuing.
    RollingUpdate { max_unavailable: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependsOnSpec {
    pub service_name: String,
    pub condition: DependsOnCondition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DependsOnCondition {
    /// Dependency just needs at least one running node.
    ServiceStarted,
    /// Dependency must have at least one Healthy node.
    ServiceHealthy,
}

// ── VOLUME IPC (v1.2) ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeRegisterMsg {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeRegisteredMsg {
    pub name: String,
    /// Absolute host path where the volume data lives.
    pub host_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeEntry {
    pub name: String,
    pub host_path: String,
    pub created_at: DateTime<Utc>,
}

// ── SECRET IPC (v1.2) ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretSetMsg {
    pub name: String,
    /// Plaintext value — Core encrypts this with DPAPI before writing to disk.
    /// The plaintext is held in memory only for the duration of the IPC call.
    pub plaintext: String,
}

// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum ErrorCode {
    Unknown           = 0,
    InvalidMessage    = 1,
    Unauthorized      = 2,
    NotFound          = 3,
    AlreadyExists     = 4,
    ResourceExhausted = 5,
    NodeStartFailed   = 6,
    RegistryFull      = 7,
    NetNameConflict   = 8,
    ProtocolMismatch  = 9,
    InternalError     = 10,
    /// A policy rule denied the requested operation.
    PolicyDenied      = 11,
}