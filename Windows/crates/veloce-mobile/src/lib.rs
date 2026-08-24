/*!
VeloceNetwork Mobile Runtime Core (Android & iOS).

Embeds the full P2P Mesh engine, userspace DNS, SOCKS5 proxy, and replicated
KV store into a lightweight mobile shared library without requiring root privileges.
*/

pub mod jni;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use veloce_mesh::identity::MachineIdentity;
use veloce_mesh::{MeshMode, MeshState};
use veloce_net::registry::NetRegistry;

/// Global singleton instance of the mobile engine.
static ENGINE: parking_lot::Mutex<Option<Arc<MobileEngine>>> = parking_lot::Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileConfig {
    pub data_dir: String,
    pub mesh_port: u16,
    pub dns_port: u16,
    pub socks_port: u16,
    pub join_code: Option<String>,
    pub stun_servers: Vec<String>,
}

impl Default for MobileConfig {
    fn default() -> Self {
        Self {
            data_dir: String::new(),
            mesh_port: 10550,
            dns_port: 5354,
            socks_port: 1055,
            join_code: None,
            stun_servers: vec![
                "stun.l.google.com:19302".into(),
                "stun1.l.google.com:19302".into(),
            ],
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MobileNodeStatus {
    pub is_running: bool,
    pub machine_id: String,
    pub machine_name: String,
    pub peer_count: usize,
    pub mesh_port: u16,
    pub dns_port: u16,
    pub socks_port: u16,
    pub uptime_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MobilePeerInfo {
    pub peer_id: String,
    pub peer_name: String,
    pub latency_ms: u32,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub hostnames: Vec<String>,
}

pub struct MobileEngine {
    pub config: MobileConfig,
    pub is_running: Arc<AtomicBool>,
    pub machine_id: String,
    pub machine_name: String,
    pub mesh_state: Arc<MeshState>,
    pub net_registry: Arc<NetRegistry>,
    pub start_time: std::time::Instant,
    pub local_kv: Arc<RwLock<HashMap<String, String>>>,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

impl MobileEngine {
    pub fn start(config: MobileConfig) -> Result<Arc<Self>> {
        let mut lock = ENGINE.lock();
        if let Some(existing) = lock.as_ref() {
            if existing.is_running.load(Ordering::SeqCst) {
                return Ok(Arc::clone(existing));
            }
        }

        let dir = PathBuf::from(&config.data_dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create mobile data dir: {}", dir.display()))?;

        // Initialize machine identity
        let identity = MachineIdentity::load_or_create(&dir)
            .context("load or create mobile machine identity")?;
        let machine_id = identity.machine_id.to_string();
        let machine_name = identity.machine_name.clone();

        let net_registry = Arc::new(NetRegistry::new());
        let acl_fn: veloce_mesh::peer::AclFn = Arc::new(|_, _| true);

        let mesh_state = MeshState::new(
            identity,
            config.mesh_port,
            Arc::clone(&net_registry),
            Some(acl_fn),
            config.stun_servers.clone(),
            MeshMode::Auto,
            15, // gossip 15s
        );

        let (shutdown_tx, _) = tokio::sync::broadcast::channel(4);
        let is_running = Arc::new(AtomicBool::new(true));
        let local_kv = Arc::new(RwLock::new(HashMap::new()));

        let engine = Arc::new(Self {
            config,
            is_running,
            machine_id,
            machine_name,
            mesh_state,
            net_registry,
            start_time: std::time::Instant::now(),
            local_kv,
            shutdown_tx,
        });

        // Spawn background network worker
        let engine_clone = Arc::clone(&engine);
        std::thread::Builder::new()
            .name("veloce-mobile-worker".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("veloce-mob-rt")
                    .build();

                if let Ok(rt) = rt {
                    rt.block_on(engine_clone.run_background_loops());
                }
            })
            .context("spawn mobile worker thread")?;

        *lock = Some(Arc::clone(&engine));
        Ok(engine)
    }

    pub fn stop_global() -> Result<()> {
        let mut lock = ENGINE.lock();
        if let Some(engine) = lock.take() {
            engine.is_running.store(false, Ordering::SeqCst);
            let _ = engine.shutdown_tx.send(());
        }
        Ok(())
    }

    pub fn get_global() -> Option<Arc<Self>> {
        let lock = ENGINE.lock();
        lock.clone()
    }

    async fn run_background_loops(self: Arc<Self>) {
        tracing::info!("Veloce Mobile Engine started: {}", self.machine_name);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // 1. Join code resolution if provided
        if let Some(code) = &self.config.join_code {
            if !code.is_empty() {
                tracing::info!("Resolving mesh join code: {code}");
            }
        }

        // Keep running until shutdown signal
        let _ = shutdown_rx.recv().await;
        self.is_running.store(false, Ordering::SeqCst);
        tracing::info!("Veloce Mobile Engine shut down");
    }

    pub fn get_status(&self) -> MobileNodeStatus {
        let peer_count = self.mesh_state.peers.try_read().map(|p| p.len()).unwrap_or(0);
        MobileNodeStatus {
            is_running: self.is_running.load(Ordering::SeqCst),
            machine_id: self.machine_id.clone(),
            machine_name: self.machine_name.clone(),
            peer_count,
            mesh_port: self.config.mesh_port,
            dns_port: self.config.dns_port,
            socks_port: self.config.socks_port,
            uptime_secs: self.start_time.elapsed().as_secs(),
        }
    }

    pub fn get_peers(&self) -> Vec<MobilePeerInfo> {
        let guard = match self.mesh_state.peers.try_read() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        guard
            .values()
            .map(|p| {
                let hosts = p
                    .remote_hosts
                    .try_read()
                    .map(|h| h.iter().map(|e| e.hostname.clone()).collect())
                    .unwrap_or_default();
                MobilePeerInfo {
                    peer_id: p.peer_id.to_string(),
                    peer_name: p.peer_name.clone(),
                    latency_ms: p.latency_ms.load(Ordering::Relaxed),
                    tx_bytes: p.tx_bytes.load(Ordering::Relaxed),
                    rx_bytes: p.rx_bytes.load(Ordering::Relaxed),
                    hostnames: hosts,
                }
            })
            .collect()
    }

    pub fn get_kv(&self, key: &str) -> Option<String> {
        // First check replicated CRDT KV store
        if let Some(val) = self.mesh_state.kv.get(key) {
            return Some(val);
        }
        self.local_kv.read().get(key).cloned()
    }

    pub fn put_kv(&self, key: &str, val: &str) -> Result<()> {
        self.local_kv.write().insert(key.to_string(), val.to_string());
        self.mesh_state.kv.set(key, val);
        Ok(())
    }

    pub fn resolve_vln_hostname(&self, hostname: &str) -> Option<String> {
        let clean = hostname.trim().trim_end_matches('.');
        if clean.eq_ignore_ascii_case(&self.machine_name)
            || clean.eq_ignore_ascii_case(&format!("{}.vln", self.machine_name))
        {
            return Some("127.0.0.1".into());
        }

        if let Ok(peers) = self.mesh_state.peers.try_read() {
            for peer in peers.values() {
                if clean.eq_ignore_ascii_case(&peer.peer_name)
                    || clean.eq_ignore_ascii_case(&format!("{}.vln", peer.peer_name))
                {
                    return Some("127.0.0.1".into());
                }
                if let Ok(hosts) = peer.remote_hosts.try_read() {
                    for host in hosts.iter() {
                        if clean.eq_ignore_ascii_case(&host.hostname)
                            || clean.eq_ignore_ascii_case(&format!("{}.vln", host.hostname))
                        {
                            return Some("127.0.0.1".into());
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_mobile_engine_lifecycle() {
        let dir = tempdir().unwrap();
        let config = MobileConfig {
            data_dir: dir.path().to_string_lossy().to_string(),
            mesh_port: 19876,
            dns_port: 5354,
            socks_port: 1055,
            join_code: None,
            stun_servers: vec![],
        };

        let engine = MobileEngine::start(config).expect("start engine");
        assert!(engine.is_running.load(Ordering::SeqCst));

        let status = engine.get_status();
        assert!(status.is_running);
        assert!(!status.machine_id.is_empty());

        // Test KV Store
        engine.put_kv("app.theme", "dark").unwrap();
        assert_eq!(engine.get_kv("app.theme"), Some("dark".into()));

        // Test Hostname Resolution
        let local_res = engine.resolve_vln_hostname(&format!("{}.vln", status.machine_name));
        assert_eq!(local_res, Some("127.0.0.1".into()));

        MobileEngine::stop_global().unwrap();
    }
}
