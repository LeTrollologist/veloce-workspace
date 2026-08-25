/*!
Shared state threaded through every subsystem.
Constructed once at startup; cloned (Arc) everywhere.
*/

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use anyhow::Context;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::policy::PolicyEngine;
use crate::registry::Registry;
use crate::job::{NodeHandle, NodeEventMsg};
use crate::volume::VolumeRegistry;
use crate::secrets::SecretsVault;
use crate::reconciler::Reconciler;
use veloce_ipc::message::{HealthCheck, NodeLogChunkMsg, HealthStatus};
use veloce_net::{NetRegistry, PortForwardTable};
use veloce_mesh::{MeshState, DEFAULT_MESH_PORT};

pub struct CoreState {
    registry:     Registry,
    node_table:   Arc<NodeTable>,
    net_registry: Arc<NetRegistry>,
    /// Optional P2P mesh state.  `None` if the mesh server failed to bind.
    pub mesh:     Option<Arc<MeshState>>,
    /// Policy engine (capability RBAC + mesh ACL).  Always present; file-absent
    /// means permissive defaults.
    pub policy:   Arc<PolicyEngine>,
    shutdown:     AtomicBool,
    /// Per-session pre-shared key.  32 truly random bytes from OsRng, written to
    /// the PSK file at startup; every connecting client must echo them.
    psk:          [u8; 32],

    // ── v1.1 ──────────────────────────────────────────────────────────────────
    /// Active TCP port-forward rules (host → node port mappings).
    pub port_forward_table: Arc<PortForwardTable>,

    // ── v1.2 ──────────────────────────────────────────────────────────────────
    /// Named volume registry persisted in the data directory.
    pub volume_registry: Arc<VolumeRegistry>,
    /// DPAPI-backed secret vault.
    pub secrets_vault: Arc<SecretsVault>,

    // ── v1.3 ──────────────────────────────────────────────────────────────────
    /// Desired-state reconciler.  The background task is started in `run_core`.
    pub reconciler: Arc<Reconciler>,

    // ── v2.1 ──────────────────────────────────────────────────────────────────
    /// Layer-7 HTTP Ingress router.
    pub ingress_router: Arc<veloce_net::IngressRouter>,

    // ── v3.1 ──────────────────────────────────────────────────────────────────
    /// Horizontal Process Autoscaler (HPA) engine.
    pub autoscale: Arc<crate::autoscale::AutoscaleEngine>,
    /// CronJob task scheduler.
    pub cron: Arc<crate::cron::CronScheduler>,

    // ── v3.2 ──────────────────────────────────────────────────────────────────
    /// Ingress TLS engine with dynamic SNI and self-signed CA for *.vln.
    pub tls_manager: Arc<veloce_net::TlsManager>,

    // ── v3.4 ──────────────────────────────────────────────────────────────────
    /// Veloce Hub application catalog and deployment engine.
    pub hub: Arc<crate::hub::HubCatalogEngine>,

    // ── v3.8 (Enterprise OIDC SSO) ────────────────────────────
    pub oidc: Arc<crate::oidc::OidcEngine>,

    // ── v4.0 (Bridge to Cloud) ────────────────────────────────
    pub bridge: Arc<crate::bridge::BridgeEngine>,

    // ── v4.1 (Zero-Trust Team Share) ──────────────────────────
    pub share: Arc<crate::share::ShareEngine>,

    // ── v4.2 (OpenTelemetry Distributed Tracing) ──────────────
    pub otel: Arc<crate::otel::OtelEngine>,
}

impl CoreState {
    pub fn new() -> anyhow::Result<Self> {
        let (dir, registry) = Self::open_registry_and_dir()?;

        let psk = generate_and_persist_psk()?;
        let net_registry = Arc::new(NetRegistry::new());

        // Load the policy engine (permissive if file absent).
        let policy = PolicyEngine::load_or_default(dir.join("veloce-policy.toml"));

        // Build mesh identity and state, wiring the policy ACL callback and
        // passing the operator-configured STUN / mesh-mode / gossip settings.
        let mesh = match veloce_mesh::identity::MachineIdentity::load_or_create(&dir) {
            Ok(id) => {
                let policy_acl  = Arc::clone(&policy);
                let acl_fn: veloce_mesh::peer::AclFn = Arc::new(move |hostname, peer| {
                    policy_acl.check_mesh_acl(hostname, peer)
                });
                // Snapshot policy values needed by the mesh layer.
                let (stun_servers, mesh_mode, gossip_secs) = {
                    let cfg = policy.config();
                    (
                        cfg.stun_servers.clone(),
                        match cfg.mesh_mode {
                            crate::policy::MeshMode::Auto    => veloce_mesh::MeshMode::Auto,
                            crate::policy::MeshMode::LanOnly => veloce_mesh::MeshMode::LanOnly,
                            crate::policy::MeshMode::Wan     => veloce_mesh::MeshMode::Wan,
                        },
                        cfg.gossip_interval_secs,
                    )
                };
                Some(MeshState::new(
                    id,
                    DEFAULT_MESH_PORT,
                    Arc::clone(&net_registry),
                    Some(acl_fn),
                    stun_servers,
                    mesh_mode,
                    gossip_secs,
                ))
            }
            Err(e) => {
                tracing::warn!("mesh identity init failed (mesh disabled): {e}");
                None
            }
        };

        let port_forward_table = Arc::new(PortForwardTable::new());
        let ingress_router     = Arc::new(veloce_net::IngressRouter::new());
        let volume_registry    = VolumeRegistry::new(&dir);
        let secrets_vault      = SecretsVault::new(&dir);
        let node_table         = Arc::new(NodeTable::new());

        // Reconciler needs CoreState — it is wired up after construction.
        // We use a temporary Arc<Reconciler> with a placeholder; the real
        // CoreState is passed via `Reconciler::new` call in main.rs after
        // Arc::new(CoreState { … }) is created.  To break the cycle we use
        // a OnceLock-based approach: Reconciler takes Arc<CoreState> in its
        // `run` call, not at construction.
        let reconciler = Arc::new(crate::reconciler::Reconciler::new_detached());
        let autoscale  = crate::autoscale::AutoscaleEngine::new();
        let cron       = crate::cron::CronScheduler::new();
        let tls_manager = Arc::new(veloce_net::TlsManager::new_self_signed()?);
        let hub        = crate::hub::HubCatalogEngine::new(&dir);
        let oidc       = Arc::new(crate::oidc::OidcEngine::new(&dir));
        let bridge     = Arc::new(crate::bridge::BridgeEngine::new());
        let share      = Arc::new(crate::share::ShareEngine::new());
        let otel       = Arc::new(crate::otel::OtelEngine::new());

        Ok(Self {
            registry,
            node_table,
            net_registry,
            mesh,
            policy,
            shutdown:     AtomicBool::new(false),
            psk,
            port_forward_table,
            ingress_router,
            volume_registry,
            secrets_vault,
            reconciler,
            autoscale,
            cron,
            tls_manager,
            hub,
            oidc,
            bridge,
            share,
            otel,
        })
    }

    pub fn registry(&self)            -> &Registry                    { &self.registry }
    pub fn node_table(&self)          -> &Arc<NodeTable>              { &self.node_table }
    pub fn net_registry(&self)        -> &Arc<NetRegistry>            { &self.net_registry }
    pub fn port_forward_table(&self)  -> &Arc<PortForwardTable>       { &self.port_forward_table }
    pub fn ingress_router(&self)      -> &Arc<veloce_net::IngressRouter> { &self.ingress_router }
    pub fn volume_registry(&self)     -> &Arc<VolumeRegistry>         { &self.volume_registry }
    pub fn secrets_vault(&self)       -> &Arc<SecretsVault>           { &self.secrets_vault }
    pub fn autoscale(&self)           -> &Arc<crate::autoscale::AutoscaleEngine> { &self.autoscale }
    pub fn cron(&self)                -> &Arc<crate::cron::CronScheduler> { &self.cron }
    pub fn tls_manager(&self)         -> &Arc<veloce_net::TlsManager> { &self.tls_manager }
    pub fn hub(&self)                 -> &Arc<crate::hub::HubCatalogEngine> { &self.hub }
    pub fn oidc(&self)                -> &Arc<crate::oidc::OidcEngine> { &self.oidc }
    pub fn reconciler(&self)          -> &Arc<Reconciler>             { &self.reconciler }
    /// The current session's PSK bytes.  Clients must send these verbatim.
    pub fn psk(&self) -> &[u8; 32] { &self.psk }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
}

/// Generate 32 cryptographically random bytes via OsRng, write hex to the PSK
/// file, and return the raw bytes.  The file is recreated on every Core startup
/// — any SDK connections from the previous session are thereby invalidated.
///
/// Using OsRng directly (rather than two UUIDs) gives the full 256 bits of
/// entropy without the 12 fixed version/variant bits that UUIDs sacrifice.
fn generate_and_persist_psk() -> anyhow::Result<[u8; 32]> {
    use rand::RngCore;

    let mut psk = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut psk);

    let hex: String = psk.iter().map(|b| format!("{b:02x}")).collect();

    let path = veloce_ipc::psk_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &hex)
        .map_err(|e| anyhow::anyhow!("write PSK to {}: {e}", path.display()))?;

    tracing::info!("session PSK written to {}", path.display());
    Ok(psk)
}

/// In-memory live node table (complementing the durable mmap registry).
pub struct NodeTable {
    nodes:           RwLock<HashMap<Uuid, NodeHandle>>,
    /// Per-node health status set by the health-check loop (v1.1).
    health_statuses: RwLock<HashMap<Uuid, HealthStatus>>,
    /// Maps node_id → (service_name, replica_index) for reconciler-managed nodes (v1.3).
    service_labels:  RwLock<HashMap<Uuid, (String, u32)>>,
}

impl NodeTable {
    pub fn new() -> Self {
        Self {
            nodes:           RwLock::new(HashMap::new()),
            health_statuses: RwLock::new(HashMap::new()),
            service_labels:  RwLock::new(HashMap::new()),
        }
    }

    pub fn insert(&self, handle: NodeHandle) {
        self.nodes.write().insert(handle.node_id, handle);
    }

    pub fn remove(&self, id: Uuid) -> Option<NodeHandle> {
        let handle = self.nodes.write().remove(&id);
        if handle.is_some() {
            self.health_statuses.write().remove(&id);
            self.service_labels.write().remove(&id);
        }
        handle
    }

    #[allow(dead_code)]
    pub fn get_pid(&self, id: Uuid) -> Option<u32> {
        self.nodes.read().get(&id).map(|h| h.pid)
    }

    // ── Health tracking (v1.1) ────────────────────────────────────────────

    #[allow(dead_code)]
    pub fn set_health(&self, id: Uuid, status: HealthStatus) {
        self.health_statuses.write().insert(id, status);
    }

    pub fn get_health(&self, id: Uuid) -> HealthStatus {
        self.health_statuses.read().get(&id).copied().unwrap_or_default()
    }

    // ── Service label tracking (v1.3) ─────────────────────────────────────

    pub fn set_service_label(&self, id: Uuid, service_name: String, replica_index: u32) {
        self.service_labels.write().insert(id, (service_name, replica_index));
    }

    #[allow(dead_code)]
    pub fn get_service_label(&self, id: Uuid) -> Option<(String, u32)> {
        self.service_labels.read().get(&id).cloned()
    }

    // ── Bulk queries ──────────────────────────────────────────────────────

    /// Query live CPU and memory usage for every node.
    /// Returns `(node_id, pid, cpu_ms, mem_bytes)` per node.
    /// Holds the read lock only for the duration of the call.
    pub fn query_all_resources(&self) -> Vec<(Uuid, u32, u64, u64)> {
        self.nodes.read().values().map(|h| {
            let (cpu_ms, mem_bytes) = h.query_resources();
            (h.node_id, h.pid, cpu_ms, mem_bytes)
        }).collect()
    }

    /// Snapshot of all live nodes — includes a raw process handle (isize)
    /// so the health loop can call WaitForSingleObject without holding the lock.
    /// On non-Windows, `proc_handle_raw` is always 0.
    pub fn list_live(&self) -> Vec<NodeSummary> {
        let health_guard  = self.health_statuses.read();
        let service_guard = self.service_labels.read();
        self.nodes.read().values().map(|h| {
            let label = service_guard.get(&h.node_id).cloned();
            NodeSummary {
                node_id:         h.node_id,
                pid:             h.pid,
                slot_idx:        h.slot_idx,
                app_name:        h.app_name.clone(),
                pipe_path:       h.pipe_path.clone(),
                #[cfg(windows)]
                proc_handle_raw: h.proc_handle_raw(),
                #[cfg(not(windows))]
                proc_handle_raw: 0isize,
                event_tx:        h.event_tx.clone(),
                log_tx:          h.log_tx.clone(),
                health:          health_guard.get(&h.node_id).copied().unwrap_or_default(),
                service_name:    label.as_ref().map(|(n, _)| n.clone()),
                replica_index:   label.map(|(_, i)| i),
                health_check:    h.spawn_msg.health_check.clone(),
            }
        }).collect()
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NodeSummary {
    pub node_id:         Uuid,
    pub pid:             u32,
    pub slot_idx:        usize,
    pub app_name:        String,
    pub pipe_path:       String,
    /// Raw Win32 HANDLE value (as isize).  0 on non-Windows.
    ///
    /// **IMPORTANT — never serialize this field over the IPC pipe.**
    /// Win32 HANDLEs are process-local and meaningless to any other process.
    /// This field is only used internally by the health-monitoring loop
    /// (`WaitForSingleObject`).  The IPC wire type `NodeInfo` intentionally
    /// omits it.
    pub proc_handle_raw: isize,
    pub event_tx:        tokio::sync::broadcast::Sender<NodeEventMsg>,
    pub log_tx:          tokio::sync::broadcast::Sender<NodeLogChunkMsg>,
    /// Current health status (from the health-check loop).
    pub health:          HealthStatus,
    /// Compose service name, if managed by the reconciler.
    pub service_name:    Option<String>,
    /// Replica index within the service.
    pub replica_index:   Option<u32>,
    /// Optional health-check probe config (from the original SpawnNodeMsg).
    pub health_check:    Option<HealthCheck>,
}

impl CoreState {
    fn open_registry_and_dir() -> anyhow::Result<(PathBuf, Registry)> {
        if let Ok(custom) = std::env::var("VELOCE_DATA_DIR") {
            let dir = PathBuf::from(custom);
            let _ = std::fs::create_dir_all(&dir);
            let reg = Registry::open(dir.join("veloce-registry.bin"))?;
            return Ok((dir, reg));
        }

        // 1. Try system directory first
        let sys_dir = primary_system_data_dir();
        if std::fs::create_dir_all(&sys_dir).is_ok() {
            if let Ok(reg) = Registry::open(sys_dir.join("veloce-registry.bin")) {
                tracing::info!("Using system data directory: {}", sys_dir.display());
                return Ok((sys_dir, reg));
            }
        }

        // 2. Fall back to user local AppData directory
        let user_dir = user_data_dir();
        std::fs::create_dir_all(&user_dir)
            .with_context(|| format!("failed to create user data directory in {}", user_dir.display()))?;
        let reg = Registry::open(user_dir.join("veloce-registry.bin"))
            .with_context(|| format!("failed to open registry in {}", user_dir.display()))?;
        tracing::info!("Using user data directory: {}", user_dir.display());
        Ok((user_dir, reg))
    }
}

pub fn primary_system_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let pd = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
        PathBuf::from(pd).join("VeloceSolutions").join("VeloceCore")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/veloce-core")
    }
}

pub fn user_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let lad = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
            std::env::var("USERPROFILE")
                .map(|p| format!("{p}\\AppData\\Local"))
                .unwrap_or_else(|_| ".".into())
        });
        PathBuf::from(lad).join("VeloceSolutions").join("VeloceCore")
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".local").join("share").join("veloce").join("core")
    }
}