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
use crate::job::NodeHandle;
use veloce_net::NetRegistry;

pub struct CoreState {
    registry:     Registry,
    node_table:   Arc<NodeTable>,
    net_registry: Arc<NetRegistry>,
    shutdown:     AtomicBool,
}

impl CoreState {
    pub fn new() -> anyhow::Result<Self> {
        let db_path = data_dir().join("veloce-registry.bin");
        std::fs::create_dir_all(db_path.parent().unwrap())?;
        let registry = Registry::open(&db_path)?;

        Ok(Self {
            registry,
            node_table:   Arc::new(NodeTable::new()),
            net_registry: Arc::new(NetRegistry::new()),
            shutdown:     AtomicBool::new(false),
        })
    }

    pub fn registry(&self)     -> &Registry         { &self.registry }
    pub fn node_table(&self)   -> &Arc<NodeTable>   { &self.node_table }
    pub fn net_registry(&self) -> &Arc<NetRegistry> { &self.net_registry }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
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
    pub proc_handle_raw: isize,
    pub event_tx:        tokio::sync::broadcast::Sender<crate::job::NodeEventMsg>,
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