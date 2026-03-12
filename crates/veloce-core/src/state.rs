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
use parking_lot::RwLock;
use uuid::Uuid;

use crate::registry::Registry;
use crate::job::{NodeHandle, NodeEventMsg};
use veloce_ipc::message::NodeLogChunkMsg;
use veloce_net::NetRegistry;
use veloce_mesh::{MeshState, DEFAULT_MESH_PORT};

pub struct CoreState {
    registry:     Registry,
    node_table:   Arc<NodeTable>,
    net_registry: Arc<NetRegistry>,
    /// Optional P2P mesh state.  `None` if the mesh server failed to bind.
    pub mesh:     Option<Arc<MeshState>>,
    shutdown:     AtomicBool,
    /// Per-session pre-shared key.  32 truly random bytes from OsRng, written to
    /// the PSK file at startup; every connecting client must echo them.
    psk:          [u8; 32],
}

impl CoreState {
    pub fn new() -> anyhow::Result<Self> {
        let dir = data_dir();
        let db_path = dir.join("veloce-registry.bin");
        std::fs::create_dir_all(&dir)?;
        let registry = Registry::open(&db_path)?;

        let psk = generate_and_persist_psk()?;
        let net_registry = Arc::new(NetRegistry::new());

        // Build mesh identity and state.
        let mesh = match veloce_mesh::identity::MachineIdentity::load_or_create(&dir) {
            Ok(id) => Some(MeshState::new(id, DEFAULT_MESH_PORT, Arc::clone(&net_registry))),
            Err(e) => {
                tracing::warn!("mesh identity init failed (mesh disabled): {e}");
                None
            }
        };

        Ok(Self {
            registry,
            node_table:   Arc::new(NodeTable::new()),
            net_registry,
            mesh,
            shutdown:     AtomicBool::new(false),
            psk,
        })
    }

    pub fn registry(&self)     -> &Registry         { &self.registry }
    pub fn node_table(&self)   -> &Arc<NodeTable>   { &self.node_table }
    pub fn net_registry(&self) -> &Arc<NetRegistry> { &self.net_registry }
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
    nodes: RwLock<HashMap<Uuid, NodeHandle>>,
}

impl NodeTable {
    pub fn new() -> Self {
        Self { nodes: RwLock::new(HashMap::new()) }
    }

    pub fn insert(&self, handle: NodeHandle) {
        self.nodes.write().insert(handle.node_id, handle);
    }

    pub fn remove(&self, id: Uuid) -> Option<NodeHandle> {
        self.nodes.write().remove(&id)
    }

    pub fn get_pid(&self, id: Uuid) -> Option<u32> {
        self.nodes.read().get(&id).map(|h| h.pid)
    }

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
        self.nodes.read().values().map(|h| NodeSummary {
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
        }).collect()
    }
}

#[derive(Debug, Clone)]
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
}

fn data_dir() -> PathBuf {
    // %PROGRAMDATA%\VeloceSolutions\VeloceCore   (Windows)
    // /var/lib/veloce-core                        (Linux fallback)
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