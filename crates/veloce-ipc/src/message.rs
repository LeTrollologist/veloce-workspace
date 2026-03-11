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

    // ── Registry queries ──────────────────────────────────────
    /// Client → Core: read a key from the mmap registry.
    RegistryGet     = 0x30,
    /// Core → Client: value response (or not-found).
    RegistryValue   = 0x31,
    /// Client → Core: write a key.
    RegistrySet     = 0x32,
    /// Core → Client: write acknowledged.
    RegistryAck     = 0x33,

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

    // ── Error ─────────────────────────────────────────────────
    Error           = 0xFF,
}

impl TryFrom<u8> for MessageType {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        use MessageType::*;
        Ok(match v {
            0x00 => Ping,            0x01 => Pong,
            0x10 => Handshake,       0x11 => HandshakeAck,    0x12 => Goodbye,
            0x20 => SpawnNode,       0x21 => NodeSpawned,     0x22 => KillNode,
            0x23 => NodeKilled,      0x24 => QueryNodes,      0x25 => NodeList,
            0x26 => NodeEvent,
            0x30 => RegistryGet,     0x31 => RegistryValue,   0x32 => RegistrySet,
            0x33 => RegistryAck,
            0x40 => NetRegisterHost, 0x41 => NetHostRegistered,
            0x42 => NetUnregisterHost,0x43 => NetResolve,
            0x44 => NetResolveResult, 0x45 => NetHostList,
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

    // Registry
    RegistryGet { key: String },
    RegistryValue(RegistryValueMsg),
    RegistrySet { key: String, value: Vec<u8> },
    RegistryAck { key: String },

    // VeloceNet
    NetRegisterHost(NetRegisterHostMsg),
    NetHostRegistered { hostname: String, addr: String },
    NetUnregisterHost { hostname: String },
    NetResolve { hostname: String },
    NetResolveResult(NetResolveResultMsg),
    NetHostList(Vec<NetHostEntry>),

    // Error
    Error(ErrorMsg),
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
            NodeEvent(_)           => MessageType::NodeEvent,
            RegistryGet { .. }     => MessageType::RegistryGet,
            RegistryValue(_)       => MessageType::RegistryValue,
            RegistrySet { .. }     => MessageType::RegistrySet,
            RegistryAck { .. }     => MessageType::RegistryAck,
            NetRegisterHost(_)     => MessageType::NetRegisterHost,
            NetHostRegistered{..}  => MessageType::NetHostRegistered,
            NetUnregisterHost{..}  => MessageType::NetUnregisterHost,
            NetResolve{..}         => MessageType::NetResolve,
            NetResolveResult(_)    => MessageType::NetResolveResult,
            NetHostList(_)         => MessageType::NetHostList,
            Error(_)               => MessageType::Error,
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
pub struct ErrorMsg {
    pub code: ErrorCode,
    pub message: String,
    /// The correlation_id of the request that caused this error, if applicable.
    pub context_id: Option<Uuid>,
}

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
}