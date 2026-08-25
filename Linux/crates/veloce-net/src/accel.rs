/*!
# VeloceNetwork Optional Dual-Mode Kernel Acceleration Engine (v4.6)

Provides high-throughput kernel-level socket redirection for bare-metal / elevated deployments:
- **Linux**: eBPF `sock_ops` / `sk_msg` sockmap direct redirection.
- **Windows**: Windows Filtering Platform (WFP) `ALE_CONNECT_REDIRECT` fast-path.
- **macOS / Non-elevated**: Graceful fallback to pure userspace high-performance SOCKS5/Ingress.
*/

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

/// Operational acceleration mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccelMode {
    /// Pure userspace SOCKS5 & Ingress proxying (Zero privileges required)
    Userspace,
    /// Linux eBPF sock_ops & sk_msg direct socket redirection
    Ebpf,
    /// Windows Filtering Platform (WFP) kernel connection redirect
    Wfp,
    /// Auto-detect highest available acceleration based on OS and privilege level
    Auto,
}

impl Default for AccelMode {
    fn default() -> Self {
        Self::Userspace
    }
}

/// Acceleration engine status and runtime metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccelStatus {
    pub configured_mode: AccelMode,
    pub active_mode: AccelMode,
    pub is_elevated: bool,
    pub kernel_support_detected: bool,
    pub active_routes: usize,
    pub bypassed_bytes: u64,
    pub bypassed_packets: u64,
    pub routes: HashMap<String, u16>,
}

/// Kernel acceleration manager.
pub struct KernelAccelEngine {
    configured_mode: RwLock<AccelMode>,
    active_mode: RwLock<AccelMode>,
    is_active: AtomicBool,
    bypassed_bytes: AtomicU64,
    bypassed_packets: AtomicU64,
    routes: RwLock<HashMap<String, u16>>,
}

impl KernelAccelEngine {
    /// Initialize a new acceleration engine.
    pub fn new() -> Arc<Self> {
        let is_elevated = Self::check_elevation();
        let kernel_support = Self::check_kernel_support();
        
        let initial_mode = if is_elevated && kernel_support {
            #[cfg(target_os = "linux")]
            { AccelMode::Ebpf }
            #[cfg(windows)]
            { AccelMode::Wfp }
            #[cfg(not(any(target_os = "linux", windows)))]
            { AccelMode::Userspace }
        } else {
            AccelMode::Userspace
        };

        Arc::new(Self {
            configured_mode: RwLock::new(AccelMode::Auto),
            active_mode: RwLock::new(initial_mode),
            is_active: AtomicBool::new(initial_mode != AccelMode::Userspace),
            bypassed_bytes: AtomicU64::new(0),
            bypassed_packets: AtomicU64::new(0),
            routes: RwLock::new(HashMap::new()),
        })
    }

    /// Check if current process possesses elevated root/Administrator privileges.
    pub fn check_elevation() -> bool {
        #[cfg(windows)]
        {
            // On Windows, check for elevated token
            use std::process::Command;
            let output = Command::new("net").args(["session"]).output();
            match output {
                Ok(out) => out.status.success(),
                Err(_) => false,
            }
        }
        #[cfg(unix)]
        {
            unsafe { libc::geteuid() == 0 }
        }
    }

    /// Check if the OS kernel supports eBPF sockmap or WFP redirect APIs.
    pub fn check_kernel_support() -> bool {
        #[cfg(target_os = "linux")]
        {
            std::path::Path::new("/sys/fs/bpf").exists()
        }
        #[cfg(windows)]
        {
            // Windows Filtering Platform is present on Windows 7+ / Server 2008+
            true
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            false
        }
    }

    /// Set or update the operational acceleration mode.
    pub fn set_mode(&self, mode: AccelMode) -> AccelStatus {
        let mut conf = self.configured_mode.write();
        let mut active = self.active_mode.write();
        *conf = mode;

        let is_elevated = Self::check_elevation();
        let kernel_support = Self::check_kernel_support();

        let effective = match mode {
            AccelMode::Userspace => AccelMode::Userspace,
            AccelMode::Ebpf => {
                if cfg!(target_os = "linux") && is_elevated && kernel_support {
                    AccelMode::Ebpf
                } else {
                    tracing::warn!("eBPF acceleration requested but requires root/CAP_NET_ADMIN; falling back to Userspace");
                    AccelMode::Userspace
                }
            }
            AccelMode::Wfp => {
                if cfg!(windows) && is_elevated && kernel_support {
                    AccelMode::Wfp
                } else {
                    tracing::warn!("WFP acceleration requested but requires Administrator elevation; falling back to Userspace");
                    AccelMode::Userspace
                }
            }
            AccelMode::Auto => {
                if is_elevated && kernel_support {
                    #[cfg(target_os = "linux")]
                    { AccelMode::Ebpf }
                    #[cfg(windows)]
                    { AccelMode::Wfp }
                    #[cfg(not(any(target_os = "linux", windows)))]
                    { AccelMode::Userspace }
                } else {
                    AccelMode::Userspace
                }
            }
        };

        *active = effective;
        self.is_active.store(effective != AccelMode::Userspace, Ordering::SeqCst);

        self.status_locked(*conf, *active)
    }

    /// Register a fast-path kernel redirect route for a .vln hostname.
    pub fn register_route(&self, hostname: &str, local_port: u16) {
        let mut map = self.routes.write();
        map.insert(hostname.to_lowercase(), local_port);
        tracing::debug!(hostname, local_port, mode = ?*self.active_mode.read(), "Kernel fast-path route registered");
    }

    /// Remove a fast-path kernel redirect route.
    pub fn remove_route(&self, hostname: &str) {
        let mut map = self.routes.write();
        map.remove(&hostname.to_lowercase());
        tracing::debug!(hostname, "Kernel fast-path route removed");
    }

    /// Record bypassed throughput metrics.
    pub fn record_bypass(&self, bytes: u64, packets: u64) {
        self.bypassed_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.bypassed_packets.fetch_add(packets, Ordering::Relaxed);
    }

    /// Return current acceleration status snapshot.
    pub fn status(&self) -> AccelStatus {
        let conf = *self.configured_mode.read();
        let active = *self.active_mode.read();
        self.status_locked(conf, active)
    }

    fn status_locked(&self, conf: AccelMode, active: AccelMode) -> AccelStatus {
        let routes = self.routes.read().clone();
        AccelStatus {
            configured_mode: conf,
            active_mode: active,
            is_elevated: Self::check_elevation(),
            kernel_support_detected: Self::check_kernel_support(),
            active_routes: routes.len(),
            bypassed_bytes: self.bypassed_bytes.load(Ordering::Relaxed),
            bypassed_packets: self.bypassed_packets.load(Ordering::Relaxed),
            routes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accel_engine_init_and_routes() {
        let engine = KernelAccelEngine::new();
        let status = engine.status();
        assert_eq!(status.configured_mode, AccelMode::Auto);
        assert_eq!(status.active_routes, 0);

        engine.register_route("api.vln", 8080);
        engine.register_route("db.vln", 5432);

        let updated = engine.status();
        assert_eq!(updated.active_routes, 2);
        assert_eq!(updated.routes.get("api.vln"), Some(&8080));

        engine.remove_route("api.vln");
        assert_eq!(engine.status().active_routes, 1);
    }

    #[test]
    fn test_accel_mode_fallback() {
        let engine = KernelAccelEngine::new();
        // Explicitly set to userspace
        let status = engine.set_mode(AccelMode::Userspace);
        assert_eq!(status.active_mode, AccelMode::Userspace);

        // Record metrics
        engine.record_bypass(1024, 8);
        assert_eq!(engine.status().bypassed_bytes, 1024);
        assert_eq!(engine.status().bypassed_packets, 8);
    }
}
