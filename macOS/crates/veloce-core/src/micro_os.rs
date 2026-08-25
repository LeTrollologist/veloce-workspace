/*!
VeloceOS — Personal Micro-Mini OS Runtime & Virtual Supervisor (v4.3).

Provides userspace OS virtualization:
- System boot lifecycle and uptime management.
- Dynamic `/proc` file generation for `/proc/status`, `/proc/nodes`, `/proc/mesh`, `/proc/mounts`.
- Virtual process table managing micro-services, Wasm tasks, and virtual devices.
*/

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use veloce_ipc::message::OsStatusMsg;
use crate::vfs::VfsEngine;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicroProcessInfo {
    pub pid: u64,
    pub name: String,
    pub binary_path: String,
    pub memory_bytes: u64,
    pub cpu_percent: f32,
    pub started_at_secs: u64,
}

pub struct MicroOsRuntime {
    pub os_name: String,
    pub kernel_version: String,
    pub boot_time: Instant,
    pub vfs: Arc<VfsEngine>,
    pub micro_procs: RwLock<Vec<MicroProcessInfo>>,
    next_pid: AtomicU64,
    pub virtual_mounts: RwLock<Vec<String>>,
}

impl MicroOsRuntime {
    pub fn new(vfs: Arc<VfsEngine>) -> Arc<Self> {
        let runtime = Arc::new(Self {
            os_name: "VeloceOS".to_string(),
            kernel_version: "4.3.0-userspace".to_string(),
            boot_time: Instant::now(),
            vfs: Arc::clone(&vfs),
            micro_procs: RwLock::new(Vec::new()),
            next_pid: AtomicU64::new(100),
            virtual_mounts: RwLock::new(vec![
                "/ (velocevfs, rootfs, rw)".to_string(),
                "/proc (procfs, virtual, ro)".to_string(),
                "/dev (devfs, virtual, rw)".to_string(),
                "/tmp (tmpfs, memory, rw)".to_string(),
                "/vln/storage (vfs_volume, encrypted, rw)".to_string(),
            ]),
        });

        // Register dynamic /proc generator with VFS
        let runtime_weak = Arc::downgrade(&runtime);
        vfs.set_proc_provider(Arc::new(move |handler| {
            if let Some(rt) = runtime_weak.upgrade() {
                rt.generate_proc_content(handler)
            } else {
                Ok(b"VeloceOS unavailable\n".to_vec())
            }
        }));

        runtime
    }

    /// Return full OS status for IPC and CLI.
    pub fn status(&self) -> OsStatusMsg {
        let uptime_secs = self.boot_time.elapsed().as_secs();
        let (total_inodes, used_vfs_bytes) = self.vfs.usage_metrics();
        let active_micro_procs = self.micro_procs.read().len();
        let virtual_mounts = self.virtual_mounts.read().clone();

        OsStatusMsg {
            os_name: self.os_name.clone(),
            kernel_version: self.kernel_version.clone(),
            uptime_secs,
            total_inodes,
            used_vfs_bytes,
            active_micro_procs,
            virtual_mounts,
        }
    }

    /// Generate dynamic string content for `/proc` virtual endpoints.
    pub fn generate_proc_content(&self, handler_name: &str) -> Result<Vec<u8>> {
        match handler_name {
            "version" => {
                let s = format!("{} Kernel version {}\n", self.os_name, self.kernel_version);
                Ok(s.into_bytes())
            }
            "status" => {
                let status = self.status();
                let json = serde_json::to_string_pretty(&status).unwrap_or_default();
                Ok(format!("{}\n", json).into_bytes())
            }
            "nodes" => {
                let procs = self.micro_procs.read();
                let mut out = String::from("PID\tNAME\t\tMEM(KB)\tCPU%\tPATH\n");
                for p in procs.iter() {
                    out.push_str(&format!(
                        "{}\t{}\t\t{}\t{:.1}\t{}\n",
                        p.pid,
                        p.name,
                        p.memory_bytes / 1024,
                        p.cpu_percent,
                        p.binary_path
                    ));
                }
                Ok(out.into_bytes())
            }
            "mesh" => {
                Ok(b"VeloceOS P2P WireGuard-Grade Mesh: Active\nProtocols: Noise_IK_25519_ChaChaPoly_BLAKE2s\n".to_vec())
            }
            "mounts" => {
                let mounts = self.virtual_mounts.read();
                let mut out = String::new();
                for m in mounts.iter() {
                    out.push_str(&format!("{}\n", m));
                }
                Ok(out.into_bytes())
            }
            _ => Ok(format!("/proc/{} endpoint\n", handler_name).into_bytes()),
        }
    }

    /// Register a running micro-process in the virtual process table.
    pub fn register_process(&self, name: &str, binary_path: &str, memory_bytes: u64, cpu_percent: f32) -> u64 {
        let pid = self.next_pid.fetch_add(1, Ordering::SeqCst);
        let info = MicroProcessInfo {
            pid,
            name: name.to_string(),
            binary_path: binary_path.to_string(),
            memory_bytes,
            cpu_percent,
            started_at_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        self.micro_procs.write().push(info);
        pid
    }

    /// Unregister a micro-process upon exit.
    pub fn unregister_process(&self, pid: u64) {
        self.micro_procs.write().retain(|p| p.pid != pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_micro_os_status_and_proc_generation() {
        let vfs = Arc::new(VfsEngine::new());
        let os = MicroOsRuntime::new(Arc::clone(&vfs));

        let status = os.status();
        assert_eq!(status.os_name, "VeloceOS");
        assert!(status.total_inodes >= 5);

        // Read /proc/status through VFS
        let status_read = vfs.read_file("/proc/status").unwrap();
        let status_str = String::from_utf8_lossy(&status_read.data);
        assert!(status_str.contains("VeloceOS"));

        // Register process
        let pid = os.register_process("web-api", "/bin/web-api.wasm", 1024 * 1024, 12.5);
        assert!(pid >= 100);

        let nodes_read = vfs.read_file("/proc/nodes").unwrap();
        let nodes_str = String::from_utf8_lossy(&nodes_read.data);
        assert!(nodes_str.contains("web-api"));

        os.unregister_process(pid);
        let nodes_read2 = vfs.read_file("/proc/nodes").unwrap();
        assert!(!String::from_utf8_lossy(&nodes_read2.data).contains("web-api"));
    }
}
